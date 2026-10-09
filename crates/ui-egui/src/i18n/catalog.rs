//! Parser and lookup table for translation catalogs (`*.tsv`, format documented in `zh-hans.tsv`).
//! The format is PdfCraft's; WordCraft uses its plain (context-free) entries.
//!
//! Validation is strict (unknown escapes, unknown `@` contexts, placeholder, ellipsis and plural
//! form mismatches, duplicates), but a bad line is only skipped and reported: the rest of the
//! catalog still loads and the skipped string shows in English. The tests insist the bundled
//! catalogs have no errors at all.

use std::collections::{HashMap, HashSet};

/// A catalog entry as read from the file.
#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub context: String,
    pub source: String,
    pub translation: String,
}

/// A parsed catalog. Values are owned for the life of the process (catalogs are built once).
#[derive(Debug, Default)]
pub struct Catalog {
    /// English source → translation (the hot path, looked up every frame).
    plain: HashMap<String, String>,
}

fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => return Err(format!("unknown escape \\{other}; use \\n, \\t or \\\\")),
            None => return Err("trailing backslash; use \\\\ for a literal backslash".into()),
        }
    }
    Ok(out)
}

/// `{name}` placeholders of a template, sorted (translators may reorder them).
pub fn placeholders(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some((_, after_open)) = rest.split_once('{') {
        let Some((name, after_close)) = after_open.split_once('}') else { break };
        names.push(name);
        rest = after_close;
    }
    names.sort_unstable();
    names
}

/// Check one entry against its English source.
fn validate(entry: &Entry) -> Result<(), String> {
    let Entry { context, source, translation } = entry;
    if !context.is_empty() {
        return Err(format!("contexts aren't used in WordCraft catalogs (found {context:?}); leave the first column empty"));
    }
    if placeholders(source) != placeholders(translation) {
        return Err("placeholders differ from the English source".into());
    }
    if source.ends_with('…') != translation.ends_with('…') {
        return Err("trailing ellipsis differs from the English source".into());
    }
    Ok(())
}

fn parse_line(line: &str) -> Result<Entry, String> {
    let mut columns = line.split('\t');
    let (Some(context), Some(source), Some(translation), None) = (columns.next(), columns.next(), columns.next(), columns.next()) else {
        return Err("expected `context<TAB>source<TAB>translation`".into());
    };
    let entry = Entry { context: unescape(context)?, source: unescape(source)?, translation: unescape(translation)? };
    if entry.source.is_empty() || entry.translation.is_empty() {
        return Err("source and translation must be nonempty".into());
    }
    validate(&entry)?;
    Ok(entry)
}

/// Read the entries of a catalog file. Malformed lines and duplicates (the first entry wins) are
/// returned as errors and skipped, so a bad translation never breaks the UI.
pub fn parse_entries(text: &str) -> (Vec<Entry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    let mut seen = HashSet::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let line_no = index.saturating_add(1);
        match parse_line(line) {
            Ok(entry) if !seen.insert((entry.context.clone(), entry.source.clone())) => {
                errors.push(format!("line {line_no}: duplicate key {:?} / {:?}", entry.context, entry.source));
            }
            Ok(entry) => entries.push(entry),
            Err(error) => errors.push(format!("line {line_no}: {error}")),
        }
    }
    (entries, errors)
}

impl Catalog {
    /// Build a catalog from its valid entries; the errors of the skipped lines come back too.
    pub fn parse(text: &str) -> (Catalog, Vec<String>) {
        let (entries, errors) = parse_entries(text);
        let plain = entries.into_iter().map(|e| (e.source, e.translation)).collect();
        (Catalog { plain }, errors)
    }

    pub fn plain(&self, s: &str) -> Option<&str> {
        self.plain.get(s).map(String::as_str)
    }
}
