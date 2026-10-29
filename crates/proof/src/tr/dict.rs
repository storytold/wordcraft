//! Turkish Hunspell dictionary parser with `FLAG num` support.
//!
//! Turkish uses `FLAG num`, storing thousands of numerical suffix flags (0..6465).
//! This engine parses `/usr/share/hunspell/tr_TR.aff` and `.dic` safely without panics.

use std::collections::HashMap;

/// A suffix rule for Turkish.
#[derive(Clone, Debug)]
pub struct TrSuffixRule {
    pub strip: Box<str>,
    pub add: Box<str>,
    pub cond: Box<str>,
}

/// Dedicated Turkish Hunspell dictionary engine.
#[derive(Clone, Debug, Default)]
pub struct TrDict {
    /// Stem -> list of numerical flags it carries.
    stems: HashMap<String, Box<[u16]>>,
    /// Flag number -> list of suffix rules.
    rules: HashMap<u16, Vec<TrSuffixRule>>,
    /// Suffix 'add' string -> list of (flag, rule_index).
    suffix_index: HashMap<String, Vec<(u16, u16)>>,
    /// Longest suffix 'add' string in characters.
    max_suffix_len: usize,
}

impl TrDict {
    /// Returns true if empty (no dictionary loaded).
    pub fn is_empty(&self) -> bool {
        self.stems.is_empty()
    }

    /// Load from `.aff` and `.dic` contents.
    pub fn from_hunspell(aff: &str, dic: &str) -> Result<Self, String> {
        let mut rules: HashMap<u16, Vec<TrSuffixRule>> = HashMap::new();
        let mut max_suffix_len = 0;

        for line in aff.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // SFX <flag_num> <strip> <add> [<cond>]
            if parts.len() >= 4 && parts[0] == "SFX" {
                let Ok(flag) = parts[1].parse::<u16>() else { continue };
                // Skip header line: `SFX <flag> Y/N <count>`
                if parts.len() == 4 && (parts[2] == "Y" || parts[2] == "N") && parts[3].parse::<usize>().is_ok() {
                    continue;
                }
                let strip = if parts[2] == "0" { "" } else { parts[2] };
                let add = if parts[3] == "0" { "" } else { parts[3] };
                let cond = if parts.len() > 4 { parts[4] } else { "." };

                max_suffix_len = max_suffix_len.max(add.chars().count());

                rules.entry(flag).or_default().push(TrSuffixRule { strip: strip.into(), add: add.into(), cond: cond.into() });
            }
        }

        let mut suffix_index: HashMap<String, Vec<(u16, u16)>> = HashMap::new();
        for (&flag, r_list) in &rules {
            for (idx, r) in r_list.iter().enumerate() {
                if let Ok(idx_u16) = u16::try_from(idx) {
                    suffix_index.entry(r.add.to_string()).or_default().push((flag, idx_u16));
                }
            }
        }

        let mut stems: HashMap<String, Box<[u16]>> = HashMap::new();
        for (i, line) in dic.lines().enumerate() {
            if i == 0 && line.trim().parse::<usize>().is_ok() {
                continue;
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((word, flags_part)) = line.split_once('/') {
                let flags: Vec<u16> = flags_part.split(',').filter_map(|f| f.parse::<u16>().ok()).collect();
                stems.insert(word.to_string(), flags.into_boxed_slice());
            } else {
                stems.insert(line.to_string(), Box::new([]));
            }
        }

        Ok(TrDict { stems, rules, suffix_index, max_suffix_len })
    }

    /// Check if word is correct (stem or affixed).
    pub fn contains(&self, word: &str) -> bool {
        // Longer input is never a dictionary word, and isn't scanned (#3625).
        if self.is_empty() || word.is_empty() || word.len() > crate::affix::MAX_WORD_BYTES {
            return false;
        }

        // Direct stem hit
        if self.stems.contains_key(word) {
            return true;
        }

        // Try stripping suffixes matching the tail
        let word_len = word.chars().count();
        let char_indices: Vec<(usize, char)> = word.char_indices().collect();

        // Check each possible suffix length up to max_suffix_len
        for cut_chars in 0..=self.max_suffix_len.min(word_len) {
            let byte_idx = if cut_chars == 0 { word.len() } else { char_indices[word_len - cut_chars].0 };

            let add = &word[byte_idx..];
            let base = &word[..byte_idx];

            if let Some(matches) = self.suffix_index.get(add) {
                for &(flag, rule_idx) in matches {
                    let Some(rule_list) = self.rules.get(&flag) else { continue };
                    let Some(rule) = rule_list.get(rule_idx as usize) else { continue };

                    // Check condition on root
                    let mut candidate = String::with_capacity(base.len() + rule.strip.len());
                    candidate.push_str(base);
                    candidate.push_str(&rule.strip);

                    if self.stems.get(&candidate).is_some_and(|f_list| f_list.contains(&flag)) {
                        return true;
                    }
                }
            }
        }

        false
    }
}
