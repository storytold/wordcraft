//! The small amount of URL encoding OAuth and Drive need: percent-encoding of query values and
//! form bodies, and decoding of the loopback redirect's query string.

/// Percent-encodes everything except RFC 3986 unreserved characters.
pub fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `k=v&k=v` with both sides percent-encoded (works as a query string and as an
/// `application/x-www-form-urlencoded` body).
pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", pct(k), pct(v))).collect::<Vec<_>>().join("&")
}

/// Decodes `%XX` and `+`. Invalid escapes are kept literally; invalid UTF-8 is replaced.
pub fn unpct(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        match c {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => match (b.get(i + 1).and_then(hex), b.get(i + 2).and_then(hex)) {
                (Some(h), Some(l)) => {
                    out.push(h << 4 | l);
                    i += 3;
                }
                _ => {
                    out.push(b'%');
                    i += 1;
                }
            },
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: &u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Parses a query string into decoded pairs, at most `max` of them.
pub fn parse_query(q: &str, max: usize) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .take(max)
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (unpct(k), unpct(v)),
            None => (unpct(p), String::new()),
        })
        .collect()
}

/// Quotes a string literal for a Drive `q` search expression.
pub fn drive_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if matches!(c, '\'' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('\'');
    out
}
