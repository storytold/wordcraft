//! STYLEREF fields (ECMA-376 Part 1 §17.16.5.59): the text, or the paragraph number, of the
//! nearest paragraph (or run) with a given style.
//!
//! In a header or footer the search starts on the page the header is drawn on: its first
//! paragraph of the style (with `\l`, its last), else the nearest one before the page, else the
//! nearest one after. In the body it starts at the field's own paragraph and goes back towards
//! the start of the document, then forward. Headers and footers are evaluated at layout time,
//! page by page ([`crate::FieldCtx::styleref`]); body fields are updated like other fields.
//!
//! Switches: `\l` searches the page bottom-up; `\n` the paragraph number, `\r` and `\w` the
//! number in full context (all levels; `\r` is treated like `\w`), `\s` a chapter number (the
//! number alone, or the heading's count when it isn't numbered); `\t` drops the text around
//! the number; `\p` adds "above" / "below".

use std::collections::HashMap;

use wordcraft_doc::numbering::Counters;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::section::NumFormat;
use wordcraft_doc::{Document, Path, StoryRef, StyleKind};

/// The result when no paragraph has the style (or the style doesn't exist).
pub const NO_TEXT: &str = "Error: no paragraph in this style.";

/// Which number a STYLEREF shows instead of the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Number {
    /// The text (no number switch).
    #[default]
    Text,
    /// `\n`: the paragraph's own number.
    Plain,
    /// `\r` / `\w`: the number with the levels above it.
    Full,
    /// `\s`: a chapter number.
    Chapter,
}

/// A parsed STYLEREF instruction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StyleRefCode {
    /// Style name or id, or a heading level `1`–`9`.
    pub style: String,
    /// `\l`: the last paragraph of the style on the page.
    pub last: bool,
    pub number: Number,
    /// `\p`: "above" / "below".
    pub position: bool,
    /// `\t`: only the number, without the text of its level.
    pub suppress: bool,
}

/// Parse `STYLEREF "Heading 1" \l \n …`. Quoted text stays whole; an unquoted name of several
/// words is joined back up.
pub fn parse(instr: &str) -> StyleRefCode {
    let mut words: Vec<(String, bool)> = Vec::new();
    let mut chars = instr.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let mut w = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' if matches!(chars.peek(), Some('"' | '\\')) => w.extend(chars.next()),
                    c => w.push(c),
                }
            }
            words.push((w, true));
        } else {
            let mut w = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == '"' {
                    break;
                }
                w.push(c);
                chars.next();
            }
            words.push((w, false));
        }
    }
    let mut code = StyleRefCode::default();
    let mut it = words.into_iter().skip(1);
    while let Some((w, quoted)) = it.next() {
        let switch = if quoted { None } else { w.strip_prefix('\\').map(|r| r.chars().next().map(|c| c.to_ascii_lowercase())) };
        match switch {
            Some(Some('l')) => code.last = true,
            Some(Some('n')) => code.number = Number::Plain,
            Some(Some('r' | 'w')) => code.number = Number::Full,
            Some(Some('s')) => code.number = Number::Chapter,
            Some(Some('p')) => code.position = true,
            Some(Some('t')) => code.suppress = true,
            // General switches (`\* MERGEFORMAT`, `\@ …`, `\# …`) take a word.
            Some(Some('*' | '@' | '#')) => {
                it.next();
            }
            Some(_) => {}
            None if code.style.is_empty() => code.style = w,
            None if !quoted => {
                code.style.push(' ');
                code.style.push_str(&w);
            }
            None => {}
        }
    }
    code
}

/// What the search looks for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Para(String),
    Char(String),
}

/// A body paragraph, as STYLEREF sees it.
#[derive(Clone, Debug)]
struct Entry {
    path: Path,
    style: String,
    text: String,
    /// Character-styled runs: (style id, text).
    runs: Vec<(String, String)>,
    /// The list label as shown (`Chapter 2.`).
    label: Option<String>,
    /// Each level's value up to the paragraph's, formatted; and whether the label's own
    /// template shows that level.
    levels: Vec<(String, bool)>,
    /// How many paragraphs of its style there are up to this one.
    ordinal: u32,
}

/// Where a STYLEREF field is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum At {
    /// In a header or footer of a page whose body starts at entry `first`; entries
    /// `first..=last` are on the page (`last` is `None` when no body paragraph is).
    Page { first: usize, last: Option<usize> },
    /// In body paragraph `i`.
    Body(usize),
}

/// The body's paragraphs with their styles, text and numbers, in document order.
#[derive(Clone, Debug, Default)]
pub struct StyleIndex {
    entries: Vec<Entry>,
    by_path: HashMap<Path, usize>,
    /// Lower-cased style id or name → (id, character style).
    styles: HashMap<String, (String, bool)>,
}

fn clean(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().trim().to_string()
}

impl StyleIndex {
    pub fn build(doc: &Document) -> StyleIndex {
        let mut styles = HashMap::new();
        for st in &doc.styles.styles {
            let v = (st.id.clone(), st.kind == StyleKind::Character);
            styles.entry(st.name.to_lowercase()).or_insert_with(|| v.clone());
            styles.insert(st.id.to_lowercase(), v);
        }
        let mut counters = Counters::default();
        let mut ordinals: HashMap<String, u32> = HashMap::new();
        let mut entries = Vec::new();
        let mut by_path = HashMap::new();
        for path in doc.para_paths(StoryRef::Body) {
            let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
            let style = p.props.style.clone().unwrap_or_else(|| "Normal".to_string());
            let ord = ordinals.entry(style.clone()).or_insert(0);
            *ord = ord.saturating_add(1);
            let ordinal = *ord;
            let num = p.props.numbering.or_else(|| doc.styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0);
            let (mut label, mut levels) = (None, Vec::new());
            if let Some(n) = num
                && let Some((l, def)) = counters.next_label(&doc.numbering, n.num, n.level)
                && def.format != NumFormat::Bullet
            {
                let vals = counters.values(&doc.numbering, n.num).unwrap_or_default();
                for (k, v) in vals.iter().enumerate().take(usize::from(n.level.min(8)) + 1) {
                    let Some(lv) = doc.numbering.level(n.num, k as u8) else { continue };
                    if matches!(lv.format, NumFormat::None | NumFormat::Bullet) {
                        continue;
                    }
                    let v = if *v == 0 { lv.start } else { *v };
                    let fmt = if def.legal && lv.format != NumFormat::DecimalZero { NumFormat::Decimal } else { lv.format };
                    levels.push((fmt.format(v), def.text.contains(&format!("%{}", k + 1))));
                }
                label = Some(l);
            }
            let mut runs: Vec<(String, String)> = Vec::new();
            let mut off = 0usize;
            for r in &p.runs {
                let end = off.saturating_add(r.len);
                if let Some(cs) = &r.props.style
                    && let Some(t) = p.text.get(off..end)
                {
                    let t: String = t.chars().filter(|c| *c != wordcraft_doc::para::OBJ).collect();
                    match runs.last_mut() {
                        Some((s, prev)) if s == cs => prev.push_str(&t),
                        _ => runs.push((cs.clone(), t)),
                    }
                }
                off = end;
            }
            by_path.insert(path.clone(), entries.len());
            entries.push(Entry { path, style, text: clean(&p.plain_text()), runs, label, levels, ordinal });
        }
        StyleIndex { entries, by_path, styles }
    }

    /// The entry of the body paragraph at `path`.
    pub fn index_of(&self, path: &Path) -> Option<usize> {
        self.by_path.get(path).copied()
    }

    /// Where the header or footer of a page is, from the body paragraphs on the page (by path)
    /// and the page's first top-level block.
    pub fn page_at<'a>(&self, paths: impl Iterator<Item = &'a Path>, first_block: usize) -> At {
        let mut range: Option<(usize, usize)> = None;
        for i in paths.filter_map(|p| self.index_of(p)) {
            range = Some(range.map_or((i, i), |(a, b)| (a.min(i), b.max(i))));
        }
        match range {
            Some((first, last)) => At::Page { first, last: Some(last) },
            None => {
                let first = self.entries.iter().position(|e| e.path.0.first().is_some_and(|b| *b as usize >= first_block));
                At::Page { first: first.unwrap_or(self.entries.len()), last: None }
            }
        }
    }

    fn target(&self, style: &str) -> Option<Target> {
        let style = style.trim();
        if let Ok(n) = style.parse::<u8>() {
            if !(1..=9).contains(&n) {
                return None;
            }
            // The built-in style is named `heading N` whatever its id (localized Word writes
            // ids like `berschrift1`); else an id `HeadingN`.
            let id = self
                .styles
                .get(&format!("heading {n}"))
                .or_else(|| self.styles.get(&format!("heading{n}")))
                .map(|s| s.0.clone())
                .unwrap_or_else(|| format!("Heading{n}"));
            return Some(Target::Para(id));
        }
        let (id, chr) = self.styles.get(&style.to_lowercase())?;
        Some(if *chr { Target::Char(id.clone()) } else { Target::Para(id.clone()) })
    }

    fn text_of(&self, i: usize, t: &Target) -> Option<String> {
        let e = self.entries.get(i)?;
        match t {
            Target::Para(id) => (e.style == *id).then(|| e.text.clone()),
            Target::Char(id) => e.runs.iter().find(|(s, _)| s == id).map(|(_, t)| clean(t)),
        }
    }

    /// The field's result.
    pub fn eval(&self, instr: &str, at: At) -> String {
        let code = parse(instr);
        let Some(target) = self.target(&code.style) else { return NO_TEXT.to_string() };
        let n = self.entries.len();
        // Candidates in search order, with whether each one counts as below the field (lazy:
        // headers evaluate this for every page on every pass).
        let order: Box<dyn Iterator<Item = (usize, bool)>> = match at {
            At::Page { first, last } => {
                let on = match last {
                    Some(l) if first <= l => first..l.min(n.saturating_sub(1)).saturating_add(1),
                    _ => 0..0,
                };
                let after = last.map_or(first, |l| l.saturating_add(1));
                let page: Box<dyn Iterator<Item = usize>> = if code.last { Box::new(on.rev()) } else { Box::new(on) };
                Box::new(page.map(|i| (i, true)).chain((0..first.min(n)).rev().map(|i| (i, false))).chain((after..n).map(|i| (i, true))))
            }
            At::Body(i) => Box::new((0..=i.min(n.saturating_sub(1))).rev().map(|k| (k, false)).chain((i.saturating_add(1)..n).map(|k| (k, true)))),
        };
        let Some((i, text, below)) = order.into_iter().find_map(|(i, below)| self.text_of(i, &target).map(|t| (i, t, below))) else {
            return NO_TEXT.to_string();
        };
        let Some(e) = self.entries.get(i) else { return NO_TEXT.to_string() };
        let mut out = match code.number {
            Number::Text if code.position => String::new(),
            Number::Text => text,
            Number::Chapter => e.label.as_deref().map(chapter_number).filter(|s| !s.is_empty()).unwrap_or_else(|| e.ordinal.to_string()),
            Number::Plain | Number::Full => {
                let own = match &e.label {
                    Some(l) if code.suppress => suppress(l, &e.levels),
                    Some(l) => l.trim().trim_end_matches('.').to_string(),
                    None => e.ordinal.to_string(),
                };
                if code.number == Number::Full && e.label.is_some() {
                    // Levels above that the label's own template doesn't show.
                    let above: Vec<&str> =
                        e.levels.iter().take(e.levels.len().saturating_sub(1)).filter(|(_, shown)| !shown).map(|(v, _)| v.as_str()).collect();
                    if above.is_empty() { own } else { format!("{}.{own}", above.join(".")) }
                } else {
                    own
                }
            }
        };
        if code.position {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(if below { "below" } else { "above" });
        }
        out
    }
}

/// The number part of a label (`Chapter 2.` → `2`).
fn chapter_number(label: &str) -> String {
    let word = label.split_whitespace().last().unwrap_or("");
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_string()
}

/// `\t`: the label without its words that hold no number (`Chapter 2.` → `2`).
fn suppress(label: &str, levels: &[(String, bool)]) -> String {
    let kept: Vec<&str> = label.split_whitespace().filter(|w| levels.iter().any(|(v, _)| !v.is_empty() && w.contains(v.as_str()))).collect();
    let s = if kept.is_empty() { label.trim().to_string() } else { kept.join(" ") };
    s.trim_end_matches('.').to_string()
}

/// Does `p` hold a STYLEREF field?
pub fn has_styleref(p: &wordcraft_doc::Paragraph) -> bool {
    p.objects.iter().any(is_styleref)
}

fn is_styleref(o: &InlineObject) -> bool {
    matches!(o, InlineObject::Field { instr, .. } if crate::fields::field_name(instr) == "STYLEREF")
}

/// Does any header or footer of `doc` hold a STYLEREF field?
pub fn headers_use_styleref(doc: &Document) -> bool {
    doc.parts.keys().any(|id| doc.para_paths(StoryRef::Part(*id)).iter().any(|p| doc.para(StoryRef::Part(*id), p).is_some_and(has_styleref)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_levels_and_switches() {
        crate::test_fonts();
        let c = parse(r#"STYLEREF "Heading 1" \l \n \t \* MERGEFORMAT"#);
        assert_eq!(c.style, "Heading 1");
        assert!(c.last && c.suppress && !c.position);
        assert_eq!(c.number, Number::Plain);
        assert_eq!(parse("STYLEREF 1 \\s").style, "1");
        assert_eq!(parse("STYLEREF 1 \\s").number, Number::Chapter);
        assert_eq!(parse("STYLEREF Heading 2 \\p").style, "Heading 2");
        assert_eq!(parse("STYLEREF").style, "");
    }

    #[test]
    fn suppress_keeps_the_number() {
        crate::test_fonts();
        assert_eq!(suppress("Chapter 2.", &[("2".into(), true)]), "2");
        assert_eq!(chapter_number("Chapter IV."), "IV");
    }
}
