/// A piece of a format section: literal text, a number character (`#`, `0`, `,`, `.`), or the
/// exponent of a scientific format (`E+` when true, `E-` when false).
enum Tok {
    Lit(String),
    Num(char),
    Exp(bool),
}

/// The sections of a format code (positive; negative; zero; text), split outside quotes.
fn sections(code: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted, mut escaped) = (0, false, false);
    for (i, c) in code.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if !quoted => escaped = true,
            '"' => quoted = !quoted,
            ';' if !quoted => {
                out.push(code.get(start..i).unwrap_or(""));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(code.get(start..).unwrap_or(""));
    out
}

/// The first section of a format code.
fn first_section(code: &str) -> &str {
    sections(code).first().copied().unwrap_or("")
}

/// The pieces of one format section. Quoted text, escapes and `[$€-407]` currency symbols are
/// literal; other brackets (`[Red]`) are dropped; `_x` (room for `x`) is a space; `*x` is dropped.
fn format_toks(section: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut chars = section.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => toks.push(Tok::Lit(chars.by_ref().take_while(|q| *q != '"').collect())),
            '[' => {
                let inner: String = chars.by_ref().take_while(|q| *q != ']').collect();
                if let Some(sym) = currency(&inner) {
                    toks.push(Tok::Lit(sym.to_string()));
                }
            }
            '\\' => toks.extend(chars.next().map(|n| Tok::Lit(n.to_string()))),
            '_' => {
                chars.next();
                toks.push(Tok::Lit(" ".to_string()));
            }
            '*' => {
                chars.next();
            }
            'E' | 'e' if matches!(chars.peek(), Some('+' | '-')) => toks.push(Tok::Exp(chars.next() == Some('+'))),
            '#' | '0' | ',' | '.' => toks.push(Tok::Num(c)),
            c => toks.push(Tok::Lit(c.to_string())),
        }
    }
    toks
}

/// The symbol of a `$€-407` bracket content (`[$€-407]`), if it is one.
fn currency(inner: &str) -> Option<&str> {
    inner.strip_prefix('$').map(|rest| rest.split('-').next().unwrap_or(""))
}

/// Tick text for `v` in a number format code (`#,##0`, `0.0`, `0%`, `0.00E+00`, and literal text
/// such as `"€"0.00` or `#,##0.00 [$€-407]`). A negative number takes the second section when
/// there is one, which shows it without a minus (`#,##0;(#,##0)`). `General` shows as many
/// decimals as the step needs.
pub(super) fn format_value(v: f64, code: &str, step: f64) -> String {
    let v = if v.abs() < step * 1e-9 { 0.0 } else { v };
    let secs = sections(code);
    match secs.get(1) {
        Some(neg) if v < 0.0 => format_section(-v, neg, step),
        _ => format_section(v, secs.first().copied().unwrap_or(""), step),
    }
}

/// `v` in one section of a format code.
fn format_section(v: f64, section: &str, step: f64) -> String {
    let toks = format_toks(section);
    let is_num = |t: &Tok| matches!(t, Tok::Num(_) | Tok::Exp(_));
    let lit = |r: &[Tok]| -> String {
        r.iter()
            .filter_map(|t| match t {
                Tok::Lit(l) => Some(l.as_str()),
                _ => None,
            })
            .collect()
    };
    let (Some(s), Some(e)) = (toks.iter().position(is_num), toks.iter().rposition(is_num)) else { return general(v, step) };
    let body = toks.get(s..=e).unwrap_or(&[]);
    let (pre, suf) = (lit(toks.get(..s).unwrap_or(&[])), lit(toks.get(e + 1..).unwrap_or(&[])));
    let digits = |r: &[Tok]| -> String { r.iter().filter_map(|t| if let Tok::Num(c) = t { Some(*c) } else { None }).collect() };
    if let Some(k) = body.iter().position(|t| matches!(t, Tok::Exp(_))) {
        let plus = matches!(body.get(k), Some(Tok::Exp(true)));
        let mantissa = digits(body.get(..k).unwrap_or(&[]));
        let exp_digits = digits(body.get(k + 1..).unwrap_or(&[])).len();
        return format!("{pre}{}{suf}", scientific(v, decimals(&mantissa), exp_digits, plus));
    }
    let body = digits(body);
    let pct = pre.contains('%') || suf.contains('%');
    let v = if pct { v * 100.0 } else { v };
    let digits = format!("{:.*}", decimals(&body), v.abs());
    let (ip, fp) = match digits.split_once('.') {
        Some((a, b)) => (a.to_string(), format!(".{b}")),
        None => (digits.clone(), String::new()),
    };
    let mut grouped = String::new();
    for (k, ch) in ip.chars().enumerate() {
        if body.contains(',') && k > 0 && (ip.len() - k) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let sign = if v < 0.0 && digits.chars().any(|c| c != '0' && c != '.') { "-" } else { "" };
    format!("{sign}{pre}{grouped}{fp}{suf}")
}

/// Decimal places of a number body (`#,##0.00` has 2).
fn decimals(body: &str) -> usize {
    body.split_once('.').map_or(0, |(_, d)| d.chars().filter(|c| matches!(c, '0' | '#')).count()).min(12)
}

/// `v` with no number format (`General`): as many decimals as the tick step needs.
fn general(v: f64, step: f64) -> String {
    let mut d = 0;
    while d < 6 && (step * 10f64.powi(d)).fract().abs() > 1e-9 {
        d += 1;
    }
    let s = format!("{:.*}", d as usize, v);
    if s.trim_start_matches(['-', '0', '.']).is_empty() { s.trim_start_matches('-').to_string() } else { s }
}

/// `v` in scientific notation: `decimals` mantissa decimals, at least `exp_digits` exponent
/// digits, and a `+` on positive exponents when `plus` (`0.00E+00`: 1.23E+04).
fn scientific(v: f64, decimals: usize, exp_digits: usize, plus: bool) -> String {
    let s = format!("{:.*e}", decimals, v.abs());
    let (mantissa, exp) = s.split_once('e').unwrap_or((s.as_str(), "0"));
    let exp: i32 = if v == 0.0 { 0 } else { exp.parse().unwrap_or(0) };
    let sign = if exp < 0 {
        "-"
    } else if plus {
        "+"
    } else {
        ""
    };
    let minus = if v < 0.0 { "-" } else { "" };
    format!("{minus}{mantissa}E{sign}{:0w$}", exp.unsigned_abs(), w = exp_digits.clamp(1, 4))
}

/// A moment as Excel shows a serial day number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Stamp {
    pub(super) y: i64,
    pub(super) m: u32,
    pub(super) d: u32,
    /// Day of the week, 0 = Sunday.
    weekday: u32,
    /// Seconds into the day.
    secs: u32,
}

/// The moment of Excel serial day `serial` as Excel shows it, from 0 ("January 0, 1900") to
/// 31 December 9999. In the 1900 system serial 60 is Excel's phantom 29 February 1900, and
/// serials before it count from 31 December 1899; in the 1904 system (`date1904`) serial 0 is
/// 1 January 1904. `None` outside that range.
pub(super) fn excel_stamp(serial: f64, date1904: bool) -> Option<Stamp> {
    let serial = if date1904 { serial + 1462.0 } else { serial };
    if !(0.0..2_958_466.0).contains(&serial) {
        return None;
    }
    let whole = serial.floor();
    let secs = (((serial - whole) * 86_400.0).round() as u32).min(86_399);
    let day = whole as i64;
    let (y, m, d) = match day {
        0 => (1900, 1, 0),
        60 => (1900, 2, 29),
        1..=59 => civil(day + 1),
        _ => civil(day),
    };
    // Excel's weekdays count serial 1 as a Sunday.
    Some(Stamp { y, m, d, weekday: (day - 1).rem_euclid(7) as u32, secs })
}

/// Calendar date (year, month, day) of day `n` counted from 30 December 1899 (an Excel serial
/// day number). Serial `n` is `n - 25_569` days from 1970-01-01.
///
/// The caller's serial guard keeps `n` inside chrono's range, but the Gregorian calendar repeats
/// every 400 years (146_097 days), so this stays exact for any `n` without hand-rolled calendar
/// math: chrono converts the day within its cycle and whole cycles go back into the year (an
/// i128 so the extremes can't overflow). 1970-01-01 is `num_days_from_ce` 719_163, so serial `n`
/// is CE day `n - 25_569 + 719_163`.
pub(super) fn civil(n: i64) -> (i64, u32, u32) {
    use chrono::Datelike;
    // CE day of serial `n`, in i128 so no `n` overflows.
    let from_ce = i128::from(n) - 25_569 + 719_163;
    let (cycles, rest) = (from_ce.div_euclid(146_097), from_ce.rem_euclid(146_097));
    match i32::try_from(rest).ok().and_then(chrono::NaiveDate::from_num_days_from_ce_opt) {
        Some(date) => {
            let y = i128::from(date.year()) + cycles * 400;
            (i64::try_from(y).unwrap_or(if y < 0 { i64::MIN } else { i64::MAX }), date.month(), date.day())
        }
        // Unreachable: every `rest` in 0..146_097 is a valid CE day.
        None => (1900, 1, 1),
    }
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Whether a format code (outside its quoted and bracketed parts) has date or time fields.
pub(super) fn is_date_code(code: &str) -> bool {
    format_toks_raw(code).any(|c| matches!(c.to_ascii_lowercase(), 'y' | 'd' | 'm' | 'h' | 's'))
}

/// The characters of a format code's first section outside quotes and brackets.
fn format_toks_raw(code: &str) -> impl Iterator<Item = char> + '_ {
    let mut quoted = false;
    let mut bracket = false;
    first_section(code).chars().filter(move |c| match c {
        '"' => {
            quoted = !quoted;
            false
        }
        '[' if !quoted => {
            bracket = true;
            false
        }
        ']' if bracket => {
            bracket = false;
            false
        }
        _ => !quoted && !bracket,
    })
}

/// A piece of a date format: literal text, a field (`y`, `m`, `d`, `h`, `s`, or `n` for minutes)
/// with its length, or AM/PM (`AM/PM` when true, `A/P` when false).
enum DTok {
    Lit(String),
    Field(char, usize),
    AmPm(bool),
}

/// The pieces of a date format's first section. `m` or `mm` right after an hour or right before
/// a seconds field is minutes.
fn date_toks(code: &str) -> Vec<DTok> {
    let section: Vec<char> = first_section(code).chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while let Some(&c) = section.get(i) {
        let at = i;
        i += 1;
        match c {
            '"' => {
                let lit: String = section.get(i..).unwrap_or(&[]).iter().take_while(|q| **q != '"').collect();
                i += lit.chars().count() + 1;
                toks.push(DTok::Lit(lit));
            }
            '[' => {
                let inner: String = section.get(i..).unwrap_or(&[]).iter().take_while(|q| **q != ']').collect();
                i += inner.chars().count() + 1;
                toks.extend(currency(&inner).map(|s| DTok::Lit(s.to_string())));
            }
            '\\' | '_' | '*' => {
                let next = section.get(i).copied();
                i += 1;
                match (c, next) {
                    ('\\', Some(n)) => toks.push(DTok::Lit(n.to_string())),
                    ('_', _) => toks.push(DTok::Lit(" ".to_string())),
                    _ => {}
                }
            }
            'a' | 'A' if starts_ci(&section, at, "AM/PM") => {
                i += 4;
                toks.push(DTok::AmPm(true));
            }
            'a' | 'A' if starts_ci(&section, at, "A/P") => {
                i += 2;
                toks.push(DTok::AmPm(false));
            }
            _ if matches!(c.to_ascii_lowercase(), 'y' | 'm' | 'd' | 'h' | 's') => {
                let mut n = 1;
                while section.get(i).is_some_and(|q| q.eq_ignore_ascii_case(&c)) {
                    i += 1;
                    n += 1;
                }
                toks.push(DTok::Field(c.to_ascii_lowercase(), n));
            }
            c => toks.push(DTok::Lit(c.to_string())),
        }
    }
    mark_minutes(&mut toks);
    toks
}

/// Whether `chars` from `at` on start with `pat`, ignoring ASCII case.
fn starts_ci(chars: &[char], at: usize, pat: &str) -> bool {
    pat.chars().enumerate().all(|(k, p)| chars.get(at + k).is_some_and(|c| c.eq_ignore_ascii_case(&p)))
}

/// Turns the `m`/`mm` fields that mean minutes (after an hour, or before seconds) into `n`.
fn mark_minutes(toks: &mut [DTok]) {
    let fields: Vec<usize> = toks.iter().enumerate().filter(|(_, t)| matches!(t, DTok::Field(..))).map(|(k, _)| k).collect();
    let kind = |k: Option<&usize>, toks: &[DTok]| match k.and_then(|k| toks.get(*k)) {
        Some(DTok::Field(c, _)) => Some(*c),
        _ => None,
    };
    for (j, k) in fields.iter().enumerate() {
        let after_hour = j.checked_sub(1).and_then(|p| kind(fields.get(p), toks)) == Some('h');
        let before_secs = kind(fields.get(j + 1), toks) == Some('s');
        if let Some(DTok::Field(c @ 'm', n)) = toks.get_mut(*k)
            && *n <= 2
            && (after_hour || before_secs)
        {
            *c = 'n';
        }
    }
}

/// A moment in a date format: day, month, year, weekday (`ddd`, `dddd`), hour, minute and
/// second fields replaced (12-hour with AM/PM); quoted text and currency kept, other brackets
/// dropped.
pub(super) fn date_text(t: Stamp, code: &str) -> String {
    let toks = date_toks(code);
    let twelve = toks.iter().any(|k| matches!(k, DTok::AmPm(_)));
    let hour = t.secs / 3600;
    let mut out = String::new();
    for tok in &toks {
        match tok {
            DTok::Lit(s) => out.push_str(s),
            DTok::AmPm(long) => out.push_str(match (hour < 12, long) {
                (true, true) => "AM",
                (false, true) => "PM",
                (true, false) => "A",
                (false, false) => "P",
            }),
            DTok::Field(c, n) => out.push_str(&field_text(t, *c, *n, twelve)),
        }
    }
    out
}

/// One date or time field (`c` as in [`DTok::Field`], `n` letters long).
fn field_text(t: Stamp, c: char, n: usize, twelve: bool) -> String {
    let month = MONTHS.get((t.m as usize).clamp(1, 12) - 1).copied().unwrap_or("");
    let weekday = WEEKDAYS.get(t.weekday as usize).copied().unwrap_or("");
    let pad = |v: u32| if n >= 2 { format!("{v:02}") } else { v.to_string() };
    let hour = t.secs / 3600;
    match c {
        'y' if n <= 2 => format!("{:02}", t.y.rem_euclid(100)),
        'y' => format!("{:04}", t.y),
        'd' if n <= 2 => pad(t.d),
        'd' if n == 3 => weekday.get(..3).unwrap_or(weekday).to_string(),
        'd' => weekday.to_string(),
        'm' if n <= 2 => pad(t.m),
        'm' if n == 3 => month.get(..3).unwrap_or(month).to_string(),
        'm' if n == 4 => month.to_string(),
        'm' => month.get(..1).unwrap_or(month).to_string(),
        'h' if twelve => pad(if hour.is_multiple_of(12) { 12 } else { hour % 12 }),
        'h' => pad(hour),
        'n' => pad(t.secs / 60 % 60),
        _ => pad(t.secs % 60),
    }
}
