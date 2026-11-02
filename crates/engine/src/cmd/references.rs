//! References tab: table of contents, footnotes/endnotes, captions; field updates.

use serde_json::{Value, json};
use wordcraft_doc::numbering::{Counters, LevelSuffix};
use wordcraft_doc::para::{InlineObject, NoteKind, OBJ};
use wordcraft_doc::props::{CharProps, TabAlign, TabLeader, TabStop};
use wordcraft_doc::section::NumFormat;
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
                remove_toc(s, start, end)?;
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
            .params(r#"{"text"?: string, "mark"?: string (a custom mark instead of the number)}"#),
        CommandSpec::new("references.endnote", "Insert Endnote", "References › Footnotes", |s, v| note(s, v, NoteKind::Endnote))
            // macOS takes ⌘⌥D to show and hide the Dock; Word for Mac uses ⌘⌥E.
            .key(super::mac_or("Mod+Alt+E", "Mod+Alt+D"))
            .params(r#"{"text"?: string, "mark"?: string (a custom mark instead of the number)}"#),
        CommandSpec::new("references.convertNotes", "Convert Notes", "References › Footnotes", |s, v| {
            let (foot, end) = match p::req_str(v, "to")? {
                "endnote" | "endnotes" => (true, false),
                "footnote" | "footnotes" => (false, true),
                "swap" => (true, true),
                t => return Err(CmdError::Params(format!("unknown `to` {t:?}; use endnote, footnote or swap"))),
            };
            let n = s.doc.convert_notes(foot, end);
            Ok(json!({"converted": n}))
        })
        .params(r#"{"to": "endnote" (footnotes become endnotes) | "footnote" (endnotes become footnotes) | "swap"}"#),
        CommandSpec::new("references.nextFootnote", "Next Footnote", "References › Footnotes", |s, _| {
            let caret = s.sel.focus.clone();
            let refs = note_refs(s, Some(NoteKind::Footnote));
            if let Some(p) = refs.iter().find(|p| **p > caret).or(refs.first()) {
                s.sel = Selection::caret(p.clone());
            }
            sel_result(s)
        })
        .key(super::non_mac("Alt+Shift+> / Alt+Shift+."))
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
        // F9: update fields; the selected ones that showed their code show their result again.
        CommandSpec::new("references.updateFields", "Update Field", "References", |s, v| {
            update_fields_in(s, v.get("all").and_then(Value::as_bool).unwrap_or(false))?;
            super::fields::show_results(s)?;
            sel_result(s)
        })
        .params(r#"{"all"?: bool (the body and every header, footer, note and text box, wherever the caret is; default: the story the caret is in)}"#)
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

    /// A table of figures (`TOC \c` or `TOC \a`, ECMA-376 §17.16.5.68), not a table of contents.
    pub fn is_table_of_figures(&self) -> bool {
        self.name == "TOC" && (self.has('c') || self.has('a'))
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
    let clean = |n: &str| n.chars().filter(|c| !matches!(c, '"' | ',' | ';' | '\\')).take(64).collect::<String>().trim().to_string();
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

/// The headings of the body, in document order, as (body block, level 1–9, text, leading page
/// breaks): what Insert › Link › Place in This Document lists.
pub(crate) fn headings(s: &Session) -> Vec<(usize, u8, String, usize)> {
    let f = FieldCode::parse("TOC \\o \"1-9\" \\u");
    toc_entries(s, &f).into_iter().filter(|e| e.path.depth() == 0).map(|e| (e.path.last(), e.level.saturating_add(1), e.text, e.lead)).collect()
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
    // Every numbered paragraph advances the list counters, so an entry carries the number its
    // heading shows (`1.1`), followed by the level's separator, as Word writes it.
    let mut counters = Counters::default();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        let number = p
            .props
            .numbering
            .or_else(|| s.doc.styles.resolve_para(&p.props).numbering)
            .filter(|n| n.num != 0)
            .and_then(|n| counters.next_label(&s.doc.numbering, n.num, n.level))
            .filter(|(label, def)| !label.is_empty() && !matches!(def.format, NumFormat::Bullet | NumFormat::None))
            .map(|(label, def)| match def.suffix {
                LevelSuffix::Tab => format!("{label}\t"),
                LevelSuffix::Space => format!("{label} "),
                LevelSuffix::Nothing => label,
            });
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
                let text = number.map_or_else(|| t.clone(), |n| format!("{n}{t}"));
                v.push(TocEntry { path: path.clone(), level: l - 1, text, lead });
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

/// Removes the TOC blocks `start..=end` and returns where the TOC was. A field paragraph that ends
/// a section stays, emptied, so its section break isn't lost (#3889). A heading paragraph of its
/// own just above the field's paragraph (Word's tables start the field after their heading) goes
/// with the table.
fn remove_toc(s: &mut Session, start: usize, end: usize) -> Result<usize, CmdError> {
    for i in (start + 1..=end).rev() {
        s.doc.remove_block(StoryRef::Body, &Path::top(i))?;
    }
    let at = match s.doc.para_mut(StoryRef::Body, &Path::top(start)) {
        Ok(p) if p.section.is_some() => {
            *p = Paragraph { section: p.section.take(), ..Paragraph::new() };
            start + 1
        }
        _ => {
            s.doc.remove_block(StoryRef::Body, &Path::top(start))?;
            start
        }
    };
    let titled = start
        .checked_sub(1)
        .and_then(|i| s.doc.body.get(i))
        .and_then(|b| b.as_para())
        .is_some_and(|p| p.props.style.as_deref() == Some("TOCHeading") && p.section.is_none() && toc_field(p).is_none());
    if titled {
        s.doc.remove_block(StoryRef::Body, &Path::top(start - 1))?;
        return Ok(at - 1);
    }
    Ok(at)
}

/// The caret is in the table of contents (its heading or an entry), where Word's context menu
/// offers Update Field for the whole table.
pub fn caret_in_toc(s: &Session) -> bool {
    let at = &s.sel.focus;
    if at.story != StoryRef::Body || at.path.depth() != 0 {
        return false;
    }
    if toc_range(s).is_some_and(|(start, end)| (start..=end).contains(&at.path.last())) {
        return true;
    }
    // A saved or imported TOC is a range field spanning its entries until Update Table turns it
    // into the engine's TOC (#2182, a save and reopen; not the flow in the report). Only the
    // range Update Table would convert counts, so the menu never refreshes a different TOC.
    s.doc.field_ranges(StoryRef::Body).into_iter().find(|r| convertible_toc_range(s, r)).is_some_and(|r| {
        // The caret at the field's own marker (offset 0 of the first entry) and at the end of
        // the last entry belong to the range as well.
        (r.start.path.last()..=r.end.path.last()).contains(&at.path.last())
    })
}

/// The range field `range_toc_to_field` converts: an unlocked, top-level TOC (not a table of
/// figures) when the body has no engine TOC yet.
fn convertible_toc_range(s: &Session, r: &wordcraft_doc::FieldRange) -> bool {
    let field = FieldCode::parse(&r.instr);
    toc_range(s).is_none()
        && field.name == "TOC"
        && !field.is_table_of_figures()
        && r.depth == 0
        && !r.locked
        && r.start.path.depth() == 0
        && r.end.path.depth() == 0
}

/// A TOC imported as a range field (`FieldStart`…`FieldEnd`: one without a TOC heading, or with
/// content around it the engine's TOC can't reproduce) becomes the engine's TOC before an update,
/// so Update Table refreshes it rather than adding a second one: an empty `TOC` field where the
/// range started, its old result gone, and the regenerated entries after that paragraph (text
/// that followed the range moves below them). Only an unlocked, top-level TOC that isn't inside
/// another range field, and only when the body has no engine TOC already.
fn range_toc_to_field(s: &mut Session) -> Result<(), CmdError> {
    if toc_range(s).is_some() {
        return Ok(());
    }
    let Some(r) = s.doc.field_ranges(StoryRef::Body).into_iter().find(|r| convertible_toc_range(s, r)) else {
        return Ok(());
    };
    let Some(after_end) = r.end.off.checked_add(OBJ.len_utf8()) else { return Ok(()) };
    let end = Pos { off: after_end, ..r.end.clone() };
    let props = s.doc.para_at(&r.start).map(|p| p.props_of_char(r.start.off).clone()).unwrap_or_default();
    let mut at = s.doc.delete_range(&r.start, &end)?;
    // A range that filled its own paragraphs right under a TOC heading (as Word and other writers
    // lay it out) leaves an empty paragraph behind, which would show as a blank line above the
    // entries. The field goes at the end of the heading instead, as `insert_toc` places it.
    let i = at.path.last();
    // Or one holding only the end of Word's TOC control, still styled as an entry: Update Table
    // would delete it with the old entries anyway.
    let leftover = |p: &Paragraph| {
        p.is_empty()
            || (p.objects.iter().all(|o| matches!(o, InlineObject::ControlEnd))
                && p.text.chars().all(|c| c == OBJ)
                && p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading"))
    };
    if at.path.depth() == 0
        && i > 0
        && s.doc.para_at(&at).is_some_and(leftover)
        // The paragraph before may also be the empty one the writer leaves where the engine's TOC
        // field was (the field is saved opening in the first entry), so the next save and update
        // doesn't add a blank line each time (#2182). A blank line the user typed right above a
        // range TOC is reused the same way, so deleting it later deletes the TOC field. So is one
        // ending the section before, as above entries in columns (#3889).
        && s.doc.body.get(i - 1).and_then(|b| b.as_para()).is_some_and(|p| {
            p.props.style.as_deref() == Some("TOCHeading") || (p.is_empty() && p.objects.is_empty())
        })
    {
        s.doc.remove_block(StoryRef::Body, &at.path)?;
        let heading = Path::top(i - 1);
        let len = s.doc.para(StoryRef::Body, &heading).map_or(0, |p| p.len());
        at = Pos { story: StoryRef::Body, path: heading, off: len };
    }
    let field = InlineObject::Field {
        instr: r.instr.trim().to_string(),
        result: String::new(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    };
    let after = s.doc.insert_object(&at, field, &props)?;
    if s.doc.para_at(&after).is_some_and(|p| after.off < p.len()) {
        s.doc.split_paragraph(&after)?;
    }
    s.clamp_selection();
    Ok(())
}

/// The TOC field instruction in `p` (not a table of figures).
fn toc_field(p: &Paragraph) -> Option<String> {
    p.objects.iter().find_map(|o| match o {
        InlineObject::Field { instr, .. } => {
            let f = FieldCode::parse(instr);
            (f.name == "TOC" && !f.is_table_of_figures()).then(|| instr.clone())
        }
        _ => None,
    })
}

/// The first cached number-separator tab formatting at each TOC level.
fn cached_toc_tabs(s: &Session) -> Vec<(String, Vec<TabStop>)> {
    let span = toc_range(s).map(|(start, end)| (start.saturating_add(1), end)).or_else(|| {
        s.doc.field_ranges(StoryRef::Body).into_iter().find_map(|r| {
            let f = FieldCode::parse(&r.instr);
            (r.depth == 0 && !r.locked && r.start.path.depth() == 0 && r.end.path.depth() == 0 && f.name == "TOC" && !f.is_table_of_figures())
                .then_some((r.start.path.last(), r.end.path.last()))
        })
    });
    let Some(blocks) = span.and_then(|(start, end)| s.doc.body.get(start..=end)) else { return Vec::new() };
    let mut tabs: Vec<(String, Vec<TabStop>)> = Vec::new();
    for p in blocks.iter().filter_map(|b| b.as_para()) {
        let Some(style) = &p.props.style else { continue };
        let Some(direct) = &p.props.tabs else { continue };
        if !direct.iter().any(|t| matches!(t.align, TabAlign::Left | TabAlign::Center | TabAlign::Decimal)) {
            continue;
        }
        if !style.starts_with("TOC") || style == "TOCHeading" || tabs.iter().any(|(id, _)| id == style) {
            continue;
        }
        tabs.push((style.clone(), direct.clone()));
    }
    tabs
}

fn insert_toc(s: &mut Session, v: &Value) -> CmdResult {
    let tabs = cached_toc_tabs(s);
    range_toc_to_field(s)?;
    let replace = v.as_object().is_some_and(|m| !m.is_empty());
    // Replacing a table puts the new one where the old one was, in place of an empty paragraph.
    let mut placeholder = false;
    let at = match toc_range(s) {
        // `{}` updates the table there; settings replace it where it is.
        Some(_) if !replace => {
            regenerate_toc(s, None, &tabs)?;
            return sel_result(s);
        }
        Some((start, end)) => {
            let start = remove_toc(s, start, end)?;
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
    head.insert_object(
        head.len(),
        InlineObject::Field { instr: toc_instr(v), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
        &CharProps::default(),
    )?;
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
    regenerate_toc(s, Some(leader), &tabs)?;
    s.clamp_selection();
    sel_result(s)
}

/// A hidden `_Toc` bookmark around the heading at body block `i` (after its leading page breaks),
/// reusing one already there; its name.
pub(crate) fn toc_bookmark(s: &mut Session, i: usize, lead: usize) -> Result<String, CmdError> {
    bookmark_heading(s, &Path::top(i), lead)
}

fn bookmark_heading(s: &mut Session, path: &Path, lead: usize) -> Result<String, CmdError> {
    const MARK: usize = wordcraft_doc::para::OBJ.len_utf8();
    let para = s.doc.para(StoryRef::Body, path).ok_or_else(|| CmdError::Failed("heading is gone".into()))?;
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
    let pos = |off| Pos { story: StoryRef::Body, path: path.clone(), off };
    s.doc.insert_object(&pos(end), InlineObject::BookmarkEnd { name: name.clone() }, &props)?;
    s.doc.insert_object(&pos(lead), InlineObject::BookmarkStart { name: name.clone() }, &props)?;
    // The caret in that paragraph stays on the same text.
    let shift = |p: &mut Pos| {
        if p.story == StoryRef::Body && &p.path == path {
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
    let tabs = cached_toc_tabs(s);
    regenerate_toc(s, None, &tabs)
}

/// Rebuild the entries the TOC field's switches ask for. The tab leader is `leader`, or the one
/// the entries had (Word keeps it in the entries' tab stops, not in the field).
fn regenerate_toc(s: &mut Session, leader: Option<TabLeader>, tabs: &[(String, Vec<TabStop>)]) -> Result<(), CmdError> {
    range_toc_to_field(s)?;
    let Some((start, end)) = toc_range(s) else { return Ok(()) };
    let Some(instr) = s.doc.body.get(start).and_then(|b| b.as_para()).and_then(toc_field) else { return Ok(()) };
    // A TOC field imported with an inline placeholder/cached result must have its result cleared so
    // that the engine's TOC shape matches `toc_span` and the DOCX writer spans the field across entries.
    if let Ok(p) = s.doc.para_mut(StoryRef::Body, &Path::top(start)) {
        for o in &mut p.objects {
            if let InlineObject::Field { instr, result, .. } = o {
                let f = FieldCode::parse(instr);
                if f.name == "TOC" && !f.is_table_of_figures() {
                    result.clear();
                }
            }
        }
        p.touch();
    }
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
    // The column width of the TOC's own section, not the caret's: after Select All the caret sits
    // at the end of the document, possibly in a wider (landscape) section. In columns, the first
    // column's, as Word does (#3889); after a field paragraph ending a section, the next section's.
    let after_break = s.doc.body.get(start).and_then(|b| b.as_para()).is_some_and(|p| p.section.is_some());
    let section = s.doc.section_index_of(start + usize::from(after_break));
    let column = |x: &wordcraft_doc::SectionProps| x.column_boxes().first().map_or(x.text_width(), |(_, w)| *w);
    let width = s.doc.sections().get(section).map_or_else(|| column(&super::page::sect(s)), |(_, x)| column(x));
    let entries = toc_entries(s, &f);
    let links: Vec<Option<String>> = if f.has('h') {
        let mut v = Vec::new();
        for e in &entries {
            // TC fields link to their paragraph's start.
            let lead = s.doc.para(StoryRef::Body, &e.path).map_or(0, |p| p.text.bytes().take_while(|b| *b == 0x0C).count());
            v.push(Some(format!("#{}", bookmark_heading(s, &e.path, lead)?)));
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
            let mut target = e.path.clone();
            if let Some(top) = target.0.first_mut()
                && (*top as usize) > start
            {
                *top = top.saturating_add(u32::try_from(shift).unwrap_or(u32::MAX));
            }
            let page =
                l.caret(&Pos { story: StoryRef::Body, path: target, off: e.lead }).and_then(|c| l.pages.get(c.page)).map(|pg| pg.number).unwrap_or(1);
            let show_page = no_pages.is_none_or(|(a, b)| !(a..=b).contains(&(e.level + 1)));
            let text = if show_page { format!("{}{sep}{page}", e.text) } else { e.text.clone() };
            let props = CharProps { link: links.get(k).cloned().flatten(), ..Default::default() };
            let mut para = Paragraph::with_text(&text, props).styled(&format!("TOC{}", e.level + 1));
            para.mark = CharProps::default();
            if e.text.contains('\t') {
                para.props.tabs = tabs.iter().find(|(style, _)| Some(style) == para.props.style.as_ref()).map(|(_, tabs)| tabs.clone());
            }
            if show_page && sep.contains('\t') {
                let right = TabStop { pos: width - 0.5, align: TabAlign::Right, leader };
                para.props.tabs = Some(entry_tabs(s, &para.props, &e.text, right));
            }
            s.doc.insert_block(StoryRef::Body, &Path::top(start + 1 + k), Block::Para(para))?;
        }
    }
    s.touch();
    Ok(())
}

/// The tab stops of a TOC entry: the right-aligned stop for its page number, and, when the
/// entry text itself has no tab, Clear stops for the left, centre and decimal stops its style
/// sets before that. Those stops are for a number set off by a tab (`1.1<tab>Title`); with the
/// number set off by a space, or no number, a short entry's page tab would stop at one of them
/// (without the leader), and so would the tab on the last line of an entry that wraps.
fn entry_tabs(s: &Session, props: &wordcraft_doc::ParaProps, text: &str, right: TabStop) -> Vec<TabStop> {
    let mut tabs = props.tabs.clone().unwrap_or_default();
    tabs.retain(|t| t.pos < right.pos - 0.5 && !matches!(t.align, TabAlign::Right | TabAlign::Bar));
    if !text.contains('\t') {
        let style = s.doc.styles.resolve_para(&wordcraft_doc::ParaProps { style: props.style.clone(), ..Default::default() });
        for t in style.tabs {
            if t.pos < right.pos - 0.5 && !matches!(t.align, TabAlign::Right | TabAlign::Bar) {
                tabs.push(TabStop { pos: t.pos, align: TabAlign::Clear, leader: TabLeader::None });
            }
        }
    }
    tabs.push(right);
    tabs
}

fn note(s: &mut Session, v: &Value, kind: NoteKind) -> CmdResult {
    let text = p::str(v, "text").unwrap_or("").to_string();
    // A custom mark: a few characters, as Word's dialog takes.
    let custom: String = p::str(v, "mark").unwrap_or("").trim().chars().filter(|c| !c.is_control()).take(10).collect();
    let (pk, style, refstyle) = match kind {
        NoteKind::Footnote => (PartKind::Footnote, "FootnoteText", "FootnoteReference"),
        NoteKind::Endnote => (PartKind::Endnote, "EndnoteText", "EndnoteReference"),
    };
    let mut para = Paragraph::new().styled(style);
    let id = s.doc.try_add_part(pk, Vec::new())?;
    para.insert_object(
        0,
        InlineObject::NoteRef { kind, id, custom: custom.clone() },
        &CharProps { style: Some(refstyle.into()), ..Default::default() },
    )?;
    let len = para.len();
    para.insert_text(len, &format!(" {text}"), &CharProps::default())?;
    s.doc.set_story(StoryRef::Part(id), vec![para_block(para)])?;
    let at = delete_selection(s)?;
    s.doc.insert_object(&at, InlineObject::NoteRef { kind, id, custom }, &CharProps { style: Some(refstyle.into()), ..Default::default() })?;
    // Like Word, the caret moves into the new note.
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Part(id)));
    Ok(json!({"id": id}))
}

/// `references.noteOptions`: footnote or endnote placement, number format, start and restart,
/// for the whole document (each section then follows it) or the selection's sections.
pub(super) fn note_options(s: &mut Session, v: &Value) -> CmdResult {
    use wordcraft_doc::section::{MAX_NOTE_START, NotePos, NoteProps, NoteRestart, NumFormat};
    let fmt = |k: &str| -> Result<Option<NumFormat>, CmdError> {
        match p::str(v, k) {
            None => Ok(None),
            Some(f) if NumFormat::from_ooxml(f).ooxml() == f => Ok(Some(NumFormat::from_ooxml(f))),
            Some(f) => Err(CmdError::Params(format!("unknown number format {f:?}"))),
        }
    };
    // The earlier form: the document's number formats.
    let (old_foot, old_end) = (fmt("footnoteFormat")?, fmt("endnoteFormat")?);
    let endnote = match p::str(v, "kind") {
        None | Some("footnote") => false,
        Some("endnote") => true,
        Some(k) => return Err(CmdError::Params(format!("unknown `kind` {k:?}; use footnote or endnote"))),
    };
    let pos = match p::str(v, "pos") {
        None => None,
        Some(x) => match NotePos::from_ooxml(x) {
            Some(p @ (NotePos::PageBottom | NotePos::BeneathText)) if !endnote => Some(p),
            Some(p @ (NotePos::SectEnd | NotePos::DocEnd)) if endnote => Some(p),
            _ => {
                let ok = if endnote { "sectEnd or docEnd" } else { "pageBottom or beneathText" };
                return Err(CmdError::Params(format!("`pos` {x:?} doesn't apply here; use {ok}")));
            }
        },
    };
    let num_restart = match p::str(v, "numRestart") {
        None => None,
        Some(r) => match NoteRestart::from_ooxml(r) {
            Some(NoteRestart::EachPage) if endnote => return Err(CmdError::Params("endnotes can't start again on each page".into())),
            Some(r) => Some(r),
            None => return Err(CmdError::Params(format!("unknown `numRestart` {r:?}; use continuous, eachSect or eachPage"))),
        },
    };
    let change = NoteProps {
        pos,
        num_fmt: fmt("numFmt")?,
        num_start: p::f32(v, "numStart").map(|n| n.round().clamp(1.0, MAX_NOTE_START as f32) as u32),
        num_restart,
    };
    let whole = match p::str(v, "scope") {
        None | Some("document") => true,
        Some("section") => false,
        Some(x) => return Err(CmdError::Params(format!("unknown `scope` {x:?}; use document or section"))),
    };
    // Set `from`'s fields on `to`; `clear` instead unsets the fields `from` sets.
    let merge = |to: &mut NoteProps, from: &NoteProps, clear: bool| {
        if from.pos.is_some() {
            to.pos = if clear { None } else { from.pos };
        }
        if from.num_fmt.is_some() {
            to.num_fmt = if clear { None } else { from.num_fmt };
        }
        if from.num_start.is_some() {
            to.num_start = if clear { None } else { from.num_start };
        }
        if from.num_restart.is_some() {
            to.num_restart = if clear { None } else { from.num_restart };
        }
    };
    fn pick(sect: &mut wordcraft_doc::SectionProps, endnote: bool) -> &mut NoteProps {
        if endnote { &mut sect.endnote_pr } else { &mut sect.footnote_pr }
    }
    if change.is_empty() {
        // Nothing to change: the options in effect.
    } else if whole {
        let st = &mut s.doc.settings;
        if let Some(f) = change.num_fmt {
            *(if endnote { &mut st.endnote_format } else { &mut st.footnote_format }) = f;
        }
        merge(if endnote { &mut st.endnote_pr } else { &mut st.footnote_pr }, &NoteProps { num_fmt: None, ..change }, false);
        // Every section follows the document now.
        let ends: Vec<usize> = s.doc.sections().iter().map(|(e, _)| *e).collect();
        for e in ends {
            merge(pick(s.doc.section_mut(e), endnote), &change, true);
        }
    } else {
        super::page::with_sect(s, |sect| merge(pick(sect, endnote), &change, false))?;
    }
    if let Some(f) = old_foot {
        s.doc.settings.footnote_format = f;
    }
    if let Some(f) = old_end {
        s.doc.settings.endnote_format = f;
    }
    let sect = super::page::sect(s);
    Ok(json!({
        "footnote": s.doc.note_options(&sect, false),
        "endnote": s.doc.note_options(&sect, true),
        "footnoteFormat": s.doc.settings.footnote_format,
        "endnoteFormat": s.doc.settings.endnote_format,
    }))
}

fn note_refs(s: &Session, kind: Option<NoteKind>) -> Vec<Pos> {
    let mut v = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        for o in p.object_offsets() {
            if let Some(InlineObject::NoteRef { kind: k, .. }) = p.object_at(o)
                && (kind.is_none() || kind == Some(*k))
            {
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
    let field = |instr: String| InlineObject::Field { instr, result: String::new(), locked: false, dirty: false, code: false, form: String::new() };
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
    // A selected picture, shape or chart: next to the paragraph holding it, in the same cell or
    // text box (#1230). Otherwise next to the caret's top-level block: a paragraph, or the whole
    // table it is in.
    let f = s.sel.focus.clone();
    let at = match super::objects::object_selection(s) {
        Some((pos, _)) => {
            let k = pos.path.last();
            pos.path.with_last(if above { k } else { k.saturating_add(1) })
        }
        None => {
            let top = f.path.0.first().copied().unwrap_or(0) as usize;
            Path::top(if above { top } else { top.saturating_add(1) })
        }
    };
    s.doc.insert_block(f.story, &at, Block::Para(para))?;
    update_seq(s)?;
    let off = s.doc.para(f.story, &at).map_or(0, |p| p.len());
    s.sel = Selection::caret(Pos { story: f.story, path: at, off });
    sel_result(s)
}

/// Renumber the body's SEQ fields in document order, honouring `\* format`, `\s level` (restart
/// after each heading of that level), `\r n` (reset to n), `\c` (repeat) and `\h` (hide), and
/// fill body STYLEREF fields ([`wordcraft_layout::styleref`]): chapter numbers (`STYLEREF n \s`,
/// the last level-n heading's number, or its count when the heading isn't numbered) and the
/// text or number of the nearest paragraph of a style.
pub fn update_seq(s: &mut Session) -> Result<(), CmdError> {
    let mut headings = [0u32; 9];
    let mut seq: std::collections::HashMap<String, (u32, u32)> = Default::default();
    // Built on the first STYLEREF (field results don't change what it finds).
    let mut styles: Option<wordcraft_layout::styleref::StyleIndex> = None;
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        if let Some(l) = heading_style_level(p)
            && let Some(k) = (l as usize).checked_sub(1)
            && let Some(count) = headings.get_mut(k)
        {
            *count = count.saturating_add(1);
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
                    updates.push((k, if f.has('h') { String::new() } else { wordcraft_layout::fields::field_number(instr, fmt, n.0) }));
                }
                "STYLEREF" => {
                    let ix = styles.get_or_insert_with(|| wordcraft_layout::styleref::StyleIndex::build(&s.doc));
                    if let Some(i) = ix.index_of(&path) {
                        updates.push((k, text_result(instr, ix.eval(instr, wordcraft_layout::styleref::At::Body(i)))));
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
            if let Some(InlineObject::Field { result, dirty, .. }) = para.objects.get_mut(k) {
                // A computed result is no longer stale: Word's F9 clears `w:dirty` too.
                *result = r;
                *dirty = false;
            }
        }
        para.touch();
    }
    Ok(())
}

/// Whether `instr` has the one-letter switch `\sw`, also run together with another (`\p\h`).
fn has_switch(instr: &str, sw: char) -> bool {
    instr.split('\\').skip(1).any(|s| {
        let mut chars = s.chars();
        chars.next().is_some_and(|c| c.eq_ignore_ascii_case(&sw)) && chars.next().is_none_or(|c| !c.is_alphanumeric())
    })
}

/// Update fields of the story being edited, as Word's F9 does: dates, REF/PAGEREF, the document
/// information fields (TITLE, SUBJECT, AUTHOR, KEYWORDS, COMMENTS, LASTSAVEDBY, REVNUM, NUMWORDS,
/// NUMCHARS, FILENAME, DOCPROPERTY) in the body, or in the header, footer, note or text box the
/// caret is in (the body's fields stay as they are then). SEQ numbering, citations and TOC are
/// body-only, so they run only when the caret is in the body.
pub fn update_fields(s: &mut Session) -> Result<(), CmdError> {
    update_fields_in(s, false)
}

/// [`update_fields`], or with `all` the whole document: the body (with SEQ, citations and TOC) and
/// every part story, wherever the caret is (what printing needs).
pub fn update_fields_in(s: &mut Session, all: bool) -> Result<(), CmdError> {
    let caret_story = s.sel.focus.story;
    let in_body = all || caret_story == StoryRef::Body;
    if in_body {
        update_seq(s)?;
    }
    let mut pages: std::collections::HashMap<String, (u32, NumFormat)> = Default::default();
    let formats: Vec<NumFormat> = s.doc.sections().into_iter().map(|(_, sec)| sec.page_num_format).collect();
    let marks = s.doc.bookmarks();
    for (name, pos) in marks {
        let l = s.layout();
        let page_info = l
            .caret(&pos)
            .and_then(|c| l.pages.get(c.page))
            .map(|p| {
                let format = formats.get(p.section).copied().unwrap_or_default();
                (p.number, format)
            })
            .unwrap_or((1, NumFormat::Decimal));
        pages.entry(name).or_insert(page_info);
    }
    let stories: Vec<StoryRef> =
        if all { std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k))).collect() } else { vec![caret_story] };
    // The document properties as the fields read and set them (`SUBJECT "New subject"`): a field
    // after one that sets a property shows the new value.
    let mut core = s.doc.core.clone();
    for (story, path) in stories.into_iter().flat_map(|st| s.doc.para_paths(st).into_iter().map(move |p| (st, p))).collect::<Vec<_>>() {
        let Some(p) = s.doc.para(story, &path) else { continue };
        if !p.objects.iter().any(|o| matches!(o, InlineObject::Field { .. })) {
            continue;
        }
        let mut updates: Vec<(usize, String)> = Vec::new();
        for (k, o) in p.objects.iter().enumerate() {
            let InlineObject::Field { instr, locked: false, .. } = o else { continue };
            // A table formula reads the cells around its own cell again.
            if instr.trim_start().starts_with('=') {
                if let Some((tp, r, c)) = path.cell()
                    && let Some(t) = s.doc.table(story, &tp)
                {
                    let default_lang = super::default_lang(s);
                    let text = match super::table_formula::evaluate_field_with(t, Some(&s.doc), default_lang.as_deref(), r, c, instr) {
                        Ok(text) => text,
                        Err(e) => format!("!{e}"),
                    };
                    updates.push((k, text));
                } else if let Some(fmt) = wordcraft_layout::fields::format_switch(instr) {
                    // `{ = 12345 \* BahtText }` outside a table: evaluate only single literals.
                    // Multi-operand expressions (e.g. `{ = 100 + 50 \* Hex }`) are left
                    // unchanged so their Word-computed cached results are preserved.
                    let token = wordcraft_layout::fields::literal_after_eq(instr);
                    let val = if fmt == NumFormat::BahtText {
                        token.and_then(wordcraft_doc::section::parse_baht_token)
                    } else {
                        token.and_then(|t| t.parse::<f64>().ok())
                    };
                    if fmt == NumFormat::BahtText {
                        if let Some(text) = val.and_then(wordcraft_doc::section::baht_text_f64) {
                            updates.push((k, text));
                        }
                    } else if fmt == NumFormat::DollarText {
                        if let Some(x) = val {
                            if let Some(text) = wordcraft_layout::fields::dollar_text(x) {
                                updates.push((k, wordcraft_layout::fields::apply_case_switch(instr, text)));
                            }
                        }
                    } else if let Some(x) = val.filter(|x| *x >= 0.0 && *x <= f64::from(u32::MAX) && x.fract() == 0.0) {
                        updates.push((k, wordcraft_layout::fields::field_number(instr, fmt, x as u32)));
                    }
                }
                continue;
            }
            let name = wordcraft_layout::fields::field_name(instr);
            match name.as_str() {
                "DATE" | "TIME" | "CREATEDATE" | "SAVEDATE" | "PRINTDATE" => {
                    let pic = date_picture(instr).unwrap_or_else(|| if name == "TIME" { "h:mm am/pm".into() } else { "M/d/yyyy".into() });
                    updates.push((k, text_result(instr, super::insert::format_date(&pic))));
                }
                "REF" | "PAGEREF" => {
                    let f = FieldCode::parse(instr);
                    let target = f.args.first().map(|s| s.split('\\').next().unwrap_or("").trim_matches('"')).unwrap_or("");
                    let r = if name == "REF" {
                        if !has_switch(instr, 'p') { bookmark_text(&s.doc, target) } else { None }
                    } else if !has_switch(instr, 'p') {
                        pages.get(target).map(|(num, page_fmt)| {
                            let fmt = wordcraft_layout::fields::format_switch(instr).unwrap_or(*page_fmt);
                            wordcraft_layout::fields::field_number(instr, fmt, *num)
                        })
                    } else {
                        None
                    };
                    if let Some(r) = r {
                        updates.push((k, if name == "REF" { text_result(instr, r) } else { r }));
                    }
                }
                // The summary properties; a quoted text after the name replaces the property.
                "TITLE" | "SUBJECT" | "AUTHOR" | "KEYWORDS" | "COMMENTS" => {
                    if let Some(v) = new_property_value(instr) {
                        set_summary_property(&mut core, &name, v);
                    }
                    updates.push((k, text_result(instr, summary_property(&core, &name).to_string())));
                }
                "LASTSAVEDBY" => updates.push((k, text_result(instr, core.last_modified_by.clone()))),
                "NUMWORDS" => updates.push((k, count_text(instr, s.doc.word_count()))),
                "NUMCHARS" => {
                    let chars = s.doc.plain_text(StoryRef::Body).chars().filter(|c| !c.is_whitespace()).count();
                    updates.push((k, count_text(instr, chars)));
                }
                "REVNUM" => updates.push((k, count_text(instr, core.revision as usize))),
                // DOCPROPERTY <name>: a custom property first, then a built-in one (#1798).
                "DOCPROPERTY" => {
                    if let Some(name) = doc_property_arg(instr)
                        && let Some(v) = doc_property(&s.doc, &core, &name)
                    {
                        updates.push((k, text_result(instr, v)));
                    }
                }
                "FILENAME" => {
                    let (fn_name, fn_path) = s.resolved_filenames();
                    let text = if has_switch(instr, 'p') { fn_path } else { fn_name };
                    updates.push((k, text_result(instr, text)));
                }
                "BAHTTEXT" => {
                    // `{ BAHTTEXT 12345 }` — number is the second token.
                    // Use parse_baht_token to avoid f64 precision loss (e.g. "1.005" → 1.01).
                    let val = instr.split_whitespace().nth(1).and_then(wordcraft_doc::section::parse_baht_token);
                    if let Some(text) = val.and_then(wordcraft_doc::section::baht_text_f64) {
                        updates.push((k, text));
                    }
                }
                // `{ 12345 \* <fmt> }` — field name IS the number; apply any recognised format switch.
                _ => {
                    if let Some(fmt) = wordcraft_layout::fields::format_switch(instr) {
                        if fmt == wordcraft_doc::section::NumFormat::BahtText {
                            // Use parse_baht_token for precision (e.g. "1.005" → 1.01).
                            let val = wordcraft_doc::section::parse_baht_token(&name);
                            if let Some(text) = val.and_then(wordcraft_doc::section::baht_text_f64) {
                                updates.push((k, text));
                            }
                        } else {
                            let val = name.parse::<f64>().ok();
                            if fmt == wordcraft_doc::section::NumFormat::DollarText {
                                if let Some(x) = val {
                                    if let Some(text) = wordcraft_layout::fields::dollar_text(x) {
                                        updates.push((k, wordcraft_layout::fields::apply_case_switch(instr, text)));
                                    }
                                }
                            } else if let Some(x) = val.filter(|x| *x >= 0.0 && *x <= f64::from(u32::MAX) && x.fract() == 0.0) {
                                updates.push((k, wordcraft_layout::fields::field_number(instr, fmt, x as u32)));
                            }
                        }
                    }
                }
            }
        }
        if core != s.doc.core {
            s.doc.core.clone_from(&core);
        }
        if updates.is_empty() {
            continue;
        }
        let para = s.doc.para_mut(story, &path)?;
        for (k, r) in updates {
            if let Some(InlineObject::Field { result, dirty, .. }) = para.objects.get_mut(k) {
                // A computed result is no longer stale: Word's F9 clears `w:dirty` too.
                *result = r;
                *dirty = false;
            }
        }
        para.touch();
    }
    if !in_body {
        return Ok(());
    }
    super::citations::update_citations(s)?;
    update_toc(s)
}

/// Refresh every unlocked `DATE` and `TIME` field to now, as Word does whenever it shows,
/// prints or exports a document: a letter written from a template carries today's date, not the
/// date the template was last saved (#1672).
///
/// Only those two. `CREATEDATE`, `SAVEDATE` and `PRINTDATE` record when something happened and
/// are not "now"; `REF`, `PAGEREF`, `TOC` and citations are refreshed on an explicit update
/// (`fields.updateAll`) and not on open, again as Word does. A field carrying `w:fldLock` keeps
/// its stored result, which is what the lock is for.
pub fn refresh_date_fields(doc: &mut wordcraft_doc::Document) {
    let stories: Vec<StoryRef> = std::iter::once(StoryRef::Body).chain(doc.parts.keys().copied().map(StoryRef::Part)).collect();
    for story in stories {
        for path in doc.para_paths(story) {
            let Some(p) = doc.para(story, &path) else { continue };
            let mut updates: Vec<(usize, String)> = Vec::new();
            for (k, o) in p.objects.iter().enumerate() {
                let InlineObject::Field { instr, locked: false, .. } = o else { continue };
                let name = wordcraft_layout::fields::field_name(instr);
                if !matches!(name.as_str(), "DATE" | "TIME") {
                    continue;
                }
                // Word's default picture when the code carries no `\@` switch.
                let pic = date_picture(instr).unwrap_or_else(|| if name == "TIME" { "h:mm am/pm".into() } else { "M/d/yyyy".into() });
                updates.push((k, text_result(instr, super::insert::format_date(&pic))));
            }
            if updates.is_empty() {
                continue;
            }
            let Ok(para) = doc.para_mut(story, &path) else { continue };
            for (k, r) in updates {
                if let Some(InlineObject::Field { result, .. }) = para.objects.get_mut(k) {
                    *result = r;
                }
            }
            para.touch();
        }
    }
}

/// The summary property a TITLE, SUBJECT, AUTHOR, KEYWORDS or COMMENTS field shows.
fn summary_property<'a>(core: &'a wordcraft_doc::CoreProps, field: &str) -> &'a str {
    match field {
        "TITLE" => &core.title,
        "SUBJECT" => &core.subject,
        "AUTHOR" => &core.creator,
        "KEYWORDS" => &core.keywords,
        "COMMENTS" => &core.description,
        _ => "",
    }
}

fn set_summary_property(core: &mut wordcraft_doc::CoreProps, field: &str, value: String) {
    match field {
        "TITLE" => core.title = value,
        "SUBJECT" => core.subject = value,
        "AUTHOR" => core.creator = value,
        "KEYWORDS" => core.keywords = value,
        "COMMENTS" => core.description = value,
        _ => {}
    }
}

/// The new value a summary-property field gives its property: the argument after the field's
/// name (`SUBJECT "Quarterly report"`, or one word without quotes: `SUBJECT Draft`), at most 255
/// characters as Word allows.
fn new_property_value(instr: &str) -> Option<String> {
    FieldCode::parse(instr).args.first().map(|a| a.chars().take(255).collect())
}

/// A count (NUMWORDS, NUMCHARS, REVNUM) in the field's `\*` number format, digits without one or
/// when the format does not cover the count (see [`super::table_formula::number_format_covers`]),
/// with the field's capitalization switches.
fn count_text(instr: &str, n: usize) -> String {
    let fmt = wordcraft_layout::fields::format_switch(instr)
        .filter(|f| super::table_formula::number_format_covers(*f, n as f64))
        .unwrap_or(NumFormat::Decimal);
    wordcraft_layout::fields::field_number(instr, fmt, u32::try_from(n).unwrap_or(u32::MAX))
}

/// A field's computed text with its capitalization switches (`\* Upper`, `\* Lower`, `\* Caps`,
/// `\* FirstCap`) applied in order.
fn text_result(instr: &str, text: String) -> String {
    wordcraft_layout::fields::apply_case_switch(instr, text)
}

/// The property name of a `DOCPROPERTY` field: the word (or quoted text) after the field name,
/// before any switch (`DOCPROPERTY "Project Name" \* MERGEFORMAT`).
fn doc_property_arg(instr: &str) -> Option<String> {
    let rest = instr.trim().split_once(char::is_whitespace)?.1.trim_start();
    let name = match rest.strip_prefix('"') {
        Some(q) => q.split('"').next().unwrap_or(""),
        None => rest.split([' ', '\\']).next().unwrap_or(""),
    };
    (!name.is_empty()).then(|| name.to_string())
}

/// The value of a document property under the name a `DOCPROPERTY` field uses: a custom property
/// when one has this name, else the built-in property (English names, case-insensitively, as
/// Word's field accepts) from `core`. `None` for a name this application doesn't know.
fn doc_property(doc: &wordcraft_doc::Document, core: &wordcraft_doc::CoreProps, name: &str) -> Option<String> {
    if let Some(v) = doc.custom_prop(name) {
        return Some(v.to_string());
    }
    let c = core;
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "title" => c.title.clone(),
        "subject" => c.subject.clone(),
        "author" | "creator" => c.creator.clone(),
        "keywords" => c.keywords.clone(),
        "comments" | "description" => c.description.clone(),
        "category" => c.category.clone(),
        "lastmodifiedby" | "last modified by" | "lastsavedby" | "last saved by" => c.last_modified_by.clone(),
        "revisionnumber" | "revision number" | "revision" => c.revision.to_string(),
        _ => return None,
    })
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

    /// #775: body STYLEREF fields show the nearest paragraph of the style before them.
    #[test]
    fn body_styleref_finds_the_heading_before_it() {
        let mut s = Session::new(wordcraft_doc::Document::from_text("Intro\nChapter One\ntext here\nChapter Two\nend"));
        for t in ["Chapter One", "Chapter Two"] {
            s.run("select.text", &json!({"text": t})).unwrap();
            s.run("styles.apply", &json!({"style": "Heading 1"})).unwrap();
        }
        s.run("select.text", &json!({"text": "text here"})).unwrap();
        s.run("caret.end", &json!({})).unwrap();
        s.run("insert.field", &json!({"instr": r#"STYLEREF "Heading 1" \* MERGEFORMAT"#})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("insert.field", &json!({"instr": "STYLEREF 1 \\p"})).unwrap();
        s.run("caret.docStart", &json!({})).unwrap();
        s.run("insert.field", &json!({"instr": "STYLEREF \"Heading 1\" \\p"})).unwrap();
        s.run("insert.field", &json!({"instr": "STYLEREF \"No Such Style\""})).unwrap();
        let results: Vec<String> = fields(&s).into_iter().map(|f| f.1).collect();
        assert_eq!(results, ["below", wordcraft_layout::styleref::NO_TEXT, "Chapter One", "above"]);
    }

    /// #775: a template's running header `STYLEREF "heading 1"` (a complex field, opened from
    /// DOCX) shows each page's chapter, and keeps its code through a save.
    #[test]
    fn docx_styleref_header_shows_each_page_s_chapter() {
        let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
        let heading = |t: &str, brk: bool| {
            let b = if brk { "<w:pageBreakBefore/>" } else { "" };
            format!(
                r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/>{b}</w:pPr><w:r><w:t>{t}</w:t></w:r></w:p><w:p><w:r><w:t>Body of {t}.</w:t></w:r></w:p>"#
            )
        };
        let body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:document {ns}><w:body>{}{}{}<w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr></w:body></w:document>"#,
            heading("Rivers", false),
            heading("Mountains", true),
            heading("Deserts", true)
        );
        let header = format!(
            r#"<w:hdr {ns}><w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> STYLEREF  "heading 1" \* MERGEFORMAT </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Cached</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p></w:hdr>"#
        );
        let styles = format!(
            r#"<w:styles {ns}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/></w:style></w:styles>"#
        );
        let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/><Relationship Id="rIdS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, text) in [
            ("_rels/.rels", root),
            ("word/document.xml", body.as_str()),
            ("word/_rels/document.xml.rels", rels),
            ("word/header1.xml", header.as_str()),
            ("word/styles.xml", styles.as_str()),
        ] {
            zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut zw, text.as_bytes()).unwrap();
        }
        let bytes = zw.finish().unwrap().into_inner();
        let headers = |s: &mut Session| -> Vec<String> {
            s.layout()
                .pages
                .iter()
                .map(|p| {
                    p.header
                        .iter()
                        .filter_map(|it| {
                            if let wordcraft_layout::Placed::Lines { para, .. } = it {
                                Some(para.shown.iter().map(|x| x.1.clone()).collect::<String>())
                            } else {
                                None
                            }
                        })
                        .collect::<String>()
                })
                .collect()
        };
        let mut s = Session::new(crate::io::open_bytes("chapters.docx", &bytes).unwrap());
        assert_eq!(headers(&mut s), ["Rivers", "Mountains", "Deserts"]);
        let saved = crate::io::save_bytes("chapters.docx", &s.doc).unwrap();
        let mut again = Session::new(crate::io::open_bytes("chapters.docx", &saved).unwrap());
        assert_eq!(headers(&mut again), ["Rivers", "Mountains", "Deserts"]);
        let hid = again.doc.last_section.headers.default.unwrap();
        let instr = again.doc.para_paths(StoryRef::Part(hid)).iter().find_map(|p| {
            again.doc.para(StoryRef::Part(hid), p).unwrap().objects.iter().find_map(|o| match o {
                InlineObject::Field { instr, .. } => Some(instr.clone()),
                _ => None,
            })
        });
        assert!(instr.is_some_and(|i| i.contains(r#"STYLEREF  "heading 1""#)));
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

    #[test]
    fn update_fields_ref_and_pageref() {
        let mut s = Session::new(wordcraft_doc::Document::from_text("Bookmark target text here.\nSecond paragraph."));
        s.run("select.text", &json!({"text": "Bookmark target text"})).unwrap();
        s.run("insert.bookmark", &json!({"name": "TargetMark"})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("insert.field", &json!({"instr": "REF \"TargetMark\" \\h", "result": "old"})).unwrap();
        s.run("insert.field", &json!({"instr": "REF TargetMark\\h", "result": "old"})).unwrap();
        s.run("insert.field", &json!({"instr": "PAGEREF \"TargetMark\" \\* Roman", "result": "0"})).unwrap();
        s.run("insert.field", &json!({"instr": "PAGEREF TargetMark \\p", "result": "above"})).unwrap();
        s.run("insert.field", &json!({"instr": "REF TargetMark \\p", "result": "above"})).unwrap();

        update_fields(&mut s).unwrap();

        let res: Vec<(String, String)> = fields(&s);
        assert_eq!(res[0], ("REF \"TargetMark\" \\h".into(), "Bookmark target text".into()));
        assert_eq!(res[1], ("REF TargetMark\\h".into(), "Bookmark target text".into()));
        assert_eq!(res[2], ("PAGEREF \"TargetMark\" \\* Roman".into(), "I".into()));
        assert_eq!(res[3], ("PAGEREF TargetMark \\p".into(), "above".into()));
        assert_eq!(res[4], ("REF TargetMark \\p".into(), "above".into()));
    }

    /// A document with a body AUTHOR field and a header holding AUTHOR, NUMWORDS and a locked TITLE,
    /// all stale; the author is "Ada".
    fn doc_with_header_fields() -> (wordcraft_doc::Document, u32) {
        use wordcraft_doc::{CharProps, Paragraph, PartKind};
        let field = |instr: &str, locked: bool| InlineObject::Field {
            instr: instr.into(),
            result: "stale".into(),
            locked,
            dirty: false,
            code: false,
            form: String::new(),
        };
        let mut doc = wordcraft_doc::Document::from_text("one two three");
        doc.core.creator = "Ada".into();
        let mut p = Paragraph::new();
        for (instr, locked) in [("AUTHOR", false), ("NUMWORDS", false), ("TITLE", true)] {
            let off = p.len();
            p.insert_object(off, field(instr, locked), &CharProps::default()).unwrap();
        }
        let body_path = doc.para_paths(StoryRef::Body).remove(0);
        doc.para_mut(StoryRef::Body, &body_path).unwrap().insert_object(0, field("AUTHOR", false), &CharProps::default()).unwrap();
        let hdr = doc.add_part(PartKind::Header, vec![std::sync::Arc::new(wordcraft_doc::Block::Para(p))]);
        (doc, hdr)
    }

    fn story_fields(s: &Session, story: StoryRef) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for path in s.doc.para_paths(story) {
            for o in &s.doc.para(story, &path).unwrap().objects {
                if let InlineObject::Field { instr, result, .. } = o {
                    out.push((instr.clone(), result.clone()));
                }
            }
        }
        out
    }

    #[test]
    fn update_fields_filename() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.path = Some(std::path::PathBuf::from("/docs/my_thesis.docx"));
        s.run("insert.field", &serde_json::json!({"instr": "FILENAME"})).unwrap();
        let fields = story_fields(&s, StoryRef::Body);
        assert_eq!(fields[0], ("FILENAME".into(), "my_thesis.docx".into()));

        s.run("insert.field", &serde_json::json!({"instr": r"FILENAME \p"})).unwrap();
        let fields = story_fields(&s, StoryRef::Body);
        assert_eq!(fields[1], (r"FILENAME \p".into(), "/docs/my_thesis.docx".into()));
    }

    #[test]
    fn update_fields_filename_titled_unsaved() {
        let mut doc = wordcraft_doc::Document::new();
        doc.core.title = "Important Document".into();
        let mut s = Session::new(doc);
        s.run("insert.field", &serde_json::json!({"instr": "FILENAME"})).unwrap();
        let fields = story_fields(&s, StoryRef::Body);
        assert_eq!(fields[0], ("FILENAME".into(), "Document1".into()));

        let l = s.layout();
        let text: String = l.pages[0]
            .items
            .iter()
            .filter_map(|it| {
                if let wordcraft_layout::Placed::Lines { para, .. } = it {
                    Some(para.shown.iter().map(|x| x.1.clone()).collect::<String>())
                } else {
                    None
                }
            })
            .collect();
        assert!(text.contains("Document1"), "live layout renders Document1: {text}");
        assert!(!text.contains("Important Document"), "live layout must not render title: {text}");

        s.run("references.updateFields", &serde_json::json!({})).unwrap();
        let fields_after = story_fields(&s, StoryRef::Body);
        assert_eq!(fields_after[0], ("FILENAME".into(), "Document1".into()));
    }

    /// #1706 (Word check): Update Field with the caret in the body leaves a header's fields alone.
    #[test]
    fn update_fields_in_the_body_leaves_headers_alone() {
        let (doc, hdr) = doc_with_header_fields();
        let mut s = Session::new(doc);
        s.run("references.updateFields", &json!({})).unwrap();
        assert_eq!(story_fields(&s, StoryRef::Body)[0], ("AUTHOR".to_string(), "Ada".to_string()));
        assert!(story_fields(&s, StoryRef::Part(hdr)).iter().all(|(_, r)| r == "stale"), "{:?}", story_fields(&s, StoryRef::Part(hdr)));
    }

    /// #1706: with the caret in a header (or other part), Update Field refreshes that part's
    /// fields, but not locked ones and not the body's.
    #[test]
    fn update_fields_in_a_header_updates_that_header() {
        let (doc, hdr) = doc_with_header_fields();
        let mut s = Session::new(doc);
        let path = s.doc.para_paths(StoryRef::Part(hdr)).remove(0);
        s.sel = crate::session::Selection::caret(Pos::new(StoryRef::Part(hdr), path, 0));
        s.run("references.updateFields", &json!({})).unwrap();
        let got = story_fields(&s, StoryRef::Part(hdr));
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[0].1, "Ada");
        assert_eq!(got[1].1, s.doc.word_count().to_string());
        assert_eq!(got[2].1, "stale", "a locked field stays as it was");
        assert!(story_fields(&s, StoryRef::Body).iter().all(|(_, r)| r == "stale"), "the body is not touched");
    }

    /// A session whose body reads "one two three" and whose header holds stale fields with
    /// these codes, the caret in the header.
    fn header_fields_session(doc: wordcraft_doc::Document, instrs: &[&str]) -> (Session, u32) {
        use wordcraft_doc::{CharProps, Paragraph, PartKind};
        let mut p = Paragraph::new();
        for instr in instrs {
            let off = p.len();
            let field =
                InlineObject::Field { instr: (*instr).into(), result: "stale".into(), locked: false, dirty: false, code: false, form: String::new() };
            p.insert_object(off, field, &CharProps::default()).unwrap();
        }
        let mut doc = doc;
        let hdr = doc.add_part(PartKind::Header, vec![std::sync::Arc::new(wordcraft_doc::Block::Para(p))]);
        let mut s = Session::new(doc);
        let path = s.doc.para_paths(StoryRef::Part(hdr)).remove(0);
        s.sel = crate::session::Selection::caret(Pos::new(StoryRef::Part(hdr), path, 0));
        (s, hdr)
    }

    /// SUBJECT, KEYWORDS, COMMENTS, LASTSAVEDBY and REVNUM show the document properties, and
    /// NUMCHARS counts the body's characters without spaces; the counts take a `\*` format.
    #[test]
    fn document_information_fields_show_the_properties() {
        let mut doc = wordcraft_doc::Document::from_text("one two three");
        doc.core.subject = "Budget".into();
        doc.core.keywords = "plan; costs".into();
        doc.core.description = "Draft for review".into();
        doc.core.last_modified_by = "Grace".into();
        doc.core.revision = 7;
        let codes = ["SUBJECT", "KEYWORDS", "COMMENTS", "LASTSAVEDBY", "REVNUM", r"REVNUM \* ROMAN", "NUMCHARS", r"NUMWORDS \* roman"];
        let (mut s, hdr) = header_fields_session(doc, &codes);
        s.run("references.updateFields", &json!({})).unwrap();
        let got: Vec<String> = story_fields(&s, StoryRef::Part(hdr)).into_iter().map(|(_, r)| r).collect();
        assert_eq!(got, ["Budget", "plan; costs", "Draft for review", "Grace", "7", "VII", "11", "iii"]);
    }

    /// A quoted text after TITLE, SUBJECT, AUTHOR, KEYWORDS or COMMENTS replaces the property
    /// (at most 255 characters); the field and later ones, DOCPROPERTY too, show the new value.
    #[test]
    fn summary_fields_with_a_new_value_replace_the_property() {
        let mut doc = wordcraft_doc::Document::from_text("one two three");
        doc.core.subject = "Old".into();
        let long = "x".repeat(300);
        let long_code = format!("COMMENTS \"{long}\"");
        let codes = [
            r#"SUBJECT "Quarterly report""#,
            "SUBJECT",
            r#"TITLE "Plan" \* MERGEFORMAT"#,
            r#"AUTHOR "Ada""#,
            r#"KEYWORDS "a, b""#,
            &long_code,
            "DOCPROPERTY Subject",
        ];
        let (mut s, hdr) = header_fields_session(doc, &codes);
        s.run("references.updateFields", &json!({})).unwrap();
        let got: Vec<String> = story_fields(&s, StoryRef::Part(hdr)).into_iter().map(|(_, r)| r).collect();
        assert_eq!(got[..5], ["Quarterly report", "Quarterly report", "Plan", "Ada", "a, b"]);
        assert_eq!(got[5].chars().count(), 255);
        assert_eq!(got[6], "Quarterly report", "DOCPROPERTY reads the subject the paragraph set");
        let c = &s.doc.core;
        assert_eq!((c.subject.as_str(), c.title.as_str(), c.creator.as_str(), c.keywords.as_str()), ("Quarterly report", "Plan", "Ada", "a, b"));
        assert_eq!(c.description, got[5]);
    }

    /// One word after the name sets the property without quotes; a switch alone sets nothing.
    #[test]
    fn summary_fields_take_one_word_without_quotes() {
        let mut doc = wordcraft_doc::Document::from_text("one two three");
        doc.core.subject = "Old".into();
        doc.core.creator = "Grace".into();
        doc.core.title = "Plan".into();
        let (mut s, hdr) = header_fields_session(doc, &["SUBJECT Draft", "AUTHOR Ada", r"TITLE \* Upper"]);
        s.run("references.updateFields", &json!({})).unwrap();
        let got: Vec<String> = story_fields(&s, StoryRef::Part(hdr)).into_iter().map(|(_, r)| r).collect();
        assert_eq!(got, ["Draft", "Ada", "PLAN"]);
        let c = &s.doc.core;
        assert_eq!((c.subject.as_str(), c.creator.as_str(), c.title.as_str()), ("Draft", "Ada", "Plan"));
    }

    /// `\* Upper`, `\* Lower`, `\* Caps` and `\* FirstCap` apply to the text Update Field computes,
    /// and `\* CardText` and `\* OrdText` are lowercase unless one of them follows.
    #[test]
    fn update_fields_apply_the_capitalization_switches() {
        let mut doc = wordcraft_doc::Document::from_text("one two three");
        doc.core.subject = "budget".into();
        doc.core.creator = "Ada LOVELACE".into();
        doc.core.title = "weekly report on sales".into();
        doc.core.description = "weekly report on sales".into();
        doc.core.last_modified_by = "grace".into();
        doc.core.revision = 7;
        let codes = [
            r"SUBJECT \* Upper",
            r"AUTHOR \* Lower",
            r"TITLE \* Caps \* MERGEFORMAT",
            r"COMMENTS \*FirstCap",
            r"LASTSAVEDBY \* upper",
            r"NUMWORDS \* CardText",
            r"NUMWORDS \* CardText \* Caps",
            r"REVNUM \* OrdText",
            r"REVNUM \* OrdText \* FirstCap",
            r"DOCPROPERTY Subject \* Upper",
            r#"SUBJECT "new \* Upper""#,
        ];
        let (mut s, hdr) = header_fields_session(doc, &codes);
        s.run("references.updateFields", &json!({})).unwrap();
        let got: Vec<String> = story_fields(&s, StoryRef::Part(hdr)).into_iter().map(|(_, r)| r).collect();
        assert_eq!(
            got,
            [
                "BUDGET",
                "ada lovelace",
                "Weekly Report On Sales",
                "Weekly report on sales",
                "GRACE",
                "three",
                "Three",
                "seventh",
                "Seventh",
                "BUDGET",
                "new \\* Upper"
            ]
        );
    }

    /// The capitalization switches also apply to REF, PAGEREF, SEQ and FILENAME results in the body.
    #[test]
    fn body_fields_apply_the_capitalization_switches() {
        let mut s = Session::new(wordcraft_doc::Document::from_text("Bookmark target text here.\nSecond paragraph."));
        s.run("select.text", &json!({"text": "Bookmark target text"})).unwrap();
        s.run("insert.bookmark", &json!({"name": "TargetMark"})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        for instr in [r"REF TargetMark \* Upper", r"PAGEREF TargetMark \* CardText", r"SEQ Figure \* OrdText \* Caps", r"FILENAME \* Lower"] {
            s.run("insert.field", &json!({"instr": instr, "result": "old"})).unwrap();
        }
        update_fields(&mut s).unwrap();
        let got: Vec<String> = fields(&s).into_iter().map(|(_, r)| r).collect();
        let name = s.resolved_filenames().0.to_lowercase();
        assert_eq!(got, ["BOOKMARK TARGET TEXT".to_string(), "one".into(), "First".into(), name]);
    }

    /// A count the field's number format does not cover keeps its digits.
    #[test]
    fn counts_past_the_number_format_keep_their_digits() {
        let mut doc = wordcraft_doc::Document::from_text("one two three");
        doc.core.revision = 1_000_000;
        let codes = [r"REVNUM \* ROMAN", r"REVNUM \* CardText", r"REVNUM \* alphabetic", r"NUMWORDS \* ROMAN"];
        let (mut s, hdr) = header_fields_session(doc, &codes);
        s.run("references.updateFields", &json!({})).unwrap();
        let got: Vec<String> = story_fields(&s, StoryRef::Part(hdr)).into_iter().map(|(_, r)| r).collect();
        assert_eq!(got, ["1000000", "1000000", "1000000", "III"]);
    }

    /// #1706: `all: true` (printing) updates the whole document even with the caret in a header.
    #[test]
    fn update_fields_all_covers_every_story() {
        let (doc, hdr) = doc_with_header_fields();
        let mut s = Session::new(doc);
        let path = s.doc.para_paths(StoryRef::Part(hdr)).remove(0);
        s.sel = crate::session::Selection::caret(Pos::new(StoryRef::Part(hdr), path, 0));
        s.run("references.updateFields", &json!({"all": true})).unwrap();
        let got = story_fields(&s, StoryRef::Part(hdr));
        assert_eq!((got[0].1.as_str(), got[2].1.as_str()), ("Ada", "stale"), "{got:?}");
        assert_eq!(story_fields(&s, StoryRef::Body)[0].1, "Ada");
        // Junk values fall back to the caret's story.
        s.run("references.updateFields", &json!({"all": "yes"})).unwrap();
    }

    #[test]
    fn toc_tabs_ignore_nonentries() {
        let mut s = Session::new(wordcraft_doc::Document::from_text("Before\nContents\nOld\t9\nAfter"));
        let stop = |pos| TabStop { pos, align: TabAlign::Left, leader: TabLeader::None };
        for (i, pos) in [(0, 96.0), (1, 84.0), (2, 24.0)] {
            let p = s.doc.para_mut(StoryRef::Body, &Path::top(i)).unwrap();
            p.props.style = Some("TOC2".into());
            p.props.tabs = Some(vec![stop(pos)]);
        }
        s.doc
            .para_mut(StoryRef::Body, &Path::top(1))
            .unwrap()
            .insert_object(
                0,
                InlineObject::Field { instr: "TOC".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
                &CharProps::default(),
            )
            .unwrap();
        assert_eq!(cached_toc_tabs(&s), vec![("TOC2".into(), vec![stop(24.0)])]);
    }

    /// #1230: a selected picture or shape in a table cell gets its caption in the cell, under
    /// (or over) the paragraph that holds it, not after the whole table; the number counts on.
    #[test]
    fn caption_for_an_object_in_a_cell_stays_in_the_cell() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 1, "cols": 2})).unwrap();
        s.run("insert.shape", &json!({"kind": "ellipse", "width": 60, "height": 40})).unwrap();
        let (at, _) = crate::cmd::objects::selected(&s).expect("the shape is selected");
        let (table, r, c) = at.path.cell().expect("the shape is in a cell");
        s.run("references.caption", &json!({"label": "Figure", "text": "Kiln"})).unwrap();
        let cell = |s: &Session| s.doc.table(StoryRef::Body, &table).unwrap().rows[r].cells[c].blocks.clone();
        let texts: Vec<String> = cell(&s).iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect();
        assert_eq!(texts.len(), 2, "{texts:?}");
        assert_eq!(texts[1], "Figure 1: Kiln", "the caption is under the shape, in the cell");
        assert_eq!(s.doc.body.iter().filter(|b| b.as_para().is_some_and(|p| p.props.style.as_deref() == Some("Caption"))).count(), 0);
        // Above: over the shape's paragraph, and it is Figure 1 (the one below becomes 2).
        let end = Pos { off: at.off + wordcraft_doc::para::OBJ.len_utf8(), ..at.clone() };
        s.sel = Selection { anchor: at.clone(), focus: end };
        assert!(crate::cmd::objects::object_selection(&s).is_some());
        s.run("references.caption", &json!({"label": "Figure", "text": "Lid", "position": "above"})).unwrap();
        let texts: Vec<String> = cell(&s).iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect();
        assert_eq!(texts.len(), 3, "{texts:?}");
        assert_eq!((texts[0].as_str(), texts[2].as_str()), ("Figure 1: Lid", "Figure 2: Kiln"));
        // A picture outside a table: the caption goes right after its paragraph.
        let mut s = Session::new(wordcraft_doc::Document::from_text("Before\nAfter"));
        s.run("select.text", &json!({"text": "Before"})).unwrap();
        s.run("caret.end", &json!({})).unwrap();
        s.run("insert.shape", &json!({"kind": "ellipse", "width": 60, "height": 40})).unwrap();
        s.run("references.caption", &json!({})).unwrap();
        let texts: Vec<String> = s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect();
        assert_eq!(texts.get(1).map(String::as_str), Some("Figure 1"), "{texts:?}");
    }
}
