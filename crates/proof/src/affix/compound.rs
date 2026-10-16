//! Compound words and word breaks, as Hunspell's affix files describe them.
//!
//! Languages such as Hungarian write compounds as one word (`almafa`, `házkulcsban`), so a
//! dictionary lists the parts and marks which may join:
//!
//! - **Compound flags.** A part needs `COMPOUNDFLAG` or the flag for its place (`COMPOUNDBEGIN`,
//!   `COMPOUNDMIDDLE`, `COMPOUNDEND`), on the stem or on an affix's continuation classes. A
//!   prefix may start a compound and a suffix may end it; inside, an affix needs
//!   `COMPOUNDPERMITFLAG` or a compound flag of its own. `COMPOUNDFORBIDFLAG`, on a stem or a suffix, keeps the word out of
//!   the first and middle places (`nárciszmetrózik` ends in one). `ONLYINCOMPOUND` stems and
//!   affixes appear only inside compounds; parts are at least `COMPOUNDMIN` letters (default 3)
//!   and carry at most one suffix (two with `COMPOUNDMORESUFFIXES`).
//! - **Size.** At most `COMPOUNDWORDMAX` words (a `COMPOUNDROOT` stem, itself a compound, counts
//!   as two). Past that a compound is still correct when it has at most the `COMPOUNDSYLLABLE`
//!   count of syllables: Hungarian's 6–3 rule, where a compound of more than two words and more
//!   than six syllables takes a hyphen. Syllables are the vowels `COMPOUNDSYLLABLE` lists; the
//!   inflections on the last part don't count. A suffix after which the word may compound again
//!   forms a new word and counts, as do the suffix classes `SYLLABLENUM` lists. An inflected
//!   form listed as a stem counts as its lemma (`st:` in its morphological fields: `falevelek`
//!   as `falevél`).
//! - **Boundaries.** `CHECKCOMPOUNDDUP` (no part twice in a row), `CHECKCOMPOUNDTRIPLE` (no
//!   letter three times across a boundary: `sakkkör`), `CHECKCOMPOUNDCASE` (no capital at a
//!   boundary), `CHECKCOMPOUNDPATTERN` (no part ending in one string followed by one starting
//!   with another) and `CHECKCOMPOUNDREP` (no compound, and no two neighbouring parts, that is a
//!   `REP` misspelling of a word: `hajlak` for `haljak`). Two neighbouring parts that together
//!   are a listed word are read as that word (`szezon` + `légzáró`, never `lég` + `záró`).
//! - **`COMPOUNDRULE`**: patterns of flags with `*` and `?` (`0*1`: numbers such as
//!   `1999-ben`), matched by stems, the last of which may carry suffixes.
//! - **`BREAK`**: strings at which a word is split and both sides checked (`Petőfi-vers`); `^`
//!   and `$` anchor one to the start or the end (`-féle$`). Only dictionaries that declare
//!   `BREAK` get breaks; Hunspell's built-in default (break at hyphens) isn't applied, so the
//!   languages that check hyphenated words themselves keep doing so.
//!
//! `NEEDAFFIX` stems are words only with an affix; it is read here with the other flags of this
//! family. Searches are bounded: at most [`MAX_PARTS`] parts and [`PART_BUDGET`] part lookups
//! per word, and [`BREAK_BUDGET`] checks for the breaks of one word.

use super::{AffixDict, Mask, Parse, Rule};

/// Directives read into [`Compounding`].
pub(super) const DIRECTIVES: &[&str] = &[
    "BREAK",
    "CHECKCOMPOUNDCASE",
    "CHECKCOMPOUNDDUP",
    "CHECKCOMPOUNDPATTERN",
    "CHECKCOMPOUNDREP",
    "CHECKCOMPOUNDTRIPLE",
    "COMPOUNDBEGIN",
    "COMPOUNDEND",
    "COMPOUNDFIRST",
    "COMPOUNDFLAG",
    "COMPOUNDFORBIDFLAG",
    "COMPOUNDLAST",
    "COMPOUNDMIDDLE",
    "COMPOUNDMIN",
    "COMPOUNDMORESUFFIXES",
    "COMPOUNDPERMITFLAG",
    "COMPOUNDROOT",
    "COMPOUNDRULE",
    "COMPOUNDSYLLABLE",
    "COMPOUNDWORDMAX",
    "NEEDAFFIX",
    "ONLYINCOMPOUND",
    "PSEUDOROOT",
    "SYLLABLENUM",
];

/// Directives whose entries follow a count line (`BREAK 3`).
const TABLES: [&str; 3] = ["BREAK", "CHECKCOMPOUNDPATTERN", "COMPOUNDRULE"];

/// Most parts in one compound.
pub(super) const MAX_PARTS: usize = 12;
/// Most part lookups while checking one word for compounds.
pub(super) const PART_BUDGET: u32 = 6000;
/// Most checks spent on the break points of one word.
pub(super) const BREAK_BUDGET: u32 = 64;

/// Private-use flags the reader adds to an inflected form listed as a stem: one, two or three
/// syllables more than its lemma (`st:` in its morphology), which the 6–3 rule doesn't count.
const LEMMA_FLAGS: [char; 3] = ['\u{F8F1}', '\u{F8F2}', '\u{F8F3}'];

/// The flag for a stem `extra` syllables longer than its lemma (none for 0).
pub(super) fn lemma_flag(extra: usize) -> Option<char> {
    LEMMA_FLAGS.get(extra.checked_sub(1)?.min(LEMMA_FLAGS.len() - 1)).copied()
}

/// The vowels of `COMPOUNDSYLLABLE`, when the dictionary counts syllables.
pub(super) fn syllable_vowels(lines: &[String]) -> Option<Vec<char>> {
    lines.iter().find_map(|l| {
        let mut f = l.split(' ');
        (f.next() == Some("COMPOUNDSYLLABLE")).then(|| f.nth(1).unwrap_or("").chars().collect())
    })
}

/// Skips the count line of each table directive (the first line with one number).
#[derive(Default)]
pub(super) struct TableHeaders {
    seen: [bool; TABLES.len()],
}

impl TableHeaders {
    /// The directive line to keep, or `None` for a table's count line.
    pub(super) fn line(&mut self, fields: &[&str]) -> Option<String> {
        let dir = fields.first().copied().unwrap_or("");
        if let Some(i) = TABLES.iter().position(|&t| t == dir)
            && let Some(seen) = self.seen.get_mut(i)
            && !*seen
            && fields.len() == 2
            && fields.get(1).is_some_and(|n| n.parse::<usize>().is_ok())
        {
            *seen = true;
            return None;
        }
        Some(fields.join(" "))
    }
}

/// `lines` with the count line each table needs in front of its entries.
pub(super) fn with_table_headers(lines: &[String]) -> Vec<String> {
    let dir = |l: &String| l.split(' ').next().unwrap_or("").to_string();
    let mut out: Vec<String> = lines.iter().filter(|l| !TABLES.contains(&dir(l).as_str())).cloned().collect();
    for t in TABLES {
        let entries: Vec<&String> = lines.iter().filter(|l| dir(l) == t).collect();
        if !entries.is_empty() {
            out.push(format!("{t} {}", entries.len()));
            out.extend(entries.into_iter().cloned());
        }
    }
    out
}

/// How often a `COMPOUNDRULE` element may repeat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Repeat {
    One,
    Any,
    Optional,
}

/// A `CHECKCOMPOUNDPATTERN`: no part ending in `end` (with `end_flag`, if any) before one
/// starting with `begin` (with `begin_flag`).
#[derive(Clone, Debug)]
struct Pattern {
    end: String,
    end_flag: Mask,
    begin: String,
    begin_flag: Mask,
}

/// The compounding settings of a dictionary (see the module docs).
#[derive(Clone, Debug)]
pub(super) struct Compounding {
    flag: Mask,
    begin: Mask,
    middle: Mask,
    end: Mask,
    pub(super) only_in: Mask,
    permit: Mask,
    forbid: Mask,
    root: Mask,
    pub(super) need_affix: Mask,
    min: usize,
    /// `COMPOUNDMORESUFFIXES`: a part may carry two suffixes in a row (one otherwise).
    more_suffixes: bool,
    word_max: Option<usize>,
    syllable_max: Option<usize>,
    vowels: Box<[char]>,
    syllable_num: Mask,
    /// [`LEMMA_FLAGS`]' bits.
    lemma: [Mask; 3],
    check_dup: bool,
    check_rep: bool,
    check_case: bool,
    check_triple: bool,
    patterns: Vec<Pattern>,
    rules: Vec<Vec<(Mask, Repeat)>>,
    breaks: Vec<String>,
}

impl Default for Compounding {
    fn default() -> Self {
        Compounding {
            flag: Mask::EMPTY,
            begin: Mask::EMPTY,
            middle: Mask::EMPTY,
            end: Mask::EMPTY,
            only_in: Mask::EMPTY,
            permit: Mask::EMPTY,
            forbid: Mask::EMPTY,
            root: Mask::EMPTY,
            need_affix: Mask::EMPTY,
            min: 3,
            more_suffixes: false,
            word_max: None,
            syllable_max: None,
            vowels: Box::new([]),
            syllable_num: Mask::EMPTY,
            lemma: [Mask::EMPTY; 3],
            check_dup: false,
            check_rep: false,
            check_case: false,
            check_triple: false,
            patterns: Vec::new(),
            rules: Vec::new(),
            breaks: Vec::new(),
        }
    }
}

/// `text/flag`: the text and the flag's bit (none when there is no `/`).
fn split_flag(field: &str, bit_of: &dyn Fn(char) -> Mask) -> (String, Mask) {
    match field.split_once('/') {
        Some((t, f)) => (t.to_string(), f.chars().fold(Mask::EMPTY, |m, c| m | bit_of(c))),
        None => (field.to_string(), Mask::EMPTY),
    }
}

/// Parse a `COMPOUNDRULE` pattern: flags, each optionally followed by `*` or `?`; a flag may be
/// written in parentheses, `(x)`.
fn parse_rule(text: &str, bit_of: &dyn Fn(char) -> Mask) -> Result<Vec<(Mask, Repeat)>, String> {
    let mut out: Vec<(Mask, Repeat)> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let flag = match c {
            '*' | '?' => {
                let Some(last) = out.last_mut() else { return Err(format!("COMPOUNDRULE {text}: `{c}` with nothing before it")) };
                last.1 = if c == '*' { Repeat::Any } else { Repeat::Optional };
                continue;
            }
            '(' => {
                let inner: String = chars.by_ref().take_while(|&d| d != ')').collect();
                let mut it = inner.chars();
                match (it.next(), it.next()) {
                    (Some(f), None) => f,
                    _ => return Err(format!("COMPOUNDRULE {text}: only one-character flags are supported")),
                }
            }
            c => c,
        };
        out.push((bit_of(flag), Repeat::One));
    }
    Ok(out)
}

impl Compounding {
    /// Read the directive lines kept by [`TableHeaders`].
    pub(super) fn from_lines(lines: &[String], bit_of: &dyn Fn(char) -> Mask) -> Result<Compounding, String> {
        let mut c = Compounding { lemma: LEMMA_FLAGS.map(bit_of), ..Compounding::default() };
        for line in lines {
            let f: Vec<&str> = line.split(' ').filter(|s| !s.is_empty()).collect();
            let at = |k: usize| f.get(k).copied().unwrap_or("");
            let flags = |s: &str| s.chars().fold(Mask::EMPTY, |m, ch| m | bit_of(ch));
            let number = |s: &str| s.parse::<usize>().map_err(|_| format!("{line}: not a number"));
            match at(0) {
                "COMPOUNDFLAG" => c.flag = flags(at(1)),
                "COMPOUNDBEGIN" | "COMPOUNDFIRST" => c.begin = flags(at(1)),
                "COMPOUNDMIDDLE" => c.middle = flags(at(1)),
                "COMPOUNDEND" | "COMPOUNDLAST" => c.end = flags(at(1)),
                "ONLYINCOMPOUND" => c.only_in = flags(at(1)),
                "COMPOUNDPERMITFLAG" => c.permit = flags(at(1)),
                "COMPOUNDFORBIDFLAG" => c.forbid = flags(at(1)),
                "COMPOUNDROOT" => c.root = flags(at(1)),
                "NEEDAFFIX" | "PSEUDOROOT" => c.need_affix = flags(at(1)),
                "COMPOUNDMIN" => c.min = number(at(1))?.max(1),
                "COMPOUNDMORESUFFIXES" => c.more_suffixes = true,
                "COMPOUNDWORDMAX" => c.word_max = Some(number(at(1))?),
                "COMPOUNDSYLLABLE" => {
                    c.syllable_max = Some(number(at(1))?);
                    c.vowels = at(2).chars().collect();
                }
                "SYLLABLENUM" => c.syllable_num = flags(at(1)),
                "CHECKCOMPOUNDDUP" => c.check_dup = true,
                "CHECKCOMPOUNDREP" => c.check_rep = true,
                "CHECKCOMPOUNDCASE" => c.check_case = true,
                "CHECKCOMPOUNDTRIPLE" => c.check_triple = true,
                // A third field is a simplified spelling the compound may take instead; those
                // patterns aren't applied.
                "CHECKCOMPOUNDPATTERN" if f.len() == 3 => {
                    let (end, end_flag) = split_flag(at(1), bit_of);
                    let (begin, begin_flag) = split_flag(at(2), bit_of);
                    if end != "0" {
                        c.patterns.push(Pattern { end, end_flag, begin, begin_flag });
                    }
                }
                "COMPOUNDRULE" => c.rules.push(parse_rule(at(1), bit_of)?),
                "BREAK" if !at(1).is_empty() => c.breaks.push(at(1).to_string()),
                _ => {}
            }
        }
        Ok(c)
    }

    /// Does the dictionary build compounds at all?
    fn enabled(&self) -> bool {
        !(self.flag | self.begin | self.middle | self.end).is_empty()
    }
}

/// Where a part sits in a compound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Place {
    First,
    Middle,
    Last,
}

/// What the search keeps about an accepted part.
#[derive(Clone, Copy, Debug)]
struct Part {
    /// How many words it counts as (2 for a `COMPOUNDROOT` stem).
    words: usize,
    stem: Mask,
    /// Syllables of its suffixes that don't count toward the 6–3 rule.
    uncounted: usize,
}

/// One compound search: the word, its char boundaries, and the parts looked up so far.
struct Search<'w> {
    w: &'w str,
    /// Byte offsets of the char boundaries, `0` and `w.len()` included.
    bounds: Vec<usize>,
    parts: std::collections::HashMap<(usize, usize, Place), Option<Part>>,
    budget: u32,
    deny: Mask,
}

impl AffixDict {
    /// Is `w` (normalized, case as written) a compound word?
    pub(super) fn compound_word(&self, w: &str, deny: Mask) -> bool {
        let c = &self.compound;
        if (!c.enabled() && c.rules.is_empty()) || w.len() > super::MAX_WORD_BYTES {
            return false;
        }
        let bounds: Vec<usize> = w.char_indices().map(|(i, _)| i).chain(std::iter::once(w.len())).collect();
        if bounds.len() < 3 {
            return false;
        }
        let mut s = Search { w, bounds, parts: std::collections::HashMap::new(), budget: PART_BUDGET, deny };
        (c.enabled() && self.compound_from(&mut s, 0, 0, None)) || c.rules.iter().any(|r| self.rule_from(&mut s, r, 0, 0, 0))
    }

    /// Can `s.w` from boundary `start` on be the rest of a compound that has `words` words so
    /// far, the last of them `prev` (byte range and part)?
    fn compound_from(&self, s: &mut Search<'_>, start: usize, words: usize, prev: Option<(usize, usize, Part)>) -> bool {
        let c = &self.compound;
        let last_bound = s.bounds.len() - 1;
        if words >= MAX_PARTS {
            return false;
        }
        for end in (start + c.min)..=last_bound {
            if s.budget == 0 {
                return false;
            }
            let is_last = end == last_bound;
            if start == 0 && is_last {
                continue;
            }
            if !is_last && last_bound - end < c.min {
                continue;
            }
            let (Some(&a), Some(&b)) = (s.bounds.get(start), s.bounds.get(end)) else { continue };
            let place = if start == 0 {
                Place::First
            } else if is_last {
                Place::Last
            } else {
                Place::Middle
            };
            let Some(part) = self.part(s, a, b, place) else { continue };
            if let Some((pa, pb, prev_part)) = prev
                && !self.boundary_ok(s.w, (pa, pb, prev_part), (a, b, part))
            {
                continue;
            }
            let total = words + part.words;
            if is_last {
                if self.compound_done(s.w, total, part) {
                    return true;
                }
            } else if self.compound_from(s, end, total, Some((a, b, part))) {
                return true;
            }
        }
        false
    }

    /// Is `s.w[a..b]` a compound part at `place`? Looked up once per range and place.
    fn part(&self, s: &mut Search<'_>, a: usize, b: usize, place: Place) -> Option<Part> {
        if let Some(p) = s.parts.get(&(a, b, place)) {
            return *p;
        }
        s.budget = s.budget.saturating_sub(1);
        let text = s.w.get(a..b)?;
        let c = &self.compound;
        let mut best: Option<Part> = None;
        let deny = s.deny;
        self.parses(text, &mut |p| {
            if !self.part_ok(p, place, deny) {
                return false;
            }
            let words = if p.stem.meets(c.root) { 2 } else { 1 };
            let uncounted = if place == Place::Last { self.uncounted_syllables(p) } else { 0 };
            if best.is_none_or(|q| (words, std::cmp::Reverse(uncounted)) < (q.words, std::cmp::Reverse(q.uncounted))) {
                best = Some(Part { words, stem: p.stem, uncounted });
            }
            // The last part may yet have a reading with more uncounted inflection.
            words == 1 && place != Place::Last
        });
        s.parts.insert((a, b, place), best);
        best
    }

    /// May this parse stand at `place` in a compound?
    fn part_ok(&self, p: &Parse<'_>, place: Place, deny: Mask) -> bool {
        let c = &self.compound;
        if p.stem.meets(self.forbidden | deny) {
            return false;
        }
        if !p.affixed() && p.stem.meets(c.need_affix) || p.outer_cont().meets(c.need_affix) {
            return false;
        }
        if p.suffixes[1].is_some() && !c.more_suffixes {
            return false;
        }
        let roles = p.stem | p.conts();
        let at_place = match place {
            Place::First => c.begin,
            Place::Middle => c.middle,
            Place::Last => c.end,
        };
        if !roles.meets(c.flag | at_place) {
            return false;
        }
        if place != Place::Last && (p.stem.meets(c.forbid) || p.suffix_rules().any(|r| r.cont.meets(c.forbid))) {
            return false;
        }
        // Inside a compound an affix needs `COMPOUNDPERMITFLAG`, unless it carries the compound
        // flag itself (`kedvez` + `ő` → `kedvezőpocak`).
        let inside = c.permit | c.flag | at_place;
        if place != Place::First && p.prefix.is_some_and(|r| !r.cont.meets(inside)) {
            return false;
        }
        !(place != Place::Last && p.suffix_rules().any(|r: &Rule| !r.cont.meets(inside)))
    }

    /// Syllables of the suffixes on a parse that the 6–3 rule doesn't count: the inflections.
    /// A suffix after which the word may compound again (a compound flag in its continuation
    /// classes: `ápol` + `ás`) forms a new word and counts, as do the classes `SYLLABLENUM`
    /// lists (`-szerű`, `-féle`).
    fn uncounted_syllables(&self, p: &Parse<'_>) -> usize {
        let c = &self.compound;
        let forms_word = c.flag | c.begin | c.middle | c.end | c.syllable_num;
        let lemma = (1..).zip(c.lemma).find(|(_, m)| p.stem.meets(*m)).map_or(0, |(n, _)| n);
        lemma + p.suffix_rules().filter(|r| !r.cont.meets(forms_word) && !r.bit.meets(c.syllable_num)).map(|r| self.syllables(&r.add)).sum::<usize>()
    }

    fn syllables(&self, s: &str) -> usize {
        s.chars().filter(|ch| self.compound.vowels.contains(&ch.to_lowercase().next().unwrap_or(*ch))).count()
    }

    /// May part `(c, d)` follow part `(a, b)`?
    fn boundary_ok(&self, w: &str, (a, b, prev): (usize, usize, Part), (c, d, next): (usize, usize, Part)) -> bool {
        let cfg = &self.compound;
        let (Some(left), Some(right)) = (w.get(a..b), w.get(c..d)) else { return false };
        if cfg.check_dup && left == right {
            return false;
        }
        // Two parts that together are a listed word are that word (`lég` + `záró` is
        // `légzáró`), which compounds only as its own flags allow.
        let joined = format!("{left}{right}");
        if self.stem_mask(&joined).is_some() {
            return false;
        }
        if cfg.check_rep && self.rep_misspelling(&joined) {
            return false;
        }
        let mut l = left.chars().rev();
        let mut r = right.chars();
        let (l1, l2, r1, r2) = (l.next(), l.next(), r.next(), r.next());
        if cfg.check_triple && l1.is_some() && l1 == r1 && (l2 == l1 || r2 == r1) {
            return false;
        }
        if cfg.check_case
            && let (Some(x), Some(y)) = (l1, r1)
            && (x.is_uppercase() || y.is_uppercase())
            && x != '-'
            && y != '-'
        {
            return false;
        }
        !cfg.patterns.iter().any(|p| {
            left.ends_with(p.end.as_str())
                && right.starts_with(p.begin.as_str())
                && (p.end_flag.is_empty() || prev.stem.meets(p.end_flag))
                && (p.begin_flag.is_empty() || next.stem.meets(p.begin_flag))
        })
    }

    /// The checks on a whole compound of `words` words whose last part is `last`.
    fn compound_done(&self, w: &str, words: usize, last: Part) -> bool {
        let c = &self.compound;
        if let Some(max) = c.word_max
            && words > max
        {
            let Some(syllables) = c.syllable_max else { return false };
            if self.syllables(w).saturating_sub(last.uncounted) > syllables {
                return false;
            }
        }
        !(c.check_rep && self.rep_misspelling(w))
    }

    /// Is `w` a `REP` misspelling of a word: does one replacement turn it into a word without
    /// compounding? `^` and `$` tie a pattern to the start or the end.
    fn rep_misspelling(&self, w: &str) -> bool {
        for (from, to) in &self.rep {
            let (anchored_start, from) = match from.strip_prefix('^') {
                Some(f) => (true, f),
                None => (false, from.as_str()),
            };
            let (anchored_end, from) = match from.strip_suffix('$') {
                Some(f) => (true, f),
                None => (false, from),
            };
            if from.is_empty() {
                continue;
            }
            for (i, _) in w.match_indices(from) {
                if (anchored_start && i != 0) || (anchored_end && i + from.len() != w.len()) {
                    continue;
                }
                let (Some(head), Some(tail)) = (w.get(..i), w.get(i + from.len()..)) else { continue };
                if self.simple(&format!("{head}{to}{tail}"), Mask::EMPTY) {
                    return true;
                }
            }
        }
        false
    }

    /// Does `s.w` from boundary `start` on match `rule` from element `at` on, with `parts`
    /// parts so far? Every part is a stem carrying its element's flag; the last may carry
    /// suffixes.
    fn rule_from(&self, s: &mut Search<'_>, rule: &[(Mask, Repeat)], at: usize, start: usize, parts: usize) -> bool {
        let last_bound = s.bounds.len() - 1;
        if start == last_bound {
            return parts >= 2 && rule.get(at..).unwrap_or_default().iter().all(|e| e.1 != Repeat::One);
        }
        let Some(&(flag, repeat)) = rule.get(at) else { return false };
        if parts >= MAX_PARTS || s.budget == 0 {
            return false;
        }
        if repeat != Repeat::One && self.rule_from(s, rule, at + 1, start, parts) {
            return true;
        }
        for end in (start + 1)..=last_bound {
            s.budget = s.budget.saturating_sub(1);
            if s.budget == 0 {
                return false;
            }
            let (Some(&a), Some(&b)) = (s.bounds.get(start), s.bounds.get(end)) else { continue };
            let Some(text) = s.w.get(a..b) else { continue };
            let ok = if end == last_bound {
                self.parses(text, &mut |p| p.prefix.is_none() && p.stem.meets(flag) && !p.stem.meets(self.forbidden | s.deny))
            } else {
                self.stem_mask(text).is_some_and(|m| m.meets(flag) && !m.meets(self.forbidden | s.deny))
            };
            let next = if repeat == Repeat::Any { at } else { at + 1 };
            if ok && self.rule_from(s, rule, next, end, parts + 1) {
                return true;
            }
        }
        false
    }

    /// Is `w` correct when split at a `BREAK` point, both sides checked (each side may break
    /// again)? `^x` and `x$` take `x` off the start or the end.
    pub(super) fn check_breaks(&self, w: &str, deny: Mask, budget: &mut u32) -> bool {
        for b in &self.compound.breaks {
            if *budget == 0 {
                return false;
            }
            if let Some(x) = b.strip_prefix('^').filter(|x| !x.is_empty()) {
                if let Some(rest) = w.strip_prefix(x).filter(|r| !r.is_empty()) {
                    *budget -= 1;
                    if self.side_ok(rest, deny, budget) {
                        return true;
                    }
                }
                continue;
            }
            if let Some(x) = b.strip_suffix('$').filter(|x| !x.is_empty()) {
                if let Some(rest) = w.strip_suffix(x).filter(|r| !r.is_empty()) {
                    *budget -= 1;
                    if self.side_ok(rest, deny, budget) {
                        return true;
                    }
                }
                continue;
            }
            for (i, _) in w.match_indices(b.as_str()) {
                let (Some(left), Some(right)) = (w.get(..i), w.get(i + b.len()..)) else { continue };
                if left.is_empty() || right.is_empty() || *budget == 0 {
                    continue;
                }
                *budget -= 1;
                if self.side_ok(left, deny, budget) && self.side_ok(right, deny, budget) {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::super::AffixDict;

    /// A small dictionary in the shape of the Hungarian one, written for these tests.
    const AFF: &str = "SET UTF-8
FORBIDDENWORD W
COMPOUNDFLAG Y
COMPOUNDBEGIN B
COMPOUNDEND E
COMPOUNDPERMITFLAG P
COMPOUNDFORBIDFLAG F
ONLYINCOMPOUND O
NEEDAFFIX N
COMPOUNDROOT R
COMPOUNDMIN 2
COMPOUNDWORDMAX 2
COMPOUNDSYLLABLE 6 aáeéiíoóöőuúüű
CHECKCOMPOUNDDUP
CHECKCOMPOUNDTRIPLE
CHECKCOMPOUNDCASE
CHECKCOMPOUNDREP
CHECKCOMPOUNDPATTERN 1
CHECKCOMPOUNDPATTERN ssz sz
COMPOUNDRULE 1
COMPOUNDRULE n*m
BREAK 3
BREAK -
BREAK ^-
BREAK -féle$
REP 1
REP jl lj
SFX s Y 2
SFX s 0 ban .
SFX s 0 ok .
SFX d Y 1
SFX d 0 ó/YPs .
SFX i Y 1
SFX i 0 i/F .
";

    const DIC: &str = "40
alma/Ys
fa/Ys
ház/Ysi
tető/Ys
kulcs/Ys
haj/Y
lak/Y
haljak
kis/B
szék/Ys
sakk/Y
kör/Y
háztető/YRs
mikro/OY
vár/Yd
üzelm/Ns
baj/N
baj/Ys
lég/Y
záró/Y
légzáró/s
szezon/Y
rossz/Y
Petőfi/Y
híd/Y
Petőfi-híd/W
almaház/W
SMS
SMS-ben
ipso facto\t1
1/n
9/n
9-es/m
";

    fn dict() -> AffixDict {
        AffixDict::from_hunspell(AFF, DIC, &["test data"]).unwrap()
    }

    fn assert_words(d: &AffixDict, good: &[&str], bad: &[&str]) {
        for w in good {
            assert!(d.check(w), "{w} should be correct");
        }
        for w in bad {
            assert!(!d.check(w), "{w} should be wrong");
        }
    }

    #[test]
    fn compound_flags_and_places() {
        let d = dict();
        assert_words(&d, &["almafa", "almafaban", "házkulcs", "kisház", "almafaház", "Almafa", "ALMAFA"], &["házkis", "almafával", "kulcsx"]);
    }

    #[test]
    fn six_three_rule_counts_syllables_without_inflection() {
        let d = dict();
        // Four words: alma + tető + kulcs + alma has 7 syllables; with the inflection `ban` on
        // the last part, 6 of 7 count.
        assert_words(&d, &["almatetőkulcsház", "almatetőkulcsfaban", "háztetőalma"], &["almatetőkulcsalma"]);
    }

    #[test]
    fn boundaries() {
        let d = dict();
        assert_words(
            &d,
            &["sakkház", "rosszház", "hajház", "lakhaj", "szezonlég", "légzáró"],
            // Duplicate, triple letter, capital at a boundary, `ssz`|`sz`, a REP misspelling of
            // `haljak`, and `lég` + `záró`, which is the listed (non-compounding) `légzáró`.
            &["almaalma", "sakkkör", "almaHáz", "rosszszék", "hajlak", "hajlakház", "szezonlégzáró"],
        );
    }

    #[test]
    fn affixes_inside_compounds() {
        let d = dict();
        // `ó` carries the compound flag and the permit flag; `ban` neither; `i` forbids the
        // first and middle places but may end a compound.
        assert_words(&d, &["váró", "váróház", "házi", "almaházi"], &["házbanalma", "háziház"]);
        // A part carries one suffix: `váróban` (vár + ó + ban) is a word, but not a compound part.
        assert!(d.check("váróban"));
        assert!(!d.check("almaváróban"));
        let more = AffixDict::from_hunspell(&format!("{AFF}COMPOUNDMORESUFFIXES\n"), DIC, &[]).unwrap();
        assert!(more.check("almaváróban"));
    }

    #[test]
    fn only_in_compound_and_need_affix() {
        let d = dict();
        assert_words(&d, &["mikroház", "üzelmok", "baj", "bajok"], &["mikro", "üzelm"]);
    }

    #[test]
    fn forbidden_words_stay_wrong() {
        let d = dict();
        assert_words(&d, &["Petőfi-ház", "PETŐFI-HÁZ", "házalma"], &["almaház", "Petőfi-híd", "PETŐFI-HÍD"]);
    }

    #[test]
    fn compound_rules_and_breaks() {
        let d = dict();
        assert_words(
            &d,
            &["1999-es", "19-es", "alma-ház", "-alma", "alma-féle", "alma-ház-tető", "SMS-ben", "SMS-BEN"],
            &["1999", "19x", "alma-xyz", "alma-", "SMS-BAN"],
        );
    }

    #[test]
    fn dictionary_phrases_are_not_words() {
        let d = dict();
        assert!(!d.check("ipso") && !d.check("facto"));
    }

    #[test]
    fn listed_inflections_count_as_their_lemma() {
        // `levelek` is listed with its lemma `levél`: its plural ending is inflection.
        let dic = |morph: &str| format!("4\ntető/Y\nfa/Y\nkulcs/Y\nlevelek/Y{morph}\n");
        let with = AffixDict::from_hunspell(AFF, &dic("\tst:levél ts:PLUR"), &[]).unwrap();
        let without = AffixDict::from_hunspell(AFF, &dic(""), &[]).unwrap();
        let derived = AffixDict::from_hunspell(AFF, &dic("\tst:levél ds:x"), &[]).unwrap();
        assert!(with.check("tetőfakulcslevelek"), "6 of 7 syllables count");
        assert!(!without.check("tetőfakulcslevelek") && !derived.check("tetőfakulcslevelek"));
        let back = AffixDict::from_bytes(&with.to_bytes()).unwrap();
        assert!(back.check("tetőfakulcslevelek"));
    }

    #[test]
    fn compact_format_keeps_compounding() {
        let d = dict();
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert_words(
            &back,
            &["almafa", "kisház", "váróház", "mikroház", "1999-es", "alma-ház", "Petőfi-ház", "almatetőkulcsfaban"],
            &["házkis", "sakkkör", "hajlak", "mikro", "üzelm", "Petőfi-híd", "almaház", "almatetőkulcsalma"],
        );
    }

    #[test]
    fn dictionaries_without_compounding_are_unchanged() {
        // No compound flags, no BREAK: neither compounds nor hyphen splits are accepted.
        let d = AffixDict::from_hunspell("SET UTF-8\nSFX a Y 1\nSFX a 0 y .\n", "2\nkot/a\npies/a\n", &[]).unwrap();
        assert!(d.check("koty") && !d.check("kotpies") && !d.check("kot-pies"));
    }

    proptest::proptest! {
        #[test]
        fn compound_lookups_never_panic(s in "[a-zőüöáéí -]{0,60}") {
            let d = dict();
            let _ = d.check(&s);
            let _ = d.check(&s.to_uppercase());
        }
    }
}
