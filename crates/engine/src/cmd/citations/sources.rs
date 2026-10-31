use super::generated::update_generated;
use super::render::{authors, cite, corporate};
use super::styles::{CiteStyle, Order, style};
use super::{CmdError, CmdResult, Document, InlineObject, Selection, Session, Source, StoryRef, Value, delete_selection, json, p, sort_key};

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
    let quoted_tag = tag.chars().any(|c| c.is_whitespace() || c == '"' || c == '\\').then(|| quote_field_arg(&tag));
    let tag_arg = quoted_tag.as_deref().unwrap_or(&tag);
    let instr = if pages.is_empty() { format!("CITATION {tag_arg}") } else { format!("CITATION {tag_arg} \\p {}", quote_field_arg(&pages)) };
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

fn quote_field_arg(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The end of a complete quoted argument, using FieldCode's escape rules.
fn quoted_argument_end(value: &str) -> Option<usize> {
    let mut chars = value.strip_prefix('"')?.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        match c {
            '"' => {
                let end = at.checked_add(2)?;
                let rest = value.get(end..)?;
                return rest.chars().next().is_none_or(char::is_whitespace).then_some(end);
            }
            '\\' if matches!(chars.peek(), Some((_, '"' | '\\'))) => {
                chars.next();
            }
            _ => {}
        }
    }
    None
}

/// Older insertions wrote tags verbatim. Resolve an exact stored literal tag without
/// rewriting its instruction; an existing decoded quoted tag keeps its parsed meaning.
fn citation_code(instr: &str, source_tags: &std::collections::HashSet<String>) -> super::FieldCode {
    let field = super::FieldCode::parse(instr);
    if field.name != "CITATION" {
        return field;
    }
    let trimmed = instr.trim_start();
    let Some(rest) = trimmed.find(char::is_whitespace).and_then(|at| trimmed.get(at..)) else { return field };
    let rest = rest.trim_start();
    let quoted_end = quoted_argument_end(rest);
    if !rest.starts_with('\\') && quoted_end.is_none() {
        // Older insertions also wrote multiword tags verbatim. Keep the whole tag up to
        // the first switch, including when it is missing, rather than citing its prefix.
        let switch = rest.char_indices().find(|(at, c)| {
            *c == '\\'
                && rest.get(*at..).and_then(|tail| tail.chars().nth(1)).is_some_and(|next| !next.is_whitespace())
                && rest.get(..*at).is_some_and(|head| head.ends_with(char::is_whitespace))
        });
        let raw = rest.get(..switch.map_or(rest.len(), |(at, _)| at)).unwrap_or(rest).trim_end();
        if raw.chars().any(char::is_whitespace) {
            let Some(tail) = rest.get(raw.len()..) else { return field };
            return super::FieldCode::parse(&format!("CITATION {}{tail}", quote_field_arg(raw)));
        }
    }
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let Some(raw) = rest.get(..end) else { return field };
    let unquoted_literal = if raw.starts_with('"') { quoted_end.is_none() } else { raw.contains('"') || raw.starts_with('\\') };
    if !unquoted_literal && field.args.first().is_some_and(|tag| source_tags.contains(tag)) {
        return field;
    }
    // A missing legacy source must not resolve to a shorter parsed prefix, and a complete
    // quoted argument must not resolve to its first raw word.
    if !unquoted_literal && (!source_tags.contains(raw) || quoted_end.is_some_and(|end| end != raw.len())) {
        return field;
    }
    let Some(tail) = rest.get(end..) else { return field };
    super::FieldCode::parse(&format!("CITATION {}{tail}", quote_field_arg(raw)))
}

/// The tag and `\p` pages of a CITATION field instruction.
fn citation_field(instr: &str, source_tags: &std::collections::HashSet<String>) -> Option<(String, String)> {
    let field = citation_code(instr, source_tags);
    if field.name != "CITATION" {
        return None;
    }
    let tag = field.args.into_iter().next()?;
    let pages = field.switches.into_iter().find(|(key, _)| *key == 'p').and_then(|(_, arg)| arg).unwrap_or_default();
    Some((tag, pages))
}

/// Tags cited in the body, including each `\m` source, in order of first citation.
fn cited_tags(doc: &Document) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let source_tags: std::collections::HashSet<String> = doc.sources.iter().map(|source| source.tag.clone()).collect();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        for o in &p.objects {
            let InlineObject::Field { instr, .. } = o else { continue };
            let field = citation_code(instr, &source_tags);
            if field.name != "CITATION" {
                continue;
            }
            let Some(primary) = field.args.first() else { continue };
            let secondary = field.switches.iter().filter(|(key, _)| *key == 'm').filter_map(|(_, arg)| arg.as_ref());
            for tag in std::iter::once(primary).chain(secondary) {
                if seen.insert(tag.clone()) {
                    out.push(tag.clone());
                }
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
    let source_tags: std::collections::HashSet<String> = s.doc.sources.iter().map(|source| source.tag.clone()).collect();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        let updates: Vec<(usize, String)> = p
            .objects
            .iter()
            .enumerate()
            .filter_map(|(k, o)| match o {
                InlineObject::Field { instr, .. } => {
                    let (tag, pages) = citation_field(instr, &source_tags)?;
                    let src = s.doc.sources.iter().find(|x| x.tag == tag)?;
                    Some((k, cite(src, st.name, number_of(&s.doc, &order, &tag), &pages)))
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
