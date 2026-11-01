use super::properties::{
    bookmark_text, count_text, date_picture, doc_property, doc_property_arg, new_property_value, set_summary_property, summary_property, text_result,
};
use super::toc::{heading_style_level, update_toc};
use super::{
    Block, CharProps, CmdError, CmdResult, FieldCode, InlineObject, NumFormat, Paragraph, Path, Pos, Selection, Session, StoryRef, Value, p,
    sel_result,
};

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
pub(super) fn caption(s: &mut Session, v: &Value) -> CmdResult {
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
    let at = match crate::cmd::objects::object_selection(s) {
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
                    && let Some(t) = s.doc.table(StoryRef::Body, &tp)
                {
                    let default_lang = crate::cmd::default_lang(s);
                    let text = match crate::cmd::table_formula::evaluate_field_with(t, Some(&s.doc), default_lang.as_deref(), r, c, instr) {
                        Ok(text) => text,
                        Err(e) => format!("!{e}"),
                    };
                    updates.push((k, text));
                }
                continue;
            }
            let name = wordcraft_layout::fields::field_name(instr);
            match name.as_str() {
                "DATE" | "TIME" | "CREATEDATE" | "SAVEDATE" | "PRINTDATE" => {
                    let pic = date_picture(instr).unwrap_or_else(|| if name == "TIME" { "h:mm am/pm".into() } else { "M/d/yyyy".into() });
                    updates.push((k, text_result(instr, crate::cmd::insert::format_date(&pic))));
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
                _ => {}
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
    crate::cmd::citations::update_citations(s)?;
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
                updates.push((k, text_result(instr, crate::cmd::insert::format_date(&pic))));
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
