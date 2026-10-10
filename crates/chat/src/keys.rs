//! Keys, invite codes and constant-time comparison. The random bytes come from the caller
//! ([`crate::Env::random`]).

const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTVWXYZ23456789";

/// Bytes as lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `XXXX-XXXX-XXXX` from a 30-letter alphabet (about 59 bits).
pub fn invite_code(bytes: &[u8; 12]) -> String {
    let mut s = String::with_capacity(14);
    for (i, x) in bytes.iter().enumerate() {
        if i == 4 || i == 8 {
            s.push('-');
        }
        let idx = usize::from(*x) % CODE_ALPHABET.len();
        s.push(char::from(CODE_ALPHABET.get(idx).copied().unwrap_or(b'A')));
    }
    s
}

/// A code as [`invite_code`] writes it.
pub fn valid_code(code: &str) -> bool {
    code.len() == 14
        && code.char_indices().all(|(i, c)| if i == 4 || i == 9 { c == '-' } else { u8::try_from(c).is_ok_and(|b| CODE_ALPHABET.contains(&b)) })
}

/// Compare two secrets in time that does not depend on where they differ.
pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_keys_from_given_bytes() {
        assert_eq!(hex(&[0, 255, 16]), "00ff10");
        let c = invite_code(&[0; 12]);
        assert_eq!(c, "AAAA-AAAA-AAAA");
        assert!(valid_code(&c));
        assert!(valid_code(&invite_code(&[29, 30, 255, 7, 1, 2, 3, 4, 5, 6, 7, 8])));
        for bad in ["", "AAAA-AAAA-AAA", "AAAA_AAAA-AAAA", "AAAA-AAAA-AAAI", "aaaa-aaaa-aaaa", "AAAA-AAAA-AAAÁ"] {
            assert!(!valid_code(bad), "{bad}");
        }
        assert!(ct_eq("abc", "abc") && !ct_eq("abc", "abd") && !ct_eq("abc", "ab"));
    }
}
