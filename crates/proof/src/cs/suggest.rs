//! Czech suggestions: bounded MAP variants, REP replacements, KEY neighbours, TRY edits
//! and a stem scan. All linguistic tables are read from the installed dictionary, so Indonesian
//! ([`crate::id`]) uses the same ranking with its own dictionary.

use std::collections::{HashMap, HashSet};

use crate::affix::{AffixDict, Casing, capitalize, casing};

const MAX_LETTERS: usize = 40;
const MAX_MAP_CHANGES: usize = 4;
const MAX_MAP_VARIANTS: usize = 4096;
/// Stems the scan of round 6 expands.
const SCAN_STEMS: usize = 48;

const COST_CASE: u32 = 0;
const COST_ADD_DIACRITIC: u32 = 4;
const COST_OTHER_MAP: u32 = 7;
const COST_DROP_DIACRITIC: u32 = 11;
const COST_REP: u32 = 10;
const COST_SPLIT: u32 = 12;
const COST_SPLIT_RARE: u32 = 17;
const COST_KEY: u32 = 13;
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
    /// Check a lower-case candidate (a word, or two separated by a space) and keep it if every
    /// word is in the dictionary, as written or capitalized (names).
    fn offer(&mut self, cand: &str, cost: u32) {
        if cand.is_empty() || cand == self.input || !self.tested.insert(cand.to_string()) {
            return;
        }
        let mut words = Vec::new();
        let mut cost = cost;
        for part in cand.split(' ') {
            if self.d.contains_suggestible(part) {
                words.push(part.to_string());
            } else {
                let c = capitalize(part);
                if !self.d.contains_suggestible(&c) {
                    return;
                }
                words.push(c);
                // A name ranks after a common word at the same distance (a capitalized name).
                cost += 1;
            }
        }
        self.keep(words.join(" "), cost);
    }

    /// Keep a suggestion known to be correct.
    fn keep(&mut self, text: String, cost: u32) {
        if !text.split(' ').all(|part| self.d.can_suggest(part)) {
            return;
        }
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
    map.iter().filter(|g| g.contains(&c)).flat_map(|g| g.iter().copied()).filter(|&x| x != c).collect()
}

/// The cost of changing letter `from` to its MAP relative `to`. Adding a missing accent is the
/// commonest slip (typing without diacritics); an accent the writer did type
/// is likelier right, so taking one away costs more than a known confusion from the REP table.
fn map_cost(from: char, to: char) -> u32 {
    match (from.is_ascii(), to.is_ascii()) {
        (true, false) => COST_ADD_DIACRITIC,
        (false, true) => COST_DROP_DIACRITIC,
        _ => COST_OTHER_MAP,
    }
}

/// Every way of changing up to [`MAX_MAP_CHANGES`] letters to a MAP relative, with its cost.
fn map_variants(chars: &[char], alts: &[Vec<char>], i: usize, changes: usize, cost: u32, buf: &mut Vec<char>, out: &mut Vec<(String, u32)>) {
    if out.len() >= MAX_MAP_VARIANTS {
        return;
    }
    let Some(&c) = chars.get(i) else {
        if changes > 0 {
            out.push((buf.iter().collect(), cost));
        }
        return;
    };
    buf.push(c);
    map_variants(chars, alts, i + 1, changes, cost, buf, out);
    buf.pop();
    if changes < MAX_MAP_CHANGES {
        for &a in alts.get(i).map(Vec::as_slice).unwrap_or_default() {
            buf.push(a);
            map_variants(chars, alts, i + 1, changes + 1, cost + map_cost(c, a), buf, out);
            buf.pop();
        }
    }
}

/// A letter's MAP representative (the first of its group), lower case: as defined by the installed table.
fn fold(c: char, map: &[Vec<char>]) -> char {
    let l = c.to_lowercase().next().unwrap_or(c);
    map.iter().find(|g| g.contains(&l)).and_then(|g| g.first().copied()).unwrap_or(l)
}

/// Optimal string alignment distance where a MAP slip (dictionary-defined relatives) costs 1 and any other
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

/// Spelling suggestions for a Czech word, best first, at most `max`, in the word's letter case.
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
    let input = word.to_lowercase();
    let chars: Vec<char> = input.chars().collect();
    let mut col = Collector { d, input: input.clone(), tested: HashSet::new(), found: HashMap::new(), seq: 0 };
    // 1. Letter case.
    if d.contains(&input) && word != input {
        col.keep(input.clone(), COST_CASE);
    } else if d.contains(&capitalize(&input)) && word != capitalize(&input) {
        col.keep(capitalize(&input), COST_CASE);
    }
    // 2. Accents and other MAP slips.
    let map = d.map();
    let alts: Vec<Vec<char>> = chars.iter().map(|&c| alternatives(c, map)).collect();
    let mut variants = Vec::new();
    map_variants(&chars, &alts, 0, 0, 0, &mut Vec::with_capacity(n), &mut variants);
    variants.sort_by_key(|v| v.1);
    for (v, cost) in &variants {
        col.offer(v, *cost);
    }
    // 3. Common confusions.
    for (from, to) in d.rep() {
        // `^` and `$` anchor an entry to the word's start or end (`REP ^nml$ namelĳk`).
        let (start, from) = from.strip_prefix('^').map_or((false, from.as_str()), |f| (true, f));
        let (end, from) = from.strip_suffix('$').map_or((false, from), |f| (true, f));
        if from.is_empty() {
            continue;
        }
        for (i, _) in input.match_indices(from) {
            if (start && i != 0) || (end && i + from.len() != input.len()) {
                continue;
            }
            if let (Some(a), Some(b)) = (input.get(..i), input.get(i + from.len()..)) {
                col.offer(&format!("{a}{to}{b}"), COST_REP);
            }
        }
    }
    // 4. A missing space; both halves must be dictionary words.
    for (i, _) in input.char_indices().skip(1) {
        let (Some(a), Some(b)) = (input.get(..i), input.get(i..)) else { continue };
        let long = |part: &str| part.chars().count() >= 3;
        let cost = if long(a) && long(b) { COST_SPLIT } else { COST_SPLIT_RARE };
        col.offer(&format!("{a} {b}"), cost);
    }
    // 5. Keyboard neighbours from the installed dictionary's KEY rows.
    for row in d.key().split('|').take(32) {
        let keys: Vec<char> = row.chars().take(128).collect();
        for pair in keys.windows(2) {
            let [a, b] = pair else { continue };
            for (i, &c) in chars.iter().enumerate() {
                let replacement = if c == *a {
                    Some(*b)
                } else if c == *b {
                    Some(*a)
                } else {
                    None
                };
                if let Some(replacement) = replacement {
                    let mut v = chars.clone();
                    if let Some(x) = v.get_mut(i) {
                        *x = replacement;
                    }
                    col.offer(&v.iter().collect::<String>(), COST_KEY);
                }
            }
        }
    }
    // One-letter edits use only the runtime TRY alphabet, with bounded work.
    let mut letters: Vec<char> = d.try_chars().chars().filter(|c| c.is_lowercase()).take(128).collect();
    letters.sort_unstable();
    letters.dedup();
    let s = |v: &[char]| v.iter().collect::<String>();
    for i in 0..n.saturating_sub(1) {
        if chars.get(i) != chars.get(i + 1) {
            let mut v = chars.clone();
            v.swap(i, i + 1);
            col.offer(&s(&v), COST_SWAP);
        }
    }
    for i in 0..n {
        let mut v = chars.clone();
        v.remove(i);
        col.offer(&s(&v), COST_DROP_OR_ADD);
    }
    for i in 0..=n {
        for &c in &letters {
            let mut v = chars.clone();
            v.insert(i, c);
            col.offer(&s(&v), COST_DROP_OR_ADD);
        }
    }
    for i in 0..n {
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
    // Among equal costs, the change nearest the word's start is the likelier intent
    // (the first changed letter breaks ties).
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

    #[test]
    fn an_extra_trailing_letter_is_dropped_before_a_short_word_split() {
        let d = AffixDict::from_hunspell("", "2\npočítač\nz\n", &[]).unwrap();
        let suggestions = suggest_in(&d, "počítačz", 8);
        assert_eq!(suggestions.first().map(String::as_str), Some("počítač"), "{suggestions:?}");
        assert!(suggestions.contains(&"počítač z".into()), "{suggestions:?}");
    }

    #[test]
    fn suggestions_are_fast_enough() {
        crate::cs::test_dict::install();
        let _ = suggest("město", 1);
        let t = std::time::Instant::now();
        for word in ["zqčeskévelmidlouhéslovo", "pocitacz", "nejrychllejší"] {
            let _ = suggest(word, 6);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("3 Czech suggestion calls: {ms:.0} ms");
        assert!(ms < 20_000.0, "{ms} ms");
    }

    #[test]
    fn nosuggest_stems_and_forms_are_never_offered() {
        crate::affix::tests::assert_nosuggest_is_honoured(suggest_in);
    }

    #[test]
    fn czech_suggestions_use_runtime_tables_and_case() {
        crate::cs::test_dict::install();
        for (word, want) in [
            ("mesto", "město"),
            ("mňe", "mě"),
            ("xůž", "nůž"),
            ("škloa", "škola"),
            ("pocitac", "počítač"),
            ("Mesto", "Město"),
            ("praha", "Praha"),
            ("toškola", "to škola"),
        ] {
            assert!(suggest(word, 8).contains(&want.into()), "{word}: {:?}", suggest(word, 8));
        }
        // Each table changes the answer without a hard-coded Czech alphabet or confusion list.
        for (table, wrong, right) in
            [("MAP 1\nMAP cč\n", "caj", "čaj"), ("REP 1\nREP xy město\n", "xy", "město"), ("TRY š\n", "kola", "škola"), ("KEY xn\n", "xůž", "nůž")]
        {
            let dic = format!("1\n{right}\n");
            let d = AffixDict::from_hunspell(table, &dic, &[]).unwrap();
            assert!(suggest_in(&d, wrong, 4).contains(&right.into()), "{table}");
            let without = AffixDict::from_hunspell("", &dic, &[]).unwrap();
            assert!(!suggest_in(&without, wrong, 4).contains(&right.into()), "must depend on {table}");
        }
    }

    #[test]
    fn suggestions_stay_bounded_and_skip_forbidden_words() {
        crate::cs::test_dict::install();
        for w in ["", "dvě slova", &"ě".repeat(41)] {
            assert!(suggest(w, 4).is_empty());
        }
        assert!(suggest("mesto", 0).is_empty());
        assert!(suggest_in(&AffixDict::default(), "mesto", 4).is_empty());
        assert!(suggest("město", 4).len() <= 4);
        assert!(!suggest("město", 4).contains(&"město".into()));
        assert!(!suggest("zqzakázane", 8).contains(&"zqzakázané".into()));
    }
}
