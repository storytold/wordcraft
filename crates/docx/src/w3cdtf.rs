//! W3C date-time ("W3CDTF", W3C NOTE-datetime) check for the core properties dates
//! (`dcterms:created` / `dcterms:modified`, ECMA-376 Part 2 §11). Word reports a file whose
//! typed date isn't one as corrupted, so the reader drops such a value and the writer never
//! writes one. Only real calendar days and offsets within ±14:00 are accepted; times are
//! written with seconds, adding `:00` to minute-only values.

/// Whether `s` is `YYYY`, `YYYY-MM`, `YYYY-MM-DD` or `YYYY-MM-DDThh:mm[:ss[.s+]][TZD]`, with
/// TZD `Z` or `±hh:mm` (the time zone is optional, as in xsd:dateTime).
pub(crate) fn is_w3cdtf(s: &str) -> bool {
    let b = s.as_bytes();
    let Some((year, rest)) = digits(b, 4) else { return false };
    if rest.is_empty() {
        return true;
    }
    let Some((month, rest)) = rest.strip_prefix(b"-").and_then(|r| digits(r, 2)) else { return false };
    if !(1..=12).contains(&month) {
        return false;
    }
    if rest.is_empty() {
        return true;
    }
    let days = match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    let Some(rest) = field(rest, b'-', 1, days) else { return false };
    if rest.is_empty() {
        return true;
    }
    let Some(rest) = field(rest, b'T', 0, 23) else { return false };
    let Some(mut rest) = field(rest, b':', 0, 59) else { return false };
    if rest.first() == Some(&b':') {
        let Some(r) = field(rest, b':', 0, 59) else { return false };
        rest = r;
        if rest.first() == Some(&b'.') {
            let n = rest.iter().skip(1).take_while(|c| c.is_ascii_digit()).count();
            if n == 0 {
                return false;
            }
            rest = rest.get(1 + n..).unwrap_or_default();
        }
    }
    match rest {
        [] | [b'Z'] => true,
        [b'+' | b'-', tz @ ..] => {
            let Some((h, tz)) = digits(tz, 2) else { return false };
            matches!(field(tz, b':', 0, if h == 14 { 0 } else { 59 }), Some([])) && h <= 14
        }
        _ => false,
    }
}

/// `s` as written: a minute-only time gets `:00` seconds (`xs:dateTime` needs them).
pub(crate) fn normalize(s: &str) -> String {
    if is_w3cdtf(s) && s.as_bytes().get(10) == Some(&b'T') && s.as_bytes().get(16) != Some(&b':') {
        // Validation guarantees an ASCII date and time before this insertion point.
        if let (Some(time), Some(zone)) = (s.get(..16), s.get(16..)) {
            return format!("{time}:00{zone}");
        }
    }
    s.to_string()
}

/// `n` leading ASCII digits as a number, and what follows them.
fn digits(b: &[u8], n: usize) -> Option<(u32, &[u8])> {
    let head = b.get(..n)?;
    if !head.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let v = head.iter().fold(0u32, |v, c| v * 10 + u32::from(c - b'0'));
    Some((v, b.get(n..)?))
}

/// `sep` then two digits in `lo..=hi`; returns what follows.
fn field(b: &[u8], sep: u8, lo: u32, hi: u32) -> Option<&[u8]> {
    let (&first, rest) = b.split_first()?;
    if first != sep {
        return None;
    }
    let (v, rest) = digits(rest, 2)?;
    (lo..=hi).contains(&v).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::is_w3cdtf;

    #[test]
    fn forms() {
        for ok in ["2014", "2014-10", "2014-10-28", "2014-10-28T11:34", "2014-10-28T11:34Z", "2014-10-28T11:34:00.123-05:30"] {
            assert!(is_w3cdtf(ok), "{ok}");
        }
        for bad in [
            "",
            "201",
            "20145",
            "2014-1",
            "2014-10-28T",
            "2014-10-28T11",
            "2014-10-28T11:34:00.",
            "2014-10-28T11:34:00+0100",
            "2014-10-28T11:34:00+01:60",
            "2014-10-28T11:34:00Zx",
            "２０１４",
        ] {
            assert!(!is_w3cdtf(bad), "{bad}");
        }
    }

    #[test]
    fn calendar_days_and_offsets() {
        for ok in [
            "2016-02-29",
            "2000-02-29T12:00:00Z",
            "2014-01-31",
            "2014-12-31T23:59:59Z",
            "2014-10-28T11:34:00+14:00",
            "2014-10-28T11:34:00-14:00",
            "2014-10-28T11:34:00+05:45",
        ] {
            assert!(is_w3cdtf(ok), "{ok}");
        }
        for bad in [
            "2014-02-29",
            "1900-02-29T00:00:00Z",
            "2014-02-30",
            "2014-04-31T00:00:00Z",
            "2014-06-31",
            "2014-09-31",
            "2014-11-31",
            "2014-10-28T11:34:00+14:30",
            "2014-10-28T11:34:00-15:00",
            "2014-10-28T11:34:00+23:59",
            "2014-10-28T11:34:00.123-23:59",
        ] {
            assert!(!is_w3cdtf(bad), "{bad}");
        }
    }

    #[test]
    fn normalize_adds_seconds() {
        for (v, want) in [
            ("2014-10-28T11:34Z", "2014-10-28T11:34:00Z"),
            ("2014-10-28T11:34", "2014-10-28T11:34:00"),
            ("2014-10-28T11:34+01:00", "2014-10-28T11:34:00+01:00"),
            ("2014-10-28T11:34-08:00", "2014-10-28T11:34:00-08:00"),
            ("2014-10-28T11:34:00Z", "2014-10-28T11:34:00Z"),
            ("2014-10-28T11:34:00.5+01:00", "2014-10-28T11:34:00.5+01:00"),
            ("2014-10-28", "2014-10-28"),
            ("2014-10", "2014-10"),
            ("2014", "2014"),
        ] {
            assert_eq!(super::normalize(v), want, "{v}");
        }
    }

    #[test]
    fn normalize_leaves_invalid_values_unchanged() {
        for value in ["2014-02-29T11:34Z", "2014-10-28T11:34+14:01", "2014-10-28T11:34-14:01", "2014-10-28T11:３４Z", ""] {
            assert!(!is_w3cdtf(value), "{value}");
            assert_eq!(super::normalize(value), value);
        }
    }
}
