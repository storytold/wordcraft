//! Spelling suggestions from an affix dictionary, one engine for every language that reads one
//! (Czech, Croatian, Indonesian, Italian, Polish, Portuguese, Russian, Spanish). Candidates come
//! in rounds of rising cost, each checked against the dictionary:
//! 1. letter case (`warszawa` → `Warszawa`);
//! 2. missing or wrong diacritics and other MAP slips, a few letters changed (`zolw` → `żółw`):
//!    adding a missing accent is the commonest slip and ranks first, while taking away one the
//!    writer typed ranks after the known confusions;
//! 3. the REP table of common confusions (`żeka` → `rzeka`);
//! 4. a missing space (`napewno` → `na pewno`);
//! 5. keyboard neighbours from the KEY rows, then one typo: a letter left out, added, swapped with
//!    the next or mistyped (TRY letters);
//! 6. when that finds fewer than asked for: the stems that start like the word (MAP relatives
//!    folded) and share the most letter pairs with it, whose forms are ranked by an edit distance in
//!    which a MAP slip costs half (two typos at most).
//!
//! The tables come from the dictionary; a language's [`Tuning`] adds the ones its dictionaries may
//! lack and the slips no table can say. The input is bounded ([`MAX_LETTERS`]), and so is every
//! round, so a call costs a few thousand dictionary lookups at most.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use crate::affix::{AffixDict, Casing, capitalize, casing};

/// Longest word suggestions are looked for, in letters.
pub(crate) const MAX_LETTERS: usize = 40;
const MAX_MAP_VARIANTS: usize = 4096;
/// Stems the scan of round 6 expands.
const SCAN_STEMS: usize = 48;

const COST_CASE: u32 = 0;
pub(crate) const COST_ADD_DIACRITIC: u32 = 4;
pub(crate) const COST_OTHER_MAP: u32 = 7;
const COST_DROP_DIACRITIC: u32 = 11;
const COST_REP: u32 = 10;
pub(crate) const COST_SPLIT: u32 = 12;
/// A missing space leaving a half shorter than three letters ([`Split::Anywhere`]).
const COST_SPLIT_SHORT: u32 = 17;
/// A missing space between two longer halves with no short word beside it ([`Split::Near`]).
pub(crate) const COST_SPLIT_LONG: u32 = 18;
const COST_KEY: u32 = 13;
const COST_SWAP: u32 = 14;
const COST_DROP_OR_ADD: u32 = 15;
pub(crate) const COST_MISTYPE: u32 = 16;
const COST_SCAN: u32 = 20;

/// What a language adds to its dictionary's tables.
pub(crate) struct Tuning {
    /// Where a missing space may go (round 4).
    pub split: Split,
    /// TRY letters for a dictionary without a TRY line.
    pub alphabet: &'static str,
    /// MAP groups the language adds to its dictionary's (each string is one group: `"cčć"`).
    pub map: &'static [&'static str],
    /// REP confusions the language adds to its dictionary's.
    pub rep: &'static [(&'static str, &'static str)],
    /// The cost of a REP confusion.
    pub rep_cost: u32,
    /// Most letters a MAP variant changes.
    pub max_map_changes: usize,
    /// The cost of changing letter `from` to its MAP relative `to`.
    pub map_cost: fn(char, char) -> u32,
    /// The cost of typing letter `c` at `i` of the word (`chars`, lower case) instead.
    pub mistype_cost: fn(&[char], usize, char) -> u32,
}

/// Where a missing space may go.
pub(crate) enum Split {
    /// Anywhere, dearer when a half is shorter than three letters.
    Anywhere,
    /// After a short word that often runs into the next one (`na pewno`, `w ogóle`) or before one
    /// that runs into the word before (`kocham cię`), dearer between two longer halves, and never
    /// leaving a stray short half (`że ka`).
    Near { leading: &'static [&'static str], trailing: &'static [&'static str] },
    /// The language's own rule: the cost of the halves `a b`, or `None` to skip them.
    Custom(fn(&str, &str) -> Option<u32>),
}

impl Tuning {
    /// The dictionary's tables alone (Czech, Indonesian, Spanish).
    pub(crate) const DEFAULT: Tuning =
        Tuning { split: Split::Anywhere, alphabet: "", map: &[], rep: &[], rep_cost: COST_REP, max_map_changes: 4, map_cost, mistype_cost };
}

/// The cost of changing letter `from` to its MAP relative `to`. Adding a missing accent is the
/// commonest slip (typing without diacritics); an accent the writer did type is likelier right,
/// so taking one away costs more than a known confusion from the REP table.
pub(crate) fn map_cost(from: char, to: char) -> u32 {
    match (from.is_ascii(), to.is_ascii()) {
        (true, false) => COST_ADD_DIACRITIC,
        (false, true) => COST_DROP_DIACRITIC,
        _ => COST_OTHER_MAP,
    }
}

/// Any mistyped letter costs the same.
fn mistype_cost(_: &[char], _: usize, _: char) -> u32 {
    COST_MISTYPE
}

struct Collector<'a> {
    d: &'a AffixDict,
    /// The word, lower case.
    input: String,
    /// Candidate → the cheapest cost it was offered at (rounds don't run in rising cost).
    tested: HashMap<String, u32>,
    /// Suggestion → (cost, order found).
    found: HashMap<String, (u32, usize)>,
    seq: usize,
}

impl Collector<'_> {
    /// Check a lower-case candidate (a word, or two separated by a space) and keep it if every
    /// word is in the dictionary, as written or capitalized (names).
    fn offer(&mut self, cand: &str, cost: u32) {
        if cand.is_empty() || cand == self.input {
            return;
        }
        match self.tested.get_mut(cand) {
            Some(prev) if *prev <= cost => return,
            Some(prev) => *prev = cost,
            None => {
                self.tested.insert(cand.to_string(), cost);
            }
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
                // A name ranks after a common word at the same distance (`moze` → `może`, then
                // `Mozę`).
                cost += 1;
            }
        }
        self.keep(words.join(" "), cost);
    }

    /// Keep a suggestion known to be correct, unless the dictionary says never to offer it.
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

/// The MAP groups: the dictionary's, then the language's.
fn groups<'a>(d: &'a AffixDict, t: &Tuning) -> Cow<'a, [Vec<char>]> {
    if t.map.is_empty() {
        return Cow::Borrowed(d.map());
    }
    Cow::Owned(d.map().iter().cloned().chain(t.map.iter().map(|g| g.chars().collect())).collect())
}

/// The other members of every MAP group of `c` (its likely confusions), each once.
fn alternatives(c: char, map: &[Vec<char>]) -> Vec<char> {
    let mut out: Vec<char> = Vec::new();
    for x in map.iter().filter(|g| g.contains(&c)).flat_map(|g| g.iter().copied()) {
        if x != c && !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

/// Every way of changing up to `t.max_map_changes` letters of `chars` to a MAP relative (`alts`,
/// letter by letter), with its cost, each written after `buf`.
fn map_variants(chars: &[char], alts: &[Vec<char>], t: &Tuning, changes: usize, cost: u32, buf: &mut Vec<char>, out: &mut Vec<(String, u32)>) {
    if out.len() >= MAX_MAP_VARIANTS {
        return;
    }
    let Some((&c, rest)) = chars.split_first() else {
        if changes > 0 {
            out.push((buf.iter().collect(), cost));
        }
        return;
    };
    let (here, later) = alts.split_first().map_or((&[][..], &[][..]), |(h, l)| (h.as_slice(), l));
    buf.push(c);
    map_variants(rest, later, t, changes, cost, buf, out);
    buf.pop();
    if changes < t.max_map_changes {
        for &a in here {
            buf.push(a);
            map_variants(rest, later, t, changes + 1, cost + (t.map_cost)(c, a), buf, out);
            buf.pop();
        }
    }
}

/// A letter's MAP representative (the first of its first group), lower case: `ż` → `z`, `ó` → `o`.
pub(crate) fn fold(c: char, map: &[Vec<char>]) -> char {
    let l = c.to_lowercase().next().unwrap_or(c);
    map.iter().find(|g| g.contains(&l)).and_then(|g| g.first().copied()).unwrap_or(l)
}

/// Optimal string alignment distance where a MAP slip (`a`/`ą`, `u`/`ó`) costs 1 and any other
/// edit 2; `limit + 1` once it is certainly above `limit`.
pub(crate) fn weighted_distance(a: &[char], b: &[char], related: &dyn Fn(char, char) -> bool, limit: u32) -> u32 {
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

/// The cost of a missing space between `a` and `b`, or `None` when the split isn't offered.
fn split_cost(split: &Split, a: &str, b: &str) -> Option<u32> {
    let long = |part: &str| part.chars().count() >= 3;
    match split {
        Split::Anywhere => Some(if long(a) && long(b) { COST_SPLIT } else { COST_SPLIT_SHORT }),
        Split::Near { leading, trailing } => {
            if (leading.contains(&a) && long(b)) || (long(a) && trailing.contains(&b)) {
                Some(COST_SPLIT)
            } else if long(a) && long(b) {
                Some(COST_SPLIT_LONG)
            } else {
                None
            }
        }
        Split::Custom(cost) => cost(a, b),
    }
}

/// Spelling suggestions for `word` from `d`, best first, at most `max`, in the word's letter case.
pub(crate) fn suggest_in(d: &AffixDict, word: &str, max: usize, t: &Tuning) -> Vec<String> {
    let word = word.trim();
    let n = word.chars().count();
    if n == 0 || n > MAX_LETTERS || max == 0 || d.is_empty() || word.contains(char::is_whitespace) {
        return Vec::new();
    }
    let case = casing(word);
    let input = word.to_lowercase();
    let chars: Vec<char> = input.chars().collect();
    let mut col = Collector { d, input: input.clone(), tested: HashMap::new(), found: HashMap::new(), seq: 0 };
    add_case(&mut col, word, &input);
    let map = groups(d, t);
    add_map_slips(&mut col, &chars, &map, t);
    add_confusions(&mut col, &input, t);
    add_splits(&mut col, &input, &t.split);
    add_key_neighbours(&mut col, &chars);
    add_typos(&mut col, &chars, t);
    // 6. Further off: scan the stems that start like the word.
    if col.count_within(COST_MISTYPE) < max {
        scan(&mut col, &chars, &map);
    }
    let mut found: Vec<(String, (u32, usize))> = col.found.into_iter().collect();
    // Among equal costs, the change nearest the word's start is the likelier intent
    // (the first changed letter breaks ties).
    let diff_at = |text: &str| chars.iter().zip(text.chars()).position(|(a, b)| *a != b).unwrap_or(n);
    found.sort_by_key(|(text, (cost, seq))| (*cost, diff_at(text), *seq));
    let mut out: Vec<String> = Vec::new();
    for (text, _) in found {
        let recased = match case {
            Casing::Initial => capitalize(&text),
            Casing::Upper if n > 1 => text.to_uppercase(),
            _ => text.clone(),
        };
        // A KEEPCASE word only exists in its dictionary case: `Km` would be flagged, so offer `km`.
        let t = if recased != text && text.split(' ').any(|p| d.case_locked(p)) { text } else { recased };
        if t != word && !out.contains(&t) {
            out.push(t);
        }
        if out.len() >= max {
            break;
        }
    }
    out
}

/// Round 1: letter case.
fn add_case(col: &mut Collector, word: &str, input: &str) {
    if col.d.contains(input) && word != input {
        col.keep(input.to_string(), COST_CASE);
    } else if col.d.contains(&capitalize(input)) && word != capitalize(input) {
        col.keep(capitalize(input), COST_CASE);
    }
}

/// Round 2: accents and other MAP slips.
fn add_map_slips(col: &mut Collector, chars: &[char], map: &[Vec<char>], t: &Tuning) {
    let alts: Vec<Vec<char>> = chars.iter().map(|&c| alternatives(c, map)).collect();
    let mut variants = Vec::new();
    map_variants(chars, &alts, t, 0, 0, &mut Vec::with_capacity(chars.len()), &mut variants);
    variants.sort_by_key(|v| v.1);
    for (v, cost) in &variants {
        col.offer(v, *cost);
    }
}

/// Round 3: common confusions.
fn add_confusions(col: &mut Collector, input: &str, t: &Tuning) {
    let rep = col.d.rep().iter().map(|(from, to)| (from.as_str(), to.as_str())).chain(t.rep.iter().copied());
    for (from, to) in rep {
        if from.is_empty() {
            continue;
        }
        for (i, _) in input.match_indices(from) {
            if let (Some(a), Some(b)) = (input.get(..i), input.get(i + from.len()..)) {
                col.offer(&format!("{a}{to}{b}"), t.rep_cost);
            }
        }
    }
}

/// Round 4: a missing space; both halves must be dictionary words.
fn add_splits(col: &mut Collector, input: &str, split: &Split) {
    for (i, _) in input.char_indices().skip(1) {
        let (Some(a), Some(b)) = (input.get(..i), input.get(i..)) else { continue };
        if let Some(cost) = split_cost(split, a, b) {
            col.offer(&format!("{a} {b}"), cost);
        }
    }
}

/// Round 5: keyboard neighbours from the dictionary's KEY rows.
fn add_key_neighbours(col: &mut Collector, chars: &[char]) {
    for row in col.d.key().split('|').take(32) {
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
                    let mut v = chars.to_vec();
                    if let Some(x) = v.get_mut(i) {
                        *x = replacement;
                    }
                    col.offer(&v.iter().collect::<String>(), COST_KEY);
                }
            }
        }
    }
}

/// Round 5, second half: one typo, with the TRY letters (the language's when the dictionary has
/// none), bounded and each once, in their order: a TRY line lists the commonest letters first, so
/// equal-cost candidates come out in that order.
fn add_typos(col: &mut Collector, chars: &[char], t: &Tuning) {
    let d = col.d;
    let n = chars.len();
    let try_chars = if d.try_chars().is_empty() { t.alphabet } else { d.try_chars() };
    let mut letters: Vec<char> = Vec::new();
    for c in try_chars.chars().filter(|c| c.is_lowercase()).take(128) {
        if !letters.contains(&c) {
            letters.push(c);
        }
    }
    let s = |v: &[char]| v.iter().collect::<String>();
    for i in 0..n.saturating_sub(1) {
        if chars.get(i) != chars.get(i + 1) {
            let mut v = chars.to_vec();
            v.swap(i, i + 1);
            col.offer(&s(&v), COST_SWAP);
        }
    }
    for i in 0..n {
        let mut v = chars.to_vec();
        v.remove(i);
        col.offer(&s(&v), COST_DROP_OR_ADD);
    }
    for i in 0..=n {
        for &c in &letters {
            let mut v = chars.to_vec();
            v.insert(i, c);
            col.offer(&s(&v), COST_DROP_OR_ADD);
        }
    }
    for i in 0..n {
        for &c in &letters {
            if chars.get(i) != Some(&c) {
                let mut v = chars.to_vec();
                if let Some(x) = v.get_mut(i) {
                    *x = c;
                }
                col.offer(&s(&v), (t.mistype_cost)(chars, i, c));
            }
        }
    }
}

/// Round 6: rank the stems starting with the word's first letter (or a MAP relative of it, either
/// case) by shared letter pairs, then keep the forms of the best within two typos.
fn scan(col: &mut Collector, chars: &[char], map: &[Vec<char>]) {
    let d = col.d;
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
        let suggestions = suggest_in(&d, "počítačz", 8, &Tuning::DEFAULT);
        assert_eq!(suggestions.first().map(String::as_str), Some("počítač"), "{suggestions:?}");
        assert!(suggestions.contains(&"počítač z".into()), "{suggestions:?}");
    }

    /// A candidate offered again at a lower cost (a REP confusion after a MAP diacritic drop)
    /// ranks at the lower one.
    #[test]
    fn a_cheaper_later_offer_wins() {
        let d = AffixDict::from_hunspell("", "1\ncafe\n", &[]).unwrap();
        let mut col = Collector { d: &d, input: "cafè".into(), tested: HashMap::new(), found: HashMap::new(), seq: 0 };
        col.offer("cafe", COST_DROP_DIACRITIC);
        col.offer("cafe", COST_REP);
        col.offer("cafe", COST_SCAN);
        assert_eq!(col.found.get("cafe").map(|e| e.0), Some(COST_REP));
    }

    #[test]
    fn distance_weighs_map_slips_half() {
        let map = vec![vec!['a', 'ą'], vec!['o', 'ó', 'u']];
        let related = |a: char, b: char| fold(a, &map) == fold(b, &map);
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(weighted_distance(&c("ktury"), &c("który"), &related, 9), 1);
        assert_eq!(weighted_distance(&c("kot"), &c("kto"), &related, 9), 2);
        assert_eq!(weighted_distance(&c("kot"), &c("kotek"), &related, 9), 4);
        assert_eq!(weighted_distance(&c("kot"), &c("pies"), &related, 2), 3);
        assert_eq!(weighted_distance(&c(""), &c("ab"), &related, 9), 4);
    }

    /// A KEEPCASE word is offered in its dictionary case when the word's own case would be flagged
    /// (`Kmm` → `km`, never `Km`), and a capitalised KEEPCASE word gets its dictionary form.
    #[test]
    fn keepcase_suggestions_are_not_recased_into_errors() {
        let d = AffixDict::from_hunspell("KEEPCASE K\n", "2\nkm/K\npies\n", &[]).unwrap();
        assert!(!d.check("Km") && !d.check("KM"));
        for (word, want) in [("Kmm", "km"), ("KMM", "km"), ("Km", "km")] {
            let got = suggest_in(&d, word, 4, &Tuning::DEFAULT);
            assert!(got.iter().all(|s| s.split(' ').all(|p| d.check(p))), "{word}: {got:?}");
            assert_eq!(got.first().map(String::as_str), Some(want), "{word}: {got:?}");
        }
        assert_eq!(suggest_in(&d, "Pies", 4, &Tuning::DEFAULT), Vec::<String>::new());
        assert_eq!(suggest_in(&d, "Piesx", 4, &Tuning::DEFAULT).first().map(String::as_str), Some("Pies"));
    }

    /// A language's tables come after its dictionary's, a TRY alphabet stands in for a missing TRY
    /// line, and a split rule decides which missing spaces are offered.
    #[test]
    fn tuning_adds_to_the_dictionary_tables() {
        let d = AffixDict::from_hunspell("", "3\nčaj\nkot\nna\n", &[]).unwrap();
        let tuned = Tuning { map: &["cč"], alphabet: "t", rep: &[("xy", "kot")], ..Tuning::DEFAULT };
        assert!(!suggest_in(&d, "caj", 4, &Tuning::DEFAULT).contains(&"čaj".into()));
        assert!(suggest_in(&d, "caj", 4, &tuned).contains(&"čaj".into()), "the language's MAP group");
        assert!(suggest_in(&d, "xy", 4, &tuned).contains(&"kot".into()), "the language's REP pair");
        assert!(suggest_in(&d, "ko", 4, &tuned).contains(&"kot".into()), "the language's TRY letters");
        let near = Tuning { split: Split::Near { leading: &["na"], trailing: &[] }, ..Tuning::DEFAULT };
        assert!(suggest_in(&d, "nakot", 4, &near).contains(&"na kot".into()));
        assert!(!suggest_in(&d, "kotna", 4, &near).contains(&"kot na".into()), "no stray short half");
        assert!(suggest_in(&d, "kotna", 4, &Tuning::DEFAULT).contains(&"kot na".into()));
    }
}
