//! Compound words and word breaks, as Hunspell's affix files describe them.
//!
//! Languages such as Hungarian write compounds as one word (`almafa`, `házkulcsban`), so a
//! dictionary lists the parts and marks which may join:
//!
//! - **Compound flags.** A part needs `COMPOUNDFLAG` or the flag for its place (`COMPOUNDBEGIN`,
//!   `COMPOUNDMIDDLE`, `COMPOUNDEND`), on the stem or on an affix's continuation classes. A
//!   prefix may start a compound and a suffix may end it; a prefix on the last part needs
//!   `COMPOUNDPERMITFLAG`, a suffix before the end that or a compound flag of its own.
//!   `COMPOUNDFORBIDFLAG`, on a stem or a suffix, keeps the word out of the first and middle
//!   places (`nárciszmetrózik` ends in one). `ONLYINCOMPOUND` stems and affixes appear only
//!   inside compounds; parts are at least `COMPOUNDMIN` letters (default 3), and a part before
//!   the last carries at most one suffix (two with `COMPOUNDMORESUFFIXES`). Homonyms (`szán` the
//!   noun and the verb) are separate entries, as in Hunspell.
//! - **Size.** At most `COMPOUNDWORDMAX` words (a `COMPOUNDROOT` stem, itself a compound, counts
//!   as two). Past that a compound is still correct when it has at most the `COMPOUNDSYLLABLE`
//!   count of syllables: Hungarian's 6–3 rule, where a compound of more than two words and more
//!   than six syllables takes a hyphen. As in Hunspell, the compound's syllables are counted only
//!   in a Hungarian dictionary (`LANG hu` or `hu_HU`; elsewhere only those of a bare last stem),
//!   and there a prefix of two syllables or more on any part counts as a word. Syllables are the
//!   vowels `COMPOUNDSYLLABLE` lists. The last part's outer suffix doesn't count when it is an
//!   inflection (no continuation classes); with `SYLLABLENUM` given, Hunspell's Hungarian flags
//!   add syllables back (see `uncounted_syllables`). An inflected form listed as a stem counts
//!   as its lemma (`st:` in its morphological fields: `falevelek` as `falevél`).
//! - **Boundaries.** `CHECKCOMPOUNDDUP` (the last two parts not one dictionary entry, its forms
//!   included), `CHECKCOMPOUNDTRIPLE` (no
//!   letter three times across a boundary: `sakkkör`), `CHECKCOMPOUNDCASE` (no capital at a
//!   boundary), `CHECKCOMPOUNDPATTERN` (no part ending in one string, or with `0` a stem as
//!   listed, followed by one starting with another; a simplified spelling the pattern may name
//!   isn't accepted) and `CHECKCOMPOUNDREP` (no compound, and no two neighbouring parts, that is a
//!   `REP` misspelling of a word: `hajlak` for `haljak`). Two neighbouring parts that together
//!   are a listed word are read as that word (`szezon` + `légzáró`, never `lég` + `záró`).
//! - **`COMPOUNDRULE`**: patterns of flags with `*` and `?` (`0*1`: numbers such as
//!   `1999-ben`), matched by stems of at least `COMPOUNDMIN` letters, as many as the word holds,
//!   the last of which may carry suffixes.
//! - **`BREAK`**: strings at which a word is split and both sides checked like words, each in its
//!   own letter case (`Petőfi-vers`, `alma-Fa`; a word in capitals is split capitalized); `^`
//!   and `$` anchor one to the start or the end (`-féle$`). A word with ten break points or more
//!   is wrong, and a reading that is a listed `FORBIDDENWORD` isn't split, as in Hunspell; in a
//!   Hungarian dictionary the left side of a hyphen may be listed with it (`vegyiáru-`). Only
//!   dictionaries that declare
//!   `BREAK` get breaks; Hunspell's built-in default (break at hyphens) isn't applied, so the
//!   languages that check hyphenated words themselves keep doing so.
//!
//! `NEEDAFFIX` stems are words only with an affix; it is read here with the other flags of this
//! family. Searches are bounded: at most [`MAX_PARTS`] parts (`COMPOUNDRULE`: the word's length)
//! and [`PART_BUDGET`] part and `REP` lookups per search (a state that failed once isn't searched
//! again), [`WORD_PART_BUDGET`] for all the searches of one checked word (its case readings and
//! break sides), and [`BREAK_BUDGET`] checks for the breaks of one word, each side checked once.

use super::{AffixDict, Mask, Parse, Rule};
use crate::patterns::FxHasher;
use std::hash::{Hash, Hasher};

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
    "LANG",
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
/// A word with this many break points or more is wrong, as in Hunspell (`fa-fa-…` with ten
/// hyphens).
const MAX_BREAK_POINTS: usize = 10;

/// Most part lookups for all the compound searches of one checked word (its case readings and
/// break sides together); each search gets at most [`PART_BUDGET`] of them.
pub(super) const WORD_PART_BUDGET: u32 = 16 * PART_BUDGET;

/// The checks of one word: the break budget left, the sides already checked, the part lookups
/// left for the word, and whether a search ran out of them (its "wrong" isn't remembered).
pub(super) struct Breaks {
    budget: u32,
    pub(super) parts: u32,
    pub(super) exhausted: bool,
    pub(super) seen: crate::patterns::FxMap<String, bool>,
}

impl Breaks {
    pub(super) fn new() -> Breaks {
        Breaks { budget: BREAK_BUDGET, parts: WORD_PART_BUDGET, exhausted: false, seen: crate::patterns::FxMap::default() }
    }
}

/// Private-use flags the reader adds to an inflected form listed as a stem: one, two or three
/// syllables more than its lemma (`st:` in its morphology), which the 6–3 rule doesn't count.
const LEMMA_FLAGS: [char; 3] = ['\u{F8F1}', '\u{F8F2}', '\u{F8F3}'];

/// The flag for a stem `extra` syllables longer than its lemma (none for 0).
pub(super) fn lemma_flag(extra: usize) -> Option<char> {
    LEMMA_FLAGS.get(extra.checked_sub(1)?.min(LEMMA_FLAGS.len() - 1)).copied()
}

/// The syllables of `s`: its letters, in lower case, that are among `vowels`.
pub(super) fn count_syllables(vowels: &[char], s: &str) -> usize {
    s.chars().filter(|ch| vowels.contains(&ch.to_lowercase().next().unwrap_or(*ch))).count()
}

/// The vowels of `COMPOUNDSYLLABLE`, when the dictionary counts syllables.
pub(super) fn syllable_vowels(lines: &[String]) -> Option<Vec<char>> {
    lines.iter().find_map(|l| {
        let mut f = l.split(' ');
        (f.next() == Some("COMPOUNDSYLLABLE")).then(|| f.nth(1).unwrap_or("").chars().collect())
    })
}

/// Skips the count line of each table directive: its first line, when that is a number (a
/// trailing comment may follow, as in Hunspell: `BREAK 3 # breaks`).
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
/// starting with `begin` (with `begin_flag`). An `end` of `0` stands for a part that is a stem
/// as listed, without affixes.
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
    /// `SYLLABLENUM` is given: only that counts, as in Hunspell, which then applies its Hungarian
    /// flags (`c`, `J`, `I`; [`AffixDict::uncounted_syllables`]).
    syllable_num: bool,
    /// The bit of the flag `J`, for Hunspell's Hungarian `SYLLABLENUM` rule.
    j: Mask,
    /// The bits of the flags `F`, `G` and `H`: in a Hungarian dictionary such a stem (`forgalmi`,
    /// `belföldi`, `sokszínű`) may start a compound before a hyphen (`forgalmiadó-hiány`), as with the
    /// Hunspell command line.
    fused: Mask,
    /// [`LEMMA_FLAGS`]' bits.
    lemma: [Mask; 3],
    check_dup: bool,
    check_rep: bool,
    check_case: bool,
    check_triple: bool,
    patterns: Vec<Pattern>,
    rules: Vec<Vec<(Mask, Repeat)>>,
    breaks: Vec<String>,
    /// `LANG hu` or `hu_HU`: the dictionary is Hungarian (see [`AffixDict::part_ok`]).
    hungarian: bool,
    /// The text before the first space of each stem that has one (`vasbeton` of `vasbeton fal`):
    /// parts that read as such a stem are no compound ([`AffixDict::word_pair`]).
    pub(super) pair_heads: std::collections::HashSet<String>,
    /// The longest of `pair_heads`, in chars.
    pub(super) pair_head_chars: usize,
    /// The last three letters (or fewer, of a shorter one) of each of `pair_heads`.
    pub(super) pair_head_ends: std::collections::HashSet<String>,
    /// The first four letters (or fewer) of each stem with a space, the space taken out, no
    /// further than a suffix can't strip.
    pub(super) pair_starts: std::collections::HashSet<String>,
    /// Some stem with a space is too short for `pair_starts`: the start isn't tested first.
    pair_starts_all: bool,
    /// The first letter or two of each forbidden stem that may end a compound, itself or through
    /// a suffix (`ta` of `tanárúr`), no further than a
    /// suffix can't strip: only a rest starting so is read for one ([`AffixDict::rest_is_whole`]).
    forbidden_ends: std::collections::HashSet<String>,
    /// Some forbidden stem is too short for `forbidden_ends`: every rest is read.
    forbidden_ends_all: bool,
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
            syllable_num: false,
            j: Mask::EMPTY,
            fused: Mask::EMPTY,
            lemma: [Mask::EMPTY; 3],
            check_dup: false,
            check_rep: false,
            check_case: false,
            check_triple: false,
            patterns: Vec::new(),
            rules: Vec::new(),
            breaks: Vec::new(),
            hungarian: false,
            pair_heads: std::collections::HashSet::new(),
            pair_head_chars: 0,
            pair_head_ends: std::collections::HashSet::new(),
            pair_starts: std::collections::HashSet::new(),
            pair_starts_all: false,
            forbidden_ends: std::collections::HashSet::new(),
            forbidden_ends_all: false,
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

/// The text of the stem a parse of `text` reads, its affixes taken off.
fn stem_text(text: &str, p: &Parse<'_>) -> String {
    let mut s = text.to_string();
    for r in p.suffix_rules() {
        if let Some(base) = s.strip_suffix(&*r.add) {
            s = format!("{base}{}", r.strip);
        }
    }
    if let Some(r) = p.prefix
        && let Some(rest) = s.strip_prefix(&*r.add)
    {
        s = format!("{}{rest}", r.strip);
    }
    s
}

impl Compounding {
    /// The flags with which a part may end a compound.
    pub(super) fn ends(&self) -> Mask {
        self.flag | self.end
    }

    /// Index the stems with a space, `(text before the space, the stem without it, the most
    /// letters its suffixes strip)`, prefixed forms that strip letters included, and the
    /// forbidden stems that may end a compound, `(stem, the most letters its suffixes strip)`.
    pub(super) fn index_words(&mut self, spaced: &[(String, String, usize)], forbidden: &[(String, usize)]) {
        let opening = |w: &str, strip: usize, most: usize| -> Option<String> {
            let n = w.chars().count().saturating_sub(strip).min(most);
            (n > 0).then(|| w.chars().take(n).collect())
        };
        for (head, joined, strip) in spaced {
            self.pair_heads.insert(head.clone());
            self.pair_head_ends.insert(head.chars().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect());
            match opening(joined, *strip, 4) {
                Some(o) => {
                    self.pair_starts.insert(o);
                }
                None => self.pair_starts_all = true,
            }
        }
        self.pair_head_chars = self.pair_heads.iter().map(|h| h.chars().count()).max().unwrap_or(0);
        for (w, strip) in forbidden {
            match opening(w, *strip, 2) {
                Some(o) => {
                    self.forbidden_ends.insert(o);
                }
                None => self.forbidden_ends_all = true,
            }
        }
    }

    /// Read the directive lines kept by [`TableHeaders`].
    pub(super) fn from_lines(lines: &[String], bit_of: &dyn Fn(char) -> Mask) -> Result<Compounding, String> {
        let mut c =
            Compounding { lemma: LEMMA_FLAGS.map(bit_of), j: bit_of('J'), fused: bit_of('F') | bit_of('G') | bit_of('H'), ..Compounding::default() };
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
                "SYLLABLENUM" => c.syllable_num = !at(1).is_empty(),
                "CHECKCOMPOUNDDUP" => c.check_dup = true,
                "CHECKCOMPOUNDREP" => c.check_rep = true,
                "CHECKCOMPOUNDCASE" => c.check_case = true,
                "CHECKCOMPOUNDTRIPLE" => c.check_triple = true,
                // A third field, a simplified spelling the compound may take instead, isn't
                // applied: the spelling the pattern forbids stays wrong, as in Hunspell. Fields
                // after it (a comment) don't count.
                "CHECKCOMPOUNDPATTERN" if f.len() >= 3 => {
                    let (end, end_flag) = split_flag(at(1), bit_of);
                    let (begin, begin_flag) = split_flag(at(2), bit_of);
                    c.patterns.push(Pattern { end, end_flag, begin, begin_flag });
                }
                "COMPOUNDRULE" => c.rules.push(parse_rule(at(1), bit_of)?),
                "BREAK" if !at(1).is_empty() => c.breaks.push(at(1).to_string()),
                // Hunspell knows Hungarian by these two names only (`hu_RO` isn't).
                "LANG" => c.hungarian = matches!(at(1), "hu" | "hu_HU"),
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
    /// No suffix: the stem ends where the part does (`CHECKCOMPOUNDPATTERN 0`).
    suffixless: bool,
    /// What a last part adds to the 6–3 rule's count, the smaller the better (0 elsewhere): in a
    /// Hungarian dictionary minus its uncounted syllables ([`AffixDict::uncounted_syllables`]),
    /// in another the syllables of a bare stem (the only ones Hunspell counts there).
    counted: isize,
    /// For `CHECKCOMPOUNDDUP`, the hash of the dictionary entry (stem text and flags) of its
    /// first reading in Hunspell's order: the bare stem, else the affixed reading `part` picks.
    entry: u64,
    /// With a bare first reading, the entry of the first affixed one, which Hunspell reads
    /// instead when the bare one repeats the part before (`lakó` as `lakik` + `ó`).
    fallback: Option<u64>,
}

/// One compound search: the word, its char boundaries, the parts looked up so far and the
/// states that led nowhere.
struct Search<'w> {
    w: &'w str,
    /// Byte offsets of the char boundaries, `0` and `w.len()` included.
    bounds: Vec<usize>,
    parts: crate::patterns::FxMap<(usize, usize, Place), Option<Part>>,
    /// `(start, start of the previous part, words so far)` of each [`AffixDict::compound_from`]
    /// call that failed: the rest of the word can't be found from there again.
    failed: crate::patterns::FxMap<(usize, usize, usize), ()>,
    budget: u32,
    deny: Mask,
    /// Only compounds of two parts.
    two_only: bool,
    /// The side before a hyphen in a Hungarian dictionary: an `F`, `G` or `H` stem may start the
    /// compound, a first part with the compound flag keeps any suffix, no 6–3 rule.
    dash: bool,
    /// Per boundary: may only a last part start there ([`AffixDict::rest_is_whole`])?
    whole_rests: crate::patterns::FxMap<usize, bool>,
}

impl AffixDict {
    /// Is `w` (normalized, case as written) a compound word?
    pub(super) fn compound_word(&self, w: &str, deny: Mask, budget: &mut u32) -> bool {
        self.compound_search(w, deny, budget, true)
    }

    /// [`AffixDict::compound_word`]; `two_words`: a word that splits into two words of its own
    /// is a compound only as those two.
    fn compound_search(&self, w: &str, deny: Mask, budget: &mut u32, two_words: bool) -> bool {
        let c = &self.compound;
        if (!c.enabled() && c.rules.is_empty()) || w.len() > super::MAX_WORD_BYTES {
            return false;
        }
        let bounds: Vec<usize> = w.char_indices().map(|(i, _)| i).chain(std::iter::once(w.len())).collect();
        if bounds.len() < 3 {
            return false;
        }
        if *budget == 0 {
            return false;
        }
        let mut s = Search {
            w,
            bounds,
            parts: Default::default(),
            failed: Default::default(),
            budget: *budget,
            deny,
            two_only: false,
            dash: !two_words && c.hungarian,
            whole_rests: Default::default(),
        };
        let mut found = c.enabled() && self.compound_from(&mut s, 0, 0, None);
        // A word that splits into two words of its own (`műanyag` + `cső`) is a compound only
        // as those two, as with the Hunspell command line: `műanyagcső` is wrong while
        // `mű` + `anyag` + `cső` would parse, but `műanyagcsőgyár` is right.
        // A compound of two parts is right either way, and the cheaper search: the split into
        // two words is looked for only without one.
        if found && two_words {
            s.two_only = true;
            s.failed.clear();
            found = self.compound_from(&mut s, 0, 0, None) || !self.two_words(w, deny);
        }
        let found = found || c.rules.iter().any(|r| self.rule_from(&mut s, r, 0, 0, 0));
        *budget = s.budget;
        found
    }

    /// Does `w` split into two words of their own (a listed stem or an affixed form, not
    /// forbidden), anywhere?
    fn two_words(&self, w: &str, deny: Mask) -> bool {
        let word = |t: &str| self.simple_reading(t, deny) == Some(super::Reading::Word);
        w.char_indices().skip(1).any(|(i, _)| w.get(..i).is_some_and(word) && w.get(i..).is_some_and(word))
    }

    /// Can `s.w` from boundary `start` on be the rest of a compound that has `words` words so
    /// far, the last of them `prev` (byte range and part)?
    fn compound_from(&self, s: &mut Search<'_>, start: usize, words: usize, prev: Option<(usize, usize, Part)>) -> bool {
        if words >= MAX_PARTS {
            return false;
        }
        // The previous part ends at `start`, so its start and `words` decide what may follow.
        let state = (start, prev.map_or(usize::MAX, |(a, _, _)| a), words);
        if s.failed.contains_key(&state) {
            return false;
        }
        let found = self.compound_rest(s, start, words, prev);
        if !found {
            s.failed.insert(state, ());
        }
        found
    }

    /// [`AffixDict::compound_from`] without the record of failed states.
    fn compound_rest(&self, s: &mut Search<'_>, start: usize, words: usize, prev: Option<(usize, usize, Part)>) -> bool {
        let c = &self.compound;
        let last_bound = s.bounds.len() - 1;
        for end in (start + c.min)..=last_bound {
            if s.budget == 0 {
                return false;
            }
            let is_last = end == last_bound;
            if start == 0 && is_last {
                continue;
            }
            if !is_last && (last_bound - end < c.min || (s.two_only && start > 0)) {
                continue;
            }
            // A rest already found whole takes no more parts from here.
            if !is_last && s.whole_rests.get(&start) == Some(&true) {
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
            // Every candidate costs one unit, looked up before or not, so the cache can't let a
            // long word try split after split for free.
            s.budget = s.budget.saturating_sub(1);
            let Some(part) = self.part(s, a, b, place) else { continue };
            if !is_last && self.rest_is_whole(s, start) {
                continue;
            }
            if let Some((pa, pb, prev_part)) = prev
                && !self.boundary_ok(s.w, (pa, pb, prev_part), (a, b, part), is_last, &mut s.budget)
            {
                continue;
            }
            let total = words + part.words;
            if is_last {
                if self.compound_done(s.w, total, part, s.dash, &mut s.budget) {
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
        let text = s.w.get(a..b)?;
        let c = &self.compound;
        let mut best: Option<Part> = None;
        let deny = s.deny;
        // Before a hyphen a suffix stays on the first part only if its text isn't a stem of its
        // own (`ácsi` is listed: `ácsiház-` is wrong).
        let index = self.stem_index(text);
        let listed = index.is_some();
        // For `CHECKCOMPOUNDDUP`, the dictionary entries the part reads as: a stem with the
        // homonym's flags (`barát` + `ság` and `barát` + `jára` are two entries in hu_HU).
        let entry = |stem: &str, m: Mask| -> u64 {
            let mut h = FxHasher::default();
            (stem, m).hash(&mut h);
            h.finish()
        };
        let dash_first = s.dash && place == Place::First;
        let lift = dash_first && !listed;
        // A bare stem flagged `F`, `G` or `H` starts one there, as its first entry in the dictionary
        // says (a plain homonym listed before it stops it: `évi`).
        let fused = dash_first && self.entries(index).next().is_some_and(|m| m.meets(c.fused));
        self.parses(text, &mut |p| {
            if !self.part_ok(p, place, deny, fused, lift, dash_first) {
                return false;
            }
            // In a Hungarian dictionary a prefix of more than one syllable counts as a word of its
            // own, on any part, as in Hunspell (`kerekasztal` + `elő` + `döntőbíró`).
            let long_prefix = c.hungarian && p.prefix.is_some_and(|r| self.syllables(&r.add) > 1);
            let words = if p.stem.meets(c.root) { 2 } else { 1 } + usize::from(long_prefix);
            let last = place == Place::Last;
            let counted = match (last, c.hungarian) {
                (false, _) => 0,
                (true, true) => self.uncounted_syllables(p).saturating_neg(),
                (true, false) if p.affixed() => 0,
                (true, false) => isize::try_from(self.syllables(text)).unwrap_or(isize::MAX),
            };
            if best.is_none_or(|q| (words, counted) < (q.words, q.counted)) {
                best = Some(Part { words, stem: p.stem, suffixless: p.suffixes[0].is_none(), counted, entry: 0, fallback: None });
            }
            // The last part may yet have a reading that adds fewer syllables to the count.
            words == 1 && place != Place::Last
        });
        // The entries in Hunspell's order: a bare listed text is itself, an affixed reading its
        // stem (`vaskor/Y` or `vas` + `kor`; a bare `kamatos` that can't stand there is `kamat`
        // + `os`); the shorter affix first (`bolyh` + `oz` before `boly` + `hoz`).
        if c.check_dup
            && let Some(part) = best.as_mut()
        {
            // Hunspell takes the first homonym that may stand there, then the first affixed
            // reading with the compound flag (`bar/Y` + `ra` before `barr/x` + `a`), and only then
            // one with the flag of the place: on the last part in the order of a word, before it
            // suffixes first (`kihá` + `z` before `ki` + `ház`).
            let (mut bare, mut flagged, mut placed) = (None, None, [None; 4]);
            self.parses_in_steps(text, &mut |p| {
                if !self.part_ok(p, place, deny, fused, lift, dash_first) {
                    return false;
                }
                if !p.affixed() {
                    bare.get_or_insert_with(|| entry(text, p.stem));
                } else {
                    let step = match (place, p.prefix.is_some(), p.suffixes[1].is_some()) {
                        _ if (p.stem | p.checked_cont()).meets(c.flag) => &mut flagged,
                        (Place::Last, ..) => &mut placed[0],
                        (_, false, two) => &mut placed[usize::from(two)],
                        (_, true, two) => &mut placed[2 + usize::from(two)],
                    };
                    step.get_or_insert_with(|| entry(&stem_text(text, p), p.stem));
                }
                // The bare readings come first, and a part before the last needs its first only.
                flagged.is_some() || bare.is_some() && place != Place::Last || c.flag.is_empty() && placed[0].is_some()
            });
            let affixed = flagged.or_else(|| placed.into_iter().flatten().next());
            part.entry = bare.or(affixed).unwrap_or(0);
            part.fallback = if bare.is_some() { affixed } else { None };
        }
        s.parts.insert((a, b, place), best);
        best
    }

    /// May this parse stand at `place` in a compound?
    fn part_ok(&self, p: &Parse<'_>, place: Place, deny: Mask, fused: bool, lift: bool, dash_first: bool) -> bool {
        let c = &self.compound;
        if p.stem.meets(self.forbidden | deny) {
            return false;
        }
        if !p.affixed() && p.stem.meets(c.need_affix) || p.needs_affix(c.need_affix) {
            return false;
        }
        // Two suffixes in a row need `COMPOUNDMORESUFFIXES`, except on the last part, which
        // Hunspell checks like a word (`alma` + `váró` + `ban`).
        if place != Place::Last && p.suffixes[1].is_some() && !c.more_suffixes {
            return false;
        }
        // The flags count on the stem and on the affix Hunspell checks: the inner of two suffixes,
        // else the suffix, else the prefix (`kih` + `á` + `z/Y` has no compound flag).
        let roles = p.stem | p.checked_cont();
        let at_place = match place {
            Place::First => c.begin,
            Place::Middle => c.middle,
            Place::Last => c.end,
        };
        // Before a hyphen a bare stem flagged `F`, `G` or `H` may start a compound (`forgalmi`,
        // not `balról`), and a first part with the compound flag keeps any suffix (`jövedék` +
        // `i` + `adó-`); `COMPOUNDFORBIDFLAG` doesn't count on that first part, as with the
        // Hunspell command line.
        let fused_start = fused && !p.affixed();
        let lifted = lift && roles.meets(c.flag);
        if !roles.meets(c.flag | at_place) && !fused_start {
            return false;
        }
        if place != Place::Last && !dash_first && (p.stem.meets(c.forbid) || p.suffix_rules().any(|r| r.cont.meets(c.forbid))) {
            return false;
        }
        // Inside a compound a suffix needs `COMPOUNDPERMITFLAG`, unless it carries the compound
        // flag itself (`kedvez` + `ő` → `kedvezőpocak`); a prefix on the last part needs the
        // permit flag, as in Hunspell (`anyag` + `ki/Y` `szánt` is wrong, `anyag` + `be/P` `szánt`
        // is right), one on a middle part doesn't (`anyag` + `elszánt` + `ház`). Homonyms are separate parses, so the prefix flag of the verb `szán` doesn't
        // join the compound flag of the noun `szán` (`előanyag` + `elszánt` is wrong).
        let inside = c.permit | c.flag | at_place;
        if place == Place::Last && p.prefix.is_some_and(|r| !r.cont.meets(c.permit)) {
            return false;
        }

        if lifted {
            return true;
        }
        !(place != Place::Last && p.suffix_rules().any(|r: &Rule| !r.cont.meets(inside)))
    }

    /// Syllables of a parse's suffixes that the 6–3 rule doesn't count, as Hunspell counts them in
    /// a Hungarian dictionary (the only one where it counts them): the outer suffix's when it
    /// carries no continuation classes, an inflection (`hoz` of `végrehajt` + `ás` + `hoz`); and
    /// one for a suffix with them ending in `i` but not `ti` or `yi` (`-ási`), alone or under
    /// another (`-ási` + `ak`).
    /// Negative when `SYLLABLENUM` adds syllables back: two for a suffix flagged `c` (`-szerű`,
    /// `-fajta`), one for `J`, and one for `I` on a stem flagged `J`; the suffix is the outer one,
    /// or the one under it when the outer appends nothing.
    fn uncounted_syllables(&self, p: &Parse<'_>) -> isize {
        let c = &self.compound;
        let lemma = (1..).zip(c.lemma).find(|(_, m)| p.stem.meets(*m)).map_or(0, |(n, _)| n);
        let Some(outer) = p.suffixes[0] else { return lemma };
        let inflection = if outer.cont.is_empty() { isize::try_from(self.syllables(&outer.add)).unwrap_or(isize::MAX) } else { 0 };
        // The suffix Hunspell checks on its own: the inner of two, else a lone one with classes.
        let checked = p.suffixes[1].or(Some(outer).filter(|r| !r.cont.is_empty()));
        let ends_in_i = |r: &Rule| {
            let mut back = r.add.chars().rev();
            back.next() == Some('i') && !matches!(back.next(), Some('t' | 'y'))
        };
        let taken = inflection.saturating_add(isize::from(checked.is_some_and(ends_in_i)));
        // An outer suffix that appends nothing leaves the flag of the one under it, if any.
        let flagged = if outer.add.is_empty() { p.suffixes[1].filter(|r| !r.add.is_empty()) } else { Some(outer) };
        let added = match flagged.map(|r| r.flag) {
            _ if !c.syllable_num => 0,
            Some('c') => 2,
            Some('J') => 1,
            Some('I') if p.stem.meets(c.j) => 1,
            _ => 0,
        };
        lemma.saturating_add(taken).saturating_sub(added)
    }

    fn syllables(&self, s: &str) -> usize {
        count_syllables(&self.compound.vowels, s)
    }

    /// May part `(c, d)` follow part `(a, b)`? `CHECKCOMPOUNDREP` spends `budget`.
    fn boundary_ok(&self, w: &str, (a, b, prev): (usize, usize, Part), (c, d, next): (usize, usize, Part), last: bool, budget: &mut u32) -> bool {
        let cfg = &self.compound;
        let (Some(left), Some(right)) = (w.get(a..b), w.get(c..d)) else { return false };
        // The same entry as the last two parts, its forms too (`ház` + `házban`, `házban` +
        // `ház-`), as with the Hunspell command line; earlier pairs don't count (`házházfa`).
        // The last part is a duplicate of the part before when its first reading is the same
        // entry: an affixed one, or a bare one whose first affixed reading (if any) is it too.
        let repeats = next.entry == prev.entry && next.fallback.is_none_or(|f| f == prev.entry);
        if cfg.check_dup && last && repeats {
            return false;
        }
        let Some(joined) = w.get(a..d) else { return false };
        if cfg.check_rep && self.rep_misspelling(joined, budget) {
            return false;
        }
        if !cfg.pair_heads.is_empty() && self.word_pair(joined, budget) {
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
            // `0`: the previous part's stem ends at the boundary, prefixed or not (`el` `kis` +
            // `szék`), as in Hunspell.
            let end = if p.end == "0" {
                prev.suffixless && (p.end_flag.is_empty() || prev.stem.meets(p.end_flag))
            } else {
                left.ends_with(p.end.as_str()) && (p.end_flag.is_empty() || prev.stem.meets(p.end_flag))
            };
            end && right.starts_with(p.begin.as_str()) && (p.begin_flag.is_empty() || next.stem.meets(p.begin_flag))
        })
    }

    /// The checks on a whole compound of `words` words whose last part is `last`.
    fn compound_done(&self, w: &str, words: usize, last: Part, dash: bool, budget: &mut u32) -> bool {
        let c = &self.compound;
        // Before a hyphen the 6–3 rule doesn't apply (`almatetőkulcsalma-`).
        if let Some(max) = c.word_max
            && words > max
            && !dash
        {
            let Some(syllables) = c.syllable_max else { return false };
            // As in Hunspell, the compound's syllables count only in a Hungarian dictionary;
            // elsewhere only a bare last stem's own.
            let whole = if c.hungarian { isize::try_from(self.syllables(w)).unwrap_or(isize::MAX) } else { 0 };
            let counted = whole.saturating_add(last.counted);
            if counted > isize::try_from(syllables).unwrap_or(isize::MAX) {
                return false;
            }
        }
        !(c.check_rep && self.rep_misspelling(w, budget))
    }

    /// Is the rest of the word from boundary `start` a whole that no further parts may make up,
    /// as with the Hunspell command line? So it is when it reads as a stem with a space
    /// (`hajnyíró gép`: `hajnyírógép` is wrong) or as a forbidden word that may end a compound,
    /// by its own flags or the classes of the suffix right on it, a suffix after that aside
    /// (`tanárúr`: `háztanárúr` is wrong, `tanárúrház` right). Looked up once per boundary.
    fn rest_is_whole(&self, s: &mut Search<'_>, start: usize) -> bool {
        if let Some(&known) = s.whole_rests.get(&start) {
            return known;
        }
        let rest = s.bounds.get(start).and_then(|&a| s.w.get(a..)).unwrap_or_default();
        let c = &self.compound;
        // A forbidden stem without a prefix that may end a compound itself or through the suffix
        // right on it.
        let starts = |n: usize| rest.char_indices().nth(n).map_or(rest, |(k, _)| rest.get(..k).unwrap_or(rest));
        let opens = c.forbidden_ends_all || c.forbidden_ends.contains(starts(1)) || c.forbidden_ends.contains(starts(2));
        let forbidden_end =
            opens && self.parses(rest, &mut |p| p.prefix.is_none() && p.stem.meets(self.forbidden) && (p.stem | p.checked_cont()).meets(c.ends()));
        let whole = forbidden_end || (!c.pair_heads.is_empty() && self.word_pair(rest, &mut s.budget));
        s.whole_rests.insert(start, whole);
        whole
    }

    /// Is `w`, two adjacent parts, a word of the dictionary written with a space somewhere
    /// (`vasbeton fal`, `vasbeton falban`)? Then it is that word, written apart, and no compound,
    /// as with the Hunspell command line. Any listed stem or affixed reading counts, a prefixed
    /// one too (`ki` + `alma fa`). Only a position where the text before it ends in the first
    /// word of such a stem (after at most a prefix) is looked up, at one unit of `budget`; with
    /// none left, `w` counts as such a pair.
    fn word_pair(&self, w: &str, budget: &mut u32) -> bool {
        let c = &self.compound;
        // A cheap test first: such a stem starts the text, after at most a prefix.
        let opens = |(k, _): (usize, char)| {
            let rest = w.get(k..).unwrap_or_default();
            let ends = rest.char_indices().skip(1).map(|(e, _)| e).chain(std::iter::once(rest.len())).take(4);
            ends.filter_map(|e| rest.get(..e)).any(|t| c.pair_starts.contains(t))
        };
        if !c.pair_starts_all && !w.char_indices().take(self.max_pfx_chars + 1).any(opens) {
            return false;
        }
        let at: Vec<usize> = w.char_indices().map(|(k, _)| k).chain(std::iter::once(w.len())).collect();
        let chars = at.len() - 1;
        let mut candidate = String::with_capacity(w.len() + 1);
        for (n, &i) in at.iter().enumerate().take(chars).skip(1) {
            let (Some(head), Some(tail)) = (w.get(..i), w.get(i..)) else { continue };
            // A cheap test first: the text before the space ends as some first word does.
            let ends = (1..=3.min(n)).any(|k| at.get(n - k).and_then(|&e| head.get(e..)).is_some_and(|h| c.pair_head_ends.contains(h)));
            if !ends {
                continue;
            }
            // The first word starts after at most a prefix and is at most the longest head.
            let first = n.saturating_sub(c.pair_head_chars)..=self.max_pfx_chars.min(n - 1);
            if !first.into_iter().any(|j| at.get(j).and_then(|&k| head.get(k..)).is_some_and(|h| c.pair_heads.contains(h))) {
                continue;
            }
            candidate.clear();
            candidate.push_str(head);
            candidate.push(' ');
            if *budget == 0 {
                return true;
            }
            *budget -= 1;
            candidate.push_str(tail);
            if self.stem_mask(&candidate).is_some() || self.simple_reading(&candidate, Mask::EMPTY).is_some() {
                return true;
            }
        }
        false
    }

    /// Is `w` a `REP` misspelling of a word: does one replacement turn it into a word without
    /// compounding? Patterns tied to the start or the end (`^orr`, `r$`) don't count, as in
    /// Hunspell (`orrvadász` is right though `orvvadász` is a word). Each word looked up costs
    /// one unit of `budget`; with none left, `w` counts as a misspelling.
    fn rep_misspelling(&self, w: &str, budget: &mut u32) -> bool {
        for (from, to) in &self.rep {
            if from.is_empty() || from.starts_with('^') || from.ends_with('$') {
                continue;
            }
            for (i, _) in w.match_indices(from.as_str()) {
                let (Some(head), Some(tail)) = (w.get(..i), w.get(i + from.len()..)) else { continue };
                if *budget == 0 {
                    return true;
                }
                *budget -= 1;
                // Any listed stem or affixed reading counts, a forbidden one too, as in
                // Hunspell's `candidate_check`.
                let candidate = format!("{head}{to}{tail}");
                if self.stem_mask(&candidate).is_some() || self.simple_reading(&candidate, Mask::EMPTY).is_some() {
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
        // Parts are at least `COMPOUNDMIN` letters, as in Hunspell, and as many as the word
        // holds (`1111111111111-es`): each is shorter than the word, so the depth is bounded.
        if s.budget == 0 {
            return false;
        }
        if repeat != Repeat::One && self.rule_from(s, rule, at + 1, start, parts) {
            return true;
        }
        for end in (start + self.compound.min)..=last_bound {
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

    /// Is `w`, the side before a hyphen, a compound in one of its [`AffixDict::case_readings`]
    /// once these are allowed: a split into two words of its own, a first part with any suffix,
    /// a first part flagged `F`. In a Hungarian dictionary, as with the Hunspell command line:
    /// `közlésó` and `forgalmiadó` are wrong, `közlésó-`, `Közlésó-`, `forgalmiadó-hiány` and
    /// `jövedékiadó-fizetési` right.
    fn dash_side(&self, w: &str, deny: Mask, st: &mut Breaks) -> bool {
        if !self.compound.hungarian {
            return false;
        }
        for (v, case_deny) in self.case_readings(w) {
            if self.stem_mask(&v).is_some_and(|m| m.meets(self.forbidden)) || self.simple_reading(&v, deny | case_deny).is_some() {
                continue;
            }
            let given = PART_BUDGET.min(st.parts);
            let mut left = given;
            let found = self.compound_search(&v, deny | case_deny, &mut left, false);
            st.parts -= given - left;
            st.exhausted |= left == 0;
            if found {
                return true;
            }
        }
        false
    }

    /// Is `w` correct when split at a `BREAK` point, both sides checked (each side may break
    /// again)? `^x` and `x$` take `x` off the start or the end.
    pub(super) fn check_breaks(&self, w: &str, deny: Mask, st: &mut Breaks) -> bool {
        let breaks = &self.compound.breaks;
        let anchored = |b: &str| b.starts_with('^') || b.ends_with('$');
        let points: usize = breaks.iter().filter(|b| !b.is_empty() && !anchored(b)).map(|b| w.matches(b.as_str()).count()).sum();
        if points >= MAX_BREAK_POINTS {
            return false;
        }
        for b in breaks {
            if st.budget == 0 {
                return false;
            }
            if let Some(x) = b.strip_prefix('^').filter(|x| !x.is_empty()) {
                if let Some(rest) = w.strip_prefix(x).filter(|r| !r.is_empty()) {
                    st.budget -= 1;
                    if self.side_ok(rest, deny, st) {
                        return true;
                    }
                }
                continue;
            }
            if let Some(x) = b.strip_suffix('$').filter(|x| !x.is_empty()) {
                if let Some(rest) = w.strip_suffix(x).filter(|r| !r.is_empty()) {
                    st.budget -= 1;
                    if self.side_ok(rest, deny, st) || (x == "-" && self.dash_side(rest, deny, st)) {
                        return true;
                    }
                }
                continue;
            }
            for (i, _) in w.match_indices(b.as_str()) {
                let (Some(left), Some(right)) = (w.get(..i), w.get(i + b.len()..)) else { continue };
                if left.is_empty() || right.is_empty() || st.budget == 0 {
                    continue;
                }
                st.budget -= 1;
                // In a Hungarian dictionary the left side of a hyphen may also be a listed word
                // with the hyphen (`vegyiáru-` + `kereskedés`), as in Hunspell.
                let dash = || self.compound.hungarian && b == "-" && left.len() < super::MAX_WORD_BYTES;
                if self.side_ok(right, deny, st)
                    && (self.side_ok(left, deny, st) || (dash() && (self.side_ok(&format!("{left}-"), deny, st) || self.dash_side(left, deny, st))))
                {
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
LANG hu_HU
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
        // Two suffixes in a row (vár + ó + ban) need COMPOUNDMORESUFFIXES before the last part;
        // the last is checked like a word, as Hunspell does: `almaváróban` is correct.
        assert!(d.check("váróban") && d.check("almaváróban"));
        let more = AffixDict::from_hunspell(&format!("{AFF}COMPOUNDMORESUFFIXES\n"), DIC, &[]).unwrap();
        assert!(more.check("almaváróban"));
    }

    #[test]
    fn only_in_compound_and_need_affix() {
        let d = dict();
        assert_words(&d, &["mikroház", "üzelmok", "baj", "bajok"], &["mikro", "üzelm"]);
    }

    #[test]
    fn first_middle_and_last_places() {
        // `COMPOUNDFIRST` and `COMPOUNDLAST` are the older names of `COMPOUNDBEGIN` and
        // `COMPOUNDEND`; a `COMPOUNDMIDDLE` part sits only between two others.
        let dic = "3\nkis/B\nnagy/M\nház/E\n";
        for (first, last) in [("COMPOUNDBEGIN", "COMPOUNDEND"), ("COMPOUNDFIRST", "COMPOUNDLAST")] {
            let aff = format!("SET UTF-8\n{first} B\nCOMPOUNDMIDDLE M\n{last} E\n");
            let d = AffixDict::from_hunspell(&aff, dic, &[]).unwrap();
            assert_words(&d, &["kisház", "kisnagyház", "kisnagynagyház"], &["kisnagy", "nagyház", "házkis", "kiskis", "nagynagy"]);
        }
    }

    #[test]
    fn pseudoroot_is_needaffix() {
        let aff = AFF.replace("NEEDAFFIX N", "PSEUDOROOT N");
        let d = AffixDict::from_hunspell(&aff, DIC, &[]).unwrap();
        assert_words(&d, &["üzelmok", "üzelmban", "baj", "bajok"], &["üzelm"]);
    }

    /// The 6–3 rule's syllables as the Hunspell 1.7 command line counts them: three parts of 2, 1
    /// and 3 syllables, then a suffix.
    #[test]
    fn six_three_syllables_follow_hunspell() {
        let aff = |extra: &str| format!("SET UTF-8\nFLAG UTF-8\nLANG hu_HU\nCOMPOUNDFLAG Y\nCOMPOUNDWORDMAX 2\nCOMPOUNDSYLLABLE 6 aeiou\n{extra}");
        let word = |extra: &str, dic: &str, w: &str| AffixDict::from_hunspell(&aff(extra), dic, &[]).unwrap().check(w);
        // An inflection doesn't count, whatever SYLLABLENUM lists: only its flags c, J and I do.
        let one = "SFX S Y 1\nSFX S 0 ka .\n";
        assert!(!word(one, "3\ntata/Y\ntet/Y\ntototutu/YS\n", "tatatettototutuka"), "7 of 8 count");
        for extra in ["", "SYLLABLENUM S\n", "SYLLABLENUM m\n"] {
            assert!(word(&format!("{extra}{one}"), "3\ntata/Y\ntet/Y\ntototu/YS\n", "tatatettototuka"), "{extra:?}");
        }
        for (flag, stem, right) in [('c', "", false), ('J', "", false), ('I', "", true), ('I', "J", false)] {
            let extra = format!("SYLLABLENUM m\nSFX {flag} Y 1\nSFX {flag} 0 ka .\n");
            let dic = format!("3\ntata/Y\ntet/Y\ntototu/Y{flag}{stem}\n");
            assert_eq!(word(&extra, &dic, "tatatettototuka"), right, "{flag} on a stem with {stem:?}");
        }
        // Of two suffixes the outer inflection doesn't count; a suffix with continuation classes
        // counts, but one syllable of a lone one ending in `i` (not `ti` or `yi`) doesn't.
        let two = "SFX A Y 1\nSFX A 0 to/B .\nSFX B Y 1\nSFX B 0 ka .\nSFX T Y 1\nSFX T 0 ti/B .\nSFX E Y 1\nSFX E 0 si/B .\n";
        let dic = "4\ntata/Y\ntet/Y\ntotu/YA\ntototu/YTE\n";
        assert!(word(two, dic, "tatatettotutoka") && word(two, dic, "tatatettototusi"));
        assert!(!word(two, dic, "tatatettototuti"));
        assert!(word(two, "4\ntata/Y\ntet/Y\ntototu/YE\nx\n", "tatatettototusika"), "an inner -si leaves one out");
        // A suffix that appends nothing adds no syllable back.
        assert!(word("SYLLABLENUM m\nSFX c Y 1\nSFX c 0 0/Y .\n", "3\ntata/Y\ntet/Y\ntototu/c\n", "tatatettototu"));
        // Without a Hungarian LANG (`hu_RO` isn't one), only a bare last stem's syllables count.
        for lang in ["", "LANG hu_RO\n"] {
            let plain = |dic: &str, w: &str| AffixDict::from_hunspell(&aff("").replace("LANG hu_HU\n", lang), dic, &[]).unwrap().check(w);
            assert!(plain("3\ntata/Y\ntet/Y\ntototutu/Y\n", "tatatettototutu"), "{lang:?}");
            assert!(!plain("3\ntata/Y\ntet/Y\ntototututututu/Y\n", "tatatettototututututu"), "{lang:?}");
        }
        // A bare reading loses to an affixed one that counts fewer syllables.
        let two_readings = "SET UTF-8\nFLAG UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDWORDMAX 2\nCOMPOUNDSYLLABLE 6 aeiou\nSFX S Y 1\nSFX S 0 tutu/X .\nSFX X Y 1\nSFX X 0 ka .\n";
        let d = AffixDict::from_hunspell(two_readings, "4\ntata/Y\ntet/Y\ntototututututu/Y\ntototututu/YS\n", &[]).unwrap();
        assert!(d.check("tatatettototututututu"));
        // Under an outer suffix that appends nothing, the inner one's flag adds syllables back.
        let empty_outer = "SET UTF-8\nFLAG UTF-8\nLANG hu_HU\nCOMPOUNDFLAG Y\nCOMPOUNDWORDMAX 2\nCOMPOUNDSYLLABLE 6 aeiou\nSYLLABLENUM c\nNEEDAFFIX N\nSFX c Y 2\nSFX c 0 ka/cN .\nSFX c 0 0 .\n";
        assert!(!AffixDict::from_hunspell(empty_outer, "3\ntata/Y\ntet/Y\ntotu/Yc\n", &[]).unwrap().check("tatatettotuka"));
        let suffixed = "SET UTF-8\nFLAG UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDWORDMAX 2\nCOMPOUNDSYLLABLE 6 aeiou\nSFX S Y 1\nSFX S 0 ka .\n";
        assert!(AffixDict::from_hunspell(suffixed, "3\ntata/Y\ntet/Y\ntototututututu/YS\n", &[]).unwrap().check("tatatettototututututuka"));
    }

    /// A pair of parts the dictionary lists with a space is no compound, as with the Hunspell 1.7
    /// command line (`cpdwordpair_check`).
    #[test]
    fn a_pair_listed_with_a_space_is_no_compound() {
        let aff = "SET UTF-8\nFLAG UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDMIN 2\nSFX S Y 1\nSFX S 0 ban .\n";
        let d = |dic: &str| AffixDict::from_hunspell(aff, dic, &[]).unwrap();
        let plain = d("2\nalma/YS\nfa/YS\n");
        assert!(plain.check("almafa") && plain.check("almafaban"));
        let pair = d("4\nalma/YS\nfa/YS\nház/YS\nalma fa\n");
        assert!(!pair.check("almafa") && pair.check("almafaban") && pair.check("faalma"));
        assert!(!pair.check("házalmafa") && !pair.check("almafaház"), "anywhere in the compound");
        let inflected = d("3\nalma/YS\nfa/YS\nalma fa/S\n");
        assert!(!inflected.check("almafa") && !inflected.check("almafaban"));
        assert!(d("3\nalma/YS\nfa/YS\nalmafa\n").check("almafaban"), "a listed compound stays right");
    }

    /// A word that splits into two words of its own (anywhere, a short or affixed one too) is a
    /// compound only as those two, as with the Hunspell 1.7 command line; a forbidden word or a
    /// compound doesn't count as one of them.
    #[test]
    fn two_words_of_their_own_make_a_compound_only_as_two() {
        let aff = "SET UTF-8\nFLAG UTF-8\nFORBIDDENWORD !\nCOMPOUNDFLAG Y\nCOMPOUNDMIN 3\nSFX S Y 1\nSFX S 0 ot .\n";
        let parts = "bab/Y\ncec/Y\ndid/Y\nfof/Y\n";
        let check = |extra: &[&str], w: &str| {
            let dic = format!("{}\n{parts}{}\n", 4 + extra.len(), extra.join("\n"));
            AffixDict::from_hunspell(aff, &dic, &[]).unwrap().check(w)
        };
        assert!(check(&[], "babcecdid") && check(&[], "babcecdidfof"));
        // babcec, cecdid or babce + cdid (not at a boundary), ba + bcecdid (shorter than
        // COMPOUNDMIN), bab + cecdid: two words, and no compound of two.
        for extra in [&["babcec"][..], &["cecdid"], &["babce", "cdid"], &["ba", "bcecdid"], &["babce/Y", "cdid"], &["babce", "cdid/Y"]] {
            assert!(!check(extra, "babcecdid"), "{extra:?}");
        }
        assert!(check(&["babce/Y", "cdid/Y"], "babcecdid"), "a compound of the two");
        assert!(check(&["cecdid", "babcec/Y"], "babcecdid"), "bab + cecdid is no compound, babcec + did is");
        assert!(check(&["babce/!", "cdid"], "babcecdid"), "a forbidden word is none of the two");
        // In four parts a pair of them is no word of its own beside the rest.
        for extra in ["babcec", "cecdid", "didfof"] {
            assert!(check(&[extra], "babcecdidfof"), "{extra}");
        }
        assert!(!check(&["babcecdid"], "babcecdidfof") && !check(&["babcec", "didfof"], "babcecdidfof"));
        // An affixed word counts: darab + ot.
        let affixed = |w: &str| AffixDict::from_hunspell(aff, "4\ndara/Y\nbot/Y\nhaj/Y\ndarab/S\n", &[]).unwrap().check(w);
        assert!(affixed("darabot") && !affixed("darabothaj") && !affixed("hajdarabot"));
    }

    /// In a Hungarian dictionary the side before a hyphen may split into two words of its own:
    /// `darabothaj` is wrong, `darabothaj-` and `darabothaj-haj` are right.
    #[test]
    fn before_a_hyphen_two_words_of_their_own_may_compound() {
        let aff =
            "SET UTF-8\nFLAG UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDMIN 3\nSFX S Y 1\nSFX S 0 ot .\nWORDCHARS -\nBREAK 3\nBREAK -\nBREAK ^-\nBREAK -$\n";
        let dic = "4\ndara/Y\nbot/Y\nhaj/Y\ndarab/S\n";
        let hu = AffixDict::from_hunspell(&format!("{aff}LANG hu_HU\n"), dic, &[]).unwrap();
        assert!(!hu.check("darabothaj") && hu.check("darabothaj-") && hu.check("darabothaj-haj"));
        for w in ["Darabothaj-", "DARABOTHAJ-", "Darabothaj-haj", "DARABOTHAJ-HAJ"] {
            assert!(hu.check(w), "{w}");
        }
        assert!(!hu.check("Darabothaj"));
        let plain = AffixDict::from_hunspell(aff, dic, &[]).unwrap();
        assert!(!plain.check("darabothaj-") && plain.check("dara-"));
    }

    /// A stem with a space checks each part with the next and with the rest of the word, a space
    /// anywhere in them, a prefixed reading too; a run of parts in the middle isn't checked.
    #[test]
    fn a_stem_with_a_space_checks_each_part_with_the_next_and_the_rest() {
        let aff = "SET UTF-8\nFLAG UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDMIN 3\nPFX P Y 1\nPFX P 0 ki .\n";
        let check = |spaced: &str, w: &str| {
            AffixDict::from_hunspell(aff, &format!("6\nbab/YP\ncec/YP\ndid/YP\nfof/YP\ngug/YP\n{spaced}\n"), &[]).unwrap().check(w)
        };
        for spaced in ["bab cec", "cec did", "did fof", "cec didfof", "bab cecdidfof", "ba bcec"] {
            assert!(!check(spaced, "babcecdidfof"), "{spaced}");
        }
        assert!(check("bab cecdid", "babcecdidfof"), "two parts in the middle");
        assert!(check("cec didfof", "babcecdidfofgug"));
        assert!(!check("bab cec/P", "kibabcec"), "a prefixed reading");
    }

    /// A forbidden word that may end a compound ends the search there: `háztanárúr` is wrong
    /// (`ház` + `tanár` + `úr` would parse), `tanárúrház` and `háztanárúrkert` are right, as with
    /// the Hunspell 1.7 command line. Without the compound flag it stops nothing.
    #[test]
    fn a_forbidden_word_that_may_end_a_compound_ends_it_there() {
        let aff = "SET UTF-8\nFLAG UTF-8\nFORBIDDENWORD !\nCOMPOUNDFLAG Y\nCOMPOUNDMIN 2\nSFX S Y 1\nSFX S 0 ok .\n";
        let d = |flags: &str| AffixDict::from_hunspell(aff, &format!("5\nház/Y\ntanár/Y\núr/YS\nkert/Y\ntanárúr/{flags}\n"), &[]).unwrap();
        let ends = d("!Y");
        assert!(!ends.check("tanárúr") && !ends.check("háztanárúr"));
        assert!(ends.check("tanárúrház") && ends.check("háztanárúrkert") && ends.check("háztanárúrok"));
        let affixed = d("!YS");
        assert!(!affixed.check("háztanárúrok") && !affixed.check("tanárúrok"));
        let plain = d("!");
        assert!(!plain.check("tanárúr") && plain.check("háztanárúr"));
    }

    /// The quick tests before a forbidden end or a stem with a space miss none of them: a
    /// forbidden stem that compounds only through a suffix or is one letter long, a prefixed
    /// forbidden stem (no end), a suffix that strips the stem's start, a prefix that changes it.
    #[test]
    fn quick_tests_miss_no_forbidden_end_or_pair() {
        let check = |aff: &str, dic: &[&str], w: &str| {
            AffixDict::from_hunspell(&format!("SET UTF-8\nFLAG UTF-8\n{aff}"), &format!("{}\n{}\n", dic.len(), dic.join("\n")), &[]).unwrap().check(w)
        };
        let forbid = "FORBIDDENWORD !\nCOMPOUNDFLAG Y\n";
        assert!(!check(
            &format!("{forbid}COMPOUNDMIN 2\nSFX S Y 1\nSFX S 0 ok/Y .\n"),
            &["ház/Y", "tanár/Y", "úr/YS", "kert/Y", "tanárúr/!S", "kitanár/Y"],
            "háztanárúrok"
        ));
        assert!(!check(&format!("{forbid}COMPOUNDMIN 1\nSFX S Y 1\nSFX S 0 bab .\n"), &["ház/Y", "ób/Y", "ab/Y", "ó/!YS"], "házóbab"));
        assert!(check(
            &format!("{forbid}COMPOUNDMIN 2\nPFX P Y 1\nPFX P 0 ki .\n"),
            &["ház/Y", "tanár/YP", "úr/Y", "kert/Y", "tanárúr/!YP", "kifli/!Y"],
            "házkitanárúr"
        ));
        assert!(!check("COMPOUNDFLAG Y\nCOMPOUNDMIN 1\nSFX S Y 1\nSFX S a ák a\n", &["ló/Y", "fa/YS", "ló fa/S"], "lófák"));
        assert!(!check("COMPOUNDFLAG Y\nCOMPOUNDMIN 1\nPFX C Y 1\nPFX C B b B\n", &["bab/Y", "cec/Y", "Bab cec/C"], "babcec"));
        // The suffix on the stem decides, not one after it: `ok` + `ra/Y` doesn't end the
        // compound there, `ok/TY` + `ra` does.
        let two = |inner: &str, outer: &str| {
            let aff = format!("{forbid}COMPOUNDMIN 2\nSFX S Y 1\nSFX S 0 {inner} .\nSFX T Y 1\nSFX T 0 {outer} .\n");
            check(&aff, &["ház/Y", "tanár/Y", "úr/YS", "kert/Y", "tanárúr/!S", "tanya/!Y"], "háztanárúrokra")
        };
        assert!(two("ok/T", "ra/Y") && !two("ok/TY", "ra"));
        // Before a hyphen in any letter case, a forbidden capitalized reading aside.
        let aff = "LANG hu_HU\nFORBIDDENWORD !\nCOMPOUNDFLAG Y\nCOMPOUNDMIN 3\nWORDCHARS -\nBREAK 3\nBREAK -\nBREAK ^-\nBREAK -$\n";
        for w in ["Babcecdid-", "BABCECDID-", "Babcecdid-did"] {
            assert!(check(aff, &["bab/Y", "cec/Y", "did/Y", "Babcecdid/!"], w), "{w}");
        }
    }

    /// In a Hungarian dictionary a stem flagged `F` (`forgalmi`) may start a compound before a
    /// hyphen, as with the Hunspell 1.7 command line: `pirosalma` is wrong, `pirosalma-`,
    /// `pirosalmafa-` and `pirosalma-fa` are right; it may not stand later in one.
    #[test]
    fn a_stem_flagged_f_starts_a_compound_before_a_hyphen() {
        let aff =
            "SET UTF-8\nFLAG UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDMIN 1\nWORDCHARS -\nBREAK 3\nBREAK -\nBREAK ^-\nBREAK -$\nSFX S Y 1\nSFX S 0 ban .\n";
        let dic = "3\npiros/F\nalma/YS\nfa/YS\n";
        let hu = AffixDict::from_hunspell(&format!("{aff}LANG hu_HU\n"), dic, &[]).unwrap();
        for w in ["pirosalma-", "pirosalma-fa", "pirosalma-faban", "pirosalmaban-", "pirosalmafa-", "piros-"] {
            assert!(hu.check(w), "{w}");
        }
        for w in ["pirosalma", "fapirosalma-", "almapiros-"] {
            assert!(!hu.check(w), "{w}");
        }
        // Only bare: an `F` stem with a suffix doesn't start one.
        let aff_s = format!("{aff}LANG hu_HU\nSFX B Y 1\nSFX B 0 ból .\n");
        let bal = AffixDict::from_hunspell(&aff_s, "3\nbal/FB\nház/YB\nkert/Y\n", &[]).unwrap();
        assert!(bal.check("balház-") && bal.check("házbólkert-") && !bal.check("balbólház-"));
        // `G` does the same; the stem's first entry decides.
        let order = |dic: &str| AffixDict::from_hunspell(&format!("{aff}LANG hu_HU\n"), dic, &[]).unwrap().check("pirosalma-");
        assert!(order("3\npiros/G\nalma/Y\nfa/Y\n") && order("4\npiros/G\npiros\nalma/Y\nfa/Y\n"));
        assert!(!order("4\npiros\npiros/G\nalma/Y\nfa/Y\n") && !order("4\npiros\npiros/F\nalma/Y\nfa/Y\n"));
        let plain = AffixDict::from_hunspell(aff, dic, &[]).unwrap();
        assert!(!plain.check("pirosalma-") && !plain.check("pirosalma-fa"));
        let no_flag = AffixDict::from_hunspell(&format!("{aff}LANG hu_HU\n"), "3\npiros/F\nalma/S\nfa/YS\n", &[]).unwrap();
        assert!(!no_flag.check("pirosalma-"), "the next part needs the compound flag");
        // The first part keeps any suffix there, a later one doesn't.
        let suffixed = |stem: &str| {
            let aff = format!("{aff}LANG hu_HU\nCOMPOUNDPERMITFLAG @\nSFX I Y 1\nSFX I 0 i .\n");
            AffixDict::from_hunspell(&aff, &format!("3\n{stem}\nadó/Y\nház/Y\n"), &[]).unwrap()
        };
        let d = suffixed("jövedék/IYS");
        for w in ["jövedékiadó-", "jövedékiadó-ház", "jövedékbanadó-"] {
            assert!(d.check(w), "{w}");
        }
        for w in ["jövedékiadó", "jövedékiház", "házjövedékiadó-"] {
            assert!(!d.check(w), "{w}");
        }
        assert!(!suffixed("jövedék/I").check("jövedékiadó-"), "the stem needs the compound flag");
        assert!(!suffixed("jövedék/IYS\njövedéki").check("jövedékiadó-"), "not for a listed stem");
        // Nor does the 6–3 rule apply there.
        let long = AffixDict::from_hunspell(
            &format!("{aff}LANG hu_HU\nCOMPOUNDWORDMAX 2\nCOMPOUNDSYLLABLE 6 aáeéiíoóöőuúüű\n"),
            "4\nalma/Y\ntető/Y\nkulcs/Y\nház/Y\n",
            &[],
        )
        .unwrap();
        assert!(!long.check("almatetőkulcsalma") && long.check("almatetőkulcsalma-") && long.check("almatetőkulcsalma-ház"));
    }

    /// A Hungarian dictionary that breaks at hyphens, for the hyphen-side tests.
    const HYPHEN_AFF: &str = "SET UTF-8\nFLAG UTF-8\nLANG hu_HU\nCOMPOUNDFLAG Y\nCOMPOUNDBEGIN v\nCOMPOUNDFORBIDFLAG %\nCOMPOUNDMIN 1\nCHECKCOMPOUNDDUP\nWORDCHARS -\nBREAK 3\nBREAK -\nBREAK ^-\nBREAK -$\nSFX I Y 1\nSFX I 0 i .\nSFX S Y 1\nSFX S 0 ban .\nSFX P Y 1\nSFX P 0 pe/% .\n";

    /// Before a hyphen a suffix stays on a first part with the compound flag only, a stem
    /// flagged `H` starts one as `F` does, `COMPOUNDFORBIDFLAG` doesn't count there, and a stem
    /// next to a form of itself is a duplicate, as with the Hunspell 1.7 command line.
    #[test]
    fn the_hyphen_side_keeps_the_compound_flag_and_duplicates() {
        let aff = HYPHEN_AFF;
        let d = |dic: &[&str]| AffixDict::from_hunspell(aff, &format!("{}\n{}\n", dic.len(), dic.join("\n")), &[]).unwrap();
        let begin = d(&["kapu/vI", "fa/Y", "jövedék/IY"]);
        assert!(!begin.check("kapuifa-") && begin.check("kapufa-") && begin.check("jövedékifa-"));
        let dup = d(&["ház/YS", "kert/Y"]);
        for w in ["házbanház-", "házházban-", "házházban"] {
            assert!(!dup.check(w), "{w}");
        }
        assert!(dup.check("házbankert-"));
        // Only the last two parts count, and a listed last part is itself.
        let pairs = d(&["ház/YS", "fa/Y"]);
        for w in ["házházfa", "házbanházfa-", "házfaház", "fafaház"] {
            assert!(pairs.check(w), "{w}");
        }
        let aff_k = format!("{aff}SFX K Y 1\nSFX K 0 kor .\n");
        let listed = AffixDict::from_hunspell(&aff_k, "2\nvas/YKS\nvaskor/Y\n", &[]).unwrap();
        assert!(listed.check("vasvaskor") && !listed.check("vasvas"));
        // A listed text that can't stand there is read through its stem.
        let bare = AffixDict::from_hunspell(&format!("{aff}SFX L Y 1\nSFX L 0 kor/Y .\n"), "2\nvas/YL\nvaskor\n", &[]).unwrap();
        assert!(!bare.check("vasvaskor"));
        assert!(!d(&["ház/YS", "házban"]).check("házházban"));
        // A last part with another reading than the stem before it is no duplicate.
        let other = AffixDict::from_hunspell(
            "SET UTF-8\nFLAG UTF-8\nLANG hu_HU\nCOMPOUNDFLAG Y\nCOMPOUNDMIN 1\nCHECKCOMPOUNDDUP\nCOMPOUNDSYLLABLE 6 aáeéiíoóöőuúüű\nSFX A Y 1\nSFX A 0 oz/Z .\nSFX Z Y 1\nSFX Z 0 t .\nSFX B Y 1\nSFX B 0 hoz .\n",
            "3\nboly/YB\nbolyh/YA\nbolyhoz\n",
            &[],
        )
        .unwrap();
        assert!(other.check("bolybolyhoz") && other.check("bolybolyhozt"));
        let ha = AffixDict::from_hunspell(&format!("{aff}SFX Z Y 1\nSFX Z 0 z/Y .\n"), "2\nház/YS\nhá/YZ\n", &[]).unwrap();
        assert!(ha.check("házház"), "házház");
        assert!(ha.check("házbanház-"), "házbanház-");
        assert!(!ha.check("házházban"), "házházban");
        // The homonym that stands there counts, not the first one listed.
        let order = d(&["ház/S", "ház/YS"]);
        for w in ["házházban", "házházban-", "házbanház-"] {
            assert!(!order.check(w), "{w}");
        }
        // A first part before a hyphen ignores `COMPOUNDFORBIDFLAG`, bare too.
        let kek = d(&["kék/Y%", "alma/Y", "fa/Y"]);
        assert!(kek.check("kékalma-") && kek.check("kékfa-alma") && !kek.check("kékalma"));
        // Two homonyms are two entries.
        let aff2 = format!("{aff}SFX D Y 1\nSFX D 0 ság/Y .\nSFX J Y 1\nSFX J 0 jára .\n");
        let homonyms = AffixDict::from_hunspell(&aff2, "2\nbarát/YD\nbarát/YJ\n", &[]).unwrap();
        assert!(homonyms.check("barátságbarátjára-") && !homonyms.check("barátbarát-"));
        assert!(!AffixDict::from_hunspell(&aff2, "1\nbarát/YDJ\n", &[]).unwrap().check("barátságbarátjára-"));
        let h = d(&["piros/H", "alma/Y", "fa/Y"]);
        assert!(h.check("pirosalma-") && h.check("pirosalma-fa"));
        let forbid = d(&["piros/F%", "alma/Y", "fa/Y", "jövedék/PY", "adó/Y"]);
        assert!(forbid.check("pirosalma-") && forbid.check("pirosalma-fa") && forbid.check("jövedékpeadó-") && !forbid.check("pirosalma"));
    }

    /// `CHECKCOMPOUNDDUP` reads each part in the order of the Hunspell 1.7 command line, and
    /// the flags count on the stem and the affix it checks them on.
    #[test]
    fn duplicates_follow_the_reading_order_of_the_command_line() {
        let aff = HYPHEN_AFF;
        // The compound flag's reading comes before one the place allows (`bar/Y` + `ra` before
        // `barr/x` + `a`), and an affixed repeat is no way out (`barát` + `ra`).
        let pass = AffixDict::from_hunspell(
            "SET UTF-8\nFLAG UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDBEGIN v\nCOMPOUNDEND x\nCHECKCOMPOUNDDUP\nSFX A Y 1\nSFX A 0 a .\nSFX B Y 1\nSFX B 0 ra .\n",
            "2\nbarr/Axv\nbar/BY\n",
            &[],
        )
        .unwrap();
        assert!(pass.check("barrbarra"));
        let ra = AffixDict::from_hunspell(&format!("{aff}SFX R Y 1\nSFX R 0 ra .\n"), "2\nbarát/YR\nbarát/YRD\n", &[]).unwrap();
        assert!(!ra.check("barátbarátra"));
        // A bare homonym is the first one listed that may stand there, with either flag.
        let dup = "SET UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDBEGIN v\nCOMPOUNDEND x\nCHECKCOMPOUNDDUP\n";
        for dic in ["2\nház/x\nház/YS\n", "2\nház/v\nház/Y\n"] {
            assert!(AffixDict::from_hunspell(dup, dic, &[]).unwrap().check("házház"), "{dic}");
        }
        // With the flag of the place, a part before the last reads suffixes before prefixes
        // (`kihá` + `z`, not `ki` + `ház`), the last part prefixes first.
        let permit = format!("{dup}COMPOUNDPERMITFLAG Q\n");
        let first =
            AffixDict::from_hunspell(&format!("{permit}PFX P Y 1\nPFX P 0 ki .\nSFX S Y 1\nSFX S 0 z/Q .\n"), "2\nház/Pvx\nkihá/Sv\n", &[]).unwrap();
        assert!(first.check("kiházház"));
        let prefix = format!("{permit}PFX P Y 1\nPFX P 0 ki/Q .\n");
        let last = AffixDict::from_hunspell(&format!("{prefix}SFX S Y 1\nSFX S 0 z .\n"), "2\nház/Pvx\nkihá/Sx\n", &[]).unwrap();
        assert!(!last.check("házkiház"));
        let two =
            AffixDict::from_hunspell(&format!("{prefix}SFX S Y 1\nSFX S 0 á/T .\nSFX T Y 1\nSFX T 0 z .\n"), "2\nház/Pvx\nkih/Sx\n", &[]).unwrap();
        assert!(!two.check("házkiház"));
        // The compound flag counts on the inner of two suffixes and not on a prefix before a
        // suffix: `kih` + `á` + `z/Y` and `ki/Y` + `há` + `z` don't have it.
        let outer =
            AffixDict::from_hunspell(&format!("{prefix}SFX S Y 1\nSFX S 0 á/T .\nSFX T Y 1\nSFX T 0 z/Y .\n"), "2\nház/Pvx\nkih/Sx\n", &[]).unwrap();
        assert!(!outer.check("házkiház"));
        let inner =
            AffixDict::from_hunspell(&format!("{prefix}SFX S Y 1\nSFX S 0 á/TY .\nSFX T Y 1\nSFX T 0 z .\n"), "2\nház/Pvx\nkih/Sx\n", &[]).unwrap();
        assert!(inner.check("házkiház"));
        let ki = AffixDict::from_hunspell(
            &format!("{permit}COMPOUNDMIN 1\nPFX P Y 1\nPFX P 0 ki/YQ .\nSFX S Y 1\nSFX S 0 z .\n"),
            "2\nhá/PSx\nkihá/SY\n",
            &[],
        )
        .unwrap();
        assert!(!ki.check("kihákiház"));
        let ki_end = AffixDict::from_hunspell(
            &format!("{permit}COMPOUNDMIN 1\nPFX P Y 1\nPFX P 0 ki/YQ .\nSFX S Y 1\nSFX S 0 z .\n"),
            "2\nhá/PS\nkihá/Sxv\n",
            &[],
        )
        .unwrap();
        assert!(!ki_end.check("kihákiház"));
        // The same holds for a part to stand there at all; a prefix alone has it on its own.
        let flags = "SET UTF-8\nCOMPOUNDMIN 1\nCOMPOUNDFLAG Y\nCOMPOUNDBEGIN v\nCOMPOUNDEND x\nCOMPOUNDPERMITFLAG Q\n";
        let cases = [
            ("SFX S Y 1\nSFX S 0 á/T .\nSFX T Y 1\nSFX T 0 z/Y .\n", "2\nház/v\nkih/S\n", "házkiház", false),
            ("SFX S Y 1\nSFX S 0 á/TY .\nSFX T Y 1\nSFX T 0 z .\n", "2\nház/v\nkih/S\n", "házkiház", true),
            ("PFX P Y 1\nPFX P 0 ki/YQ .\nSFX S Y 1\nSFX S 0 z .\n", "2\nház/v\nhá/PS\n", "házkiház", false),
            ("PFX P Y 1\nPFX P 0 ki/YQ .\n", "2\nház/v\nház/P\n", "házkiház", true),
            ("COMPOUNDMORESUFFIXES\nSFX S Y 1\nSFX S 0 á/TQ .\nSFX T Y 1\nSFX T 0 z/YQ .\n", "2\nház/Y\nkih/S\n", "kiházház", false),
            ("COMPOUNDMORESUFFIXES\nSFX S Y 1\nSFX S 0 á/TQY .\nSFX T Y 1\nSFX T 0 z/Q .\n", "2\nház/Y\nkih/S\n", "kiházház", true),
        ];
        for (affixes, dic, word, right) in cases {
            assert_eq!(AffixDict::from_hunspell(&format!("{flags}{affixes}"), dic, &[]).unwrap().check(word), right, "{affixes}");
        }
        let no_dup = AffixDict::from_hunspell(&prefix.replace("CHECKCOMPOUNDDUP\n", ""), "1\nház/Pvx\n", &[]).unwrap();
        assert!(no_dup.check("házkiház"));
    }

    #[test]
    fn every_compounding_directive_loads() {
        for dir in super::DIRECTIVES {
            let arg = match *dir {
                "COMPOUNDMIN" | "COMPOUNDWORDMAX" => "2",
                "COMPOUNDSYLLABLE" => "6 aeiou",
                "CHECKCOMPOUNDPATTERN" => "ssz sz",
                "COMPOUNDRULE" => "n*m",
                "BREAK" => "-",
                _ => "Y",
            };
            let aff = format!("SET UTF-8\n{dir} {arg}\n");
            assert!(AffixDict::from_hunspell(&aff, "1\nalma/Y\n", &[]).is_ok(), "{dir} should load");
        }
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
            // `COMPOUNDMIN 2`: `1999-es` is wrong here, as in Hunspell.
            &["9-es", "alma-ház", "-alma", "alma-féle", "alma-ház-tető", "SMS-ben", "SMS-BEN"],
            &["1999", "1999-es", "19-es", "19x", "alma-xyz", "alma-", "SMS-BAN"],
        );
    }

    #[test]
    fn break_sides_are_checked_like_words() {
        // As Hunspell 1.7 does with the Hungarian dictionary: each side of a break has its own
        // case readings (`alma-Fa`, `Petőfi-HÁZ`), and a word in capitals is split capitalized,
        // so `SÁMBOKI-HAZAY` is `Sámboki-hazay`, which is wrong.
        let d = AffixDict::from_hunspell(AFF, &format!("{DIC}Sámboki/Y\nHazay/Y\n"), &[]).unwrap();
        assert_words(
            &d,
            &["alma-Fa", "Alma-Fa", "ALMA-fa", "alma-FA", "Petőfi-Ház", "Petőfi-HÁZ", "alma-fa-Ház", "Sámboki-Hazay", "SÁMBOKI-Hazay"],
            &["aLMA-fa", "Petőfi-HáZ", "petőfi-ház", "SÁMBOKI-HAZAY", "Sámboki-hazay"],
        );
    }

    #[test]
    fn long_hyphenated_headings_are_checked_within_the_budget() {
        // Every side is checked once per word, so a heading in capitals with many parts is
        // still found correct; ten break points or more make a word wrong, as in Hunspell.
        let d = dict();
        let parts = ["alma", "fa", "ház", "tető", "kulcs", "szék", "alma", "fa", "ház", "tető", "kulcs"];
        assert!(d.check(&parts[..6].join("-").to_uppercase()));
        assert!(d.check(&parts[..10].join("-")) && d.check(&parts[..10].join("-").to_uppercase()), "nine break points");
        assert!(!d.check(&parts.join("-")), "ten break points");
        assert!(!d.check(&format!("{}-xyz", parts[..6].join("-").to_uppercase())));
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
            &["almafa", "kisház", "váróház", "mikroház", "9-es", "alma-ház", "Petőfi-ház", "almatetőkulcsfaban"],
            &["házkis", "sakkkör", "hajlak", "mikro", "üzelm", "Petőfi-híd", "almaház", "almatetőkulcsalma"],
        );
    }

    #[test]
    fn dictionaries_without_compounding_are_unchanged() {
        // No compound flags, no BREAK: neither compounds nor hyphen splits are accepted.
        let d = AffixDict::from_hunspell("SET UTF-8\nSFX a Y 1\nSFX a 0 y .\n", "2\nkot/a\npies/a\n", &[]).unwrap();
        assert!(d.check("koty") && !d.check("kotpies") && !d.check("kot-pies"));
    }

    #[test]
    fn flags_only_an_affix_carries_on_still_count() {
        // `P` (COMPOUNDPERMITFLAG) and `F` (COMPOUNDFORBIDFLAG) appear only after the `/` of a
        // suffix, on no stem and as no rule's own flag; they must still get a bit, or the
        // compound rules that test them would never match.
        let d = AffixDict::from_hunspell(AFF, DIC, &[]).unwrap();
        assert!(d.flag_chars.contains(&'P') && d.flag_chars.contains(&'F'), "{:?}", d.flag_chars);
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.flag_chars.contains(&'P') && back.flag_chars.contains(&'F'), "the compact format keeps them");
    }

    #[test]
    fn a_word_with_countless_splits_ends_quickly() {
        // Every split of a run of `a`s is a candidate; cached parts must cost budget too, or
        // the search would try split after split for free.
        let d = AffixDict::from_hunspell("SET UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDMIN 1\n", "1\na/Y\n", &[]).unwrap();
        let w = format!("{}x", "a".repeat(60));
        let t = std::time::Instant::now();
        assert!(!d.check(&w));
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn failed_states_are_searched_once() {
        // With `COMPOUNDMIN 1` the search meets the same dead ends again and again; once a state
        // has failed it costs nothing more, so the budget lasts until the one valid split, which
        // Hunspell accepts.
        let d = AffixDict::from_hunspell("SET UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDMIN 1\n", "3\naaa/Y\naaaa/Y\naab/Y\n", &[]).unwrap();
        for n in [40, 42, 44] {
            assert!(d.check(&format!("{}aab", "a".repeat(n))), "{n}");
        }
        assert!(!d.check(&format!("{}aab", "a".repeat(45))), "45 = 3 × 15 needs more than twelve parts");
    }

    #[test]
    fn compound_patterns_as_hunspell_reads_them() {
        // Checked with the Hunspell 1.7 command line: a replacement field and a trailing comment
        // leave the pattern in force, a commented count line is a count, and `0` stands for a
        // part that is a stem as listed.
        let dic = "4\nrossz/Y\nszék/Y\nház/Y\nkis/Y\n";
        let aff =
            |pattern: &str| format!("SET UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDMIN 2\nCHECKCOMPOUNDPATTERN 1 # count\nCHECKCOMPOUNDPATTERN {pattern}\n");
        for pattern in ["ssz sz", "ssz sz sz", "ssz sz # comment", "ssz sz # comment more"] {
            let d = AffixDict::from_hunspell(&aff(pattern), dic, &[]).unwrap();
            assert_words(&d, &["kisszék", "rosszház", "székrossz"], &["rosszszék"]);
        }
        let d = AffixDict::from_hunspell(&aff("0 sz"), dic, &[]).unwrap();
        assert_words(&d, &["rosszház", "székrossz"], &["rosszszék", "kisszék", "házszék"]);
        let d = AffixDict::from_hunspell(&aff("0/Y sz"), dic, &[]).unwrap();
        assert_words(&d, &["rosszház"], &["kisszék"]);
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert_words(&back, &["rosszház"], &["kisszék"]);
    }

    #[test]
    fn a_commented_count_line_is_no_break() {
        let aff = "SET UTF-8\nBREAK 1 # breaks\nBREAK -\n";
        let d = AffixDict::from_hunspell(aff, "2\nalma\nfa\n", &[]).unwrap();
        assert_words(&d, &["alma-fa"], &["alma1fa", "alma#fa"]);
    }

    #[test]
    fn a_later_prefix_needs_the_permit_flag() {
        // As the Hunspell 1.7 command line reads it, with `LANG hu_HU` or without: on the last
        // part a prefix needs `COMPOUNDPERMITFLAG`, the compound flag on it isn't enough; on a
        // middle part it needs neither (`anyagelszántház`).
        let aff = "SET UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDPERMITFLAG P\nCOMPOUNDMIN 2\nPFX e Y 1\nPFX e 0 el .\nPFX g Y 1\nPFX g 0 ki/Y .\nPFX h Y 1\nPFX h 0 be/P .\n";
        let dic = "3\nanyag/Yegh\nszánt/Yegh\nház/Y\n";
        for aff in [aff.to_string(), format!("LANG hu_HU\n{aff}")] {
            let d = AffixDict::from_hunspell(&aff, dic, &[]).unwrap();
            assert_words(&d, &["elszánt", "anyagbeszánt", "elanyagszánt", "anyagelszántház"], &["anyagelszánt", "anyagkiszánt"]);
            assert!(!AffixDict::from_bytes(&d.to_bytes()).unwrap().check("anyagkiszánt"));
        }
    }

    #[test]
    fn homonyms_are_parsed_one_by_one() {
        // `szán` the noun compounds, `szán` the verb takes the prefix: `el` + `szán` + `t` is no
        // compound part, as in Hunspell (the Hungarian `előanyagelszánt`).
        let aff = "SET UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDPERMITFLAG P\nCOMPOUNDMIN 2\nPFX e Y 1\nPFX e 0 el/P .\nSFX t Y 1\nSFX t 0 t .\n";
        let d = AffixDict::from_hunspell(aff, "3\nanyag/Y\nszán/Y\nszán/et\n", &[]).unwrap();
        assert_words(&d, &["szán", "elszán", "elszánt", "anyagszán", "szánt"], &["anyagelszánt", "anyagelszán"]);
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert_words(&back, &["elszánt", "anyagszán"], &["anyagelszánt"]);
    }

    #[test]
    fn hungarian_long_prefixes_and_hyphens() {
        // With `LANG hu_HU`, as in Hunspell: a prefix of two syllables on any part counts as a
        // word for the 6–3 rule, and the left side of a hyphen may be listed with it.
        let aff = "SET UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDPERMITFLAG P\nCOMPOUNDMIN 2\nCOMPOUNDWORDMAX 2\nCOMPOUNDSYLLABLE 6 aáeéiíoóöőuúüű\nPFX e Y 2\nPFX e 0 elő/P .\nPFX e 0 be/P .\nBREAK 1\nBREAK -\n";
        let dic = "4\nkerekasztal/Y\ndöntőbíró/Ye\nvegyiáru-\nkereskedés\n";
        let plain = AffixDict::from_hunspell(aff, dic, &[]).unwrap();
        assert_words(&plain, &["kerekasztalelődöntőbíró", "kerekasztalbedöntőbíró"], &["vegyiáru-kereskedés", "vegyiáru"]);
        let hu = AffixDict::from_hunspell(&format!("LANG hu_HU\n{aff}"), dic, &[]).unwrap();
        assert_words(&hu, &["kerekasztalbedöntőbíró", "vegyiáru-kereskedés", "VEGYIÁRU-KERESKEDÉS"], &["kerekasztalelődöntőbíró", "vegyiáru"]);
        assert_words(&hu, &["bedöntőbírókerekasztal"], &["elődöntőbírókerekasztal"]);
        assert_words(&plain, &["elődöntőbírókerekasztal"], &[]);
    }

    #[test]
    fn long_hyphenated_compounds_get_a_budget_per_side() {
        // Each break side has its own compound search; a reading that ran out of lookups isn't
        // remembered as wrong. Every side here is a long compound of `COMPOUNDMIN 1` parts.
        let d = AffixDict::from_hunspell("SET UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDMIN 1\nBREAK 1\nBREAK -\n", "3\naaa/Y\naaaa/Y\naab/Y\n", &[]).unwrap();
        let side = format!("{}aab", "a".repeat(40));
        for k in 2..=4 {
            let w = vec![side.clone(); k].join("-");
            assert!(d.check(&w) && d.check(&w.to_uppercase()) && d.check(&super::super::capitalize(&w)), "{k} sides");
        }
        assert!(!d.check(&format!("{side}-{}x", side)));
    }

    #[test]
    fn forbidden_readings_stop_the_check() {
        // Hunspell: a reading that is a listed FORBIDDENWORD makes the word wrong, before any
        // lower-case reading or break is tried.
        let d = AffixDict::from_hunspell("SET UTF-8\nFORBIDDENWORD W\nBREAK 1\nBREAK -\n", "5\nijs\nIjs/W\nalma\nfa\nalma-fa/W\n", &[]).unwrap();
        assert_words(&d, &["ijs", "alma", "fa"], &["Ijs", "IJS", "IJs", "alma-fa", "Alma-fa", "ALMA-FA"]);
    }

    #[test]
    fn compound_rules_take_any_number_of_parts_of_compoundmin_letters() {
        // Hunspell 1.7: `1` repeated as often as the word holds (60 times here), but no part
        // shorter than `COMPOUNDMIN`: with 2, `1999-es` is wrong.
        let dic = "4\n1/n\n9/n\n0/n\n9-es/m\n";
        let one = AffixDict::from_hunspell("SET UTF-8\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE n*m\n", dic, &[]).unwrap();
        assert_words(&one, &["1999-es", "19-es", &format!("{}9-es", "1".repeat(60))], &["1999", "1-es"]);
        let two = AffixDict::from_hunspell("SET UTF-8\nCOMPOUNDMIN 2\nCOMPOUNDRULE 1\nCOMPOUNDRULE n*m\n", dic, &[]).unwrap();
        assert_words(&two, &["9-es"], &["1999-es", "19-es"]);
    }

    #[test]
    fn a_zero_pattern_meets_a_prefixed_stem() {
        // `CHECKCOMPOUNDPATTERN 0 sz`: a part whose stem ends at the boundary, prefixed or not,
        // before `sz…` (Hunspell rejects `kisszék` and `elkisszék`, accepts `házosszék`).
        let aff = "SET UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDPERMITFLAG P\nCOMPOUNDMIN 2\nCHECKCOMPOUNDPATTERN 1\nCHECKCOMPOUNDPATTERN 0 sz\nPFX e Y 1\nPFX e 0 el/P .\nSFX s Y 1\nSFX s 0 os/P .\n";
        let d = AffixDict::from_hunspell(aff, "3\nkis/Ye\nszék/Y\nház/Ys\n", &[]).unwrap();
        assert_words(&d, &["házosszék", "székház"], &["kisszék", "elkisszék", "házszék"]);
    }

    #[test]
    fn compound_rep_checks_spend_the_budget() {
        // Every REP replacement looked up costs budget, so many REP hits at every boundary end
        // quickly.
        let mut aff = String::from("SET UTF-8\nCOMPOUNDFLAG Y\nCOMPOUNDMIN 1\nCHECKCOMPOUNDREP\nREP 200\n");
        for i in 0..200 {
            aff.push_str(&format!("REP a {}\n", "b".repeat(i % 7 + 1)));
        }
        let d = AffixDict::from_hunspell(&aff, "2\na/Y\naa/Y\n", &[]).unwrap();
        let t = std::time::Instant::now();
        let _ = d.check(&format!("{}x", "a".repeat(60)));
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn forms_leave_out_what_is_no_word() {
        // A bare `NEEDAFFIX` stem and every form of an `ONLYINCOMPOUND` stem aren't words.
        let d = dict();
        assert!(d.forms("mikro").is_empty());
        let forms = d.forms("üzelm");
        assert!(!forms.iter().any(|f| f == "üzelm") && forms.iter().any(|f| f == "üzelmok"), "{forms:?}");
        for f in forms {
            assert!(d.check(&f), "{f}");
        }
    }

    #[test]
    fn forms_leave_out_affixes_that_need_more() {
        // `ó/N`: the form needs another affix; `é/O`: it stands only inside compounds; `ó/Ns`
        // chains to `k`, which is a word again.
        let aff = "SET UTF-8\nNEEDAFFIX N\nONLYINCOMPOUND O\nSFX d Y 1\nSFX d 0 ó/Ns .\nSFX e Y 1\nSFX e 0 é/O .\nSFX s Y 1\nSFX s 0 k .\n";
        let d = AffixDict::from_hunspell(aff, "1\nvár/de\n", &[]).unwrap();
        let forms = d.forms("vár");
        assert!(!forms.iter().any(|f| f == "váró" || f == "váré"), "{forms:?}");
        assert!(forms.iter().any(|f| f == "várók"), "{forms:?}");
        for f in &forms {
            assert!(d.check(f), "{f}");
        }
    }

    #[test]
    fn numbered_morphology_aliases_may_carry_a_comment() {
        // `AM 1 # morphology` is the count line; alias 1 is the first entry after it.
        let aff = format!("{AFF}AM 1 # morphology\nAM st:levél ts:PLUR\n");
        let d = AffixDict::from_hunspell(&aff, "4\ntető/Y\nfa/Y\nkulcs/Y\nlevelek/Y\t1\n", &[]).unwrap();
        assert!(d.check("tetőfakulcslevelek"), "6 of 7 syllables count");
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
