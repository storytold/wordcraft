use super::header_footer::hf_part;
use super::{
    Align, Block, CharProps, CmdError, CmdResult, InlineObject, Paragraph, Rgb, Selection, Session, StoryRef, TabAlign, TabStop, TextColor, Value,
    delete_selection, json, p, para_block, sel_result, type_text,
};

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
    let mut para = Paragraph::new().styled(if header { "Header" } else { "Footer" });
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
    s.doc.set_story(StoryRef::Part(id), vec![para_block(para)])?;
    Ok(json!({"story": id}))
}

pub(super) fn date_time(s: &mut Session, v: &Value) -> CmdResult {
    let fmt = p::str(v, "format").unwrap_or("M/d/yyyy");
    let text = format_date(fmt);
    if p::bool(v, "update").unwrap_or(false) {
        let props = s.typing_props();
        let at = delete_selection(s)?;
        let end = s.doc.insert_object(
            &at,
            InlineObject::Field { instr: format!("DATE \\@ \"{fmt}\""), result: text, locked: false, dirty: false, code: false, form: String::new() },
            &props,
        )?;
        s.sel = Selection::caret(end);
    } else {
        type_text(s, &text)?;
    }
    sel_result(s)
}

/// Format today's date with a Word picture (`M/d/yyyy`, `MMMM d, yyyy`, `yyyy-MM-dd`, `dddd`…),
/// the Japanese era and numeral items (`ggge年M月d日`, 和暦) included.
pub fn format_date(fmt: &str) -> String {
    let iso = crate::cmd::now_iso();
    let y: i64 = iso.get(0..4).and_then(|x| x.parse().ok()).unwrap_or(2026);
    let m: usize = iso.get(5..7).and_then(|x| x.parse().ok()).unwrap_or(1);
    let d: u32 = iso.get(8..10).and_then(|x| x.parse().ok()).unwrap_or(1);
    let hh: u32 = iso.get(11..13).and_then(|x| x.parse().ok()).unwrap_or(0);
    let mm: u32 = iso.get(14..16).and_then(|x| x.parse().ok()).unwrap_or(0);
    let ss: u32 = iso.get(17..19).and_then(|x| x.parse().ok()).unwrap_or(0);
    format_date_parts(fmt, y, m, d, (hh, mm, ss), (crate::cmd::now_unix() / 86_400) as i64)
}

/// A Japanese era (元号) and the day it began, for the era date picture items.
struct JapaneseEra {
    /// The full name, e.g. `令和` (`ggg`).
    name: &'static str,
    /// The one-kanji abbreviation, e.g. `令` (`gg`).
    kanji: char,
    /// The Latin initial, e.g. `R` (`g`).
    latin: char,
    /// The era's first day.
    start: (i64, usize, u32),
}

/// The Japanese eras in order. The boundaries are day-accurate: 大正 began 1912-07-30, so
/// 1912-07-29 is still 明治45年.
static JAPANESE_ERAS: [JapaneseEra; 5] = [
    JapaneseEra { name: "明治", kanji: '明', latin: 'M', start: (1868, 1, 25) },
    JapaneseEra { name: "大正", kanji: '大', latin: 'T', start: (1912, 7, 30) },
    JapaneseEra { name: "昭和", kanji: '昭', latin: 'S', start: (1926, 12, 25) },
    JapaneseEra { name: "平成", kanji: '平', latin: 'H', start: (1989, 1, 8) },
    JapaneseEra { name: "令和", kanji: '令', latin: 'R', start: (2019, 5, 1) },
];

/// The era a date falls in: the latest one that had begun by then. A date before 明治 1868-01-25
/// has none, and the era items render nothing (the rest of the picture still renders).
fn japanese_era(y: i64, m: usize, d: u32) -> Option<&'static JapaneseEra> {
    JAPANESE_ERAS.iter().rev().find(|e| (y, m, d) >= e.start)
}

/// A Japanese numeral in counting form — 十七, 二十七, 三十一 — not the formal 壱/弐 forms.
fn japanese_numeral(n: usize) -> String {
    const DIGITS: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    // The items that use this never go past 59 (days ≤ 31, months ≤ 12, minutes and seconds ≤ 59);
    // a junk part falls back to its digits instead of inventing 百/千 forms.
    if n >= 100 {
        return n.to_string();
    }
    let (tens, ones) = (n / 10, n % 10);
    let mut out = String::new();
    if tens > 1 {
        out.push_str(DIGITS.get(tens).copied().unwrap_or(""));
    }
    if tens > 0 {
        out.push('十');
    }
    if ones > 0 || n == 0 {
        out.push_str(DIGITS.get(ones).copied().unwrap_or(""));
    }
    out
}

/// Format a date and time with a Word picture; `days_since` (days since 1970-01-01) gives the
/// weekday.
///
/// Besides the Gregorian items, the Japanese ones of [MS-OE376] 2.16.4.1 are supported: `g`/`gg`/
/// `ggg` the era's Latin initial, its one kanji and its full name, a longer run grouped from the
/// left (`ggg`, then `gg`, then `g`); `e`/`ee` and their other spelling `n`/`nn` the era year,
/// grouped the same way (`ee`, then `e`); `A`, `O`, `C` and `I` the day, month, seconds and minutes
/// in Japanese numerals, repeated for each instance; `aaa`/`aaaa` the weekday's one kanji and its
/// full name.
pub fn format_date_parts(fmt: &str, y: i64, m: usize, d: u32, (hh, mm, ss): (u32, u32, u32), days_since: i64) -> String {
    const MONTHS: [&str; 12] =
        ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    const DAYS: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];
    // The weekday's one kanji, in the same order as `DAYS` (index 0, 1970-01-01, was a Thursday).
    const DAYS_JA: [&str; 7] = ["木", "金", "土", "日", "月", "火", "水"];
    let month = MONTHS.get(m.saturating_sub(1)).copied().unwrap_or("January");
    let day = DAYS.get(days_since.rem_euclid(7) as usize).copied().unwrap_or("Monday");
    let weekday_ja = DAYS_JA.get(days_since.rem_euclid(7) as usize).copied().unwrap_or("月");
    // The era items. The reference defines `g`/`gg`/`ggg` as the Japanese era for every language
    // but ko-KR and zh-TW, and the picture arrives here with no run language; they render
    // unconditionally, and before 明治 there is no era to render.
    let era = japanese_era(y, m, d);
    let era_year = era.map(|e| y - e.start.0 + 1);
    let era_g = era.map(|e| e.latin.to_string()).unwrap_or_default();
    let era_gg = era.map(|e| e.kanji.to_string()).unwrap_or_default();
    let era_ggg = era.map(|e| e.name.to_string()).unwrap_or_default();
    // Era year 1 is a number (`令和1年`): the reference defines `e`/`ee` as year numbers and has no
    // 元年 form (its own examples show 令和27年-style output).
    let era_e = era_year.map(|n| n.to_string()).unwrap_or_default();
    let era_ee = era_year.map(|n| format!("{n:02}")).unwrap_or_default();
    let mut out = String::new();
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars.get(i).copied().unwrap_or(' ');
        // `am/pm` (any case) is one token: AM or PM.
        if chars.get(i..i + 5).is_some_and(|w| w.iter().collect::<String>().eq_ignore_ascii_case("am/pm")) {
            out.push_str(if hh < 12 { "AM" } else { "PM" });
            i += 5;
            continue;
        }
        // `A/P` (or `a/p`) is the other spelling of that token, and must be read before `A`, which
        // is the Japanese day numeral ([MS-OE376] 2.16.4.1). The short spelling is Word's single
        // letter, in the case the picture gives it.
        if chars.get(i..i + 3).is_some_and(|w| w.iter().collect::<String>().eq_ignore_ascii_case("a/p")) {
            out.push(if hh < 12 { c } else { chars.get(i + 2).copied().unwrap_or('P') });
            i += 3;
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
            ('M', 3) => out.push_str(month.get(..3).unwrap_or(month)),
            ('M', 2) => out.push_str(&format!("{m:02}")),
            ('M', 1) => out.push_str(&m.to_string()),
            ('d' | 'D', 4..) => out.push_str(day),
            ('d' | 'D', 3) => out.push_str(day.get(..3).unwrap_or(day)),
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
            // Japanese era items ([MS-OE376] 2.16.4.1): `g` the Latin initial, `gg` one kanji,
            // `ggg` the full name. A longer run is grouped from the left as the reference says
            // (`ggg` first, then `gg`, then `g`), so `gggg` is 令和 followed by R.
            ('g', _) => {
                for _ in 0..n / 3 {
                    out.push_str(&era_ggg);
                }
                match n % 3 {
                    2 => out.push_str(&era_gg),
                    1 => out.push_str(&era_g),
                    _ => {}
                }
            }
            // `e`/`ee` and their other spelling `n`/`nn` the era year, grouped `ee` then `e`.
            ('e' | 'n', _) => {
                for _ in 0..n / 2 {
                    out.push_str(&era_ee);
                }
                if n % 2 == 1 {
                    out.push_str(&era_e);
                }
            }
            // Japanese numerals in counting form: day, month, seconds and minutes. Every instance
            // repeats the content, as the reference specifies (`AA` is 十七十七).
            ('A', _) => out.push_str(&japanese_numeral(d as usize).repeat(n)),
            ('O', _) => out.push_str(&japanese_numeral(m).repeat(n)),
            ('C', _) => out.push_str(&japanese_numeral(ss as usize).repeat(n)),
            ('I', _) => out.push_str(&japanese_numeral(mm as usize).repeat(n)),
            // `aaa` the weekday's one kanji, `aaaa` its full name. Word gates both on the run's
            // `w:lang` (ja-JP or ko-KR; literal text otherwise); the picture here has no run
            // language and the user wrote the item, so it renders.
            ('a', 3) => out.push_str(weekday_ja),
            ('a', 4..) => out.push_str(&format!("{weekday_ja}曜日")),
            // `a` and `aa` keep meaning AM/PM (`am/pm` itself is one token, matched above).
            ('a', _) => out.push_str(if hh < 12 { "AM" } else { "PM" }),
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
/// and in one that starts with a space.
pub(super) fn drop_cap_off(s: &Session) -> Option<&'static str> {
    if s.sel.focus.path.cell().is_some() {
        return Some("drop caps can't go in a table");
    }
    let first = s.doc.para_at(&s.sel.focus).and_then(|p| p.text.chars().next());
    match first {
        None => Some("the paragraph is empty"),
        Some(c) if c.is_whitespace() => Some("the paragraph starts with a space"),
        Some(_) => None,
    }
}

/// Insert › Drop Cap and Drop Cap Options: none, or the letter dropped into the text or hung in
/// the margin, `lines` tall, `distance` points from the text, in `font` (unset: the text's).
/// None takes all of it away, the letter's font with it.
pub(super) fn drop_cap(s: &mut Session, v: &Value) -> CmdResult {
    let lines = v.get("lines").and_then(Value::as_f64).filter(|l| l.is_finite());
    let none = p::str(v, "position") == Some("none") || lines == Some(0.0);
    let margin = p::str(v, "position") == Some("margin");
    let lines = lines.map_or(3.0, |l| l.round().clamp(1.0, 10.0)) as u8;
    let distance = p::f32(v, "distance").map_or(0.0, |d| d.clamp(0.0, wordcraft_doc::resolve::MAX_DROP_CAP_DISTANCE));
    // A family name, not a paragraph of text.
    let font = p::str(v, "font").map(str::trim).filter(|f| !f.is_empty()).map(|f| f.chars().take(64).collect::<String>());
    crate::cmd::para::fmt(s, &|p| {
        p.drop_cap = (!none).then_some(lines);
        p.drop_cap_margin = (!none && margin).then_some(true);
        p.drop_cap_distance = (!none && distance > 0.0).then_some(distance);
        p.drop_cap_font = font.clone().filter(|_| !none);
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
