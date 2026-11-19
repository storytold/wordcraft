use super::PartKind;
use super::header_footer::hf_part;
use super::{
    Align, Block, CharProps, CmdError, CmdResult, InlineObject, Paragraph, Rgb, Selection, Session, StoryRef, TabAlign, TabStop, TextColor, Value,
    delete_selection, json, p, para_block, sel_result, type_text, type_text_with_lang,
};
use std::collections::BTreeSet;

fn story_has_page_field(doc: &wordcraft_doc::Document, story: StoryRef, except: Option<usize>) -> bool {
    let Some(blocks) = doc.story(story) else { return false };
    let mut seen = BTreeSet::from([story]);
    let mut pending: Vec<&Block> = blocks.iter().enumerate().filter(|(i, _)| Some(*i) != except).map(|(_, b)| &**b).collect();
    while let Some(block) = pending.pop() {
        let para = match block {
            Block::Para(para) => para,
            Block::Table(table) => {
                pending.extend(table.rows.iter().flat_map(|r| &r.cells).flat_map(|c| &c.blocks).map(|b| &**b));
                continue;
            }
        };
        for object in &para.objects {
            if matches!(object, InlineObject::Field { instr, .. } | InlineObject::FieldStart { instr, .. } if instr.split_whitespace().next().is_some_and(|name| name.eq_ignore_ascii_case("PAGE")))
            {
                return true;
            }
            for id in object.text_boxes() {
                if let Some(part) = doc.parts.get(&id).filter(|part| part.kind == PartKind::TextBox)
                    && seen.insert(StoryRef::Part(id))
                {
                    pending.extend(part.blocks.iter().map(|b| &**b));
                }
            }
        }
    }
    false
}

fn page_number_paragraph(style: &str, align: Align, x_of_y: bool) -> Result<Paragraph, CmdError> {
    let field = InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() };
    let mut para = Paragraph::new().styled(style);
    para.props.align = Some(align);
    para.props.tabs = Some(vec![TabStop { pos: 0.0, align: TabAlign::Clear, leader: Default::default() }]);
    if x_of_y {
        para.insert_text(0, "Page  of ", &CharProps::default())?;
        para.insert_object(5, field, &CharProps { bold: Some(true), ..Default::default() })?;
        let end = para.len();
        para.insert_object(
            end,
            InlineObject::Field { instr: "NUMPAGES".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() },
            &CharProps { bold: Some(true), ..Default::default() },
        )?;
    } else {
        para.insert_object(0, field, &CharProps::default())?;
    }
    Ok(para)
}

// Only cached display text is transferable; instructions, flags and formatting remain part of ownership.
fn copy_page_number_results(to: &mut Paragraph, from: &Paragraph) {
    for object in &mut to.objects {
        let InlineObject::Field { instr, result, .. } = object else { continue };
        if let Some(cached) = from.objects.iter().find_map(|object| match object {
            InlineObject::Field { instr: source, result, .. } if source.as_str() == instr.as_str() => Some(result),
            _ => None,
        }) {
            result.clone_from(cached);
        }
    }
}

pub(super) fn page_number(s: &mut Session, v: &Value) -> CmdResult {
    let position = p::str(v, "position").unwrap_or("bottom");
    let align = match p::str(v, "align").unwrap_or("center") {
        "left" => Align::Left,
        "right" => Align::Right,
        _ => Align::Center,
    };
    let x_of_y = p::str(v, "format").is_some_and(|f| f.contains("of"));
    let field = InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() };
    if position == "current" {
        let props = s.typing_props();
        let at = delete_selection(s)?;
        let end = s.doc.insert_object(&at, field, &props)?;
        s.sel = Selection::caret(end);
        return sel_result(s);
    }
    let header = position == "top";
    let id = hf_part(s, header);
    let story = StoryRef::Part(id);
    let style = if header { "Header" } else { "Footer" };
    let mut para = page_number_paragraph(style, align, x_of_y)?;
    // Replace only a top-level paragraph matching our generated structure. User text,
    // field switches/metadata and paragraph/run formatting must not be mistaken for a gallery number.
    // Cached field results and paragraph-mark formatting survive gallery changes and identical requests.
    if story_has_page_field(&s.doc, story, None) {
        let generated = s.doc.story(story).and_then(|blocks| {
            blocks.iter().enumerate().find_map(|(i, block)| {
                let Block::Para(existing) = &**block else { return None };
                let Some(align @ (Align::Left | Align::Center | Align::Right)) = existing.props.align else { return None };
                [false, true]
                    .into_iter()
                    .any(|x_of_y| {
                        page_number_paragraph(style, align, x_of_y).is_ok_and(|mut p| {
                            if p.objects.len() != existing.objects.len() {
                                return false;
                            }
                            p.mark = existing.mark.clone();
                            copy_page_number_results(&mut p, existing);
                            *existing == p
                        })
                    })
                    .then_some((i, existing))
            })
        });
        // More than one PAGE makes ownership ambiguous, including in tables or anchored text
        // boxes. The traversal visits each story once so cycles cannot hang insertion.
        if let Some((i, existing)) = generated
            && !story_has_page_field(&s.doc, story, Some(i))
        {
            para.mark = existing.mark.clone();
            copy_page_number_results(&mut para, existing);
            let replacement = para_block(para);
            if s.doc.story(story).and_then(|blocks| blocks.get(i)) != Some(&replacement) {
                let block = s.doc.story_mut(story)?.get_mut(i).ok_or_else(|| CmdError::Failed("page-number paragraph is missing".into()))?;
                *block = replacement;
            }
        }
        return Ok(json!({"story": id}));
    }
    let blocks = s.doc.story_mut(story)?;
    // A DOCX round trip can add paragraph-mark formatting to the otherwise untouched
    // Header/Footer placeholder. Reuse it without losing that formatting or adding a blank line.
    if let [block] = blocks.as_slice()
        && let Block::Para(existing) = &**block
    {
        let mut placeholder = Paragraph::new().styled(style);
        placeholder.mark = existing.mark.clone();
        if *existing == placeholder {
            para.mark = existing.mark.clone();
            blocks.clear();
        }
    }
    blocks.push(para_block(para));
    Ok(json!({"story": id}))
}

/// Insert date and time at the caret or selection, formatted with a Word picture or default format.
pub(super) fn date_time(s: &mut Session, v: &Value) -> CmdResult {
    let language = p::str(v, "language");
    if language.is_some_and(|l| !matches!(l, "en-US" | "pt-BR")) {
        return Err(CmdError::Params("language must be en-US or pt-BR".into()));
    }
    let mut props = s.typing_props();
    if let Some(lang) = language {
        props.lang = Some(lang.to_string());
    }
    // Resolve the same run language used by refresh_date_fields on subsequent updates.
    // Resolving language from the selection start ordered().0 is intentional for the fixed-text path
    // so the typed text takes the character/paragraph styling where the insertion begins.
    let effective_language = s.doc.para_at(&s.sel.ordered().0).and_then(|p| s.doc.styles.resolve_char(p.props.style.as_deref(), &props).lang);
    // A missing picture follows the selected or inherited language, just like a DATE field
    // without a \\@ switch; an explicitly supplied picture is never overridden.
    let fmt = p::str(v, "format").unwrap_or_else(|| default_date_picture(effective_language.as_deref()));
    if fmt.is_empty() || fmt.len() > 120 || fmt.contains('"') || fmt.contains('\\') {
        return Err(CmdError::Params("invalid date picture".into()));
    }
    let text = format_date_in(fmt, effective_language.as_deref());
    if p::bool(v, "update").unwrap_or(false) {
        let at = delete_selection(s)?;
        let end = s.doc.insert_object(
            &at,
            InlineObject::Field { instr: format!("DATE \\@ \"{fmt}\""), result: text, locked: false, dirty: false, code: false, form: String::new() },
            &props,
        )?;
        s.sel = Selection::caret(end);
    } else {
        type_text_with_lang(s, &text, language)?;
    }
    sel_result(s)
}

/// Format today's date with a Word picture (`M/d/yyyy`, `MMMM d, yyyy`, `yyyy-MM-dd`, `dddd`…),
/// in English.
pub fn format_date(fmt: &str) -> String {
    format_date_in(fmt, None)
}

/// Format today's date with a Word picture, month and weekday names in the language `lang` (a
/// tag such as `pt-BR`; English when `None` or a language without names here).
pub fn format_date_in(fmt: &str, lang: Option<&str>) -> String {
    let now = crate::cmd::now_unix();
    format_date_at_in(fmt, lang, now, crate::cmd::local_offset_secs(now))
}

/// Format the instant `unix` (seconds since the epoch, UTC) with a Word picture, in the zone
/// `offset_secs` east of UTC.
pub fn format_date_at(fmt: &str, unix: u64, offset_secs: i64) -> String {
    format_date_at_in(fmt, None, unix, offset_secs)
}

/// Format the instant `unix` (seconds since the epoch, UTC) with a Word picture, in the zone
/// `offset_secs` east of UTC, with language `lang`.
pub fn format_date_at_in(fmt: &str, lang: Option<&str>, unix: u64, offset_secs: i64) -> String {
    let (y, m, d, hms, days) = crate::cmd::local_civil(unix, offset_secs);
    format_date_parts_in(fmt, lang, y, m as usize, d, hms, days)
}

/// Month and weekday names of a language, as Word writes them in date fields: full names, the
/// three-letter forms (`MMM`, `ddd`) being their first three letters, without a full stop.
/// Weekdays start on Thursday (1970-01-01).
struct DateNames {
    /// Full month names (January through December).
    months: [&'static str; 12],
    /// Weekday names starting on Thursday (1970-01-01).
    days: [&'static str; 7],
    /// The picture of a `DATE` field without `\@`.
    default: &'static str,
}

const ENGLISH: DateNames = DateNames {
    months: ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
    days: ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"],
    default: "M/d/yyyy",
};

/// Portuguese (observed in Word for Mac with a pt-BR document): lower-case names, `MMM` = `out`,
/// `ddd` = `qui`, and `dd/MM/yyyy` for a field without a picture.
const PORTUGUESE: DateNames = DateNames {
    months: ["janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro", "outubro", "novembro", "dezembro"],
    days: ["quinta-feira", "sexta-feira", "sábado", "domingo", "segunda-feira", "terça-feira", "quarta-feira"],
    default: "dd/MM/yyyy",
};

/// Choose month and weekday names according to the primary language subtag.
fn date_names(lang: Option<&str>) -> &'static DateNames {
    let primary = lang.and_then(|l| l.trim().split(['-', '_']).next()).unwrap_or("");
    if primary.eq_ignore_ascii_case("pt") { &PORTUGUESE } else { &ENGLISH }
}

/// The picture Word gives a `DATE` field that has no `\@` switch, in the field's language.
pub fn default_date_picture(lang: Option<&str>) -> &'static str {
    date_names(lang).default
}

/// Format a date and time with a Word picture; `days_since` (days since 1970-01-01) gives the weekday.
pub fn format_date_parts(fmt: &str, y: i64, m: usize, d: u32, hms: (u32, u32, u32), days_since: i64) -> String {
    format_date_parts_in(fmt, None, y, m, d, hms, days_since)
}

/// [`format_date_parts`] with month and weekday names in `lang`. Text between single quotes is
/// copied as it is (`'de'`), while doubled apostrophes emit one literal quote anywhere.
#[allow(clippy::too_many_arguments)]
pub fn format_date_parts_in(fmt: &str, lang: Option<&str>, y: i64, m: usize, d: u32, (hh, mm, ss): (u32, u32, u32), days_since: i64) -> String {
    let names = date_names(lang);
    let month = names.months.get(m.saturating_sub(1)).copied().unwrap_or(names.months[0]);
    let day = names.days.get(days_since.rem_euclid(7) as usize).copied().unwrap_or(names.days[0]);
    let three = |s: &str| -> String { s.chars().take(3).collect() };
    let mut out = String::new();
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars.get(i).copied().unwrap_or(' ');
        // Text in single quotes is shown as it is; two quotes in a row are one, inside quoted
        // text or out of it.
        if c == '\'' && chars.get(i + 1) == Some(&'\'') {
            out.push('\'');
            i += 2;
            continue;
        }
        if c == '\'' {
            i += 1;
            while let Some(&q) = chars.get(i) {
                i += 1;
                if q != '\'' {
                    out.push(q);
                } else if chars.get(i) == Some(&'\'') {
                    out.push('\'');
                    i += 1;
                } else {
                    break;
                }
            }
            continue;
        }
        // `am/pm` (any case) is one token: AM or PM.
        if chars.get(i..i + 5).is_some_and(|w| w.iter().collect::<String>().eq_ignore_ascii_case("am/pm")) {
            out.push_str(if hh < 12 { "AM" } else { "PM" });
            i += 5;
            continue;
        }
        let mut n = 1;
        while chars.get(i + n) == Some(&c) {
            n += 1;
        }
        match (c, n) {
            ('y' | 'Y', 4..) => out.push_str(&format!("{y:04}")),
            ('y' | 'Y', _) => out.push_str(&format!("{:02}", y % 100)),
            ('M', 4..) => out.push_str(month),
            ('M', 3) => out.push_str(&three(month)),
            ('M', 2) => out.push_str(&format!("{m:02}")),
            ('M', 1) => out.push_str(&m.to_string()),
            ('d' | 'D', 4..) => out.push_str(day),
            ('d' | 'D', 3) => out.push_str(&three(day)),
            ('d' | 'D', 2) => out.push_str(&format!("{d:02}")),
            ('d' | 'D', 1) => out.push_str(&d.to_string()),
            ('H', 2..) => out.push_str(&format!("{hh:02}")),
            ('H', _) => out.push_str(&hh.to_string()),
            ('h', 2..) => out.push_str(&format!("{:02}", if hh.is_multiple_of(12) { 12 } else { hh % 12 })),
            ('h', _) => out.push_str(&(if hh.is_multiple_of(12) { 12 } else { hh % 12 }).to_string()),
            ('m', 2..) => out.push_str(&format!("{mm:02}")),
            ('m', _) => out.push_str(&mm.to_string()),
            ('s', 2..) => out.push_str(&format!("{ss:02}")),
            ('s', _) => out.push_str(&ss.to_string()),
            ('a' | 'A', _) => out.push_str(if hh < 12 { "AM" } else { "PM" }),
            _ => {
                for _ in 0..n {
                    out.push(c);
                }
            }
        }
        i += n;
    }
    out
}

/// Insert › Symbol: the character at the caret, in its own font when `font` names one (the rest of
/// the text keeps the caret's font).
pub(super) fn symbol(s: &mut Session, v: &Value) -> CmdResult {
    let c = p::req_str(v, "char")?;
    if c.is_empty() || c.chars().count() > 64 {
        return Err(CmdError::Params("char: 1–64 characters".into()));
    }
    let font = p::str(v, "font").map(str::trim).filter(|f| !f.is_empty());
    let Some(font) = font else {
        type_text(s, c)?;
        return sel_result(s);
    };
    if font.len() > 256 {
        return Err(CmdError::Params("font: a family name".into()));
    }
    let base = s.typing_props();
    // A symbol font's character remembers it as its own font (`w:sym`), so changing the text's
    // font later keeps the symbol; other fonts are ordinary run formatting. `w:sym` holds a 16-bit
    // code, so a character beyond the Basic Multilingual Plane stays ordinary text in that font.
    // Each character decides for itself, so a mixed string keeps its symbols and its text apart.
    let font_is_symbol = wordcraft_doc::symbol_fonts::is_symbol_font(font);
    let mut r = Ok(());
    let mut buf = [0u8; 4];
    for ch in c.chars() {
        let mut props = base.clone();
        let private_use = ('\u{F020}'..='\u{F0FF}').contains(&ch);
        if u32::from(ch) <= 0xFFFF && (font_is_symbol || private_use) {
            props.sym_font = Some(font.to_string());
        } else {
            props.font = Some(font.to_string());
        }
        s.pending = Some(props);
        r = type_text(s, ch.encode_utf8(&mut buf));
        if r.is_err() {
            break;
        }
    }
    // Typing on after the symbol goes on in the text's own font.
    s.pending = Some(base);
    r?;
    sel_result(s)
}

pub(super) fn field(s: &mut Session, v: &Value) -> CmdResult {
    let instr = p::req_str(v, "instr")?.trim().to_string();
    if instr.is_empty() || instr.len() > 4096 {
        return Err(CmdError::Params("bad field code".into()));
    }
    let result = p::str(v, "result").unwrap_or("").to_string();
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let end =
        s.doc.insert_object(&at, InlineObject::Field { instr, result, locked: false, dirty: false, code: false, form: String::new() }, &props)?;
    s.sel = Selection::caret(end);
    let _ = crate::cmd::references::update_fields(s);
    sel_result(s)
}

pub(super) fn cover_page(s: &mut Session, v: &Value) -> CmdResult {
    let title = p::str(v, "title").unwrap_or("Document Title").to_string();
    let subtitle = p::str(v, "subtitle").unwrap_or("Document subtitle").to_string();
    let author = p::str(v, "author").unwrap_or(&s.author).to_string();
    let accent = Rgb(0x15, 0x60, 0x82);
    let mut blocks = Vec::new();
    for _ in 0..8 {
        blocks.push(Block::Para(Paragraph::new()));
    }
    let mut t =
        Paragraph::with_text(&title, CharProps { size: Some(44.0), color: Some(TextColor::Rgb(accent)), ..Default::default() }).styled("Title");
    t.props.borders = Some(wordcraft_doc::props::Borders {
        bottom: Some(wordcraft_doc::props::Border { style: wordcraft_doc::props::BorderStyle::Single, width: 2.0, color: Some(accent), space: 6.0 }),
        ..Default::default()
    });
    blocks.push(Block::Para(t));
    blocks.push(Block::Para(Paragraph::with_text(&subtitle, CharProps::default()).styled("Subtitle")));
    for _ in 0..14 {
        blocks.push(Block::Para(Paragraph::new()));
    }
    blocks
        .push(Block::Para(Paragraph::with_text(&author, CharProps { bold: Some(true), color: Some(TextColor::Rgb(accent)), ..Default::default() })));
    let mut date = Paragraph::with_text(&format_date("MMMM d, yyyy"), CharProps::default());
    date.insert_text(date.len(), "\u{000C}", &CharProps::default())?;
    blocks.push(Block::Para(date));
    let frag = wordcraft_doc::edit::Fragment { blocks, ..Default::default() };
    let start = s.doc.start_of(StoryRef::Body);
    let at = s.doc.split_paragraph(&start)?;
    let _ = at;
    s.doc.insert_fragment(&start, &frag)?;
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    sel_result(s)
}

/// Why Insert › Drop Cap is off at the caret, as in Word: in a table cell, in an empty paragraph
/// and in one that starts with a space. A selection must be fit throughout (what Dropped and In
/// margin need), unless a paragraph in it has a drop cap for None to take away.
pub(super) fn drop_cap_off(s: &Session) -> Option<&'static str> {
    let why = drop_cap_selection_off(s)?;
    let story = s.sel.focus.story;
    let has_cap = crate::cmd::selected_paths(s).iter().any(|path| s.doc.para(story, path).is_some_and(|p| p.props.drop_cap.is_some()));
    (!has_cap).then_some(why)
}

fn drop_cap_first_letter(first: Option<char>) -> Option<&'static str> {
    match first {
        None => Some("the paragraph is empty"),
        Some(c) if c.is_whitespace() => Some("the paragraph starts with a space"),
        Some(_) => None,
    }
}

/// The same check for every selected paragraph, as the command formats them all.
fn drop_cap_selection_off(s: &Session) -> Option<&'static str> {
    let story = s.sel.focus.story;
    for path in crate::cmd::selected_paths(s) {
        if path.cell().is_some() {
            return Some("drop caps can't go in a table");
        }
        let first = s.doc.para(story, &path).and_then(|p| p.text.chars().next());
        if let Some(why) = drop_cap_first_letter(first) {
            return Some(why);
        }
    }
    None
}

/// Insert › Drop Cap and Drop Cap Options: none, or the letter dropped into the text or hung in
/// the margin, `lines` tall, `distance` points from the text, in `font` (unset: the text's).
/// None takes all of it away, the letter's font with it.
pub(super) fn drop_cap(s: &mut Session, v: &Value) -> CmdResult {
    let lines = v.get("lines").and_then(Value::as_f64).filter(|l| l.is_finite());
    let none = p::str(v, "position") == Some("none") || lines == Some(0.0);
    let margin = p::str(v, "position") == Some("margin");
    let lines_given = lines.is_some();
    let lines = lines.map_or(3.0, |l| l.round().clamp(1.0, 10.0)) as u8;
    let distance = p::f32(v, "distance").map_or(0.0, |d| d.clamp(0.0, wordcraft_doc::resolve::MAX_DROP_CAP_DISTANCE));
    // A family name, not a paragraph of text.
    let font = p::str(v, "font").map(str::trim).filter(|f| !f.is_empty()).map(|f| f.chars().take(64).collect::<String>());
    if !none && let Some(why) = drop_cap_selection_off(s) {
        return Err(CmdError::Failed(why.into()));
    }
    // `keep` (the menu's Dropped / In margin): a paragraph that has a drop cap keeps its own
    // lines, distance and font unless given here.
    let keep = !none && v.get("keep").and_then(Value::as_bool) == Some(true);
    let (distance_given, font_given) = (p::f32(v, "distance").is_some(), v.get("font").is_some());
    crate::cmd::para::fmt(s, &|p| {
        let own = keep && p.drop_cap.is_some();
        p.drop_cap = (!none).then(|| if own && !lines_given { p.drop_cap.unwrap_or(lines) } else { lines });
        p.drop_cap_margin = (!none && margin).then_some(true);
        let d = if own && !distance_given { p.drop_cap_distance.unwrap_or(0.0) } else { distance };
        p.drop_cap_distance = (!none && d > 0.0).then_some(d);
        if !(own && !font_given) {
            p.drop_cap_font = font.clone().filter(|_| !none);
        }
    })
}

/// Insert › Drop Cap at the caret: the menu's highlight and the Drop Cap dialog's values.
#[derive(Clone, Debug, PartialEq)]
pub struct DropCapState {
    /// `none`, `dropped` or `margin`.
    pub position: &'static str,
    /// Lines to drop (3 when there is no drop cap).
    pub lines: u8,
    /// Distance from the text, points.
    pub distance: f32,
    /// The letter's own font, if it has one.
    pub font: Option<String>,
    /// The font of the paragraph's first character.
    pub text_font: String,
}

pub fn drop_cap_state(s: &Session) -> DropCapState {
    let para = s.doc.para_at(&s.sel.focus);
    let props = para.map(|p| p.props.clone()).unwrap_or_default();
    let rp = s.doc.styles.resolve_para(&props);
    let first = para.map(|p| p.props_at(0).clone()).unwrap_or_default();
    let text_font = s.doc.styles.resolve_char_in(props.style.as_deref(), None, &first).font;
    DropCapState {
        position: match (rp.drop_cap, rp.drop_cap_margin) {
            (0, _) => "none",
            (_, true) => "margin",
            _ => "dropped",
        },
        lines: if rp.drop_cap == 0 { 3 } else { rp.drop_cap },
        distance: if rp.drop_cap == 0 { 0.0 } else { rp.drop_cap_distance },
        font: rp.drop_cap_font.filter(|_| rp.drop_cap > 0),
        text_font,
    }
}

#[cfg(test)]
mod date_time_locale_tests {
    use super::*;
    /// A DATE field without an explicit language initially uses the paragraph's and document's
    /// inherited pt-BR language, just as the subsequent refresh will.
    #[test]
    fn date_field_inherits_document_default_locale_on_first_render() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        std::sync::Arc::make_mut(&mut s.doc.styles).default_chr.lang = Some("pt-BR".into());
        s.run(
            "insert.dateTime",
            &json!({
                "format": "d 'de' MMMM 'de' yyyy", "update": true
            }),
        )
        .unwrap();

        let path = s.doc.para_paths(StoryRef::Body).remove(0);
        let p = s.doc.para(StoryRef::Body, &path).unwrap();
        let InlineObject::Field { result, .. } = &p.objects[0] else { panic!("expected DATE field") };
        let expected = format_date_in("d 'de' MMMM 'de' yyyy", Some("pt-BR"));
        assert_eq!(*result, expected);
        assert!(result.contains(" de "));
        let before = result.clone();
        crate::cmd::references::refresh_date_fields(&mut s.doc);
        let p = s.doc.para(StoryRef::Body, &path).unwrap();
        let InlineObject::Field { result, .. } = &p.objects[0] else { panic!("expected DATE field") };
        assert_eq!(*result, before, "refresh must not switch languages");
    }

    /// Fixed date text uses the selected pt-BR editing language, without dropping the
    /// caret's other character formatting or making the insertion refreshable.
    #[test]
    fn fixed_date_text_keeps_selected_language() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run(
            "insert.dateTime",
            &json!({
                "format": "d 'de' MMMM 'de' yyyy", "language": "pt-BR", "update": false
            }),
        )
        .unwrap();
        let path = s.doc.para_paths(StoryRef::Body).remove(0);
        let p = s.doc.para(StoryRef::Body, &path).unwrap();
        assert_eq!(p.props_of_char(0).lang.as_deref(), Some("pt-BR"));
        assert_eq!(p.plain_text(), format_date_in("d 'de' MMMM 'de' yyyy", Some("pt-BR")));
        assert!(p.objects.iter().all(|o| !matches!(o, InlineObject::Field { .. })));
    }

    /// Word's doubled apostrophe escapes one literal quote both before and inside a quoted run.
    /// The existing three-character abbreviations must preserve accented Portuguese characters.
    #[test]
    fn word_date_picture_doubled_apostrophes_and_accented_abbreviations() {
        let format = |pic, day| format_date_parts_in(pic, Some("pt-BR"), 2026, 10, day, (12, 0, 0), 2);
        assert_eq!(format("d''MM", 20), "20'10");
        assert_eq!(format("d 'd''água' MMMM", 20), "20 d'água outubro");
        assert_eq!(format("ddd", 20), "sáb");
        assert_eq!(format("MMM", 20), "out");
        assert_eq!(format("MMMM", 20), "outubro");
    }

    /// A pt-BR dynamic field keeps its result after DOCX round-trip and refresh.
    #[test]
    fn pt_br_date_time_dialog_inserts_dynamic_field() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.dateTime", &json!({"format": "d 'de' MMMM 'de' yyyy", "language": "pt-BR", "update": true})).unwrap();
        let bytes = wordcraft_docx::write(&s.doc).unwrap();
        let mut doc = wordcraft_docx::read(&bytes).unwrap();
        crate::cmd::references::refresh_date_fields(&mut doc);
        let paths = doc.para_paths(StoryRef::Body);
        let p = doc.para(StoryRef::Body, &paths[0]).unwrap();
        let InlineObject::Field { result, .. } = &p.objects[0] else { panic!("expected field") };
        assert_eq!(result, &format_date_in("d 'de' MMMM 'de' yyyy", Some("pt-BR")));
    }
}
