//! Stories: blocks, paragraphs, runs, fields, drawings, tables.

use std::sync::Arc;

use wordcraft_doc::connector::{ConnEnd, ShapeExtra};
use wordcraft_doc::control::{ContentControl, ControlWrap};
use wordcraft_doc::effects::{Glow, Shadow, ShapeEffects};
use wordcraft_doc::graphic::{Embedded, Graphic, GraphicItem, GraphicKind};
use wordcraft_doc::para::{AltText, Anchor, Float, FloatAlign, NoteKind, PTabAlign, PTabBase, ShapeKind, Wrap, WrapText, cap_alt};
use wordcraft_doc::props::{CharProps, NumChange, PropChange, Rgb, TextColor};
use wordcraft_doc::table::{Cell, MAX_COLS, MAX_ROWS, Row, Table};
use wordcraft_doc::{Block, Blocks, InlineObject, Paragraph, PartKind, RevisionKind, Run, para_block};

use super::Reader;
use super::props::{sectpr, tcpr, trpr};
use super::shape_style::{own_fill, preset_kind, shape_paint, vml_color, vml_fill, vml_fraction, vml_preset_kind};
use crate::package::{ContentTypes, Rels, rt};
use crate::units::{int, measure, u32_of};
use crate::xml::{El, Node};
use wordcraft_doc::wordart::TextWarp;

mod blocks;
mod drawing;
mod fields;
mod runs;
mod vml;

const INK_URI: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingInk";
const INK_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml";
/// The `a:graphicData` URI of an Office 2016 chart (`cx:chart`).
const CHARTEX_URI: &str = "http://schemas.microsoft.com/office/drawing/2014/chartex";

/// Deepest table nesting we build (deeper tables become plain paragraphs).
const MAX_TABLE_DEPTH: usize = 24;
/// Deepest field nesting we track.
const MAX_FIELD_DEPTH: usize = 32;
/// Deepest text-box-in-text-box nesting.
const MAX_STORY_DEPTH: usize = 4;
/// Deepest `v:group` nesting followed (hostile files).
const MAX_VML_GROUP_DEPTH: usize = 8;
/// Longest field code we keep (Zotero's citation codes embed item data as JSON).
const MAX_INSTR: usize = 2 * 1024 * 1024;
/// Longest legacy form field definition (`w:ffData`) we keep.
const MAX_FORM: usize = 64 * 1024;

/// Inherited run context (from `w:hyperlink`, `w:ins`, `w:del` wrappers).
#[derive(Clone, Default)]
pub struct RunCtx {
    link: Option<String>,
    ins: Option<u32>,
    del: Option<u32>,
}

#[derive(PartialEq)]
enum Phase {
    Instr,
    Result,
}

/// A buffered piece of a field's result. Objects are boxed: an inline object is much larger than
/// a string.
enum Item {
    Text(String),
    Obj(Box<InlineObject>),
}

struct FieldState {
    instr: String,
    phase: Phase,
    spilled: bool,
    buf: Vec<(Item, CharProps)>,
    props: CharProps,
    locked: bool,
    /// `w:dirty` from the begin marker: the result is stale, Word refreshes it on open.
    dirty: bool,
    /// Kept as a range field: its result flows into the document as ordinary content between
    /// `FieldStart` and `FieldEnd` markers instead of being buffered.
    range: bool,
    /// The font size of each piece of the code (where it starts in `instr`): an enclosure's
    /// style is the size of its symbol and text (see [`wordcraft_doc::eq`]).
    sizes: Vec<(usize, Option<f32>)>,
    /// A legacy form field's `w:ffData`, as raw XML.
    form: String,
}

/// The last paragraph of a block list: its last block, or inside a final table the last
/// paragraph of its last cell (nested tables to [`MAX_TABLE_DEPTH`]).
fn last_para_mut(out: &mut Blocks, depth: usize) -> Option<&mut Paragraph> {
    match Arc::make_mut(out.last_mut()?) {
        Block::Para(p) => Some(p),
        Block::Table(t) if depth < MAX_TABLE_DEPTH => last_para_mut(&mut t.rows.last_mut()?.cells.last_mut()?.blocks, depth + 1),
        Block::Table(_) => None,
    }
}

/// The field that buffers content now: the innermost one that isn't a range field, unless it
/// has spilled (then content goes straight into the paragraph).
fn sink(sc: &mut StoryCtx) -> Option<&mut FieldState> {
    match sc.fields.iter_mut().rev().find(|f| !f.range) {
        Some(f) if !f.spilled => Some(f),
        _ => None,
    }
}

/// Per-story parsing state (fields may span paragraphs; block-level markers wait for the next
/// paragraph).
#[derive(Default)]
pub struct StoryCtx {
    fields: Vec<FieldState>,
    ignored_begins: usize,
    pending: Vec<InlineObject>,
    /// Text-box nesting depth.
    pub story_depth: usize,
    /// Reading a header or footer (not a text box inside one): where Word keeps its watermark.
    pub in_hf: bool,
}

/// Paragraph content under construction.
#[derive(Default)]
struct PB {
    text: String,
    runs: Vec<Run>,
    objects: Vec<InlineObject>,
}

impl PB {
    fn push_text(&mut self, s: &str, props: &CharProps) {
        if s.is_empty() {
            return;
        }
        self.text.push_str(s);
        self.push_run(s.len(), props);
    }
    fn push_obj(&mut self, o: InlineObject, props: &CharProps) {
        self.text.push(wordcraft_doc::para::OBJ);
        self.objects.push(o);
        self.push_run(wordcraft_doc::para::OBJ.len_utf8(), props);
    }
    fn push_run(&mut self, len: usize, props: &CharProps) {
        match self.runs.last_mut() {
            Some(r) if r.props == *props => r.len += len,
            _ => self.runs.push(Run { len, props: props.clone() }),
        }
    }
}

/// Clean text from `w:t`: drop characters with special meaning in the model and control
/// characters XML producers shouldn't have put there.
fn clean_text(s: &str) -> String {
    s.chars()
        .filter_map(|c| match c {
            '\n' | '\r' => Some(' '),
            '\t' => Some('\t'),
            wordcraft_doc::para::OBJ => None,
            c if (c as u32) < 0x20 => None,
            c => Some(c),
        })
        .collect()
}

/// Check MustUnderstand against namespaces this reader handles, rather than every URI the
/// XML serializer can name. Only the selected AlternateContent branch and processed content
/// of ignorable wrappers participate. The active namespace lists are counted by scope, so
/// deeply nested markup doesn't require repeatedly scanning every ancestor's lists.
pub(super) fn validate_must_understand(root: &El) -> Result<(), crate::DocxError> {
    use std::collections::HashMap;

    fn attr<'a>(e: &'a El, name: &str) -> Option<&'a str> {
        e.attrs.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }
    fn change<K: Eq + std::hash::Hash>(map: &mut HashMap<K, usize>, keys: impl Iterator<Item = K>, add: bool) {
        for key in keys {
            if add {
                *map.entry(key).or_default() += 1;
            } else if let Some(count) = map.get_mut(&key) {
                *count -= 1;
                if *count == 0 {
                    map.remove(&key);
                }
            }
        }
    }
    fn walk<'a>(
        e: &'a El,
        ignored: &mut HashMap<&'a str, usize>,
        processed: &mut HashMap<(&'a str, &'a str), usize>,
        depth: usize,
    ) -> Result<(), crate::DocxError> {
        if depth > crate::xml::MAX_DEPTH {
            return Err(crate::DocxError::Limit("markup compatibility nesting too deep".into()));
        }
        let ignorable = attr(e, "mc:Ignorable").unwrap_or("");
        let process = attr(e, "mc:ProcessContent").unwrap_or("");
        change(ignored, ignorable.split_whitespace(), true);
        change(processed, process.split_whitespace().filter_map(|s| s.split_once(':')), true);
        let name = e.name.split_once(':');
        let skip = name.is_some_and(|(prefix, local)| {
            !crate::xml::mc_understood(prefix)
                && ignored.contains_key(prefix)
                && !processed.contains_key(&(prefix, local))
                && !processed.contains_key(&(prefix, "*"))
        });
        let result = (|| {
            if skip {
                return Ok(());
            }
            if let Some(required) = attr(e, "mc:MustUnderstand")
                && let Some(prefix) = required.split_whitespace().find(|prefix| !crate::xml::mc_understood(prefix))
            {
                return Err(crate::DocxError::Xml(format!("unsupported namespace in mc:MustUnderstand: {prefix}")));
            }
            if e.name == "mc:AlternateContent" {
                let branch =
                    children_of_choice(e).ok_or_else(|| crate::DocxError::Xml("mc:AlternateContent has no supported choice or fallback".into()))?;
                walk(branch, ignored, processed, depth + 1)?;
            } else {
                for child in &e.kids {
                    if let Node::El(child) = child {
                        walk(child, ignored, processed, depth + 1)?;
                    }
                }
            }
            Ok(())
        })();
        change(processed, process.split_whitespace().filter_map(|s| s.split_once(':')), false);
        change(ignored, ignorable.split_whitespace(), false);
        result
    }
    walk(root, &mut HashMap::new(), &mut HashMap::new(), 0)
}

fn children_of_choice(ac: &El) -> Option<&El> {
    // Prefer a Choice whose requirements we understand; otherwise use the Fallback.
    // Native ink content parts are handled specially: the choice and its fallback are retained
    // together rather than losing the source by reading the fallback alone. That doesn't imply
    // support for other wpi choices or for MustUnderstand of the whole wpi vocabulary.
    let ok = |c: &El| {
        let mut requirements = c.attr("Requires").unwrap_or("").split_whitespace().peekable();
        let mut has_ink = None;
        requirements.peek().is_some()
            && requirements.all(|r| {
                crate::xml::mc_understood(r)
                    || (r == "wpi"
                        && *has_ink.get_or_insert_with(|| {
                            c.child("w14:contentPart").is_some()
                                || c.child("w:drawing")
                                    .and_then(|d| d.find("a:graphicData"))
                                    .is_some_and(|gd| gd.attr("uri") == Some(INK_URI) && gd.child("w14:contentPart").is_some())
                        }))
            })
    };
    ac.children("mc:Choice").find(|c| ok(c)).or_else(|| ac.child("mc:Fallback"))
}

/// Gather `name` children (rows, cells), looking through content controls, custom XML and the
/// selected alternate-content branch; each with the content controls that open before it and how
/// many close after it. `pending` holds controls opened but not yet given to an element.
fn collect<'a>(e: &'a El, name: &str, out: &mut Vec<(&'a El, ControlWrap)>, pending: &mut Vec<ContentControl>, depth: usize) {
    if depth > 8 {
        return;
    }
    for k in e.els() {
        if k.name == name {
            out.push((k, ControlWrap { open: std::mem::take(pending), close: 0 }));
        } else if k.name == "w:sdt" {
            if let Some(c) = k.child("w:sdtContent") {
                let mut ctl = super::sdt::sdt_pr(k.child("w:sdtPr"), true);
                ctl.end_pr_xml = super::sdt::sdt_end_pr(k.child("w:sdtEndPr"));
                let before = out.len();
                pending.push(ctl);
                collect(c, name, out, pending, depth + 1);
                let grew = out.len() > before;
                match out.last_mut() {
                    Some((_, w)) if grew => w.close = w.close.saturating_add(1),
                    // Nothing inside: the control goes (as before).
                    _ => {
                        pending.pop();
                    }
                }
            }
        } else if k.name == "w:customXml" {
            collect(k, name, out, pending, depth + 1);
        } else if k.name == "mc:AlternateContent"
            && let Some(branch) = children_of_choice(k)
        {
            collect(branch, name, out, pending, depth + 1);
        }
    }
}

/// Placeholder text as read from a control showing it: one line, bounded.
fn placeholder_of(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(wordcraft_doc::control::MAX_CONTROL_TEXT).collect::<String>().trim().to_string()
}

fn on_off_attr(e: &El, name: &str) -> bool {
    e.attr(name).is_some_and(|v| !matches!(v, "0" | "false" | "off"))
}

/// Whether parsed content contains explicitly preserved unknown markup.
/// The source package is then the only ownership boundary we can currently reproduce exactly.
pub(super) fn needs_source_preservation(root: &El) -> bool {
    use std::collections::HashMap;
    fn update<'a>(map: &mut HashMap<&'a str, usize>, value: &'a str, add: bool) {
        for token in value.split_whitespace() {
            if add {
                *map.entry(token).or_default() += 1;
            } else if let Some(n) = map.get_mut(token) {
                *n -= 1;
                if *n == 0 {
                    map.remove(token);
                }
            }
        }
    }
    fn matches_name(map: &HashMap<&str, usize>, name: &str) -> bool {
        let Some((prefix, _)) = name.split_once(':') else { return false };
        if crate::xml::mc_understood(prefix) {
            return false;
        }
        map.contains_key(name) || map.contains_key(format!("{prefix}:*").as_str())
    }
    // Choice selection uses '?' for unknown requirements; raw serialization cannot restore the
    // original prefix binding. Some recognized requirement aliases are not declared either.
    fn raw_choice_needs_source(e: &El) -> bool {
        e.name == "mc:Choice"
            && e.attrs.iter().find(|(name, _)| name == "Requires").is_some_and(|(_, value)| {
                value.split_whitespace().any(|prefix| !crate::xml::declared_namespaces().any(|(declared, _)| declared == prefix))
            })
    }
    // Only exact paths consumed by sdt_pr may bypass opaque-attribute protection. In
    // particular, a modeled-looking child inside a raw property is still raw markup.
    #[derive(Clone, Copy)]
    enum ControlPath<'a> {
        Outside,
        Root,
        Direct(&'a str),
        Nested(&'a str),
        Raw,
    }
    impl<'a> ControlPath<'a> {
        fn enter(self, name: &'a str) -> Self {
            match (self, name) {
                (Self::Outside, "w:sdtPr") => Self::Root,
                (Self::Outside, "w:sdtEndPr") => Self::Raw,
                (Self::Outside, _) => Self::Outside,
                (Self::Root, name)
                    if matches!(
                        name,
                        "w:alias"
                            | "w:tag"
                            | "w:id"
                            | "w:lock"
                            | "w:placeholder"
                            | "w:temporary"
                            | "w:showingPlcHdr"
                            | "w:richText"
                            | "w:text"
                            | "w14:checkbox"
                            | "w:comboBox"
                            | "w:dropDownList"
                            | "w:date"
                            | "w:picture"
                            | "w:docPartObj"
                            | "w:docPartList"
                            | "w15:repeatingSection"
                            | "w15:repeatingSectionItem"
                    ) =>
                {
                    Self::Direct(name)
                }
                (Self::Direct(root), name) if modeled_control_child(root, name) => Self::Nested(root),
                _ => Self::Raw,
            }
        }
    }
    fn modeled_control_child(root: &str, name: &str) -> bool {
        match root {
            "w:placeholder" => name == "w:docPart",
            "w:comboBox" | "w:dropDownList" => name == "w:listItem",
            "w:date" => matches!(name, "w:dateFormat" | "w:lid" | "w:calendar" | "w:storeMappedDataAs"),
            "w:docPartObj" | "w:docPartList" => matches!(name, "w:docPartGallery" | "w:docPartCategory" | "w:docPartUnique"),
            "w14:checkbox" => matches!(name, "w14:checked" | "w14:checkedState" | "w14:uncheckedState"),
            "w15:repeatingSection" => matches!(name, "w15:sectionTitle" | "w15:doNotAllowInsertDeleteSection"),
            _ => false,
        }
    }
    fn modeled_control_alias(path: ControlPath<'_>, element: &str, name: &str) -> bool {
        match path {
            ControlPath::Direct(element) => {
                (matches!(element, "w:alias" | "w:tag" | "w:id" | "w:lock" | "w:temporary" | "w:showingPlcHdr") && name == "val")
                    || element == "w:text" && name == "multiLine"
                    || element == "w:date" && name == "fullDate"
                    || matches!(element, "w:comboBox" | "w:dropDownList") && name == "lastValue"
            }
            ControlPath::Nested(root) => match root {
                "w:placeholder" | "w:date" | "w:docPartObj" | "w:docPartList" | "w15:repeatingSection" => name == "val",
                "w:comboBox" | "w:dropDownList" => matches!(name, "displayText" | "value"),
                "w14:checkbox" => name == "val" || (element != "w14:checked" && name == "font"),
                _ => false,
            },
            _ => false,
        }
    }
    fn walk<'a>(
        e: &'a El,
        elements: &mut HashMap<&'a str, usize>,
        attrs: &mut HashMap<&'a str, usize>,
        control_path: ControlPath<'a>,
        in_opaque_control_property: bool,
        depth: usize,
    ) -> bool {
        if depth > crate::xml::MAX_DEPTH {
            return false;
        }
        let control_path = control_path.enter(&e.name);
        let in_control_properties = !matches!(control_path, ControlPath::Outside);
        let in_opaque_control_property = in_opaque_control_property
            || (in_control_properties && e.name.split_once(':').is_some_and(|(prefix, _)| !crate::xml::mc_understood(prefix)));
        if in_control_properties {
            // Opaque attribute values may carry arbitrary QNames, including unprefixed ones.
            // Their original namespace scope cannot be reconstructed by the raw serializer.
            let ambiguous_attr = e.attrs.iter().any(|(name, _)| {
                if name.starts_with('?') || name.starts_with("xmlns:") || name.starts_with("mc:") {
                    return false;
                }
                if !in_opaque_control_property && modeled_control_alias(control_path, &e.name, name) {
                    return false;
                }
                in_opaque_control_property
                    || name == "xsi:type"
                    || crate::xml::is_rel_attr(&e.name, name)
                    || name.split_once(':').is_none_or(|(prefix, _)| !crate::xml::mc_understood(prefix))
            });
            // Opaque text can itself be a QName, even when it has no colon (the default
            // namespace may matter), so do not guess from its contents.
            let ambiguous_text = in_opaque_control_property && e.kids.iter().any(|n| matches!(n, Node::Text(text) if !text.trim().is_empty()));
            if ambiguous_attr || ambiguous_text || raw_choice_needs_source(e) {
                return true;
            }
        }
        let element_values = e.attrs.iter().find(|(k, _)| k == "mc:PreserveElements").map_or("", |(_, v)| v.as_str());
        let attr_values = e.attrs.iter().find(|(k, _)| k == "mc:PreserveAttributes").map_or("", |(_, v)| v.as_str());
        update(elements, element_values, true);
        update(attrs, attr_values, true);
        // Arbitrary retained attribute values may be QNames: only the source package preserves
        // all namespace bindings, even for a small, relationship-free ignored control property.
        let found = matches_name(elements, &e.name)
            || e.attrs.iter().any(|(k, _)| matches_name(attrs, k))
            || e.kids.iter().any(|n| match n {
                Node::El(c) => walk(c, elements, attrs, control_path, in_opaque_control_property, depth + 1),
                Node::Text(_) => false,
            });
        update(attrs, attr_values, false);
        update(elements, element_values, false);
        found
    }
    walk(root, &mut HashMap::new(), &mut HashMap::new(), ControlPath::Outside, false, 0)
}
