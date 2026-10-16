//! Hungarian hyphenation.
//!
//! 1. **The installed patterns** (`hyph_hu_HU.dic`, the Hungarian patterns LibreOffice and the
//!    `hyphen` library use; not bundled, see the [module docs](super) for the licence). They know
//!    compound boundaries (`meg-int`, `kis-asszony`), which syllable rules can't. The file is in
//!    the `hyphen` library's format: a character set line, optional directives
//!    (`LEFTHYPHENMIN`…), then Liang patterns. Its non-standard patterns change the spelling at the
//!    break (`asszony` → `asz-szony`); WordCraft only places hyphens between letters, so it
//!    doesn't break there at all and never writes a wrong spelling.
//! 2. **The Hungarian syllable rules** otherwise: a single consonant goes to the next syllable
//!    (`ba-ba`), of several only the last does (`temp-lom`), adjacent vowels split (`fi-ú`). The
//!    digraphs (`cs`, `dz`, `dzs`, `gy`, `ly`, `ny`, `sz`, `ty`, `zs`) are one consonant, and their
//!    doubled forms (`ssz`, `ggy`…) are never split, for the same reason as above.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::patterns::Patterns;

/// The installed patterns: `None` until first use, then the loaded ones (empty when none).
static PATTERNS: RwLock<Option<Arc<Patterns>>> = RwLock::new(None);

/// File names of the Hungarian `hyphen` patterns.
const NAMES: &[&str] = &["hyph_hu_HU.dic", "hyph_hu.dic", "hyph-hu.dic"];

/// Largest pattern file read (the Hungarian one is about 2 MB).
const MAX_FILE: u64 = 16 << 20;

/// Directories that may hold `hyphen` pattern files: the dictionary folders, then the system's
/// `hyphen` folders.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = crate::hunspell::search_dirs(Some(&["dict-hu".to_string()]));
    if !cfg!(target_arch = "wasm32") && !cfg!(windows) {
        for d in ["/usr/share/hyphen", "/usr/local/share/hyphen", "/opt/homebrew/share/hyphen"] {
            dirs.push(PathBuf::from(d));
        }
    }
    dirs
}

/// The pattern file found now, if any.
pub fn installed_path() -> Option<PathBuf> {
    search_dirs().into_iter().find_map(|d| NAMES.iter().map(|n| d.join(n)).find(|p| p.is_file()))
}

/// The installed Hungarian patterns (read on first use; empty when there are none).
pub fn patterns() -> Arc<Patterns> {
    if let Some(p) = PATTERNS.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return p.clone();
    }
    let loaded = Arc::new(load_installed());
    let mut slot = PATTERNS.write().unwrap_or_else(|e| e.into_inner());
    slot.get_or_insert(loaded).clone()
}

/// Forget the loaded patterns, so the next hyphenation looks for installed ones again.
pub fn reload_patterns() {
    *PATTERNS.write().unwrap_or_else(|e| e.into_inner()) = None;
}

fn load_installed() -> Patterns {
    let Some(path) = installed_path() else {
        log::info!("Hungarian hyphenation: no hyph_hu_HU.dic installed, using the syllable rules");
        return Patterns::default();
    };
    let read = || -> Result<String, String> {
        let len = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
        if len > MAX_FILE {
            return Err(format!("too large ({len} bytes)"));
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        String::from_utf8(bytes).map_err(|_| "not UTF-8".to_string())
    };
    match read() {
        Ok(text) => parse(&text),
        Err(e) => {
            log::warn!("Hungarian hyphenation: {}: {e}", path.display());
            Patterns::default()
        }
    }
}

/// Read a pattern file in the `hyphen` library's format (see the module docs). Patterns that
/// can't be represented (over 20 letters) are skipped; a non-standard pattern keeps its letters
/// and inhibits a break where it would have changed the spelling.
pub fn parse(text: &str) -> Patterns {
    let mut p = Patterns::default();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        // The first line names the character set.
        if n == 0 || line.is_empty() || line.starts_with('%') || line.starts_with('#') || line.starts_with(|c: char| c.is_ascii_uppercase()) {
            continue;
        }
        let pattern = line.split_whitespace().next().unwrap_or("");
        match pattern.split_once('/') {
            // `as5szon2y/sz=,2,1`: an odd value here would break with a spelling change, so it
            // becomes the even value below it, which keeps other patterns from breaking there.
            Some((letters, _)) => {
                let even: String =
                    letters.chars().map(|c| c.to_digit(10).filter(|d| d % 2 == 1).and_then(|d| char::from_digit(d - 1, 10)).unwrap_or(c)).collect();
                p.insert(&even);
            }
            None => {
                p.insert(pattern);
            }
        }
    }
    p
}

/// Hungarian vowels.
fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'á' | 'e' | 'é' | 'i' | 'í' | 'o' | 'ó' | 'ö' | 'ő' | 'u' | 'ú' | 'ü' | 'ű')
}

/// Is `c` a letter the syllable rules know (the Hungarian alphabet, with `q`, `w`, `x`, `y`)?
pub fn is_letter(c: char) -> bool {
    c.is_ascii_lowercase() || is_vowel(c)
}

/// The two-letter consonants.
const DIGRAPHS: &[&str] = &["cs", "dz", "gy", "ly", "ny", "sz", "ty", "zs"];

/// Consonant units of a word: `(start, len, splittable)` in chars. A unit is a letter, a digraph,
/// `dzs`, or a doubled digraph (`ssz`, `ggy`, `ddzs`), which may not be split.
fn units(core: &[char]) -> Vec<(usize, usize, bool)> {
    let mut out = Vec::new();
    let mut i = 0;
    let at = |k: usize, s: &str| s.chars().enumerate().all(|(j, c)| core.get(k + j) == Some(&c));
    while i < core.len() {
        let c = core.get(i).copied().unwrap_or(' ');
        let len = if is_vowel(c) {
            1
        } else if at(i, "ddzs") {
            4
        } else if at(i, "dzs")
            || DIGRAPHS.iter().any(|d| {
                // A doubled digraph: its first letter, then the digraph (`ssz`, `ggy`).
                let mut ch = d.chars();
                ch.next() == Some(c) && at(i + 1, d)
            })
        {
            3
        } else if DIGRAPHS.iter().any(|d| at(i, d)) {
            2
        } else {
            1
        };
        out.push((i, len, len < 3 || at(i, "dzs")));
        i += len;
    }
    out
}

/// Break points by the Hungarian syllable rules (char indices; a hyphen goes before that char).
pub fn rule_points(core: &[char]) -> Vec<usize> {
    let u = units(core);
    let vowel = |k: usize| u.get(k).is_some_and(|&(s, _, _)| core.get(s).copied().is_some_and(is_vowel));
    let mut out = Vec::new();
    let mut last_vowel: Option<usize> = None;
    for k in 0..u.len() {
        if !vowel(k) {
            continue;
        }
        if let Some(prev) = last_vowel {
            // The units between two vowels; the break goes before the last consonant unit, or
            // between the vowels when there is none.
            let between = k - prev - 1;
            let unit = if between == 0 { k } else { k - 1 };
            if let Some(&(start, _, splittable)) = u.get(unit)
                && splittable
                && start > 0
            {
                out.push(start);
            }
        }
        last_vowel = Some(k);
    }
    out
}

/// Break points of a lowercase word: the installed patterns when they cover its letters, else
/// the syllable rules. `None` when neither applies (letters from another alphabet).
pub fn points(core: &[char]) -> Option<(Vec<usize>, bool)> {
    let p = patterns();
    if !p.is_empty()
        && let Some(points) = p.points(core)
    {
        return Some((points, true));
    }
    core.iter().all(|&c| is_letter(c)).then(|| (rule_points(core), false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hyph(word: &str, points: &[usize]) -> String {
        let mut s = String::new();
        for (i, c) in word.chars().enumerate() {
            if points.contains(&i) {
                s.push('-');
            }
            s.push(c);
        }
        s
    }

    fn rules(word: &str) -> String {
        let chars: Vec<char> = word.chars().collect();
        hyph(word, &rule_points(&chars))
    }

    #[test]
    fn syllable_rules() {
        assert_eq!(rules("baba"), "ba-ba");
        assert_eq!(rules("templom"), "temp-lom");
        assert_eq!(rules("fiú"), "fi-ú");
        assert_eq!(rules("kocsi"), "ko-csi");
        assert_eq!(rules("hegyes"), "he-gyes");
        assert_eq!(rules("asztal"), "asz-tal");
        assert_eq!(rules("bodza"), "bo-dza");
        assert_eq!(rules("bridzsel"), "bri-dzsel");
        assert_eq!(rules("kalapács"), "ka-la-pács");
        assert_eq!(rules("kenyér"), "ke-nyér");
        // Doubled digraphs would change their spelling at a break: no break there.
        assert_eq!(rules("asszony"), "asszony");
        assert_eq!(rules("hosszú"), "hosszú");
        assert_eq!(rules("eggyel"), "eggyel");
        assert_eq!(rules("a"), "a");
        assert_eq!(rules(""), "");
    }

    #[test]
    fn hyphen_files_are_read() {
        // Written for this test in the `hyphen` format: a character set, a directive, patterns
        // with boundaries, a non-standard pattern and a comment.
        let text = "UTF-8\nLEFTHYPHENMIN 2\n% comment\n1ba\n1ma\n.al1ma\nas5szo/sz=,2,1\nalmabanalmabanalmabanx1y\n";
        let p = parse(text);
        let pts = |w: &str| p.points(&w.chars().collect::<Vec<_>>()).unwrap_or_default();
        assert_eq!(hyph("baba", &pts("baba")), "ba-ba");
        assert_eq!(hyph("alma", &pts("alma")), "al-ma");
        assert_eq!(hyph("asszo", &pts("asszo")), "asszo", "the non-standard break is dropped");
        assert!(p.len() >= 4, "the 25-letter pattern is skipped, the rest kept: {}", p.len());
    }
}
