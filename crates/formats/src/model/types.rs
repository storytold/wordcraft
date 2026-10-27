use super::*;

/// Font used for code spans and code blocks.
pub const MONO_FONT: &str = "Courier New";

/// Style id of code-block paragraphs.
pub const CODE_STYLE: &str = "Code";

/// Nesting limit for tables in tables (hostile input).
pub const MAX_DEPTH: usize = 8;

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
    StyleName,
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
        }
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
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String, Fmt),
    Image(Img),
    /// A bookmark (link target).
    Anchor(String),
    /// A chart or diagram known here only by its alt text: written where alt text has a place
    /// (an HTML or Markdown image), left out of editable formats so it never turns into body text.
    Figure(String),
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
}

impl Para {
    pub fn new(kind: Kind) -> Para {
        Para { kind, ..Default::default() }
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
                Inline::Text(t, _) | Inline::Equation { linear: t, .. } | Inline::Ruby { base: t, .. } => s.push_str(t),
                Inline::Image(_) | Inline::Anchor(_) | Inline::Figure(_) => {}
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
                Inline::Equation { linear: t, .. } | Inline::Ruby { base: t, .. } => s.push_str(t),
                Inline::Image(_) | Inline::Anchor(_) | Inline::Figure(_) => {}
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
}

impl Default for Cell {
    fn default() -> Self {
        Cell { blocks: Vec::new(), colspan: 1, rowspan: 1, covered: false, header: false, shading: None, borders: None, valign: None }
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
}

impl FTable {
    pub fn cols(&self) -> usize {
        self.rows.iter().map(|r| r.iter().map(|c| c.colspan.clamp(1, 63) as usize).sum::<usize>()).max().unwrap_or(0)
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
