//! Spelling word lists: every accepted word form of a language, case kept (German nouns are
//! capitalised, so `Haus` is listed and `haus` is not).
//!
//! On-disk format (`assets/spelling/de.words`): raw-deflate of `"WCWORDS1\n"` followed by one
//! line per word, sorted by the word's bytes. Each line is front-coded against the previous one:
//! a byte with the shared prefix length (in bytes), then the rest of the word, then `\n`.
//! `cargo run -p wordcraft-proof --example wordlist` builds one from a plain list.

use std::sync::OnceLock;

const MAGIC: &[u8] = b"WCWORDS1\n";
/// Inflated size limit: a corrupt or hostile file can't make us allocate more than this.
const MAX_INFLATED: usize = 64 << 20;

/// A sorted list of words.
#[derive(Clone, Debug, Default)]
pub struct WordList {
    words: String,
    offs: Vec<u32>,
}

impl WordList {
    /// Builds a list from words in any order (sorted and de-duplicated here).
    pub fn from_words<I: IntoIterator<Item = String>>(words: I) -> WordList {
        let mut v: Vec<String> = words.into_iter().filter(|w| !w.is_empty() && !w.contains('\n')).collect();
        v.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        v.dedup();
        let mut l = WordList { words: String::new(), offs: vec![0] };
        for w in v {
            l.push(&w);
        }
        l
    }

    fn push(&mut self, w: &str) {
        self.words.push_str(w);
        self.offs.push(u32::try_from(self.words.len()).unwrap_or(u32::MAX));
    }

    pub fn len(&self) -> usize {
        self.offs.len().saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn word(&self, i: usize) -> &str {
        match (self.offs.get(i), self.offs.get(i + 1)) {
            (Some(&a), Some(&b)) => self.words.get(a as usize..b as usize).unwrap_or(""),
            _ => "",
        }
    }

    /// Is `word` listed exactly (case-sensitive)?
    pub fn contains(&self, word: &str) -> bool {
        let (mut lo, mut hi) = (0usize, self.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.word(mid).as_bytes().cmp(word.as_bytes()) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return true,
            }
        }
        false
    }

    /// The words, in byte order.
    pub fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        (0..self.len()).map(|i| self.word(i))
    }

    /// Serializes (deflate-compressed, front-coded).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut raw = MAGIC.to_vec();
        let mut prev: &[u8] = &[];
        for w in self.iter() {
            let b = w.as_bytes();
            let p = prev.iter().zip(b).take_while(|(x, y)| x == y).count().min(255);
            raw.push(p as u8);
            raw.extend_from_slice(b.get(p..).unwrap_or(&[]));
            raw.push(b'\n');
            prev = b;
        }
        miniz_oxide::deflate::compress_to_vec(&raw, 10)
    }

    /// Reads data written by [`WordList::to_bytes`].
    pub fn from_bytes(data: &[u8]) -> Result<WordList, String> {
        let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(data, MAX_INFLATED).map_err(|e| format!("word list: inflate failed: {e:?}"))?;
        let body = raw.strip_prefix(MAGIC).ok_or("word list: bad header")?;
        let mut l = WordList { words: String::with_capacity(body.len()), offs: vec![0] };
        let mut cur: Vec<u8> = Vec::new();
        let mut rest = body;
        while let Some((&p, tail)) = rest.split_first() {
            let end = tail.iter().position(|&b| b == b'\n').ok_or("word list: truncated")?;
            let p = p as usize;
            if p > cur.len() {
                return Err("word list: bad prefix".into());
            }
            cur.truncate(p);
            cur.extend_from_slice(tail.get(..end).unwrap_or(&[]));
            rest = tail.get(end + 1..).unwrap_or(&[]);
            let w = std::str::from_utf8(&cur).map_err(|_| "word list: bad utf-8")?;
            l.push(w);
        }
        Ok(l)
    }

    /// German word forms (LanguageTool's German dictionary, CC BY-SA 4.0; see ATTRIBUTION.md),
    /// loaded on first use. An unreadable list is empty, so German text is then not flagged.
    pub fn german() -> &'static WordList {
        static DE: OnceLock<WordList> = OnceLock::new();
        DE.get_or_init(|| {
            WordList::from_bytes(include_bytes!("../../../assets/spelling/de.words")).unwrap_or_else(|e| {
                log::warn!("{e}");
                WordList::default()
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let l = WordList::from_words(["Haus", "Häuser", "haushalten", "Haus", "Ärger", "a"].map(String::from));
        let back = WordList::from_bytes(&l.to_bytes()).unwrap();
        assert_eq!(back.len(), 5);
        assert!(back.contains("Haus") && back.contains("Häuser") && back.contains("Ärger") && back.contains("a"));
        assert!(!back.contains("haus") && !back.contains("Hau") && !back.contains(""));
        assert_eq!(back.iter().collect::<Vec<_>>(), l.iter().collect::<Vec<_>>());
    }

    #[test]
    fn junk_is_an_error_not_a_panic() {
        for junk in [&b""[..], b"\x00\x01\x02", b"WCWORDS1\n"] {
            let _ = WordList::from_bytes(junk);
        }
        // A prefix longer than the previous word.
        let bad = miniz_oxide::deflate::compress_to_vec(b"WCWORDS1\n\x05abc\n", 6);
        assert!(WordList::from_bytes(&bad).is_err());
        let truncated = miniz_oxide::deflate::compress_to_vec(b"WCWORDS1\n\x00abc", 6);
        assert!(WordList::from_bytes(&truncated).is_err());
    }

    #[test]
    fn the_german_list_loads() {
        let de = WordList::german();
        assert!(de.len() > 400_000, "{}", de.len());
        for w in ["Haus", "Häuser", "gehen", "ging", "schön", "Straße", "mit", "freundlichen", "Grüßen"] {
            assert!(de.contains(w), "{w}");
        }
    }
}
