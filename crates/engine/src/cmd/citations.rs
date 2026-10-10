//! References: citations and bibliography (APA, MLA, Chicago, IEEE), source manager, index,
//! table of figures, cross-references, table of authorities, footnote options.

use serde_json::{Value, json};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{CharProps, TabAlign, TabLeader, TabStop};
use wordcraft_doc::{Block, Paragraph, Path, Pos, Source, StoryRef};

use super::{delete_selection, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

pub const STYLES: [&str; 4] = ["APA", "MLA", "Chicago", "IEEE"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("references.sources", "Manage Sources", "References › Citations & Bibliography", sources)
            .params(r#"{"add"?: Source, "remove"?: tag} → list"#),
        CommandSpec::new("references.citation", "Insert Citation", "References › Citations & Bibliography", citation).params(r#"{"tag"?: string, "source"?: Source (added if new), "pages"?: string}"#),
        CommandSpec::new("references.citationStyle", "Style", "References › Citations & Bibliography", |s, v| {
            let st = p::str(v, "style").unwrap_or("APA");
            let st = STYLES.iter().find(|x| x.eq_ignore_ascii_case(st)).ok_or_else(|| CmdError::Params(format!("style must be one of {STYLES:?}")))?;
            s.bib_style = st.to_string();
            update_citations(s)?;
            Ok(json!({"style": s.bib_style}))
        })
        .params(r#"{"style": "APA|MLA|Chicago|IEEE"}"#),
        CommandSpec::new("references.bibliography", "Bibliography", "References › Citations & Bibliography", |s, v| {
            let title = p::str(v, "title").unwrap_or(if s.bib_style == "MLA" { "Works Cited" } else { "References" }).to_string();
            generated_list(s, "BIBLIOGRAPHY", &title)?;
            sel_result(s)
        })
        .params(r#"{"title"?: string}"#),
        CommandSpec::new("references.tableOfFigures", "Insert Table of Figures", "References › Captions", |s, v| {
            let label = p::str(v, "label").unwrap_or("Figure").to_string();
            generated_list(s, &format!("TOC \\c \"{label}\""), "")?;
            sel_result(s)
        })
        .params(r#"{"label"?: "Figure|Table|Equation"}"#),
        CommandSpec::new("references.updateFigures", "Update Table", "References › Captions", |s, _| {
            update_generated(s)?;
            sel_result(s)
        }),
        CommandSpec::new("references.markEntry", "Mark Entry", "References › Index", |s, v| {
            let entry = match p::str(v, "entry") {
                Some(e) => e.to_string(),
                None => s.selected_text().trim().to_string(),
            };
            if entry.is_empty() {
                return Err(CmdError::Params("select a word or give `entry`".into()));
            }
            let (_, b) = s.sel.ordered();
            s.doc.insert_object(&b, InlineObject::Field { instr: format!("XE \"{}\"", entry.replace('"', "")), result: String::new(), locked: false }, &CharProps { hidden: Some(true), ..Default::default() })?;
            Ok(json!({"entry": entry}))
        })
        .params(r#"{"entry"?: string}"#),
        CommandSpec::new("references.index", "Insert Index", "References › Index", |s, _| {
            generated_list(s, "INDEX", "Index")?;
            sel_result(s)
        }),
        CommandSpec::new("references.updateIndex", "Update Index", "References › Index", |s, _| {
            update_generated(s)?;
            sel_result(s)
        }),
        CommandSpec::new("references.markCitation", "Mark Citation", "References › Table of Authorities", |s, v| {
            let entry = p::str(v, "entry").map(str::to_string).unwrap_or_else(|| s.selected_text().trim().to_string());
            if entry.is_empty() {
                return Err(CmdError::Params("select a citation or give `entry`".into()));
            }
            let (_, b) = s.sel.ordered();
            s.doc.insert_object(&b, InlineObject::Field { instr: format!("TA \\l \"{}\"", entry.replace('"', "")), result: String::new(), locked: false }, &CharProps { hidden: Some(true), ..Default::default() })?;
            Ok(json!({"entry": entry}))
        })
        .params(r#"{"entry"?: string}"#),
        CommandSpec::new("references.tableOfAuthorities", "Insert Table of Authorities", "References › Table of Authorities", |s, _| {
            generated_list(s, "TOA", "Table of Authorities")?;
            sel_result(s)
        }),
        CommandSpec::new("insert.crossReference", "Cross-reference", "Insert › Links", cross_ref)
            .params(r#"{"to": "heading|bookmark|figure|table", "target": string (text / name / number), "show"?: "text|page|number|aboveBelow"} or {} to list targets"#),
        CommandSpec::new("references.noteOptions", "Footnote and Endnote", "References › Footnotes", |s, v| {
            if let Some(f) = p::str(v, "footnoteFormat") {
                s.doc.settings.footnote_format = wordcraft_doc::section::NumFormat::from_ooxml(f);
            }
            if let Some(f) = p::str(v, "endnoteFormat") {
                s.doc.settings.endnote_format = wordcraft_doc::section::NumFormat::from_ooxml(f);
            }
            Ok(json!({"footnoteFormat": s.doc.settings.footnote_format, "endnoteFormat": s.doc.settings.endnote_format}))
        })
        .params(r#"{"footnoteFormat"?: "decimal|lowerRoman|upperRoman|lowerLetter|upperLetter", "endnoteFormat"?: …}"#),
        CommandSpec::new("references.researcher", "Researcher", "References › Research", |s, _| {
            s.status = "Researcher needs an online service; WordCraft keeps your documents offline.".into();
            Ok(json!({"available": false}))
        })
        .pure(),
    ]
}

fn authors(a: &str) -> Vec<(String, String)> {
    a.split(';')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(|x| match x.split_once(',') {
            Some((last, first)) => (last.trim().to_string(), first.trim().to_string()),
            None => match x.rsplit_once(' ') {
                Some((first, last)) => (last.trim().to_string(), first.trim().to_string()),
                None => (x.to_string(), String::new()),
            },
        })
        .collect()
}

fn initials(first: &str) -> String {
    first.split_whitespace().filter_map(|w| w.chars().next()).map(|c| format!("{c}.")).collect::<Vec<_>>().join(" ")
}

/// In-text citation for a style.
pub fn cite(src: &Source, style: &str, n: usize, pages: &str) -> String {
    let au = authors(&src.author);
    let lasts: Vec<&str> = au.iter().map(|a| a.0.as_str()).collect();
    let who = match lasts.len() {
        0 => src.title.clone(),
        1 => lasts.first().copied().unwrap_or("").to_string(),
        2 => format!("{} {} {}", lasts.first().copied().unwrap_or(""), if style == "APA" { "&" } else { "and" }, lasts.get(1).copied().unwrap_or("")),
        _ => format!("{} et al.", lasts.first().copied().unwrap_or("")),
    };
    match style {
        "MLA" => {
            if pages.is_empty() {
                format!("({who})")
            } else {
                format!("({who} {pages})")
            }
        }
        "Chicago" => {
            if pages.is_empty() {
                format!("({who} {})", src.year)
            } else {
                format!("({who} {}, {pages})", src.year)
            }
        }
        "IEEE" => format!("[{n}]"),
        _ => {
            if pages.is_empty() {
                format!("({who}, {})", src.year)
            } else {
                format!("({who}, {}, p. {pages})", src.year)
            }
        }
    }
}

/// Bibliography entry for a style.
pub fn entry(src: &Source, style: &str, n: usize) -> String {
    let au = authors(&src.author);
    let year = if src.year.is_empty() { "n.d.".to_string() } else { src.year.clone() };
    let pubinfo = [src.city.as_str(), src.publisher.as_str()].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join(": ");
    let journal = if src.journal.is_empty() {
        String::new()
    } else {
        format!(
            "{}{}{}",
            src.journal,
            if src.volume.is_empty() { String::new() } else { format!(", {}", src.volume) },
            if src.pages.is_empty() { String::new() } else { format!(", {}", src.pages) }
        )
    };
    let url = if src.url.is_empty() { String::new() } else { format!(" {}", src.url) };
    match style {
        "MLA" => {
            let names = match au.len() {
                0 => String::new(),
                1 => au.first().map(|(l, f)| format!("{l}, {f}. ")).unwrap_or_default(),
                _ => au.first().map(|(l, f)| format!("{l}, {f}, et al. ")).unwrap_or_default(),
            };
            let container = if journal.is_empty() { src.publisher.clone() } else { journal };
            format!("{names}\u{201C}{}.\u{201D} {container}, {year}.{url}", src.title)
        }
        "Chicago" => {
            let names = au
                .iter()
                .enumerate()
                .map(|(i, (l, f))| if i == 0 { format!("{l}, {f}") } else { format!("{f} {l}") })
                .collect::<Vec<_>>()
                .join(", and ");
            let rest = if journal.is_empty() { pubinfo } else { journal };
            format!("{names}. {year}. {}. {rest}.{url}", src.title)
        }
        "IEEE" => {
            let names = au.iter().map(|(l, f)| format!("{} {l}", initials(f))).collect::<Vec<_>>().join(", ");
            let rest = if journal.is_empty() { pubinfo } else { journal };
            format!("[{n}] {names}, \u{201C}{},\u{201D} {rest}, {year}.{url}", src.title)
        }
        _ => {
            let names = au.iter().map(|(l, f)| format!("{l}, {}", initials(f))).collect::<Vec<_>>();
            let names = match names.len() {
                0 => String::new(),
                1 => names.first().cloned().unwrap_or_default(),
                _ => format!(
                    "{}, & {}",
                    names.get(..names.len() - 1).map(|x| x.join(", ")).unwrap_or_default(),
                    names.last().cloned().unwrap_or_default()
                ),
            };
            let rest = if journal.is_empty() { src.publisher.clone() } else { journal };
            format!("{names} ({year}). {}. {rest}.{url}", src.title)
        }
    }
}

fn sources(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(a) = v.get("add") {
        let mut src: Source = serde_json::from_value(a.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
        if src.tag.is_empty() {
            let base: String = authors(&src.author)
                .first()
                .map(|x| x.0.clone())
                .unwrap_or_else(|| "Source".into())
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect();
            src.tag = format!("{base}{}", src.year);
        }
        s.doc.sources.retain(|x| x.tag != src.tag);
        s.doc.sources.push(src);
        s.touch();
    }
    if let Some(t) = p::str(v, "remove") {
        s.doc.sources.retain(|x| x.tag != t);
    }
    Ok(serde_json::to_value(&s.doc.sources).unwrap_or(Value::Null))
}

fn citation(s: &mut Session, v: &Value) -> CmdResult {
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
    let n = s.doc.sources.iter().position(|x| x.tag == tag).unwrap_or(0) + 1;
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let instr = if pages.is_empty() { format!("CITATION {tag}") } else { format!("CITATION {tag} \\p {pages}") };
    let end = s.doc.insert_object(&at, InlineObject::Field { instr, result: cite(&src, &s.bib_style, n, &pages), locked: false }, &props)?;
    s.sel = Selection::caret(end);
    Ok(json!({"tag": tag}))
}

/// Refresh CITATION results and generated lists.
pub fn update_citations(s: &mut Session) -> Result<(), CmdError> {
    let style = s.bib_style.clone();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        let updates: Vec<(usize, String)> = p
            .objects
            .iter()
            .enumerate()
            .filter_map(|(k, o)| match o {
                InlineObject::Field { instr, .. } if instr.trim_start().starts_with("CITATION") => {
                    let mut it = instr.split_whitespace().skip(1);
                    let tag = it.next()?.to_string();
                    let pages = instr.split("\\p").nth(1).map(|x| x.trim().to_string()).unwrap_or_default();
                    let n = s.doc.sources.iter().position(|x| x.tag == tag)? + 1;
                    let src = s.doc.sources.get(n - 1)?;
                    Some((k, cite(src, &style, n, &pages)))
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
    update_generated(s)
}

/// Generated lists (BIBLIOGRAPHY, INDEX, table of figures, TOA): a heading paragraph holding
/// the field, then entry paragraphs marked with the "GeneratedEntry" style family.
fn generated_list(s: &mut Session, instr: &str, title: &str) -> Result<(), CmdError> {
    let at = delete_selection(s)?;
    if at.story != StoryRef::Body || at.path.depth() > 0 {
        return Err(CmdError::Failed("lists go in the main text".into()));
    }
    let mut head = if title.is_empty() { Paragraph::new() } else { Paragraph::with_text(title, CharProps::default()).styled("Heading1") };
    let end = head.len();
    head.insert_object(end, InlineObject::Field { instr: instr.to_string(), result: String::new(), locked: false }, &CharProps::default())?;
    let i = if s.doc.para_at(&at).is_some_and(|p| p.is_empty()) { at.path.last() } else { s.doc.split_paragraph(&at)?.path.last() };
    s.doc.insert_block(StoryRef::Body, &Path::top(i), Block::Para(head))?;
    update_generated(s)
}

fn list_kind(p: &Paragraph) -> Option<String> {
    p.objects.iter().find_map(|o| match o {
        InlineObject::Field { instr, .. } => {
            let t = instr.trim_start();
            (t.starts_with("BIBLIOGRAPHY") || t.starts_with("INDEX") || t.starts_with("TOA") || t.starts_with("TOC \\c")).then(|| t.to_string())
        }
        _ => None,
    })
}

const ENTRY_STYLE: &str = "ListEntry";

/// Rebuild every generated list.
pub fn update_generated(s: &mut Session) -> Result<(), CmdError> {
    if s.doc.styles.get(ENTRY_STYLE).is_none() {
        s.doc.styles.upsert(wordcraft_doc::Style {
            id: ENTRY_STYLE.into(),
            name: "List Entry".into(),
            kind: wordcraft_doc::StyleKind::Paragraph,
            based_on: Some("Normal".into()),
            para: wordcraft_doc::ParaProps { space_after: Some(4.0), ..Default::default() },
            hidden: true,
            ..Default::default()
        });
    }
    let mut i = 0;
    while i < s.doc.body.len() {
        let kind = s.doc.body.get(i).and_then(|b| b.as_para()).and_then(list_kind);
        let Some(kind) = kind else {
            i += 1;
            continue;
        };
        // Remove old entries.
        while s.doc.body.get(i + 1).and_then(|b| b.as_para()).is_some_and(|p| p.props.style.as_deref() == Some(ENTRY_STYLE)) {
            s.doc.remove_block(StoryRef::Body, &Path::top(i + 1))?;
        }
        let lines = entries_for(s, &kind);
        let width = super::page::sect(s).text_width();
        for (k, (text, hanging)) in lines.into_iter().enumerate() {
            let mut para = Paragraph::with_text(&text, CharProps::default()).styled(ENTRY_STYLE);
            if hanging {
                para.props.indent_left = Some(36.0);
                para.props.indent_first = Some(-36.0);
            }
            if text.contains('\t') {
                para.props.tabs = Some(vec![TabStop { pos: width - 0.5, align: TabAlign::Right, leader: TabLeader::Dot }]);
            }
            s.doc.insert_block(StoryRef::Body, &Path::top(i + 1 + k), Block::Para(para))?;
        }
        i += 1;
    }
    s.touch();
    Ok(())
}

fn page_of(s: &mut Session, pos: &Pos) -> u32 {
    let l = s.layout();
    l.caret(pos).and_then(|c| l.pages.get(c.page)).map(|p| p.number).unwrap_or(1)
}

/// Entry lines (text, hanging indent) for a generated list.
fn entries_for(s: &mut Session, kind: &str) -> Vec<(String, bool)> {
    if kind.starts_with("BIBLIOGRAPHY") {
        let style = s.bib_style.clone();
        let cited: Vec<String> = s
            .doc
            .para_paths(StoryRef::Body)
            .iter()
            .filter_map(|p| s.doc.para(StoryRef::Body, p))
            .flat_map(|p| p.objects.iter())
            .filter_map(|o| match o {
                InlineObject::Field { instr, .. } if instr.trim_start().starts_with("CITATION") => {
                    instr.split_whitespace().nth(1).map(str::to_string)
                }
                _ => None,
            })
            .collect();
        let mut srcs: Vec<(usize, Source)> =
            s.doc.sources.iter().cloned().enumerate().filter(|(_, x)| cited.is_empty() || cited.contains(&x.tag)).collect();
        if style != "IEEE" {
            srcs.sort_by_key(|(_, x)| authors(&x.author).first().map(|a| a.0.to_lowercase()).unwrap_or_else(|| x.title.to_lowercase()));
        }
        return srcs.into_iter().map(|(i, x)| (entry(&x, &style, i + 1), style != "IEEE")).collect();
    }
    // Collect marks with positions.
    let mut marks: Vec<(String, Pos)> = Vec::new();
    let caption_label = kind.split('"').nth(1).unwrap_or("Figure").to_string();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        if kind.starts_with("TOC \\c") {
            if p.props.style.as_deref() == Some("Caption") && p.plain_text().starts_with(&caption_label) {
                marks.push((p.plain_text(), Pos { story: StoryRef::Body, path: path.clone(), off: 0 }));
            }
            continue;
        }
        for off in p.object_offsets() {
            if let Some(InlineObject::Field { instr, .. }) = p.object_at(off) {
                let t = instr.trim_start();
                let want = if kind.starts_with("INDEX") { "XE" } else { "TA" };
                if t.starts_with(want) {
                    let e = t.split('"').nth(1).unwrap_or("").to_string();
                    marks.push((e, Pos { story: StoryRef::Body, path: path.clone(), off }));
                }
            }
        }
    }
    let mut with_pages: Vec<(String, u32)> = marks.into_iter().map(|(e, _)| (e, 0)).collect();
    // Pages (layout once).
    let positions: Vec<Pos> = {
        let mut v = Vec::new();
        for path in s.doc.para_paths(StoryRef::Body) {
            let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
            if kind.starts_with("TOC \\c") {
                if p.props.style.as_deref() == Some("Caption") && p.plain_text().starts_with(&caption_label) {
                    v.push(Pos { story: StoryRef::Body, path: path.clone(), off: 0 });
                }
                continue;
            }
            for off in p.object_offsets() {
                if let Some(InlineObject::Field { instr, .. }) = p.object_at(off) {
                    let want = if kind.starts_with("INDEX") { "XE" } else { "TA" };
                    if instr.trim_start().starts_with(want) {
                        v.push(Pos { story: StoryRef::Body, path: path.clone(), off });
                    }
                }
            }
        }
        v
    };
    for (i, pos) in positions.iter().enumerate() {
        let pg = page_of(s, pos);
        if let Some(x) = with_pages.get_mut(i) {
            x.1 = pg;
        }
    }
    if kind.starts_with("TOC \\c") {
        return with_pages.into_iter().map(|(t, p)| (format!("{t}\t{p}"), false)).collect();
    }
    // Index / TOA: group by entry, sort, list pages.
    let mut grouped: std::collections::BTreeMap<String, Vec<u32>> = Default::default();
    for (e, p) in with_pages {
        let pages = grouped.entry(e).or_default();
        if !pages.contains(&p) {
            pages.push(p);
        }
    }
    let mut out = Vec::new();
    let mut letter = None;
    for (e, pages) in grouped {
        let first = e.chars().next().map(|c| c.to_ascii_uppercase());
        if kind.starts_with("INDEX") && first != letter {
            letter = first;
            if let Some(l) = first {
                out.push((l.to_string(), false));
            }
        }
        let list = pages.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
        out.push(if kind.starts_with("INDEX") { (format!("{e}, {list}"), true) } else { (format!("{e}\t{list}"), false) });
    }
    out
}

fn cross_ref(s: &mut Session, v: &Value) -> CmdResult {
    let to = p::str(v, "to").unwrap_or("heading");
    // Targets.
    let mut targets: Vec<(String, Pos)> = Vec::new();
    match to {
        "bookmark" => targets = s.doc.bookmarks(),
        "figure" | "table" | "equation" => {
            let label = match to {
                "table" => "Table",
                "equation" => "Equation",
                _ => "Figure",
            };
            for path in s.doc.para_paths(StoryRef::Body) {
                if let Some(p) = s.doc.para(StoryRef::Body, &path)
                    && p.props.style.as_deref() == Some("Caption")
                    && p.plain_text().starts_with(label)
                {
                    targets.push((p.plain_text(), Pos { story: StoryRef::Body, path: path.clone(), off: 0 }));
                }
            }
        }
        _ => {
            for path in s.doc.para_paths(StoryRef::Body) {
                if let Some(p) = s.doc.para(StoryRef::Body, &path)
                    && s.doc.styles.resolve_para(&p.props).outline_level.is_some()
                    && !p.plain_text().trim().is_empty()
                {
                    targets.push((p.plain_text().trim().to_string(), Pos { story: StoryRef::Body, path: path.clone(), off: 0 }));
                }
            }
        }
    }
    let Some(target) = p::str(v, "target") else {
        return Ok(Value::Array(targets.iter().map(|(t, _)| json!(t)).collect()));
    };
    let (text, pos) =
        targets.into_iter().find(|(t, _)| t == target || t.starts_with(target)).ok_or_else(|| CmdError::Params(format!("no {to} `{target}`")))?;
    let show = p::str(v, "show").unwrap_or("text");
    let page = page_of(s, &pos);
    let result = match show {
        "page" => page.to_string(),
        "number" => text.split(':').next().unwrap_or(&text).trim().to_string(),
        "aboveBelow" => {
            if pos < s.sel.focus {
                "above".into()
            } else {
                "below".into()
            }
        }
        _ => text.clone(),
    };
    let instr = format!("{} _Ref \\h", if show == "page" { "PAGEREF" } else { "REF" });
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let end = s.doc.insert_object(&at, InlineObject::Field { instr, result: result.clone(), locked: false }, &props)?;
    s.sel = Selection::caret(end);
    Ok(json!({"inserted": result}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    #[test]
    fn citations_and_bibliography() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "Studios matter "})).unwrap();
        s.run("references.citation", &json!({"source": {"author": "Rivera, Alex; Chen, Mei", "title": "Shared Spaces", "year": "2021", "publisher": "Harbor Books", "city": "Portland"}})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).contains("(Rivera & Chen, 2021)"), "{}", s.doc.plain_text(StoryRef::Body));
        s.run("text.newParagraph", &json!({})).unwrap();
        s.run("references.bibliography", &json!({})).unwrap();
        let t = s.doc.plain_text(StoryRef::Body);
        assert!(t.contains("Rivera, A., & Chen, M. (2021). Shared Spaces. Harbor Books."), "{t}");
        s.run("references.citationStyle", &json!({"style": "MLA"})).unwrap();
        let t = s.doc.plain_text(StoryRef::Body);
        assert!(t.contains("(Rivera and Chen)"), "{t}");
        s.run("references.citationStyle", &json!({"style": "IEEE"})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).contains("[1]"));
    }

    #[test]
    fn bibliography_round_trip_docx() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "Studios matter "})).unwrap();
        s.run("references.citation", &json!({"source": {"author": "Rivera, Alex; Chen, Mei", "title": "Shared Spaces", "year": "2021", "publisher": "Harbor Books", "city": "Portland"}})).unwrap();
        s.run("text.newParagraph", &json!({})).unwrap();
        s.run("references.bibliography", &json!({})).unwrap();

        // Serialize to DOCX and back — sources must survive.
        let bytes = wordcraft_docx::write(&s.doc).expect("docx write");
        let doc = wordcraft_docx::read(&bytes).expect("docx read");
        eprintln!("sources after round trip: {:?}", doc.sources);
        assert_eq!(doc.sources.len(), 1, "round-tripped sources: {:?}", doc.sources);
        assert_eq!(doc.sources[0].author, "Rivera, Alex; Chen, Mei");
        assert_eq!(doc.sources[0].title, "Shared Spaces");
    }

    #[test]
    fn index_and_figures() {
        let mut s = Session::new(wordcraft_doc::Document::from_text("Kilns are hot.\nPresses are heavy.\nMore kilns."));
        s.run("select.text", &json!({"text": "Kilns"})).unwrap();
        s.run("references.markEntry", &json!({})).unwrap();
        s.run("select.text", &json!({"text": "Presses"})).unwrap();
        s.run("references.markEntry", &json!({})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("text.newParagraph", &json!({})).unwrap();
        s.run("references.index", &json!({})).unwrap();
        let t = s.doc.plain_text(StoryRef::Body);
        assert!(t.contains("Kilns, 1") && t.contains("Presses, 1"), "{t}");
        s.run("caret.docStart", &json!({})).unwrap();
        s.run("references.caption", &json!({"label": "Figure", "text": "The kiln"})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("references.tableOfFigures", &json!({})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).contains("Figure 1: The kiln\t1"));
        let list = s.run("insert.crossReference", &json!({"to": "figure"})).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);
        s.run("insert.crossReference", &json!({"to": "figure", "target": "Figure 1", "show": "number"})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).contains("Figure 1"));
    }
}
