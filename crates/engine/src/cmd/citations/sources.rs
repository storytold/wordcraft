use super::generated::update_generated;
use super::render::{authors, cite};
use super::styles::{CiteStyle, Order, style};
use super::*;

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

/// The author of a source for matching untagged entries: the editor when there is no author.
#[cfg(not(target_arch = "wasm32"))]
fn by_author(x: &Source) -> &str {
    if x.author.is_empty() { x.editor.as_str() } else { x.author.as_str() }
}

/// The stem of a new source's tag, before `1`, `2`… are added while it is taken: the first three
/// alphanumerics of the first author's name (nothing of the author's, `Src`) and the last two
/// digits of the year.
fn auto_tag_base(src: &Source) -> String {
    let name: String =
        authors(&src.author).first().map(|x| x.0.clone()).unwrap_or_default().chars().filter(|c| c.is_alphanumeric()).take(3).collect();
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
    let st = style(&s.bib_style);
    let n = number_of(&s.doc, &bib_order(&s.doc, st, crate::cmd::default_lang(s)), &tag);
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let instr = if pages.is_empty() { format!("CITATION {tag}") } else { format!("CITATION {tag} \\p {pages}") };
    let end = s.doc.insert_object(
        &at,
        InlineObject::Field { instr, result: cite(&src, st.name, n, &pages), locked: false, dirty: false, code: false, form: String::new() },
        &props,
    )?;
    s.sel = Selection::caret(end);
    // A new citation can renumber the others (numbered styles count in order of first citation).
    if st.numbered() {
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
fn cited_tags(doc: &Document) -> Vec<String> {
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

fn author_key(src: &Source) -> String {
    authors(&src.author).first().map(|a| a.0.to_lowercase()).unwrap_or_else(|| src.title.to_lowercase())
}

/// Indices into `doc.sources` in bibliography order: the cited sources (every source while
/// nothing is cited), sorted or in order of first citation as the style asks.
pub(super) fn bib_order(doc: &Document, st: &CiteStyle, lang: Option<String>) -> Vec<usize> {
    let cited = cited_tags(doc);
    let mut idx: Vec<usize> = if cited.is_empty() {
        (0..doc.sources.len()).collect()
    } else if st.order == Order::Cited {
        cited.iter().filter_map(|t| doc.sources.iter().position(|x| &x.tag == t)).collect()
    } else {
        let cited: std::collections::HashSet<&str> = cited.iter().map(String::as_str).collect();
        doc.sources.iter().enumerate().filter(|(_, x)| cited.contains(x.tag.as_str())).map(|(i, _)| i).collect()
    };
    // Sorted in the document's language (Polish authors `Łukasiewicz` before `Mazur`).
    let key = |i: &usize, by_title: bool| {
        sort_key(&doc.sources.get(*i).map(|x| if by_title { x.title.clone() } else { author_key(x) }).unwrap_or_default(), lang.as_deref())
    };
    match st.order {
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
    let st = style(&s.bib_style);
    let order = bib_order(&s.doc, st, crate::cmd::default_lang(s));
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        let updates: Vec<(usize, String)> = p
            .objects
            .iter()
            .enumerate()
            .filter_map(|(k, o)| match o {
                InlineObject::Field { instr, .. } => {
                    let (tag, pages) = citation_field(instr)?;
                    let src = s.doc.sources.iter().find(|x| x.tag == tag)?;
                    Some((k, cite(src, st.name, number_of(&s.doc, &order, tag), pages)))
                }
                _ => None,
            })
            .collect();
        if updates.is_empty() {
            continue;
        }
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
