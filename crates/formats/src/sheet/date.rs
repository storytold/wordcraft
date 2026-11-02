/// Whether a number format shows a date, a time or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DateKind {
    Date,
    Time,
    DateTime,
}

/// The built-in number formats (§18.8.30) that show dates or times.
pub(super) fn builtin_date(id: u32) -> Option<DateKind> {
    match id {
        14..=17 | 27..=36 | 50..=58 => Some(DateKind::Date),
        18..=21 | 45..=47 => Some(DateKind::Time),
        22 => Some(DateKind::DateTime),
        _ => None,
    }
}

/// Whether a custom format code (§18.8.31) shows a date or time: its first section, without
/// quoted text, escaped characters, `[…]` (colours, locales, conditions) and `_x` / `*x`
/// padding, has `d` or `y` (date), `h` or `s` (time), or `m` alone (a month).
pub(super) fn custom_date(code: &str) -> Option<DateKind> {
    let mut plain = String::new();
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            ';' => break,
            '"' => {
                for q in chars.by_ref() {
                    if q == '"' {
                        break;
                    }
                }
            }
            '[' => {
                for q in chars.by_ref() {
                    if q == ']' {
                        break;
                    }
                }
            }
            '\\' | '_' | '*' => {
                chars.next();
            }
            c => plain.extend(c.to_lowercase()),
        }
    }
    let has = |c: char| plain.contains(c);
    let date = has('d') || has('y');
    let time = has('h') || has('s');
    match (date, time) {
        (true, true) => Some(DateKind::DateTime),
        (true, false) => Some(DateKind::Date),
        (false, true) => Some(DateKind::Time),
        (false, false) if has('m') && !has('0') && !has('#') => Some(DateKind::Date),
        _ => None,
    }
}

/// A serial date-time (days since the workbook's epoch) in ISO 8601 form.
pub(super) fn serial_date(serial: f64, date1904: bool, kind: DateKind) -> Option<String> {
    // Day 2958465 is 9999-12-31.
    if !serial.is_finite() || !(0.0..2_958_466.0).contains(&serial) {
        return None;
    }
    let mut days = serial.floor() as i64;
    let mut secs = ((serial - serial.floor()) * 86_400.0).round() as i64;
    if secs >= 86_400 {
        days += 1;
        secs -= 86_400;
    }
    // Days from 1970-01-01. The 1900 system counts a 29 February 1900 that never was, so days
    // before it start a day later.
    let epoch = if date1904 {
        -24_107
    } else if days < 60 {
        -25_568
    } else {
        -25_569
    };
    let (y, m, d) = civil(days + epoch);
    let (h, mi, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let time = if s == 0 { format!("{h:02}:{mi:02}") } else { format!("{h:02}:{mi:02}:{s:02}") };
    Some(match kind {
        DateKind::Date => format!("{y:04}-{m:02}-{d:02}"),
        DateKind::Time => time,
        DateKind::DateTime => format!("{y:04}-{m:02}-{d:02} {time}"),
    })
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day).
///
/// The caller's serial guard keeps `z` inside chrono's range, but the Gregorian calendar repeats
/// every 400 years (146_097 days), so this stays exact for any `z` without hand-rolled calendar
/// math: chrono converts the day within its cycle and whole cycles go back into the year (an
/// i128 so the extremes can't overflow). 1970-01-01 is `num_days_from_ce` 719_163.
pub(crate) fn civil(z: i64) -> (i64, i64, i64) {
    use chrono::Datelike;
    let (cycles, rest) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    match i32::try_from(rest + 719_163).ok().and_then(chrono::NaiveDate::from_num_days_from_ce_opt) {
        Some(d) => {
            let y = i128::from(d.year()) + i128::from(cycles) * 400;
            (i64::try_from(y).unwrap_or(if y < 0 { i64::MIN } else { i64::MAX }), i64::from(d.month()), i64::from(d.day()))
        }
        // Unreachable: `rest + 719_163` is always a valid CE day.
        None => (1970, 1, 1),
    }
}

/// An ISO 8601 date or date-time (`2024-03-05`, `2024-03-05T13:45:00.000`) as `yyyy-mm-dd`,
/// with ` hh:mm[:ss]` when the time isn't midnight.
pub(super) fn iso_datetime(v: &str) -> Option<String> {
    let (date, time) = v.split_once('T').unwrap_or((v, ""));
    let mut it = date.splitn(3, '-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d_part = it.next()?;
    let d_digits: String = d_part.chars().take_while(char::is_ascii_digit).collect();
    let d: u32 = d_digits.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let mut out = format!("{y:04}-{m:02}-{d:02}");
    let time_clean = time.split(['Z', '+', '-', '.']).next().unwrap_or("");
    let hms: Vec<u32> = time_clean.split(':').filter_map(|p| p.parse().ok()).collect();
    if let [h, mi, rest @ ..] = hms.as_slice() {
        let s = rest.first().copied().unwrap_or(0);
        if (*h, *mi, s) != (0, 0, 0) {
            out.push_str(&if s == 0 { format!(" {h:02}:{mi:02}") } else { format!(" {h:02}:{mi:02}:{s:02}") });
        }
    }
    Some(out)
}

/// An ISO 8601 duration time (`PT13H45M00S`) or clock time (`13:45[:00]`) as `hh:mm[:ss]`.
pub(super) fn iso_time(v: &str) -> Option<String> {
    let t = v.trim();
    if let Some(t) = t.strip_prefix("PT") {
        let (mut h, mut m, mut s) = (0u64, 0u64, 0f64);
        let mut num = String::new();
        for c in t.chars() {
            match c {
                'H' => h = std::mem::take(&mut num).parse().ok()?,
                'M' => m = std::mem::take(&mut num).parse().ok()?,
                'S' => s = std::mem::take(&mut num).parse().ok()?,
                c if num.len() < 20 => num.push(c),
                _ => return None,
            }
        }
        let mut s = s.round() as u64;
        m = m.saturating_add(s / 60);
        s %= 60;
        h = h.saturating_add(m / 60);
        m %= 60;
        return Some(if s == 0 { format!("{h:02}:{m:02}") } else { format!("{h:02}:{m:02}:{s:02}") });
    }
    // xs:time: hh:mm[:ss[.sss]][Z|+...]
    let hms: Vec<u32> =
        t.split(['Z', '+', '-']).next().unwrap_or("").split(':').filter_map(|p| p.split('.').next().and_then(|x| x.parse().ok())).collect();
    if let [h, mi, rest @ ..] = hms.as_slice() {
        let s = rest.first().copied().unwrap_or(0);
        return Some(if s == 0 { format!("{h:02}:{mi:02}") } else { format!("{h:02}:{mi:02}:{s:02}") });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::iso_time;

    #[test]
    fn duration_seconds_round_up_carries() {
        assert_eq!(iso_time("PT1H59M59.6S").as_deref(), Some("02:00"));
        assert_eq!(iso_time("PT59M59.5S").as_deref(), Some("01:00"));
        assert_eq!(iso_time("PT0H0M59.4S").as_deref(), Some("00:00:59"));
        assert_eq!(iso_time("PT1H2M3S").as_deref(), Some("01:02:03"));
    }
}
