//! Content controls (`w:sdt`, ECMA-376 §17.5.2): fields of a form or template — rich and plain
//! text, check boxes, combo boxes and drop-down lists, date pickers, pictures, building block
//! galleries and repeating sections.
//!
//! A control around text is a [`InlineObject::ControlStart`] … [`InlineObject::ControlEnd`]
//! marker pair in the paragraph text, like a range field: its content is the ordinary content
//! between the markers, possibly spanning paragraphs (a block-level control sits around whole
//! paragraphs: its start at the start of the first, its end at the end of the last). Pairs nest
//! like brackets and never cross a block list. A control whose content starts or ends with a
//! table, and controls around table rows or cells, are kept on the table, row or cell
//! ([`ControlWrap`]) for the round trip.
//!
//! Editing can leave a marker without its partner. Deletions keep a marker whose partner is
//! outside what's deleted (so deleting across a control's edge empties it rather than breaking
//! it); writers call [`Document::balance_controls`] so a saved file always has whole controls.

use serde::{Deserialize, Serialize};

use crate::para::OBJ;
use crate::{Block, Blocks, Document, InlineObject, Path, Pos, StoryRef};

/// Deepest table nesting scanned (matches `walk` in the crate root).
const MAX_DEPTH: usize = 16;
/// Longest tag, title or list text kept from a file.
pub const MAX_CONTROL_TEXT: usize = 1024;
/// Most list entries a combo box or drop-down list keeps.
pub const MAX_LIST_ITEMS: usize = 1000;

/// What may be done to a control (`w:lock`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ControlLock {
    #[default]
    Unlocked,
    /// The control can't be deleted; its content can be edited.
    SdtLocked,
    /// The content can't be edited; the control can be deleted.
    ContentLocked,
    /// Neither.
    SdtContentLocked,
}

impl ControlLock {
    pub fn from_ooxml(v: &str) -> ControlLock {
        match v {
            "sdtLocked" => ControlLock::SdtLocked,
            "contentLocked" => ControlLock::ContentLocked,
            "sdtContentLocked" => ControlLock::SdtContentLocked,
            _ => ControlLock::Unlocked,
        }
    }
    /// The `w:lock` value, `None` when unlocked.
    pub fn ooxml(self) -> Option<&'static str> {
        match self {
            ControlLock::Unlocked => None,
            ControlLock::SdtLocked => Some("sdtLocked"),
            ControlLock::ContentLocked => Some("contentLocked"),
            ControlLock::SdtContentLocked => Some("sdtContentLocked"),
        }
    }
    pub fn from_flags(no_delete: bool, no_edit: bool) -> ControlLock {
        match (no_delete, no_edit) {
            (false, false) => ControlLock::Unlocked,
            (true, false) => ControlLock::SdtLocked,
            (false, true) => ControlLock::ContentLocked,
            (true, true) => ControlLock::SdtContentLocked,
        }
    }
    /// The control itself can be deleted.
    pub fn can_delete(self) -> bool {
        matches!(self, ControlLock::Unlocked | ControlLock::ContentLocked)
    }
    /// Its content can be edited.
    pub fn can_edit(self) -> bool {
        matches!(self, ControlLock::Unlocked | ControlLock::SdtLocked)
    }
}

/// An entry of a combo box or drop-down list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ListItem {
    /// What the control shows.
    pub display: String,
    /// What it stores (`w:value`); the display text when empty.
    pub value: String,
}

impl ListItem {
    pub fn value(&self) -> &str {
        if self.value.is_empty() { &self.display } else { &self.value }
    }
}

/// The kind of a content control and its settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ControlKind {
    /// Formatted text, paragraphs, anything (also controls of a type we don't model, whose type
    /// element is kept in [`ContentControl::extra`]).
    #[default]
    RichText,
    /// Unformatted text (`w:text`), on one line unless `multi_line`.
    #[serde(rename_all = "camelCase")]
    Text {
        #[serde(default)]
        multi_line: bool,
    },
    /// A check box (`w14:checkbox`): the content is the state's symbol.
    #[serde(rename_all = "camelCase")]
    CheckBox { checked: bool, checked_char: char, checked_font: String, unchecked_char: char, unchecked_font: String },
    /// A list to choose from, or type into (`w:comboBox`).
    #[serde(rename_all = "camelCase")]
    ComboBox {
        items: Vec<ListItem>,
        #[serde(default)]
        last_value: String,
    },
    /// A list to choose from (`w:dropDownList`).
    #[serde(rename_all = "camelCase")]
    DropDown {
        items: Vec<ListItem>,
        #[serde(default)]
        last_value: String,
    },
    /// A date (`w:date`): `full_date` is ISO 8601, `format` a date picture (`M/d/yyyy`).
    #[serde(rename_all = "camelCase")]
    Date {
        #[serde(default)]
        full_date: String,
        #[serde(default)]
        format: String,
        #[serde(default)]
        lid: String,
        #[serde(default)]
        calendar: String,
        #[serde(default)]
        store_as: String,
    },
    /// A picture (`w:picture`).
    Picture,
    /// A building block gallery (`w:docPartList`) or a building block placed from one
    /// (`w:docPartObj`: tables of contents, cover pages, bibliographies).
    #[serde(rename_all = "camelCase")]
    Gallery {
        /// `w:docPartList` rather than `w:docPartObj`.
        #[serde(default)]
        list: bool,
        #[serde(default)]
        gallery: String,
        #[serde(default)]
        category: String,
        #[serde(default)]
        unique: bool,
    },
    /// A repeating section (`w15:repeatingSection`), whose items are repeated.
    #[serde(rename_all = "camelCase")]
    RepeatingSection {
        #[serde(default)]
        title: String,
        #[serde(default)]
        no_insert_delete: bool,
    },
    /// One item of a repeating section (`w15:repeatingSectionItem`).
    RepeatingSectionItem,
}

/// Default check box symbols: ballot box with X, ballot box.
pub const CHECKED: char = '\u{2612}';
pub const UNCHECKED: char = '\u{2610}';

impl ControlKind {
    pub fn check_box() -> ControlKind {
        ControlKind::CheckBox {
            checked: false,
            checked_char: CHECKED,
            checked_font: String::new(),
            unchecked_char: UNCHECKED,
            unchecked_font: String::new(),
        }
    }
    /// The command-friendly name (`richText`, `checkBox`…).
    pub fn name(&self) -> &'static str {
        match self {
            ControlKind::RichText => "richText",
            ControlKind::Text { .. } => "plainText",
            ControlKind::CheckBox { .. } => "checkBox",
            ControlKind::ComboBox { .. } => "comboBox",
            ControlKind::DropDown { .. } => "dropDown",
            ControlKind::Date { .. } => "date",
            ControlKind::Picture => "picture",
            ControlKind::Gallery { .. } => "buildingBlock",
            ControlKind::RepeatingSection { .. } => "repeatingSection",
            ControlKind::RepeatingSectionItem => "repeatingSectionItem",
        }
    }
    /// The list entries of a combo box or drop-down list.
    pub fn items(&self) -> Option<&Vec<ListItem>> {
        match self {
            ControlKind::ComboBox { items, .. } | ControlKind::DropDown { items, .. } => Some(items),
            _ => None,
        }
    }
    /// Typed text may go in (not a check box, drop-down list or picture).
    pub fn takes_typing(&self) -> bool {
        !matches!(self, ControlKind::CheckBox { .. } | ControlKind::DropDown { .. } | ControlKind::Picture)
    }
    /// The text its placeholder shows by default.
    pub fn default_placeholder(&self) -> &'static str {
        match self {
            ControlKind::ComboBox { .. } | ControlKind::DropDown { .. } => "Pick an option.",
            ControlKind::Date { .. } => "Pick a date.",
            ControlKind::CheckBox { .. } | ControlKind::Picture | ControlKind::RepeatingSection { .. } | ControlKind::RepeatingSectionItem => "",
            _ => "Type here.",
        }
    }
}

/// A content control's properties (`w:sdtPr`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ContentControl {
    /// `w:id` (unique in the document).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// `w:tag`: a name for programs.
    pub tag: String,
    /// `w:alias`: the title shown on the control.
    pub title: String,
    pub lock: ControlLock,
    /// The building block holding the placeholder text (`w:placeholder/w:docPart`).
    pub placeholder: String,
    /// The content is the placeholder text (`w:showingPlcHdr`).
    pub showing_placeholder: bool,
    /// The control goes away once its content is edited (`w:temporary`).
    pub temporary: bool,
    pub kind: ControlKind,
    /// The text shown while the control is empty.
    pub placeholder_text: String,
    /// Around whole paragraphs rather than inside one.
    pub block: bool,
    /// The run properties of `w:sdtPr` (`w:rPr`), kept verbatim (XML).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub rpr_xml: String,
    /// Other `w:sdtPr` children we don't model, kept verbatim (XML) in file order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<String>,
    /// `w:sdtEndPr`, kept verbatim (XML).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub end_pr_xml: String,
}

impl ContentControl {
    pub fn new(kind: ControlKind) -> ContentControl {
        let placeholder_text = kind.default_placeholder().to_string();
        ContentControl { kind, placeholder_text, ..Default::default() }
    }
    /// The text to show while empty.
    pub fn placeholder_shown(&self) -> &str {
        if self.placeholder_text.is_empty() { self.kind.default_placeholder() } else { &self.placeholder_text }
    }
}

/// Content controls around a table, row or cell (see the module docs): the controls that open
/// just before it, outermost first, and how many of the open ones close just after it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ControlWrap {
    pub open: Vec<ContentControl>,
    pub close: u32,
}

impl ControlWrap {
    pub fn is_empty(&self) -> bool {
        self.open.is_empty() && self.close == 0
    }
}

/// A control boundary in a block list (not looking into table cells).
#[derive(Clone, Debug, PartialEq)]
pub enum ControlEvent<'a> {
    /// A control opens: at a `ControlStart` at byte `off` of paragraph `block`, or (`off`
    /// `None`) before table `block`.
    Open { block: usize, off: Option<usize>, control: &'a ContentControl },
    /// The innermost open control closes: at a `ControlEnd` at `off`, or after table `block`.
    Close { block: usize, off: Option<usize> },
}

impl ControlEvent<'_> {
    pub fn block(&self) -> usize {
        match self {
            ControlEvent::Open { block, .. } | ControlEvent::Close { block, .. } => *block,
        }
    }
    pub fn off(&self) -> Option<usize> {
        match self {
            ControlEvent::Open { off, .. } | ControlEvent::Close { off, .. } => *off,
        }
    }
}

/// The control boundaries of a block list, in order.
pub fn control_events(bl: &Blocks) -> Vec<ControlEvent<'_>> {
    let mut out = Vec::new();
    for (i, b) in bl.iter().enumerate() {
        match &**b {
            Block::Para(p) => {
                if !p.objects.iter().any(|o| matches!(o, InlineObject::ControlStart { .. } | InlineObject::ControlEnd)) {
                    continue;
                }
                for off in p.object_offsets() {
                    match p.object_at(off) {
                        Some(InlineObject::ControlStart { control }) => out.push(ControlEvent::Open { block: i, off: Some(off), control }),
                        Some(InlineObject::ControlEnd) => out.push(ControlEvent::Close { block: i, off: Some(off) }),
                        _ => {}
                    }
                }
            }
            Block::Table(t) => {
                for c in &t.controls.open {
                    out.push(ControlEvent::Open { block: i, off: None, control: c });
                }
                for _ in 0..t.controls.close.min(10_000) {
                    out.push(ControlEvent::Close { block: i, off: None });
                }
            }
        }
    }
    out
}

/// Pair events like brackets: `(open index, close index, depth)` for each whole control (in the
/// order they close), and the indexes of events without a partner.
pub fn pair_events(events: &[ControlEvent]) -> (Vec<(usize, usize, usize)>, Vec<usize>) {
    let mut open: Vec<usize> = Vec::new();
    let (mut pairs, mut orphans) = (Vec::new(), Vec::new());
    for (i, e) in events.iter().enumerate() {
        match e {
            ControlEvent::Open { .. } => open.push(i),
            ControlEvent::Close { .. } => match open.pop() {
                Some(o) => pairs.push((o, i, open.len())),
                None => orphans.push(i),
            },
        }
    }
    orphans.extend(open);
    orphans.sort_unstable();
    (pairs, orphans)
}

/// A whole content control around paragraph content.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlRange {
    pub control: ContentControl,
    /// Where the `ControlStart` marker is.
    pub start: Pos,
    /// Where the `ControlEnd` marker is (the content is everything between the two).
    pub end: Pos,
    /// Number of enclosing controls in the same block list.
    pub depth: usize,
}

impl ControlRange {
    /// The start of the content (just after the start marker).
    pub fn content_start(&self) -> Pos {
        Pos { off: self.start.off + OBJ.len_utf8(), ..self.start.clone() }
    }
    /// Whether `p` is inside the control (between its markers, either edge included).
    pub fn contains(&self, p: &Pos) -> bool {
        p.story == self.start.story
            && (&p.path, p.off) >= (&self.start.path, self.start.off + OBJ.len_utf8())
            && (&p.path, p.off) <= (&self.end.path, self.end.off)
    }
    /// Whether the content is empty (nothing but other controls' markers would be content too).
    pub fn is_empty(&self) -> bool {
        self.start.path == self.end.path && self.end.off == self.start.off + OBJ.len_utf8()
    }
}

fn scan(story: StoryRef, bl: &Blocks, prefix: &mut Vec<u32>, depth: usize, out: &mut Vec<ControlRange>) {
    if depth > MAX_DEPTH {
        return;
    }
    let events = control_events(bl);
    if !events.is_empty() {
        let (pairs, _) = pair_events(&events);
        for (o, c, d) in pairs {
            let (Some(ControlEvent::Open { block: ob, off: Some(oo), control }), Some(ControlEvent::Close { block: cb, off: Some(co) })) =
                (events.get(o), events.get(c))
            else {
                continue;
            };
            let at = |b: usize, off: usize| {
                let mut path = prefix.clone();
                path.push(u32::try_from(b).unwrap_or(u32::MAX));
                Pos::new(story, Path(path), off)
            };
            out.push(ControlRange { control: (*control).clone(), start: at(*ob, *oo), end: at(*cb, *co), depth: d });
        }
    }
    for (i, b) in bl.iter().enumerate() {
        if let Block::Table(t) = &**b {
            prefix.push(u32::try_from(i).unwrap_or(u32::MAX));
            for (r, row) in t.rows.iter().enumerate() {
                for (c, cell) in row.cells.iter().enumerate() {
                    prefix.push(u32::try_from(r).unwrap_or(u32::MAX));
                    prefix.push(u32::try_from(c).unwrap_or(u32::MAX));
                    scan(story, &cell.blocks, prefix, depth + 1, out);
                    prefix.pop();
                    prefix.pop();
                }
            }
            prefix.pop();
        }
    }
}

/// Positions of the markers without a partner in a block list and the cells inside it.
fn orphans(bl: &Blocks, prefix: &mut Vec<u32>, depth: usize, out: &mut Vec<(Path, usize)>, tables: &mut Vec<(Path, usize, bool)>) {
    if depth > MAX_DEPTH {
        return;
    }
    let events = control_events(bl);
    let (_, lone) = pair_events(&events);
    for k in lone {
        let Some(e) = events.get(k) else { continue };
        let mut path = prefix.clone();
        path.push(u32::try_from(e.block()).unwrap_or(u32::MAX));
        match (e, e.off()) {
            (_, Some(off)) => out.push((Path(path), off)),
            (ControlEvent::Open { .. }, None) => {
                // Which of the table's opens: count those before it on the same table.
                let nth = events.iter().take(k).filter(|x| matches!(x, ControlEvent::Open { block, off: None, .. } if *block == e.block())).count();
                tables.push((Path(path), nth, true));
            }
            (ControlEvent::Close { .. }, None) => tables.push((Path(path), 0, false)),
        }
    }
    for (i, b) in bl.iter().enumerate() {
        if let Block::Table(t) = &**b {
            prefix.push(u32::try_from(i).unwrap_or(u32::MAX));
            for (r, row) in t.rows.iter().enumerate() {
                for (c, cell) in row.cells.iter().enumerate() {
                    prefix.push(u32::try_from(r).unwrap_or(u32::MAX));
                    prefix.push(u32::try_from(c).unwrap_or(u32::MAX));
                    orphans(&cell.blocks, prefix, depth + 1, out, tables);
                    prefix.pop();
                    prefix.pop();
                }
            }
            prefix.pop();
        }
    }
}

/// Remove the markers without a partner from a list of blocks (a copied fragment).
pub fn balance_blocks(blocks: &mut [Block]) {
    let mut bl: Blocks = blocks.iter().cloned().map(std::sync::Arc::new).collect();
    if balance_list(&mut bl) > 0 {
        for (dst, src) in blocks.iter_mut().zip(bl) {
            *dst = std::sync::Arc::try_unwrap(src).unwrap_or_else(|a| (*a).clone());
        }
    }
}

/// Remove orphan markers (and table wraps) from a block list; returns how many went.
fn balance_list(bl: &mut Blocks) -> usize {
    let (mut paras, mut tables) = (Vec::new(), Vec::new());
    orphans(bl, &mut Vec::new(), 0, &mut paras, &mut tables);
    if paras.is_empty() && tables.is_empty() {
        return 0;
    }
    let mut d = Document { body: std::mem::take(bl), ..Document::new() };
    let removed = d.remove_orphans(StoryRef::Body, paras, tables);
    *bl = d.body;
    removed
}

impl Document {
    fn control_stories(&self) -> Vec<StoryRef> {
        std::iter::once(StoryRef::Body).chain(self.parts.keys().map(|k| StoryRef::Part(*k))).collect()
    }

    /// The whole content controls of a story (around paragraph content), in the order they close.
    pub fn control_ranges(&self, story: StoryRef) -> Vec<ControlRange> {
        let mut out = Vec::new();
        if let Some(bl) = self.story(story) {
            scan(story, bl, &mut Vec::new(), 0, &mut out);
        }
        out
    }

    /// The innermost content control holding `p`.
    pub fn control_at(&self, p: &Pos) -> Option<ControlRange> {
        let para = self.para_at(p)?;
        // Cheap test first: no controls anywhere near.
        if !para.objects.iter().any(|o| matches!(o, InlineObject::ControlStart { .. } | InlineObject::ControlEnd))
            && !self.story_has_controls(p.story)
        {
            return None;
        }
        self.control_ranges(p.story)
            .into_iter()
            .filter(|r| r.contains(p))
            .max_by(|a, b| (&a.start.path, a.start.off).cmp(&(&b.start.path, b.start.off)))
    }

    fn story_has_controls(&self, s: StoryRef) -> bool {
        let Some(bl) = self.story(s) else { return false };
        let mut any = false;
        for b in bl {
            crate::edit::each_para(b, 0, &mut |p| any |= p.objects.iter().any(|o| matches!(o, InlineObject::ControlStart { .. })));
            if any {
                return true;
            }
        }
        false
    }

    /// The control whose start marker is at `start`, to change.
    pub fn control_mut(&mut self, start: &Pos) -> Option<&mut ContentControl> {
        let p = self.para_mut(start.story, &start.path).ok()?;
        p.touch();
        match p.object_at_mut(start.off)? {
            InlineObject::ControlStart { control } => Some(control),
            _ => None,
        }
    }

    /// The content of a control as plain text (paragraphs joined by `\n`).
    pub fn control_text(&self, r: &ControlRange) -> String {
        let a = r.content_start();
        let mut out = String::new();
        for path in self.paths_between(&a, &r.end) {
            let Some(p) = self.para(a.story, &path) else { continue };
            let from = if path == a.path { a.off } else { 0 };
            let to = if path == r.end.path { r.end.off } else { p.len() };
            if !out.is_empty() {
                out.push('\n');
            }
            let mut k = p.text.get(..from).map(|t| t.chars().filter(|c| *c == OBJ).count()).unwrap_or(0);
            for c in p.text.get(from..to.max(from)).unwrap_or("").chars() {
                if c == OBJ {
                    if let Some(o) = p.objects.get(k) {
                        out.push_str(o.plain_text());
                    }
                    k += 1;
                } else {
                    out.push(c);
                }
            }
        }
        out
    }

    /// Whether any story holds a control marker without its partner.
    pub fn has_unbalanced_controls(&self) -> bool {
        self.control_stories().into_iter().any(|s| {
            self.story(s).is_some_and(|bl| {
                let (mut p, mut t) = (Vec::new(), Vec::new());
                orphans(bl, &mut Vec::new(), 0, &mut p, &mut t);
                !p.is_empty() || !t.is_empty()
            })
        })
    }

    /// Remove control markers (and table wraps' opens and closes) that have no partner. Returns
    /// how many were removed. (Row and cell wraps are balanced as they are written.)
    pub fn balance_controls(&mut self) -> usize {
        let mut removed = 0;
        for s in self.control_stories() {
            let Some(bl) = self.story(s) else { continue };
            let (mut paras, mut tables) = (Vec::new(), Vec::new());
            orphans(bl, &mut Vec::new(), 0, &mut paras, &mut tables);
            removed += self.remove_orphans(s, paras, tables);
        }
        removed
    }

    fn remove_orphans(&mut self, s: StoryRef, mut paras: Vec<(Path, usize)>, tables: Vec<(Path, usize, bool)>) -> usize {
        let mut removed = 0;
        // Last first, so earlier offsets in the same paragraph stay valid.
        paras.sort();
        for (path, off) in paras.into_iter().rev() {
            if let Ok(p) = self.para_mut(s, &path)
                && p.delete(off, off + OBJ.len_utf8()).is_ok()
            {
                removed += 1;
            }
        }
        // Table opens: highest index first per table.
        let mut tables = tables;
        tables.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        for (path, nth, open) in tables {
            if let Ok(t) = self.table_mut(s, &path) {
                if open {
                    if nth < t.controls.open.len() {
                        t.controls.open.remove(nth);
                        removed += 1;
                    }
                } else if t.controls.close > 0 {
                    t.controls.close -= 1;
                    removed += 1;
                }
            }
        }
        removed
    }

    /// The control markers in `a..b` whose partner is outside it (a deletion keeps them).
    pub(crate) fn kept_control_markers(&self, a: &Pos, b: &Pos) -> Vec<(Path, usize)> {
        let mut out = Vec::new();
        let paths = self.paths_between(a, b);
        let any = paths.iter().any(|p| {
            self.para(a.story, p).is_some_and(|q| q.objects.iter().any(|o| matches!(o, InlineObject::ControlStart { .. } | InlineObject::ControlEnd)))
        });
        if !any {
            return out;
        }
        let inside = |p: &Path, off: usize| (p, off) >= (&a.path, a.off) && (p, off + OBJ.len_utf8()) <= (&b.path, b.off);
        // Ranges whose markers are in paragraphs.
        let ranges = self.control_ranges(a.story);
        for r in &ranges {
            let (s, e) = (inside(&r.start.path, r.start.off), inside(&r.end.path, r.end.off));
            if s && !e {
                out.push((r.start.path.clone(), r.start.off));
            }
            if e && !s {
                out.push((r.end.path.clone(), r.end.off));
            }
        }
        // Markers paired with a table wrap: their partner is never deleted with them.
        for path in &paths {
            let Some(p) = self.para(a.story, path) else { continue };
            for off in p.object_offsets() {
                if !inside(path, off) || !matches!(p.object_at(off), Some(InlineObject::ControlStart { .. } | InlineObject::ControlEnd)) {
                    continue;
                }
                let paired = ranges.iter().any(|r| (r.start.path == *path && r.start.off == off) || (r.end.path == *path && r.end.off == off));
                if !paired && self.paired_with_table(a.story, path, off) {
                    out.push((path.clone(), off));
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Whether the marker at `path`/`off` is paired with a table wrap's open or close.
    fn paired_with_table(&self, s: StoryRef, path: &Path, off: usize) -> bool {
        let Some(bl) = self.container(s, path) else { return false };
        let events = control_events(bl);
        let (pairs, _) = pair_events(&events);
        let me = events.iter().position(|e| e.block() == path.last() && e.off() == Some(off));
        let Some(me) = me else { return false };
        pairs.iter().any(|(o, c, _)| {
            (*o == me && events.get(*c).is_some_and(|e| e.off().is_none())) || (*c == me && events.get(*o).is_some_and(|e| e.off().is_none()))
        })
    }

    /// Where typing at `p` should go: a caret at the start of a paragraph, before the start of
    /// a block-level control, goes inside it; one at the end, after a block-level control's
    /// end, goes inside it too (the control is around the whole paragraph).
    pub fn typing_pos(&self, p: &Pos) -> Pos {
        let Some(para) = self.para_at(p) else { return p.clone() };
        let is_start = |off: usize| matches!(para.object_at(off), Some(InlineObject::ControlStart { control }) if control.block);
        let is_end = |off: usize| matches!(para.object_at(off), Some(InlineObject::ControlEnd));
        let w = OBJ.len_utf8();
        let mut off = p.off;
        // Only block starts before the caret (or none) and block starts right after it.
        let before_ok = (0..off).step_by(w).all(&is_start) && off.is_multiple_of(w);
        if before_ok && is_start(off) {
            while is_start(off) {
                off += w;
            }
            return Pos { off, ..p.clone() };
        }
        if p.off == para.len() && off >= w && is_end(off - w) {
            // Step back over the trailing ends that close block-level controls.
            let ranges = self.control_ranges(p.story);
            while off >= w && is_end(off - w) && ranges.iter().any(|r| r.end.path == p.path && r.end.off == off - w && r.control.block) {
                off -= w;
            }
            return Pos { off, ..p.clone() };
        }
        p.clone()
    }
}

/// Every control in `bl` (and its tables), for walks that need them all.
pub fn walk_controls(bl: &Blocks, f: &mut dyn FnMut(&ContentControl)) {
    fn go(bl: &Blocks, f: &mut dyn FnMut(&ContentControl), depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for b in bl {
            match &**b {
                Block::Para(p) => {
                    for o in &p.objects {
                        if let InlineObject::ControlStart { control } = o {
                            f(control);
                        }
                    }
                }
                Block::Table(t) => {
                    t.controls.open.iter().for_each(&mut *f);
                    for r in &t.rows {
                        r.controls.open.iter().for_each(&mut *f);
                        for c in &r.cells {
                            c.controls.open.iter().for_each(&mut *f);
                            go(&c.blocks, f, depth + 1);
                        }
                    }
                }
            }
        }
    }
    go(bl, f, 0);
}
