use super::{Align, Arc, InlineObject, OBJ, REPORTED, Rgb, RubyAlign, VAlign, ruby_raise};

/// Font used for code spans and code blocks.
pub const MONO_FONT: &str = "Courier New";

/// Style id of code-block paragraphs.
pub const CODE_STYLE: &str = "Code";

/// Nesting limit for tables in tables (hostile input).
pub const MAX_DEPTH: usize = 8;

/// Deepest groups in groups kept (hostile files).
pub const MAX_GROUP_DEPTH: usize = 8;

/// Longest font name the importers keep, in bytes (as the .docx reader); a longer one (no real
/// font has one) is dropped (#3713).
pub const MAX_FONT_NAME: usize = 256;

/// Longest hyperlink target the importers keep, in bytes; a longer link is dropped and its text
/// kept (#3713).
pub const MAX_LINK: usize = 8 << 10;

/// Link and font bytes the document's runs may carry between them. A flow's spans share one
/// copy of a name ([`Fmt`]), but every run of a document owns its own: past this budget, runs
/// lose their link and font rather than multiply a small file into gigabytes (#3713).
pub(super) const MAX_RUN_STRINGS: usize = 64 << 20;

/// `name` as a span's font, unless it is longer than [`MAX_FONT_NAME`] (then reported, see
/// [`dropped`]).
pub fn kept_font(name: &str) -> Option<Arc<str>> {
    let kept = (name.len() <= MAX_FONT_NAME).then(|| name.into());
    if kept.is_none() {
        dropped(Dropped::Font);
    }
    kept
}

/// `target` as a span's link, unless it is longer than [`MAX_LINK`] (then reported, see
/// [`dropped`]).
pub fn kept_link(target: &str) -> Option<Arc<str>> {
    let kept = (target.len() <= MAX_LINK).then(|| target.into());
    if kept.is_none() {
        dropped(Dropped::Link);
    }
    kept
}

/// What an importer leaves out to stay within a limit (review of #3873: no limit drops data
/// without saying so).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dropped {
    /// A font name over [`MAX_FONT_NAME`].
    Font,
    /// A link target over [`MAX_LINK`].
    Link,
    /// Runs' links and fonts past [`MAX_RUN_STRINGS`].
    RunStrings,
    /// A style name cut to [`MAX_FONT_NAME`].
    #[allow(dead_code)]
    StyleName,
    /// ODT `text:s` spaces past the document's budget.
    Spaces,
    /// Markdown table rows and columns past the table limits or the document's cell budget.
    TableCells,
}

impl Dropped {
    fn message(self) -> String {
        match self {
            Dropped::Font => format!("import: font names longer than {MAX_FONT_NAME} bytes are dropped; that text keeps the surrounding font"),
            Dropped::Link => format!("import: links longer than {MAX_LINK} bytes are dropped; their text is kept"),
            Dropped::RunStrings => {
                format!("import: the document's runs carry more than {MAX_RUN_STRINGS} bytes of links and font names; later runs lose them")
            }
            Dropped::StyleName => format!("import: style names longer than {MAX_FONT_NAME} bytes are cut short"),
            Dropped::Spaces => {
                "import: the document's `text:s` elements ask for more spaces than it keeps; past that each stands for one".to_string()
            }
            Dropped::TableCells => format!(
                "import: Markdown tables keep {} columns and {} rows at most, and {} cells in all; the rest are left out",
                wordcraft_doc::table::MAX_COLS,
                wordcraft_doc::table::MAX_ROWS,
                crate::markdown::MAX_TABLE_CELLS
            ),
        }
    }

    /// Whether this thread's import has logged this kind ([`dropped`]).
    #[cfg(test)]
    pub(crate) fn reported(self) -> bool {
        REPORTED.with(|r| r.get() & (1u8 << (self as u8)) != 0)
    }
}

/// Log that the importer left something of `kind` out: once per kind per imported document
/// ([`begin_import`]), not once per item.
pub(crate) fn dropped(kind: Dropped) {
    let bit = 1u8 << (kind as u8);
    if REPORTED.with(|r| r.replace(r.get() | bit)) & bit == 0 {
        log::warn!("{}", kind.message());
    }
}

/// Start a new document's reports: each importer's entry point calls this first.
pub(crate) fn begin_import() {
    REPORTED.with(|r| r.set(0));
}

/// Character formatting the text formats can express.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fmt {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub code: bool,
    pub sup: bool,
    pub sub: bool,
    /// Shared: every span of a link holds the same target.
    pub link: Option<Arc<str>>,
    pub color: Option<Rgb>,
    /// Points.
    pub size: Option<f32>,
    /// Shared, as `link`.
    pub font: Option<Arc<str>>,
    /// Background (highlight / shading).
    pub background: Option<Rgb>,
    /// Tracked insertion of this text (formats that keep revisions: RTF).
    pub ins: Option<RevMark>,
    /// Tracked deletion of this text. Only [`from_doc_tracked`] keeps deleted text in a flow.
    pub del: Option<RevMark>,
    /// Tracked formatting change of this text: who made it and the formatting before it (only
    /// [`from_doc_tracked`] and formats that keep revisions set it).
    pub fmt_change: Option<Box<FmtChange<Fmt>>>,
    /// Formatting beyond the fields above, for formats that keep it (ODT): underline style and
    /// colour, double strike, caps, small caps, hidden, spacing, scale, position, outline,
    /// shadow, emboss, engrave and language, as character properties. Only flows asked for it
    /// ([`FlowOpts::chars`]) hold it.
    pub extra: Option<Box<wordcraft_doc::props::CharProps>>,
}

/// A tracked formatting change in a flow: who made it, and the formatting before it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FmtChange<T> {
    pub mark: RevMark,
    pub old: T,
}

/// Who made a tracked change and when (ISO 8601; empty = unknown).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RevMark {
    pub author: String,
    pub date: String,
    /// The source's identity of the change, when it has one (an ODT changed region; the
    /// revision index in [`from_doc_tracked`]): marks with different ids stay separate
    /// revisions. `None` (RTF) = one revision per kind, author and date.
    pub id: Option<u32>,
}

/// An embedded picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Img {
    pub data: Arc<Vec<u8>>,
    /// File extension (`png`, `jpeg`…).
    pub ext: String,
    /// Display size, points.
    pub w: f32,
    pub h: f32,
    pub alt: String,
    /// How a floating picture is placed and wrapped, for formats that keep it (ODT); `None`: in
    /// line with the text.
    pub float: Option<Box<wordcraft_doc::para::Float>>,
    /// The embedded object this picture shows, kept whole (ODT only; see [`OdfObject`]).
    pub object: Option<Arc<OdfObject>>,
    /// How much is cut off its left, top, right and bottom, fractions of its size (0..1), for
    /// formats that keep it (ODT).
    pub crop: [f32; 4],
}

/// An embedded object WordCraft shows but doesn't edit, kept whole for saving back (ODF 1.3
/// §10.4.6 `draw:object`, a sub-document such as a spreadsheet, or §10.4.7 `draw:object-ole`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OdfObject {
    /// Its media type (`application/vnd.oasis.opendocument.spreadsheet`…).
    pub mime: String,
    /// An OLE object: one binary file (`draw:object-ole`) rather than a sub-document folder.
    pub ole: bool,
    /// Its files, by their path in its folder (an OLE object's one file: "").
    pub files: Vec<(String, Arc<Vec<u8>>)>,
    /// The replacement picture as read (LibreOffice's own metafile format, say), written back
    /// in place of the picture shown.
    pub replacement: Option<Arc<Vec<u8>>>,
}

/// An editable chart in a flow ([`Inline::Chart`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    pub spec: Arc<wordcraft_doc::chart::ChartSpec>,
    /// Display size, points.
    pub w: f32,
    pub h: f32,
    pub alt: String,
    /// How a floating chart is placed and wrapped; `None`: in line with the text.
    pub float: Option<Box<wordcraft_doc::para::Float>>,
    /// The chart object as read (ODT), written back whole while the chart is unchanged, so it
    /// keeps the look WordCraft doesn't model (its fonts, lines and colours).
    pub source: Option<Arc<OdfObject>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String, Fmt),
    Image(Img),
    /// A bookmark (link target).
    Anchor(String),
    /// Where bookmark `name`, started by an [`Inline::Anchor`] before it, ends: the bookmark
    /// covers the text between. Only flows asked for fields ([`FlowOpts::fields`]) hold them;
    /// an anchor without one is a point.
    AnchorEnd(String),
    /// A chart or diagram known here only by its alt text: written where alt text has a place
    /// (an HTML or Markdown image), left out of editable formats so it never turns into body text.
    Figure(String),
    /// A chart WordCraft can edit, `w` × `h` points: only in flows asked for boxes
    /// ([`FlowOpts::boxes`]); others get its alt text as an [`Inline::Figure`].
    Chart(Chart),
    /// A content control holding plain `text` (a form control in ODT): only in flows asked for
    /// boxes ([`FlowOpts::boxes`]); others get its text.
    Control {
        control: Box<wordcraft_doc::control::ContentControl>,
        text: String,
        fmt: Fmt,
    },
    /// An equation in the linear format WordCraft keeps them in (`x=(-b±√(b^2-4ac))/2a`). Formats
    /// without equations write `linear` as text.
    Equation {
        linear: String,
        display: bool,
    },
    /// Ruby (phonetic guide): `ruby` text (furigana, pinyin…) over `base` text, in the formatting
    /// of the run it sits in (#1681). Formats without ruby write the base text
    /// ([`ruby_as_text`]).
    Ruby {
        base: String,
        ruby: String,
        fmt: Fmt,
    },
    /// A field (page number, date, document property, cross-reference): its Word field code
    /// (`PAGE`, `REF name \h`…) and the result it last showed, in its run's formatting. Only
    /// flows asked for fields ([`FlowOpts::fields`]) hold them; formats without fields write the
    /// result as text.
    Field {
        instr: String,
        result: String,
        fmt: Fmt,
        /// Its result is kept, not worked out again (Word's locked field, ODF's `text:fixed`).
        locked: bool,
    },
    /// A footnote or endnote: its custom mark (empty: numbered) and its content, without the
    /// note's own mark. Only flows asked for notes ([`FlowOpts::notes`]) hold them; formats
    /// without notes leave them out.
    Note {
        endnote: bool,
        custom: String,
        blocks: Vec<FBlock>,
        /// The reference's own formatting (beyond the note reference style).
        fmt: Fmt,
    },
    /// Where comment `id` (the flow's own number for it) starts, and the comment. A reply
    /// ([`FComment::parent`]) has no range: it only follows its comment. Only flows asked for
    /// comments ([`FlowOpts::comments`]) hold them.
    Comment {
        id: u32,
        comment: Box<FComment>,
    },
    /// Where comment `id`'s range ends.
    CommentEnd(u32),
    /// A drawing shape or text box: its kind, its size in points, its placement when it floats,
    /// its text (none for a shape without), and its look. Only flows asked for them
    /// ([`FlowOpts::boxes`]) hold them.
    Shape {
        kind: wordcraft_doc::para::ShapeKind,
        w: f32,
        h: f32,
        float: Option<Box<wordcraft_doc::para::Float>>,
        blocks: Vec<FBlock>,
        look: BoxLook,
        /// A freeform's paths ([`wordcraft_doc::para::ShapeKind::Freeform`]).
        path: Option<std::sync::Arc<wordcraft_doc::freeform::Freeform>>,
    },
    /// The start of a field whose result is the content after it, up to the matching
    /// [`Inline::FieldEnd`] (a table of contents around its entries); its field code. Only flows
    /// asked for fields ([`FlowOpts::fields`]) hold them.
    FieldStart(String),
    /// The end of the innermost [`Inline::FieldStart`].
    FieldEnd,
    /// Drawing shapes, text boxes and pictures grouped into one object: its size in points, its
    /// placement when it floats, and its members, each at its offset in the group. Only flows
    /// asked for shapes ([`FlowOpts::boxes`]) hold them.
    Group {
        w: f32,
        h: f32,
        float: Option<Box<wordcraft_doc::para::Float>>,
        members: Vec<Member>,
    },
}

/// A member of an [`Inline::Group`]: a shape, text box or picture (its placement in the group is
/// `x`, `y`, points from the group's top left corner; its own float holds only its turn and
/// mirroring).
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub x: f32,
    pub y: f32,
    pub inline: Inline,
}

/// How a text box or shape looks: its fill, its outline (colour and width, dash and
/// arrowheads), its inner margins, and whether it grows to fit its text.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxLook {
    /// Fill: a colour or a gradient; `None` is no fill.
    pub fill: Option<wordcraft_doc::fill::Fill>,
    /// Outline colour and width in points; `None` is no line.
    pub line: Option<(Rgb, f32)>,
    /// The outline's dash pattern.
    pub dash: wordcraft_doc::line_dash::LineDash,
    /// Arrowheads at a line's start and end.
    pub arrows: (bool, bool),
    /// Inner margins (left, top, right, bottom) in points; `None` is the default.
    pub inset: Option<[f32; 4]>,
    /// The box grows to its text; its height is the least it is.
    pub auto_fit: bool,
    /// The drawing id connectors glue to, and a connector's glued ends (see
    /// [`wordcraft_doc::connector`]).
    pub conn: Option<Box<Conn>>,
    /// A text box's name and the box its text continues in when it doesn't fit (linked text
    /// boxes), by name.
    pub chain: Option<Box<Chain>>,
    /// Which way a text box's text runs (turned down or up).
    pub vert: wordcraft_doc::props::TextVert,
    /// A custom shape's geometry as read (ODT; see `ShapeExtra::odf_geometry`).
    pub geometry: Option<std::sync::Arc<String>>,
}

/// A text box's name, and the name of the box its text continues in ([`BoxLook::chain`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chain {
    pub name: String,
    pub next: Option<String>,
}

/// A shape's drawing id (0: none) and, for a connector, what its ends are glued to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Conn {
    pub id: u32,
    pub start: Option<wordcraft_doc::connector::ConnEnd>,
    pub end: Option<wordcraft_doc::connector::ConnEnd>,
}

impl Default for BoxLook {
    /// A new text box's: white, with a thin black line.
    fn default() -> BoxLook {
        BoxLook {
            fill: Some(wordcraft_doc::fill::Fill::solid(Rgb::WHITE)),
            line: Some((Rgb::BLACK, 0.75)),
            dash: Default::default(),
            arrows: (false, false),
            inset: None,
            auto_fit: false,
            conn: None,
            chain: None,
            vert: Default::default(),
            geometry: None,
        }
    }
}

/// The bookmarks in `blocks` whose end ([`Inline::AnchorEnd`]) follows their start
/// ([`Inline::Anchor`]): ranges, not points. Tables' cells, notes, comments and shapes' text
/// are looked through, in document order.
pub fn ranged_anchors(blocks: &[FBlock]) -> std::collections::HashSet<String> {
    fn walk(blocks: &[FBlock], depth: usize, started: &mut std::collections::HashSet<String>, ranged: &mut std::collections::HashSet<String>) {
        if depth > MAX_DEPTH {
            return;
        }
        for b in blocks {
            match b {
                FBlock::Para(p) => {
                    for i in &p.inlines {
                        match i {
                            Inline::Anchor(n) => {
                                started.insert(n.clone());
                            }
                            Inline::AnchorEnd(n) if started.contains(n) => {
                                ranged.insert(n.clone());
                            }
                            Inline::Note { blocks, .. } | Inline::Shape { blocks, .. } => walk(blocks, depth + 1, started, ranged),
                            Inline::Comment { comment, .. } => walk(&comment.blocks, depth + 1, started, ranged),
                            _ => {}
                        }
                    }
                }
                FBlock::Table(t) => {
                    for c in t.rows.iter().flatten() {
                        walk(&c.blocks, depth + 1, started, ranged);
                    }
                }
            }
        }
    }
    let (mut started, mut ranged) = Default::default();
    walk(blocks, 0, &mut started, &mut ranged);
    ranged
}

/// `blocks` with each table replaced by the blocks of its cells, row by row (nested tables too),
/// for places that hold paragraphs only.
pub fn flatten_tables(blocks: &[FBlock]) -> Vec<FBlock> {
    let mut out = Vec::new();
    let mut stack: Vec<std::slice::Iter<'_, FBlock>> = vec![blocks.iter()];
    while let Some(top) = stack.last_mut() {
        match top.next() {
            Some(FBlock::Para(p)) => out.push(FBlock::Para(p.clone())),
            Some(FBlock::Table(t)) => {
                // Cells in reverse so they come off the stack in order.
                for c in t.rows.iter().rev().flat_map(|r| r.iter().rev()) {
                    stack.push(c.blocks.iter());
                }
            }
            None => {
                stack.pop();
            }
        }
    }
    out
}

/// A comment in a flow ([`Inline::Comment`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FComment {
    pub author: String,
    pub initials: String,
    /// ISO 8601; empty = unknown.
    pub date: String,
    pub resolved: bool,
    /// The comment this one answers (its flow id).
    pub parent: Option<u32>,
    pub blocks: Vec<FBlock>,
}

/// A field that never shows its result: `ASK` and `SET` only assign text to a bookmark
/// (ECMA-376 §17.16.5.4, §17.16.5.60), as layout shows them. Its stored result stays in the
/// flow (ODT writes it back); text exports write nothing for it.
pub fn shows_nothing(instr: &str) -> bool {
    let name = instr.split_whitespace().next().unwrap_or("");
    name.eq_ignore_ascii_case("ASK") || name.eq_ignore_ascii_case("SET")
}

/// What [`super::from_doc_with`] keeps besides text and formatting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlowOpts {
    /// Tracked changes: deleted text stays, and text carries its insertion and deletion marks.
    pub tracked: bool,
    /// Fields as [`Inline::Field`] (otherwise their result is text).
    pub fields: bool,
    /// Footnotes and endnotes as [`Inline::Note`] (otherwise left out).
    pub notes: bool,
    /// Comments as [`Inline::Comment`] and [`Inline::CommentEnd`] (otherwise left out).
    pub comments: bool,
    /// Text boxes and drawing shapes as [`Inline::Shape`] (otherwise left out).
    pub boxes: bool,
    /// Character formatting beyond the [`Fmt`] fields ([`Fmt::extra`]), hidden text kept (as
    /// hidden), and a monospaced font by its name rather than as code.
    pub chars: bool,
    /// Paragraph formatting beyond the [`Para`] fields ([`Para::extra`]).
    pub paras: bool,
    /// Lists with their identity and labels ([`ListInfo::list`], [`ListInfo::label`]; otherwise
    /// consecutive items form one list in the default look of its kind).
    pub lists: bool,
}

/// Base text size (points) a phonetic guide falls back to when the source states none: the
/// Phonetic Guide command's own fallback for an unresolved size
/// (`crates/engine/src/cmd/phonetic.rs`).
pub const RUBY_BASE_SIZE: f32 = 10.5;

/// Ruby text size for a base of `base_size` points: half the base size, rounded down to a half
/// point, as the Phonetic Guide command sets it.
fn ruby_default_size(base_size: f32) -> f32 {
    (base_size.floor() / 2.0).max(1.0)
}

/// The ruby a format without its own ruby markup gets: a [`InlineObject::Ruby`] over `base` with
/// the Phonetic Guide's defaults (centred, half the base size, raised clear of the base text).
pub fn ruby_object(base: String, ruby: String, base_size: f32) -> InlineObject {
    let base_size = if base_size.is_finite() { base_size.clamp(1.0, 1584.0) } else { RUBY_BASE_SIZE };
    let size = ruby_default_size(base_size);
    let raise = ruby_raise(0.0, base_size, size).clamp(0.0, 1584.0);
    InlineObject::Ruby { base, ruby, align: RubyAlign::default(), size, raise, base_size, lang: String::new(), props: Default::default() }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Normal,
    /// 1..=6.
    Heading(u8),
    Title,
    Quote,
    /// One line of a code block.
    Code,
    /// A horizontal rule (empty paragraph with a bottom border).
    Rule,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListInfo {
    pub ordered: bool,
    /// 0-based nesting level (0..=8).
    pub level: u8,
    /// Number of the first item of an ordered list (1 unless the source says otherwise).
    pub start: u32,
    /// The list the paragraph belongs to, when the source tells its lists apart (ODT): items
    /// of one list share its number, wherever they are, and a different number starts another
    /// list. 0: not stated (consecutive items form one list).
    pub list: u32,
    /// How the paragraph's level shows its number or bullet, when the source says.
    pub label: Option<Label>,
}

impl Default for ListInfo {
    fn default() -> Self {
        ListInfo { ordered: false, level: 0, start: 1, list: 0, label: None }
    }
}

/// A list label's prefix or suffix (`(`, `.`, `)`, `Chapter `), at most [`Affix::MAX`] bytes
/// of UTF-8 (kept inline so list paragraphs stay small).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Affix {
    bytes: [u8; Affix::MAX],
    len: u8,
}

impl Affix {
    pub const MAX: usize = 8;

    /// The characters of `s` that fit in [`Affix::MAX`] bytes, without control characters and
    /// `%` (which would read as a number placeholder).
    pub fn new(s: &str) -> Affix {
        let mut a = Affix::default();
        for c in s.chars().filter(|c| !c.is_control() && *c != '%') {
            let mut buf = [0u8; 4];
            let enc = c.encode_utf8(&mut buf).as_bytes();
            let at = a.len as usize;
            let Some(slot) = a.bytes.get_mut(at..at + enc.len()) else { break };
            slot.copy_from_slice(enc);
            a.len += enc.len() as u8;
        }
        a
    }

    pub fn as_str(&self) -> &str {
        self.bytes.get(..self.len as usize).and_then(|b| std::str::from_utf8(b).ok()).unwrap_or("")
    }
}

/// How a list level shows its label: number format, text around the number and the levels
/// shown (`1.2.`), or, for a bulleted level, its bullet (as the prefix).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label {
    pub format: wordcraft_doc::section::NumFormat,
    pub prefix: Affix,
    pub suffix: Affix,
    /// Levels shown, this one and those above it (1: only its own number).
    pub shown: u8,
    /// Where the level's text starts and how far its label hangs left of it, in twentieths of a
    /// point, and how the label sits in that room, when the source says.
    pub indent: Option<i16>,
    pub hanging: Option<i16>,
    pub align: Option<super::Align>,
}

impl Label {
    /// A bulleted level's label: `bullet`, or the default bullet.
    pub fn bulleted(bullet: Option<char>) -> Label {
        let mut b = [0u8; 4];
        let prefix = bullet.map(|c| Affix::new(c.encode_utf8(&mut b))).unwrap_or_default();
        Label {
            format: wordcraft_doc::section::NumFormat::Bullet,
            prefix,
            suffix: Affix::default(),
            shown: 1,
            indent: None,
            hanging: None,
            align: None,
        }
    }

    /// Points as twentieths of a point, on the page.
    pub fn twips(pt: f32) -> Option<i16> {
        pt.is_finite().then(|| (pt.clamp(-1584.0, 1584.0) * 20.0).round() as i16)
    }

    /// This label placed `indent` points in with its label `hanging` points left of the text,
    /// aligned `align`.
    pub fn placed(mut self, indent: f32, hanging: f32, align: super::Align) -> Label {
        (self.indent, self.hanging, self.align) = (Label::twips(indent), Label::twips(hanging), Some(align));
        self
    }

    /// The indent and hanging, in points, when stated.
    pub fn position(&self) -> Option<(f32, f32)> {
        Some((f32::from(self.indent?) / 20.0, f32::from(self.hanging.unwrap_or(0)) / 20.0))
    }

    /// The bullet of a bulleted level, when it names one.
    pub fn bullet(&self) -> Option<char> {
        (self.format == wordcraft_doc::section::NumFormat::Bullet).then(|| self.prefix.as_str().chars().next()).flatten()
    }

    /// Word's level text for 0-based `level`: `%1.`, `(%1)`, `%1.%2.`, or the bullet.
    pub fn text(&self, level: u8) -> String {
        let level = level.min(8);
        if self.format == wordcraft_doc::section::NumFormat::Bullet {
            return self.bullet().unwrap_or('•').to_string();
        }
        let shown = self.shown.clamp(1, level + 1);
        let numbers: Vec<String> = (level + 2 - shown..=level + 1).map(|n| format!("%{n}")).collect();
        format!("{}{}{}", self.prefix.as_str(), numbers.join("."), self.suffix.as_str())
    }

    /// The label a Word level text (`%1.`, `(%1)`, `%1.%2.`, a bullet) shows.
    pub fn of_level(format: wordcraft_doc::section::NumFormat, text: &str) -> Label {
        if format == wordcraft_doc::section::NumFormat::Bullet {
            return Label::bulleted(text.chars().next());
        }
        let first = text.find('%').unwrap_or(text.len());
        // The suffix follows the last placeholder's digit.
        let last =
            text.rfind('%').map(|i| i + text.get(i + 1..).and_then(|r| r.chars().next()).map_or(1, |c| 1 + c.len_utf8())).unwrap_or(text.len());
        let shown = text.matches('%').count().clamp(1, 9) as u8;
        Label {
            format,
            prefix: Affix::new(text.get(..first).unwrap_or("")),
            suffix: Affix::new(text.get(last.min(text.len())..).unwrap_or("")),
            shown,
            indent: None,
            hanging: None,
            align: None,
        }
    }
}

/// Paragraph spacing and indents (points) that a source states on the paragraph itself (RTF);
/// left out, they come from the style.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spacing {
    pub before: Option<f32>,
    pub after: Option<f32>,
    pub line: Option<wordcraft_doc::props::LineSpacing>,
    pub left: Option<f32>,
    pub right: Option<f32>,
    /// First-line indent; negative = hanging.
    pub first: Option<f32>,
    /// HTML's automatic space before / after (see `ParaProps::space_before_auto`).
    pub before_auto: bool,
    pub after_auto: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Para {
    pub kind: Kind,
    pub list: Option<ListInfo>,
    pub align: Option<Align>,
    pub page_break: bool,
    /// Keep the paragraph on the same page as the next one (ODT `fo:keep-with-next`).
    pub keep_next: bool,
    pub inlines: Vec<Inline>,
    pub spacing: Spacing,
    /// Borders the source puts on the paragraph itself (RTF `\brdrb`…).
    pub borders: Option<wordcraft_doc::props::Borders>,
    /// The section this paragraph ends (RTF `\sect`), as the document's last paragraph of it.
    pub section: Option<Box<wordcraft_doc::SectionProps>>,
    /// Tracked insertion of the paragraph mark.
    pub mark_ins: Option<Box<RevMark>>,
    /// Tracked deletion of the paragraph mark (only [`from_doc_tracked`] and formats that keep
    /// revisions set it).
    pub mark_del: Option<Box<RevMark>>,
    /// Tracked formatting change of the paragraph: the paragraph's formatting before it (kind,
    /// list, alignment, page break, spacing and borders of `old`; its inlines are unused).
    pub fmt_change: Option<Box<FmtChange<Para>>>,
    /// Paragraph formatting beyond the fields above, for formats that keep it (ODT): its own
    /// tab stops. Only flows asked for it ([`FlowOpts::paras`]) hold it.
    pub extra: Option<Box<wordcraft_doc::props::ParaProps>>,
    /// The paragraph mark's direct formatting (its size sets an empty paragraph's height), for
    /// formats that keep it (ODT): only flows asked for paragraph formatting
    /// ([`FlowOpts::paras`]) hold it, and only for paragraphs without text ([`Para::textless`]),
    /// whose drawings' runs take it too. A reader whose source gives it sets it too (HTML's
    /// `<p>&nbsp;</p>`).
    pub mark: Option<Box<wordcraft_doc::props::CharProps>>,
    /// The paragraph's markup in the file it came from, while it still is what was read
    /// ([`wordcraft_doc::kept`]): a format that keeps it (ODT) writes it as it was.
    pub kept: Option<std::sync::Arc<str>>,
    /// Sections kept as read around paragraphs ([`wordcraft_doc::kept`]): the start tags of
    /// those opening before this paragraph (outermost first), and how many close after it.
    pub wrap: Option<Box<(Vec<std::sync::Arc<str>>, u32)>>,
    /// An index (a table of contents…) kept as read that starts with this paragraph
    /// ([`wordcraft_doc::kept`]): its markup, and how many blocks of the flow it takes.
    pub index: Option<Box<(std::sync::Arc<str>, usize)>>,
    /// The group of paragraphs its markup kept as read belongs to (see [`Group`]).
    pub group: Option<Box<Group>>,
}

/// Paragraphs whose markup kept as read ties them together (a comment from one to another, a
/// tracked change, a text box continuing in another's): written as read all, or none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    /// While reading: what ties it to others, each `(key, needed)`: a key it needs another
    /// paragraph to have (`needed`), or one it has for another to need.
    pub links: Vec<(String, bool)>,
    /// The group's id and how many paragraphs it has, once known.
    pub id: Option<(u32, u32)>,
}

impl Para {
    pub fn new(kind: Kind) -> Para {
        Para { kind, ..Default::default() }
    }

    /// It holds no text of its own: nothing, or only drawings, bookmarks and field bounds. Its
    /// [`Para::mark`] formatting is then its own (its height, its drawings' runs).
    pub fn textless(&self) -> bool {
        self.inlines.iter().all(|i| {
            matches!(
                i,
                Inline::Image(_)
                    | Inline::Chart(_)
                    | Inline::Shape { .. }
                    | Inline::Group { .. }
                    | Inline::Figure(_)
                    | Inline::Anchor(_)
                    | Inline::AnchorEnd(_)
                    | Inline::CommentEnd(_)
                    | Inline::FieldStart(_)
                    | Inline::FieldEnd
            )
        })
    }
    /// Append text, merging with the previous span when the formatting matches.
    pub fn push_text(&mut self, s: &str, f: &Fmt) {
        if s.is_empty() {
            return;
        }
        if let Some(Inline::Text(t, pf)) = self.inlines.last_mut()
            && pf == f
        {
            t.push_str(s);
            return;
        }
        self.inlines.push(Inline::Text(s.to_string(), f.clone()));
    }
    /// The plain text (images and anchors left out; equations as their linear text, ruby as its
    /// base text).
    pub fn text(&self) -> String {
        let mut s = String::new();
        for i in &self.inlines {
            match i {
                Inline::Text(t, _)
                | Inline::Equation { linear: t, .. }
                | Inline::Ruby { base: t, .. }
                | Inline::Field { result: t, .. }
                | Inline::Control { text: t, .. } => s.push_str(t),
                Inline::Image(_)
                | Inline::Anchor(_)
                | Inline::Figure(_)
                | Inline::Chart(_)
                | Inline::Note { .. }
                | Inline::Comment { .. }
                | Inline::CommentEnd(_)
                | Inline::Shape { .. }
                | Inline::Group { .. }
                | Inline::FieldStart(_)
                | Inline::FieldEnd
                | Inline::AnchorEnd(_) => {}
            }
        }
        s
    }
    /// [`Self::text`] as it reads on screen: text set in a symbol font (Symbol, Wingdings…) as
    /// its Unicode equivalents ([`wordcraft_doc::symbol_fonts`]), for plain-text export.
    pub fn shown_text(&self) -> String {
        let mut s = String::new();
        for i in &self.inlines {
            match i {
                Inline::Text(t, f) => match f.font.as_deref() {
                    Some(font) if wordcraft_doc::symbol_fonts::is_symbol_font(font) => s.push_str(&wordcraft_doc::symbol_fonts::map_text(font, t)),
                    _ => s.push_str(t),
                },
                Inline::Field { instr, .. } if shows_nothing(instr) => {}
                Inline::Equation { linear: t, .. }
                | Inline::Ruby { base: t, .. }
                | Inline::Field { result: t, .. }
                | Inline::Control { text: t, .. } => s.push_str(t),
                Inline::Image(_)
                | Inline::Anchor(_)
                | Inline::Figure(_)
                | Inline::Chart(_)
                | Inline::Note { .. }
                | Inline::Comment { .. }
                | Inline::CommentEnd(_)
                | Inline::Shape { .. }
                | Inline::Group { .. }
                | Inline::FieldStart(_)
                | Inline::FieldEnd
                | Inline::AnchorEnd(_) => {}
            }
        }
        s
    }
    pub fn is_empty(&self) -> bool {
        self.inlines.iter().all(|i| matches!(i, Inline::Text(t, _) if t.is_empty()))
    }

    /// Remove whitespace at the very start and end of the paragraph's text.
    pub fn trim(&mut self) {
        const WS: &[char] = &[' ', '\t', '\r', '\n'];
        self.trim_with(WS, WS);
    }

    /// Remove the characters in `start` from the start of the paragraph's text and those in
    /// `end` from its end.
    pub fn trim_with(&mut self, start: &[char], end: &[char]) {
        // The leading spans that trim to nothing go in one `drain`: removing them one at a time
        // shifted the rest each time, quadratic in a run of separately formatted line breaks.
        let k = self.inlines.iter().take_while(|i| matches!(i, Inline::Text(t, _) if t.trim_start_matches(start).is_empty())).count();
        self.inlines.drain(..k);
        if let Some(Inline::Text(t, _)) = self.inlines.first_mut() {
            let cut = t.len() - t.trim_start_matches(start).len();
            t.replace_range(..cut, "");
        }
        while let Some(Inline::Text(t, _)) = self.inlines.last_mut() {
            let n = t.trim_end_matches(end).len();
            t.truncate(n);
            if t.is_empty() {
                self.inlines.pop();
            } else {
                break;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub blocks: Vec<FBlock>,
    pub colspan: u32,
    /// Rows this cell spans (export information; `covered` cells below carry the merge).
    pub rowspan: u32,
    /// Continuation of a vertical merge from the cell above.
    pub covered: bool,
    pub header: bool,
    pub shading: Option<Rgb>,
    /// The cell's own borders (RTF `\clbrdr…`), drawn whatever the table's style.
    pub borders: Option<wordcraft_doc::props::Borders>,
    pub valign: Option<VAlign>,
    /// The cell's padding (top, left, bottom, right), points, when the source gives it.
    pub margins: Option<[f32; 4]>,
    /// Which way its text runs (turned down or up).
    pub direction: wordcraft_doc::props::TextDirection,
    /// A picture behind its content (ODT).
    pub picture: Option<Box<CellPic>>,
    /// The width, points, the source gives a cell spanning one column (HTML): see
    /// [`FTable::widths_from_cells`].
    pub width: Option<f32>,
}

/// A cell's background picture: the picture at its own size, how it fills the cell, and where an
/// unrepeated one sits (see [`wordcraft_doc::props::CellPicture`]).
#[derive(Clone, Debug, PartialEq)]
pub struct CellPic {
    pub img: Img,
    pub repeat: wordcraft_doc::props::PictureRepeat,
    pub align: [f32; 2],
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            blocks: Vec::new(),
            colspan: 1,
            rowspan: 1,
            covered: false,
            header: false,
            shading: None,
            borders: None,
            valign: None,
            margins: None,
            direction: Default::default(),
            picture: None,
            width: None,
        }
    }
}

impl Cell {
    /// Text of the cell: paragraphs joined by `\n`.
    pub fn text(&self) -> String {
        let mut v = Vec::new();
        for b in &self.blocks {
            match b {
                FBlock::Para(p) => v.push(p.text()),
                FBlock::Table(t) => v.push(t.rows.iter().flat_map(|r| r.iter().map(Cell::text)).collect::<Vec<_>>().join(" ")),
            }
        }
        v.join("\n")
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FTable {
    pub rows: Vec<Vec<Cell>>,
    /// Grid column widths in points (empty = equal widths).
    pub widths: Vec<f32>,
    /// The source table had no borders (HTML default); the document table gets no grid style.
    pub borderless: bool,
    /// Its alignment, width and row heights, for formats that keep them (ODT).
    pub extra: Option<Box<TableExtra>>,
    /// The table's markup in the file it came from, while it still is what was read
    /// ([`wordcraft_doc::kept`]): a format that keeps it (ODT) writes it as it was.
    pub kept: Option<std::sync::Arc<str>>,
    /// Wrappers kept as read around it (sections): the start tags opening before it and how
    /// many close after it, as a paragraph's (`Para::wrap`).
    pub wrap: Option<Box<(Vec<std::sync::Arc<str>>, u32)>>,
}

/// A table's placement and row heights ([`FTable::extra`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableExtra {
    pub align: Option<wordcraft_doc::Align>,
    /// Width, points.
    pub width: Option<f32>,
    /// Width, percent of the text width.
    pub width_pct: Option<f32>,
    /// Each row's height (points) and whether it is exact (else at least), by row.
    pub heights: Vec<Option<(f32, bool)>>,
    /// Whether each row keeps from breaking across pages, by row.
    pub keep_rows: Vec<bool>,
    /// Its columns keep their widths, never fitted to their content (Word's fixed layout,
    /// `w:tblLayout`): an ODT table's always do.
    pub fixed: bool,
    /// Its own space above and below, points (ODT's table margins).
    pub space: [Option<f32>; 2],
    /// How many rows at its top repeat at the top of each page (Word's Repeat as header row;
    /// ODT's `table:table-header-rows`). Its header cells ([`FCell::header`]) may be more: a
    /// table style's header row is one too.
    pub header_rows: usize,
}

impl FTable {
    pub fn cols(&self) -> usize {
        self.rows.iter().map(|r| r.iter().map(|c| c.colspan.clamp(1, 63) as usize).sum::<usize>()).max().unwrap_or(0)
    }
    /// Grid column widths from the cells that give one (after [`Self::insert_covered`], so a
    /// cell's column counts those spanned from rows above), when every column has one.
    pub fn widths_from_cells(&mut self) {
        let mut widths = vec![None; self.cols()];
        for row in &self.rows {
            let mut g = 0;
            for c in row {
                if let (Some(w), Some(slot)) = (c.width, widths.get_mut(g)) {
                    slot.get_or_insert(w);
                }
                g += c.colspan.clamp(1, 63) as usize;
            }
        }
        self.widths = widths.into_iter().collect::<Option<Vec<f32>>>().unwrap_or_default();
    }

    /// After an importer that gives `rowspan` but no placeholder cells (HTML): insert covered
    /// cells into the rows below each spanning cell.
    pub fn insert_covered(&mut self) {
        // Each spanning cell touches up to `rowspan - 1` rows below it; a table of many rows each
        // carrying a large rowspan would otherwise be O(rows × rowspan) covered cells, each of
        // which becomes a document cell downstream. Cap the total touched so a small file can't
        // multiply into a huge table, whatever the rowspans claim.
        const MAX_COVERED_WORK: usize = 1 << 17;
        let mut work = 0usize;
        let nrows = self.rows.len();
        for r in 0..nrows {
            let mut g = 0usize;
            let mut ci = 0usize;
            while let Some(c) = self.rows.get(r).and_then(|row| row.get(ci)) {
                let (span, rs, covered) = (c.colspan.clamp(1, 63) as usize, c.rowspan.clamp(1, 1000) as usize, c.covered);
                if rs > 1 && !covered {
                    for k in 1..rs {
                        work += 1;
                        if work > MAX_COVERED_WORK {
                            return;
                        }
                        let Some(row) = self.rows.get_mut(r + k) else { break };
                        // Find the index in that row at grid column `g`.
                        let mut x = 0usize;
                        let mut at = row.len();
                        for (i, cc) in row.iter().enumerate() {
                            if x >= g {
                                at = i;
                                break;
                            }
                            x += cc.colspan.clamp(1, 63) as usize;
                        }
                        if row.len() < 64 {
                            row.insert(at, Cell { colspan: span as u32, covered: true, ..Default::default() });
                        }
                    }
                }
                g += span;
                ci += 1;
            }
        }
    }
}

/// A block of a flow. (Most are paragraphs: boxing them for the tables' sake would cost every
/// paragraph an allocation, so the size difference is fine.)
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum FBlock {
    Para(Para),
    Table(FTable),
}

/// Document properties carried by the text formats.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
    pub description: String,
    pub created: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flow {
    pub blocks: Vec<FBlock>,
    pub meta: Meta,
}

/// `inlines` with equations as their linear text (default formatting), for formats that have no
/// equations.
pub fn equations_as_text(inlines: &[Inline]) -> std::borrow::Cow<'_, [Inline]> {
    if !inlines.iter().any(|i| matches!(i, Inline::Equation { .. })) {
        return std::borrow::Cow::Borrowed(inlines);
    }
    let mut p = Para::default();
    for i in inlines {
        match i {
            Inline::Equation { linear, .. } => p.push_text(linear, &Fmt::default()),
            Inline::Text(t, f) => p.push_text(t, f),
            other => p.inlines.push(other.clone()),
        }
    }
    std::borrow::Cow::Owned(p.inlines)
}

/// `inlines` with ruby as its base text, for formats that have no ruby markup: the base text (and
/// everything around it) keeps its formatting, and the reading is left out.
pub fn ruby_as_text(inlines: &[Inline]) -> std::borrow::Cow<'_, [Inline]> {
    if !inlines.iter().any(|i| matches!(i, Inline::Ruby { .. })) {
        return std::borrow::Cow::Borrowed(inlines);
    }
    let mut p = Para::default();
    for i in inlines {
        match i {
            Inline::Ruby { base, fmt, .. } => p.push_text(base, fmt),
            other => p.inlines.push(other.clone()),
        }
    }
    std::borrow::Cow::Owned(p.inlines)
}

// ---------------------------------------------------------------------------------------------
// Flow → Document

/// Text with characters the model reserves (U+FFFC) and stray controls removed.
pub fn clean_text(s: &str) -> String {
    s.chars().filter(|c| *c != OBJ && (!c.is_control() || matches!(c, '\t' | '\n' | '\u{000C}' | '\u{000E}'))).collect()
}
