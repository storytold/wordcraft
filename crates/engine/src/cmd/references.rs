//! References tab: table of contents, footnotes/endnotes, captions; field updates.

use serde_json::{Value, json};
use wordcraft_doc::para::{InlineObject, NoteKind};
use wordcraft_doc::props::{CharProps, TabAlign, TabLeader, TabStop};
use wordcraft_doc::{Block, Paragraph, PartKind, Path, Pos, StoryRef, para_block};

use super::{delete_selection, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("references.toc", "Table of Contents", "References › Table of Contents", insert_toc).params(
            r#"{"levels"?: 1-9, "title"?: string, "pageNumbers"?: bool, "rightAlign"?: bool, "tabLeader"?: "dot|hyphen|underscore|none", "hyperlinks"?: bool, "headingStyles"?: bool, "outlineLevels"?: bool, "tcFields"?: bool, "styleLevels"?: {style name: 1-9}} (replaces an existing table; {} updates it)"#,
        ),
        CommandSpec::new("references.updateToc", "Update Table", "References › Table of Contents", |s, _| {
            update_toc(s)?;
            sel_result(s)
        }),
        CommandSpec::new("references.removeToc", "Remove Table of Contents", "References › Table of Contents", |s, _| {
            if let Some((start, end)) = toc_range(s) {
                for i in (start..=end).rev() {
                    s.doc.remove_block(StoryRef::Body, &Path::top(i))?;
                }
                s.clamp_selection();
            }
            sel_result(s)
        }),
        CommandSpec::new("references.addText", "Add Text", "References › Table of Contents", |s, v| {
            let level = p::u64(v, "level").unwrap_or(0).min(9) as u8;
            super::para::fmt(s, &|pp| pp.outline_level = if level == 0 { Some(9) } else { Some(level - 1) })
        })
        .params(r#"{"level": 0 (do not show) | 1-9}"#),
        CommandSpec::new("references.footnote", "Insert Footnote", "References › Footnotes", |s, v| note(s, v, NoteKind::Footnote))
            .key("Mod+Alt+F")
            .params(r#"{"text"?: string}"#),
        CommandSpec::new("references.endnote", "Insert Endnote", "References › Footnotes", |s, v| note(s, v, NoteKind::Endnote))
            .key("Mod+Alt+D")
            .params(r#"{"text"?: string}"#),
        CommandSpec::new("references.nextFootnote", "Next Footnote", "References › Footnotes", |s, _| {
            let caret = s.sel.focus.clone();
            let refs = note_refs(s);
            if let Some(p) = refs.iter().find(|p| **p > caret).or(refs.first()) {
                s.sel = Selection::caret(p.clone());
            }
            sel_result(s)
        })
        .pure(),
        CommandSpec::new("references.notes", "Show Notes", "References › Footnotes", |s, _| {
            let mut out = Vec::new();
            for (id, part) in &s.doc.parts {
                if matches!(part.kind, PartKind::Footnote | PartKind::Endnote) {
                    out.push(json!({"id": id, "kind": part.kind, "text": s.doc.plain_text(StoryRef::Part(*id))}));
                }
            }
            Ok(Value::Array(out))
        })
        .pure(),
        CommandSpec::new("references.caption", "Insert Caption", "References › Captions", caption)
            .params(r#"{"label"?: "Figure|Table|Equation|any label", "text"?: string, "position"?: "below|above", "excludeLabel"?: bool, "format"?: "ARABIC|alphabetic|ALPHABETIC|roman|ROMAN", "chapter"?: heading level 1-9, "separator"?: "-|.|:|—|–"}"#),
        CommandSpec::new("references.updateFields", "Update Field", "References", |s, _| {
            update_fields(s)?;
            sel_result(s)
        })
        .key("F9"),
    ]
}

/// A field instruction split into its name, positional arguments and switches (ECMA-376
/// §17.16.1): `TOC \o "1-3" \h` → name `TOC`, switches `o` = `1-3` and `h`. Quoted text stays
/// whole (without the quotes; `\"` and `\\` inside it are a quote and a backslash). A switch
/// takes the word after it as its argument unless that word is another switch.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldCode {
    pub name: String,
    pub args: Vec<String>,
    /// Switch letter (lower-cased; `*`, `@`, `#` for the general switches) and argument.
    pub switches: Vec<(char, Option<String>)>,
}

impl FieldCode {
    pub fn parse(instr: &str) -> FieldCode {
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
                    if c.is_whitespace() || (c == '"' && !w.is_empty()) {
                        break;
                    }
                    w.push(c);
                    chars.next();
                }
                words.push((w, false));
            }
        }
        let mut words = words.into_iter().peekable();
        let mut f =
            FieldCode { name: words.next().map(|(w, _)| w.trim_start_matches('=').to_ascii_uppercase()).unwrap_or_default(), ..Default::default() };
        while let Some((w, quoted)) = words.next() {
            let sw = if quoted { None } else { w.strip_prefix('\\').and_then(|r| r.chars().next()) };
            match sw {
                Some(c) => {
                    let arg = match words.peek() {
                        Some((n, q)) if *q || !n.starts_with('\\') => words.next().map(|(n, _)| n),
                        _ => None,
                    };
                    f.switches.push((c.to_ascii_lowercase(), arg));
                }
                None => f.args.push(w),
            }
        }
        f
    }

    pub fn has(&self, c: char) -> bool {
        self.switches.iter().any(|(k, _)| *k == c)
    }

    /// The argument of switch `c`, if the switch is there with one.
    pub fn arg(&self, c: char) -> Option<&str> {
        self.switches.iter().find(|(k, _)| *k == c).and_then(|(_, a)| a.as_deref())
    }
}

/// A level range such as `1-3` (or one level, `2`), clamped to 1–9.
pub fn level_range(s: &str) -> Option<(u8, u8)> {
    let mut it = s.split('-').map(|x| x.trim().parse::<u8>().ok());
    let a = it.next().flatten()?.clamp(1, 9);
    let b = match it.next() {
        Some(b) => b?.clamp(1, 9),
        None => a,
    };
    Some((a.min(b), a.max(b)))
}

/// The tab leader named in a command parameter.
pub fn leader(name: &str) -> TabLeader {
    match name {
        "none" => TabLeader::None,
        "hyphen" => TabLeader::Hyphen,
        "underscore" => TabLeader::Underscore,
        "middleDot" => TabLeader::MiddleDot,
        _ => TabLeader::Dot,
    }
}

/// The TOC field instruction for `references.toc`'s parameters (ECMA-376 §17.16.5.68): `\o`
/// heading styles, `\h` hyperlinks, `\z` hide tab and page numbers in Web view, `\u` outline
/// levels, `\f`/`\l` TC fields, `\n` without page numbers, `\p` separator, `\t` styles with levels.
pub fn toc_instr(v: &Value) -> String {
    let levels = p::u64(v, "levels").unwrap_or(3).clamp(1, 9);
    let on = |k: &str, d: bool| p::bool(v, k).unwrap_or(d);
    let mut s = String::from("TOC");
    if on("headingStyles", true) {
        s.push_str(&format!(" \\o \"1-{levels}\""));
    }
    if on("hyperlinks", true) {
        s.push_str(" \\h");
    }
    s.push_str(" \\z");
    if on("outlineLevels", true) {
        s.push_str(" \\u");
    }
    if on("tcFields", false) {
        s.push_str(&format!(" \\f \\l \"1-{levels}\""));
    }
    if !on("pageNumbers", true) {
        s.push_str(&format!(" \\n \"1-{levels}\""));
    } else if !on("rightAlign", true) {
        s.push_str(" \\p \" \"");
    }
    let clean = |n: &str| n.chars().filter(|c| !matches!(c, '"' | ',' | '\\')).take(64).collect::<String>().trim().to_string();
    let pairs: Vec<String> = match v.get("styleLevels") {
        Some(Value::Object(m)) => m
            .iter()
            .take(64)
            .filter_map(|(name, l)| Some((clean(name), l.as_u64()?.clamp(1, 9))))
            .filter(|(n, _)| !n.is_empty())
            .map(|(n, l)| format!("{n},{l}"))
            .collect(),
        _ => Vec::new(),
    };
    if !pairs.is_empty() {
        s.push_str(&format!(" \\t \"{}\"", pairs.join(",")));
    }
    s
}

/// A TOC entry: the paragraph it points at, its level (0-based), its text, and the leading page
/// breaks of that paragraph (its page is where the text after them lands).
struct TocEntry {
    path: Path,
    level: u8,
    text: String,
    lead: usize,
}

/// The heading level (1–9) of a built-in heading style.
fn heading_style_level(p: &Paragraph) -> Option<u8> {
    p.props.style.as_deref()?.strip_prefix("Heading")?.trim().parse::<u8>().ok().filter(|l| (1..=9).contains(l))
}

/// The entries a TOC field collects from the body, in document order: built-in heading styles
/// in the `\o` range, paragraphs with an outline level (`\u`), styles listed in `\t`, and TC
/// fields (`\f`, levels `\l`).
fn toc_entries(s: &Session, f: &FieldCode) -> Vec<TocEntry> {
    let o = f.has('o').then(|| f.arg('o').and_then(level_range).unwrap_or((1, 9)));
    let max = o.map(|r| r.1).unwrap_or(9);
    let styles: Vec<(String, u8)> = f
        .arg('t')
        .map(|t| {
            let parts: Vec<&str> = t.split([',', ';']).map(str::trim).collect();
            parts.chunks(2).filter_map(|c| Some((c.first()?.to_string(), c.get(1)?.parse::<u8>().ok()?.clamp(1, 9)))).collect()
        })
        .unwrap_or_default();
    let tc = f.has('f').then(|| f.arg('f').unwrap_or("").to_ascii_uppercase());
    let tc_levels = f.arg('l').and_then(level_range).unwrap_or((1, 9));
    let mut v = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        if path.depth() > 0 {
            continue;
        }
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        let style = p.props.style.as_deref().unwrap_or("Normal");
        if style.starts_with("TOC") {
            continue;
        }
        let lead = p.text.bytes().take_while(|b| *b == 0x0C).count();
        let by_style =
            styles.iter().find(|(n, _)| s.doc.styles.get(style).is_some_and(|st| st.name.eq_ignore_ascii_case(n)) || n == style).map(|(_, l)| *l);
        // `Some(9)` direct outline level is body text: "Do Not Show in Table of Contents".
        let by_outline = f.has('u').then_some(p.props.outline_level).flatten().filter(|l| *l < 9).map(|l| l + 1).filter(|l| *l <= max);
        let hidden = f.has('u') && p.props.outline_level == Some(9);
        let by_heading = o.and_then(|(a, b)| {
            let l = heading_style_level(p).or_else(|| {
                s.doc
                    .styles
                    .resolve_para(&wordcraft_doc::ParaProps { style: p.props.style.clone(), ..Default::default() })
                    .outline_level
                    .map(|l| l + 1)
            })?;
            (a..=b).contains(&l).then_some(l)
        });
        if let Some(l) = by_style.or(by_outline).or(if hidden { None } else { by_heading }) {
            let t = p.plain_text().replace(['\u{000C}', '\n', '\t'], " ").trim().to_string();
            if !t.is_empty() {
                v.push(TocEntry { path: path.clone(), level: l - 1, text: t, lead });
            }
        }
        if let Some(id) = &tc {
            for off in p.object_offsets() {
                let Some(InlineObject::Field { instr, .. }) = p.object_at(off) else { continue };
                let c = FieldCode::parse(instr);
                if c.name != "TC" {
                    continue;
                }
                let kind = c.arg('f').unwrap_or("C").to_ascii_uppercase();
                let l = c.arg('l').and_then(|l| l.trim().parse::<u8>().ok()).unwrap_or(1).clamp(1, 9);
                let text = c.args.first().map(|t| t.trim().to_string()).unwrap_or_default();
                if (id.is_empty() || *id == kind) && (tc_levels.0..=tc_levels.1).contains(&l) && !text.is_empty() {
                    v.push(TocEntry { path: path.clone(), level: l - 1, text, lead: off });
                }
            }
        }
    }
    v
}

/// The TOC block range in the body: the paragraph holding the TOC field and the TOC entries after
/// it. A TOC with `\c` is a table of figures, kept by the generated lists.
fn toc_range(s: &Session) -> Option<(usize, usize)> {
    let start = s.doc.body.iter().position(|b| b.as_para().is_some_and(|p| toc_field(p).is_some()))?;
    let mut end = start;
    while let Some(Block::Para(p)) = s.doc.body.get(end + 1).map(|b| &**b) {
        if p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading") {
            end += 1;
        } else {
            break;
        }
    }
    Some((start, end))
}

/// The TOC field instruction in `p` (not a table of figures).
fn toc_field(p: &Paragraph) -> Option<String> {
    p.objects.iter().find_map(|o| match o {
        InlineObject::Field { instr, .. } => {
            let f = FieldCode::parse(instr);
            (f.name == "TOC" && !f.has('c')).then(|| instr.clone())
        }
        _ => None,
    })
}

fn insert_toc(s: &mut Session, v: &Value) -> CmdResult {
    let replace = v.as_object().is_some_and(|m| !m.is_empty());
    // Replacing a table puts the new one where the old one was, in place of an empty paragraph.
    let mut placeholder = false;
    let at = match toc_range(s) {
        // `{}` updates the table there; settings replace it where it is.
        Some(_) if !replace => {
            update_toc(s)?;
            return sel_result(s);
        }
        Some((start, end)) => {
            for i in (start..=end).rev() {
                s.doc.remove_block(StoryRef::Body, &Path::top(i))?;
            }
            s.doc.insert_block(StoryRef::Body, &Path::top(start), Block::Para(Paragraph::new()))?;
            placeholder = true;
            s.sel = Selection::caret(Pos { story: StoryRef::Body, path: Path::top(start), off: 0 });
            s.sel.focus.clone()
        }
        None => delete_selection(s)?,
    };
    let title = p::str(v, "title").unwrap_or("Contents").to_string();
    if at.story != StoryRef::Body || at.path.depth() > 0 {
        return Err(CmdError::Failed("a table of contents goes in the main text".into()));
    }
    // Title paragraph carrying the TOC field marker.
    let mut head = Paragraph::with_text(&title, CharProps::default()).styled("TOCHeading");
    head.insert_object(head.len(), InlineObject::Field { instr: toc_instr(v), result: String::new(), locked: false }, &CharProps::default())?;
    let i = if s.doc.para_at(&at).is_some_and(|p| p.is_empty()) {
        if placeholder {
            s.doc.remove_block(StoryRef::Body, &at.path)?;
        }
        at.path.last()
    } else {
        s.doc.split_paragraph(&at)?.path.last()
    };
    s.doc.insert_block(StoryRef::Body, &Path::top(i), Block::Para(head))?;
    let leader = leader(p::str(v, "tabLeader").unwrap_or("dot"));
    regenerate_toc(s, Some(leader))?;
    s.clamp_selection();
    sel_result(s)
}

/// A hidden `_Toc` bookmark around the heading at body block `i` (after its leading page breaks),
/// reusing one already there; its name.
fn toc_bookmark(s: &mut Session, i: usize, lead: usize) -> Result<String, CmdError> {
    const MARK: usize = wordcraft_doc::para::OBJ.len_utf8();
    let para = s.doc.para(StoryRef::Body, &Path::top(i)).ok_or_else(|| CmdError::Failed("heading is gone".into()))?;
    if let Some(InlineObject::BookmarkStart { name }) = para.object_at(lead)
        && name.starts_with("_Toc")
    {
        return Ok(name.clone());
    }
    let next =
        s.doc.bookmarks().iter().filter_map(|(n, _)| n.strip_prefix("_Toc").and_then(|d| d.parse::<u64>().ok())).max().unwrap_or(0).saturating_add(1);
    let name = format!("_Toc{next:09}");
    let end = para.len();
    let props = CharProps::default();
    let pos = |off| Pos { story: StoryRef::Body, path: Path::top(i), off };
    s.doc.insert_object(&pos(end), InlineObject::BookmarkEnd { name: name.clone() }, &props)?;
    s.doc.insert_object(&pos(lead), InlineObject::BookmarkStart { name: name.clone() }, &props)?;
    // The caret in that paragraph stays on the same text.
    let shift = |p: &mut Pos| {
        if p.story == StoryRef::Body && p.path == Path::top(i) {
            if p.off > end {
                p.off = p.off.saturating_add(2 * MARK);
            } else if p.off >= lead && p.off > 0 {
                p.off = p.off.saturating_add(MARK);
            }
        }
    };
    shift(&mut s.sel.anchor);
    shift(&mut s.sel.focus);
    Ok(name)
}

/// Regenerate the TOC entries with page numbers (two layout passes: entries change pagination).
pub fn update_toc(s: &mut Session) -> Result<(), CmdError> {
    regenerate_toc(s, None)
}

/// Rebuild the entries the TOC field's switches ask for. The tab leader is `leader`, or the one
/// the entries had (Word keeps it in the entries' tab stops, not in the field).
fn regenerate_toc(s: &mut Session, leader: Option<TabLeader>) -> Result<(), CmdError> {
    let Some((start, end)) = toc_range(s) else { return Ok(()) };
    let Some(instr) = s.doc.body.get(start).and_then(|b| b.as_para()).and_then(toc_field) else { return Ok(()) };
    let f = FieldCode::parse(&instr);
    let leader = leader.unwrap_or_else(|| {
        s.doc
            .body
            .get(start + 1)
            .filter(|_| end > start)
            .and_then(|b| b.as_para())
            .and_then(|p| p.props.tabs.as_ref()?.last().map(|t| t.leader))
            .unwrap_or(TabLeader::Dot)
    });
    for i in (start + 1..=end).rev() {
        s.doc.remove_block(StoryRef::Body, &Path::top(i))?;
    }
    let width = super::page::sect(s).text_width();
    let entries = toc_entries(s, &f);
    let links: Vec<Option<String>> = if f.has('h') {
        let mut v = Vec::new();
        for e in &entries {
            // TC fields link to their paragraph's start.
            let lead = s.doc.para(StoryRef::Body, &e.path).map_or(0, |p| p.text.bytes().take_while(|b| *b == 0x0C).count());
            v.push(Some(format!("#{}", toc_bookmark(s, e.path.last(), lead)?)));
        }
        v
    } else {
        vec![None; entries.len()]
    };
    let no_pages = f.has('n').then(|| f.arg('n').and_then(level_range).unwrap_or((1, 9)));
    let sep = f.arg('p').map(str::to_string).unwrap_or_else(|| "\t".into());
    for pass in 0..2 {
        // Pass 1 lays out with the entries in place, so headings below the TOC sit `entries.len()`
        // blocks further down and their pages account for the TOC's own length.
        s.touch();
        let l = s.layout();
        if pass == 1 {
            // Replace the page numbers now that the entries exist.
            for _ in 0..entries.len() {
                s.doc.remove_block(StoryRef::Body, &Path::top(start + 1))?;
            }
        }
        let shift = if pass == 0 { 0 } else { entries.len() };
        for (k, e) in entries.iter().enumerate() {
            let target = Path::top(e.path.last() + if e.path.last() > start { shift } else { 0 });
            let page =
                l.caret(&Pos { story: StoryRef::Body, path: target, off: e.lead }).and_then(|c| l.pages.get(c.page)).map(|pg| pg.number).unwrap_or(1);
            let show_page = no_pages.is_none_or(|(a, b)| !(a..=b).contains(&(e.level + 1)));
            let text = if show_page { format!("{}{sep}{page}", e.text) } else { e.text.clone() };
            let props = CharProps { link: links.get(k).cloned().flatten(), ..Default::default() };
            let mut para = Paragraph::with_text(&text, props).styled(&format!("TOC{}", e.level + 1));
            para.mark = CharProps::default();
            if show_page && sep.contains('\t') {
                para.props.tabs = Some(vec![TabStop { pos: width - 0.5, align: TabAlign::Right, leader }]);
            }
            s.doc.insert_block(StoryRef::Body, &Path::top(start + 1 + k), Block::Para(para))?;
        }
    }
    s.touch();
    Ok(())
}

fn note(s: &mut Session, v: &Value, kind: NoteKind) -> CmdResult {
    let text = p::str(v, "text").unwrap_or("").to_string();
    let (pk, style, refstyle) = match kind {
        NoteKind::Footnote => (PartKind::Footnote, "FootnoteText", "FootnoteReference"),
        NoteKind::Endnote => (PartKind::Endnote, "EndnoteText", "EndnoteReference"),
    };
    let mut para = Paragraph::new().styled(style);
    let id = s.doc.add_part(pk, Vec::new());
    para.insert_object(
        0,
        InlineObject::NoteRef { kind, id, custom: String::new() },
        &CharProps { style: Some(refstyle.into()), ..Default::default() },
    )?;
    let len = para.len();
    para.insert_text(len, &format!(" {text}"), &CharProps::default())?;
    s.doc.set_story(StoryRef::Part(id), vec![para_block(para)])?;
    let at = delete_selection(s)?;
    s.doc.insert_object(
        &at,
        InlineObject::NoteRef { kind, id, custom: String::new() },
        &CharProps { style: Some(refstyle.into()), ..Default::default() },
    )?;
    // Like Word, the caret moves into the new note.
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Part(id)));
    Ok(json!({"id": id}))
}

fn note_refs(s: &Session) -> Vec<Pos> {
    let mut v = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        for o in p.object_offsets() {
            if matches!(p.object_at(o), Some(InlineObject::NoteRef { .. })) {
                v.push(Pos { story: StoryRef::Body, path: path.clone(), off: o });
            }
        }
    }
    v
}

/// The `\*` format switch value for a caption's number format (OOXML names work too).
pub fn seq_format(name: &str) -> &'static str {
    match name {
        "alphabetic" | "lowerLetter" => "alphabetic",
        "ALPHABETIC" | "upperLetter" => "ALPHABETIC",
        "roman" | "lowerRoman" => "roman",
        "ROMAN" | "upperRoman" => "ROMAN",
        _ => "ARABIC",
    }
}

/// Separators Word offers between a chapter number and a caption number.
pub const CHAPTER_SEPARATORS: [&str; 5] = ["-", ".", ":", "\u{2014}", "\u{2013}"];

/// Insert a caption paragraph below (or above) the caret's paragraph, table or object: the label,
/// a chapter number (`STYLEREF n \s` and a separator) and a `SEQ label \* format \s n` number
/// (ECMA-376 §17.16.5.56, §17.16.5.59).
fn caption(s: &mut Session, v: &Value) -> CmdResult {
    let label: String = p::str(v, "label").unwrap_or("Figure").chars().filter(|c| !matches!(c, '"' | '\\') && !c.is_control()).take(40).collect();
    let label = if label.trim().is_empty() { "Figure".to_string() } else { label.trim().to_string() };
    // SEQ identifiers are one word: Word joins a multi-word label with underscores.
    let ident = label.split_whitespace().collect::<Vec<_>>().join("_");
    let text = p::str(v, "text").unwrap_or("").to_string();
    let above = p::str(v, "position") == Some("above");
    let format = seq_format(p::str(v, "format").unwrap_or("ARABIC"));
    let chapter = p::u64(v, "chapter").filter(|l| (1..=9).contains(l));
    let sep = p::str(v, "separator").and_then(|x| CHAPTER_SEPARATORS.iter().find(|s| **s == x)).copied().unwrap_or("-");
    let mut para = Paragraph::new().styled("Caption");
    let field = |instr: String| InlineObject::Field { instr, result: String::new(), locked: false };
    if !p::bool(v, "excludeLabel").unwrap_or(false) {
        para.insert_text(0, &format!("{label} "), &CharProps::default())?;
    }
    let mut seq = format!("SEQ {ident} \\* {format}");
    if let Some(l) = chapter {
        para.insert_object(para.len(), field(format!("STYLEREF {l} \\s")), &CharProps::default())?;
        para.insert_text(para.len(), sep, &CharProps::default())?;
        seq.push_str(&format!(" \\s {l}"));
    }
    para.insert_object(para.len(), field(seq), &CharProps::default())?;
    if !text.is_empty() {
        para.insert_text(para.len(), &format!(": {text}"), &CharProps::default())?;
    }
    // Next to the caret's top-level block: a paragraph, or the whole table it is in.
    let f = s.sel.focus.clone();
    let top = f.path.0.first().copied().unwrap_or(0) as usize;
    let i = if above { top } else { top.saturating_add(1) };
    s.doc.insert_block(f.story, &Path::top(i), Block::Para(para))?;
    update_seq(s)?;
    let off = s.doc.para(f.story, &Path::top(i)).map_or(0, |p| p.len());
    s.sel = Selection::caret(Pos { story: f.story, path: Path::top(i), off });
    sel_result(s)
}

/// The number part of a heading's list label (`Chapter 2.` → `2`).
fn chapter_number(label: &str) -> String {
    let word = label.split_whitespace().last().unwrap_or("");
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_string()
}

/// Renumber the body's SEQ fields in document order, honouring `\* format`, `\s level` (restart
/// after each heading of that level), `\r n` (reset to n), `\c` (repeat) and `\h` (hide), and
/// fill chapter-number `STYLEREF n \s` fields with the last level-n heading's number (its list
/// label, or its count when the heading isn't numbered).
pub fn update_seq(s: &mut Session) -> Result<(), CmdError> {
    let mut counters = wordcraft_doc::numbering::Counters::default();
    let mut headings = [0u32; 9];
    let mut chapter: [String; 9] = Default::default();
    let mut heading_text: [String; 9] = Default::default();
    let mut seq: std::collections::HashMap<String, (u32, u32)> = Default::default();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        let label = p
            .props
            .numbering
            .or_else(|| s.doc.styles.resolve_para(&p.props).numbering)
            .filter(|n| n.num != 0)
            .and_then(|n| counters.next_label(&s.doc.numbering, n.num, n.level))
            .map(|(l, _)| l);
        if let Some(l) = heading_style_level(p)
            && let Some(k) = (l as usize).checked_sub(1)
            && let Some(count) = headings.get_mut(k)
        {
            *count = count.saturating_add(1);
            let n = label.map(|l| chapter_number(&l)).filter(|n| !n.is_empty()).unwrap_or_else(|| count.to_string());
            if let Some(c) = chapter.get_mut(k) {
                *c = n;
            }
            if let Some(t) = heading_text.get_mut(k) {
                *t = p.plain_text().trim().to_string();
            }
        }
        let mut updates: Vec<(usize, String)> = Vec::new();
        for (k, o) in p.objects.iter().enumerate() {
            let InlineObject::Field { instr, locked: false, .. } = o else { continue };
            let f = FieldCode::parse(instr);
            match f.name.as_str() {
                "SEQ" => {
                    let Some(id) = f.args.first() else { continue };
                    let restart =
                        f.arg('s').and_then(|l| l.parse::<usize>().ok()).and_then(|l| l.checked_sub(1)).and_then(|l| headings.get(l)).copied();
                    let n = seq.entry(id.clone()).or_insert((0, 0));
                    if let Some(epoch) = restart
                        && n.1 != epoch
                    {
                        *n = (0, epoch);
                    }
                    if let Some(r) = f.arg('r').and_then(|r| r.parse::<u32>().ok()) {
                        n.0 = r;
                    } else if !f.has('c') {
                        n.0 = n.0.saturating_add(1);
                    }
                    let fmt = wordcraft_layout::fields::format_switch(instr).unwrap_or_default();
                    updates.push((k, if f.has('h') { String::new() } else { fmt.format(n.0) }));
                }
                "STYLEREF" => {
                    let level = f
                        .args
                        .first()
                        .and_then(|a| a.strip_prefix("Heading").unwrap_or(a).trim().parse::<usize>().ok())
                        .and_then(|l| l.checked_sub(1));
                    let Some(level) = level else { continue };
                    let r = if f.has('s') || f.has('n') { chapter.get(level) } else { heading_text.get(level) };
                    if let Some(r) = r {
                        updates.push((k, r.clone()));
                    }
                }
                _ => {}
            }
        }
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

/// Update fields in the body: dates, SEQ numbering, cross-references, TOC.
pub fn update_fields(s: &mut Session) -> Result<(), CmdError> {
    update_seq(s)?;
    let mut pages: std::collections::HashMap<String, u32> = Default::default();
    for (name, pos) in s.doc.bookmarks() {
        let l = s.layout();
        let page = l.caret(&pos).and_then(|c| l.pages.get(c.page)).map(|p| p.number).unwrap_or(1);
        pages.entry(name).or_insert(page);
    }
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        if !p.objects.iter().any(|o| matches!(o, InlineObject::Field { .. })) {
            continue;
        }
        let mut updates: Vec<(usize, String)> = Vec::new();
        for (k, o) in p.objects.iter().enumerate() {
            let InlineObject::Field { instr, locked: false, .. } = o else { continue };
            let name = wordcraft_layout::fields::field_name(instr);
            match name.as_str() {
                "DATE" | "TIME" | "CREATEDATE" | "SAVEDATE" | "PRINTDATE" => {
                    let pic = date_picture(instr).unwrap_or_else(|| if name == "TIME" { "h:mm am/pm".into() } else { "M/d/yyyy".into() });
                    updates.push((k, super::insert::format_date(&pic)));
                }
                "REF" | "PAGEREF" => {
                    let target = instr.split_whitespace().nth(1).unwrap_or("");
                    let r = if name == "REF" { bookmark_text(&s.doc, target) } else { pages.get(target).map(u32::to_string) };
                    if let Some(r) = r {
                        updates.push((k, r));
                    }
                }
                "TITLE" => updates.push((k, s.doc.core.title.clone())),
                "AUTHOR" => updates.push((k, s.doc.core.creator.clone())),
                "NUMWORDS" => updates.push((k, s.doc.word_count().to_string())),
                _ => {}
            }
        }
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
    super::citations::update_citations(s)?;
    update_toc(s)
}

/// A field's `\@` date picture: the quoted text after it (switches may follow), or one word.
pub fn date_picture(instr: &str) -> Option<String> {
    let rest = instr.split_once("\\@")?.1.trim_start();
    let pic = match rest.strip_prefix('"') {
        Some(q) => q.split('"').next().unwrap_or(""),
        None => rest.split_whitespace().next().unwrap_or(""),
    };
    (!pic.is_empty()).then(|| pic.to_string())
}

/// The text a bookmark marks (field results included), up to its end in the same paragraph or
/// the paragraph's end. `None` when there is no such bookmark.
pub fn bookmark_text(doc: &wordcraft_doc::Document, name: &str) -> Option<String> {
    let (_, start) = doc.bookmarks().into_iter().find(|(n, _)| n == name)?;
    let para = doc.para(start.story, &start.path)?;
    let end = para
        .object_offsets()
        .into_iter()
        .find(|o| *o > start.off && matches!(para.object_at(*o), Some(InlineObject::BookmarkEnd { name: n }) if n == name))
        .unwrap_or(para.len());
    Some(doc.copy_range(&start, &Pos { off: end, ..start.clone() }).plain_text().trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(s: &Session) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for path in s.doc.para_paths(StoryRef::Body) {
            for o in &s.doc.para(StoryRef::Body, &path).unwrap().objects {
                if let InlineObject::Field { instr, result, .. } = o {
                    out.push((instr.clone(), result.clone()));
                }
            }
        }
        out
    }

    /// #402: Custom Table of Contents settings become TOC switches, and updating honours them:
    /// levels (`\o`), a style with a level (`\t`), no hyperlinks, page numbers after a space
    /// (`\p`), or none at all (`\n`).
    #[test]
    fn toc_switches_shape_the_entries() {
        let mut s = Session::new(wordcraft_doc::Document::from_text("Intro\nPart\nDeep\nNote"));
        for (text, style) in [("Intro", "Heading 1"), ("Part", "Heading 2"), ("Deep", "Heading 3"), ("Note", "Quote")] {
            s.run("select.text", &json!({"text": text})).unwrap();
            s.run("styles.apply", &json!({"style": style})).unwrap();
        }
        s.run("caret.docStart", &json!({})).unwrap();
        s.run("references.toc", &json!({"levels": 2, "hyperlinks": false, "rightAlign": false, "styleLevels": {"Quote": 2}})).unwrap();
        assert_eq!(fields(&s)[0].0, "TOC \\o \"1-2\" \\z \\u \\p \" \" \\t \"Quote,2\"");
        let entries: Vec<(String, String)> = s
            .doc
            .body
            .iter()
            .filter_map(|b| b.as_para())
            .filter(|p| p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading"))
            .map(|p| (p.props.style.clone().unwrap(), p.plain_text()))
            .collect();
        assert_eq!(entries, [("TOC1", "Intro 1"), ("TOC2", "Part 1"), ("TOC2", "Note 1")].map(|(a, b)| (a.to_string(), b.to_string())));
        assert!(s.doc.bookmarks().is_empty(), "no hyperlinks, no _Toc bookmarks");
        // Settings again replace the table in place; hyperlinks bookmark the headings.
        s.run("references.toc", &json!({"levels": 3, "pageNumbers": false})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Body).matches("Contents").count(), 1);
        let toc: Vec<String> =
            s.doc.body.iter().filter_map(|b| b.as_para()).filter(|p| p.props.style.as_deref() == Some("TOC3")).map(|p| p.plain_text()).collect();
        assert_eq!(toc, ["Deep"]);
        let first = s.doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.props.style.as_deref() == Some("TOC1")).unwrap();
        let link = first.runs[0].props.link.clone().unwrap();
        assert!(s.doc.bookmarks().iter().any(|(n, _)| format!("#{n}") == link), "{link}");
    }

    /// #402: Caption numbering — a custom label above the paragraph, letters, and chapter
    /// numbers that restart after each Heading 1 (`STYLEREF 1 \s`, `SEQ … \s 1`).
    #[test]
    fn caption_numbering_with_chapters() {
        let mut s = Session::new(wordcraft_doc::Document::from_text("One\nPic\nPic two\nTwo\nPic three"));
        for t in ["One", "Two"] {
            s.run("select.text", &json!({"text": t})).unwrap();
            s.run("styles.apply", &json!({"style": "Heading 1"})).unwrap();
        }
        let opts = json!({"label": "Photo Plate", "format": "upperLetter", "chapter": 1, "separator": "."});
        for t in ["Pic three", "Pic two", "Pic"] {
            s.run("select.text", &json!({"text": t})).unwrap();
            let mut v = opts.clone();
            v["position"] = json!("above");
            v["text"] = json!(t);
            s.run("references.caption", &v).unwrap();
        }
        let caps: Vec<String> =
            s.doc.body.iter().filter_map(|b| b.as_para()).filter(|p| p.props.style.as_deref() == Some("Caption")).map(|p| p.plain_text()).collect();
        assert_eq!(caps, ["Photo Plate 1.A: Pic", "Photo Plate 1.B: Pic two", "Photo Plate 2.A: Pic three"]);
        assert!(fields(&s).contains(&("SEQ Photo_Plate \\* ALPHABETIC \\s 1".to_string(), "A".to_string())), "{:?}", fields(&s));
        // The caption sits above its paragraph; excluding the label leaves the number.
        let i = s.doc.body.iter().position(|b| b.as_para().is_some_and(|p| p.plain_text() == "Pic")).unwrap();
        assert_eq!(s.doc.body[i - 1].as_para().unwrap().plain_text(), "Photo Plate 1.A: Pic");
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("references.caption", &json!({"label": "Table", "excludeLabel": true, "format": "ROMAN"})).unwrap();
        assert_eq!(s.doc.body.last().unwrap().as_para().unwrap().plain_text(), "I");
    }
}
