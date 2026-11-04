//! Date and time styles (ODF 1.3 §16.27 `number:date-style`, `number:time-style`) and Word's
//! date-time pictures (`DATE \@ "dddd, MMMM d, yyyy"`), both ways: the parts — day, month,
//! year, weekday, hours, minutes, seconds, AM/PM — and the text between them.

use super::writer::x;

/// Longest picture kept, characters.
const MAX_PICTURE: usize = 200;

/// A part of a date or time.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Part {
    /// Day of the month (long: two digits).
    Day(bool),
    /// Month: long, and by name.
    Month(bool, bool),
    /// Year (long: four digits).
    Year(bool),
    /// Day of the week by name (long: in full).
    Weekday(bool),
    Hours(bool),
    Minutes(bool),
    Seconds(bool),
    AmPm,
    Text(String),
}

/// The part element `name` (local, in the number namespace) stands for; its text, if any,
/// comes after.
pub(super) fn part(name: &str, long: bool, textual: bool) -> Option<Part> {
    Some(match name {
        "day" => Part::Day(long),
        "month" => Part::Month(long, textual),
        "year" => Part::Year(long),
        "day-of-week" => Part::Weekday(long),
        "hours" => Part::Hours(long),
        "minutes" => Part::Minutes(long),
        "seconds" => Part::Seconds(long),
        "am-pm" => Part::AmPm,
        _ => return None,
    })
}

/// The Word picture for `parts`: 12-hour hours when an AM/PM shows, text in single quotes when
/// it holds letters (a quote doubled); quotes and backslashes, which would end the field's
/// argument, left out.
pub(super) fn picture(parts: &[Part]) -> String {
    let ampm = parts.contains(&Part::AmPm);
    let pick = |long: bool, a: &'static str, b: &'static str| if long { a } else { b };
    let mut s = String::new();
    for p in parts {
        match p {
            Part::Day(l) => s.push_str(pick(*l, "dd", "d")),
            Part::Month(l, true) => s.push_str(pick(*l, "MMMM", "MMM")),
            Part::Month(l, false) => s.push_str(pick(*l, "MM", "M")),
            Part::Year(l) => s.push_str(pick(*l, "yyyy", "yy")),
            Part::Weekday(l) => s.push_str(pick(*l, "dddd", "ddd")),
            Part::Hours(l) if ampm => s.push_str(pick(*l, "hh", "h")),
            Part::Hours(l) => s.push_str(pick(*l, "HH", "H")),
            Part::Minutes(l) => s.push_str(pick(*l, "mm", "m")),
            Part::Seconds(l) => s.push_str(pick(*l, "ss", "s")),
            Part::AmPm => s.push_str("AM/PM"),
            Part::Text(t) => {
                let t: String = t.chars().filter(|c| !matches!(c, '"' | '\\')).collect();
                // Letters are quoted; an apostrophe is two, inside quoted text or out of it.
                if t.chars().any(char::is_alphabetic) {
                    s.push('\'');
                    s.push_str(&t.replace('\'', "''"));
                    s.push('\'');
                } else {
                    s.push_str(&t.replace('\'', "''"));
                }
            }
        }
    }
    s.chars().take(MAX_PICTURE).collect()
}

/// The parts of Word picture `pic` (the reverse of [`picture`]).
fn parts_of(pic: &str) -> Vec<Part> {
    let chars: Vec<char> = pic.chars().take(MAX_PICTURE).collect();
    let mut out: Vec<Part> = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    let flush = |text: &mut String, out: &mut Vec<Part>| {
        if !text.is_empty() {
            out.push(Part::Text(std::mem::take(text)));
        }
    };
    while let Some(&c) = chars.get(i) {
        // Two quotes outside quoted text are an apostrophe, as inside it.
        if c == '\'' && chars.get(i + 1) == Some(&'\'') {
            text.push('\'');
            i += 2;
            continue;
        }
        if c == '\'' {
            i += 1;
            while let Some(&q) = chars.get(i) {
                i += 1;
                if q != '\'' {
                    text.push(q);
                } else if chars.get(i) == Some(&'\'') {
                    text.push('\'');
                    i += 1;
                } else {
                    break;
                }
            }
            continue;
        }
        if chars.get(i..i + 5).is_some_and(|w| w.iter().collect::<String>().eq_ignore_ascii_case("am/pm")) {
            flush(&mut text, &mut out);
            out.push(Part::AmPm);
            i += 5;
            continue;
        }
        let mut n = 1;
        while chars.get(i + n) == Some(&c) {
            n += 1;
        }
        let p = match (c, n) {
            ('y' | 'Y', 4..) => Some(Part::Year(true)),
            ('y' | 'Y', _) => Some(Part::Year(false)),
            ('M', 4..) => Some(Part::Month(true, true)),
            ('M', 3) => Some(Part::Month(false, true)),
            ('M', 2) => Some(Part::Month(true, false)),
            ('M', _) => Some(Part::Month(false, false)),
            ('d' | 'D', 4..) => Some(Part::Weekday(true)),
            ('d' | 'D', 3) => Some(Part::Weekday(false)),
            ('d' | 'D', 2) => Some(Part::Day(true)),
            ('d' | 'D', _) => Some(Part::Day(false)),
            ('h' | 'H', n) => Some(Part::Hours(n >= 2)),
            ('m', n) => Some(Part::Minutes(n >= 2)),
            ('s', n) => Some(Part::Seconds(n >= 2)),
            _ => None,
        };
        match p {
            Some(p) => {
                flush(&mut text, &mut out);
                out.push(p);
            }
            None => {
                for _ in 0..n {
                    text.push(c);
                }
            }
        }
        i += n;
    }
    flush(&mut text, &mut out);
    out
}

/// A data style named `name` for Word picture `pic`: a date style when it shows any part of a
/// date, a time style otherwise. `None` for a picture with no part.
pub(super) fn style_xml(name: &str, pic: &str) -> Option<String> {
    let parts = parts_of(pic);
    if !parts.iter().any(|p| !matches!(p, Part::Text(_))) {
        return None;
    }
    let dated = parts.iter().any(|p| matches!(p, Part::Day(_) | Part::Month(..) | Part::Year(_) | Part::Weekday(_)));
    let long = |l: bool| if l { " number:style=\"long\"" } else { "" };
    let body: String = parts
        .iter()
        .map(|p| match p {
            Part::Day(l) => format!("<number:day{}/>", long(*l)),
            Part::Month(l, t) => format!("<number:month{}{}/>", long(*l), if *t { " number:textual=\"true\"" } else { "" }),
            Part::Year(l) => format!("<number:year{}/>", long(*l)),
            Part::Weekday(l) => format!("<number:day-of-week{}/>", long(*l)),
            Part::Hours(l) => format!("<number:hours{}/>", long(*l)),
            Part::Minutes(l) => format!("<number:minutes{}/>", long(*l)),
            Part::Seconds(l) => format!("<number:seconds{}/>", long(*l)),
            Part::AmPm => "<number:am-pm/>".to_string(),
            Part::Text(t) => format!("<number:text>{}</number:text>", x(t)),
        })
        .collect();
    let el = if dated { "number:date-style" } else { "number:time-style" };
    Some(format!("<{el} style:name=\"{name}\">{body}</{el}>"))
}

/// The `\@` picture of Word field code `instr`, if it has one.
pub(super) fn picture_of(instr: &str) -> Option<String> {
    let at = instr.find("\\@")?;
    let rest = instr.get(at + 2..)?.trim_start();
    let pic = match rest.strip_prefix('"') {
        Some(q) => q.split('"').next()?,
        None => rest.split_whitespace().next()?,
    };
    (!pic.is_empty()).then(|| pic.chars().take(MAX_PICTURE).collect())
}

/// A date's parts in the order and with the separators the locale gives them, for a date style
/// with `number:automatic-order` (§19.341): its weekday, month, day and year as US English
/// orders them (LibreOffice's "November 2, 2026", or "11/2/2026" with a numeric month). Its own
/// text between them is left out; a time's parts stay as they are.
pub(super) fn auto_ordered(parts: &[Part]) -> Vec<Part> {
    let find = |f: fn(&Part) -> bool| parts.iter().find(|p| f(p)).cloned();
    let weekday = find(|p| matches!(p, Part::Weekday(_)));
    let month = find(|p| matches!(p, Part::Month(..)));
    let day = find(|p| matches!(p, Part::Day(_)));
    let year = find(|p| matches!(p, Part::Year(_)));
    let Some(month) = month else { return parts.to_vec() };
    let textual = matches!(month, Part::Month(_, true));
    let text = |s: &str| Part::Text(s.to_string());
    let mut out = Vec::new();
    if let Some(w) = weekday {
        out.extend([w, text(", ")]);
    }
    out.push(month);
    if let Some(d) = day {
        out.extend([text(if textual { " " } else { "/" }), d]);
    }
    if let Some(y) = year {
        out.extend([text(if textual { ", " } else { "/" }), y]);
    }
    // A time with it keeps its own parts, after.
    let time: Vec<Part> = parts.iter().filter(|p| matches!(p, Part::Hours(_) | Part::Minutes(_) | Part::Seconds(_) | Part::AmPm)).cloned().collect();
    if !time.is_empty() {
        out.push(text(" "));
        for (i, p) in time.into_iter().enumerate() {
            if i > 0 && !matches!(p, Part::AmPm) {
                out.push(text(":"));
            } else if matches!(p, Part::AmPm) {
                out.push(text(" "));
            }
            out.push(p);
        }
    }
    out
}

/// English month names, in order; a month shown is its name or the start of it (three letters
/// or more).
const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];

/// The value a date or time shown as `shown` under Word picture `pic` stands for, as a fixed
/// field's `text:date-value` / `text:time-value` (§19.871, §19.880) state it: `YYYY-MM-DD`, with
/// `Thh:mm:ss` when it shows a time (on 1899-12-30, ODF's day 0, when it shows no date). `None`
/// when the text doesn't read under the picture (month names in another language, say).
pub(super) fn value_shown(pic: &str, shown: &str) -> Option<String> {
    let parts = parts_of(pic);
    let s: Vec<char> = shown.trim().chars().take(MAX_PICTURE).collect();
    let mut i = 0usize;
    let (mut day, mut month, mut year) = (None, None, None);
    let (mut hour, mut minute, mut second, mut pm) = (None, None, 0u32, None);
    // A run of digits (at most `max`) or of letters at `i`.
    let digits = |i: &mut usize, max: usize| -> Option<u32> {
        let start = *i;
        while *i - start < max && s.get(*i).is_some_and(char::is_ascii_digit) {
            *i += 1;
        }
        s.get(start..*i).filter(|d| !d.is_empty())?.iter().collect::<String>().parse().ok()
    };
    let word = |i: &mut usize| -> String {
        let start = *i;
        while s.get(*i).is_some_and(|c| c.is_alphabetic()) {
            *i += 1;
        }
        s.get(start..*i).map(|w| w.iter().collect::<String>().to_lowercase()).unwrap_or_default()
    };
    for p in &parts {
        match p {
            Part::Text(t) => {
                for c in t.chars() {
                    match s.get(i) {
                        Some(&got) if got.eq_ignore_ascii_case(&c) => i += 1,
                        // Spaces are loose: one shown where the picture has none, or none where it has one.
                        Some(&got) if got.is_whitespace() => i += 1,
                        _ if c.is_whitespace() => {}
                        _ => return None,
                    }
                }
            }
            Part::Day(_) => day = Some(digits(&mut i, 2)?),
            Part::Month(_, false) => month = Some(digits(&mut i, 2)?),
            Part::Month(_, true) => {
                let w = word(&mut i);
                let n = MONTHS.iter().position(|m| w.len() >= 3 && m.starts_with(w.as_str()))?;
                month = Some(u32::try_from(n).ok()? + 1);
            }
            Part::Year(_) => {
                let start = i;
                let y = digits(&mut i, 4)?;
                // Two digits: Word's window, 1930–2029.
                year = Some(if i - start <= 2 { if y < 30 { 2000 + y } else { 1900 + y } } else { y });
            }
            Part::Weekday(_) => {
                if word(&mut i).is_empty() {
                    return None;
                }
            }
            Part::Hours(_) => hour = Some(digits(&mut i, 2)?),
            Part::Minutes(_) => minute = Some(digits(&mut i, 2)?),
            Part::Seconds(_) => second = digits(&mut i, 2)?,
            Part::AmPm => {
                let w = word(&mut i);
                pm = Some(match w.as_str() {
                    "am" | "a" => false,
                    "pm" | "p" => true,
                    _ => return None,
                });
            }
        }
    }
    if i < s.len() {
        return None;
    }
    let time = match (hour, minute) {
        (Some(h), m) => {
            let h = match pm {
                Some(true) if h < 12 => h + 12,
                Some(false) if h == 12 => 0,
                _ => h,
            };
            let m = m.unwrap_or(0);
            (h < 24 && m < 60 && second < 60).then(|| format!("T{h:02}:{m:02}:{second:02}"))?
        }
        (None, Some(_)) => return None,
        (None, None) => String::new(),
    };
    let date = match (year, month, day) {
        (Some(y), Some(m), Some(d)) if (1..=12).contains(&m) && (1..=31).contains(&d) && y <= 9999 => format!("{y:04}-{m:02}-{d:02}"),
        (None, None, None) if !time.is_empty() => "1899-12-30".to_string(),
        _ => return None,
    };
    Some(format!("{date}{time}"))
}

#[cfg(test)]
mod value_tests {
    use super::value_shown;

    #[test]
    fn a_shown_date_or_time_reads_back_under_its_picture() {
        assert_eq!(value_shown("dddd, MMMM dd, yyyy", "Sunday, November 01, 2026").as_deref(), Some("2026-11-01"));
        assert_eq!(value_shown("dd/MM/yy", "03/11/26").as_deref(), Some("2026-11-03"));
        assert_eq!(value_shown("hh:mm AM/PM", "04:04 PM").as_deref(), Some("1899-12-30T16:04:00"));
        assert_eq!(value_shown("d MMM yyyy H:mm:ss", "5 Mar 1999 7:08:09").as_deref(), Some("1999-03-05T07:08:09"));
        // Not readable: another language's month, extra text, an impossible day, junk.
        assert_eq!(value_shown("d MMMM yyyy", "5 mars 2026"), None);
        assert_eq!(value_shown("d MMMM yyyy", "5 March 2026 and more"), None);
        assert_eq!(value_shown("dd/MM/yyyy", "45/11/2026"), None);
        assert_eq!(value_shown("dd/MM/yyyy", ""), None);
        assert_eq!(value_shown("", "anything"), None);
    }

    /// An apostrophe in a date style's text is two quotes in the picture, quoted or not, and
    /// reads back as one.
    #[test]
    fn an_apostrophe_in_a_date_styles_text_round_trips() {
        use super::{Part, parts_of, picture};
        for text in ["'", "''", "o'clock", "'tis", " ' "] {
            let parts = vec![Part::Text(text.to_string())];
            let pic = picture(&parts);
            assert_eq!(parts_of(&pic), parts, "{text:?} as {pic:?}");
        }
        assert_eq!(picture(&[Part::Text("'".into())]), "''");
    }
}
