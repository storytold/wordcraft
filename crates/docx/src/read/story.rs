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
    /// Once a field ends inside this one's code (`{ IF { REF a } = 0 … }`): the code as run XML,
    /// nested fields complete, kept as the field's `code_xml` (#1671). Empty without nesting.
    code_xml: String,
    /// The code can't be kept as XML (too long, or a tracked change inside the field that the
    /// field itself isn't in): the nested fields' results stand in for them in `instr`.
    code_xml_lost: bool,
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

fn children_of_choice(ac: &El) -> Option<&El> {
    // Prefer a Choice whose requirements we understand; else the Fallback; else any Choice.
    // `m`: Office Math (a producer may guard an equation with a picture fallback, #1273).
    const KNOWN: &[&str] = &["wps", "wpg", "wpc", "wpi", "w14", "w15", "wp14", "a14", "w16se", "w16cid", "w16", "w16cex", "w16sdtdh", "v", "m"];
    let ok = |c: &El| c.attr("Requires").unwrap_or("").split_whitespace().all(|r| KNOWN.contains(&r));
    ac.children("mc:Choice").find(|c| ok(c)).or_else(|| ac.child("mc:Fallback")).or_else(|| ac.child("mc:Choice"))
}

/// Gather `name` children (rows, cells), looking through `w:sdt`/`w:customXml` wrappers; each
/// with the content controls that open before it and how many close after it. `pending` holds
/// controls opened but not yet given to an element.
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
