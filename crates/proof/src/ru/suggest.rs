//! Russian spelling suggestions, best first.
//!
//! Russian Hunspell dictionaries carry no `MAP` or `REP` tables, so the letter groups and the
//! common confusions are defined here. Candidates come in rounds of rising cost, each checked
//! against the dictionary:
//! 1. letter case (`москва` → `Москва`);
//! 2. letter slips within a group ([`MAP`]): `е`/`ё` first, then unstressed vowels (`о`/`а`,
//!    `е`/`и`/`я`), voiced and voiceless pairs (`з`/`с`, `д`/`т`, `б`/`п`…) and `ь`/`ъ`, up to
//!    three letters changed (`превет` → `привет`, `сдесь` → `здесь`);
//! 3. the common confusions of [`REP`] (`жы` → `жи`, `чя` → `ча`, `тся`/`ться`, a dropped or
//!    doubled consonant, `здраствуй` → `здравствуй`);
//! 4. a missing space (`незнаю` → `не знаю`, `вобщем` → `в общем`), cheaper when one half is a
//!    short function word;
//! 5. one typo: a letter left out, added, swapped with the next or mistyped;
//! 6. when that finds fewer than asked for: the stems that start like the word and share the
//!    most letter pairs with it, whose forms are ranked by an edit distance in which a slip within
//!    a letter group costs half (two typos at most).
//!
//! The input is bounded (40 letters), and so is every round.

use std::collections::{HashMap, HashSet};

use crate::affix::{AffixDict, Casing, capitalize, casing};

/// Letters confused with each other: a slip inside a group is cheaper than another typo.
const MAP: &[&str] = &["её", "оа", "еия", "зс", "дт", "бп", "вф", "гк", "жш", "ьъ", "еэ", "ыи"];

/// Common confusions: a written sequence and what was meant.
const REP: &[(&str, &str)] = &[
    ("жы", "жи"),
    ("шы", "ши"),
    ("чя", "ча"),
    ("щя", "ща"),
    ("чю", "чу"),
    ("щю", "щу"),
    ("тся", "ться"),
    ("ться", "тся"),
    ("сч", "щ"),
    ("щ", "сч"),
    ("нн", "н"),
    ("н", "нн"),
    ("сс", "с"),
    ("с", "сс"),
    ("лл", "л"),
    ("л", "лл"),
    ("тт", "т"),
    ("т", "тт"),
    ("мм", "м"),
    ("м", "мм"),
    ("рр", "р"),
    ("р", "рр"),
    ("ье", "ъе"),
    ("ъе", "ье"),
    ("раст", "рост"),
    ("рост", "раст"),
    ("лож", "лаг"),
    ("лаг", "лож"),
    // Unpronounced consonants: `здраствуй`, `чуствовать`, `солнце`, `лесница`, `праздник`.
    ("аст", "авст"),
    ("уст", "увст"),
    ("лнц", "лц"),
    ("лц", "лнц"),
    ("сн", "стн"),
    ("стн", "сн"),
    ("зн", "здн"),
    ("здн", "зн"),
    ("ц", "тс"),
    ("тс", "ц"),
];

/// Short words often run into the next one (`не знаю`, `в общем`, `на верху`, `потому что`).
const LEADING_WORDS: &[&str] = &[
    "не",
    "ни",
    "в",
    "во",
    "на",
    "с",
    "со",
    "к",
    "ко",
    "о",
    "об",
    "обо",
    "от",
    "ото",
    "до",
    "из",
    "изо",
    "за",
    "по",
    "под",
    "над",
    "при",
    "про",
    "без",
    "для",
    "у",
    "а",
    "и",
    "но",
    "да",
    "же",
    "как",
    "так",
    "то",
    "что",
    "уже",
    "еще",
    "ещё",
    "все",
    "всё",
    "потому",
    "чтобы",
    "это",
];
/// Prepositions among [`LEADING_WORDS`].
const PREPOSITIONS: &[&str] = &[
    "в", "во", "на", "с", "со", "к", "ко", "о", "об", "обо", "от", "ото", "до", "из", "изо", "за", "по", "под", "над", "при", "про", "без", "для",
    "у",
];
/// Short words often run into the one before (`мог бы`, `он же`, `так ли`).
const TRAILING_WORDS: &[&str] = &["ли", "же", "бы", "ж", "б", "что"];
/// The Russian alphabet, when the dictionary has no `TRY` line.
const ALPHABET: &str = "оеаинтсрвлкмдпуяыьгзбчйхжшюцщэфъё";
const MAX_LETTERS: usize = 40;
const MAX_MAP_CHANGES: usize = 3;
const MAX_MAP_VARIANTS: usize = 4096;
/// Stems the scan of round 6 expands.
const SCAN_STEMS: usize = 48;

const COST_CASE: u32 = 0;
const COST_YO: u32 = 3;
const COST_MAP: u32 = 7;
const COST_REP: u32 = 9;
const COST_SPLIT: u32 = 12;
const COST_SPLIT_PREPOSITION: u32 = 17;
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
    /// Check a lower-case candidate (a word, or two separated by a space) and keep it if every
    /// word is in the dictionary, as written or capitalized (names).
    fn offer(&mut self, cand: &str, cost: u32) {
        if cand.is_empty() || cand == self.input || !self.tested.insert(cand.to_string()) {
            return;
        }
        let mut words = Vec::new();
        let mut cost = cost;
        for part in cand.split(' ') {
            if self.d.contains(part) {
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

/// The other members of each [`MAP`] group holding `c`.
fn alternatives(c: char) -> Vec<char> {
    let mut out: Vec<char> = MAP.iter().filter(|g| g.contains(c)).flat_map(|g| g.chars()).filter(|&x| x != c).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The cost of changing letter `from` to its group relative `to`: `е`/`ё` is the cheapest slip
/// (`ё` is optional in ordinary writing).
fn map_cost(from: char, to: char) -> u32 {
    if matches!((from, to), ('е', 'ё') | ('ё', 'е')) { COST_YO } else { COST_MAP }
}

/// Every way of changing up to [`MAX_MAP_CHANGES`] letters to a group relative, with its cost.
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

/// A letter's group representative (the first of its first group), lower case: `ё` → `е`,
/// `з` → `з`, `с` → `з`.
fn fold(c: char) -> char {
    let l = c.to_lowercase().next().unwrap_or(c);
    MAP.iter().find(|g| g.contains(l)).and_then(|g| g.chars().next()).unwrap_or(l)
}

/// Optimal string alignment distance where a slip within a letter group costs 1 and any other
/// edit 2; `limit + 1` once it is certainly above `limit`.
fn weighted_distance(a: &[char], b: &[char], limit: u32) -> u32 {
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
            } else if fold(ai) == fold(bj) {
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

/// Spelling suggestions for a Russian word, best first, at most `max`, in the word's letter case.
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
    // 2. Slips within a letter group.
    let alts: Vec<Vec<char>> = chars.iter().map(|&c| alternatives(c)).collect();
    let mut variants = Vec::new();
    map_variants(&chars, &alts, 0, 0, 0, &mut Vec::with_capacity(n), &mut variants);
    variants.sort_by_key(|v| v.1);
    for (v, cost) in &variants {
        col.offer(v, *cost);
    }
    // 3. Common confusions.
    for (from, to) in REP {
        for (i, _) in input.match_indices(from) {
            if let (Some(a), Some(b)) = (input.get(..i), input.get(i + from.len()..)) {
                col.offer(&format!("{a}{to}{b}"), COST_REP);
            }
        }
    }
    // 4. A missing space: cheap after a short leading word or before a short trailing one,
    //    dearer between two longer words, never leaving a stray short half.
    for (i, _) in input.char_indices().skip(1) {
        let (Some(a), Some(b)) = (input.get(..i), input.get(i..)) else { continue };
        let long = |p: &str| p.chars().count() >= 3;
        // A preposition takes a noun, not a verb (`с генерировать` isn't a fix for
        // `сгенерировать`); after one or two letters a split is likelier wrong than a typo.
        let preposition = PREPOSITIONS.contains(&a);
        if preposition && ["ть", "ться", "ти", "чь"].iter().any(|e| b.ends_with(e)) {
            continue;
        }
        let cost = if preposition && a.chars().count() <= 2 && LEADING_WORDS.contains(&a) && long(b) {
            COST_SPLIT_PREPOSITION
        } else if (LEADING_WORDS.contains(&a) && long(b)) || (long(a) && TRAILING_WORDS.contains(&b)) {
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
        letters = ALPHABET.chars().collect();
    }
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
    found.sort_by_key(|(_, k)| *k);
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

/// Round 6: rank the stems starting with the word's first letter (or a group relative of it,
/// either case) by shared letter pairs, then keep the forms of the best within two typos.
fn scan(col: &mut Collector, chars: &[char]) {
    let d = col.d;
    let Some(&first) = chars.first() else { return };
    let folded: Vec<char> = chars.iter().map(|&c| fold(c)).collect();
    let pairs: HashSet<(char, char)> = std::iter::once(' ').chain(folded.iter().copied()).zip(folded.iter().copied()).collect();
    let mut firsts: Vec<char> = std::iter::once(first).chain(alternatives(first)).collect();
    let upper: Vec<char> = firsts.iter().flat_map(|c| c.to_uppercase()).collect();
    firsts.extend(upper);
    firsts.sort_unstable();
    firsts.dedup();
    let n = folded.len();
    let mut best: Vec<(u32, &str)> = Vec::new();
    for f in firsts {
        let prefix = f.to_string();
        for stem in d.stems_with_prefix(&prefix) {
            let sf: Vec<char> = stem.chars().map(fold).collect();
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
    let limit = 4;
    for (_, stem) in best {
        for form in d.forms(stem) {
            let lf: Vec<char> = form.to_lowercase().chars().collect();
            if lf.len().abs_diff(chars.len()) > 2 {
                continue;
            }
            let dist = weighted_distance(&lf, chars, limit);
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
    fn russian_misspellings_get_the_right_suggestion() {
        crate::ru::test_dict::install();
        has("превет", "привет");
        has("сдесь", "здесь");
        has("жыть", "жить");
        has("чясто", "часто");
        has("незнаю", "не знаю");
        has("вобщем", "в общем");
        has("здраствуй", "здравствуй");
        has("москва", "Москва");
        has("Превет", "Привет");
        has("ПРЕВЕТ", "ПРИВЕТ");
        has("читаеться", "читается");
        has("краснного", "красного");
        has("западнный", "западный");
        assert!(!suggest("сгенерировать", 8).contains(&"с генерировать".to_string()));
    }

    #[test]
    fn the_likeliest_fix_comes_first() {
        crate::ru::test_dict::install();
        assert_eq!(suggest("превет", 3).first().map(String::as_str), Some("привет"));
        assert_eq!(suggest("незнаю", 3).first().map(String::as_str), Some("не знаю"));
        assert_eq!(suggest("жыть", 3).first().map(String::as_str), Some("жить"));
        assert_eq!(suggest("зеленый", 3).first().map(String::as_str), Some("зелёный"));
    }

    #[test]
    fn edge_cases_stay_empty_or_bounded() {
        crate::ru::test_dict::install();
        assert!(suggest("", 5).is_empty());
        assert!(suggest("дом", 0).is_empty());
        assert!(suggest(&"а".repeat(41), 5).is_empty());
        assert!(suggest("два слова", 5).is_empty());
        assert!(suggest("привет", 5).len() <= 5);
        assert!(!suggest("привет", 5).contains(&"привет".to_string()));
    }

    #[test]
    fn distance_weighs_letter_groups_half() {
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(weighted_distance(&c("сдесь"), &c("здесь"), 9), 1);
        assert_eq!(weighted_distance(&c("ёлка"), &c("елка"), 9), 1);
        assert_eq!(weighted_distance(&c("дом"), &c("дмо"), 9), 2);
        assert_eq!(weighted_distance(&c("дом"), &c("домами"), 9), 6);
        assert_eq!(weighted_distance(&c("дом"), &c("книга"), 2), 3);
    }

    #[test]
    fn suggestions_are_fast_enough() {
        crate::ru::test_dict::install();
        let _ = suggest("дом", 1);
        let t = std::time::Instant::now();
        for w in ["ыкщзвотрусскогословаааа", "здраствуйте", "краснннного"] {
            let _ = suggest(w, 6);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("3 suggestion calls: {ms:.0} ms");
        assert!(ms < 20_000.0, "{ms} ms");
    }
}
