//! Romanian spelling suggestions, best first.
//!
//! Candidates come in rounds of rising cost, each checked against the dictionary:
//! 1. letter case (`brașov` → `Brașov`);
//! 2. missing or wrong diacritics, from the dictionary's `MAP` groups (`fata` → `fată`/`față`,
//!    `si` → `și`, `cîine` → `câine`), up to four letters changed — Romanian typed without
//!    diacritics is the most common slip, so adding one ranks first, while taking away one the
//!    writer typed ranks after the known confusions;
//! 3. the dictionary's `REP` table of common confusions;
//! 4. a missing space (`deacasă` → `de acasă`) or hyphen (`sa` → `s-a`, `dămi` → `dă-mi`,
//!    `intrun` → `într-un`), cheaper when one half is a short function word or clitic;
//! 5. one typo: a letter left out, added, swapped with the next or mistyped (`TRY` letters);
//! 6. when that finds fewer than asked for: the stems that start like the word (diacritics
//!    folded) and share the most letter pairs with it, whose forms are ranked by an edit distance
//!    in which a diacritic slip costs half (two typos at most).
//!
//! Cedilla letters (`ş`, `ţ`) are read as the comma-below ones first. The input is bounded (40
//! letters), and so is every round, so a call costs ten-odd thousand dictionary lookups at most.

use std::collections::{HashMap, HashSet};

use crate::affix::{AffixDict, Casing, capitalize, casing};

/// Short words often run into the next one (`de acasă`, `la școală`, `în casă`).
const LEADING_WORDS: &[&str] = &[
    "a", "o", "e", "și", "în", "la", "de", "pe", "cu", "din", "pentru", "spre", "prin", "până", "fără", "sub", "după", "despre", "că", "să", "nu",
    "mai", "foarte", "prea", "tot", "un", "una", "cel", "cea", "cei", "cele", "al", "ale", "ai", "lui", "ei", "lor", "ce", "cine", "care", "când",
    "unde", "cum", "dacă", "dar", "iar", "sau", "ori", "nici", "chiar", "doar", "deja", "încă", "aici", "acolo", "azi", "ieri",
];
/// Short words often run into the one before (`acum o`, `ea e`).
const TRAILING_WORDS: &[&str] = &["o", "e", "a", "și", "nu", "de", "la", "că", "să", "mai", "tot", "lui", "lor", "ei", "ce", "cum"];
/// Pieces written before a hyphen (`s-a`, `n-am`, `într-o`, `m-am`, `c-ai`).
const HYPHEN_HEADS: &[&str] = &["s", "n", "m", "l", "i", "c", "v", "ți", "mi", "și", "într", "dintr", "printr", "ne", "le", "vă"];
/// Pieces written after a hyphen (`dă-mi`, `spune-i`, `într-un`, `ia-l`, `mi-a`).
const HYPHEN_TAILS: &[&str] = &[
    "a", "o", "i", "l", "le", "mi", "ți", "și", "ne", "vă", "am", "ai", "au", "ar", "aș", "ați", "un", "unul", "una", "unei", "unui", "o", "ul",
    "lea",
];
const MAX_LETTERS: usize = 40;
const MAX_MAP_CHANGES: usize = 4;
const MAX_MAP_VARIANTS: usize = 4096;
/// Diacritic variants that also try a hyphen (`intrun` → `într-un`).
const HYPHEN_VARIANTS: usize = 64;
/// Stems the scan of round 6 expands.
const SCAN_STEMS: usize = 48;

const COST_CASE: u32 = 0;
const COST_ADD_DIACRITIC: u32 = 4;
const COST_OTHER_MAP: u32 = 7;
const COST_DROP_DIACRITIC: u32 = 11;
const COST_REP: u32 = 10;
const COST_HYPHEN: u32 = 9;
const COST_SPLIT: u32 = 12;
const COST_SPLIT_RARE: u32 = 18;
const COST_SWAP: u32 = 14;
const COST_DROP_OR_ADD: u32 = 15;
const COST_MISTYPE: u32 = 16;
const COST_SCAN: u32 = 20;

struct Collector<'a> {
    d: &'a AffixDict,
    /// The word, lower case.
    input: String,
    tested: HashSet<String>,
    /// Suggestion → (cost, order found).
    found: HashMap<String, (u32, usize)>,
    seq: usize,
}

impl Collector<'_> {
    /// Check a lower-case candidate (a word, or words joined by a space or hyphen) and keep it
    /// if every word is in the dictionary, as written or capitalized (names).
    fn offer(&mut self, cand: &str, cost: u32) {
        if cand.is_empty() || cand == self.input || !self.tested.insert(cand.to_string()) {
            return;
        }
        let mut words = Vec::new();
        let mut cost = cost;
        for part in cand.split(' ') {
            if part.is_empty() {
                return;
            }
            if self.d.contains(part) || (part.contains('-') && part.split('-').all(|p| !p.is_empty() && self.d.contains(p))) {
                words.push(part.to_string());
            } else {
                let c = capitalize(part);
                if !self.d.contains(&c) {
                    return;
                }
                words.push(c);
                // A name ranks after a common word at the same distance.
                cost += 1;
            }
        }
        self.keep(words.join(" "), cost);
    }

    /// Keep a suggestion known to be correct.
    fn keep(&mut self, text: String, cost: u32) {
        self.seq += 1;
        let seq = self.seq;
        let e = self.found.entry(text).or_insert((cost, seq));
        if cost < e.0 {
            *e = (cost, seq);
        }
    }

    fn count_within(&self, cost: u32) -> usize {
        self.found.values().filter(|(c, _)| *c <= cost).count()
    }
}

/// The MAP group of each letter (its other members are its likely confusions).
fn alternatives(c: char, map: &[Vec<char>]) -> Vec<char> {
    let mut v: Vec<char> = map.iter().filter(|g| g.contains(&c)).flat_map(|g| g.iter().copied()).filter(|&x| x != c).collect();
    // Groups overlap (`aăâî`, `iîâ`): `â` meets `î` twice. A repeat would repeat a whole subtree
    // of variants.
    v.sort_unstable();
    v.dedup();
    v
}

/// The cost of changing letter `from` to its MAP relative `to`. Adding a missing diacritic is
/// the commonest slip (Romanian typed without them: `fata`); a diacritic the writer did type is
/// likelier right, so taking one away costs more than a known confusion from the REP table.
fn map_cost(from: char, to: char) -> u32 {
    match (from.is_ascii(), to.is_ascii()) {
        (true, false) => COST_ADD_DIACRITIC,
        (false, true) => COST_DROP_DIACRITIC,
        _ => COST_OTHER_MAP,
    }
}

/// Every way of changing up to [`MAX_MAP_CHANGES`] letters to a MAP relative, with its cost,
/// one change first, then two… so the [`MAX_MAP_VARIANTS`] cap on a long word drops the
/// dearest variants (many changes), never a cheap one-letter fix near its start.
fn map_variants(chars: &[char], alts: &[Vec<char>], out: &mut Vec<(String, u32)>) {
    let mut buf = Vec::with_capacity(chars.len());
    for target in 1..=MAX_MAP_CHANGES {
        if out.len() >= MAX_MAP_VARIANTS {
            break;
        }
        variants_with(chars, alts, 0, target, 0, &mut buf, out);
    }
}

/// The variants of `chars[i..]` with exactly `left` more changes, appended to `out` (bounded by
/// [`MAX_MAP_VARIANTS`]).
fn variants_with(chars: &[char], alts: &[Vec<char>], i: usize, left: usize, cost: u32, buf: &mut Vec<char>, out: &mut Vec<(String, u32)>) {
    if out.len() >= MAX_MAP_VARIANTS || chars.len().saturating_sub(i) < left {
        return;
    }
    let Some(&c) = chars.get(i) else {
        if left == 0 {
            out.push((buf.iter().collect(), cost));
        }
        return;
    };
    if left == 0 {
        // Nothing more to change: the rest as it is.
        let len = buf.len();
        buf.extend_from_slice(chars.get(i..).unwrap_or_default());
        out.push((buf.iter().collect(), cost));
        buf.truncate(len);
        return;
    }
    for &a in alts.get(i).map(Vec::as_slice).unwrap_or_default() {
        buf.push(a);
        variants_with(chars, alts, i + 1, left - 1, cost + map_cost(c, a), buf, out);
        buf.pop();
    }
    buf.push(c);
    variants_with(chars, alts, i + 1, left, cost, buf, out);
    buf.pop();
}

/// A letter's MAP representative (the first of its group), lower case: `ă` → `a`.
fn fold(c: char, map: &[Vec<char>]) -> char {
    let l = c.to_lowercase().next().unwrap_or(c);
    map.iter().find(|g| g.contains(&l)).and_then(|g| g.first().copied()).unwrap_or(l)
}

/// Optimal string alignment distance where a MAP slip (`a`/`ă`, `s`/`ș`) costs 1 and any other
/// edit 2; `limit + 1` once it is certainly above `limit`.
fn weighted_distance(a: &[char], b: &[char], related: &dyn Fn(char, char) -> bool, limit: u32) -> u32 {
    let m = b.len();
    let over = limit + 1;
    let mut prev2 = vec![0u32; m + 1];
    let mut prev: Vec<u32> = (0..=m as u32).map(|j| j * 2).collect();
    let mut cur = vec![0u32; m + 1];
    for (i, &ai) in a.iter().enumerate() {
        if let Some(c) = cur.get_mut(0) {
            *c = (i as u32 + 1) * 2;
        }
        let mut row_min = u32::MAX;
        for (j, &bj) in b.iter().enumerate() {
            let sub = if ai == bj {
                0
            } else if related(ai, bj) {
                1
            } else {
                2
            };
            let at = |v: &[u32], k: usize| v.get(k).copied().unwrap_or(u32::MAX / 4);
            let mut v = (at(&prev, j + 1) + 2).min(at(&cur, j) + 2).min(at(&prev, j) + sub);
            if i > 0 && j > 0 && a.get(i - 1) == Some(&bj) && b.get(j - 1) == Some(&ai) {
                v = v.min(at(&prev2, j - 1) + 2);
            }
            if let Some(c) = cur.get_mut(j + 1) {
                *c = v;
            }
            row_min = row_min.min(v);
        }
        if m > 0 && row_min > limit {
            return over;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev.get(m).copied().unwrap_or(over).min(over)
}

/// Offer `w` with a hyphen at each place where one half is a clitic piece (`sa` → `s-a`,
/// `dămi` → `dă-mi`). `tails` off: only the leading pieces (`într-un` from a diacritic
/// variant; trailing ones there give nonsense such as `căi-ne` for `cîine`).
fn offer_hyphens(col: &mut Collector, w: &str, cost: u32, tails: bool) {
    for (i, _) in w.char_indices().skip(1) {
        let (Some(a), Some(b)) = (w.get(..i), w.get(i..)) else { continue };
        if HYPHEN_HEADS.contains(&a) || (tails && HYPHEN_TAILS.contains(&b) && a.chars().count() >= 2) {
            col.offer(&format!("{a}-{b}"), cost + COST_HYPHEN);
        }
    }
}

/// Spelling suggestions for a Romanian word, best first, at most `max`, in the word's letter case.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(&super::dictionary(), word, max)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    let word = word.trim();
    let n = word.chars().count();
    if n == 0 || n > MAX_LETTERS || max == 0 || d.is_empty() || word.contains(char::is_whitespace) {
        return Vec::new();
    }
    let case = casing(word);
    let input = super::comma_below(&word.to_lowercase());
    let chars: Vec<char> = input.chars().collect();
    let mut col = Collector { d, input: input.clone(), tested: HashSet::new(), found: HashMap::new(), seq: 0 };
    // 1. Letter case.
    if d.contains(&input) && word != input {
        col.keep(input.clone(), COST_CASE);
    } else if d.contains(&capitalize(&input)) && word != capitalize(&input) {
        col.keep(capitalize(&input), COST_CASE);
    }
    // 2. Diacritics and other MAP slips.
    let map = d.map();
    let alts: Vec<Vec<char>> = chars.iter().map(|&c| alternatives(c, map)).collect();
    let mut variants = Vec::new();
    map_variants(&chars, &alts, &mut variants);
    variants.sort_by_key(|v| v.1);
    for (v, cost) in &variants {
        col.offer(v, *cost);
    }
    // 3. Common confusions.
    for (from, to) in d.rep() {
        if from.is_empty() {
            continue;
        }
        for (i, _) in input.match_indices(from.as_str()) {
            if let (Some(a), Some(b)) = (input.get(..i), input.get(i + from.len()..)) {
                col.offer(&format!("{a}{to}{b}"), COST_REP);
            }
        }
    }
    // 4. A missing hyphen (also after a diacritic fix: `intrun` → `într-un`), or a missing space:
    //    cheap after a short leading word or before a short trailing one, dearer between two
    //    longer words, never leaving a stray short half.
    if !input.contains('-') {
        offer_hyphens(&mut col, &input, 0, true);
        for (v, cost) in variants.iter().take(HYPHEN_VARIANTS) {
            offer_hyphens(&mut col, v, *cost, false);
        }
    }
    for (i, _) in input.char_indices().skip(1) {
        let (Some(a), Some(b)) = (input.get(..i), input.get(i..)) else { continue };
        let long = |p: &str| p.chars().count() >= 3;
        let cost = if (LEADING_WORDS.contains(&a) && long(b)) || (long(a) && TRAILING_WORDS.contains(&b)) {
            COST_SPLIT
        } else if long(a) && long(b) {
            COST_SPLIT_RARE
        } else {
            continue;
        };
        col.offer(&format!("{a} {b}"), cost);
    }
    // 5. One typo.
    let mut letters: Vec<char> = d.try_chars().chars().filter(|c| c.is_lowercase()).collect();
    if letters.is_empty() {
        letters = "iaăâșțîertolncusmpdbgfzvhjxkwyq".chars().collect();
    }
    let s = |v: &[char]| v.iter().collect::<String>();
    for i in 0..n.saturating_sub(1) {
        if chars.get(i) != chars.get(i + 1) {
            let mut v = chars.clone();
            v.swap(i, i + 1);
            col.offer(&s(&v), COST_SWAP);
        }
    }
    for i in 0..chars.len() {
        let mut v = chars.clone();
        v.remove(i);
        col.offer(&s(&v), COST_DROP_OR_ADD);
    }
    for i in 0..=chars.len() {
        for &c in &letters {
            let mut v = chars.clone();
            v.insert(i, c);
            col.offer(&s(&v), COST_DROP_OR_ADD);
        }
    }
    for i in 0..chars.len() {
        for &c in &letters {
            if chars.get(i) != Some(&c) {
                let mut v = chars.clone();
                if let Some(x) = v.get_mut(i) {
                    *x = c;
                }
                col.offer(&s(&v), COST_MISTYPE);
            }
        }
    }
    // 6. Further off: scan the stems that start like the word.
    if col.count_within(COST_MISTYPE) < max {
        scan(&mut col, &chars);
    }
    let mut found: Vec<(String, (u32, usize))> = col.found.into_iter().collect();
    // Among equal costs, the change nearest the word's start is the likelier intent.
    let diff_at = |text: &str| chars.iter().zip(text.chars()).position(|(a, b)| *a != b).unwrap_or(n);
    found.sort_by_key(|(text, (cost, seq))| (*cost, diff_at(text), *seq));
    let mut out: Vec<String> = Vec::new();
    for (text, _) in found {
        let t = match case {
            Casing::Initial => capitalize(&text),
            Casing::Upper if n > 1 => text.to_uppercase(),
            _ => text,
        };
        if t != word && !out.contains(&t) {
            out.push(t);
        }
        if out.len() >= max {
            break;
        }
    }
    out
}

/// Round 6: rank the stems starting with the word's first letter (or a MAP relative of it, either
/// case) by shared letter pairs, then keep the forms of the best within two typos.
fn scan(col: &mut Collector, chars: &[char]) {
    let d = col.d;
    let map = d.map();
    let Some(&first) = chars.first() else { return };
    let folded: Vec<char> = chars.iter().map(|&c| fold(c, map)).collect();
    let pairs: HashSet<(char, char)> = std::iter::once(' ').chain(folded.iter().copied()).zip(folded.iter().copied()).collect();
    let mut firsts: Vec<char> = std::iter::once(first).chain(alternatives(first, map)).collect();
    let upper: Vec<char> = firsts.iter().flat_map(|c| c.to_uppercase()).collect();
    firsts.extend(upper);
    firsts.sort_unstable();
    firsts.dedup();
    let n = folded.len();
    let mut best: Vec<(u32, &str)> = Vec::new();
    for f in firsts {
        let prefix = f.to_string();
        for stem in d.stems_with_prefix(&prefix) {
            let sf: Vec<char> = stem.chars().map(|c| fold(c, map)).collect();
            let shared = std::iter::once(' ').chain(sf.iter().copied()).zip(sf.iter().copied()).filter(|p| pairs.contains(p)).count();
            if shared < 2 {
                continue;
            }
            // Shared pairs, less the stem's letters the word doesn't explain, less a length gap
            // beyond what an ending explains.
            let extra = sf.len().saturating_sub(shared);
            let gap = sf.len().abs_diff(n).saturating_sub(4);
            let score = (shared * 4).saturating_sub(extra * 3 + gap * 2) as u32;
            if score == 0 {
                continue;
            }
            if best.len() < SCAN_STEMS {
                best.push((score, stem));
            } else if let Some(min) = best.iter_mut().min_by_key(|b| b.0)
                && score > min.0
            {
                *min = (score, stem);
            }
        }
    }
    best.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    let related = |a: char, b: char| fold(a, map) == fold(b, map);
    let limit = 4;
    for (_, stem) in best {
        for form in d.forms(stem) {
            let lf: Vec<char> = form.to_lowercase().chars().collect();
            if lf.len().abs_diff(chars.len()) > 2 {
                continue;
            }
            let dist = weighted_distance(&lf, chars, &related, limit);
            if dist <= limit && lf.as_slice() != chars {
                col.keep(form, COST_SCAN + dist * 3);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(word: &str, want: &str) {
        let s = suggest(word, 8);
        assert!(s.iter().any(|x| x == want), "{word} → {s:?}, want {want}");
    }

    #[test]
    fn romanian_misspellings_get_the_right_suggestion() {
        crate::ro::test_dict::install();
        has("si", "și");
        has("tara", "țară");
        has("stiinta", "știință");
        has("scoala", "școală");
        has("cîine", "câine");
        has("mîine", "mâine");
        has("lcurez", "lucrez");
        has("frumso", "frumos");
        has("deacasă", "de acasă");
        has("sa", "s-a");
        has("dămi", "dă-mi");
        has("intrun", "într-un");
        has("brasov", "Brașov");
        has("Tara", "Țară");
        has("ştiinta", "știință");
    }

    #[test]
    fn the_likeliest_fix_comes_first() {
        crate::ro::test_dict::install();
        assert_eq!(suggest("si", 3).first().map(String::as_str), Some("și"));
        assert_eq!(suggest("cîine", 3).first().map(String::as_str), Some("câine"));
        assert_eq!(suggest("stiinta", 3).first().map(String::as_str), Some("știință"));
    }

    #[test]
    fn edge_cases_stay_empty_or_bounded() {
        crate::ro::test_dict::install();
        assert!(suggest("", 5).is_empty());
        assert!(suggest("bine", 0).is_empty());
        assert!(suggest(&"a".repeat(41), 5).is_empty());
        assert!(suggest("două cuvinte", 5).is_empty());
        assert!(suggest_in(&crate::affix::AffixDict::default(), "tara", 5).is_empty());
        assert!(suggest("casă", 5).len() <= 5);
        assert!(!suggest("casă", 5).contains(&"casă".to_string()));
        let _ = suggest("-", 5);
        let _ = suggest("a-", 5);
    }

    #[test]
    fn long_words_keep_their_cheap_diacritic_fixes() {
        // Many `a`/`i`/`s`/`t` letters: far more than MAX_MAP_VARIANTS variants with four changes.
        // The one-change fixes come first, so `ș` at the start survives the cap.
        let map = vec![vec!['a', 'ă', 'â', 'î'], vec!['i', 'î', 'â'], vec!['s', 'ș'], vec!['t', 'ț']];
        let chars: Vec<char> = "stiintificitatea".chars().collect();
        let alts: Vec<Vec<char>> = chars.iter().map(|&c| alternatives(c, &map)).collect();
        // `â` is in both `aăâî` and `iîâ`: `î` is listed once.
        assert_eq!(alternatives('â', &map), ['a', 'i', 'î', 'ă']);
        let mut out = Vec::new();
        map_variants(&chars, &alts, &mut out);
        assert_eq!(out.len(), MAX_MAP_VARIANTS);
        assert!(out.iter().any(|(v, c)| v == "știintificitatea" && *c == COST_ADD_DIACRITIC), "the one-letter fix is kept");
        let pos = |k: usize| out.iter().position(|(v, _)| v.chars().zip(&chars).filter(|(a, b)| a != *b).count() == k);
        assert!(pos(1) < pos(2) && pos(2).is_some(), "fewer changes come first");
    }

    #[test]
    fn distance_weighs_diacritics_half() {
        let map = vec![vec!['a', 'ă', 'â', 'î'], vec!['s', 'ș'], vec!['t', 'ț']];
        let related = |a: char, b: char| fold(a, &map) == fold(b, &map);
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(weighted_distance(&c("fata"), &c("fată"), &related, 9), 1);
        assert_eq!(weighted_distance(&c("tara"), &c("țară"), &related, 9), 2);
        assert_eq!(weighted_distance(&c("bine"), &c("bine"), &related, 9), 0);
        assert_eq!(weighted_distance(&c("bine"), &c("rău"), &related, 2), 3);
        assert_eq!(weighted_distance(&c(""), &c("ab"), &related, 9), 4);
    }

    #[test]
    fn suggestions_are_fast_enough() {
        crate::ro::test_dict::install();
        // Warm the dictionary, then time a worst case (a long word nothing near): the scan round.
        let _ = suggest("bine", 1);
        let t = std::time::Instant::now();
        for w in ["xqzvweromanescuvant", "bucurestiului", "stiintificitatea"] {
            let _ = suggest(w, 6);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("3 suggestion calls: {ms:.0} ms");
        assert!(ms < 20_000.0, "{ms} ms");
    }
}
