//! Random keys, invite codes and constant-time comparison.

const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTVWXYZ23456789";

/// `n` random bytes from the OS, hex encoded (2n characters).
pub fn random_hex(n: usize) -> Result<String, getrandom::Error> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b)?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// `XXXX-XXXX-XXXX` from a 30-letter alphabet (about 59 bits).
pub fn invite_code() -> Result<String, getrandom::Error> {
    let mut b = [0u8; 12];
    getrandom::fill(&mut b)?;
    let mut s = String::with_capacity(14);
    for (i, x) in b.iter().enumerate() {
        if i == 4 || i == 8 {
            s.push('-');
        }
        let idx = usize::from(*x) % CODE_ALPHABET.len();
        s.push(char::from(CODE_ALPHABET.get(idx).copied().unwrap_or(b'A')));
    }
    Ok(s)
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
    fn random_hex_has_length_and_varies() {
        let a = random_hex(32).unwrap_or_default();
        let b = random_hex(32).unwrap_or_default();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    #[test]
    fn invite_code_shape() {
        let c = invite_code().unwrap_or_default();
        assert_eq!(c.len(), 14); // XXXX-XXXX-XXXX
        assert_eq!(c.matches('-').count(), 2);
        assert!(c.chars().all(|ch| ch == '-' || "ABCDEFGHJKMNPQRSTVWXYZ23456789".contains(ch)));
    }

    #[test]
    fn ct_eq_works() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "ab"));
    }
}
