//! Offline Bokmål and Nynorsk spelling against Norsk ordbank's current full forms (CC BY 4.0).
//! The data and reproducible extraction script live in `assets/spelling/`.

use std::sync::OnceLock;

pub(crate) struct Dictionary {
    words: Vec<String>,
    by_len: Vec<Vec<usize>>,
}

impl Dictionary {
    fn decode(bytes: &[u8], header: &str) -> Result<Self, String> {
        let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(bytes, 32 * 1024 * 1024).map_err(|e| format!("Norwegian dictionary: {e:?}"))?;
        let text = std::str::from_utf8(&raw).map_err(|e| e.to_string())?;
        let body = text.strip_prefix(header).ok_or("Norwegian dictionary: invalid header")?;
        let words: Vec<String> = body.lines().map(str::to_string).collect();
        if words.is_empty() || words.windows(2).any(|w| w.first() >= w.get(1)) {
            return Err("Norwegian dictionary: empty or unsorted word list".into());
        }
        let mut by_len = vec![Vec::new(); 121];
        for (i, word) in words.iter().enumerate() {
            let n = word.chars().count();
            let Some(bucket) = by_len.get_mut(n) else { return Err("Norwegian dictionary: word too long".into()) };
            bucket.push(i);
        }
        Ok(Self { words, by_len })
    }

    pub(crate) fn get(language: super::Language) -> Option<&'static Self> {
        static NB: OnceLock<Option<Dictionary>> = OnceLock::new();
        static NN: OnceLock<Option<Dictionary>> = OnceLock::new();
        let (cache, bytes, header): (_, &[u8], _) = match language {
            super::Language::Bokmal => (&NB, include_bytes!("../../../assets/spelling/nb-NO.dic"), "WCNB1\n"),
            super::Language::Nynorsk => (&NN, include_bytes!("../../../assets/spelling/nn-NO.dic"), "WCNN1\n"),
            _ => return None,
        };
        cache
            .get_or_init(|| match Self::decode(bytes, header) {
                Ok(dict) => Some(dict),
                Err(error) => {
                    log::warn!("{error}; {language:?} spelling disabled");
                    None
                }
            })
            .as_ref()
    }

    pub(crate) fn contains(&self, word: &str) -> bool {
        self.words.binary_search_by(|w| w.as_str().cmp(word)).is_ok()
    }

    pub(crate) fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        let target: Vec<char> = word.chars().collect();
        let n = target.len();
        if n == 0 || n > 40 || max == 0 {
            return Vec::new();
        }
        let mut scored = Vec::new();
        for size in n.saturating_sub(2)..=n + 2 {
            for index in self.by_len.get(size).into_iter().flatten() {
                let Some(candidate) = self.words.get(*index) else { continue };
                let distance = super::damerau(&target, candidate);
                if distance <= 2 {
                    let first = usize::from(candidate.chars().next() != target.first().copied());
                    scored.push((distance, first, n.abs_diff(size), candidate));
                }
            }
        }
        scored.sort_unstable();
        scored.into_iter().take(max.min(32)).map(|(_, _, _, w)| w.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_dictionary_is_complete_and_valid() {
        let d = Dictionary::get(super::super::Language::Bokmal).expect("valid bundled dictionary");
        assert!(d.words.len() > 500_000, "{}", d.words.len());
        for word in ["norsk", "bokmål", "stavekontroll", "bøkene", "skriver", "skrev", "skrevet", "ærlig", "øvelse", "åpen"] {
            assert!(d.contains(word), "{word}");
        }
    }

    #[test]
    fn nynorsk_dictionary_is_distinct_and_valid() {
        let d = Dictionary::get(super::super::Language::Nynorsk).expect("valid bundled Nynorsk dictionary");
        assert_eq!(d.words.len(), 409_690);
        for word in ["eg", "ikkje", "nynorsk", "stavekontroll", "bøkene", "opne", "skreiv", "skrive", "ærleg", "øving"] {
            assert!(d.contains(word), "{word}");
        }
        for word in ["jeg", "skriver", "åpen"] {
            assert!(!d.contains(word), "{word}");
        }
        let nb = include_bytes!("../../../assets/spelling/nb-NO.dic");
        assert!(Dictionary::decode(nb, "WCNN1\n").is_err(), "must not load Bokmål as Nynorsk");
    }

    #[test]
    fn corrupt_dictionary_returns_an_error() {
        assert!(Dictionary::decode(b"invalid deflate", "WCNB1\n").is_err());
        let bad = miniz_oxide::deflate::compress_to_vec(b"WCNB1\nzebra\nape\n", 6);
        assert!(Dictionary::decode(&bad, "WCNB1\n").is_err());
    }
}
