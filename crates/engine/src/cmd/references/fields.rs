use super::properties::{
    bookmark_ends_in_its_paragraph, bookmark_text, bookmark_text_at, count_text, date_picture, doc_property, doc_property_arg, new_property_value,
    set_summary_property, summary_property, text_result,
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
    // Numbers first: a STYLEREF to a heading that holds a SEQ field shows its new number.
    let numbers = seq_results(&s.doc);
    apply_results(&mut s.doc, numbers)?;
    let refs = styleref_results(&s.doc);
    apply_results(&mut s.doc, refs)
}

/// A field's fresh result: its story and paragraph, its object index there and the text.
type FieldResult = (StoryRef, Path, usize, String);

/// The SEQ numbering so far while the document is walked in order.
#[derive(Default)]
struct SeqCount {
    /// How many headings of each level there have been (for `\s`).
    headings: [u32; 9],
    /// Per sequence: its number, and the heading count it was last restarted at.
    seq: std::collections::HashMap<String, (u32, u32)>,
    /// Text boxes already counted: a story shown twice, or inside itself, counts once.
    boxes: std::collections::HashSet<u32>,
    out: Vec<FieldResult>,
}

/// The fresh results of the unlocked SEQ fields in the body and in the text boxes anchored there
/// ([`update_seq`]). A text box's fields count where the box is anchored, before the fields after
/// the anchor in its paragraph and after those before it (measured in Word for Windows).
fn seq_results(doc: &wordcraft_doc::Document) -> Vec<FieldResult> {
    let mut count = SeqCount::default();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        if let Some(l) = heading_style_level(p)
            && let Some(k) = (l as usize).checked_sub(1)
            && let Some(n) = count.headings.get_mut(k)
        {
            *n = n.saturating_add(1);
        }
        count_seq(doc, StoryRef::Body, &path, p, &mut count);
    }
    count.out
}

/// Number the SEQ fields of paragraph `p` (at `path` in `story`) and of the text boxes anchored in it.
fn count_seq(doc: &wordcraft_doc::Document, story: StoryRef, path: &Path, p: &Paragraph, count: &mut SeqCount) {
    for (k, o) in p.objects.iter().enumerate() {
        for id in o.text_boxes() {
            if count.boxes.insert(id) {
                let part = StoryRef::Part(id);
                for bp in doc.para_paths(part) {
                    if let Some(boxed) = doc.para(part, &bp) {
                        count_seq(doc, part, &bp, boxed, count);
                    }
                }
            }
        }
        let InlineObject::Field { instr, locked: false, .. } = o else { continue };
        let f = FieldCode::parse(instr);
        if f.name != "SEQ" {
            continue;
        }
        let Some(id) = f.args.first() else { continue };
        let restart = f.arg('s').and_then(|l| l.parse::<usize>().ok()).and_then(|l| l.checked_sub(1)).and_then(|l| count.headings.get(l)).copied();
        let n = count.seq.entry(id.clone()).or_insert((0, 0));
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
        let r = if f.has('h') { String::new() } else { wordcraft_layout::fields::field_number(instr, fmt, n.0) };
        count.out.push((story, path.clone(), k, r));
    }
}

/// The fresh results of the body's unlocked STYLEREF fields ([`update_seq`]), from the text
/// `doc` holds: the SEQ fields in the headings they find should already show their numbers.
fn styleref_results(doc: &wordcraft_doc::Document) -> Vec<FieldResult> {
    let mut out = Vec::new();
    // Built on the first STYLEREF.
    let mut styles: Option<wordcraft_layout::styleref::StyleIndex> = None;
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        for (k, o) in p.objects.iter().enumerate() {
            let InlineObject::Field { instr, locked: false, .. } = o else { continue };
            if FieldCode::parse(instr).name != "STYLEREF" {
                continue;
            }
            let ix = styles.get_or_insert_with(|| wordcraft_layout::styleref::StyleIndex::build(doc));
            if let Some(i) = ix.index_of(&path) {
                out.push((StoryRef::Body, path.clone(), k, text_result(instr, ix.eval(instr, wordcraft_layout::styleref::At::Body(i)))));
            }
        }
    }
    out
}

/// What Word prints for a `REF` to a bookmark the document doesn't have.
pub const REFERENCE_NOT_FOUND: &str = "Error! Reference source not found.";

/// The fresh results of the body's unlocked REF fields as output shows them: the bookmark's
/// text, or Word's error when the document lacks the bookmark (measured in Word for Windows);
/// `\p` ("above" or "below") and the paragraph number switches `\r`, `\n` and `\w` keep the
/// stored result, and so does a REF to a bookmark that ends in a later paragraph: Word shows such
/// a bookmark's paragraphs as paragraphs of the result, which a field's one-paragraph result
/// can't hold (measured in Word for Windows). A bookmark in a text box is found too, and a REF to
/// it shows the box's text as `stored` holds it, its own fields not yet refreshed, as Word's
/// output does. The document's bookmarks are looked up once, and each bookmark's text read once,
/// however many fields refer to it.
fn ref_results(doc: &wordcraft_doc::Document, stored: &wordcraft_doc::Document) -> Vec<FieldResult> {
    let mut out = Vec::new();
    let mut starts: std::collections::HashMap<String, Pos> = std::collections::HashMap::new();
    for (name, start) in doc.bookmarks().into_iter().chain(text_box_bookmarks(doc)) {
        // The first of two bookmarks with one name, as `bookmark_text` takes.
        starts.entry(name).or_insert(start);
    }
    // A bookmark's text, `None` when it ends in a later paragraph.
    let mut texts: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        for (k, o) in p.objects.iter().enumerate() {
            let InlineObject::Field { instr, locked: false, .. } = o else { continue };
            // `\r`, `\n` and `\w` show the bookmarked paragraph's number, which isn't computed
            // here: those keep their stored result rather than print the paragraph's text.
            if wordcraft_layout::fields::field_name(instr) != "REF" || ['p', 'r', 'n', 'w'].iter().any(|&sw| has_switch(instr, sw)) {
                continue;
            }
            let f = FieldCode::parse(instr);
            let target = f.args.first().map(|s| s.split('\\').next().unwrap_or("").trim_matches('"')).unwrap_or("");
            let Some(start) = starts.get(target) else {
                out.push((StoryRef::Body, path.clone(), k, REFERENCE_NOT_FOUND.to_string()));
                continue;
            };
            let text = texts.entry(target.to_string()).or_insert_with(|| {
                let source = if start.story == StoryRef::Body { doc } else { stored };
                bookmark_ends_in_its_paragraph(source, target, start).then(|| bookmark_text_at(source, target, start)).flatten()
            });
            if let Some(r) = text.clone() {
                out.push((StoryRef::Body, path.clone(), k, text_result(instr, r)));
            }
        }
    }
    out
}

/// The bookmarks in the document's text boxes, in story order.
fn text_box_bookmarks(doc: &wordcraft_doc::Document) -> Vec<(String, Pos)> {
    let mut out = Vec::new();
    for (id, part) in &doc.parts {
        if part.kind != wordcraft_doc::PartKind::TextBox {
            continue;
        }
        let story = StoryRef::Part(*id);
        for path in doc.para_paths(story) {
            let Some(p) = doc.para(story, &path) else { continue };
            for (off, o) in p.objects_with_offsets() {
                if let InlineObject::BookmarkStart { name } = o {
                    out.push((name.clone(), Pos { story, path: path.clone(), off }));
                }
            }
        }
    }
    out
}

/// Put `results` into their fields, clearing `w:dirty` (a computed result is no longer stale:
/// Word's F9 clears it too).
fn apply_results(doc: &mut wordcraft_doc::Document, results: Vec<FieldResult>) -> Result<(), CmdError> {
    for (story, path, k, r) in results {
        let para = doc.para_mut(story, &path)?;
        if let Some(InlineObject::Field { result, dirty, .. }) = para.objects.get_mut(k) {
            *result = r;
            *dirty = false;
        }
        para.touch();
    }
    Ok(())
}

/// `results` less those its fields already show.
fn changed(doc: &wordcraft_doc::Document, mut results: Vec<FieldResult>) -> Vec<FieldResult> {
    results.retain(|(story, path, k, r)| {
        !matches!(doc.para(*story, path).and_then(|p| p.objects.get(*k)), Some(InlineObject::Field { result, .. }) if result == r)
    });
    results
}

/// The document as Word prints and exports it (PDF, print jobs, page images). Laying a document
/// out for output, Word fills in the body's unlocked SEQ, STYLEREF and REF fields, even with
/// "Update fields before printing" off, while NUMWORDS, FILENAME, the summary properties and
/// formulas keep their stored results (measured with Word for Windows, printing and exporting
/// fields whose stored results were wrong). The document itself keeps its stored results.
/// `None` when every such field already shows its fresh result, so nothing is copied then.
pub fn output_fields(doc: &wordcraft_doc::Document) -> Option<wordcraft_doc::Document> {
    // Numbers first, then the fields that read them: a STYLEREF to a heading holding a SEQ field,
    // and a REF to a bookmarked caption, show the new number.
    let seq = changed(doc, seq_results(doc));
    let mut out = (!seq.is_empty()).then(|| doc.clone());
    if let Some(d) = out.as_mut() {
        apply_results(d, seq).ok()?;
    }
    let styles = changed(out.as_ref().unwrap_or(doc), styleref_results(out.as_ref().unwrap_or(doc)));
    if !styles.is_empty() {
        apply_results(out.get_or_insert_with(|| doc.clone()), styles).ok()?;
    }
    let refs = changed(out.as_ref().unwrap_or(doc), ref_results(out.as_ref().unwrap_or(doc), doc));
    if !refs.is_empty() {
        apply_results(out.get_or_insert_with(|| doc.clone()), refs).ok()?;
    }
    out
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
