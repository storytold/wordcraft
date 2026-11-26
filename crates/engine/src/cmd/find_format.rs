//! Find and Replace by formatting (#6147): Word's Find what › Format and Replace with › Format.
//!
//! `edit.find`, `edit.advancedFind`, `edit.findNext`, `edit.findPrevious`, `edit.replace` and
//! `edit.replaceAll` take a `format` filter. With text, a match counts only when every character
//! of it has the formatting; with empty `text`, the matches are the stretches of text that have it
//! (a paragraph mark ends a stretch, and so does text marked deleted by Track Changes). The
//! formatting a character has is what it shows, so a run that is bold because its style says so
//! matches `{"bold": true}`, as in Word.
//!
//! `withFormat` is what Replace with › Format sets: formatting applied to each replaced match.
//! When it is given and the replacement text is empty, the found text stays and only the
//! formatting changes (as in Word); otherwise the text is replaced and the new text is formatted.
//!
//! `format` is kept in the find state, like the other search options, until `"format": null` or
//! `{}` (Word's No Formatting button) clears it. `withFormat` belongs to one call: it is not kept,
//! so a later plain Replace All with empty `with` still deletes the matches. The Replace with
//! text is kept like the other options: a call with only `withFormat` (no `with`) still replaces
//! with the text of the last search; `"with": ""` makes it a format-only call.

use serde_json::Value;
use wordcraft_doc::props::{CharProps, Highlight, Underline, VertAlign};
use wordcraft_doc::styles::PartSet;
use wordcraft_doc::{Block, Path, Pos, StoryRef, StyleKind, StyleSheet, TextColor};

use super::edit::Found;
use crate::{CmdError, Session};

/// The most matches a format-only search returns, as for a text search.
const MAX_MATCHES: usize = 100_000;

/// The keys of `format` and `withFormat`.
const KEYS: [&str; 14] = [
    "bold",
    "italic",
    "underline",
    "strike",
    "smallCaps",
    "allCaps",
    "hidden",
    "subscript",
    "superscript",
    "font",
    "size",
    "color",
    "highlight",
    "style",
];

/// A highlight colour to look for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HighlightMatch {
    /// Any highlight colour.
    Any,
    /// This colour; `Highlight::None` finds text that is not highlighted.
    Is(Highlight),
}

/// A style to look for: a paragraph style (the paragraph has it) or a character style (the run has it).
#[derive(Clone, Debug, PartialEq)]
pub struct StyleFilter {
    pub id: String,
    pub paragraph: bool,
}

/// Find what › Format: the formatting a match must have. A field left `None` is not looked at.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormatFilter {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    /// Any underline (`true`) or none (`false`).
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub small_caps: Option<bool>,
    pub all_caps: Option<bool>,
    pub hidden: Option<bool>,
    pub subscript: Option<bool>,
    pub superscript: Option<bool>,
    pub font: Option<String>,
    pub size: Option<f32>,
    pub color: Option<TextColor>,
    pub highlight: Option<HighlightMatch>,
    pub style: Option<StyleFilter>,
}

/// Replace with › Format: what to set on each replaced match.
#[derive(Clone, Debug, Default)]
pub struct FormatApply {
    /// Character formatting to set (fields left `None` stay as they are).
    pub props: CharProps,
    /// A paragraph style to give the paragraphs of the match.
    pub para_style: Option<String>,
}

impl FormatFilter {
    fn is_empty(&self) -> bool {
        *self == FormatFilter::default()
    }

    /// Whether anything besides the style is asked for, so the run's formatting must be resolved.
    pub(crate) fn needs_resolved(&self) -> bool {
        [self.bold, self.italic, self.underline, self.strike, self.small_caps, self.all_caps, self.hidden, self.subscript, self.superscript]
            .iter()
            .any(Option::is_some)
            || self.font.is_some()
            || self.size.is_some()
            || self.color.is_some()
            || self.highlight.is_some()
    }

    /// Whether a run with `run`'s direct formatting, in a paragraph of style `para_style` (in a
    /// table cell whose table style gives `table`), has the formatting asked for.
    pub(crate) fn run_matches(&self, styles: &StyleSheet, para_style: Option<&str>, table: Option<&CharProps>, run: &CharProps) -> bool {
        if let Some(st) = &self.style {
            let has = if st.paragraph { para_style.unwrap_or("Normal") == st.id } else { run.style.as_deref() == Some(st.id.as_str()) };
            if !has {
                return false;
            }
        }
        if !self.needs_resolved() {
            return true;
        }
        let r = styles.resolve_char_in(para_style, table, run);
        let eq = |want: Option<bool>, has: bool| want.is_none_or(|w| w == has);
        eq(self.bold, r.bold)
            && eq(self.italic, r.italic)
            && eq(self.underline, r.underline != Underline::None)
            && eq(self.strike, r.strike)
            && eq(self.small_caps, r.small_caps)
            && eq(self.all_caps, r.caps)
            && eq(self.hidden, r.hidden)
            && eq(self.subscript, r.vert_align == VertAlign::Subscript)
            && eq(self.superscript, r.vert_align == VertAlign::Superscript)
            && self.font.as_ref().is_none_or(|f| r.font.eq_ignore_ascii_case(f))
            && self.size.is_none_or(|sz| (r.size - sz).abs() < 0.05)
            && self.color.is_none_or(|c| r.color == c)
            && match self.highlight {
                None => true,
                Some(HighlightMatch::Any) => r.highlight.is_some(),
                Some(HighlightMatch::Is(h)) => r.highlight == h.rgb(),
            }
    }
}

impl FormatApply {
    pub(super) fn style_id(&self) -> Option<&str> {
        self.para_style.as_deref().or(self.props.style.as_deref())
    }

    pub(super) fn has_direct_formatting(&self) -> bool {
        let mut props = self.props.clone();
        props.style = None;
        props != CharProps::default()
    }

    fn is_empty(&self) -> bool {
        self.props == CharProps::default() && self.para_style.is_none()
    }
}

/// The value of an optional true/false key.
fn flag(o: &serde_json::Map<String, Value>, key: &str) -> Result<Option<bool>, CmdError> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(CmdError::Params(format!("`{key}` is true or false"))),
    }
}

/// The value of an optional text key: not empty and not absurdly long.
fn text<'a>(o: &'a serde_json::Map<String, Value>, key: &str) -> Result<Option<&'a str>, CmdError> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(t)) if !t.trim().is_empty() && t.chars().count() <= 128 => Ok(Some(t.trim())),
        Some(_) => Err(CmdError::Params(format!("`{key}` is a text of 1 to 128 characters"))),
    }
}

fn size(o: &serde_json::Map<String, Value>) -> Result<Option<f32>, CmdError> {
    match o.get("size") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_f64() {
            Some(x) if (1.0..=1638.0).contains(&x) => Ok(Some(x as f32)),
            _ => Err(CmdError::Params("`size` is a number between 1 and 1638".into())),
        },
    }
}

fn color(o: &serde_json::Map<String, Value>) -> Result<Option<TextColor>, CmdError> {
    match text(o, "color")? {
        None => Ok(None),
        Some(c) => TextColor::parse(c).map(Some).ok_or_else(|| CmdError::Params("`color` is RRGGBB or auto".into())),
    }
}

/// A highlight colour by name (as `format.highlight` takes it).
pub(crate) fn highlight_named(c: &str) -> Option<Highlight> {
    Highlight::ALL.iter().copied().find(|h| {
        h.ooxml().eq_ignore_ascii_case(c)
            || h.name().replace(['-', ' ', '%'], "").eq_ignore_ascii_case(&c.replace(['-', ' ', '%'], ""))
            || format!("{h:?}").eq_ignore_ascii_case(c)
    })
}

/// The `highlight` key: a colour name, `"none"`, `false` (not highlighted) or, to look for, `true` (any).
fn highlight(o: &serde_json::Map<String, Value>, allow_any: bool) -> Result<Option<HighlightMatch>, CmdError> {
    match o.get("highlight") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(true)) if allow_any => Ok(Some(HighlightMatch::Any)),
        Some(Value::Bool(false)) => Ok(Some(HighlightMatch::Is(Highlight::None))),
        Some(Value::String(c)) => {
            highlight_named(c).map(|h| Some(HighlightMatch::Is(h))).ok_or_else(|| CmdError::Params(format!("unknown highlight colour `{c}`")))
        }
        Some(_) => {
            Err(CmdError::Params("`highlight` is a colour name, \"none\"".to_string() + if allow_any { ", true or false" } else { " or false" }))
        }
    }
}

/// The style named by `style`: its id and whether it is a paragraph style.
fn style(s: &Session, o: &serde_json::Map<String, Value>) -> Result<Option<StyleFilter>, CmdError> {
    let Some(name) = text(o, "style")? else { return Ok(None) };
    let st = s.doc.styles.find(name).ok_or_else(|| CmdError::Params(format!("no style `{name}`")))?;
    match st.kind {
        StyleKind::Paragraph => Ok(Some(StyleFilter { id: st.id.clone(), paragraph: true })),
        StyleKind::Character => Ok(Some(StyleFilter { id: st.id.clone(), paragraph: false })),
        _ => Err(CmdError::Params(format!("`{name}` is not a paragraph or character style"))),
    }
}

/// The object of `format` / `withFormat`, or `None` for null; refuses a value that is not an
/// object and keys that are not formatting.
fn object<'a>(v: &'a Value, what: &str) -> Result<Option<&'a serde_json::Map<String, Value>>, CmdError> {
    match v {
        Value::Null => Ok(None),
        Value::Object(o) => match o.keys().find(|k| !KEYS.contains(&k.as_str())) {
            Some(k) => Err(CmdError::Params(format!("`{what}` has no `{k}`; it takes {}", KEYS.join(", ")))),
            None => Ok(Some(o)),
        },
        _ => Err(CmdError::Params(format!("`{what}` is an object (or null for no formatting)"))),
    }
}

/// `format`: what Find what › Format asks for. `None` is no formatting (null or `{}`).
pub(crate) fn parse_filter(s: &Session, v: &Value) -> Result<Option<FormatFilter>, CmdError> {
    let Some(o) = object(v, "format")? else { return Ok(None) };
    let f = FormatFilter {
        bold: flag(o, "bold")?,
        italic: flag(o, "italic")?,
        underline: flag(o, "underline")?,
        strike: flag(o, "strike")?,
        small_caps: flag(o, "smallCaps")?,
        all_caps: flag(o, "allCaps")?,
        hidden: flag(o, "hidden")?,
        subscript: flag(o, "subscript")?,
        superscript: flag(o, "superscript")?,
        font: text(o, "font")?.map(str::to_string),
        size: size(o)?,
        color: color(o)?,
        highlight: highlight(o, true)?,
        style: style(s, o)?,
    };
    Ok((!f.is_empty()).then_some(f))
}

/// `withFormat`: what Replace with › Format sets. `None` is no formatting (null or `{}`).
pub(crate) fn parse_apply(s: &Session, v: &Value) -> Result<Option<FormatApply>, CmdError> {
    let Some(o) = object(v, "withFormat")? else { return Ok(None) };
    let mut a = FormatApply::default();
    let c = &mut a.props;
    c.bold = flag(o, "bold")?;
    c.bold_cs = c.bold;
    c.italic = flag(o, "italic")?;
    c.italic_cs = c.italic;
    c.underline = flag(o, "underline")?.map(|on| if on { Underline::Single } else { Underline::None });
    c.strike = flag(o, "strike")?;
    c.small_caps = flag(o, "smallCaps")?;
    c.caps = flag(o, "allCaps")?;
    c.hidden = flag(o, "hidden")?;
    let (sub, sup) = (flag(o, "subscript")?, flag(o, "superscript")?);
    c.vert_align = match (sub, sup) {
        (Some(true), Some(true)) => return Err(CmdError::Params("`subscript` and `superscript` exclude each other".into())),
        (Some(true), _) => Some(VertAlign::Subscript),
        (_, Some(true)) => Some(VertAlign::Superscript),
        (None, None) => None,
        _ => Some(VertAlign::Baseline),
    };
    c.font = text(o, "font")?.map(str::to_string);
    // Word rounds a size to half points.
    c.size = size(o)?.map(|x| (x * 2.0).round() / 2.0);
    c.size_cs = c.size;
    c.color = color(o)?;
    c.highlight = match highlight(o, false)? {
        Some(HighlightMatch::Is(h)) => Some(h),
        _ => None,
    };
    match style(s, o)? {
        Some(StyleFilter { id, paragraph: true }) => a.para_style = Some(id),
        Some(StyleFilter { id, paragraph: false }) => a.props.style = Some(id),
        None => {}
    }
    Ok((!a.is_empty()).then_some(a))
}

/// Read `format` and `withFormat` from a find or replace command's params into the find state.
/// A key that is absent leaves what is kept; null or `{}` clears it. Nothing changes on an error.
pub(crate) fn read(s: &mut Session, v: &Value) -> Result<(), CmdError> {
    if let Some(f) = v.get("format").map(|x| parse_filter(s, x)).transpose()? {
        s.find.format = f;
    }
    Ok(())
}

/// `withFormat` of this call (not kept).
pub(crate) fn read_with(s: &Session, v: &Value) -> Result<Option<FormatApply>, CmdError> {
    Ok(v.get("withFormat").map(|x| parse_apply(s, x)).transpose()?.flatten())
}

/// A format-only search with nothing to put in place of the found text would delete it; ask for
/// replacement text or a `withFormat` instead.
pub(crate) fn check_replace(s: &Session, with: Option<&FormatApply>) -> Result<(), CmdError> {
    // A format-only match has no found text: `^&` and group references would expand to nothing
    // and Replace All would delete the formatted text.
    if s.find.format.is_some() && s.find.query.is_empty() {
        let pieces = super::find_pattern::parse_replace(&s.find.replace, &super::edit::find_opts(&s.find));
        if pieces
            .iter()
            .any(|p| matches!(p, super::find_pattern::Piece::Found | super::find_pattern::Piece::Group(_) | super::find_pattern::Piece::Named(_)))
        {
            return Err(CmdError::Params("a search by formatting only has no found text for `^&` or a group in `with`".into()));
        }
    }
    if s.find.format.is_some() && s.find.query.is_empty() && s.find.replace.is_empty() && with.is_none() {
        return Err(CmdError::Params("replacing found formatting needs `with` text or a `withFormat`".into()));
    }
    Ok(())
}

/// The stretches of text with the formatting in the find state (a search with empty text),
/// paragraph by paragraph, in order.
pub(crate) fn runs(s: &Session, story: StoryRef) -> Vec<Found> {
    let Some(f) = s.find.format.as_ref() else { return Vec::new() };
    let mut out = Vec::new();
    let push = |out: &mut Vec<Found>, path: &Path, (a, b): (usize, usize)| {
        out.push(Found { a: Pos { story, path: path.clone(), off: a }, b: Pos { story, path: path.clone(), off: b }, rep: None });
    };
    for path in s.doc.para_paths(story) {
        let Some(p) = s.doc.para(story, &path) else { continue };
        let mut open: Option<(usize, usize)> = None;
        let table = table_chr(s, f, story, &path);
        for (r, ch) in p.run_ranges() {
            if r.is_empty() {
                continue;
            }
            // Text marked deleted is no longer part of the document: it ends a stretch.
            if ch.del.is_none() && f.run_matches(&s.doc.styles, p.props.style.as_deref(), table.as_ref(), ch) {
                match open.as_mut() {
                    Some((_, end)) if *end == r.start => *end = r.end,
                    _ => {
                        if let Some(iv) = open.take() {
                            push(&mut out, &path, iv);
                        }
                        open = Some((r.start, r.end));
                    }
                }
            } else if let Some(iv) = open.take() {
                push(&mut out, &path, iv);
            }
        }
        if let Some(iv) = open.take() {
            push(&mut out, &path, iv);
        }
        if out.len() >= MAX_MATCHES {
            out.truncate(MAX_MATCHES);
            break;
        }
    }
    out
}

/// The run formatting the table style gives the cell holding the paragraph at `path` (the
/// innermost table's, with its conditional parts), when the filter looks at resolved formatting.
pub(super) fn table_chr(s: &Session, f: &FormatFilter, story: StoryRef, path: &Path) -> Option<CharProps> {
    if !f.needs_resolved() || path.0.len() < 3 {
        return None;
    }
    let mut blocks = s.doc.story(story)?;
    let mut chr = None;
    let mut i = 0;
    while let Some(Block::Table(tbl)) = blocks.get(usize::try_from(*path.0.get(i)?).ok()?).map(|b| &**b) {
        let (ri, ci) = (usize::try_from(*path.0.get(i + 1)?).ok()?, usize::try_from(*path.0.get(i + 2)?).ok()?);
        let cell = tbl.rows.get(ri)?.cells.get(ci)?;
        chr = None;
        if let Some(st) = tbl.props.style.as_deref().and_then(|id| s.doc.styles.table_style(id)) {
            let sizes = (st.parts.band_size.unwrap_or(0), st.parts.col_band_size.unwrap_or(0));
            let region = PartSet::of_cell(&tbl.props.look, sizes, (ri, tbl.rows.len()), (tbl.grid_col(ri, ci), cell.span(), tbl.cols()));
            let mut c = CharProps::default();
            c.overlay(&st.chr);
            for part in region.iter() {
                c.overlay(&st.part(part).chr);
            }
            chr = Some(c);
        }
        blocks = &cell.blocks;
        i += 3;
    }
    chr
}

/// Replace with has formatting and no text: the found text stays and only its formatting changes.
pub(crate) fn keeps_text(s: &Session, with: Option<&FormatApply>) -> bool {
    with.is_some() && s.find.replace.is_empty()
}

/// Apply Replace with › Format to `a..b`, the text that stands in a match's place. Text marked
/// deleted (the replaced text, under Track Changes) is left as it is.
pub(crate) fn apply_with(
    s: &mut Session,
    a: &Pos,
    b: &Pos,
    with: Option<&FormatApply>,
    tracker: Option<&super::fmt_revisions::ParaPropTracker>,
) -> Result<(), CmdError> {
    let Some(w) = with else { return Ok(()) };
    if a.story != b.story {
        return Ok(());
    }
    if w.props != CharProps::default() {
        s.doc.format_range(a, b, &|c| {
            if c.del.is_none() {
                c.overlay(&w.props);
            }
        })?;
    }
    if let Some(id) = &w.para_style {
        if let Some(tracker) = tracker {
            for path in s.doc.para_paths_in(a, b) {
                let Some(para) = s.doc.para(a.story, &path) else { continue };
                let before = para.props.clone();
                let mut after = before.clone();
                after.style = Some(id.clone());
                after.fmt_change = tracker.changed(s, &before, &after);
                let para = s.doc.para_mut(a.story, &path)?;
                para.props = after;
                para.touch();
            }
        } else {
            s.doc.format_paragraphs(a, b, &|p| p.style = Some(id.clone()))?;
        }
    }
    Ok(())
}

/// Replace the match the selection is on (`rep`, its Replace with text), then format what stands there.
pub(crate) fn replace_current(s: &mut Session, rep: Option<String>, with: Option<&FormatApply>) -> Result<(), CmdError> {
    if keeps_text(s, with) {
        let (a, b) = s.sel.ordered();
        return apply_with(s, &a, &b, with, None);
    }
    let tracker = replacement_tracker(s, with);
    replace_text(s, &rep.unwrap_or_default(), with, tracker.as_ref())
}

pub(super) fn replacement_tracker(s: &Session, with: Option<&FormatApply>) -> Option<super::fmt_revisions::ParaPropTracker> {
    (s.doc.settings.track_changes && s.prefs.markup.track_formatting && !keeps_text(s, with) && with.is_some_and(|w| w.para_style.is_some()))
        .then(|| super::fmt_revisions::ParaPropTracker::new(s))
}

pub(super) fn replace_text(
    s: &mut Session,
    rep: &str,
    with: Option<&FormatApply>,
    tracker: Option<&super::fmt_revisions::ParaPropTracker>,
) -> Result<(), CmdError> {
    let (a, _) = s.sel.ordered();
    super::paste_text(s, rep)?;
    let b = s.sel.focus.clone();
    apply_with(s, &a, &b, with, tracker)
}
