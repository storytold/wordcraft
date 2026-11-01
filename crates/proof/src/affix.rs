//! Hunspell-style affix dictionaries — the subset the SJP.PL Polish dictionary (`pl_PL`) uses.
//!
//! A word is correct when it is a stem, a stem with one suffix, a stem with one prefix, or a stem
//! with a prefix and a suffix that both allow cross products; each rule applies only to stems
//! carrying its flag and ending (suffixes) or starting (prefixes) as its condition says. Highly
//! inflected languages store millions of forms this way in a few hundred thousand stems: the
//! Polish dictionary's ~350k stems and ~7,500 rules accept ~4.5 million forms.
//!
//! The rules and stems come from a Hunspell `.aff`/`.dic` pair ([`AffixDict::from_hunspell`]),
//! read at runtime from the dictionaries installed on the system ([`crate::hunspell`]). They can
//! also be stored in WordCraft's compact format ([`AffixDict::to_bytes`]): the rules as text
//! lines and the stems front-coded, raw-deflated together (the Polish dictionary is under 1 MB).
//!
//! Supported: one-character flags (any Unicode character — `FLAG UTF-8`), two-character flags
//! (`FLAG long`) and numeric flags (`FLAG num`: comma-separated numbers from 1 to 65,535), up to
//! 65,536 distinct flags in all ([`FlagMode`]); `PFX`/`SFX` with strip, append and condition (`.`,
//! letters, `[abc]`, `[^abc]`), cross products, continuation classes on suffixes (at most two
//! suffixes in a row, as in Hunspell), `FORBIDDENWORD`, `NOSUGGEST`, and the suggestion tables
//! `TRY`, `KEY`, `MAP` and `REP`. Rejected: compounding, flag aliases (`AF`), `NEEDAFFIX` and the
//! other directives that change which words are correct.
//!
//! Each distinct flag gets a dense `u16` id; a stem's flags and a rule's continuation classes are
//! a sorted set of ids, interned once in a flat table, so membership is a binary search in a few
//! ids and a dictionary with thousands of flags (Serbian: 1,903) costs no more per lookup.
//!
//! Lookups never panic: stems sit in one string, found through an open-addressing hash table, and
//! a probe allocates only small scratch strings for the candidate roots it tries.

use std::collections::HashMap;
use std::hash::Hasher;

use crate::patterns::{FxHasher, FxMap};

const MAGIC: &str = "WCAFF1";
/// Longest word (in bytes) looked up; longer input is never a dictionary word.
pub const MAX_WORD_BYTES: usize = 120;
/// Empty slot in the stem hash table.
const EMPTY: u32 = u32::MAX;
/// Most flags one `.dic` entry or one continuation class field may carry. Real dictionaries use a
/// few hundred at most (Belarusian: 264); a longer field is hostile and is refused (`.aff`) or
/// skipped (`.dic`, though its `FORBIDDENWORD` and `NOSUGGEST` flags still apply to the word).
pub const MAX_FLAGS_PER_ENTRY: usize = 1024;
/// Most flags one stem may carry once its homonyms (the same word on several `.dic` lines) are
/// merged, and so most flags a stem line of the compact format may carry. Larger than
/// [`MAX_FLAGS_PER_ENTRY`] because merged sets aren't `.dic` fields; a homonym that would take the
/// set past it adds only its `FORBIDDENWORD` and `NOSUGGEST` flags, as an exclusion is never lost.
pub const MAX_FLAGS_PER_STEM: usize = 8 * MAX_FLAGS_PER_ENTRY;
/// Most distinct flags a dictionary may use: each gets a `u16` id.
const MAX_DISTINCT_FLAGS: usize = 1 << 16;

/// Directives that change which words are correct in ways this engine doesn't implement.
const UNSUPPORTED: &[&str] = &[
    "AF",
    "AM",
    "CHECKSHARPS",
    "CIRCUMFIX",
    "COMPLEXPREFIXES",
    "COMPOUNDBEGIN",
    "COMPOUNDEND",
    "COMPOUNDFLAG",
    "COMPOUNDMIDDLE",
    "COMPOUNDMIN",
    "COMPOUNDPERMITFLAG",
    "COMPOUNDRULE",
    "FULLSTRIP",
    "ICONV",
    "IGNORE",
    "KEEPCASE",
    "LEMMA_PRESENT",
    "NEEDAFFIX",
    "OCONV",
    "ONLYINCOMPOUND",
    "PSEUDOROOT",
    "SUBSTANDARD",
];

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
/// How many suffixes may sit under the outermost one: the depth `form_has_flag` walks.
const INNER_CHAIN: u8 = MAX_CHAIN - 1;
/// Most forms [`AffixDict::forms`] lists for one stem (real stems have a few hundred at most).
pub const MAX_FORMS: usize = 4096;
/// Most rules [`AffixDict::forms`] tries along continuation chains and prefix–suffix cross
/// products for one stem: real dictionaries need a few thousand, hostile ones are cut off.
const MAX_FORMS_WORK: usize = 1 << 18;

/// One prefix or suffix rule.
#[derive(Clone, Debug)]
struct Rule {
    /// The rule's flag (a dense id, see [`AffixDict::flag_keys`]).
    flag: u16,
    cross: bool,
    strip: Box<str>,
    add: Box<str>,
    /// Continuation classes (`SFX A 0 able/B`): flags the derived form carries, licensing
    /// further affixes, as a flag set id (0: none). Chained application is bounded by [`MAX_CHAIN`].
    cont: u32,
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
    /// The rules of `rules` with continuation classes: the only ones a chain can go through.
    cont_rules: Vec<u32>,
}

/// A stem list with affix rules (see the module docs).
#[derive(Clone, Debug, Default)]
pub struct AffixDict {
    /// All stems, concatenated, sorted by bytes.
    words: String,
    offs: Vec<u32>,
    /// Per stem: its flag set id.
    flags: Vec<u32>,
    /// Distinct flag sets, each a sorted run of flag ids in `set_data`: set `i` is
    /// `set_data[set_offs[i]..set_offs[i + 1]]`. Set 0 is the empty set.
    set_offs: Vec<u32>,
    set_data: Vec<u16>,
    /// The `FORBIDDENWORD` flag; stems carrying it are not words.
    forbidden: Option<u16>,
    /// Stems with NOSUGGEST are correct, but none of their forms are suggested.
    no_suggest: Option<u16>,
    /// Open-addressing hash table of stem indices (`EMPTY` = free).
    table: Vec<u32>,
    /// How the source wrote flags (kept for the compact format).
    flag_mode: FlagMode,
    /// Each flag id's flag as the source wrote it, sorted (the id is the index).
    flag_keys: Vec<FlagKey>,
    /// Whether any rule has continuation classes (lookups skip the chain walk otherwise).
    has_cont: bool,
    /// Per flag id: whether some rule's continuation classes hold it, so that a form (not only
    /// a stem) can carry it.
    cont_flag: Vec<bool>,
    prefixes: Vec<Rule>,
    suffixes: Vec<Rule>,
    /// Appended string → groups of suffix rules by stripped string.
    sfx_index: FxMap<Box<str>, Vec<SfxGroup>>,
    /// Per flag id: the suffix rules of that flag, in rule order.
    sfx_by_flag: Vec<Vec<u32>>,
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
    /// The `FORBIDDENWORD` flag: stems carrying it are listed but never correct.
    forbidden: Option<FlagKey>,
    no_suggest: Option<FlagKey>,
    /// How flags are written, from the `FLAG` line.
    mode: FlagMode,
    /// Whether a flag has been read: a later `FLAG` line can't change how the earlier ones read.
    flag_read: bool,
}

impl Rules {
    /// A `FLAG` line.
    fn set_mode(&mut self, name: &str) -> Result<(), String> {
        let mode = FlagMode::from_name(name).ok_or_else(|| format!("FLAG {name} is not supported"))?;
        if self.flag_read && mode != self.mode {
            return Err(format!("FLAG {name} after flags were read"));
        }
        self.mode = mode;
        Ok(())
    }

    /// The one flag of a rule header or a single-flag directive.
    fn one_flag(&mut self, field: &str) -> Result<FlagKey, String> {
        self.flag_read = true;
        self.mode.parse_one(field)
    }

    /// The flags of a continuation class field, checked now and read again when the rules are built.
    fn check_flags(&mut self, field: &str) -> Result<(), String> {
        self.flag_read = true;
        self.mode.parse(field).map(drop)
    }
}

/// A rule as parsed from text, before flags get ids: `(flag, cross, strip, add, cont, cond)`.
type RawRule = (FlagKey, bool, String, String, String, String);

/// Most distinct `TRY` characters kept; a hostile `.aff` could otherwise make every suggestion
/// round cost time and memory proportional to the file size.
const MAX_TRY_CHARS: usize = 128;

/// Keeps the first [`MAX_TRY_CHARS`] distinct characters of a `TRY` value, in order.
fn cap_try(s: &str) -> String {
    let mut out = String::new();
    let mut n = 0;
    for c in s.chars() {
        if n >= MAX_TRY_CHARS {
            break;
        }
        if !out.contains(c) {
            out.push(c);
            n += 1;
        }
    }
    out
}

/// `0` means empty in Hunspell's strip and append fields.
fn zero_empty(s: &str) -> String {
    if s == "0" { String::new() } else { s.to_string() }
}

/// U+FE0F VARIATION SELECTOR-16 asks for the emoji form of the character before it. A dictionary
/// that uses an emoji as a flag may write it after the flag (RLA-ES Spanish: `PFX ☎️`, `banco/S☎️`),
/// but it is not a flag of its own: it is dropped wherever flags are read.
const EMOJI_SELECTOR: char = '\u{fe0f}';

/// A flag as written in the source, before it gets a dense id: a character (one-character mode),
/// two characters (`FLAG long`, the first in the high 32 bits) or a number (`FLAG num`).
type FlagKey = u64;

/// How flags are written: the `.aff`'s `FLAG` line, also kept in the compact format. Every flag
/// field (`.dic` entries, `PFX`/`SFX` flags and continuation classes, `FORBIDDENWORD`…) is read
/// and written here, and nowhere else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlagMode {
    /// One character per flag (no `FLAG` line, `FLAG UTF-8`, `FLAG DEFAULT`).
    #[default]
    Char,
    /// Two characters per flag (`FLAG long`): `Aa`, `B1`.
    Long,
    /// Decimal numbers from 1 to 65,535 separated by commas (`FLAG num`): `12,104`. As in
    /// Hunspell, each number is the item's leading digits: Nepali writes `0/17X` for flag 17.
    Num,
}

impl FlagMode {
    /// The mode a `FLAG` line names, if this engine reads it.
    fn from_name(name: &str) -> Option<FlagMode> {
        match name {
            "UTF-8" | "DEFAULT" | "utf-8" | "default" | "UTF8" | "" => Some(FlagMode::Char),
            "long" | "LONG" => Some(FlagMode::Long),
            "num" | "NUM" => Some(FlagMode::Num),
            _ => None,
        }
    }

    /// The `FLAG` value the compact format writes (one-character mode writes no line).
    fn name(self) -> Option<&'static str> {
        match self {
            FlagMode::Char => None,
            FlagMode::Long => Some("long"),
            FlagMode::Num => Some("num"),
        }
    }

    /// The flags of a field (`.dic` flags, continuation classes), in source order, at most
    /// [`MAX_FLAGS_PER_ENTRY`]. An empty field has none.
    fn parse(self, field: &str) -> Result<Vec<FlagKey>, String> {
        self.parse_at_most(field, MAX_FLAGS_PER_ENTRY)
    }

    /// The flags of a field, at most `cap` of them.
    fn parse_at_most(self, field: &str, cap: usize) -> Result<Vec<FlagKey>, String> {
        let mut out = Vec::new();
        let mut push = |k: FlagKey| {
            if out.len() >= cap {
                return Err(format!("more than {cap} flags in `{}`", field.chars().take(40).collect::<String>()));
            }
            out.push(k);
            Ok(())
        };
        match self {
            FlagMode::Char => {
                for c in field.chars().filter(|&c| c != EMOJI_SELECTOR) {
                    push(FlagKey::from(c))?;
                }
            }
            FlagMode::Long => {
                let mut chars = field.chars();
                while let Some(a) = chars.next() {
                    let b = chars.next().ok_or_else(|| format!("long flags `{field}` have an odd number of characters"))?;
                    push((FlagKey::from(a) << 32) | FlagKey::from(b))?;
                }
            }
            FlagMode::Num if field.is_empty() => {}
            FlagMode::Num => {
                for item in field.split(',') {
                    // The leading digits (no sign), 1 to 65,535: at most five digits.
                    let digits = item.bytes().take_while(u8::is_ascii_digit).count();
                    let n = if (1..=5).contains(&digits) { item.get(..digits).and_then(|d| d.parse::<u32>().ok()) } else { None };
                    match n {
                        Some(n @ 1..=0xffff) => push(FlagKey::from(n))?,
                        _ => return Err(format!("`{}` is not a flag number (1 to 65535)", item.chars().take(20).collect::<String>())),
                    }
                }
            }
        }
        Ok(out)
    }

    /// The one flag of a field (`PFX`/`SFX` flags, `FORBIDDENWORD`, `NOSUGGEST`).
    fn parse_one(self, field: &str) -> Result<FlagKey, String> {
        match self.parse(field)?.as_slice() {
            [k] => Ok(*k),
            _ => Err(format!("`{field}` is not one flag")),
        }
    }

    /// Append the flags `keys` to `out` as the source writes them.
    fn write(self, keys: impl Iterator<Item = FlagKey>, out: &mut String) {
        let ch = |k: FlagKey| u32::try_from(k).ok().and_then(char::from_u32);
        for (i, k) in keys.enumerate() {
            match self {
                FlagMode::Char => out.extend(ch(k)),
                FlagMode::Long => {
                    out.extend(ch(k >> 32));
                    out.extend(ch(k & 0xffff_ffff));
                }
                FlagMode::Num => {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&k.to_string());
                }
            }
        }
    }
}

/// Stem flags that rule a lookup out: `FORBIDDENWORD`, and `NOSUGGEST` for suggestions.
#[derive(Clone, Copy, Debug, Default)]
struct Excluded([Option<u16>; 2]);

impl Excluded {
    /// Does the flag set `set` carry an excluded flag?
    fn hits(self, set: &[u16]) -> bool {
        self.0.iter().flatten().any(|f| has(set, *f))
    }
}

/// Does the sorted flag set `set` hold `flag`?
fn has(set: &[u16], flag: u16) -> bool {
    set.binary_search(&flag).is_ok()
}

/// Do two sorted flag sets share a flag? (A binary search per flag of the smaller one.)
fn intersects(a: &[u16], b: &[u16]) -> bool {
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    small.iter().any(|f| has(large, *f))
}

/// Read Hunspell `.aff` text (already decoded to UTF-8).
fn parse_aff(aff: &str) -> Result<Rules, String> {
    let mut r = Rules::default();
    let mut cross: HashMap<(bool, FlagKey), bool> = HashMap::new();
    let mut map_header = false;
    let mut rep_header = false;
    for (n, line) in aff.lines().enumerate() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let Some(&dir) = f.first() else { continue };
        if dir.starts_with('#') {
            continue;
        }
        let at = |k: usize| f.get(k).copied().unwrap_or("");
        match dir {
            "SET" | "LANG" | "NAME" | "VERSION" | "HOME" | "WORDCHARS" | "MAXNGRAMSUGS" | "MAXDIFF" | "ONLYMAXDIFF" | "NOSPLITSUGS"
            | "SUGSWITHDOTS" | "WARN" | "FORBIDWARN" | "MAXCPDSUGS" => {}
            "FLAG" => r.set_mode(at(1)).map_err(|e| format!("line {}: {e}", n + 1))?,
            directive @ ("FORBIDDENWORD" | "NOSUGGEST") => {
                let flag = r.one_flag(at(1)).map_err(|e| format!("line {}: {directive} needs one flag: {e}", n + 1))?;
                if directive == "FORBIDDENWORD" {
                    r.forbidden = Some(flag);
                } else {
                    r.no_suggest = Some(flag);
                }
            }
            "TRY" => r.try_chars = cap_try(at(1)),
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
                let flag = r.one_flag(at(1)).map_err(|e| format!("line {}: {e}", n + 1))?;
                // Only a header ignores trailing fields; a rule condition beginning with # is literal.
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
                let cond = if f.len() >= 5 { at(4) } else { "." };
                parse_cond(cond).map_err(|e| format!("line {}: {e}", n + 1))?;
                r.check_flags(cont).map_err(|e| format!("line {}: {e}", n + 1))?;
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
                    if c != EMOJI_SELECTOR {
                        flags.push(c);
                    }
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
        AffixDict::build(rules, stems, comments.iter().map(ToString::to_string).collect(), MAX_FLAGS_PER_ENTRY)
    }

    /// Build from parsed rules and stems whose flag fields carry at most `entry_cap` flags each
    /// ([`MAX_FLAGS_PER_ENTRY`] for `.dic` entries, [`MAX_FLAGS_PER_STEM`] for compact data).
    fn build(rules: Rules, stems: Vec<(String, String)>, comments: Vec<String>, entry_cap: usize) -> Result<AffixDict, String> {
        let mode = rules.mode;
        // A `.dic` entry whose flags don't read (a bad number, more than MAX_FLAGS_PER_STEM flags)
        // is skipped, as a word Hunspell would read with the wrong flags: better unknown than
        // wrongly accepted. One with more than `entry_cap` flags is not a word either, but its
        // FORBIDDENWORD and NOSUGGEST flags still apply to its homonyms: an exclusion is never lost.
        let mut skipped = 0usize;
        let exclusion_keys = [rules.forbidden, rules.no_suggest];
        let is_exclusion_key = |k: &FlagKey| exclusion_keys.contains(&Some(*k));
        // Each stem's flags are a range of `stem_keys`: one buffer, not one allocation per stem.
        // The flag says whether the entry is a word (false: it only carries exclusions).
        let mut stem_keys: Vec<FlagKey> = Vec::new();
        let mut stems: Vec<(String, usize, usize, bool)> = stems
            .into_iter()
            .filter_map(|(w, f)| match mode.parse_at_most(&f, MAX_FLAGS_PER_STEM.max(entry_cap)) {
                Ok(keys) => {
                    let start = stem_keys.len();
                    let word = keys.len() <= entry_cap;
                    if word {
                        stem_keys.extend(keys);
                    } else {
                        skipped += 1;
                        stem_keys.extend(keys.into_iter().filter(is_exclusion_key));
                    }
                    Some((w, start, stem_keys.len(), word))
                }
                Err(_) => {
                    skipped += 1;
                    None
                }
            })
            .collect();
        if skipped > 0 {
            log::debug!("affix dictionary: {skipped} entries with unreadable or too many flags skipped");
        }
        let conts = |rs: &[RawRule]| rs.iter().map(|r| mode.parse(&r.4)).collect::<Result<Vec<_>, _>>();
        let (pfx_cont, sfx_cont) = (conts(&rules.prefixes)?, conts(&rules.suffixes)?);
        // Flags: every one a rule, a stem or a directive uses gets an id, in key order.
        let mut distinct: FxMap<FlagKey, u16> = FxMap::default();
        let all = rules.prefixes.iter().chain(&rules.suffixes).map(|r| r.0);
        let all = all.chain(pfx_cont.iter().chain(&sfx_cont).flatten().copied()).chain(stem_keys.iter().copied());
        for k in all.chain(rules.forbidden).chain(rules.no_suggest) {
            distinct.entry(k).or_insert(0);
            if distinct.len() > MAX_DISTINCT_FLAGS {
                return Err(format!("more than {MAX_DISTINCT_FLAGS} distinct flags"));
            }
        }
        let mut flag_keys: Vec<FlagKey> = distinct.keys().copied().collect();
        flag_keys.sort_unstable();
        // At most 2^16 keys, so every index fits a u16.
        for (i, k) in flag_keys.iter().enumerate() {
            if let (Some(id), Ok(i)) = (distinct.get_mut(k), u16::try_from(i)) {
                *id = i;
            }
        }
        let id_of = |k: FlagKey| distinct.get(&k).copied();
        // Stems sorted by bytes; homonyms (the same word twice) are merged.
        stems.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut d = AffixDict {
            words: String::with_capacity(stems.iter().map(|s| s.0.len()).sum()),
            offs: Vec::with_capacity(stems.len() + 1),
            set_offs: vec![0, 0],
            forbidden: rules.forbidden.and_then(id_of),
            no_suggest: rules.no_suggest.and_then(id_of),
            flag_mode: mode,
            try_chars: rules.try_chars,
            key: rules.key,
            map: rules.map,
            rep: rules.rep,
            comments,
            ..AffixDict::default()
        };
        d.offs.push(0);
        let mut set_ids: FxMap<Vec<u16>, u32> = FxMap::default();
        set_ids.insert(Vec::new(), 0);
        let exclusions: Vec<u16> = exclusion_keys.into_iter().flatten().filter_map(id_of).collect();
        let mut i = 0;
        let mut dropped = 0usize;
        let mut merged: Vec<u16> = Vec::new();
        let mut fresh: Vec<u16> = Vec::new();
        let mut carried: Vec<u16> = Vec::new();
        while i < stems.len() {
            let Some((word, ..)) = stems.get(i) else { break };
            merged.clear();
            carried.clear();
            let mut is_word = false;
            let mut j = i;
            while let Some((w, a, b, entry_is_word)) = stems.get(j)
                && w == word
            {
                j += 1;
                fresh.clear();
                fresh.extend(stem_keys.get(*a..*b).unwrap_or_default().iter().filter_map(|&k| id_of(k)));
                for &f in &fresh {
                    if exclusions.contains(&f) && !carried.contains(&f) {
                        carried.push(f);
                    }
                }
                if !entry_is_word {
                    continue;
                }
                is_word = true;
                fresh.sort_unstable();
                fresh.dedup();
                fresh.retain(|&f| !has(&merged, f));
                // Homonyms can add up to more flags than one entry holds; past MAX_FLAGS_PER_STEM
                // an entry's other flags are dropped (its exclusions are carried below), so the
                // compact data never holds a set its reader refuses.
                if merged.len().saturating_add(fresh.len()) <= MAX_FLAGS_PER_STEM {
                    merged.extend_from_slice(&fresh);
                    // A sorted run and a sorted tail: the stable sort merges them in linear time.
                    merged.sort();
                } else {
                    dropped += 1;
                }
            }
            if !is_word {
                i = j;
                continue;
            }
            for &f in &carried {
                if has(&merged, f) {
                    continue;
                }
                // Make room by dropping a flag that excludes nothing (there are at most two
                // exclusion flags, so a full set always has one).
                if merged.len() >= MAX_FLAGS_PER_STEM
                    && let Some(at) = merged.iter().rposition(|g| !exclusions.contains(g))
                {
                    merged.remove(at);
                }
                if let Err(at) = merged.binary_search(&f) {
                    merged.insert(at, f);
                }
            }
            let id = d.intern(&mut set_ids, &mut merged)?;
            d.words.push_str(word);
            d.offs.push(u32::try_from(d.words.len()).map_err(|_| "stems too large".to_string())?);
            d.flags.push(id);
            i = j;
        }
        if dropped > 0 {
            log::debug!("affix dictionary: {dropped} homonym entries past {MAX_FLAGS_PER_STEM} merged flags kept only their exclusions");
        }
        let mut build_rules = |raw: Vec<RawRule>, conts: Vec<Vec<FlagKey>>, d: &mut AffixDict| -> Result<Vec<Rule>, String> {
            raw.into_iter()
                .zip(conts)
                .map(|((flag, cross, strip, add, _, cond), cont)| {
                    let flag = id_of(flag).ok_or("affix flag without an id")?;
                    let cont = d.intern(&mut set_ids, &mut cont.iter().filter_map(|&k| id_of(k)).collect())?;
                    Ok(Rule { flag, cross, strip: strip.into(), add: add.into(), cont, cond: parse_cond(&cond)?, cond_text: cond.into() })
                })
                .collect()
        };
        d.prefixes = build_rules(rules.prefixes, pfx_cont, &mut d)?;
        d.suffixes = build_rules(rules.suffixes, sfx_cont, &mut d)?;
        d.flag_keys = flag_keys;
        d.index();
        Ok(d)
    }

    /// The id of the flag set `flags` (sorted and deduplicated here), adding it if it is new.
    fn intern(&mut self, ids: &mut FxMap<Vec<u16>, u32>, flags: &mut Vec<u16>) -> Result<u32, String> {
        flags.sort_unstable();
        flags.dedup();
        if let Some(&id) = ids.get(flags.as_slice()) {
            return Ok(id);
        }
        let id = u32::try_from(self.set_offs.len().saturating_sub(1)).map_err(|_| "too many distinct flag sets".to_string())?;
        self.set_data.extend_from_slice(flags);
        self.set_offs.push(u32::try_from(self.set_data.len()).map_err(|_| "flag sets too large".to_string())?);
        ids.insert(flags.clone(), id);
        Ok(id)
    }

    /// The flag set `id`, sorted (empty when unknown).
    fn set(&self, id: u32) -> &[u16] {
        let i = id as usize;
        match (self.set_offs.get(i), self.set_offs.get(i + 1)) {
            (Some(&a), Some(&b)) => self.set_data.get(a as usize..b as usize).unwrap_or_default(),
            _ => &[],
        }
    }

    /// Build the suffix index and the stem hash table.
    fn index(&mut self) {
        let mut idx: FxMap<Box<str>, Vec<SfxGroup>> = FxMap::default();
        // Each (appended, stripped) pair's group, found by hash rather than by scanning the
        // appended string's groups: a dictionary with many strips per suffix stays linear.
        let mut group_of: FxMap<(&str, &str), usize> = FxMap::default();
        let mut by_flag: Vec<Vec<u32>> = vec![Vec::new(); self.flag_keys.len()];
        for (i, r) in self.suffixes.iter().enumerate() {
            let groups = idx.entry(r.add.clone()).or_default();
            let Ok(i) = u32::try_from(i) else { break };
            let g = match group_of.get(&(&*r.add, &*r.strip)).copied() {
                Some(g) => g,
                None => {
                    group_of.insert((&r.add, &r.strip), groups.len());
                    groups.push(SfxGroup { strip: r.strip.clone(), ..SfxGroup::default() });
                    groups.len() - 1
                }
            };
            if let Some(g) = groups.get_mut(g) {
                g.rules.push(i);
                if r.cont != 0 {
                    g.cont_rules.push(i);
                }
            }
            if let Some(list) = by_flag.get_mut(usize::from(r.flag)) {
                list.push(i);
            }
        }
        self.sfx_by_flag = by_flag;
        let mut cont_flag = vec![false; self.flag_keys.len()];
        for r in self.prefixes.iter().chain(&self.suffixes) {
            for &f in self.set(r.cont) {
                if let Some(c) = cont_flag.get_mut(usize::from(f)) {
                    *c = true;
                }
            }
        }
        self.has_cont = cont_flag.contains(&true);
        self.cont_flag = cont_flag;
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

    /// Append the flags of the set `ids` (or one flag) to `out` as the source wrote them.
    fn write_flags(&self, ids: &[u16], out: &mut String) {
        self.flag_mode.write(ids.iter().filter_map(|&i| self.flag_keys.get(usize::from(i)).copied()), out);
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

    /// What lookups exclude: forbidden words, and NOSUGGEST stems too for suggestions.
    fn excluded(&self, suggestible: bool) -> Excluded {
        Excluded([self.forbidden, if suggestible { self.no_suggest } else { None }])
    }

    /// Flag set of a stem, if `w` is one.
    fn stem_flags(&self, w: &str) -> Option<&[u16]> {
        self.stem_flags_excluding(w, self.excluded(false))
    }

    fn stem_flags_excluding(&self, w: &str, excluded: Excluded) -> Option<&[u16]> {
        if self.table.is_empty() {
            return None;
        }
        let mask = self.table.len() - 1;
        let mut slot = hash_str(w) as usize & mask;
        for _ in 0..self.table.len() {
            let t = *self.table.get(slot)?;
            if t == EMPTY {
                return None;
            }
            if self.word(t as usize) == Some(w) {
                let set = self.set(*self.flags.get(t as usize)?);
                // All derivations use this lookup: exclusions apply to stems and their forms.
                return (!excluded.hits(set)).then_some(set);
            }
            slot = (slot + 1) & mask;
        }
        None
    }

    /// Can a derived form carry `flag` (does some rule's continuation classes hold it)?
    fn is_cont_flag(&self, flag: u16) -> bool {
        self.cont_flag.get(usize::from(flag)).copied().unwrap_or(false)
    }

    /// Does `w` carry any flag of `flags` — as a stem, or as the form of a suffix rule whose
    /// continuation classes include one (chained suffixation, depth-bounded)?
    ///
    /// The rules of a group that could lead to `w` are merged into one set of their flags (sorted)
    /// and followed once, not once per rule: the answer is the same, but thousands of rules of one
    /// flag cost one walk, not one each (#3613).
    fn form_has_flag(&self, w: &str, flags: &[u16], depth: u8, excluded: Excluded) -> bool {
        if self.stem_flags_excluding(w, excluded).is_some_and(|f| intersects(f, flags)) {
            return true;
        }
        // Without continuation classes no rule leads from a form to another.
        if depth == 0 || !self.has_cont {
            return false;
        }
        let mut root = String::with_capacity(w.len() + 8);
        let mut inner: Vec<u16> = Vec::new();
        let cuts = std::iter::once(w.len()).chain(w.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (w.get(..b), w.get(b..)) else { continue };
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups.iter().filter(|g| !g.cont_rules.is_empty()) {
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                inner.clear();
                inner.extend(
                    g.cont_rules
                        .iter()
                        .filter_map(|&i| self.suffixes.get(i as usize))
                        .filter(|r| intersects(self.set(r.cont), flags) && r.cond_matches(&root, true))
                        .map(|r| r.flag),
                );
                inner.sort_unstable();
                inner.dedup();
                if !inner.is_empty() && self.form_has_flag(&root, &inner, depth - 1, excluded) {
                    return true;
                }
            }
        }
        false
    }

    /// Is `w` exactly (case and all) a stem or an affixed form of one?
    pub fn contains(&self, w: &str) -> bool {
        self.contains_excluding(w, self.excluded(false))
    }

    /// Exactly spelled dictionary words eligible for suggestions (NOSUGGEST stems excluded).
    pub fn contains_suggestible(&self, w: &str) -> bool {
        self.contains_excluding(w, self.excluded(true))
    }

    fn contains_excluding(&self, w: &str, excluded: Excluded) -> bool {
        if w.is_empty() || w.len() > MAX_WORD_BYTES {
            return false;
        }
        if self.stem_flags_excluding(w, excluded).is_some() || self.suffixed(w, None, excluded) {
            return true;
        }
        let mut root = String::with_capacity(w.len() + 8);
        for r in &self.prefixes {
            let Some(rest) = w.strip_prefix(&*r.add) else { continue };
            if rest.is_empty() {
                continue;
            }
            root.clear();
            root.push_str(&r.strip);
            root.push_str(rest);
            if !r.cond_matches(&root, false) {
                continue;
            }
            if self.stem_flags_excluding(&root, excluded).is_some_and(|f| has(f, r.flag)) {
                return true;
            }
            if self.is_cont_flag(r.flag) && self.form_has_flag(&root, &[r.flag], INNER_CHAIN, excluded) {
                return true;
            }
            if r.cross && self.suffixed(&root, Some(r.flag), excluded) {
                return true;
            }
        }
        false
    }

    /// Is `w` a stem with one suffix? With `need` set (a prefix was taken off), the stem must
    /// carry that flag too and the suffix must allow cross products.
    fn suffixed(&self, w: &str, need: Option<u16>, excluded: Excluded) -> bool {
        let mut root = String::with_capacity(w.len() + 8);
        let mut flags: Vec<u16> = Vec::new();
        // Each split point from the end: the appended string is `w[b..]`, at most the longest
        // suffix, and something must stay before it.
        let cuts = std::iter::once(w.len()).chain(w.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (w.get(..b), w.get(b..)) else { continue };
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups {
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                let rules = || g.rules.iter().filter_map(|&i| self.suffixes.get(i as usize));
                // The stem path: `root` is a stem carrying the rule's flag (and the prefix's,
                // when a cross product is being checked).
                if let Some(f) = self.stem_flags_excluding(&root, excluded)
                    && need.is_none_or(|n| has(f, n))
                    && rules().any(|r| has(f, r.flag) && (need.is_none() || r.cross) && r.cond_matches(&root, true))
                {
                    return true;
                }
                // Continuation classes: `root` need not be a stem carrying this rule's flag —
                // it may be a form that gained the flag from another rule's continuation. The
                // matching rules' flags are looked for together, in one walk. Without continuation
                // classes that walk would only repeat the stem path above.
                if need.is_none() && self.has_cont {
                    flags.clear();
                    // Only a flag some continuation class holds can come from another rule.
                    flags.extend(rules().filter(|r| self.is_cont_flag(r.flag) && r.cond_matches(&root, true)).map(|r| r.flag));
                    flags.sort_unstable();
                    flags.dedup();
                    if !flags.is_empty() && self.form_has_flag(&root, &flags, INNER_CHAIN, excluded) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Is `w` correct, Hunspell style: as written, or — for a capitalized word or one in capitals
    /// — in lower case (and capitalized), since a sentence may start with any word and headings
    /// may be in capitals. Lower-case input never matches a capitalized stem (`warszawa`).
    pub fn check(&self, w: &str) -> bool {
        self.check_excluding(w, self.excluded(false))
    }

    /// Whether a suggestion is correct in its displayed case and has an eligible stem.
    pub fn can_suggest(&self, w: &str) -> bool {
        self.check_excluding(w, self.excluded(true))
    }

    fn check_excluding(&self, w: &str, excluded: Excluded) -> bool {
        if self.contains_excluding(w, excluded) {
            return true;
        }
        match casing(w) {
            Casing::Lower | Casing::Mixed => false,
            Casing::Initial => self.contains_excluding(&w.to_lowercase(), excluded),
            Casing::Upper => {
                let lower = w.to_lowercase();
                self.contains_excluding(&capitalize(&lower), excluded) || self.contains_excluding(&lower, excluded)
            }
        }
    }

    /// Every form of the stem `stem` (itself included), unsorted, possibly with repeats; at most
    /// [`MAX_FORMS`], and at most [`MAX_FORMS_WORK`] rules tried along the continuation chains and
    /// cross products, so a dictionary with thousands of self-continuing rules can't make one stem
    /// cost millions of strings (#3613).
    pub fn forms(&self, stem: &str) -> Vec<String> {
        let Some(f) = self.stem_flags(stem) else { return Vec::new() };
        let mut out = vec![stem.to_string()];
        let mut cross_sfx = Vec::new();
        // Suffix forms queued with the continuation classes they carry, for chained expansion;
        // each (form, classes) pair is expanded once.
        let mut chain: Vec<(String, u32)> = Vec::new();
        let mut queued: std::collections::HashSet<(String, u32)> = std::collections::HashSet::new();
        let mut work = 0usize;
        for r in &self.suffixes {
            if !has(f, r.flag) || !r.cond_matches(stem, true) {
                continue;
            }
            let Some(base) = stem.strip_suffix(&*r.strip) else { continue };
            let form = format!("{base}{}", r.add);
            if r.cross {
                cross_sfx.push(form.clone());
            }
            if r.cont != 0 && queued.insert((form.clone(), r.cont)) {
                chain.push((form.clone(), r.cont));
            }
            out.push(form);
            if out.len() >= MAX_FORMS {
                return out;
            }
        }
        // Continuation classes chain suffixes (`workable/B` → `workables`), up to [`MAX_CHAIN`]
        // suffixes in all — the same bound the checker walks.
        'chain: for _ in 0..INNER_CHAIN {
            let level = std::mem::take(&mut chain);
            if level.is_empty() {
                break;
            }
            for (form, flags) in level {
                for r in self.suffixes_with(flags).into_iter().filter_map(|i| self.suffixes.get(i as usize)) {
                    work += 1;
                    if work > MAX_FORMS_WORK {
                        break 'chain;
                    }
                    if !r.cond_matches(&form, true) {
                        continue;
                    }
                    let Some(base) = form.strip_suffix(&*r.strip) else { continue };
                    let next = format!("{base}{}", r.add);
                    if r.cont != 0 && queued.insert((next.clone(), r.cont)) {
                        chain.push((next.clone(), r.cont));
                    }
                    out.push(next);
                    if out.len() >= MAX_FORMS {
                        return out;
                    }
                }
            }
        }
        for r in &self.prefixes {
            if !has(f, r.flag) || !r.cond_matches(stem, false) {
                continue;
            }
            let Some(rest) = stem.strip_prefix(&*r.strip) else { continue };
            out.push(format!("{}{rest}", r.add));
            if r.cross {
                for s in &cross_sfx {
                    work += 1;
                    if out.len() >= MAX_FORMS || work > MAX_FORMS_WORK {
                        return out;
                    }
                    if let Some(rest) = s.strip_prefix(&*r.strip) {
                        out.push(format!("{}{rest}", r.add));
                    }
                }
            }
            if out.len() >= MAX_FORMS {
                return out;
            }
        }
        out
    }

    /// The suffix rules carrying any flag of the set `set`, in rule order.
    fn suffixes_with(&self, set: u32) -> Vec<u32> {
        let mut rules: Vec<u32> = Vec::new();
        let mut lists = 0;
        for &flag in self.set(set) {
            if let Some(list) = self.sfx_by_flag.get(usize::from(flag))
                && !list.is_empty()
            {
                rules.extend_from_slice(list);
                lists += 1;
            }
        }
        if lists > 1 {
            rules.sort_unstable();
        }
        rules
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

    /// Serialize to WordCraft's compact format: raw-deflate of `WCAFF1`, a `FLAG` line when flags
    /// aren't one character each, the comments, the tables and rules as text lines, a `WORDS`
    /// line, then one line per stem, front-coded against the previous one (a byte with the shared
    /// prefix length in bytes, the rest of the word, `/` and its flags if any). Flags are written
    /// as the `.aff` wrote them, so one-character data is the same as before `FLAG` lines existed.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        out.push_str(MAGIC);
        out.push('\n');
        if let Some(name) = self.flag_mode.name() {
            out.push_str(&format!("FLAG {name}\n"));
        }
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
        // In one-character mode a directive flag that no stem or rule uses isn't written, as
        // before flag ids existed, so those dictionaries' compact data stays byte for byte the same.
        let used = |flag: u16| {
            self.flags.iter().any(|&id| self.set(id).contains(&flag))
                || self.prefixes.iter().chain(&self.suffixes).any(|r| r.flag == flag || self.set(r.cont).contains(&flag))
        };
        for (directive, flag) in [("FORBIDDENWORD", self.forbidden), ("NOSUGGEST", self.no_suggest)] {
            if let Some(flag) = flag.filter(|&f| self.flag_mode != FlagMode::Char || used(f)) {
                out.push_str(directive);
                out.push(' ');
                self.write_flags(&[flag], &mut out);
                out.push('\n');
            }
        }
        let z = |s: &str| if s.is_empty() { "0".to_string() } else { s.to_string() };
        for (kind, rules) in [("PFX", &self.prefixes), ("SFX", &self.suffixes)] {
            for r in rules {
                let y = if r.cross { "Y" } else { "N" };
                // Continuation classes ride on the appended field, as in the `.aff` source.
                let mut add = z(&r.add);
                let cont = self.set(r.cont);
                if !cont.is_empty() {
                    add.push('/');
                    self.write_flags(cont, &mut add);
                }
                let mut flag = String::new();
                self.write_flags(&[r.flag], &mut flag);
                out.push_str(&format!("{kind} {flag} {y} {} {} {}\n", z(&r.strip), add, r.cond_text));
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
            let set = self.flags.get(i).map_or(&[][..], |&id| self.set(id));
            if !set.is_empty() {
                let mut flags = String::from("/");
                self.write_flags(set, &mut flags);
                raw.extend_from_slice(flags.as_bytes());
            }
            raw.push(b'\n');
            prev = b;
        }
        miniz_oxide::deflate::compress_to_vec(&raw, 10)
    }

    /// Read data written by [`AffixDict::to_bytes`].
    pub fn from_bytes(data: &[u8]) -> Result<AffixDict, String> {
        let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(data, 64 << 20)
            .map_err(|e| format!("affix dictionary: inflate failed: {:?}", e.status))?;
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
            match at(0) {
                "FLAG" => rules.set_mode(at(1)).map_err(|e| format!("affix dictionary: {e}"))?,
                "TRY" => rules.try_chars = cap_try(at(1)),
                "KEY" => rules.key = at(1).to_string(),
                "MAP" => rules.map.push(at(1).chars().collect()),
                "REP" => rules.rep.push((at(1).replace('_', " "), at(2).replace('_', " "))),
                directive @ ("FORBIDDENWORD" | "NOSUGGEST") => {
                    let flag = rules.one_flag(at(1)).map_err(|e| format!("affix dictionary: bad {directive} in `{line}`: {e}"))?;
                    if directive == "FORBIDDENWORD" {
                        rules.forbidden = Some(flag);
                    } else {
                        rules.no_suggest = Some(flag);
                    }
                }
                kind @ ("PFX" | "SFX") => {
                    let flag = rules.one_flag(at(1)).map_err(|e| format!("affix dictionary: bad flag in `{line}`: {e}"))?;
                    if f.len() != 6 {
                        return Err(format!("affix dictionary: bad rule `{line}`"));
                    }
                    // Continuation classes ride on the appended field, as in the `.aff` source.
                    let (add, cont) = match at(4).split_once('/') {
                        Some((a, c)) => (zero_empty(a), c.to_string()),
                        None => (zero_empty(at(4)), String::new()),
                    };
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
        AffixDict::build(rules, stems, comments, MAX_FLAGS_PER_STEM)
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

    #[test]
    fn try_alphabet_is_capped_and_deduplicated() {
        let aff = format!("TRY {}\n", "abcdefghijklmnopqrstuvwxyz".repeat(10_000));
        let d = AffixDict::from_hunspell(&aff, "1\nkot\n", &[]).unwrap();
        assert_eq!(d.try_chars(), "abcdefghijklmnopqrstuvwxyz");
        let compact = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert_eq!(compact.try_chars(), "abcdefghijklmnopqrstuvwxyz");

        // More distinct characters than the cap, each repeated: keep the first 128 in order.
        let many: String = (0x100u32..0x100 + 300).filter_map(char::from_u32).collect();
        let aff = format!("TRY {many}{many}\n");
        let want: String = many.chars().take(128).collect();
        let d = AffixDict::from_hunspell(&aff, "1\nkot\n", &[]).unwrap();
        assert_eq!(d.try_chars(), want);
        let compact = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert_eq!(compact.try_chars(), want);

        // An oversized TRY line fed straight to the compact reader is capped as well.
        let raw = format!("{MAGIC}\nTRY {many}{many}\nWORDS\n\0kot\n");
        let bytes = miniz_oxide::deflate::compress_to_vec(raw.as_bytes(), 6);
        assert_eq!(AffixDict::from_bytes(&bytes).unwrap().try_chars(), want);
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
        // mode the reader assumes without a FLAG line.
        for mode in ["UTF-8", "DEFAULT"] {
            let aff = format!("SET UTF-8\nFLAG {mode}\nSFX a Y 1\nSFX a   0   y   [^y]\n");
            let d = AffixDict::from_hunspell(&aff, "1\nkot/a\n", &[]).unwrap();
            assert!(d.contains("kot") && d.contains("koty"), "{mode}");
        }
        assert!(AffixDict::from_hunspell("FLAG UTF-8\n", "0\n", &[]).is_ok(), "FLAG alone is harmless");
    }

    #[test]
    fn emoji_flags_with_a_variation_selector_load() {
        // RLA-ES (the Spanish dictionaries in LibreOffice and most Linux distributions) writes its
        // telephone flag as ☎ followed by U+FE0F, in the rule and on the stems.
        let aff = "SET UTF-8\nFLAG UTF-8\nPFX \u{260e}\u{fe0f} Y 1\nPFX \u{260e}\u{fe0f} 0 tele .\nSFX S Y 1\nSFX S 0 s [aeiou]\n";
        let d = AffixDict::from_hunspell(aff, "2\nbanco/S\u{260e}\u{fe0f}\nbasura/S\n", &[]).unwrap();
        assert!(d.contains("banco") && d.contains("bancos"));
        assert!(d.contains("telebanco") && d.contains("telebancos"), "the ☎ prefix applies");
        assert!(d.contains("basuras") && !d.contains("telebasura"), "only to stems that carry it");
        assert!(!d.flag_keys.contains(&FlagKey::from(EMOJI_SELECTOR)), "the selector is not a flag");
    }

    #[test]
    fn emoji_forbiddenword_and_nosuggest_flags_load() {
        // The same selector after the flag of the directives that name one flag.
        let aff = "SET UTF-8\nFLAG UTF-8\nFORBIDDENWORD \u{260e}\u{fe0f}\nNOSUGGEST \u{1f6ab}\u{fe0f}\n";
        let d = AffixDict::from_hunspell(aff, "3\nbanco\nbanko/\u{260e}\u{fe0f}\nbancu/\u{1f6ab}\u{fe0f}\n", &[]).unwrap();
        assert!(d.contains("banco") && d.contains("bancu"));
        assert!(!d.contains("banko"), "a stem with the FORBIDDENWORD flag is not a word");
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
    fn continuation_classes_merge_the_flags_of_a_group() {
        // `s` under B and under C share one group (same append, same strip): a form carrying
        // either flag takes it, and the stem carrying neither doesn't.
        let aff = "SET UTF-8\nSFX A N 1\nSFX A 0 able/C .\nSFX D N 1\nSFX D 0 er/B .\nSFX B N 1\nSFX B 0 s .\nSFX C N 1\nSFX C 0 s [e]\n";
        let d = AffixDict::from_hunspell(aff, "2\nwork/A\nplay/D\n", &[]).unwrap();
        assert!(d.contains("workables") && d.contains("players"));
        assert!(!d.contains("works") && !d.contains("plays") && !d.contains("workabless"));
        assert!(d.forms("work").iter().any(|f| f == "workables") && d.forms("play").iter().any(|f| f == "players"));
    }

    /// `n` suffix rules of flag A on one stem, `rule(i)` giving each rule's strip, append and condition.
    fn many_rules(n: usize, rule: impl Fn(usize) -> String) -> AffixDict {
        let mut aff = format!("SET UTF-8\nSFX A Y {n}\n");
        for i in 0..n {
            aff.push_str(&format!("SFX A {}\n", rule(i)));
        }
        AffixDict::from_hunspell(&aff, "1\nkot/A\n", &[]).unwrap()
    }

    #[test]
    fn self_continuing_rules_check_in_linear_time() {
        // Thousands of empty-append rules that continue with their own flag: each check used to
        // follow every rule from every rule, millions of steps a word (#3613). The checks take
        // about a millisecond; the old checker took 14 s for two of them at 16,000 rules, so at
        // 40,000 (quadratic: minutes) the 10 s bound leaves a busy machine a wide margin.
        let d = many_rules(40_000, |_| "0 0/A .".into());
        let t = std::time::Instant::now();
        assert!(d.check("kot") && !d.check("pies") && !d.check("kotx"));
        assert!(t.elapsed().as_secs_f64() < 10.0, "{:?}", t.elapsed());
    }

    #[test]
    fn forms_of_self_continuing_rules_are_bounded() {
        // Every rule's form continues with every rule: millions of strings for one stem (#3613).
        let same = many_rules(3000, |_| "0 0/A .".into());
        let distinct = many_rules(3000, |i| format!("0 x{i}/A ."));
        let t = std::time::Instant::now();
        let (a, b) = (same.forms("kot"), distinct.forms("kot"));
        // The cap on the count is what tells the old code (9 million forms each) apart; the
        // generous time bound (tens of ms here) only backs up the rule-visit cap.
        assert!(t.elapsed().as_secs_f64() < 10.0, "{:?}", t.elapsed());
        assert!(a.len() <= MAX_FORMS && b.len() <= MAX_FORMS, "{} {}", a.len(), b.len());
        assert!(b.iter().any(|f| f == "kotx0x1"), "chained forms are still listed");
    }

    #[test]
    fn many_strips_per_suffix_index_in_linear_time() {
        // One append with thousands of strips: the index used to scan the append's groups for
        // each rule. Building 200,000 rules takes well under a second; the old index took 3 s at
        // 30,000 and over 10 minutes at 300,000.
        let n = 200_000;
        let t = std::time::Instant::now();
        let d = many_rules(n, |i| format!("k{i} y ."));
        assert!(t.elapsed().as_secs_f64() < 10.0, "{:?}", t.elapsed());
        assert_eq!(d.sfx_index.get("y").map(Vec::len), Some(n));
        assert!(!d.contains("koty"));
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
        // `FLAG long` and `FLAG num` load now (see the tests below); an unknown mode doesn't.
        assert!(AffixDict::from_hunspell("FLAG long\n", DIC, &[]).is_ok());
        assert!(AffixDict::from_hunspell("FLAG num\n", DIC, &[]).is_ok());
        assert!(AffixDict::from_hunspell("FLAG bogus\n", DIC, &[]).is_err());
        assert!(AffixDict::from_hunspell("COMPOUNDFLAG X\n", DIC, &[]).is_err());
        assert!(AffixDict::from_hunspell("SFX a Y 1\nSFX a 0 y [ab\n", DIC, &[]).is_err());
        assert!(AffixDict::from_bytes(b"not deflate").is_err());
        let ok = dict().to_bytes();
        assert!(AffixDict::from_bytes(&ok[..ok.len() / 2]).is_err());
        let garbage = miniz_oxide::deflate::compress_to_vec(b"WCAFF1\nWORDS\n\x09abc\n", 6);
        assert!(AffixDict::from_bytes(&garbage).is_err(), "prefix longer than the previous word");
        let empty = AffixDict::default();
        assert!(!empty.contains("kot") && !empty.check("Kot") && empty.forms("kot").is_empty());
    }

    /// A small `FLAG num` dictionary (made-up words): prefixes, suffixes, a continuation class,
    /// a cross product, the largest flag number, and numeric FORBIDDENWORD and NOSUGGEST flags.
    const NUM_AFF: &str = "SET UTF-8\nFLAG num\nFORBIDDENWORD 900\nNOSUGGEST 901\nPFX 7 Y 1\nPFX 7 0 ne .\nSFX 12 Y 2\nSFX 12 0 a .\nSFX 12 0 om/1503 .\nSFX 1503 N 1\nSFX 1503 0 e .\nSFX 65535 N 1\nSFX 65535 0 i .\n";
    const NUM_DIC: &str = "5\ngrad/12,7\nmost/65535\nkuca/12,901\ngradd/900\nlist/1503\n";

    fn assert_num_dict(d: &AffixDict) {
        for w in ["grad", "grada", "gradom", "gradome", "negrad", "negrada", "negradom", "most", "mosti", "list", "liste", "Grada", "MOSTI"] {
            assert!(d.check(w), "{w}");
        }
        // 1503 reaches `grad` only through `om`; 12 and 65535 belong to other stems; 7 is a prefix.
        for w in ["grade", "gradi", "mosta", "nemost", "lista", "listom", "gradomi", "7grad", "grad12"] {
            assert!(!d.check(w), "{w}");
        }
        assert!(!d.check("gradd") && !d.check("Gradd") && d.forms("gradd").is_empty(), "numeric FORBIDDENWORD");
        assert!(d.check("kuca") && d.check("kucaa") && !d.can_suggest("kuca") && !d.can_suggest("kucaom"), "numeric NOSUGGEST");
        assert!(d.can_suggest("gradome"));
        let mut forms = d.forms("grad");
        forms.sort();
        forms.dedup();
        assert_eq!(forms, ["grad", "grada", "gradom", "gradome", "negrad", "negrada", "negradom"]);
    }

    #[test]
    fn flag_num_dictionaries_load_and_check() {
        let d = AffixDict::from_hunspell(NUM_AFF, NUM_DIC, &[]).unwrap();
        assert_eq!((d.len(), d.rule_count()), (5, 5));
        assert_num_dict(&d);
    }

    #[test]
    fn flag_num_dictionaries_round_trip_the_compact_format() {
        let d = AffixDict::from_hunspell(NUM_AFF, NUM_DIC, &["num test"]).unwrap();
        let bytes = d.to_bytes();
        let raw = miniz_oxide::inflate::decompress_to_vec(&bytes).unwrap();
        assert!(raw.starts_with(b"WCAFF1\nFLAG num\n"), "the mode is written before any flag");
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("FORBIDDENWORD 900\n") && text.contains("SFX 12 Y 0 om/1503 .\n") && text.contains("grad/7,12\n"));
        let back = AffixDict::from_bytes(&bytes).unwrap();
        assert_num_dict(&back);
        assert_eq!(back.comments(), ["num test"]);
        assert_eq!(back.to_bytes(), bytes, "a second round trip writes the same data");
    }

    /// A small `FLAG long` dictionary: two-character flags that share characters with each other.
    const LONG_AFF: &str = "SET UTF-8\nFLAG long\nFORBIDDENWORD !!\nSFX Aa Y 1\nSFX Aa 0 s/Bb .\nSFX Bb N 1\nSFX Bb 0 ly .\nPFX Cc Y 1\nPFX Cc 0 un .\nSFX aA N 1\nSFX aA 0 er .\n";
    const LONG_DIC: &str = "5\nkind/AaCc\nhope/Bb\nbadd/!!\nodd/Aab\nwarm/aA\n";

    fn assert_long_dict(d: &AffixDict) {
        for w in ["kind", "kinds", "kindsly", "unkind", "unkinds", "hope", "hopely", "warm", "warmer"] {
            assert!(d.check(w), "{w}");
        }
        for w in ["kindly", "kinder", "hopes", "unhope", "warms", "unwarm", "badd", "odd", "odds"] {
            assert!(!d.check(w), "{w}");
        }
    }

    #[test]
    fn flag_long_dictionaries_load_check_and_round_trip() {
        let d = AffixDict::from_hunspell(LONG_AFF, LONG_DIC, &[]).unwrap();
        // `odd/Aab` has an odd number of flag characters: the entry is skipped.
        assert_eq!(d.len(), 4);
        assert_long_dict(&d);
        let bytes = d.to_bytes();
        assert!(miniz_oxide::inflate::decompress_to_vec(&bytes).unwrap().starts_with(b"WCAFF1\nFLAG long\n"));
        let back = AffixDict::from_bytes(&bytes).unwrap();
        assert_long_dict(&back);
        assert_eq!(back.to_bytes(), bytes);
        assert!(AffixDict::from_hunspell("FLAG long\nSFX A Y 1\nSFX A 0 s .\n", "0\n", &[]).is_err(), "a one-character long flag");
    }

    #[test]
    fn malformed_flag_numbers_are_refused_in_the_aff_and_skipped_in_the_dic() {
        for rule in ["x", "0", "65536", "99999999999999999999999", "+5", "-1", "1,2", " "] {
            let aff = format!("FLAG num\nSFX {rule} Y 1\nSFX {rule} 0 a .\n");
            assert!(AffixDict::from_hunspell(&aff, "1\nw/1\n", &[]).is_err(), "rule flag `{rule}`");
        }
        for cont in ["2,,3", "2,", ",2", "a", "0", "70000", "x2"] {
            let aff = format!("FLAG num\nSFX 1 Y 1\nSFX 1 0 a/{cont} .\n");
            assert!(AffixDict::from_hunspell(&aff, "1\nw/1\n", &[]).is_err(), "continuation `{cont}`");
        }
        for directive in ["FORBIDDENWORD 1,2", "FORBIDDENWORD abc", "NOSUGGEST 0", "NOSUGGEST", "FORBIDDENWORD 65536"] {
            assert!(AffixDict::from_hunspell(&format!("FLAG num\n{directive}\n"), "0\n", &[]).is_err(), "{directive}");
        }
        assert!(AffixDict::from_hunspell("SFX a Y 1\nSFX a 0 y .\nFLAG num\n", "0\n", &[]).is_err(), "FLAG after flags");
        assert!(AffixDict::from_hunspell("FLAG num\nFLAG num\nSFX 1 Y 1\nSFX 1 0 a .\n", "0\n", &[]).is_ok());
        // The .dic: entries whose flags don't read are left out; the rest of the file loads.
        let dic = "8\nok/1\nzero/0\nbig/70000\nhuge/123456789012345678901234567890\nsign/-1\ncomma/1,\nletters/a1\nplain\n";
        let d = AffixDict::from_hunspell("FLAG num\nSFX 1 Y 1\nSFX 1 0 a .\n", dic, &[]).unwrap();
        assert!(d.check("ok") && d.check("oka") && d.check("plain"));
        for w in ["zero", "big", "huge", "sign", "comma", "commaa", "letters"] {
            assert!(!d.check(w), "{w}");
        }
        assert_eq!(d.len(), 2);
        // Text after a number's digits is ignored, as Hunspell does (Nepali's `0/17X`).
        let d = AffixDict::from_hunspell("FLAG num\nSFX 1 Y 1\nSFX 1 0 a/2X .\nSFX 2 N 1\nSFX 2 0 i .\n", "1\nw/1x,7\n", &[]).unwrap();
        assert!(d.check("wa") && d.check("wai") && !d.check("wi"));
    }

    #[test]
    fn thousands_of_flags_load() {
        // Serbian uses 1,903 numeric flags, Belarusian and Sinhala 5,000, each stem one or two:
        // 3,000 flags here, all appending `a` (one suffix group of 3,000 rules), plus flags up to
        // the largest number.
        let mut aff = String::from("SET UTF-8\nFLAG num\n");
        let mut dic = String::from("3002\n");
        for n in 1..=3000 {
            aff.push_str(&format!("SFX {n} N 1\nSFX {n} 0 a [^q]\n"));
            dic.push_str(&format!("w{n}/{n}\n"));
        }
        aff.push_str("SFX 65000 N 1\nSFX 65000 0 i .\n");
        dic.push_str("dva/3000,65000\nq/1\n");
        let d = AffixDict::from_hunspell(&aff, &dic, &[]).unwrap();
        assert_eq!(d.rule_count(), 3001);
        for w in ["w1a", "w1500a", "w3000a", "dva", "dvaa", "dvai", "q"] {
            assert!(d.check(w), "{w}");
        }
        assert!(!d.check("qa") && !d.check("w1i") && d.check("w1"));
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.check("w2999a") && back.check("dvai") && !back.check("w7i"));

        // More than 128 one-character flags (#4966): 300 letters, each its own suffix.
        let letters: Vec<char> = (0x100u32..0x100 + 300).filter_map(char::from_u32).collect();
        let mut aff = String::from("SET UTF-8\nFLAG UTF-8\n");
        for c in &letters {
            aff.push_str(&format!("SFX {c} N 1\nSFX {c} 0 {c} .\n"));
        }
        let all: String = letters.iter().take(256).collect();
        let d = AffixDict::from_hunspell(&aff, &format!("2\nbase/{all}\nlast/{}\n", letters[299]), &[]).unwrap();
        assert!(d.check(&format!("base{}", letters[0])) && d.check(&format!("base{}", letters[255])));
        assert!(!d.check(&format!("base{}", letters[299])) && d.check(&format!("last{}", letters[299])));
    }

    #[test]
    fn flag_counts_are_capped() {
        let field = |n: usize| (1..=n).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let aff = "FLAG num\nSFX 1 Y 1\nSFX 1 0 a .\n";
        // At most MAX_FLAGS_PER_ENTRY flags per .dic entry: one more and the entry is skipped.
        let dic = format!("2\nfull/{}\nover/{}\n", field(MAX_FLAGS_PER_ENTRY), field(MAX_FLAGS_PER_ENTRY + 1));
        let d = AffixDict::from_hunspell(aff, &dic, &[]).unwrap();
        assert!(d.check("fulla") && !d.check("over") && !d.check("overa"));
        let cont = format!("FLAG num\nSFX 1 Y 1\nSFX 1 0 a/{} .\n", field(MAX_FLAGS_PER_ENTRY + 1));
        assert!(AffixDict::from_hunspell(&cont, "0\n", &[]).is_err(), "too many continuation classes");
        // More distinct flags than a u16 id holds (only one-character flags can get there): an
        // error, not a panic or a wrapped id.
        let chars: Vec<char> =
            (0x4e00u32..0xa000).chain(0xac00..0xd7a4).chain(0x20000..0x2a6e0).filter_map(char::from_u32).take(MAX_DISTINCT_FLAGS + 1).collect();
        assert_eq!(chars.len(), MAX_DISTINCT_FLAGS + 1);
        let mut dic = String::new();
        for (i, chunk) in chars.chunks(MAX_FLAGS_PER_ENTRY).enumerate() {
            dic.push_str(&format!("w{i}/{}\n", chunk.iter().collect::<String>()));
        }
        let err = AffixDict::from_hunspell("", &dic, &[]).unwrap_err();
        assert!(err.contains("distinct flags"), "{err}");
        // Homonyms are merged into one set. The per-entry cap is for .dic fields, not merged sets:
        // two entries adding up to more than MAX_FLAGS_PER_ENTRY are both kept (up to
        // MAX_FLAGS_PER_STEM), and the compact data reloads with the word and all its flags.
        let aff2 = format!("{aff}SFX {n} Y 1\nSFX {n} 0 b .\n", n = MAX_FLAGS_PER_ENTRY + 5);
        let first = field(MAX_FLAGS_PER_ENTRY);
        let second = (MAX_FLAGS_PER_ENTRY + 1..=MAX_FLAGS_PER_ENTRY + 10).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let dic = format!("2\nsame/{first}\nsame/{second}\n");
        let d = AffixDict::from_hunspell(&aff2, &dic, &[]).unwrap();
        assert!(d.check("same") && d.check("samea") && d.check("sameb"), "the second homonym's flags are merged");
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.check("same") && back.check("samea") && back.check("sameb"));
    }

    /// `n` homonym lines of `same`, each with MAX_FLAGS_PER_ENTRY distinct numeric flags (the
    /// first starting at 1), the last one ending with the flags `extra` (comma-separated).
    fn homonyms(n: usize, extra: &str) -> String {
        let mut dic = format!("{n}\n");
        for e in 0..n {
            let flags = e * MAX_FLAGS_PER_ENTRY + 1..=(e + 1) * MAX_FLAGS_PER_ENTRY;
            let mut flags: Vec<String> = flags.map(|i| i.to_string()).collect();
            if e + 1 == n && !extra.is_empty() {
                // Still exactly MAX_FLAGS_PER_ENTRY flags: the entry itself is within its cap.
                let extra: Vec<&str> = extra.split(',').collect();
                flags.truncate(MAX_FLAGS_PER_ENTRY - extra.len());
                flags.extend(extra.iter().map(ToString::to_string));
            }
            dic.push_str(&format!("same/{}\n", flags.join(",")));
        }
        dic
    }

    #[test]
    fn merged_homonyms_between_the_entry_and_stem_caps_round_trip() {
        // Eight full entries: exactly MAX_FLAGS_PER_STEM merged flags, the largest set build()
        // makes. The compact reader takes it back, word and flags.
        let n = MAX_FLAGS_PER_STEM / MAX_FLAGS_PER_ENTRY;
        let last = MAX_FLAGS_PER_STEM;
        let aff = format!("FLAG num\nSFX 1 Y 1\nSFX 1 0 a .\nSFX {last} Y 1\nSFX {last} 0 z .\n");
        let d = AffixDict::from_hunspell(&aff, &homonyms(n, ""), &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        for d in [&d, &back] {
            assert!(d.check("same") && d.check("samea") && d.check("samez"));
        }
        // One flag past the stem cap in compact data is refused, as an oversized .dic entry is.
        let flags = (1..=MAX_FLAGS_PER_STEM + 1).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let raw = format!("{MAGIC}\nFLAG num\nSFX 1 Y 0 a .\nWORDS\n\0same/{flags}\n\0kot\n");
        let d = AffixDict::from_bytes(&miniz_oxide::deflate::compress_to_vec(raw.as_bytes(), 6)).unwrap();
        assert!(!d.check("same") && d.check("kot"));
    }

    #[test]
    fn a_homonym_past_the_merged_cap_keeps_its_exclusions() {
        // Nine full homonyms: the ninth takes the merged set past MAX_FLAGS_PER_STEM, and it is
        // the only one with the exclusion flag. Its other flags are dropped; the exclusion isn't.
        let n = MAX_FLAGS_PER_STEM / MAX_FLAGS_PER_ENTRY + 1;
        let x = 60_000;
        let rules = "SFX 1 Y 1\nSFX 1 0 a .\n";
        let forbidden = AffixDict::from_hunspell(&format!("FLAG num\nFORBIDDENWORD {x}\n{rules}"), &homonyms(n, &x.to_string()), &[]).unwrap();
        let back = AffixDict::from_bytes(&forbidden.to_bytes()).unwrap();
        for d in [&forbidden, &back] {
            assert!(!d.check("same") && !d.check("samea") && !d.contains("same"), "still forbidden");
            assert!(!d.contains_suggestible("same"));
        }
        let no_suggest = AffixDict::from_hunspell(&format!("FLAG num\nNOSUGGEST {x}\n{rules}"), &homonyms(n, &x.to_string()), &[]).unwrap();
        let back = AffixDict::from_bytes(&no_suggest.to_bytes()).unwrap();
        for d in [&no_suggest, &back] {
            assert!(d.check("same") && d.check("samea"), "correct");
            assert!(!d.contains_suggestible("same") && !d.contains_suggestible("samea"), "but never suggested");
        }
        // Both exclusions on the overflowing homonym: both carried.
        let both = format!("FLAG num\nFORBIDDENWORD {x}\nNOSUGGEST {y}\n{rules}", y = x + 1);
        let d = AffixDict::from_hunspell(&both, &homonyms(n, &format!("{x},{}", x + 1)), &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        for d in [&d, &back] {
            assert!(!d.check("same") && !d.contains_suggestible("same"));
        }
        // A single entry over MAX_FLAGS_PER_ENTRY is not a word, but its exclusion still rules
        // its homonym out.
        let over = format!("{},{x}", (2..=MAX_FLAGS_PER_ENTRY + 1).map(|i| i.to_string()).collect::<Vec<_>>().join(","));
        let d = AffixDict::from_hunspell(&format!("FLAG num\nFORBIDDENWORD {x}\n{rules}"), &format!("2\nsame/1\nsame/{over}\n"), &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        for d in [&d, &back] {
            assert!(!d.check("same") && !d.check("samea"), "an oversized entry's FORBIDDENWORD applies");
        }
        // Alone, an oversized entry is still no word, exclusion or not.
        let d = AffixDict::from_hunspell(&format!("FLAG num\nNOSUGGEST {x}\n{rules}"), &format!("1\nsame/{over}\n"), &[]).unwrap();
        assert!(!d.check("same"));
    }

    #[test]
    fn an_unused_directive_flag_is_not_written_in_character_mode() {
        // FORBIDDENWORD names a flag no stem or rule uses: the compact data leaves it out, as it
        // did before flag ids, in one-character mode only.
        let d = AffixDict::from_hunspell("FORBIDDENWORD F\nNOSUGGEST N\nSFX a Y 1\nSFX a 0 y .\n", "1\nkot/a\n", &[]).unwrap();
        let raw = String::from_utf8(miniz_oxide::inflate::decompress_to_vec(&d.to_bytes()).unwrap()).unwrap();
        assert!(!raw.contains("FORBIDDENWORD") && !raw.contains("NOSUGGEST"), "{raw}");
        // A used one is still written.
        let d = AffixDict::from_hunspell("FORBIDDENWORD F\nSFX a Y 1\nSFX a 0 y .\n", "2\nkot/a\nzly/F\n", &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.check("koty") && !back.check("zly"));
    }

    #[test]
    fn compact_data_without_a_flag_line_still_loads() {
        // Data written before the FLAG line existed: one-character flags, no FLAG line.
        let old = miniz_oxide::deflate::compress_to_vec(b"WCAFF1\n# old data\nFORBIDDENWORD F\nSFX a Y 0 y .\nWORDS\n\x00kot/a\n\x00zly/F\n", 6);
        let d = AffixDict::from_bytes(&old).unwrap();
        assert!(d.check("kot") && d.check("koty") && !d.check("zly"));
        // One-character dictionaries still write no FLAG line, so their data is unchanged.
        let raw = miniz_oxide::inflate::decompress_to_vec(&dict().to_bytes()).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("\nFLAG"));
        // A FLAG line after a flag, or an unknown one, is bad data.
        for head in ["WCAFF1\nSFX a Y 0 y .\nFLAG num\n", "WCAFF1\nFLAG bogus\n"] {
            let bytes = miniz_oxide::deflate::compress_to_vec(format!("{head}WORDS\n\x00kot\n").as_bytes(), 6);
            assert!(AffixDict::from_bytes(&bytes).is_err(), "{head:?}");
        }
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

        #[test]
        fn any_flag_field_reads_without_panicking(field in "[0-9,a-zA-Z!\u{fe0f}\u{1f6ab}]{0,24}", word in "\\PC{0,12}") {
            for mode in ["num", "long", "UTF-8"] {
                let aff = format!("FLAG {mode}
FORBIDDENWORD {field}
SFX {field} Y 1
SFX {field} 0 a/{field} .
");
                for aff in [aff.as_str(), &format!("FLAG {mode}
SFX 12 Y 1
SFX 12 0 a .
")] {
                    if let Ok(d) = AffixDict::from_hunspell(aff, &format!("2
w/{field}
x/12
"), &[]) {
                        let _ = (d.check(&word), d.check("wa"), d.forms("w"), d.forms("x"));
                        let back = AffixDict::from_bytes(&d.to_bytes());
                        proptest::prop_assert!(back.is_ok(), "{mode} {field:?}: {back:?}");
                    }
                }
            }
        }
    }
}
