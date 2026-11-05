//! W3C date-time ("W3CDTF", W3C NOTE-datetime) check for the core properties dates
//! (`dcterms:created` / `dcterms:modified`, ECMA-376 Part 2 §11). Word reports a file whose
//! typed date isn't one as corrupted, so the reader drops such a value and the writer never
//! writes one.

/// Whether `s` is `YYYY`, `YYYY-MM`, `YYYY-MM-DD` or `YYYY-MM-DDThh:mm[:ss[.s+]][TZD]`, with
/// TZD `Z` or `±hh:mm` (the time zone is optional, as in xsd:dateTime).
pub(crate) fn is_w3cdtf(s: &str) -> bool {
    let b = s.as_bytes();
    let Some((_, rest)) = digits(b, 4) else { return false };
    if rest.is_empty() {
        return true;
    }
    let Some(rest) = field(rest, b'-', 1, 12) else { return false };
    if rest.is_empty() {
        return true;
    }
    let Some(rest) = field(rest, b'-', 1, 31) else { return false };
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
            matches!(field(tz, b':', 0, 59), Some([])) && h <= 23
        }
        _ => false,
    }
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
        for ok in ["2014", "2014-10", "2014-10-28", "2014-10-28T11:34", "2014-10-28T11:34Z", "2014-10-28T11:34:00.123-23:59"] {
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
}
