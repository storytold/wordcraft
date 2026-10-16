//! Hunspell-style affix dictionaries, including Polish inflection and German compounding.
//!
//! Words are stems or forms licensed by their prefix/suffix flags and conditions. Continuation
//! flags license further affixes, and compound flags license word parts in the declared positions.
//! Highly inflected languages store millions of forms this way in a few hundred thousand stems.
//!
//! The rules and stems come from a Hunspell `.aff`/`.dic` pair ([`AffixDict::from_hunspell`]),
//! read at runtime from the dictionaries installed on the system ([`crate::hunspell`]). They can
//! also be stored in WordCraft's compact format ([`AffixDict::to_bytes`]): the rules as text
//! lines and the stems front-coded, raw-deflated together (the Polish dictionary is under 1 MB).
//!
//! Supported: per-character flags, prefix/suffix rules, two suffix continuation levels,
//! forbidden and virtual stems, case restrictions, positional compounds and compound-only
//! linking forms. This is an original implementation of the public Hunspell dictionary format;
//! it does not include Hunspell code or dictionary data. Unsupported correctness directives
//! are rejected instead of silently accepting words under different rules.
//!
//! Lookups are bounded by [`MAX_WORD_BYTES`], two suffix levels and a finite compound search.

use std::cell::Cell;
use std::collections::HashMap;
use std::hash::Hasher;

use crate::patterns::{FxHasher, FxMap};

const MAGIC: &str = "WCAFF1";
/// Longest word (in bytes) looked up; longer input is never a dictionary word.
pub const MAX_WORD_BYTES: usize = 120;
/// Shared by casing, suffix continuation, compounds and BREAK. Malicious dictionaries can
/// repeat zero-length rules thousands of times; reject an inconclusive lookup before that
/// turns a spelling check into quadratic or exponential work.
const MAX_LOOKUP_WORK: usize = 100_000;
/// Empty slot in the stem hash table.
const EMPTY: u32 = u32::MAX;

/// Directives that change which words are correct in ways this engine doesn't implement.
const UNSUPPORTED: &[&str] = &[
    "AF",
    "AM",
    "COMPLEXPREFIXES",
    "COMPOUNDRULE",
    "FULLSTRIP",
    "ICONV",
    "IGNORE",
    "LEMMA_PRESENT",
    "OCONV",
    "SUBSTANDARD",
    "COMPOUNDFORBIDFLAG",
    "COMPOUNDMORESUFFIXES",
    "COMPOUNDROOT",
    "COMPOUNDSYLLABLE",
    "SYLLABLENUM",
    "FORCEUCASE",
    "CHECKCOMPOUNDDUP",
    "CHECKCOMPOUNDREP",
    "CHECKCOMPOUNDCASE",
    "CHECKCOMPOUNDTRIPLE",
    "CHECKCOMPOUNDPATTERN",
    "SIMPLIFIEDTRIPLE",
];

const FORBIDDEN: usize = 0;
const ONLY_COMPOUND: usize = 1;
const NEED_AFFIX: usize = 2;
const KEEP_CASE: usize = 3;
const CIRCUMFIX: usize = 4;
const COMPOUND: usize = 5;
const BEGIN: usize = 6;
const MIDDLE: usize = 7;
const END: usize = 8;
const PERMIT: usize = 9;
const NO_SUGGEST: usize = 10;
const FLAG_DIRECTIVES: [&str; 11] = [
    "FORBIDDENWORD",
    "ONLYINCOMPOUND",
    "NEEDAFFIX",
    "KEEPCASE",
    "CIRCUMFIX",
    "COMPOUNDFLAG",
    "COMPOUNDBEGIN",
    "COMPOUNDMIDDLE",
    "COMPOUNDEND",
    "COMPOUNDPERMITFLAG",
    "NOSUGGEST",
];

#[derive(Clone, Debug)]
struct Controls {
    flags: [Option<char>; FLAG_DIRECTIVES.len()],
    compound_min: usize,
    compound_max: usize,
    check_sharps: bool,
    breaks: Vec<String>,
    explicit_breaks: bool,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            flags: [None; FLAG_DIRECTIVES.len()],
            compound_min: 3,
            compound_max: MAX_WORD_BYTES,
            check_sharps: false,
            breaks: vec!["-".into(), "^-".into(), "-$".into()],
            explicit_breaks: false,
        }
    }
}

impl Controls {
    /// Options shared by the Hunspell reader and the compact-format reader.
    fn parse(&mut self, directive: &str, value: &str) -> Result<bool, String> {
        let directive = if directive == "PSEUDOROOT" { "NEEDAFFIX" } else { directive };
        if let Some(index) = FLAG_DIRECTIVES.iter().position(|&d| d == directive) {
            let mut chars = value.chars();
            let (Some(flag), None) = (chars.next(), chars.next()) else {
                return Err(format!("{directive} needs one flag character"));
            };
            if let Some(slot) = self.flags.get_mut(index) {
                *slot = Some(flag);
            }
            return Ok(true);
        }
        match directive {
            "CHECKSHARPS" => self.check_sharps = true,
            "COMPOUNDMIN" | "COMPOUNDWORDMAX" => {
                let number = value.parse::<usize>().map_err(|_| format!("{directive} needs a positive number"))?;
                if number == 0 {
                    return Err(format!("{directive} needs a positive number"));
                }
                let bounded = number.min(MAX_WORD_BYTES + 1);
                if directive == "COMPOUNDMIN" {
                    self.compound_min = bounded;
                } else {
                    self.compound_max = bounded;
                }
            }
            "BREAK" => {
                if !self.explicit_breaks {
                    value.parse::<usize>().map_err(|_| "BREAK needs a count before its entries".to_string())?;
                    self.breaks.clear();
                    self.explicit_breaks = true;
                } else if !value.is_empty() && value.len() <= MAX_WORD_BYTES && self.breaks.len() < MAX_WORD_BYTES {
                    self.breaks.push(value.to_string());
                } else {
                    return Err("invalid or excessive BREAK entries".to_string());
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    First,
    Middle,
    Last,
}

#[derive(Clone, Copy, Default)]
struct Lookup<'a> {
    work: Option<&'a Cell<usize>>,
    part: Option<Part>,
    case_fallback: bool,
    sharp_case: bool,
    excluded: u128,
}

impl Lookup<'_> {
    fn spend(self) -> bool {
        self.work.is_none_or(|work| match work.get().checked_sub(1) {
            Some(remaining) => {
                work.set(remaining);
                true
            }
            None => false,
        })
    }
}

/// Rules removed from a candidate, in application order. The fixed size bounds both work
/// and stack use; a continuation class can license one further suffix.
#[derive(Clone, Copy, Default)]
struct Applied<'a> {
    prefix: Option<&'a Rule>,
    inner: Option<&'a Rule>,
    outer: Option<&'a Rule>,
}

/// One character class of a rule's condition.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Cond {
    Any,
    Char(char),
    Set { negated: bool, chars: Box<[char]> },
}

impl Cond {
    fn matches(&self, c: char) -> bool {
        match self {
            Cond::Any => true,
            Cond::Char(x) => *x == c,
            Cond::Set { negated, chars } => chars.contains(&c) != *negated,
        }
    }
}

/// Parse a Hunspell condition (`.`, `[^i]e`, `[km]anie`…).
fn parse_cond(text: &str) -> Result<Box<[Cond]>, String> {
    if text == "." {
        return Ok(Box::new([]));
    }
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '.' => out.push(Cond::Any),
            '[' => {
                let mut set = Vec::new();
                let mut negated = false;
                let mut closed = false;
                let mut first = true;
                for d in chars.by_ref() {
                    if d == ']' {
                        closed = true;
                        break;
                    }
                    if first && d == '^' {
                        negated = true;
                    } else {
                        set.push(d);
                    }
                    first = false;
                }
                if !closed {
                    return Err(format!("unclosed `[` in condition `{text}`"));
                }
                out.push(Cond::Set { negated, chars: set.into_boxed_slice() });
            }
            ']' => return Err(format!("stray `]` in condition `{text}`")),
            c => out.push(Cond::Char(c)),
        }
    }
    Ok(out.into_boxed_slice())
}

/// How many suffixes a word may carry in a row through continuation classes (`work` +
/// `able/B` + `s` → `workables` is two). Hunspell strips at most two suffixes, so a third
/// chained suffix is not a word; the bound also keeps hostile input from walking deep recursions.
const MAX_CHAIN: u8 = 2;
/// How many suffixes may sit under the outermost one during form generation.
const INNER_CHAIN: u8 = MAX_CHAIN - 1;

/// One prefix or suffix rule.
#[derive(Clone, Debug)]
struct Rule {
    /// Bit of the rule's flag in the stems' flag masks.
    bit: u128,
    flag: char,
    cross: bool,
    strip: Box<str>,
    add: Box<str>,
    /// Continuation classes (`SFX A 0 able/B`): flags the derived form carries, licensing
    /// further affixes. Chained application is bounded by [`MAX_CHAIN`].
    cont: u128,
    cond: Box<[Cond]>,
    cond_text: Box<str>,
}

impl Rule {
    /// Does `root` end (suffix rule) or start (prefix rule) as the condition says?
    fn cond_matches(&self, root: &str, suffix: bool) -> bool {
        if self.cond.is_empty() {
            return true;
        }
        if suffix {
            let mut it = root.chars().rev();
            self.cond.iter().rev().all(|c| it.next().is_some_and(|x| c.matches(x)))
        } else {
            let mut it = root.chars();
            self.cond.iter().all(|c| it.next().is_some_and(|x| c.matches(x)))
        }
    }
}

/// Suffix rules sharing an appended string and a stripped one: one stem lookup serves them all.
#[derive(Clone, Debug, Default)]
struct SfxGroup {
    strip: Box<str>,
    rules: Vec<u32>,
}

/// A stem list with affix rules (see the module docs).
#[derive(Clone, Debug, Default)]
pub struct AffixDict {
    /// All stems, concatenated, sorted by bytes.
    words: String,
    offs: Vec<u32>,
    /// Per stem: index into `sets`.
    flags: Vec<u16>,
    /// Distinct flag masks.
    sets: Vec<u128>,
    /// Correctness directives, and the bit corresponding to each declared special flag.
    controls: Controls,
    special: [u128; FLAG_DIRECTIVES.len()],
    /// Additional flag sets for homonyms. Keeping them separate avoids inventing combinations
    /// or turning a valid homonym into a virtual/compound-only stem.
    homonyms: FxMap<u32, Vec<u16>>,
    /// Open-addressing hash table of stem indices (`EMPTY` = free).
    table: Vec<u32>,
    /// Flag char of each bit.
    flag_chars: Vec<char>,
    prefixes: Vec<Rule>,
    suffixes: Vec<Rule>,
    /// Appended string → groups of suffix rules by stripped string.
    sfx_index: FxMap<Box<str>, Vec<SfxGroup>>,
    /// Longest appended suffix, in chars.
    max_sfx_chars: usize,
    try_chars: String,
    key: String,
    map: Vec<Vec<char>>,
    rep: Vec<(String, String)>,
    /// Provenance and licence lines, kept through [`AffixDict::to_bytes`].
    comments: Vec<String>,
}

fn hash_str(s: &str) -> u64 {
    let mut h = FxHasher::default();
    h.write(s.as_bytes());
    h.finish()
}

/// Letter case of a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Casing {
    /// No capitals (`kot`).
    Lower,
    /// First letter capital, no other (`Kot`).
    Initial,
    /// Every cased letter a capital, at least two (`KOT`).
    Upper,
    /// Anything else (`McDonald`, `kOT`).
    Mixed,
}

/// The casing of `w`.
pub fn casing(w: &str) -> Casing {
    let (mut upper, mut lower, mut first_upper) = (0usize, 0usize, false);
    for (i, c) in w.chars().enumerate() {
        if c.is_uppercase() {
            upper += 1;
            if i == 0 {
                first_upper = true;
            }
        } else if c.is_lowercase() {
            lower += 1;
        }
    }
    if upper == 0 {
        Casing::Lower
    } else if upper == 1 && first_upper {
        Casing::Initial
    } else if lower == 0 {
        Casing::Upper
    } else {
        Casing::Mixed
    }
}

/// `w` with its first letter a capital.
pub fn capitalize(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Parsed rules and tables, shared by the Hunspell reader and the compact format.
#[derive(Default)]
struct Rules {
    prefixes: Vec<RawRule>,
    suffixes: Vec<RawRule>,
    try_chars: String,
    key: String,
    map: Vec<Vec<char>>,
    rep: Vec<(String, String)>,
    controls: Controls,
}

/// A rule as parsed from text, before flags become bits: `(flag, cross, strip, add, cont, cond)`.
type RawRule = (char, bool, String, String, String, String);

/// `0` means empty in Hunspell's strip and append fields.
fn zero_empty(s: &str) -> String {
    if s == "0" { String::new() } else { s.to_string() }
}

/// Read Hunspell `.aff` text (already decoded to UTF-8).
fn parse_aff(aff: &str) -> Result<Rules, String> {
    let mut r = Rules::default();
    let mut cross: HashMap<(bool, char), bool> = HashMap::new();
    let mut map_header = false;
    let mut rep_header = false;
    for (n, line) in aff.lines().enumerate() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let Some(&dir) = f.first() else { continue };
        if dir.starts_with('#') {
            continue;
        }
        let at = |k: usize| f.get(k).copied().unwrap_or("");
        if r.controls.parse(dir, at(1)).map_err(|e| format!("line {}: {e}", n + 1))? {
            continue;
        }
        match dir {
            "SET" | "LANG" | "NAME" | "VERSION" | "HOME" | "WORDCHARS" | "MAXNGRAMSUGS" | "MAXDIFF" | "ONLYMAXDIFF" | "NOSPLITSUGS"
            | "SUGSWITHDOTS" | "WARN" | "FORBIDWARN" | "MAXCPDSUGS" => {}
            "FLAG" => {
                // Per-character flag modes (`FLAG UTF-8`, as VERO's pt_BR dictionary uses) work
                // with the one-character parsing here; only the multi-character encodings differ.
                if !matches!(at(1), "UTF-8" | "DEFAULT" | "utf-8" | "default" | "UTF8" | "") {
                    return Err(format!("line {}: FLAG {} is not supported (one-character flags only)", n + 1, at(1)));
                }
            }
            "TRY" => r.try_chars = at(1).to_string(),
            "KEY" => r.key = at(1).to_string(),
            "MAP" => {
                if !map_header && at(1).parse::<usize>().is_ok() {
                    map_header = true;
                } else if !at(1).contains('(') {
                    r.map.push(at(1).chars().collect());
                }
            }
            "REP" => {
                if !rep_header && f.len() == 2 && at(1).parse::<usize>().is_ok() {
                    rep_header = true;
                } else if f.len() >= 3 {
                    r.rep.push((at(1).replace('_', " "), at(2).replace('_', " ")));
                }
            }
            "PFX" | "SFX" => {
                let suffix = dir == "SFX";
                let mut flag_chars = at(1).chars();
                let (Some(flag), None) = (flag_chars.next(), flag_chars.next()) else {
                    return Err(format!("line {}: flag `{}` is not one character", n + 1, at(1)));
                };
                // Header comments are ignored, while a hash in a rule condition stays literal.
                let is_header = f.len() >= 4 && matches!(at(2), "Y" | "N") && at(3).parse::<usize>().is_ok() && !cross.contains_key(&(suffix, flag));
                if is_header {
                    cross.insert((suffix, flag), at(2) == "Y");
                    continue;
                }
                if f.len() < 4 {
                    return Err(format!("line {}: short affix rule", n + 1));
                }
                // `able/B`: the appended text and its continuation classes.
                let (add, cont) = match at(3).split_once('/') {
                    Some((a, c)) => (a, c),
                    None => (at(3), ""),
                };
                if at(2).len() > MAX_WORD_BYTES || add.len() > MAX_WORD_BYTES || cont.chars().count() > 128 {
                    return Err(format!("line {}: excessive affix length", n + 1));
                }
                let cond = if f.len() >= 5 { at(4) } else { "." };
                parse_cond(cond).map_err(|e| format!("line {}: {e}", n + 1))?;
                let c = cross.get(&(suffix, flag)).copied().unwrap_or(false);
                let rule: RawRule = (flag, c, zero_empty(at(2)), zero_empty(add), cont.to_string(), cond.to_string());
                if suffix {
                    r.suffixes.push(rule);
                } else {
                    r.prefixes.push(rule);
                }
            }
            d if UNSUPPORTED.contains(&d) => return Err(format!("line {}: {d} is not supported", n + 1)),
            _ => {}
        }
    }
    Ok(r)
}

impl AffixDict {
    /// Build from Hunspell `.aff` and `.dic` text (decoded to UTF-8; the `.dic` count line and
    /// morphological fields are ignored). `comments` (provenance, licence) are kept with the data.
    pub fn from_hunspell(aff: &str, dic: &str, comments: &[&str]) -> Result<AffixDict, String> {
        // Some dictionaries open with a UTF-8 BOM; it must not glue itself to the first
        // directive or turn the `.dic` count line into a stem.
        let aff = aff.strip_prefix('\u{feff}').unwrap_or(aff);
        let dic = dic.strip_prefix('\u{feff}').unwrap_or(dic);
        let rules = parse_aff(aff)?;
        let mut stems: Vec<(String, String)> = Vec::new();
        for (n, line) in dic.lines().enumerate() {
            let Some(tok) = line.split_whitespace().next() else { continue };
            if n == 0 && tok.parse::<usize>().is_ok() {
                continue;
            }
            // `\/` is a literal slash in a word; the first plain `/` starts the flags.
            let mut word = String::new();
            let mut flags = String::new();
            let mut chars = tok.chars().peekable();
            let mut in_flags = false;
            while let Some(c) = chars.next() {
                if in_flags {
                    flags.push(c);
                } else if c == '\\' && chars.peek() == Some(&'/') {
                    word.push('/');
                    chars.next();
                } else if c == '/' {
                    in_flags = true;
                } else {
                    word.push(c);
                }
            }
            if word.is_empty() || word.len() > MAX_WORD_BYTES || word.contains(['\n', '/']) {
                continue;
            }
            stems.push((word, flags));
        }
        AffixDict::build(rules, stems, comments.iter().map(|s| s.to_string()).collect())
    }

    fn build(rules: Rules, mut stems: Vec<(String, String)>, comments: Vec<String>) -> Result<AffixDict, String> {
        // Flags: every one a rule or a stem uses gets a bit.
        let mut flag_chars: Vec<char> = rules.prefixes.iter().chain(&rules.suffixes).map(|r| r.0).collect();
        flag_chars.extend(rules.prefixes.iter().chain(&rules.suffixes).flat_map(|r| r.4.chars()));
        flag_chars.extend(rules.controls.flags.iter().flatten().copied());
        flag_chars.extend(stems.iter().flat_map(|s| s.1.chars()));
        flag_chars.sort_unstable();
        flag_chars.dedup();
        if flag_chars.len() > 128 {
            return Err(format!("{} distinct flags; at most 128 are supported", flag_chars.len()));
        }
        let bit_of = |c: char| flag_chars.iter().position(|&x| x == c).map_or(0, |i| 1u128 << i);
        let special = rules.controls.flags.map(|flag| flag.map_or(0, bit_of));
        // Stems sorted by bytes; homonyms (the same word twice) are merged.
        stems.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut d = AffixDict {
            words: String::with_capacity(stems.iter().map(|s| s.0.len()).sum()),
            offs: Vec::with_capacity(stems.len() + 1),
            flag_chars: flag_chars.clone(),
            controls: rules.controls,
            special,
            try_chars: rules.try_chars,
            key: rules.key,
            map: rules.map,
            rep: rules.rep,
            comments,
            ..AffixDict::default()
        };
        d.offs.push(0);
        let mut set_ids: HashMap<u128, u16> = HashMap::new();
        let mut i = 0;
        while i < stems.len() {
            let Some((word, _)) = stems.get(i) else { break };
            let mut masks = Vec::new();
            let mut j = i;
            while let Some((w, f)) = stems.get(j)
                && w == word
            {
                let mask = f.chars().map(bit_of).fold(0, |a, b| a | b);
                if !masks.contains(&mask) {
                    masks.push(mask);
                }
                j += 1;
            }
            let mut ids = Vec::new();
            for mask in masks {
                let next_id = u16::try_from(d.sets.len()).map_err(|_| "too many distinct flag sets".to_string())?;
                let id = *set_ids.entry(mask).or_insert(next_id);
                if usize::from(id) == d.sets.len() {
                    d.sets.push(mask);
                }
                ids.push(id);
            }
            let id = ids.first().copied().unwrap_or(0);
            if ids.len() > 1 {
                let word_index = u32::try_from(d.flags.len()).map_err(|_| "too many stems".to_string())?;
                d.homonyms.insert(word_index, ids.into_iter().skip(1).collect());
            }
            d.words.push_str(word);
            d.offs.push(u32::try_from(d.words.len()).map_err(|_| "stems too large".to_string())?);
            d.flags.push(id);
            i = j;
        }
        let mk = |(flag, cross, strip, add, cont, cond): RawRule| -> Result<Rule, String> {
            let cont = cont.chars().fold(0u128, |m, c| m | bit_of(c));
            Ok(Rule { bit: bit_of(flag), flag, cross, strip: strip.into(), add: add.into(), cont, cond: parse_cond(&cond)?, cond_text: cond.into() })
        };
        d.prefixes = rules.prefixes.into_iter().map(mk).collect::<Result<_, _>>()?;
        d.suffixes = rules.suffixes.into_iter().map(mk).collect::<Result<_, _>>()?;
        d.index();
        Ok(d)
    }

    /// Build the suffix index and the stem hash table.
    fn index(&mut self) {
        let mut idx: FxMap<Box<str>, Vec<SfxGroup>> = FxMap::default();
        for (i, r) in self.suffixes.iter().enumerate() {
            let groups = idx.entry(r.add.clone()).or_default();
            let Ok(i) = u32::try_from(i) else { break };
            match groups.iter_mut().find(|g| g.strip == r.strip) {
                Some(g) => g.rules.push(i),
                None => groups.push(SfxGroup { strip: r.strip.clone(), rules: vec![i] }),
            }
        }
        self.max_sfx_chars = self.suffixes.iter().map(|r| r.add.chars().count()).max().unwrap_or(0);
        self.sfx_index = idx;
        let n = self.len();
        let size = (n.max(1) * 2).next_power_of_two();
        let mut table = vec![EMPTY; size];
        let mask = size - 1;
        for i in 0..n {
            let Some(w) = self.word(i) else { continue };
            let mut slot = hash_str(w) as usize & mask;
            while table.get(slot).is_some_and(|&t| t != EMPTY) {
                slot = (slot + 1) & mask;
            }
            if let (Some(t), Ok(i)) = (table.get_mut(slot), u32::try_from(i)) {
                *t = i;
            }
        }
        self.table = table;
    }

    /// Number of stems.
    pub fn len(&self) -> usize {
        self.flags.len()
    }

    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }

    /// Number of prefix and suffix rules.
    pub fn rule_count(&self) -> usize {
        self.prefixes.len() + self.suffixes.len()
    }

    fn word(&self, i: usize) -> Option<&str> {
        let a = *self.offs.get(i)? as usize;
        let b = *self.offs.get(i + 1)? as usize;
        self.words.get(a..b)
    }

    /// The flag characters of `mask`, in flag order (how the compact format stores flags).
    fn flag_chars_for(&self, mask: u128) -> String {
        self.flag_chars.iter().enumerate().filter(|&(i, _)| mask & (1u128 << i) != 0).map(|(_, c)| c).collect()
    }

    /// The stems, in byte order.
    pub fn stems(&self) -> impl Iterator<Item = &str> + '_ {
        (0..self.len()).filter_map(|i| self.word(i))
    }

    /// The stems starting with `prefix`, in byte order (a binary search, then a scan).
    pub fn stems_with_prefix<'a>(&'a self, prefix: &str) -> impl Iterator<Item = &'a str> + use<'a> {
        let prefix = prefix.to_string();
        let first = {
            let (mut lo, mut hi) = (0usize, self.len());
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if self.word(mid).is_some_and(|w| w.as_bytes() < prefix.as_bytes()) {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        (first..self.len()).map_while(move |i| self.word(i).filter(|w| w.starts_with(prefix.as_str())))
    }

    /// Index of an exact dictionary entry, including explicitly forbidden spellings.
    fn raw_stem_index(&self, w: &str, lookup: Lookup<'_>) -> Option<u32> {
        if self.table.is_empty() {
            return None;
        }
        let mask = self.table.len() - 1;
        let mut slot = hash_str(w) as usize & mask;
        for _ in 0..self.table.len() {
            if !lookup.spend() {
                return None;
            }
            let index = *self.table.get(slot)?;
            if index == EMPTY {
                return None;
            }
            if self.word(index as usize) == Some(w) {
                return Some(index);
            }
            slot = (slot + 1) & mask;
        }
        None
    }

    fn forbidden_word(&self, w: &str, lookup: Lookup<'_>) -> bool {
        self.raw_stem_index(w, lookup).is_some_and(|index| self.masks(index).any(|flags| !lookup.spend() || flags & self.special[FORBIDDEN] != 0))
    }

    fn stem_index(&self, w: &str, lookup: Lookup<'_>) -> Option<u32> {
        let index = self.raw_stem_index(w, lookup)?;
        self.masks(index).all(|flags| lookup.spend() && flags & self.special[FORBIDDEN] == 0).then_some(index)
    }

    fn masks(&self, index: u32) -> impl Iterator<Item = u128> + '_ {
        self.flags
            .get(index as usize)
            .into_iter()
            .chain(self.homonyms.get(&index).into_iter().flatten())
            .filter_map(|&id| self.sets.get(usize::from(id)).copied())
    }

    fn stem_flags(&self, w: &str) -> Option<u128> {
        let work = Cell::new(MAX_LOOKUP_WORK);
        let lookup = Lookup { work: Some(&work), ..Lookup::default() };
        let index = self.stem_index(w, lookup)?;
        self.masks(index).try_fold(0, |a, b| lookup.spend().then_some(a | b))
    }

    /// Validate one complete derivation against a single homonym's flags.
    fn accepts(&self, root: &str, flags: u128, applied: Applied<'_>, lookup: Lookup<'_>) -> bool {
        let Applied { prefix, inner, outer } = applied;
        let suffix = inner.or(outer);
        let continuation = inner.map_or(0, |r| r.cont) | outer.map_or(0, |r| r.cont);
        if let Some(p) = prefix
            && flags & p.bit == 0
            && continuation & p.bit == 0
        {
            return false;
        }
        if let Some(s) = suffix
            && flags & s.bit == 0
            && prefix.is_none_or(|p| p.cont & s.bit == 0)
        {
            return false;
        }
        if let (Some(i), Some(o)) = (inner, outer)
            && i.cont & o.bit == 0
        {
            return false;
        }
        if let (Some(p), Some(s)) = (prefix, suffix)
            && !(p.cross && s.cross)
            && p.cont & s.bit == 0
            && continuation & p.bit == 0
        {
            return false;
        }
        let p_cont = prefix.map_or(0, |r| r.cont);
        let s_cont = outer.or(inner).map_or(0, |r| r.cont);
        let combined = flags | p_cont | continuation;
        if combined & (self.special[FORBIDDEN] | lookup.excluded) != 0 {
            return false;
        }
        if prefix.is_none() && suffix.is_none() && flags & self.special[NEED_AFFIX] != 0 {
            return false;
        }
        if suffix.is_none() && p_cont & self.special[NEED_AFFIX] != 0 {
            return false;
        }
        if prefix.is_none() && s_cont & self.special[NEED_AFFIX] != 0 {
            return false;
        }
        if (p_cont & self.special[CIRCUMFIX] != 0) != (continuation & self.special[CIRCUMFIX] != 0) {
            return false;
        }
        if lookup.case_fallback && combined & self.special[KEEP_CASE] != 0 && !(lookup.sharp_case && root.contains('ß')) {
            return false;
        }
        if let Some(part) = lookup.part {
            // Two suffixes inside a compound require COMPOUNDMORESUFFIXES. That directive
            // is deliberately unsupported, so ordinary compound parts allow one suffix.
            if inner.is_some() {
                return false;
            }
            let position = match part {
                Part::First => BEGIN,
                Part::Middle => MIDDLE,
                Part::Last => END,
            };
            if combined & (self.special[COMPOUND] | self.special[position]) == 0 {
                return false;
            }
            if part != Part::First && prefix.is_some() && p_cont & self.special[PERMIT] == 0 {
                return false;
            }
            if part != Part::Last && suffix.is_some() && continuation & self.special[PERMIT] == 0 {
                return false;
            }
        } else if combined & self.special[ONLY_COMPOUND] != 0 {
            return false;
        }
        true
    }

    fn matched_stem(&self, root: &str, applied: Applied<'_>, lookup: Lookup<'_>) -> bool {
        lookup.spend()
            && self
                .stem_index(root, lookup)
                .is_some_and(|index| self.masks(index).take_while(|_| lookup.spend()).any(|flags| self.accepts(root, flags, applied, lookup)))
    }

    /// Strip zero, one or two suffixes. Rules are indexed by their appended string, so a
    /// compound probe does not scan the dictionary or every suffix rule.
    fn unprefixed(&self, w: &str, prefix: Option<&Rule>, lookup: Lookup<'_>) -> bool {
        if self.matched_stem(w, Applied { prefix, ..Applied::default() }, lookup) {
            return true;
        }
        self.suffix_matches(w, Applied { prefix, ..Applied::default() }, lookup, MAX_CHAIN)
    }

    fn suffix_matches(&self, w: &str, applied: Applied<'_>, lookup: Lookup<'_>, remaining: u8) -> bool {
        if remaining == 0 {
            return false;
        }
        let mut root = String::with_capacity(w.len().saturating_add(8));
        let cuts = std::iter::once(w.len()).chain(w.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0));
        for b in cuts.take(self.max_sfx_chars.saturating_add(1)) {
            let (Some(base), Some(add)) = (w.get(..b), w.get(b..)) else { continue };
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for group in groups {
                root.clear();
                root.push_str(base);
                root.push_str(&group.strip);
                if root.len() > MAX_WORD_BYTES {
                    continue;
                }
                for rule in group.rules.iter().filter_map(|&i| self.suffixes.get(i as usize)) {
                    if !lookup.spend() {
                        return false;
                    }
                    if !rule.cond_matches(&root, true) || applied.outer.is_some_and(|outer| rule.cont & outer.bit == 0) {
                        continue;
                    }
                    let next =
                        if applied.outer.is_some() { Applied { inner: Some(rule), ..applied } } else { Applied { outer: Some(rule), ..applied } };
                    if self.matched_stem(&root, next, lookup) {
                        return true;
                    }
                    if remaining > 1 && self.suffix_matches(&root, next, lookup, remaining - 1) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn simple(&self, w: &str, lookup: Lookup<'_>) -> bool {
        if !lookup.spend() {
            return false;
        }
        if self.unprefixed(w, None, lookup) {
            return true;
        }
        let mut root = String::with_capacity(w.len().saturating_add(8));
        for rule in &self.prefixes {
            if !lookup.spend() {
                return false;
            }
            let Some(rest) = w.strip_prefix(&*rule.add) else { continue };
            if rest.is_empty() {
                continue;
            }
            root.clear();
            root.push_str(&rule.strip);
            root.push_str(rest);
            if root.len() <= MAX_WORD_BYTES && rule.cond_matches(&root, false) && self.unprefixed(&root, Some(rule), lookup) {
                return true;
            }
        }
        false
    }

    /// Compounds use exact surface casing, including the dictionary's explicit lowercase
    /// compound-only noun entries. Never lowercase a whole German word to find its roots.
    fn compound(&self, w: &str, lookup: Lookup<'_>) -> bool {
        if !lookup.spend() {
            return false;
        }
        if self.special[COMPOUND] | self.special[BEGIN] == 0 || self.controls.compound_max < 2 {
            return false;
        }
        let cuts: Vec<usize> = w.char_indices().map(|(i, _)| i).chain(std::iter::once(w.len())).collect();
        let count = cuts.len().saturating_sub(1);
        let minimum = self.controls.compound_min;
        if count < minimum.saturating_mul(2) {
            return false;
        }
        let max_parts = self.controls.compound_max.min(count / minimum);
        let mut reachable = vec![vec![false; count + 1]; max_parts + 1];
        if let Some(start) = reachable.first_mut().and_then(|row| row.first_mut()) {
            *start = true;
        }
        // A component's validity depends on its position, not the number of preceding parts.
        // Cache it so ambiguous segmentations remain polynomial even for hostile input.
        let mut cache = HashMap::new();
        for parts in 0..max_parts {
            for start in 0..count {
                if !reachable.get(parts).and_then(|row| row.get(start)).copied().unwrap_or(false) {
                    continue;
                }
                for end in start.saturating_add(minimum)..=count {
                    if !lookup.spend() {
                        return false;
                    }
                    if (end == count && parts == 0) || (end != count && count - end < minimum) {
                        continue;
                    }
                    let position = if end == count {
                        Part::Last
                    } else if parts == 0 {
                        Part::First
                    } else {
                        Part::Middle
                    };
                    let Some(component) = cuts.get(start).zip(cuts.get(end)).and_then(|(&a, &b)| w.get(a..b)) else {
                        continue;
                    };
                    let key = (start, end, position as u8);
                    let correct = *cache.entry(key).or_insert_with(|| self.simple(component, Lookup { part: Some(position), ..lookup }));
                    if correct {
                        if end == count {
                            return true;
                        }
                        if let Some(slot) = reachable.get_mut(parts + 1).and_then(|row| row.get_mut(end)) {
                            *slot = true;
                        }
                    }
                }
            }
        }
        false
    }

    fn exact(&self, w: &str, lookup: Lookup<'_>) -> bool {
        !w.is_empty()
            && w.len() <= MAX_WORD_BYTES
            && lookup.spend()
            && !self.forbidden_word(w, lookup)
            && (self.simple(w, lookup) || self.compound(w, lookup))
    }

    /// Is `w` exactly (case and all) a standalone stem, affixed form or valid compound?
    pub fn contains(&self, w: &str) -> bool {
        let work = Cell::new(MAX_LOOKUP_WORK);
        !w.is_empty() && w.len() <= MAX_WORD_BYTES && self.exact(w, Lookup { work: Some(&work), ..Lookup::default() })
    }

    /// Exactly spelled dictionary words eligible for suggestions (NOSUGGEST stems excluded).
    pub fn contains_suggestible(&self, w: &str) -> bool {
        let work = Cell::new(MAX_LOOKUP_WORK);
        !w.is_empty()
            && w.len() <= MAX_WORD_BYTES
            && self.exact(w, Lookup { work: Some(&work), excluded: self.special[NO_SUGGEST], ..Lookup::default() })
    }

    fn check_case(&self, w: &str, lookup: Lookup<'_>) -> bool {
        if self.forbidden_word(w, lookup) {
            return false;
        }
        if self.exact(w, lookup) {
            return true;
        }
        let lookup = Lookup { case_fallback: true, sharp_case: self.controls.check_sharps, ..lookup };
        match casing(w) {
            Casing::Lower | Casing::Mixed => false,
            Casing::Initial => self.exact(&w.to_lowercase(), lookup),
            Casing::Upper => {
                let lower = w.to_lowercase();
                if self.exact(&capitalize(&lower), lookup) || self.exact(&lower, lookup) {
                    return true;
                }
                if !self.controls.check_sharps {
                    return false;
                }
                // SS can denote ß in uppercase German. Try a bounded number of combinations;
                // ordinary lowercase `ss` remains distinct from `ß`.
                let mut variants = vec![lower];
                let mut at = 0;
                while at < variants.len() && variants.len() < 64 {
                    let Some(current) = variants.get(at).cloned() else { break };
                    for (offset, _) in current.match_indices("ss") {
                        let Some((before, after)) = current.get(..offset).zip(current.get(offset + 2..)) else { continue };
                        let candidate = format!("{before}ß{after}");
                        if self.exact(&candidate, lookup) || self.exact(&capitalize(&candidate), lookup) {
                            return true;
                        }
                        if variants.len() < 64 && !variants.contains(&candidate) {
                            variants.push(candidate);
                        }
                    }
                    at += 1;
                }
                false
            }
        }
    }

    /// Hunspell casing, KEEPCASE and CHECKSHARPS, followed by explicitly configured BREAK
    /// points. Break checking is bounded and each component retains its own case rules.
    pub fn check(&self, w: &str) -> bool {
        if w.is_empty() || w.len() > MAX_WORD_BYTES {
            return false;
        }
        let work = Cell::new(MAX_LOOKUP_WORK);
        self.check_breaks(w, 0, &mut HashMap::new(), Lookup { work: Some(&work), ..Lookup::default() })
    }

    /// Whether a suggestion is correct in its displayed case and has an eligible stem.
    pub fn can_suggest(&self, w: &str) -> bool {
        if w.is_empty() || w.len() > MAX_WORD_BYTES {
            return false;
        }
        let work = Cell::new(MAX_LOOKUP_WORK);
        self.check_breaks(w, 0, &mut HashMap::new(), Lookup { work: Some(&work), excluded: self.special[NO_SUGGEST], ..Lookup::default() })
    }

    fn check_breaks<'a>(&self, w: &'a str, depth: u8, memo: &mut HashMap<(&'a str, u8), bool>, lookup: Lookup<'_>) -> bool {
        if !lookup.spend() {
            return false;
        }
        if let Some(&correct) = memo.get(&(w, depth)) {
            return correct;
        }
        let correct = !self.forbidden_word(w, lookup) && self.breaks_uncached(w, depth, memo, lookup);
        memo.insert((w, depth), correct);
        correct
    }

    fn breaks_uncached<'a>(&self, w: &'a str, depth: u8, memo: &mut HashMap<(&'a str, u8), bool>, lookup: Lookup<'_>) -> bool {
        if self.check_case(w, lookup) {
            return true;
        }
        if depth >= 8 {
            return false;
        }
        for pattern in &self.controls.breaks {
            if !lookup.spend() {
                return false;
            }
            if let Some(start) = pattern.strip_prefix('^') {
                if !start.is_empty()
                    && let Some(rest) = w.strip_prefix(start)
                    && !rest.is_empty()
                    && self.check_breaks(rest, depth + 1, memo, lookup)
                {
                    return true;
                }
            } else if let Some(end) = pattern.strip_suffix('$') {
                if !end.is_empty()
                    && let Some(rest) = w.strip_suffix(end)
                    && !rest.is_empty()
                    && self.check_breaks(rest, depth + 1, memo, lookup)
                {
                    return true;
                }
            } else if !pattern.is_empty() {
                for (offset, _) in w.match_indices(pattern) {
                    if !lookup.spend() {
                        return false;
                    }
                    let Some((left, right)) = w.get(..offset).zip(w.get(offset + pattern.len()..)) else { continue };
                    if !left.is_empty()
                        && !right.is_empty()
                        && self.check_breaks(left, depth + 1, memo, lookup)
                        && self.check_breaks(right, depth + 1, memo, lookup)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Every form of the stem `stem` (itself included), unsorted, possibly with repeats.
    pub fn forms(&self, stem: &str) -> Vec<String> {
        let Some(f) = self.stem_flags(stem) else { return Vec::new() };
        let mut out = vec![stem.to_string()];
        let mut cross_sfx = Vec::new();
        // Suffix forms queued with the continuation classes they carry, for chained expansion.
        let mut chain: Vec<(String, u128)> = Vec::new();
        for r in &self.suffixes {
            if f & r.bit == 0 || !r.cond_matches(stem, true) {
                continue;
            }
            let Some(base) = stem.strip_suffix(&*r.strip) else { continue };
            let form = format!("{base}{}", r.add);
            if r.cross {
                cross_sfx.push(form.clone());
            }
            if r.cont != 0 {
                chain.push((form.clone(), r.cont));
            }
            out.push(form);
        }
        // Continuation classes chain suffixes (`workable/B` → `workables`), up to [`MAX_CHAIN`]
        // suffixes in all — the same bound the checker walks.
        for _ in 0..INNER_CHAIN {
            let level = std::mem::take(&mut chain);
            if level.is_empty() {
                break;
            }
            for (form, flags) in level {
                for r in &self.suffixes {
                    if flags & r.bit == 0 || !r.cond_matches(&form, true) {
                        continue;
                    }
                    let Some(base) = form.strip_suffix(&*r.strip) else { continue };
                    let next = format!("{base}{}", r.add);
                    if r.cont != 0 {
                        chain.push((next.clone(), r.cont));
                    }
                    out.push(next);
                }
            }
        }
        for r in &self.prefixes {
            if f & r.bit == 0 || !r.cond_matches(stem, false) {
                continue;
            }
            let Some(rest) = stem.strip_prefix(&*r.strip) else { continue };
            out.push(format!("{}{rest}", r.add));
            if r.cross {
                for s in &cross_sfx {
                    if let Some(rest) = s.strip_prefix(&*r.strip) {
                        out.push(format!("{}{rest}", r.add));
                    }
                }
            }
        }
        out.retain(|form| self.contains(form));
        out
    }

    /// The `TRY` letters, most frequent first (suggestions try these).
    pub fn try_chars(&self) -> &str {
        &self.try_chars
    }

    /// The `KEY` keyboard rows, separated by `|` (adjacent letters are likely mistypes).
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The `MAP` groups: letters often confused with each other (`aą`, `zżź`, `oóu`…).
    pub fn map(&self) -> &[Vec<char>] {
        &self.map
    }

    /// The `REP` table: common misspellings, `(wrong, right)`.
    pub fn rep(&self) -> &[(String, String)] {
        &self.rep
    }

    /// The provenance and licence lines.
    pub fn comments(&self) -> &[String] {
        &self.comments
    }

    /// Serialize to WordCraft's compact format: raw-deflate of `WCAFF1`, the comments, the
    /// tables and rules as text lines, a `WORDS` line, then one line per stem, front-coded
    /// against the previous one (a byte with the shared prefix length in bytes, the rest of the
    /// word, `/` and its flags if any).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        out.push_str(MAGIC);
        out.push('\n');
        for c in &self.comments {
            out.push_str("# ");
            out.push_str(c.trim());
            out.push('\n');
        }
        if !self.try_chars.is_empty() {
            out.push_str(&format!("TRY {}\n", self.try_chars));
        }
        if !self.key.is_empty() {
            out.push_str(&format!("KEY {}\n", self.key));
        }
        for m in &self.map {
            out.push_str(&format!("MAP {}\n", m.iter().collect::<String>()));
        }
        for (a, b) in &self.rep {
            out.push_str(&format!("REP {} {}\n", a.replace(' ', "_"), b.replace(' ', "_")));
        }
        for (directive, flag) in FLAG_DIRECTIVES.iter().zip(&self.controls.flags) {
            if let Some(flag) = flag {
                out.push_str(&format!("{directive} {flag}\n"));
            }
        }
        if self.controls.check_sharps {
            out.push_str("CHECKSHARPS\n");
        }
        out.push_str(&format!("COMPOUNDMIN {}\nCOMPOUNDWORDMAX {}\n", self.controls.compound_min, self.controls.compound_max));
        out.push_str(&format!("BREAK {}\n", self.controls.breaks.len()));
        for pattern in &self.controls.breaks {
            out.push_str(&format!("BREAK {pattern}\n"));
        }
        let z = |s: &str| if s.is_empty() { "0".to_string() } else { s.to_string() };
        for (kind, rules) in [("PFX", &self.prefixes), ("SFX", &self.suffixes)] {
            for r in rules {
                let y = if r.cross { "Y" } else { "N" };
                // Continuation classes ride on the appended field, as in the `.aff` source.
                let mut add = z(&r.add);
                let cont = self.flag_chars_for(r.cont);
                if !cont.is_empty() {
                    add.push('/');
                    add.push_str(&cont);
                }
                out.push_str(&format!("{kind} {} {y} {} {} {}\n", r.flag, z(&r.strip), add, r.cond_text));
            }
        }
        out.push_str("WORDS\n");
        let mut raw = out.into_bytes();
        let mut prev: &[u8] = &[];
        for i in 0..self.len() {
            let Some(w) = self.word(i) else { continue };
            let b = w.as_bytes();
            let p = prev.iter().zip(b).take_while(|(x, y)| x == y).count().min(255);
            raw.push(p as u8);
            raw.extend_from_slice(b.get(p..).unwrap_or_default());
            let masks: Vec<u128> = self.masks(i as u32).collect();
            for (n, mask) in masks.into_iter().enumerate() {
                if n != 0 {
                    raw.push(b.len().min(255) as u8);
                }
                if mask != 0 {
                    raw.push(b'/');
                    raw.extend_from_slice(self.flag_chars_for(mask).as_bytes());
                }
                raw.push(b'\n');
            }
            prev = b;
        }
        miniz_oxide::deflate::compress_to_vec(&raw, 10)
    }

    /// Read data written by [`AffixDict::to_bytes`].
    pub fn from_bytes(data: &[u8]) -> Result<AffixDict, String> {
        let raw =
            miniz_oxide::inflate::decompress_to_vec_with_limit(data, 64 << 20).map_err(|e| format!("affix dictionary: inflate failed: {e:?}"))?;
        let words_at = raw.windows(7).position(|w| w == b"\nWORDS\n").ok_or("affix dictionary: no WORDS section")?;
        let head = std::str::from_utf8(raw.get(..words_at).unwrap_or_default()).map_err(|_| "affix dictionary: bad utf-8 in the rules")?;
        let mut lines = head.lines();
        if lines.next() != Some(MAGIC) {
            return Err("affix dictionary: bad header".into());
        }
        let mut rules = Rules::default();
        let mut comments = Vec::new();
        for line in lines {
            if let Some(c) = line.strip_prefix("# ") {
                comments.push(c.to_string());
                continue;
            }
            let f: Vec<&str> = line.split_whitespace().collect();
            let at = |k: usize| f.get(k).copied().unwrap_or("");
            if rules.controls.parse(at(0), at(1))? {
                continue;
            }
            match at(0) {
                "TRY" => rules.try_chars = at(1).to_string(),
                "KEY" => rules.key = at(1).to_string(),
                "MAP" => rules.map.push(at(1).chars().collect()),
                "REP" => rules.rep.push((at(1).replace('_', " "), at(2).replace('_', " "))),
                kind @ ("PFX" | "SFX") => {
                    let mut fc = at(1).chars();
                    let (Some(flag), None) = (fc.next(), fc.next()) else { return Err(format!("affix dictionary: bad flag in `{line}`")) };
                    if f.len() != 6 {
                        return Err(format!("affix dictionary: bad rule `{line}`"));
                    }
                    // Continuation classes ride on the appended field, as in the `.aff` source.
                    let (add, cont) = match at(4).split_once('/') {
                        Some((a, c)) => (zero_empty(a), c.to_string()),
                        None => (zero_empty(at(4)), String::new()),
                    };
                    if at(3).len() > MAX_WORD_BYTES || add.len() > MAX_WORD_BYTES || cont.chars().count() > 128 {
                        return Err("affix dictionary: excessive affix length".to_string());
                    }
                    let rule: RawRule = (flag, at(2) == "Y", zero_empty(at(3)), add, cont, at(5).to_string());
                    if kind == "SFX" {
                        rules.suffixes.push(rule);
                    } else {
                        rules.prefixes.push(rule);
                    }
                }
                "" => {}
                other => return Err(format!("affix dictionary: unknown line `{other}`")),
            }
        }
        let body = raw.get(words_at + 7..).unwrap_or_default();
        let mut stems: Vec<(String, String)> = Vec::new();
        let mut cur: Vec<u8> = Vec::new();
        let mut i = 0;
        while let Some(&p) = body.get(i) {
            let rest = body.get(i + 1..).unwrap_or_default();
            let len = rest.iter().position(|&b| b == b'\n').ok_or("affix dictionary: truncated")?;
            let line = rest.get(..len).unwrap_or_default();
            i += len + 2;
            let p = p as usize;
            if p > cur.len() {
                return Err("affix dictionary: bad prefix".into());
            }
            let (tail, flags) = match line.iter().position(|&b| b == b'/') {
                Some(k) => (line.get(..k).unwrap_or_default(), line.get(k + 1..).unwrap_or_default()),
                None => (line, &[][..]),
            };
            cur.truncate(p);
            cur.extend_from_slice(tail);
            let word = std::str::from_utf8(&cur).map_err(|_| "affix dictionary: bad utf-8 in a stem")?;
            let flags = std::str::from_utf8(flags).map_err(|_| "affix dictionary: bad utf-8 in flags")?;
            if word.is_empty() || word.len() > MAX_WORD_BYTES {
                return Err("affix dictionary: bad stem".into());
            }
            stems.push((word.to_string(), flags.to_string()));
        }
        AffixDict::build(rules, stems, comments)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const AFF: &str = "SET UTF-8\nTRY aeoiułkr\n\nPFX b Y 1\nPFX b   0   nie   .\n\nSFX a Y 3\nSFX a   0   y     [^y]\nSFX a   ek  ka    ek\nSFX a   0   em    .\n\nSFX c N 1\nSFX c   a   ą     a\n\nMAP 2\nMAP aą\nMAP oóu\nREP 2\nREP ż rz\nREP u ó\n";
    const DIC: &str = "4\nkot/ab\npiesek/a\nkrowa/c\nWarszawa/c\n";

    fn dict() -> AffixDict {
        AffixDict::from_hunspell(AFF, DIC, &["test data"]).unwrap()
    }

    #[test]
    fn affix_headers_allow_comments_but_rule_conditions_stay_literal() {
        let aff = "SET UTF-8\nPFX X Y 1 # prefix comment\nPFX X 0 ne . # rule comment\nSFX y Y 1 # suffix comment\nSFX y 0 y . # rule comment\n";
        let d = AffixDict::from_hunspell(aff, "1\nstrom/Xy\n", &[]).unwrap();
        assert_eq!(d.rule_count(), 2, "headers must not become affix rules");
        for w in ["strom", "stromy", "nestrom", "nestromy"] {
            assert!(d.contains(w), "{w}");
        }
        for w in ["stromY1", "stromN1", "X", "y"] {
            assert!(!d.contains(w), "{w}");
        }
        let hash = AffixDict::from_hunspell("SFX H Y 1\nSFX H 0 # .\nTRY a#b\nKEY a#b\n", "1\nstrom/H\n", &[]).unwrap();
        assert!(hash.contains("strom#"), "a hash in an affix field is literal");
        let literal = AffixDict::from_hunspell(
            "PFX P Y 1 # header\nPFX P 0 pre #\nSFX S Y 1 # header\nSFX S 0 post #\n",
            "3\n#stem/P\nstem#/S\nstem/PS\n",
            &[],
        )
        .unwrap();
        assert!(literal.contains("pre#stem") && literal.contains("stem#post"));
        assert!(!literal.contains("prestem") && !literal.contains("stempost"), "# must remain a literal condition");

        assert_eq!(hash.try_chars(), "a#b");
        assert_eq!(hash.key(), "a#b");
        assert_eq!(AffixDict::from_bytes(&hash.to_bytes()).unwrap().key(), "a#b");
    }

    /// Shared hand-written fixture: collectors must reject every path to NOSUGGEST forms.
    pub(crate) fn assert_nosuggest_is_honoured(suggest: impl Fn(&AffixDict, &str, usize) -> Vec<String>) {
        let aff = "SET UTF-8\nNOSUGGEST N\nFORBIDDENWORD F\nTRY abeklmnostxy\nMAP 1\nMAP ae\nKEY ak\nREP 2\nREP xy toka\nREP zz normal\nPFX X Y 1\nPFX X 0 pre .\nSFX A Y 1\nSFX A 0 ble/B .\nSFX B Y 1\nSFX B 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "4\ntoka/NXA\nnormal/XA\nTkname/N\nbanned/F\n", &[]).unwrap();
        for word in d.forms("toka").into_iter().chain(["TOKA".into(), "Toka".into(), "Tkname".into(), "TKNAME".into()]) {
            assert!(d.check(&word), "NOSUGGEST is still correct: {word}");
            assert!(!d.can_suggest(&word), "NOSUGGEST form: {word}");
        }
        for input in ["tokax", "tokablesx", "pretokax", "pretokablesx", "toka", "TOKA", "tkname", "teka", "xy", "normaltoka", "toak", "taak"] {
            let results = suggest(&d, input, 32);
            assert!(results.iter().all(|word| word.split(' ').all(|part| d.can_suggest(part))), "{input}: {results:?}");
            assert!(results.iter().all(|word| !word.to_lowercase().contains("toka") && word.to_lowercase() != "tkname"), "{input}: {results:?}");
        }
        assert!(suggest(&d, "norrmal", 8).contains(&"normal".into()));
        assert!(suggest(&d, "zz", 8).contains(&"normal".into()));
        // Filtering happens before display casing: ordinary mixed-case stems retain the
        // existing all-caps suggestion behaviour, with and without a NOSUGGEST declaration.
        for aff in ["", "NOSUGGEST N\n"] {
            let ordinary = AffixDict::from_hunspell(aff, "2\nmixCase\nhidden/N\n", &[]).unwrap();
            assert!(suggest(&ordinary, "MIXCASX", 8).contains(&"MIXCASE".into()));
        }
    }

    #[test]
    fn nosuggest_is_correct_but_not_suggestible_and_survives_compact_format() {
        let d = AffixDict::from_hunspell("NOSUGGEST N\nSFX S Y 1\nSFX S 0 s .\n", "2\nsecret/NS\nnormal/S\n", &[]).unwrap();
        for dict in [&d, &AffixDict::from_bytes(&d.to_bytes()).unwrap()] {
            for word in ["secret", "secrets", "Secret", "SECRETS"] {
                assert!(dict.check(word), "{word}");
                assert!(!dict.can_suggest(word), "{word}");
            }
            assert!(dict.contains_suggestible("normal") && dict.can_suggest("NORMALS"));
        }
        assert!(AffixDict::from_hunspell("NOSUGGEST NN\n", "1\nword\n", &[]).is_err());
    }

    #[test]
    fn stems_suffixes_prefixes_and_cross_products() {
        let d = dict();
        assert_eq!(d.len(), 4);
        for w in ["kot", "koty", "kotem", "piesek", "pieska", "piesekem", "krowa", "krową", "niekot", "niekoty", "niekotem", "Warszawa", "Warszawą"]
        {
            assert!(d.contains(w), "{w}");
        }
        for w in ["kotą", "pieseka", "pieskem", "niepiesek", "niekrową", "krowy", "warszawa", "", "nie", "y", "ka"] {
            assert!(!d.contains(w), "{w}");
        }
    }

    #[test]
    fn case_variants() {
        let d = dict();
        assert!(d.check("Kot") && d.check("KOT") && d.check("Koty") && d.check("WARSZAWĄ") && d.check("Warszawą"));
        assert!(!d.check("warszawa") && !d.check("kOT") && !d.check("KoT"));
        assert_eq!(casing("Kot"), Casing::Initial);
        assert_eq!(casing("KOT"), Casing::Upper);
        assert_eq!(casing("K"), Casing::Initial);
        assert_eq!(casing("kot"), Casing::Lower);
        assert_eq!(casing("McDonald"), Casing::Mixed);
        assert_eq!(capitalize("żółw"), "Żółw");
    }

    #[test]
    fn forms_are_all_accepted() {
        let d = dict();
        let mut forms = d.forms("kot");
        forms.sort();
        forms.dedup();
        assert_eq!(forms, ["kot", "kotem", "koty", "niekot", "niekotem", "niekoty"]);
        for stem in d.stems().map(str::to_string).collect::<Vec<_>>() {
            for f in d.forms(&stem) {
                assert!(d.contains(&f), "{stem} → {f}");
            }
        }
    }

    #[test]
    fn compact_format_round_trips() {
        let d = dict();
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert_eq!(back.len(), d.len());
        assert_eq!(back.rule_count(), d.rule_count());
        assert_eq!(back.comments(), ["test data"]);
        assert_eq!(back.rep(), d.rep());
        assert_eq!(back.map(), d.map());
        assert_eq!(back.try_chars(), "aeoiułkr");
        for w in ["kotem", "niekoty", "pieska", "krową", "Warszawą"] {
            assert!(back.contains(w), "{w}");
        }
        assert!(!back.contains("kotą"));
    }

    #[test]
    fn flag_utf8_and_default_modes_are_accepted() {
        // `FLAG UTF-8` (VERO pt_BR) and `FLAG DEFAULT` are per-character flag modes: the same
        // mode the reader already assumes. Only `long` and `num` stay unsupported.
        for mode in ["UTF-8", "DEFAULT"] {
            let aff = format!("SET UTF-8\nFLAG {mode}\nSFX a Y 1\nSFX a   0   y   [^y]\n");
            let d = AffixDict::from_hunspell(&aff, "1\nkot/a\n", &[]).unwrap();
            assert!(d.contains("kot") && d.contains("koty"), "{mode}");
        }
        assert!(AffixDict::from_hunspell("FLAG UTF-8\n", "0\n", &[]).is_ok(), "FLAG alone is harmless");
    }

    #[test]
    fn more_than_64_distinct_flags_load() {
        // VERO pt_BR uses 106 distinct flags (FLAG UTF-8). Build one with every basic Latin
        // letter, the digits and eight accented letters: 70 flags, 64 < n <= 128.
        let mut flags = String::new();
        let mut aff = String::from("SET UTF-8\nFLAG UTF-8\n");
        let chars: Vec<char> = ('a'..='z').chain('A'..='Z').chain('0'..='9').chain(['á', 'é', 'í', 'ó', 'ú', 'ç', 'ã', 'õ']).collect();
        assert_eq!(chars.len(), 70);
        for c in chars {
            aff.push_str(&format!("SFX {c} N 1\nSFX {c}   0   {c}   .\n"));
            flags.push(c);
        }
        let d = AffixDict::from_hunspell(&aff, &format!("1\nverbo/{flags}\n"), &[]).unwrap();
        assert!(d.contains("verbo"));
        assert!(d.contains("verboa") && d.contains("verboz") && d.contains("verbo9"));
        assert!(d.contains("verboç") && d.contains("verboã"), "multi-byte UTF-8 flags");
        assert!(!d.contains("verboß"), "ß is not a flag and not a stem");
    }

    #[test]
    fn forbidden_words_are_rejected() {
        // VERO pt_BR marks spellings it must reject (flag ý under FLAG UTF-8) as stems carrying
        // FORBIDDENWORD: listed, but never correct and never inflected.
        let aff = "SET UTF-8\nFLAG UTF-8\nFORBIDDENWORD ý\nSFX a Y 1\nSFX a   0   y   [^y]\n";
        let d = AffixDict::from_hunspell(aff, "2\nkot/a\nimundo/ý\n", &[]).unwrap();
        assert!(d.check("kot") && d.check("koty") && d.check("Kot") && d.check("KOT"));
        assert!(!d.check("imundo") && !d.check("Imundo") && !d.check("IMUNDO"));
        assert!(!d.contains("imundoy"), "a forbidden stem must not inflect");
        assert!(d.forms("imundo").is_empty());
    }

    #[test]
    fn continuation_classes_chain_suffixes() {
        // `SFX A 0 able/B`: the form derived by A carries flag B, so a suffix of class B may
        // follow (workable → workables). The bare stem has A only, so `works` stays wrong.
        let aff = "SET UTF-8\nFLAG UTF-8\nSFX A N 1\nSFX A   0   able/B   .\nSFX B N 1\nSFX B   0   s   .\n";
        let d = AffixDict::from_hunspell(aff, "1\nwork/A\n", &[]).unwrap();
        assert!(d.contains("work") && d.contains("workable") && d.contains("workables"));
        assert!(!d.contains("works"), "the stem has A, not B");
        assert!(!d.contains("workabless"), "B declares no continuation");
        assert!(d.check("Workable") && d.check("WORKABLES"));
    }

    #[test]
    fn continuation_chains_stop_at_two_suffixes() {
        // Hunspell strips at most two suffixes: work + able/B + s/C is a word, a third (C) is not.
        let aff = "SET UTF-8\nFLAG UTF-8\nSFX A N 1\nSFX A   0   able/B   .\nSFX B N 1\nSFX B   0   s/C   .\nSFX C N 1\nSFX C   0   x   .\n";
        let d = AffixDict::from_hunspell(aff, "1\nwork/A\n", &[]).unwrap();
        assert!(d.contains("workable") && d.contains("workables"));
        assert!(!d.contains("workablesx"), "three chained suffixes exceed the limit");
        assert!(!d.forms("work").iter().any(|f| f == "workablesx"));
    }

    #[test]
    fn forms_include_continuation_chains() {
        let aff = "SET UTF-8\nFLAG UTF-8\nSFX A N 1\nSFX A   0   able/B   .\nSFX B N 1\nSFX B   0   s   .\n";
        let d = AffixDict::from_hunspell(aff, "1\nwork/A\n", &[]).unwrap();
        let mut f = d.forms("work");
        f.sort();
        f.dedup();
        assert_eq!(f, ["work", "workable", "workables"]);
        for form in d.forms("work") {
            assert!(d.contains(&form), "every listed form is accepted: {form}");
        }
    }

    #[test]
    fn compact_format_round_trips_continuations_and_forbidden() {
        let aff = "SET UTF-8\nFLAG UTF-8\nFORBIDDENWORD ý\nSFX A N 1\nSFX A   0   able/B   .\nSFX B N 1\nSFX B   0   s   .\n";
        let d = AffixDict::from_hunspell(aff, "2\nwork/A\nimundo/ý\n", &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.contains("work") && back.contains("workable") && back.contains("workables"));
        assert!(!back.contains("works"), "the stem has A, not B, after a round trip");
        assert!(!back.check("imundo") && !back.contains("imundoy"), "forbidden survives a round trip");
    }

    #[test]
    fn dic_bom_glued_to_the_count_line_is_skipped() {
        // VERO's .dic opens with a UTF-8 BOM glued to its count line; it must not become a stem.
        let aff = "SET UTF-8\nSFX a Y 1\nSFX a   0   y   [^y]\n";
        let d = AffixDict::from_hunspell(aff, "\u{feff}1\nkot/a\n", &[]).unwrap();
        assert_eq!(d.len(), 1);
        assert!(d.contains("kot") && d.contains("koty"));
    }

    #[test]
    fn unsupported_and_broken_input_is_an_error() {
        assert!(AffixDict::from_hunspell("FLAG long\n", DIC, &[]).is_err());
        assert!(AffixDict::from_hunspell("FLAG num\n", DIC, &[]).is_err());
        assert!(AffixDict::from_hunspell("COMPOUNDRULE 1\n", DIC, &[]).is_err());
        assert!(AffixDict::from_hunspell("SFX a Y 1\nSFX a 0 y [ab\n", DIC, &[]).is_err());
        assert!(AffixDict::from_bytes(b"not deflate").is_err());
        let ok = dict().to_bytes();
        assert!(AffixDict::from_bytes(&ok[..ok.len() / 2]).is_err());
        let garbage = miniz_oxide::deflate::compress_to_vec(b"WCAFF1\nWORDS\n\x09abc\n", 6);
        assert!(AffixDict::from_bytes(&garbage).is_err(), "prefix longer than the previous word");
        let empty = AffixDict::default();
        assert!(!empty.contains("kot") && !empty.check("Kot") && empty.forms("kot").is_empty());
    }

    /// Synthetic data written for these tests; no third-party dictionary entries or rules.
    const GERMAN_AFF: &str = "SET UTF-8\nCOMPOUNDBEGIN x\nCOMPOUNDMIDDLE y\nCOMPOUNDEND z\nCOMPOUNDMIN 2\nCOMPOUNDWORDMAX 3\nONLYINCOMPOUND o\nCOMPOUNDPERMITFLAG c\nNEEDAFFIX h\nKEEPCASE w\nCIRCUMFIX f\nFORBIDDENWORD d\nCHECKSHARPS\nBREAK 2\nBREAK -\nBREAK .\nSFX A Y 1\nSFX A 0 s/xyoc .\nSFX B Y 1\nSFX B 0 0/xoc .\nPFX P Y 1\nPFX P 0 ur .\nPFX Q Y 1\nPFX Q 0 ur/c .\nSFX S Y 1\nSFX S 0 bar .\nPFX F Y 1\nPFX F 0 ge/f .\nSFX G Y 1\nSFX G 0 t/f .\n";
    const GERMAN_DIC: &str = "16\nWolke/AB\nlicht/zo\nzimmer/yo\nBuch/xPS\nhaus/zoQPS\nleise/w\nweiß/w\nMisch/Fg\nleb/hFG\narm/S\nBlock/x\nblock/yo\nTor/z\nKunst/A\nform/zo\nSonne\nzweig/zoPS\n";

    fn german_dict() -> AffixDict {
        AffixDict::from_hunspell(GERMAN_AFF, GERMAN_DIC, &[]).unwrap()
    }

    #[test]
    fn german_positional_compounds_and_linking_forms_keep_case() {
        let dictionary = german_dict();
        for word in ["Wolke", "Wolkelicht", "Wolkeslicht", "Wolkeszimmerlicht", "Kunstsform", "Buchhaus"] {
            assert!(dictionary.check(word), "{word}");
        }
        for word in ["wolke", "licht", "Wolkes", "Wolkzimmerlicht", "WolkeLicht", "Wolkeszimmerzimmerlicht", "Lichtwolke", "Bucharmbar"] {
            assert!(!dictionary.check(word), "{word}");
        }
        // No compound flags are granted merely because both halves are standalone words.
        assert!(!dictionary.check("SonneWolke"));
        assert!(dictionary.forms("Kunst").iter().all(|word| dictionary.contains(word)));
    }

    #[test]
    fn compound_affixes_need_permission_at_internal_boundaries() {
        let dictionary = german_dict();
        assert!(dictionary.contains("urBuchhaus"), "prefix at compound start");
        assert!(!dictionary.contains("Wolkeurzweig"), "internal prefix without permit");
        assert!(dictionary.contains("Wolkeurhaus"), "internal prefix with permit");
        assert!(dictionary.contains("Buchhausbar"), "suffix at compound end");
        assert!(!dictionary.contains("Buchbarhaus"), "internal suffix without permit");
        let chained =
            AffixDict::from_hunspell("COMPOUNDFLAG c\nCOMPOUNDMIN 2\nSFX A Y 1\nSFX A 0 s/Bc .\nSFX B Y 1\nSFX B 0 t/c .\n", "1\nroot/Ac\n", &[])
                .unwrap();
        assert!(chained.contains("rootst"), "standalone words may use two suffixes");
        assert!(chained.contains("rootroots"), "a compound end may use one suffix");
        assert!(!chained.contains("rootrootst"), "two suffixes need COMPOUNDMORESUFFIXES");
    }

    #[test]
    fn virtual_stems_circumfixes_and_homonyms() {
        let dictionary = german_dict();
        assert!(!dictionary.contains("leb"));
        assert!(!dictionary.contains("geleb") && !dictionary.contains("lebt"));
        assert!(dictionary.contains("gelebt"));
        let homonyms =
            AffixDict::from_hunspell("NEEDAFFIX h\nONLYINCOMPOUND o\nSFX A N 1\nSFX A 0 s .\n", "3\nWort/ho\nWort/A\nWort/A\n", &[]).unwrap();
        assert_eq!(homonyms.len(), 1);
        assert!(homonyms.contains("Wort") && homonyms.contains("Worts"));
        let back = AffixDict::from_bytes(&homonyms.to_bytes()).unwrap();
        assert!(back.contains("Wort") && back.contains("Worts"));
        let need_on_affix =
            AffixDict::from_hunspell("NEEDAFFIX h\nPFX P Y 1\nPFX P 0 un/h .\nSFX S Y 1\nSFX S 0 bar .\n", "1\nles/PS\n", &[]).unwrap();
        assert!(!need_on_affix.contains("unles"));
        assert!(need_on_affix.contains("unlesbar"));
    }

    #[test]
    fn keepcase_and_german_sharp_s_only_relax_legal_capitals() {
        let dictionary = german_dict();
        assert!(dictionary.check("leise"));
        assert!(!dictionary.check("Leise") && !dictionary.check("LEISE"));
        for word in ["weiß", "Weiß", "WEISS", "WEIẞ"] {
            assert!(dictionary.check(word), "{word}");
        }
        assert!(!dictionary.check("weiss") && !dictionary.check("Weiss"));
        let normal = AffixDict::from_hunspell("", "1\nweiß\n", &[]).unwrap();
        assert!(!normal.check("WEISS"), "SS conversion requires CHECKSHARPS");
    }

    #[test]
    fn compound_flag_limits_count_characters_and_forbidden_spelling_overrides_derivations() {
        let dictionary =
            AffixDict::from_hunspell("COMPOUNDFLAG c\nCOMPOUNDMIN 2\nCOMPOUNDWORDMAX 2\nFORBIDDENWORD d\n", "4\nöö/c\nää/c\nööää/d\nß/c\n", &[])
                .unwrap();
        assert!(dictionary.contains("ääöö"));
        assert!(!dictionary.check("ööää"), "explicit forbidden compound");
        assert!(!dictionary.contains("ääööää"), "word limit");
        assert!(!dictionary.contains("ßää"), "minimum is Unicode characters, not bytes");
        let affixed = AffixDict::from_hunspell("FORBIDDENWORD d\nSFX A N 1\nSFX A 0 s .\n", "2\nwork/A\nworks/d\n", &[]).unwrap();
        assert!(!affixed.contains("works") && !affixed.check("Works"));
    }

    #[test]
    fn explicit_breaks_and_all_correctness_directives_survive_compact_format() {
        let dictionary = AffixDict::from_bytes(&german_dict().to_bytes()).unwrap();
        for word in ["Wolkeslicht", "Wolkeszimmerlicht", "gelebt", "WEISS", "Wolke-Sonne", "Wolke.Sonne"] {
            assert!(dictionary.check(word), "{word}");
        }
        for word in ["Wolkes", "licht", "LEISE", "-Wolke", "Wolke-", "Wolke.Quatsch"] {
            assert!(!dictionary.check(word), "{word}");
        }
        let no_breaks = AffixDict::from_hunspell("BREAK 0\n", "2\nWolke\nSonne\n", &[]).unwrap();
        assert!(!no_breaks.check("Wolke-Sonne"));
        let forbidden = AffixDict::from_hunspell("FORBIDDENWORD d\n", "3\nfoo\nbar\nfoo-bar/d\n", &[]).unwrap();
        assert!(!forbidden.check("foo-bar"), "BREAK cannot bypass forbidden spelling");
    }

    #[test]
    fn hostile_compounds_and_ambiguous_breaks_are_bounded() {
        let dictionary = AffixDict::from_hunspell("COMPOUNDFLAG c\nCOMPOUNDMIN 1\n", "3\na/c\naa/c\nfoo\n", &[]).unwrap();
        assert!(!dictionary.check(&format!("{}b", "a".repeat(MAX_WORD_BYTES - 1))));
        assert!(!dictionary.check(&format!("{}b", "foo-".repeat(29))));
        assert!(!dictionary.check(&"a".repeat(MAX_WORD_BYTES + 1)));
        for aff in ["COMPOUNDMIN 0", "COMPOUNDWORDMAX -1", "KEEPCASE ab", "BREAK nope", "COMPOUNDMIN 99999999999999999999999999999999999"] {
            assert!(AffixDict::from_hunspell(aff, "1\na\n", &[]).is_err(), "{aff}");
        }
    }

    #[test]
    fn repeated_zero_suffix_rules_share_a_finite_lookup_budget() {
        let aff = format!("SFX A Y 1024\n{}", "SFX A 0 0/A .\n".repeat(1024));
        let dictionary = AffixDict::from_hunspell(&aff, "1\nroot/A\n", &[]).unwrap();
        let work = Cell::new(MAX_LOOKUP_WORK);
        assert!(!dictionary.check_breaks("missing", 0, &mut HashMap::new(), Lookup { work: Some(&work), ..Lookup::default() }));
        assert_eq!(work.get(), 0, "the recursive continuation paths stop at the shared budget");
        assert!(!dictionary.check("missing"));
        assert!(dictionary.check("root"), "a direct hit remains inexpensive");
    }

    proptest::proptest! {
        #[test]
        fn lookups_never_panic(s in "\\PC{0,40}", bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..200)) {
            let d = dict();
            let _ = d.check(&s);
            let _ = d.forms(&s);
            let _ = AffixDict::from_bytes(&bytes);
            let _ = AffixDict::from_bytes(&miniz_oxide::deflate::compress_to_vec(&bytes, 1));
        }
    }
}
