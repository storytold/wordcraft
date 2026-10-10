//! Liang hyphenation patterns (TeX notation, e.g. `.ab1c`, `4t1ion`): parsing and application.
//!
//! The bundled English set is *ours*: trained from the public-domain Moby Hyphenator word list by
//! DesignCraft's patgen (see `examples/hyphgen.rs`). The Polish set is the TeX one by Hanna
//! Kołodziejska, Bogusław Jackowski and Marek Ryćko (hyph-utf8 `hyph-pl.tex`, used under its MIT
//! licence option), bundled unmodified: [`Patterns::parse`] reads TeX's `\patterns{…}` and
//! `\hyphenation{…}` blocks directly. Patterns are stored in a hash map keyed by the packed letter
//! string, so applying them to a word costs `O(len × max_pattern_len)` lookups.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Word-boundary marker in patterns.
pub const DOT: char = '.';
/// Longest pattern the packed key supports (6 bits a letter in a `u128`; the Polish set's longest
/// is 14 including the dots).
pub const MAX_PATTERN_LEN: usize = 20;

/// Small, fast multiplicative hasher for packed integer keys and short strings (FxHash-style).
#[derive(Default, Clone, Copy)]
pub struct FxHasher(u64);

impl Hasher for FxHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for c in chunks {
            self.write_u64(u64::from_le_bytes(*c));
        }
        for &b in rest {
            self.write_u64(b as u64);
        }
    }
    fn write_u8(&mut self, i: u8) {
        self.write_u64(i as u64);
    }
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_u128(&mut self, i: u128) {
        self.write_u64(i as u64);
        self.write_u64((i >> 64) as u64);
    }
}

pub type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

/// Maps characters to 6-bit codes (1 = word boundary, 2.. = letters); 0 = not in the alphabet.
#[derive(Clone, Debug)]
pub struct Alphabet {
    ascii: [u8; 128],
    other: Vec<(char, u8)>,
    next: u8,
}

impl Default for Alphabet {
    fn default() -> Self {
        let mut a = Alphabet { ascii: [0; 128], other: Vec::new(), next: 2 };
        a.ascii[DOT as usize] = 1;
        a
    }
}

impl Alphabet {
    pub fn code(&self, c: char) -> u8 {
        if (c as u32) < 128 { self.ascii[c as usize] } else { self.other.iter().find(|x| x.0 == c).map_or(0, |x| x.1) }
    }
    /// Code for `c`, adding it if new (None when the 6-bit alphabet is full).
    pub fn intern(&mut self, c: char) -> Option<u8> {
        let k = self.code(c);
        if k != 0 {
            return Some(k);
        }
        if self.next >= 63 {
            return None;
        }
        let k = self.next;
        self.next += 1;
        if (c as u32) < 128 {
            self.ascii[c as usize] = k;
        } else {
            self.other.push((c, k));
        }
        Some(k)
    }
}

/// Packed key of a code string (≤ [`MAX_PATTERN_LEN`] codes): 6 bits a code, the length on top.
#[inline]
pub fn pack(codes: &[u8]) -> u128 {
    let mut k = 0u128;
    for &c in codes {
        k = (k << 6) | c as u128;
    }
    k | ((codes.len() as u128) << 120)
}

/// A compiled pattern set, with its exception words (TeX's `\hyphenation{…}`).
#[derive(Clone, Debug, Default)]
pub struct Patterns {
    alpha: Alphabet,
    /// Packed letters → inter-letter values (`len + 1` entries).
    map: FxMap<u128, Box<[u8]>>,
    max_len: usize,
    /// Lowercase word → its break points (char indices), overriding the patterns.
    exceptions: HashMap<String, Box<[usize]>>,
}

/// Which TeX block [`Patterns::parse`] is reading.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    Patterns,
    Exceptions,
}

impl Patterns {
    /// Parse patterns: whitespace-separated, `%` comments to the end of the line (`#` too at the
    /// start of a line). A whole TeX pattern file works as is: `\patterns{…}` holds patterns and
    /// `\hyphenation{…}` exception words written with their hyphens (`ni-gdy`); outside any
    /// block, a token with a hyphen is an exception and anything else a pattern.
    pub fn parse(text: &str) -> Patterns {
        let mut p = Patterns::default();
        let mut block = Block::Patterns;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let line = line.split('%').next().unwrap_or("");
            for tok in line.split_whitespace() {
                let mut tok = tok;
                if let Some(cmd) = tok.strip_prefix('\\') {
                    let (name, rest) = cmd.split_once('{').unwrap_or((cmd, ""));
                    block = if name == "hyphenation" { Block::Exceptions } else { Block::Patterns };
                    tok = rest;
                }
                let closes = tok.ends_with('}');
                let tok = tok.trim_end_matches('}');
                if !tok.is_empty() {
                    if block == Block::Exceptions || tok.contains('-') {
                        p.insert_exception(tok);
                    } else {
                        p.insert(tok);
                    }
                }
                if closes {
                    block = Block::Patterns;
                }
            }
        }
        p
    }

    /// Add one pattern like `a1b`; returns false if it can't be represented.
    pub fn insert(&mut self, pat: &str) -> bool {
        let mut codes = Vec::with_capacity(pat.len());
        let mut vals = vec![0u8];
        for c in pat.chars() {
            if let Some(d) = c.to_digit(10) {
                *vals.last_mut().unwrap_or(&mut 0) = d as u8;
            } else {
                let Some(k) = self.alpha.intern(c) else { return false };
                codes.push(k);
                vals.push(0);
            }
        }
        if codes.is_empty() || codes.len() > MAX_PATTERN_LEN {
            return false;
        }
        self.max_len = self.max_len.max(codes.len());
        self.map.insert(pack(&codes), vals.into_boxed_slice());
        true
    }

    /// Add an exception word written with its hyphens (`ni-gdy`; no hyphen: never broken).
    pub fn insert_exception(&mut self, word: &str) {
        let mut plain = String::with_capacity(word.len());
        let mut points = Vec::new();
        let mut n = 0usize;
        for c in word.chars() {
            if c == '-' {
                if n > 0 && points.last() != Some(&n) {
                    points.push(n);
                }
            } else {
                plain.extend(c.to_lowercase());
                n += 1;
            }
        }
        points.retain(|&i| i < n);
        if n > 0 {
            self.exceptions.insert(plain, points.into_boxed_slice());
        }
    }

    /// Break points of an exception word (lowercase), if it is one.
    pub fn exception(&self, word: &str) -> Option<&[usize]> {
        self.exceptions.get(word).map(|b| &b[..])
    }

    /// Number of exception words.
    pub fn exception_count(&self) -> usize {
        self.exceptions.len()
    }

    #[allow(dead_code)]
    pub(crate) fn from_parts(alpha: Alphabet, map: FxMap<u128, Box<[u8]>>, max_len: usize) -> Patterns {
        Patterns { alpha, map, max_len, exceptions: HashMap::new() }
    }

    /// Make `letters` part of the alphabet even if no pattern mentions them.
    pub fn add_letters(&mut self, letters: impl IntoIterator<Item = char>) {
        for c in letters {
            let _ = self.alpha.intern(c);
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Inter-letter values for a lowercase word: `out[i]` is the value *before* char `i`
    /// (`out.len() == word.len() + 1`). None if a char is outside the pattern alphabet.
    pub fn values(&self, word: &[char]) -> Option<Vec<u8>> {
        let mut codes = Vec::with_capacity(word.len() + 2);
        codes.push(1);
        for &c in word {
            let k = self.alpha.code(c);
            if k == 0 {
                return None;
            }
            codes.push(k);
        }
        codes.push(1);
        let mut vals = vec![0u8; codes.len() + 1];
        apply(&self.map, self.max_len, &codes, &mut vals);
        // Padded position p sits before padded char p; word char i is padded char i + 1.
        vals.get(1..codes.len()).map(<[u8]>::to_vec)
    }

    /// Break points (char indices, a hyphen goes before that char) of a lowercase word.
    pub fn points(&self, word: &[char]) -> Option<Vec<usize>> {
        let v = self.values(word)?;
        Some((1..word.len()).filter(|&i| v.get(i).is_some_and(|x| x % 2 == 1)).collect())
    }

    /// The patterns in TeX notation (sorted by letters), e.g. for writing a pattern file.
    pub fn to_tex(&self) -> Vec<String> {
        let mut rev: Vec<(u8, char)> =
            (0u8..128).filter(|&c| self.alpha.ascii[c as usize] != 0).map(|c| (self.alpha.ascii[c as usize], c as char)).collect();
        rev.extend(self.alpha.other.iter().map(|&(c, k)| (k, c)));
        let ch = |k: u8| rev.iter().find(|x| x.0 == k).map_or('?', |x| x.1);
        let mut out: Vec<(String, String)> = self
            .map
            .iter()
            .map(|(&key, vals)| {
                let n = ((key >> 120) as usize).min(MAX_PATTERN_LEN);
                let letters: Vec<char> = (0..n).map(|i| ch(((key >> (6 * (n - 1 - i))) & 63) as u8)).collect();
                let mut s = String::new();
                for (i, l) in letters.iter().enumerate() {
                    if let Some(&v) = vals.get(i).filter(|v| **v > 0) {
                        s.push(char::from(b'0' + v.min(9)));
                    }
                    s.push(*l);
                }
                if let Some(&v) = vals.get(n).filter(|v| **v > 0) {
                    s.push(char::from(b'0' + v.min(9)));
                }
                (letters.into_iter().collect(), s)
            })
            .collect();
        out.sort();
        out.into_iter().map(|x| x.1).collect()
    }
}

/// Apply `map` to padded `codes`, raising `vals` (len = codes.len() + 1; shorter is tolerated).
#[inline]
pub fn apply(map: &FxMap<u128, Box<[u8]>>, max_len: usize, codes: &[u8], vals: &mut [u8]) {
    let n = codes.len();
    for s in 0..n {
        let mut k = 0u128;
        for (len, &c) in codes.iter().skip(s).take(max_len.min(MAX_PATTERN_LEN)).enumerate() {
            k = (k << 6) | c as u128;
            if let Some(v) = map.get(&(k | (((len + 1) as u128) << 120))) {
                for (j, &x) in v.iter().enumerate() {
                    if let Some(slot) = vals.get_mut(s + j)
                        && x > *slot
                    {
                        *slot = x;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liang_example() {
        // The classic example from Liang's thesis: "hy-phen-ation".
        let p = Patterns::parse("hy3ph he2n hena4 hen5at 1na n2at 1tio 2io o2n");
        let w: Vec<char> = "hyphenation".chars().collect();
        assert_eq!(p.points(&w).unwrap(), vec![2, 6]);
        let mut tex = p.to_tex();
        tex.sort();
        assert!(tex.contains(&"hen5at".to_string()));
        assert!(p.values(&['h', 'é']).is_none());
    }

    #[test]
    fn tex_files_parse_with_blocks_comments_and_exceptions() {
        let p = Patterns::parse(
            "% a comment\n\\patterns{\n1na n2at % trailing comment\n.ab4cdefghijklm5n\n}\n\\hyphenation{\nni-gdy\nna-dal\nzawsze\n}\nhy-phen-ation\n",
        );
        assert_eq!(p.len(), 3);
        assert_eq!(p.exception("nigdy"), Some(&[2][..]));
        assert_eq!(p.exception("nadal"), Some(&[2][..]));
        assert_eq!(p.exception("zawsze"), Some(&[][..]));
        assert_eq!(p.exception("hyphenation"), Some(&[2, 6][..]));
        assert_eq!(p.exception_count(), 4);
        // A pattern longer than the old 9-letter limit applies.
        let w: Vec<char> = "abcdefghijklmn".chars().collect();
        assert_eq!(p.values(&w).unwrap()[13], 5);
        assert!(p.to_tex().contains(&".ab4cdefghijklm5n".to_string()));
    }

    #[test]
    fn apply_tolerates_short_value_buffers() {
        let p = Patterns::parse("a1b");
        let mut vals = [0u8; 1];
        apply(&p.map, p.max_len, &[1, 2, 3, 1], &mut vals);
    }
}
