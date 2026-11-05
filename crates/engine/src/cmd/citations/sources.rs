use super::csl_styles::{StyleRef, csl_order, session_style};
use super::generated::update_generated;
use super::render::{authors, corporate};
use super::styles::Order;
use super::{CmdError, CmdResult, Document, InlineObject, Path, Selection, Session, Source, StoryRef, Value, delete_selection, json, p, sort_key};
use wordcraft_csl::Position;

/// The tag a new source gets, as Word makes it: the first three letters of the first author's
/// name and the last two digits of the year (`Smi20`), with `1`, `2`… added while it is taken.
fn auto_tag(doc: &Document, src: &Source) -> String {
    let base = auto_tag_base(src);
    let taken = |t: &str| doc.sources.iter().any(|x| x.tag == t);
    if !taken(&base) {
        return base;
    }
    (1u32..).map(|n| format!("{base}{n}")).find(|t| !taken(t)).unwrap_or(base)
}

/// The author of a source for matching untagged entries: the corporate author (which takes the
/// author's place when a source is saved), else the author, else the editor.
#[cfg(not(target_arch = "wasm32"))]
fn by_author(x: &Source) -> &str {
    [x.corporate.trim(), x.author.as_str(), x.editor.as_str()].into_iter().find(|w| !w.is_empty()).unwrap_or_default()
}

/// The stem of a new source's tag, before `1`, `2`… are added while it is taken: the first three
/// alphanumerics of the first author's name or the corporate author (nothing of the author's,
/// `Src`) and the last two digits of the year.
fn auto_tag_base(src: &Source) -> String {
    let first = corporate(src).map(str::to_string).or_else(|| authors(&src.author).first().map(|x| x.0.clone())).unwrap_or_default();
    let name: String = first.chars().filter(|c| c.is_alphanumeric()).take(3).collect();
    let name = if name.is_empty() { "Src".to_string() } else { name };
    let digits: Vec<char> = src.year.chars().filter(char::is_ascii_digit).collect();
    let year: String = digits.get(digits.len().saturating_sub(2)..).unwrap_or_default().iter().collect();
    format!("{name}{year}")
}

/// Manage Sources: applies `add`, `remove` and `import` (in that order) and returns the
/// document's sources. `import` reads a Word bibliography file (`b:Sources`, #2640).
pub(super) fn sources(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(a) = v.get("add") {
        let mut src: Source = serde_json::from_value(a.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
        // A new source's tag is made from its author and year; an edit keeps the tag it has.
        if src.tag.is_empty() {
            src.tag = auto_tag(&s.doc, &src);
        }
        // Edit Source keeps the source's place in the list and refreshes its citations.
        match s.doc.sources.iter().position(|x| x.tag == src.tag) {
            Some(i) => {
                if let Some(x) = s.doc.sources.get_mut(i) {
                    *x = src;
                }
                update_citations(s)?;
            }
            None => s.doc.sources.push(src),
        }
        s.touch();
    }
    if let Some(t) = p::str(v, "remove") {
        s.doc.sources.retain(|x| x.tag != t);
    }
    if let Some(path) = p::str(v, "import") {
        import_sources(s, path)?;
    }
    Ok(serde_json::to_value(&s.doc.sources).unwrap_or(Value::Null))
}

/// The largest bibliography sources file imported from disk.
#[cfg(not(target_arch = "wasm32"))]
const MAX_SOURCES_FILE: u64 = 32 << 20;

/// Import the sources of a Word bibliography file (`b:Sources`: Word's own `Sources.xml`) into
/// this document (#2640). A tag already here is left alone — an import never overwrites an
/// edited source; a source with no tag is matched to an existing one by title and author, and
/// otherwise gets a tag the way a new source does.
fn import_sources(s: &mut Session, path: &str) -> Result<(), CmdError> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, path);
        return Err(CmdError::Failed("reading files by path isn't available on the web".into()));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Through `io`, which keeps automation inside its read root (#769).
        let bytes = crate::io::read_file_capped(path, MAX_SOURCES_FILE, "sources file").map_err(CmdError::Failed)?;
        let file = wordcraft_docx::read_sources(&bytes).map_err(|e| CmdError::Failed(e.to_string()))?;
        let mut added = false;
        // The tags and (title, author-or-editor) keys already here, once: an import of thousands
        // of untagged sources must not rescan the list per source. Tags follow `auto_tag`: the
        // stem, then 1, 2… while taken.
        let mut tags: std::collections::HashSet<String> = s.doc.sources.iter().map(|x| x.tag.clone()).collect();
        let mut keys: std::collections::HashSet<(String, String)> =
            s.doc.sources.iter().map(|x| (x.title.clone(), by_author(x).to_string())).collect();
        let mut next: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for mut src in file.sources {
            if s.doc.sources.len() >= wordcraft_docx::MAX_SOURCES {
                break;
            }
            if src.tag.is_empty() {
                // A source with no tag is matched to one already here by its title and author (or
                // editor), as repeated parts of a document are; otherwise it gets a tag the way a
                // new source does.
                if keys.contains(&(src.title.clone(), by_author(&src).to_string())) {
                    continue;
                }
                let base = auto_tag_base(&src);
                let n = next.entry(base.clone()).or_insert(0);
                let mut tag = if *n == 0 { base.clone() } else { format!("{base}{}", *n) };
                while tags.contains(&tag) {
                    *n += 1;
                    tag = format!("{base}{}", *n);
                }
                *n = n.saturating_add(1);
                src.tag = tag;
            }
            if !tags.insert(src.tag.clone()) {
                continue;
            }
            keys.insert((src.title.clone(), by_author(&src).to_string()));
            s.doc.sources.push(src);
            added = true;
        }
        if added {
            s.touch();
        }
        Ok(())
    }
}

/// The library format a file's extension names, or `None` to sniff the content.
#[cfg(not(target_arch = "wasm32"))]
fn library_ext(path: &str, bytes: &[u8]) -> Option<&'static str> {
    match std::path::Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("bib") => Some("bib"),
        Some("ris") => Some("ris"),
        Some("json") => Some("json"),
        _ => sniffed_format(bytes),
    }
}

/// The citation-library format of the content, when it can be told: an RIS record opening the
/// file, CSL-JSON items, or a BibTeX `@`-entry opening a line anywhere.
#[cfg(not(target_arch = "wasm32"))]
fn sniffed_format(b: &[u8]) -> Option<&'static str> {
    /// The slice from its first non-whitespace byte.
    fn after_ws(x: &[u8]) -> &[u8] {
        x.iter().position(|c| !c.is_ascii_whitespace()).and_then(|i| x.get(i..)).unwrap_or(&[])
    }
    let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(b);
    let t = after_ws(b);
    // `TY  - …` (spacing around the hyphen tolerated): an RIS record opens the file.
    if t.get(..2).is_some_and(|x| x.eq_ignore_ascii_case(b"TY")) && after_ws(t.get(2..).unwrap_or(&[])).first() == Some(&b'-') {
        return Some("ris");
    }
    if t.first() == Some(&b'{') || t.first() == Some(&b'[') {
        let top: Value = serde_json::from_slice(b).ok()?;
        let items = match &top {
            Value::Array(a) => a.as_slice(),
            Value::Object(o) => o.get("items").and_then(Value::as_array).map_or(&[][..], Vec::as_slice),
            _ => &[],
        };
        if items.iter().any(|it| it.as_object().is_some_and(|o| o.contains_key("id") || o.contains_key("type"))) {
            return Some("json");
        }
    }
    // An `@`-entry opening a line anywhere: BibTeX.
    b.split(|c| *c == b'\n').any(|line| after_ws(line).first() == Some(&b'@')).then_some("bib")
}

/// Whether a parsed source holds any field worth keeping: entries with nothing usable are
/// skipped by an import, not added empty.
#[cfg(not(target_arch = "wasm32"))]
fn has_any_field(src: &Source) -> bool {
    [
        &src.author,
        &src.title,
        &src.year,
        &src.journal,
        &src.volume,
        &src.pages,
        &src.publisher,
        &src.city,
        &src.url,
        &src.editor,
        &src.edition,
        &src.issue,
        &src.month,
        &src.day,
        &src.accessed,
        &src.doi,
    ]
    .iter()
    .any(|f| !f.is_empty())
}

/// Import a citation library — BibTeX, RIS or CSL-JSON, the formats reference managers export —
/// into Manage Sources (`references.importSources`). A library is keyed data: a source whose tag
/// is already here is updated in place, unlike a Word bibliography `import`, which leaves an
/// existing tag alone. A source with no usable field is skipped with a reason, and one with no
/// tag gets a tag the way a new source does.
pub(super) fn import_library(s: &mut Session, v: &Value) -> CmdResult {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, v);
        return Err(CmdError::Failed("reading files by path isn't available on the web".into()));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = p::req_str(v, "path")?;
        let forced = match p::str(v, "format").unwrap_or("auto").to_ascii_lowercase().as_str() {
            "auto" => None,
            "bibtex" => Some("bib"),
            "ris" => Some("ris"),
            "csl-json" => Some("json"),
            other => return Err(CmdError::Params(format!("`format` must be bibtex, ris, csl-json or auto, not `{other}`"))),
        };
        // Refused by size before it is read in: the file is named by the user and could be any
        // size on disk. The 1 KiB margin lets the parser's own message govern files between the caps.
        let cap = wordcraft_formats::MAX_LIBRARY_BYTES as u64 + 1024;
        let bytes = crate::io::read_file_capped(path, cap, "source library").map_err(|e| CmdError::Params(e.to_string()))?;
        let ext = forced
            .or_else(|| library_ext(path, &bytes))
            .ok_or_else(|| CmdError::Params(format!("{path}: not a BibTeX, RIS or CSL-JSON library; give `format` or a .bib/.ris/.json file")))?;
        let sources = wordcraft_formats::import_sources(ext, &bytes)
            .unwrap_or_else(|| Err("not a BibTeX, RIS or CSL-JSON library".into()))
            .map_err(|e| CmdError::Params(format!("{path}: {e}")))?;
        let mut added = 0;
        let mut updated = 0;
        let mut skipped: Vec<Value> = Vec::new();
        // Each tag's place in the list, and the tags taken, once: an update-in-place or a minted
        // tag must not rescan the sources per entry (#4890). Minted tags follow `auto_tag`: the
        // stem, then 1, 2… while taken.
        let mut index: std::collections::HashMap<String, usize> = s.doc.sources.iter().enumerate().map(|(i, x)| (x.tag.clone(), i)).collect();
        let mut next: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for mut src in sources {
            if src.tag.is_empty() && !has_any_field(&src) {
                skipped.push(json!({"reason": "no usable field"}));
                continue;
            }
            if src.tag.is_empty() {
                let base = auto_tag_base(&src);
                let n = next.entry(base.clone()).or_insert(0);
                let mut tag = if *n == 0 { base.clone() } else { format!("{base}{}", *n) };
                while index.contains_key(&tag) {
                    *n += 1;
                    tag = format!("{base}{}", *n);
                }
                *n = n.saturating_add(1);
                src.tag = tag;
            }
            match index.get(&src.tag).copied() {
                Some(i) => {
                    if let Some(x) = s.doc.sources.get_mut(i) {
                        *x = src;
                    }
                    updated += 1;
                }
                None if s.doc.sources.len() >= wordcraft_docx::MAX_SOURCES => {
                    // The document keeps no more sources than a DOCX file holds; past the cap
                    // a save would silently drop them on reopen (#1058).
                    skipped.push(json!({"tag": src.tag, "reason": "the document's source list is full"}));
                }
                None => {
                    index.insert(src.tag.clone(), s.doc.sources.len());
                    s.doc.sources.push(src);
                    added += 1;
                }
            }
        }
        if added + updated > 0 {
            update_citations(s)?;
            s.touch();
        }
        let total = added + updated + skipped.len();
        Ok(json!({"added": added, "updated": updated, "skipped": skipped, "total": total}))
    }
}

pub(super) fn citation(s: &mut Session, v: &Value) -> CmdResult {
    if v.get("source").is_some() {
        sources(s, &json!({"add": v.get("source")}))?;
    }
    let tag = match p::str(v, "tag") {
        Some(t) => t.to_string(),
        None => match v.get("source") {
            Some(_) => s.doc.sources.last().map(|x| x.tag.clone()).unwrap_or_default(),
            None => return Err(CmdError::Params("give `tag` or `source`".into())),
        },
    };
    let src = s.doc.sources.iter().find(|x| x.tag == tag).cloned().ok_or_else(|| CmdError::Params(format!("no source `{tag}`")))?;
    let pages = p::str(v, "pages").unwrap_or("").to_string();
    // The style, the number and the first result are settled before anything is deleted or
    // inserted: a missing CSL style is an error that changes nothing.
    let (result, refresh) = {
        let st = session_style(s)?;
        let order = bib_order(&s.doc, &st, crate::cmd::default_lang(s));
        let n = number_of(&s.doc, &order, &tag);
        // Inserted as a first citation; the refresh below sets the positions (CSL) straight.
        (st.cite(&src, Position::First, n, &pages), st.numbered() || matches!(st, StyleRef::Csl(_)))
    };
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let instr = if pages.is_empty() { format!("CITATION {tag}") } else { format!("CITATION {tag} \\p {pages}") };
    let end =
        s.doc.insert_object(&at, InlineObject::Field { instr, result, locked: false, dirty: false, code: false, form: String::new() }, &props)?;
    s.sel = Selection::caret(end);
    // A new citation can renumber the others (numbered styles count in order of first
    // citation) and moves the CSL positions of the citations around it.
    if refresh {
        refresh_citations(s)?;
    }
    Ok(json!({"tag": tag}))
}

/// The tag and `\p` pages of a CITATION field instruction.
fn citation_field(instr: &str) -> Option<(&str, &str)> {
    let mut words = instr.split_whitespace();
    if words.next()? != "CITATION" {
        return None;
    }
    let tag = words.next()?;
    let pages = instr.split("\\p").nth(1).map(str::trim).unwrap_or("");
    Some((tag, pages))
}

/// Tags cited in the body, in order of first citation.
pub(super) fn cited_tags(doc: &Document) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        for o in &p.objects {
            if let InlineObject::Field { instr, .. } = o
                && let Some((tag, _)) = citation_field(instr)
                && seen.insert(tag.to_string())
            {
                out.push(tag.to_string());
            }
        }
    }
    out
}

/// A source's place in an author-sorted bibliography: the corporate author, else the first
/// author's surname, else the title.
fn author_key(src: &Source) -> String {
    corporate(src)
        .map(str::to_lowercase)
        .or_else(|| authors(&src.author).first().map(|a| a.0.to_lowercase()))
        .unwrap_or_else(|| src.title.to_lowercase())
}

/// Indices into `doc.sources` in bibliography order: the cited sources (every source while
/// nothing is cited), sorted or in order of first citation as the style asks. A CSL style
/// orders by its bibliography's `<sort>` keys — first-cited when it sorts nothing.
pub(super) fn bib_order(doc: &Document, st: &StyleRef, lang: Option<String>) -> Vec<usize> {
    let b = match st {
        StyleRef::Built(b) => b,
        StyleRef::Csl(style) => return csl_order(doc, style),
    };
    let cited = cited_tags(doc);
    let mut idx: Vec<usize> = if cited.is_empty() {
        (0..doc.sources.len()).collect()
    } else if b.order == Order::Cited {
        cited.iter().filter_map(|t| doc.sources.iter().position(|x| &x.tag == t)).collect()
    } else {
        let cited: std::collections::HashSet<&str> = cited.iter().map(String::as_str).collect();
        doc.sources.iter().enumerate().filter(|(_, x)| cited.contains(x.tag.as_str())).map(|(i, _)| i).collect()
    };
    // Sorted in the document's language (Polish authors `Łukasiewicz` before `Mazur`).
    let key = |i: &usize, by_title: bool| {
        sort_key(&doc.sources.get(*i).map(|x| if by_title { x.title.clone() } else { author_key(x) }).unwrap_or_default(), lang.as_deref())
    };
    match b.order {
        Order::Author => idx.sort_by_cached_key(|i| key(i, false)),
        Order::Title => idx.sort_by_cached_key(|i| key(i, true)),
        Order::Cited => {}
    }
    idx
}

/// A source's number in a numbered style: its place in the bibliography order.
fn number_of(doc: &Document, order: &[usize], tag: &str) -> usize {
    order.iter().position(|&i| doc.sources.get(i).is_some_and(|x| x.tag == tag)).unwrap_or(order.len()).saturating_add(1)
}

/// Refresh CITATION results and generated lists.
pub fn update_citations(s: &mut Session) -> Result<(), CmdError> {
    refresh_citations(s)?;
    update_generated(s)
}

/// Refresh the results of the CITATION fields in the body.
fn refresh_citations(s: &mut Session) -> Result<(), CmdError> {
    // Gathered against the document as it stands, then applied, so the style borrow ends
    // before a paragraph is touched. A missing CSL style errors before anything changes.
    let updates: Vec<(Path, Vec<(usize, String)>)> = {
        let st = session_style(s)?;
        let order = bib_order(&s.doc, &st, crate::cmd::default_lang(s));
        // CSL positions: a citation is the first of its source, a later one, or directly
        // follows the same source's citation (ibid).
        let mut seen = std::collections::HashSet::new();
        let mut prev: Option<String> = None;
        let mut all = Vec::new();
        for path in s.doc.para_paths(StoryRef::Body) {
            let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
            let updates: Vec<(usize, String)> = p
                .objects
                .iter()
                .enumerate()
                .filter_map(|(k, o)| {
                    let InlineObject::Field { instr, .. } = o else { return None };
                    let (tag, pages) = citation_field(instr)?;
                    let position = if prev.as_deref() == Some(tag) {
                        Position::Ibid
                    } else if seen.insert(tag.to_string()) {
                        Position::First
                    } else {
                        Position::Subsequent
                    };
                    prev = Some(tag.to_string());
                    let src = s.doc.sources.iter().find(|x| x.tag == tag)?;
                    Some((k, st.cite(src, position, number_of(&s.doc, &order, tag), pages)))
                })
                .collect();
            if !updates.is_empty() {
                all.push((path, updates));
            }
        }
        all
    };
    for (path, updates) in updates {
        let para = s.doc.para_mut(StoryRef::Body, &path)?;
        for (k, r) in updates {
            if let Some(InlineObject::Field { result, .. }) = para.objects.get_mut(k) {
                *result = r;
            }
        }
        para.touch();
    }
    Ok(())
}
