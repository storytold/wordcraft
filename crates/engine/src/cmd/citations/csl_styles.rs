//! References ▸ Citation Styles (CSL): styles the user installs (`references.cslStyles`),
//! beside the twelve built-ins. [`StyleRef`] is what the citation machinery sees — built-in
//! or CSL — so no caller has to know which kind is in effect (#1058).

use std::cmp::Ordering;

use serde_json::{Value, json};
use wordcraft_csl::{Element as CslElement, Position, SortDirection, SortKey as CslSortKey, TextSource};
use wordcraft_doc::{Document, Source};

use super::render::{built_cite, built_entry};
use super::sources::{bib_order, cited_tags};
use super::styles::{CiteStyle, STYLES, find_style};
use super::{CmdError, CmdResult, Session, p};

/// One CSL style loaded in the session: the parsed style and the `.csl` file it came from
/// (its name in the styles folder, which `references.cslStyles` installs and removes).
#[derive(Clone, Debug)]
pub struct CslStyleEntry {
    /// File name inside the styles folder (`ieee.csl`).
    pub file: String,
    pub style: wordcraft_csl::Style,
}

impl CslStyleEntry {
    /// What the style is called: the CSL `<info><id>`, or the file's stem when the file has
    /// none — the `<id>` of `csl:<id>`, and the id `references.cslStyles` lists and removes.
    pub fn id(&self) -> &str {
        if self.style.id.is_empty() { self.file.rsplit_once('.').map_or(&self.file, |(stem, _)| stem) } else { &self.style.id }
    }
}

/// The style a session's citations render with: one of the twelve built-ins, or a CSL style
/// the user installed. Carries the behaviors the citation machinery consumes — in-text cite
/// text, bibliography entries and their order, the bibliography's title, whether cites are
/// numbered — so no caller has to know which kind it holds.
pub(super) enum StyleRef<'a> {
    Built(&'static CiteStyle),
    Csl(&'a wordcraft_csl::Style),
}

impl StyleRef<'_> {
    /// Numbered styles cite `[1]`/`(1)` and list entries without a hanging indent; a CSL
    /// style is numbered when its citation layout cites `citation-number`.
    pub(super) fn numbered(&self) -> bool {
        match self {
            StyleRef::Built(st) => st.numbered(),
            StyleRef::Csl(st) => csl_numbered(st),
        }
    }

    /// The bibliography's default heading; CSL styles don't name theirs, so they fall back
    /// to ours.
    pub(super) fn bib_title(&self) -> String {
        match self {
            StyleRef::Built(st) => st.bib_title.to_string(),
            StyleRef::Csl(st) => st.bib_title().unwrap_or("Bibliography").to_string(),
        }
    }

    /// One in-text citation: `n` is the source's number in numbered styles, `position` the
    /// first/subsequent/ibid state CSL's choose branches test (built-ins ignore it).
    pub(super) fn cite(&self, src: &Source, position: Position, n: usize, pages: &str) -> String {
        match self {
            StyleRef::Built(st) => built_cite(st, src, n, pages),
            StyleRef::Csl(st) => st.cite(src, position, n, pages),
        }
    }

    /// Bibliography entries (text, hanging indent) in the style's own order.
    pub(super) fn bib_entries(&self, doc: &Document, lang: Option<String>) -> Vec<(String, bool)> {
        let hanging = !self.numbered();
        let order = bib_order(doc, self, lang);
        match self {
            StyleRef::Built(st) => order
                .into_iter()
                .enumerate()
                .filter_map(|(k, i)| doc.sources.get(i).map(|x| (built_entry(st, x, k.saturating_add(1)), hanging)))
                .collect(),
            // `entries` renders them in the order it sorted (numbering numeric styles from
            // it) — the same order `bib_order` decided on.
            StyleRef::Csl(st) => {
                let items: Vec<Source> = order.into_iter().filter_map(|i| doc.sources.get(i).cloned()).collect();
                st.entries(&items).into_iter().map(|(text, _)| (text, hanging)).collect()
            }
        }
    }
}

/// The session's CSL style called `name`: `csl:<id>`, the bare id, or its title (any case).
pub(super) fn find_csl_entry<'a>(s: &'a Session, name: &str) -> Option<&'a CslStyleEntry> {
    let n = name.trim();
    let id = n.strip_prefix("csl:").unwrap_or(n);
    s.csl_styles.iter().find(|e| e.id().eq_ignore_ascii_case(id) || (!e.style.title.is_empty() && e.style.title.eq_ignore_ascii_case(n)))
}

/// The session's CSL style with this id (what a stored `csl:<id>` names).
fn csl_entry_by_id<'a>(s: &'a Session, id: &str) -> Option<&'a CslStyleEntry> {
    s.csl_styles.iter().find(|e| e.id().eq_ignore_ascii_case(id.trim()))
}

/// The session's style: a built-in by name or alias (as ever), then the installed CSL
/// styles. A `csl:` style that is no longer installed is a parameter error naming it —
/// never a silent APA fallback; any other unknown name stays APA, as it always was.
pub(super) fn session_style(s: &Session) -> Result<StyleRef<'_>, CmdError> {
    if let Some(st) = find_style(&s.bib_style) {
        return Ok(StyleRef::Built(st));
    }
    if let Some(id) = s.bib_style.strip_prefix("csl:") {
        return csl_entry_by_id(s, id)
            .map(|e| StyleRef::Csl(&e.style))
            .ok_or_else(|| CmdError::Params(format!("CSL style {id} is not installed; re-add it in References ▸ Style")));
    }
    Ok(StyleRef::Built(&STYLES[0]))
}

/// Does the style's citation layout cite by number (`citation-number`), directly or through
/// a macro? Macro chains are bounded at parse time; `seen` keeps the walk cheap anyway.
fn csl_numbered(style: &wordcraft_csl::Style) -> bool {
    fn uses_number(el: &CslElement, style: &wordcraft_csl::Style, seen: &mut Vec<String>) -> bool {
        match el {
            CslElement::Text(t) => match &t.source {
                TextSource::Variable(v) => v.eq_ignore_ascii_case("citation-number"),
                TextSource::Macro(m) => {
                    if seen.iter().any(|x| x == m) {
                        return false;
                    }
                    seen.push(m.clone());
                    style.macros.get(m).is_some_and(|els| els.iter().any(|e| uses_number(e, style, seen)))
                }
                _ => false,
            },
            CslElement::Number(n) => n.variable.eq_ignore_ascii_case("citation-number"),
            CslElement::Group(g) => g.elements.iter().any(|e| uses_number(e, style, seen)),
            CslElement::Choose(c) => {
                c.branches.iter().any(|b| b.layout.elements.iter().any(|e| uses_number(e, style, seen)))
                    || c.else_branch.as_ref().is_some_and(|e| e.elements.iter().any(|x| uses_number(x, style, seen)))
            }
            CslElement::Names(n) => n.substitute.as_ref().is_some_and(|els| els.iter().any(|e| uses_number(e, style, seen))),
            CslElement::Date(_) | CslElement::Label(_) => false,
        }
    }
    style.citation.elements.iter().any(|e| uses_number(e, style, &mut Vec::new()))
}

/// Indices into `doc.sources` of the cited sources, in the CSL style's bibliography order:
/// the order `Style::entries` gives — its sort keys, first-cited when it sorts nothing
/// (every source while nothing is cited) — so citation numbers and entries agree.
pub(super) fn csl_order(doc: &Document, style: &wordcraft_csl::Style) -> Vec<usize> {
    let cited = cited_tags(doc);
    let idx: Vec<usize> = if cited.is_empty() {
        (0..doc.sources.len()).collect()
    } else {
        cited.iter().filter_map(|t| doc.sources.iter().position(|x| &x.tag == t)).collect()
    };
    let Some(bib) = style.bibliography.as_ref() else { return idx };
    // Each source's sort keys — exactly the keys `entries` computes for it — then the same
    // stable comparison it orders the entries by.
    let mut keyed: Vec<(usize, Vec<String>)> = idx
        .into_iter()
        .map(|i| {
            let keys = doc.sources.get(i).and_then(|x| style.entries(std::slice::from_ref(x)).into_iter().next().map(|(_, k)| k)).unwrap_or_default();
            (i, keys)
        })
        .collect();
    keyed.sort_by(|a, b| compare_csl_keys(&a.1, &b.1, &bib.sort));
    keyed.into_iter().map(|(i, _)| i).collect()
}

/// The comparison `wordcraft_csl` orders bibliography entries by (its `compare_keys`),
/// mirrored here so citation numbers match the entries' order; ties keep the caller's order.
fn compare_csl_keys(a: &[String], b: &[String], sort: &[CslSortKey]) -> Ordering {
    for (i, key) in sort.iter().enumerate() {
        let ord = a.get(i).map_or("", String::as_str).cmp(b.get(i).map_or("", String::as_str));
        if ord != Ordering::Equal {
            return match key.direction {
                SortDirection::Ascending => ord,
                SortDirection::Descending => ord.reverse(),
            };
        }
    }
    Ordering::Equal
}

/// The styles folder `add`/`remove` work in: the one the host wired up, or a parameter
/// error naming what's missing (the web build has none).
fn csl_dir(s: &Session) -> Result<std::path::PathBuf, CmdError> {
    s.csl_styles_dir.clone().ok_or_else(|| CmdError::Params("no CSL styles folder: this session keeps the built-in styles".into()))
}

/// References ▸ Citation Styles (CSL): list the installed styles, add one by copying its
/// file into the styles folder, remove one by id. Adding a style that is already installed
/// (same id or file) replaces it.
pub(super) fn csl_styles(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(path) = p::str(v, "add") {
        return csl_add(s, path);
    }
    if let Some(id) = p::str(v, "remove") {
        return csl_remove(s, id);
    }
    Ok(json!({"styles": s.csl_styles.iter().map(|e| json!({"id": e.id(), "title": e.style.title, "file": e.file})).collect::<Vec<_>>()}))
}

/// Read `path`, check it parses, and install it under its own file name.
fn csl_add(s: &mut Session, path: &str) -> CmdResult {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        // The styles folder itself is host wiring the web build doesn't have.
        return Err(csl_dir(s).err().unwrap_or_else(|| CmdError::Params("reading files by path isn't available on the web".into())));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = csl_dir(s)?;
        // Refused by size before it is read in; the 1 KiB margin lets the parser's own message
        // govern files between the caps, as `references.importSources` does. The parser's cap
        // is 2 MiB.
        let cap = 2 << 20;
        let bytes = crate::io::read_file_capped(path, cap as u64 + 1024, "CSL style").map_err(|e| CmdError::Params(e.to_string()))?;
        let style = wordcraft_csl::parse(&bytes).map_err(CmdError::Params)?;
        // Only the file's own name — never a path — and never empty (the path is hostile input).
        let file = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or("style.csl")
            .to_string();
        crate::io::create_dir(&dir).map_err(CmdError::Params)?;
        // `write_file`'s error already names the target path.
        crate::io::write_file(&dir.join(&file).to_string_lossy(), &bytes).map_err(CmdError::Params)?;
        let entry = CslStyleEntry { file, style };
        let id = entry.id().to_string();
        let warnings = entry.style.warned().to_vec();
        // Replace the style it is (same id, or the same file re-added), else append.
        match s.csl_styles.iter().position(|e| e.id().eq_ignore_ascii_case(&id) || e.file == entry.file) {
            Some(i) => {
                if let Some(x) = s.csl_styles.get_mut(i) {
                    *x = entry;
                }
            }
            None => s.csl_styles.push(entry),
        }
        Ok(json!({"id": id, "warnings": warnings}))
    }
}

/// Take the style `id` out of the folder and the session; the file goes first, so a session
/// that fails to remove it keeps the style.
fn csl_remove(s: &mut Session, id: &str) -> CmdResult {
    let dir = csl_dir(s)?;
    let i =
        s.csl_styles.iter().position(|e| e.id().eq_ignore_ascii_case(id.trim())).ok_or_else(|| CmdError::Params(format!("no CSL style `{id}`")))?;
    let file = s.csl_styles.get(i).map(|e| e.file.clone()).unwrap_or_default();
    // `delete_file`'s error already names the file's path.
    crate::io::delete_file(&dir.join(&file)).map_err(CmdError::Params)?;
    s.csl_styles.remove(i);
    Ok(json!({"removed": true}))
}
