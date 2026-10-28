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
//! Supported: one-character flags (any Unicode character — `FLAG UTF-8`) and two-character ones
//! (`FLAG long`), at most 128 distinct that matter (flags only stems carry, with no rule or
//! directive using them, are dropped), `PFX`/`SFX` with strip, append and condition (`.`,
//! letters, `[abc]`, `[^abc]`), cross products, continuation classes on suffixes (at most two
//! suffixes in a row, as in Hunspell), `FULLSTRIP`, `FORBIDDENWORD`, `NOSUGGEST`, `KEEPCASE`, `FORCEUCASE`, `BREAK 0`, `ICONV`/`OCONV`,
//! the compounding the Dutch dictionary uses ([`compound`]), and the suggestion tables `TRY`,
//! `KEY`, `MAP` and `REP`. Rejected: numeric flags, `COMPOUNDFLAG`, `NEEDAFFIX` and the other
//! directives that change which words are correct.
//!
//! Lookups never panic: stems sit in one string, found through an open-addressing hash table, and
//! a probe allocates only small scratch strings for the candidate roots it tries.

mod compound;

use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::Hasher;

use crate::patterns::{FxHasher, FxMap};
use compound::{Pattern, Quant};

const MAGIC: &str = "WCAFF1";
/// Longest word (in bytes) looked up; longer input is never a dictionary word.
pub const MAX_WORD_BYTES: usize = 120;
/// Empty slot in the stem hash table.
const EMPTY: u32 = u32::MAX;

/// Directives that change which words are correct in ways this engine doesn't implement.
const UNSUPPORTED: &[&str] =
    &["AF", "AM", "CHECKSHARPS", "CIRCUMFIX", "COMPLEXPREFIXES", "COMPOUNDFLAG", "IGNORE", "LEMMA_PRESENT", "NEEDAFFIX", "PSEUDOROOT", "SUBSTANDARD"];

/// Compounding limits this engine doesn't apply. They only matter when the dictionary compounds
/// (`COMPOUNDRULE`, `COMPOUNDBEGIN`…), so they are an error then and ignored otherwise.
const UNSUPPORTED_WITH_COMPOUNDS: &[&str] = &[
    "CHECKCOMPOUNDREP",
    "CHECKCOMPOUNDTRIPLE",
    "COMPOUNDFORBIDFLAG",
    "COMPOUNDMORESUFFIXES",
    "COMPOUNDROOT",
    "COMPOUNDSYLLABLE",
    "COMPOUNDWORDMAX",
    "SIMPLIFIEDTRIPLE",
];

/// First character a `FLAG long` flag maps to (Supplementary Private Use Area-A): the engine
/// keeps one `char` per flag, and these never appear in a dictionary's own one-character flags.
const LONG_FLAG_BASE: u32 = 0xF_0000;

/// How the `.aff`'s `FLAG` line encodes flags. Each two-character flag (`FLAG long`) becomes one
/// private-use character, so the rest of the engine works with `char` flags in every mode.
#[derive(Default)]
struct FlagCodec {
    long: bool,
    names: HashMap<String, char>,
}

impl FlagCodec {
    /// The character of one flag name (`A`, or `Ca` under `FLAG long`); `None` when the name has
    /// the wrong length for the mode.
    fn flag(&mut self, name: &str) -> Option<char> {
        let mut it = name.chars().filter(|&c| c != EMOJI_SELECTOR || self.long);
        if !self.long {
            return match (it.next(), it.next()) {
                (Some(c), None) if !c.is_whitespace() => Some(c),
                _ => None,
            };
        }
        if name.chars().count() != 2 {
            return None;
        }
        if let Some(&c) = self.names.get(name) {
            return Some(c);
        }
        let n = u32::try_from(self.names.len()).ok()?;
        let c = char::from_u32(LONG_FLAG_BASE.checked_add(n)?)?;
        self.names.insert(name.to_string(), c);
        Some(c)
    }

    /// A flag list (`abc`, or `CaCbCp` under `FLAG long`) as one character per flag. An odd last
    /// character under `FLAG long` is read as a flag of its own, as a lenient reader would.
    fn decode(&mut self, list: &str) -> String {
        if !self.long {
            return list.chars().filter(|&c| c != EMOJI_SELECTOR).collect();
        }
        let chars: Vec<char> = list.chars().collect();
        chars.chunks(2).filter_map(|pair| self.flag(&pair.iter().collect::<String>()).or_else(|| pair.first().copied())).collect()
    }
}

/// Replace, left to right, the longest `from` of `table` found at each position with its `to`
/// (`ICONV`/`OCONV`). Borrowed when nothing matches.
fn convert<'a>(table: &[(Box<str>, Box<str>)], w: &'a str) -> Cow<'a, str> {
    if table.is_empty() || !table.iter().any(|(from, _)| !from.is_empty() && w.contains(&**from)) {
        return Cow::Borrowed(w);
    }
    let mut out = String::with_capacity(w.len());
    let mut i = 0;
    while let Some(rest) = w.get(i..) {
        let Some(c) = rest.chars().next() else { break };
        let best = table.iter().filter(|(from, _)| !from.is_empty() && rest.starts_with(&**from)).max_by_key(|(from, _)| from.len());
        match best {
            Some((from, to)) => {
                out.push_str(to);
                i += from.len();
            }
            None => {
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    Cow::Owned(out)
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

/// One dictionary entry. Homonyms share a spelling/index, but retain separate flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stem {
    index: usize,
    flags: u128,
}

/// Which stem spellings a capitalization probe may consult.
#[derive(Clone, Copy)]
enum StemLookup {
    Exact,
    Uppercase,
    MixedOnly,
}

/// A stem list with affix rules (see the module docs).
#[derive(Clone, Debug, Default)]
pub struct AffixDict {
    /// All stems, concatenated, sorted by bytes.
    words: String,
    offs: Vec<u32>,
    /// Per stem: index into `sets`.
    flags: Vec<u16>,
    /// Additional independent flag sets for a spelling with homonyms.
    homonyms: FxMap<usize, Box<[u16]>>,
    /// Distinct flag masks.
    sets: Vec<u128>,
    /// The `FORBIDDENWORD` flag's bit; stems carrying it are not words (0: none declared).
    forbidden: u128,
    /// Stems with NOSUGGEST are correct, but none of their forms are suggested.
    no_suggest: u128,
    /// `KEEPCASE`: stems correct only in their own case (no capitalized or upper-case variants).
    keep_case: u128,
    /// A final compound component carrying `FORCEUCASE` requires an initial capital.
    force_ucase: u128,
    /// Allow an affix to replace the entire stem (`FULLSTRIP`).
    full_strip: bool,
    /// `BREAK 0`: disable independent hyphen splitting in language wrappers.
    no_break: bool,
    /// `ONLYINCOMPOUND`: stems, and forms of suffixes carrying it, valid only inside compounds.
    only_in_compound: u128,
    /// `COMPOUNDBEGIN`, `COMPOUNDMIDDLE`, `COMPOUNDEND`, `COMPOUNDPERMITFLAG` (0: none declared).
    cpd_begin: u128,
    cpd_middle: u128,
    cpd_end: u128,
    cpd_permit: u128,
    /// `COMPOUNDRULE` patterns: a flag per part, each with its quantifier.
    compound_rules: Vec<Box<[(u128, Quant)]>>,
    /// First-part flags, computed once so optional prefixes are not rescanned per lookup.
    compound_first: u128,
    /// Shortest compound part, in characters (`COMPOUNDMIN`; Hunspell's default is 3).
    compound_min: usize,
    /// `CHECKCOMPOUNDDUP`: a part repeated right after itself isn't a compound.
    check_dup: bool,
    /// `CHECKCOMPOUNDCASE`: no capital on either side of a part boundary (except at a hyphen).
    check_case: bool,
    /// `CHECKCOMPOUNDPATTERN`: part boundaries that aren't allowed.
    patterns: Vec<Pattern>,
    /// `ICONV`/`OCONV`: input conversion before lookups, output conversion of suggestions (the
    /// Dutch dictionary stores `ij` as the single letter `ĳ`).
    iconv: Vec<(Box<str>, Box<str>)>,
    oconv: Vec<(Box<str>, Box<str>)>,
    /// Open-addressing hash table of stem indices (`EMPTY` = free).
    table: Vec<u32>,
    /// Initial-capital aliases of mixed-case stems, used only for uppercase input.
    uppercase_stems: FxMap<Box<str>, Vec<usize>>,
    /// Flag char of each bit.
    flag_chars: Vec<char>,
    prefixes: Vec<Rule>,
    suffixes: Vec<Rule>,
    /// Appended string → groups of suffix rules by stripped string.
    sfx_index: FxMap<Box<str>, Vec<SfxGroup>>,
    /// Per flag bit: the suffix rules of that flag, in rule order.
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
    forbidden: Option<char>,
    no_suggest: Option<char>,
    keep_case: Option<char>,
    force_ucase: Option<char>,
    full_strip: bool,
    no_break: bool,
    only_in_compound: Option<char>,
    cpd_begin: Option<char>,
    cpd_middle: Option<char>,
    cpd_end: Option<char>,
    cpd_permit: Option<char>,
    compound_rules: Vec<Vec<(char, Quant)>>,
    compound_min: Option<usize>,
    check_dup: bool,
    check_case: bool,
    patterns: Vec<RawPattern>,
    iconv: Vec<(String, String)>,
    oconv: Vec<(String, String)>,
    /// How flags are written (one or two characters); the `.dic` is read with the same codec.
    codec: FlagCodec,
}

impl Rules {
    /// Does the dictionary compound (a rule or a part flag declared)?
    fn compounds(&self) -> bool {
        !self.compound_rules.is_empty() || self.cpd_begin.is_some() || self.cpd_middle.is_some() || self.cpd_end.is_some()
    }

    /// Every flag a directive, compound rule or pattern refers to.
    fn special_flags(&self) -> Vec<char> {
        let singles = [
            self.forbidden,
            self.no_suggest,
            self.keep_case,
            self.force_ucase,
            self.only_in_compound,
            self.cpd_begin,
            self.cpd_middle,
            self.cpd_end,
            self.cpd_permit,
        ];
        let mut out: Vec<char> = singles.into_iter().flatten().collect();
        out.extend(self.compound_rules.iter().flatten().map(|&(c, _)| c));
        out.extend(self.patterns.iter().flat_map(|p| [p.1, p.3]).flatten());
        out
    }
}

/// A rule as parsed from text, before flags become bits: `(flag, cross, strip, add, cont, cond)`.
type RawRule = (char, bool, String, String, String, String);

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

/// A `CHECKCOMPOUNDPATTERN` as parsed: `(end, end flag, begin, begin flag)`.
type RawPattern = (String, Option<char>, String, Option<char>);

/// Read a `COMPOUNDRULE` pattern: flags (`ab`, or `(ab)` — required under `FLAG long`), each
/// optionally followed by `*` or `?`.
fn parse_compound_rule(text: &str, codec: &mut FlagCodec) -> Result<Vec<(char, Quant)>, String> {
    let mut out: Vec<(char, Quant)> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let flag = match c {
            '*' | '?' => {
                let Some(last) = out.last_mut().filter(|l| l.1 == Quant::One) else {
                    return Err(format!("`{c}` without a flag in COMPOUNDRULE `{text}`"));
                };
                last.1 = if c == '*' { Quant::Star } else { Quant::Opt };
                continue;
            }
            '(' => {
                let mut name = String::new();
                let mut closed = false;
                for d in chars.by_ref() {
                    if d == ')' {
                        closed = true;
                        break;
                    }
                    name.push(d);
                }
                if !closed {
                    return Err(format!("unclosed `(` in COMPOUNDRULE `{text}`"));
                }
                codec.flag(&name)
            }
            c if !codec.long => Some(c),
            _ => None,
        };
        let Some(flag) = flag else { return Err(format!("bad flag in COMPOUNDRULE `{text}`")) };
        out.push((flag, Quant::One));
    }
    if out.is_empty() {
        return Err("empty COMPOUNDRULE".into());
    }
    Ok(out)
}

/// One side of a `CHECKCOMPOUNDPATTERN`: `text`, `text/flag` or `/flag` (`0` is empty).
fn parse_pattern_side(side: &str, codec: &mut FlagCodec) -> Result<(String, Option<char>), String> {
    match side.split_once('/') {
        Some((text, flag)) => {
            let Some(f) = codec.flag(flag) else { return Err(format!("bad flag in CHECKCOMPOUNDPATTERN `{side}`")) };
            Ok((zero_empty(text), Some(f)))
        }
        None => Ok((zero_empty(side), None)),
    }
}

/// The text of a compound rule, written back with one-character flags in parentheses.
fn compound_rule_text(rule: &[(char, Quant)]) -> String {
    let mut s = String::new();
    for &(c, q) in rule {
        s.push('(');
        s.push(c);
        s.push(')');
        match q {
            Quant::One => {}
            Quant::Opt => s.push('?'),
            Quant::Star => s.push('*'),
        }
    }
    s
}

/// `0` means empty in Hunspell's strip and append fields.
fn zero_empty(s: &str) -> String {
    if s == "0" { String::new() } else { s.to_string() }
}

/// U+FE0F VARIATION SELECTOR-16 asks for the emoji form of the character before it. A dictionary
/// that uses an emoji as a flag may write it after the flag (RLA-ES Spanish: `PFX ☎️`, `banco/S☎️`),
/// but it is not a flag of its own: it is dropped wherever flags are read.
const EMOJI_SELECTOR: char = '\u{fe0f}';

/// Read Hunspell `.aff` text (already decoded to UTF-8).
fn parse_aff(aff: &str) -> Result<Rules, String> {
    let mut r = Rules::default();
    let mut cross: HashMap<(bool, char), bool> = HashMap::new();
    let mut map_header = false;
    let mut rep_header = false;
    let mut rule_header = false;
    let mut pattern_header = false;
    let (mut iconv_header, mut oconv_header) = (false, false);
    let mut compound_limits: Vec<String> = Vec::new();
    for (n, line) in aff.lines().enumerate() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let Some(&dir) = f.first() else { continue };
        if dir.starts_with('#') {
            continue;
        }
        let at = |k: usize| f.get(k).copied().unwrap_or("");
        let is_count = |k: usize| (f.len() == k + 1 || at(k + 1).starts_with('#')) && at(k).parse::<usize>().is_ok();
        match dir {
            "SET" | "LANG" | "NAME" | "VERSION" | "HOME" | "WORDCHARS" | "MAXNGRAMSUGS" | "MAXDIFF" | "ONLYMAXDIFF" | "NOSPLITSUGS"
            | "SUGSWITHDOTS" | "WARN" | "FORBIDWARN" | "MAXCPDSUGS" => {}
            "FLAG" => {
                // Per-character flag modes (`FLAG UTF-8`, as VERO's pt_BR dictionary uses) work
                // with the one-character parsing; `long` (OpenTaal's nl_NL) maps each pair to one
                // character. Numeric flags aren't read.
                match at(1) {
                    "UTF-8" | "DEFAULT" | "utf-8" | "default" | "UTF8" | "" => {}
                    "long" | "LONG" => r.codec.long = true,
                    other => return Err(format!("line {}: FLAG {other} is not supported", n + 1)),
                }
            }
            directive @ ("FORBIDDENWORD" | "NOSUGGEST" | "KEEPCASE" | "FORCEUCASE" | "ONLYINCOMPOUND" | "COMPOUNDBEGIN" | "COMPOUNDMIDDLE"
            | "COMPOUNDEND" | "COMPOUNDPERMITFLAG") => {
                let Some(c) = r.codec.flag(at(1)) else { return Err(format!("line {}: {directive} needs one flag", n + 1)) };
                let slot = match directive {
                    "FORBIDDENWORD" => &mut r.forbidden,
                    "NOSUGGEST" => &mut r.no_suggest,
                    "KEEPCASE" => &mut r.keep_case,
                    "FORCEUCASE" => &mut r.force_ucase,
                    "ONLYINCOMPOUND" => &mut r.only_in_compound,
                    "COMPOUNDBEGIN" => &mut r.cpd_begin,
                    "COMPOUNDMIDDLE" => &mut r.cpd_middle,
                    "COMPOUNDEND" => &mut r.cpd_end,
                    _ => &mut r.cpd_permit,
                };
                *slot = Some(c);
            }
            "FULLSTRIP" => r.full_strip = true,
            "BREAK" if at(1) == "0" => r.no_break = true,
            "COMPOUNDMIN" => r.compound_min = at(1).parse().ok(),
            "CHECKCOMPOUNDDUP" => r.check_dup = true,
            "CHECKCOMPOUNDCASE" => r.check_case = true,
            "COMPOUNDRULE" => {
                if !rule_header && r.compound_rules.is_empty() && is_count(1) {
                    rule_header = true;
                } else {
                    r.compound_rules.push(parse_compound_rule(at(1), &mut r.codec).map_err(|e| format!("line {}: {e}", n + 1))?);
                }
            }
            "CHECKCOMPOUNDPATTERN" => {
                if !pattern_header && r.patterns.is_empty() && is_count(1) {
                    pattern_header = true;
                } else if f.len() >= 3 {
                    if at(1).split('/').next() == Some("0") {
                        return Err(format!("line {}: CHECKCOMPOUNDPATTERN unmodified-stem mode is not supported", n + 1));
                    }
                    // Ignoring a replacement would also discard its boundary prohibition.
                    if f.len() > 3 && !at(3).starts_with('#') {
                        return Err(format!("line {}: CHECKCOMPOUNDPATTERN replacements are not supported", n + 1));
                    }
                    let side = |s: &str, codec: &mut FlagCodec| parse_pattern_side(s, codec).map_err(|e| format!("line {}: {e}", n + 1));
                    let (end, end_flag) = side(at(1), &mut r.codec)?;
                    let (begin, begin_flag) = side(at(2), &mut r.codec)?;
                    r.patterns.push((end, end_flag, begin, begin_flag));
                }
            }
            conv @ ("ICONV" | "OCONV") => {
                let (header, table) = if conv == "ICONV" { (&mut iconv_header, &mut r.iconv) } else { (&mut oconv_header, &mut r.oconv) };
                if !*header && table.is_empty() && is_count(1) {
                    *header = true;
                } else if f.len() >= 3 {
                    table.push((at(1).to_string(), at(2).to_string()));
                }
            }
            d if UNSUPPORTED_WITH_COMPOUNDS.contains(&d) => compound_limits.push(format!("line {}: {d} is not supported", n + 1)),
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
                if !rep_header && is_count(1) {
                    rep_header = true;
                } else if f.len() >= 3 {
                    r.rep.push((at(1).replace('_', " "), at(2).replace('_', " ")));
                }
            }
            "PFX" | "SFX" => {
                let suffix = dir == "SFX";
                let Some(flag) = r.codec.flag(at(1)) else {
                    return Err(format!("line {}: bad flag `{}`", n + 1, at(1)));
                };
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
                let c = cross.get(&(suffix, flag)).copied().unwrap_or(false);
                let cont = r.codec.decode(cont);
                let rule: RawRule = (flag, c, zero_empty(at(2)), zero_empty(add), cont, cond.to_string());
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
    if r.compounds()
        && let Some(e) = compound_limits.into_iter().next()
    {
        return Err(e);
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
        let mut rules = parse_aff(aff)?;
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
            let flags = rules.codec.decode(&flags);
            stems.push((word, flags));
        }
        AffixDict::build(rules, stems, comments.iter().map(ToString::to_string).collect())
    }

    fn build(rules: Rules, mut stems: Vec<(String, String)>, comments: Vec<String>) -> Result<AffixDict, String> {
        // Flags: every one a rule, a directive, a compound rule or a pattern uses gets a bit. A
        // flag only stems carry does nothing here and gets none (OpenTaal's nl_NL marks stems
        // with ~25 such flags, beyond the 128 bits otherwise).
        let mut flag_chars: Vec<char> = rules.prefixes.iter().chain(&rules.suffixes).map(|r| r.0).collect();
        flag_chars.extend(rules.special_flags());
        flag_chars.sort_unstable();
        flag_chars.dedup();
        if flag_chars.len() > 128 {
            return Err(format!("{} distinct flags; at most 128 are supported", flag_chars.len()));
        }
        let bit_of = |c: char| flag_chars.iter().position(|&x| x == c).map_or(0, |i| 1u128 << i);
        let bits = |f: Option<char>| f.map_or(0, bit_of);
        let forbidden = bits(rules.forbidden);
        let conv = |t: Vec<(String, String)>| -> Vec<(Box<str>, Box<str>)> { t.into_iter().map(|(a, b)| (a.into(), b.into())).collect() };
        // Stems sorted by bytes; homonyms share their spelling, not their flag sets.
        stems.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut d = AffixDict {
            words: String::with_capacity(stems.iter().map(|s| s.0.len()).sum()),
            offs: Vec::with_capacity(stems.len() + 1),
            flag_chars: flag_chars.clone(),
            forbidden,
            no_suggest: bits(rules.no_suggest),
            keep_case: bits(rules.keep_case),
            force_ucase: bits(rules.force_ucase),
            full_strip: rules.full_strip,
            no_break: rules.no_break,
            only_in_compound: bits(rules.only_in_compound),
            cpd_begin: bits(rules.cpd_begin),
            cpd_middle: bits(rules.cpd_middle),
            cpd_end: bits(rules.cpd_end),
            cpd_permit: bits(rules.cpd_permit),
            compound_rules: rules.compound_rules.iter().map(|r| r.iter().map(|&(c, q)| (bit_of(c), q)).collect()).collect(),
            // Hunspell's default is 3; `COMPOUNDMIN 0` means 1.
            compound_min: rules.compound_min.unwrap_or(3).max(1),
            check_dup: rules.check_dup,
            check_case: rules.check_case,
            patterns: rules
                .patterns
                .iter()
                .map(|(end, ef, begin, bf)| Pattern {
                    end: end.as_str().into(),
                    end_flag: bits(*ef),
                    begin: begin.as_str().into(),
                    begin_flag: bits(*bf),
                })
                .collect(),
            iconv: conv(rules.iconv),
            oconv: conv(rules.oconv),
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
            let mut ids = Vec::new();
            let mut j = i;
            while let Some((w, f)) = stems.get(j)
                && w == word
            {
                let mask = f.chars().map(bit_of).fold(0, |a, b| a | b);
                let id = if let Some(&id) = set_ids.get(&mask) {
                    id
                } else {
                    let id = u16::try_from(d.sets.len()).map_err(|_| "too many distinct flag sets".to_string())?;
                    d.sets.push(mask);
                    set_ids.insert(mask, id);
                    id
                };
                ids.push(id);
                j += 1;
            }
            ids.sort_unstable();
            ids.dedup();
            if let Some(extra) = ids.get(1..).filter(|extra| !extra.is_empty()) {
                d.homonyms.insert(d.flags.len(), extra.into());
            }
            d.words.push_str(word);
            d.offs.push(u32::try_from(d.words.len()).map_err(|_| "stems too large".to_string())?);
            d.flags.push(ids.first().copied().unwrap_or(0));
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
        // Each (appended, stripped) pair's group, found by hash rather than by scanning the
        // appended string's groups: a dictionary with many strips per suffix stays linear.
        let mut group_of: FxMap<(&str, &str), usize> = FxMap::default();
        let mut by_flag: Vec<Vec<u32>> = vec![Vec::new(); 128];
        for (i, r) in self.suffixes.iter().enumerate() {
            let groups = idx.entry(r.add.clone()).or_default();
            let Ok(i) = u32::try_from(i) else { break };
            match group_of.get(&(&*r.add, &*r.strip)).and_then(|&g| groups.get_mut(g)) {
                Some(g) => g.rules.push(i),
                None => {
                    group_of.insert((&r.add, &r.strip), groups.len());
                    groups.push(SfxGroup { strip: r.strip.clone(), rules: vec![i] });
                }
            }
            if r.bit != 0
                && let Some(list) = by_flag.get_mut(r.bit.trailing_zeros() as usize)
            {
                list.push(i);
            }
        }
        self.sfx_by_flag = by_flag;
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
        self.compound_first = self.compound_rules.iter().fold(0, |mask, rule| mask | compound::first_flags(rule));
        self.uppercase_stems = self.stems().enumerate().filter(|(_, w)| casing(w) == Casing::Mixed).fold(FxMap::default(), |mut index, (i, w)| {
            index.entry(capitalize(&w.to_lowercase()).into_boxed_str()).or_insert_with(Vec::new).push(i);
            index
        });
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

    /// The character of `mask` when it holds exactly one flag.
    fn flag_char(&self, mask: u128) -> Option<char> {
        let s = self.flag_chars_for(mask);
        let mut it = s.chars();
        match (it.next(), it.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        }
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

    /// The index of a spelling in the shared stem text.
    fn stem_index(&self, w: &str) -> Option<usize> {
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
                return Some(t as usize);
            }
            slot = (slot + 1) & mask;
        }
        None
    }

    fn stem_masks(&self, index: usize) -> impl Iterator<Item = u128> + '_ {
        self.flags
            .get(index)
            .into_iter()
            .chain(self.homonyms.get(&index).into_iter().flat_map(|extra| extra.iter()))
            .filter_map(|&id| self.sets.get(usize::from(id)))
            .copied()
    }

    /// Independent homonyms that carry none of the excluded flags. Whole-word prohibitions
    /// retain precedence; entry-specific restrictions such as ONLYINCOMPOUND stay separate.
    fn stems_excluding(&self, w: &str, excluded: u128) -> impl Iterator<Item = Stem> + '_ {
        self.lookup_stems(w, excluded, StemLookup::Exact)
    }

    /// Case aliases participate only in the standalone uppercase fallback. Conditions
    /// still see the probed spelling, as they do for ordinary capitalization variants.
    fn lookup_stems(&self, w: &str, excluded: u128, lookup: StemLookup) -> impl Iterator<Item = Stem> + '_ {
        let aliases = if matches!(lookup, StemLookup::Exact) { None } else { self.uppercase_stems.get(w) };
        let exact = if matches!(lookup, StemLookup::MixedOnly) { None } else { self.stem_index(w) };
        exact.into_iter().chain(aliases.into_iter().flatten().copied()).flat_map(move |index| {
            let prohibited = self.stem_masks(index).any(|f| f & excluded & (self.forbidden | self.no_suggest) != 0);
            self.stem_masks(index).filter(move |f| !prohibited && f & excluded == 0).map(move |flags| Stem { index, flags })
        })
    }

    /// Does `w` carry any flag of `flags` — as a stem, or as the form of a suffix rule whose
    /// continuation classes include one (chained suffixation, depth-bounded)?
    ///
    /// The rules of a group that could lead to `w` are merged into one mask of their flags and
    /// followed once, not once per rule: the answer is the same, but thousands of rules of one
    /// flag cost one walk, not one each (#3613).
    fn form_has_flag(&self, w: &str, flags: u128, depth: u8, excluded: u128, lookup: StemLookup) -> bool {
        if self.lookup_stems(w, excluded, lookup).any(|s| s.flags & flags != 0) {
            return true;
        }
        if depth == 0 {
            return false;
        }
        let mut root = String::with_capacity(w.len() + 8);
        let cuts = std::iter::once(w.len()).chain(w.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0 || self.full_strip));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (w.get(..b), w.get(b..)) else { continue };
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups {
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                let inner = g
                    .rules
                    .iter()
                    .filter_map(|&i| self.suffixes.get(i as usize))
                    .filter(|r| r.cont & excluded & self.only_in_compound == 0 && r.cont & flags != 0 && r.cond_matches(&root, true))
                    .fold(0u128, |m, r| m | r.bit);
                if inner != 0 && self.form_has_flag(&root, inner, depth - 1, excluded, lookup) {
                    return true;
                }
            }
        }
        false
    }

    /// Can a rule's forms stand alone? Not when its continuation classes carry `ONLYINCOMPOUND`
    /// (Dutch `trainings-` exists only as the first part of a compound).
    fn standalone(&self, r: &Rule) -> bool {
        r.cont & self.only_in_compound == 0
    }

    /// `w` after the dictionary's input conversion (`ICONV`): the form its stems are stored in.
    pub fn iconv<'a>(&self, w: &'a str) -> Cow<'a, str> {
        convert(&self.iconv, w)
    }

    /// `w` after the dictionary's output conversion (`OCONV`): how a stored form is shown.
    pub fn oconv<'a>(&self, w: &'a str) -> Cow<'a, str> {
        convert(&self.oconv, w)
    }

    /// Is `w` exactly (case and all) a stem, an affixed form of one or a compound?
    pub fn contains(&self, w: &str) -> bool {
        self.contains_excluding(&self.iconv(w), self.forbidden, compound::CHECK_BUDGET, w.chars().next().is_some_and(char::is_uppercase), 0)
    }

    /// Exactly spelled dictionary words eligible for suggestions (NOSUGGEST stems excluded).
    /// Compounds count only when found quickly: suggesting tries thousands of candidates.
    pub fn contains_suggestible(&self, w: &str) -> bool {
        self.contains_excluding(
            &self.iconv(w),
            self.forbidden | self.no_suggest,
            compound::SUGGEST_BUDGET,
            w.chars().next().is_some_and(char::is_uppercase),
            0,
        )
    }

    /// Explicit forbidden/NOSUGGEST spellings override other analyses of that spelling.
    /// Case and compound-position restrictions remain specific to each independent stem.
    fn excludes_spelling(&self, w: &str, excluded: u128) -> bool {
        let prohibited = excluded & (self.forbidden | self.no_suggest);
        prohibited != 0 && self.stem_index(w).is_some_and(|index| self.stem_masks(index).any(|f| f & prohibited != 0))
    }

    /// `budget`: part lookups a compound analysis may take (0: no compounds).
    /// `head_excluded`: extra restrictions on a case-folded standalone word or compound head.
    fn contains_excluding(&self, w: &str, excluded: u128, budget: u32, capitalized: bool, head_excluded: u128) -> bool {
        self.contains_in_position(w, excluded, budget, capitalized, head_excluded, self.only_in_compound)
    }

    /// `only_in_compound` is the normal position mask, or zero for a prohibition probe.
    fn contains_in_position(&self, w: &str, excluded: u128, budget: u32, capitalized: bool, head_excluded: u128, only_in_compound: u128) -> bool {
        if w.is_empty() || w.len() > MAX_WORD_BYTES {
            return false;
        }
        if self.excludes_spelling(w, excluded) {
            return false;
        }
        let alone = excluded | head_excluded | only_in_compound;
        if self.stems_excluding(w, alone).next().is_some()
            || self.suffixed(w, 0, alone, StemLookup::Exact)
            || self.prefixed(w, alone, StemLookup::Exact)
        {
            return true;
        }
        // An excluded derivation cannot regain spelling or suggestion eligibility as a compound.
        budget > 0 && !self.excludes_derivation(w, excluded) && self.compound(w, excluded, budget, capitalized, head_excluded)
    }

    /// Is `w` a stem with one prefix (and possibly a suffix, as a cross product)?
    fn prefixed(&self, w: &str, excluded: u128, lookup: StemLookup) -> bool {
        let mut root = String::with_capacity(w.len() + 8);
        for r in self.prefixes.iter().filter(|r| r.cont & excluded & self.only_in_compound == 0) {
            let Some(rest) = w.strip_prefix(&*r.add) else { continue };
            if rest.is_empty() && !self.full_strip {
                continue;
            }
            root.clear();
            root.push_str(&r.strip);
            root.push_str(rest);
            if !r.cond_matches(&root, false) {
                continue;
            }
            if self.lookup_stems(&root, excluded, lookup).any(|s| s.flags & r.bit != 0) {
                return true;
            }
            if self.form_has_flag(&root, r.bit, INNER_CHAIN, excluded, lookup) {
                return true;
            }
            if r.cross && self.suffixed(&root, r.bit, excluded, lookup) {
                return true;
            }
        }
        false
    }

    /// Is `w` a stem with one suffix? With `need` set (a prefix was taken off), the stem must
    /// carry those flags too and the suffix must allow cross products.
    fn suffixed(&self, w: &str, need: u128, excluded: u128, lookup: StemLookup) -> bool {
        let mut root = String::with_capacity(w.len() + 8);
        // Each split point from the end: the appended string is `w[b..]`, at most the longest
        // suffix, and something must stay before it.
        let cuts = std::iter::once(w.len()).chain(w.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0 || self.full_strip));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (w.get(..b), w.get(b..)) else { continue };
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups {
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                let rules =
                    || g.rules.iter().filter_map(|&i| self.suffixes.get(i as usize)).filter(|r| r.cont & excluded & self.only_in_compound == 0);
                // The stem path: `root` is a stem carrying the rule's flag (and the prefix's,
                // when a cross product is being checked).
                if self
                    .lookup_stems(&root, excluded, lookup)
                    .any(|s| s.flags & need == need && rules().any(|r| s.flags & r.bit != 0 && (need == 0 || r.cross) && r.cond_matches(&root, true)))
                {
                    return true;
                }
                // Continuation classes: `root` need not be a stem carrying this rule's flag —
                // it may be a form that gained the flag from another rule's continuation. The
                // matching rules' flags are looked for together, in one walk.
                if need == 0 {
                    let flags = rules().filter(|r| r.cond_matches(&root, true)).fold(0u128, |m, r| m | r.bit);
                    if flags != 0 && self.form_has_flag(&root, flags, INNER_CHAIN, excluded, lookup) {
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
        self.check_excluding(&self.iconv(w), self.forbidden, compound::CHECK_BUDGET)
    }

    /// A forbidden stem or derivation must not become valid through a language's hyphen fallback.
    pub(crate) fn forbids(&self, w: &str) -> bool {
        self.excludes_derivation(&self.iconv(w), self.forbidden)
    }

    /// Detect prohibited derivations even when stems or rules require a compound position.
    /// Ignore that position restriction for both probes; never recursively attempt compounds.
    fn excludes_derivation(&self, w: &str, excluded: u128) -> bool {
        let prohibited = excluded & (self.forbidden | self.no_suggest);
        prohibited != 0
            && ((!self.check_in_position(w, prohibited, 0, 0) && self.check_in_position(w, 0, 0, 0))
                || (casing(w) == Casing::Upper
                    && !self.contains_uppercase_in_position(w, prohibited, StemLookup::Uppercase, 0)
                    && self.contains_uppercase_in_position(w, 0, StemLookup::Uppercase, 0)))
    }

    /// Default Hunspell breaks check hyphen components independently; BREAK 0 disables that.
    pub(crate) fn permits_hyphen_breaks(&self) -> bool {
        !self.no_break
    }

    /// A known prohibited spelling must not regain validity through an acronym exemption.
    /// Unknown acronyms remain exempt; listed and affixed KEEPCASE variants do not.
    pub(crate) fn rejects_explicit_word(&self, w: &str) -> bool {
        let w = self.iconv(w);
        if self.check_excluding(&w, self.forbidden, compound::CHECK_BUDGET) {
            return false;
        }
        let core = w.trim_matches('-');
        // Default breaks may accept wrapped lowercase KEEPCASE words, but do not
        // exempt mixed-case KEEPCASE entries such as iPod, even without BREAK 0.
        if core != w
            && casing(core) == Casing::Upper
            && !self.check_excluding(core, self.forbidden, compound::CHECK_BUDGET)
            && self.contains_uppercase(core, 0, StemLookup::MixedOnly)
        {
            return true;
        }
        if self.excludes_derivation(&w, self.forbidden) || self.stem_index(&w).is_some() {
            return true;
        }
        let lower = w.to_lowercase();
        [lower.clone(), capitalize(&lower)].iter().any(|candidate| {
            self.stem_index(candidate).is_some()
                || self.uppercase_stems.contains_key(candidate.as_str())
                || self.contains_excluding(candidate, self.forbidden, compound::CHECK_BUDGET, true, 0)
        }) || (casing(&w) == Casing::Upper && self.contains_uppercase(&w, 0, StemLookup::Uppercase))
    }

    /// Whether a suggestion is correct in its displayed case and has an eligible stem.
    pub fn can_suggest(&self, w: &str) -> bool {
        self.check_excluding(&self.iconv(w), self.forbidden | self.no_suggest, compound::SUGGEST_BUDGET)
    }

    fn check_excluding(&self, w: &str, excluded: u128, budget: u32) -> bool {
        self.check_in_position(w, excluded, budget, self.only_in_compound)
    }

    fn check_in_position(&self, w: &str, excluded: u128, budget: u32, only_in_compound: u128) -> bool {
        // Keep the input capitalization when looking up its lowercase dictionary parts.
        let capitalized = w.chars().next().is_some_and(char::is_uppercase);
        if self.contains_in_position(w, excluded, budget, capitalized, 0, only_in_compound) {
            return true;
        }
        let case = casing(w);
        if matches!(case, Casing::Lower | Casing::Mixed) {
            return false;
        }
        // A prohibited exact spelling or derivation is terminal. Case folding must not
        // reinterpret it as a different, allowed compound (or a suggestion).
        let prohibited = excluded & (self.forbidden | self.no_suggest);
        if prohibited != 0 && (self.excludes_spelling(w, prohibited) || self.contains_in_position(w, 0, 0, capitalized, 0, only_in_compound)) {
            return false;
        }
        // A folded standalone word or compound head must permit capitalization. Later
        // compound components retain their dictionary case, including KEEPCASE stems.
        match case {
            Casing::Lower | Casing::Mixed => false,
            Casing::Initial => self.contains_in_position(&w.to_lowercase(), excluded, budget, capitalized, self.keep_case, only_in_compound),
            Casing::Upper => {
                let lower = w.to_lowercase();
                self.contains_in_position(&capitalize(&lower), excluded, budget, capitalized, self.keep_case, only_in_compound)
                    || self.contains_in_position(&lower, excluded, budget, capitalized, self.keep_case, only_in_compound)
                    || self.contains_uppercase_in_position(w, excluded | self.keep_case, StemLookup::MixedOnly, only_in_compound)
            }
        }
    }

    /// All-caps input also matches mixed-case dictionary entries (`Noord-Holland`).
    /// Affixes are analysed on the initial-capital probe, rather than uppercasing
    /// their literals or conditions. Exact-case APIs never use these aliases.
    fn contains_uppercase(&self, w: &str, excluded: u128, lookup: StemLookup) -> bool {
        self.contains_uppercase_in_position(w, excluded, lookup, self.only_in_compound)
    }

    fn contains_uppercase_in_position(&self, w: &str, excluded: u128, lookup: StemLookup, only_in_compound: u128) -> bool {
        if w.is_empty() || w.len() > MAX_WORD_BYTES {
            return false;
        }
        let lower = w.to_lowercase();
        let excluded = excluded | only_in_compound;
        [capitalize(&lower), lower].iter().any(|word| {
            !self.excludes_spelling(word, excluded)
                && (self.lookup_stems(word, excluded, lookup).next().is_some()
                    || self.suffixed(word, 0, excluded, lookup)
                    || self.prefixed(word, excluded, lookup))
        })
    }

    /// Every form of the stem `stem` (itself included), unsorted, possibly with repeats; at most
    /// [`MAX_FORMS`], and at most [`MAX_FORMS_WORK`] rules tried along the continuation chains and
    /// cross products, so a dictionary with thousands of self-continuing rules can't make one stem
    /// cost millions of strings (#3613).
    pub fn forms(&self, stem: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut work = 0usize;
        // Homonyms remain independent, but share the per-stem output and work limits.
        for s in self.stems_excluding(stem, self.forbidden | self.only_in_compound) {
            if out.len() >= MAX_FORMS || work > MAX_FORMS_WORK {
                break;
            }
            self.forms_for(stem, s.flags, &mut out, &mut work);
        }
        out
    }

    fn forms_for(&self, stem: &str, f: u128, out: &mut Vec<String>, work: &mut usize) {
        out.push(stem.to_string());
        if out.len() >= MAX_FORMS {
            return;
        }
        let mut cross_sfx = Vec::new();
        // Suffix forms queued with the continuation classes they carry, for chained expansion;
        // each (form, classes) pair is expanded once.
        let mut chain: Vec<(String, u128)> = Vec::new();
        let mut queued: std::collections::HashSet<(String, u128)> = std::collections::HashSet::new();
        for r in self.suffixes.iter().filter(|r| self.standalone(r)) {
            if f & r.bit == 0 || !r.cond_matches(stem, true) {
                continue;
            }
            let Some(base) = stem.strip_suffix(&*r.strip) else { continue };
            if base.is_empty() && !self.full_strip {
                continue;
            }
            let form = format!("{base}{}", r.add);
            if r.cross {
                cross_sfx.push(form.clone());
            }
            if r.cont != 0 && queued.insert((form.clone(), r.cont)) {
                chain.push((form.clone(), r.cont));
            }
            out.push(form);
            if out.len() >= MAX_FORMS {
                return;
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
                    *work += 1;
                    if *work > MAX_FORMS_WORK {
                        break 'chain;
                    }
                    if !self.standalone(r) || !r.cond_matches(&form, true) {
                        continue;
                    }
                    let Some(base) = form.strip_suffix(&*r.strip) else { continue };
                    if base.is_empty() && !self.full_strip {
                        continue;
                    }
                    let next = format!("{base}{}", r.add);
                    if r.cont != 0 && queued.insert((next.clone(), r.cont)) {
                        chain.push((next.clone(), r.cont));
                    }
                    out.push(next);
                    if out.len() >= MAX_FORMS {
                        return;
                    }
                }
            }
        }
        for r in self.prefixes.iter().filter(|r| self.standalone(r)) {
            if f & r.bit == 0 || !r.cond_matches(stem, false) {
                continue;
            }
            let Some(rest) = stem.strip_prefix(&*r.strip) else { continue };
            if !rest.is_empty() || self.full_strip {
                out.push(format!("{}{rest}", r.add));
            }
            if r.cross {
                for s in &cross_sfx {
                    *work += 1;
                    if out.len() >= MAX_FORMS || *work > MAX_FORMS_WORK {
                        return;
                    }
                    if let Some(rest) = s.strip_prefix(&*r.strip)
                        && (!rest.is_empty() || self.full_strip)
                    {
                        out.push(format!("{}{rest}", r.add));
                    }
                }
            }
            if out.len() >= MAX_FORMS {
                return;
            }
        }
    }

    /// The suffix rules carrying any flag of `mask`, in rule order.
    fn suffixes_with(&self, mask: u128) -> Vec<u32> {
        let mut rules: Vec<u32> = Vec::new();
        let mut lists = 0;
        for (bit, list) in self.sfx_by_flag.iter().enumerate() {
            if bit < 128 && mask & (1u128 << bit) != 0 && !list.is_empty() {
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
        for (directive, mask) in [
            ("FORBIDDENWORD", self.forbidden),
            ("NOSUGGEST", self.no_suggest),
            ("KEEPCASE", self.keep_case),
            ("FORCEUCASE", self.force_ucase),
            ("ONLYINCOMPOUND", self.only_in_compound),
            ("COMPOUNDBEGIN", self.cpd_begin),
            ("COMPOUNDMIDDLE", self.cpd_middle),
            ("COMPOUNDEND", self.cpd_end),
            ("COMPOUNDPERMITFLAG", self.cpd_permit),
        ] {
            if mask != 0
                && let Some(c) = self.flag_char(mask)
            {
                out.push_str(&format!("{directive} {c}\n"));
            }
        }
        if self.compounds() {
            out.push_str(&format!("COMPOUNDMIN {}\n", self.compound_min));
        }
        if self.full_strip {
            out.push_str("FULLSTRIP\n");
        }
        if self.check_dup {
            out.push_str("CHECKCOMPOUNDDUP\n");
        }
        if self.check_case {
            out.push_str("CHECKCOMPOUNDCASE\n");
        }
        for rule in &self.compound_rules {
            let chars: Option<Vec<(char, Quant)>> = rule.iter().map(|&(bit, q)| self.flag_char(bit).map(|c| (c, q))).collect();
            if let Some(chars) = chars {
                out.push_str(&format!("COMPOUNDRULE {}\n", compound_rule_text(&chars)));
            }
        }
        for p in &self.patterns {
            let side = |text: &str, bit: u128| {
                let mut s = if text.is_empty() { "0".to_string() } else { text.to_string() };
                if let Some(c) = self.flag_char(bit) {
                    s.push('/');
                    s.push(c);
                }
                s
            };
            out.push_str(&format!("CHECKCOMPOUNDPATTERN {} {}\n", side(&p.end, p.end_flag), side(&p.begin, p.begin_flag)));
        }
        for (directive, table) in [("ICONV", &self.iconv), ("OCONV", &self.oconv)] {
            for (from, to) in
                table.iter().filter(|(a, b)| !a.is_empty() && !b.is_empty() && !a.contains(char::is_whitespace) && !b.contains(char::is_whitespace))
            {
                out.push_str(&format!("{directive} {from} {to}\n"));
            }
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
        if self.no_break {
            out.push_str("BREAK 0\n");
        }
        out.push_str("WORDS\n");
        let mut raw = out.into_bytes();
        let mut prev: &[u8] = &[];
        for i in 0..self.len() {
            let Some(w) = self.word(i) else { continue };
            let b = w.as_bytes();
            // Repeated spellings use the existing front-coded WORDS syntax; no new format.
            for mask in self.stem_masks(i) {
                let p = prev.iter().zip(b).take_while(|(x, y)| x == y).count().min(255);
                raw.push(p as u8);
                raw.extend_from_slice(b.get(p..).unwrap_or_default());
                if mask != 0 {
                    raw.push(b'/');
                    raw.extend_from_slice(self.flag_chars_for(mask).as_bytes());
                }
                raw.push(b'\n');
                prev = b;
            }
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
                "TRY" => rules.try_chars = cap_try(at(1)),
                "KEY" => rules.key = at(1).to_string(),
                "MAP" => rules.map.push(at(1).chars().collect()),
                "REP" => rules.rep.push((at(1).replace('_', " "), at(2).replace('_', " "))),
                directive @ ("FORBIDDENWORD" | "NOSUGGEST" | "KEEPCASE" | "FORCEUCASE" | "ONLYINCOMPOUND" | "COMPOUNDBEGIN" | "COMPOUNDMIDDLE"
                | "COMPOUNDEND" | "COMPOUNDPERMITFLAG") => {
                    let Some(c) = rules.codec.flag(at(1)) else { return Err(format!("affix dictionary: bad {directive} in `{line}`")) };
                    let slot = match directive {
                        "FORBIDDENWORD" => &mut rules.forbidden,
                        "NOSUGGEST" => &mut rules.no_suggest,
                        "KEEPCASE" => &mut rules.keep_case,
                        "FORCEUCASE" => &mut rules.force_ucase,
                        "ONLYINCOMPOUND" => &mut rules.only_in_compound,
                        "COMPOUNDBEGIN" => &mut rules.cpd_begin,
                        "COMPOUNDMIDDLE" => &mut rules.cpd_middle,
                        "COMPOUNDEND" => &mut rules.cpd_end,
                        _ => &mut rules.cpd_permit,
                    };
                    *slot = Some(c);
                }
                "FULLSTRIP" => rules.full_strip = true,
                "BREAK" if at(1) == "0" => rules.no_break = true,
                "COMPOUNDMIN" => rules.compound_min = at(1).parse().ok(),
                "CHECKCOMPOUNDDUP" => rules.check_dup = true,
                "CHECKCOMPOUNDCASE" => rules.check_case = true,
                "COMPOUNDRULE" => {
                    let rule = parse_compound_rule(at(1), &mut rules.codec).map_err(|e| format!("affix dictionary: {e}"))?;
                    rules.compound_rules.push(rule);
                }
                "CHECKCOMPOUNDPATTERN" => {
                    // Compact data uses 0 for an empty side. The public Hunspell end-0 mode
                    // is different and is rejected by parse_aff; keep older compact data readable.
                    let (end, end_flag) = parse_pattern_side(at(1), &mut rules.codec)?;
                    let (begin, begin_flag) = parse_pattern_side(at(2), &mut rules.codec)?;
                    rules.patterns.push((end, end_flag, begin, begin_flag));
                }
                "ICONV" => rules.iconv.push((at(1).to_string(), at(2).to_string())),
                "OCONV" => rules.oconv.push((at(1).to_string(), at(2).to_string())),
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
    #[test]
    fn fullstrip_cross_products_preserve_the_remaining_suffix() {
        for enabled in [false, true] {
            for (rules, combined, alone, needs_fullstrip) in [
                ("PFX A Y 1\nPFX A ab c .\nSFX B Y 1\nSFX B 0 x .\n", "cx", "c", false),
                ("PFX A Y 1\nPFX A 0 c .\nSFX B Y 1\nSFX B ab y .\n", "cy", "y", true),
            ] {
                let aff = format!("SET UTF-8\n{}{rules}", if enabled { "FULLSTRIP\n" } else { "" });
                let d = super::AffixDict::from_hunspell(&aff, "1\nab/AB\n", &[]).unwrap();
                for dict in [d.clone(), super::AffixDict::from_bytes(&d.to_bytes()).unwrap()] {
                    assert_eq!(dict.contains(combined), enabled || !needs_fullstrip, "{combined}, FULLSTRIP={enabled}");
                    assert_eq!(dict.forms("ab").iter().any(|w| w == combined), enabled || !needs_fullstrip, "{combined}, FULLSTRIP={enabled}");
                    assert_eq!(dict.contains(alone), enabled, "{alone}, FULLSTRIP={enabled}");
                    assert_eq!(dict.forms("ab").iter().any(|w| w == alone), enabled, "{alone}, FULLSTRIP={enabled}");
                }
            }
        }
    }

    #[test]
    fn fullstrip_replaces_the_entire_stem_only_when_declared() {
        let rules = "SET UTF-8\nFLAG long\nPFX AB Y 1\nPFX AB à pq .\nSFX CD Y 1\nSFX CD à xy/EF .\nSFX EF Y 1\nSFX EF xy zz .\n";
        for enabled in [false, true] {
            let aff = format!("{}{}", if enabled { "FULLSTRIP\n" } else { "" }, rules);
            let d = super::AffixDict::from_hunspell(&aff, "1\nà/ABCD\n", &[]).unwrap();
            for dict in [d.clone(), super::AffixDict::from_bytes(&d.to_bytes()).unwrap()] {
                for word in ["pq", "xy", "zz"] {
                    assert_eq!(dict.contains(word), enabled, "{word}, FULLSTRIP={enabled}");
                    assert_eq!(dict.forms("à").iter().any(|f| f == word), enabled, "{word}, FULLSTRIP={enabled}");
                }
                assert!(dict.contains("à"));
                assert!(!dict.contains(""));
                assert!(!dict.contains("xyz"));
            }
        }
    }

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
        // mode the reader already assumes. Only `long` and `num` stay unsupported.
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
        assert!(!d.flag_chars.contains(&EMOJI_SELECTOR), "the selector is not a flag");
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
    fn forbidden_compound_only_spellings_block_language_hyphen_fallback() {
        let dictionary =
            AffixDict::from_hunspell("FORBIDDENWORD F\nONLYINCOMPOUND X\n", "4\nalpha\nbeta\nalpha-beta/FX\nalpha-beta/X\n", &[]).unwrap();
        let restored = AffixDict::from_bytes(&dictionary.to_bytes()).unwrap();
        for dictionary in [&dictionary, &restored] {
            for word in ["alpha-beta", "Alpha-beta", "ALPHA-BETA", "-alpha-beta", "alpha-beta-"] {
                assert!(!dictionary.check(word), "{word}");
                assert!(!crate::ca::is_correct_with(dictionary, word), "Catalan {word}");
                assert!(!crate::nl::is_correct_with(dictionary, word), "Dutch {word}");
            }
            assert!(dictionary.forbids("alpha-beta"));
        }
        let allowed = AffixDict::from_hunspell("ONLYINCOMPOUND X\n", "3\nalpha\nbeta\nalpha-beta/X\n", &[]).unwrap();
        assert!(!allowed.check("alpha-beta"));
        assert!(!allowed.forbids("alpha-beta"));
        assert!(crate::ca::is_correct_with(&allowed, "alpha-beta"));
        assert!(crate::nl::is_correct_with(&allowed, "alpha-beta"));
    }

    #[test]
    fn forbidden_compound_only_derivations_block_language_hyphen_fallback() {
        for (affix, entries, words) in [
            (
                "SFX S Y 1\nSFX S 0 s .\nPFX P Y 1\nPFX P 0 pre .\n",
                "3\nalpha/P\nbeta/S\nalpha-beta/FXSP\n",
                &["alpha-betas", "prealpha-beta", "prealpha-betas", "Alpha-betas", "ALPHA-BETAS"][..],
            ),
            ("SFX S N 1\nSFX S 0 s/T .\nSFX T N 1\nSFX T 0 t .\n", "3\nalpha\nbeta/S\nalpha-beta/FXS\n", &["alpha-betast"][..]),
            ("SFX S N 1\nSFX S 0 s/X .\nSFX B N 1\nSFX B 0 s .\n", "3\nalpha\nbeta/B\nalpha-beta/FS\n", &["alpha-betas"][..]),
        ] {
            let affix = format!("FORBIDDENWORD F\nONLYINCOMPOUND X\n{affix}");
            let forbidden = AffixDict::from_hunspell(&affix, entries, &[]).unwrap();
            let restored = AffixDict::from_bytes(&forbidden.to_bytes()).unwrap();
            let allowed = AffixDict::from_hunspell(&affix, &entries.replace("/F", "/"), &[]).unwrap();
            for word in words {
                for dictionary in [&forbidden, &restored] {
                    assert!(dictionary.forbids(word), "forbidden derivation {word}");
                    assert!(!crate::ca::is_correct_with(dictionary, word), "Catalan {word}");
                    assert!(!crate::nl::is_correct_with(dictionary, word), "Dutch {word}");
                }
                assert!(!allowed.forbids(word), "allowed derivation {word}");
                assert!(crate::ca::is_correct_with(&allowed, word), "Catalan allowed {word}");
                assert!(crate::nl::is_correct_with(&allowed, word), "Dutch allowed {word}");
            }
        }
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
    fn forms_budget_is_shared_across_independent_homonyms() {
        let mut aff = String::from("SET UTF-8\nSFX A Y 3000\nSFX B Y 3000\n");
        for flag in ['A', 'B'] {
            for i in 0..3000 {
                aff.push_str(&format!("SFX {flag} 0 x{i}/{flag} .\n"));
            }
        }
        let d = AffixDict::from_hunspell(&aff, "2\nkot/A\nkot/B\n", &[]).unwrap();
        let forms = d.forms("kot");
        assert!(forms.len() <= MAX_FORMS, "{} forms exceed the per-stem cap", forms.len());
        assert!(forms.iter().any(|f| f == "kotx0x1"));

        // Starting the next homonym can consume the final slot with the stem itself.
        let mut aff = format!("SET UTF-8\nSFX A N {}\nSFX B N 1\nSFX B 0 b .\n", MAX_FORMS - 2);
        for i in 0..MAX_FORMS - 2 {
            aff.push_str(&format!("SFX A 0 a{i} .\n"));
        }
        let d = AffixDict::from_hunspell(&aff, "2\nkot/A\nkot/B\n", &[]).unwrap();
        assert_eq!(d.forms("kot").len(), MAX_FORMS);
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
    fn long_flags_map_to_one_character_each() {
        // `FLAG long` (OpenTaal nl_NL): two characters per flag, in rules, continuations and stems.
        let aff = "SET UTF-8\nFLAG long\nFORBIDDENWORD Fw\nSFX Aa Y 1\nSFX Aa 0 en/Bb .\nSFX Bb Y 1\nSFX Bb 0 s .\nPFX Pa Y 1\nPFX Pa 0 ge .\n";
        let d = AffixDict::from_hunspell(aff, "3\nwerk/AaPa\nloop/Bb\nfout/Fw\n", &[]).unwrap();
        for w in ["werk", "werken", "werkens", "gewerk", "gewerken", "loops"] {
            assert!(d.contains(w), "{w}");
        }
        for w in ["werks", "loopen", "geloop", "fout", "werkA", "werka"] {
            assert!(!d.contains(w), "{w}");
        }
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.contains("gewerken") && back.contains("werkens") && !back.contains("fout"));
        // A flag name of the wrong length is an error, not a silent mismatch.
        assert!(AffixDict::from_hunspell("FLAG long\nSFX A Y 1\nSFX A 0 s .\n", "1\nwerk/A\n", &[]).is_err());
        assert!(AffixDict::from_hunspell("FLAG long\nFORBIDDENWORD F\n", "1\nwerk\n", &[]).is_err());
    }

    #[test]
    fn flags_only_stems_carry_take_no_bit() {
        // 140 one-character flags on a stem, two of them used: the unused ones don't count
        // against the 128-bit limit.
        let flags: String = (0..140u32).filter_map(|i| char::from_u32(0x4E00 + i)).collect();
        let d = AffixDict::from_hunspell("SET UTF-8\nSFX 一 Y 1\nSFX 一 0 s .\n", &format!("1\nwerk/{flags}\n"), &[]).unwrap();
        assert!(d.contains("werk") && d.contains("werks"));
    }

    #[test]
    fn input_and_output_conversion() {
        // OpenTaal stores `ij` as the ligature `ĳ` and converts typed text with ICONV.
        let aff = "SET UTF-8\nICONV 3\nICONV ’ '\nICONV ij ĳ\nICONV IJ Ĳ\nOCONV 2\nOCONV ĳ ij\nOCONV Ĳ IJ\nSFX A Y 1\nSFX A 0 's .\n";
        let d = AffixDict::from_hunspell(aff, "3\nzĳn\nĲsland\nauto/A\n", &[]).unwrap();
        for w in ["zijn", "zĳn", "IJsland", "Ĳsland", "ZIJN", "Zijn", "auto's", "auto’s"] {
            assert!(d.check(w), "{w}");
        }
        for w in ["zjin", "Ijsland", "ijsland"] {
            assert!(!d.check(w), "{w}");
        }
        assert_eq!(d.iconv("bij zijn IJsland"), "bĳ zĳn Ĳsland");
        assert_eq!(d.oconv("Ĳsland zĳn"), "IJsland zijn");
        assert_eq!(d.iconv("geen"), "geen");
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.check("zijn") && back.check("IJsland") && back.check("auto’s"));
        assert_eq!(back.oconv("zĳn"), "zijn");
    }

    #[test]
    fn keepcase_stems_match_only_as_written() {
        let aff = "SET UTF-8\nKEEPCASE K\nSFX s Y 1\nSFX s 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "2\niPod/Ks\nauto/s\n", &[]).unwrap();
        assert!(d.check("iPod") && d.check("iPods"));
        assert!(!d.check("IPOD") && !d.check("Ipod") && !d.check("ipod"));
        assert!(d.check("Auto") && d.check("AUTOS"));
        assert!(!AffixDict::from_bytes(&d.to_bytes()).unwrap().check("IPOD"));
    }

    #[test]
    fn initial_capital_keeps_unchanged_compound_component_case() {
        for rules in ["COMPOUNDBEGIN B\nCOMPOUNDEND E\n", "COMPOUNDRULE 1\nCOMPOUNDRULE BE\n"] {
            let aff = format!("SET UTF-8\nKEEPCASE K\nCOMPOUNDMIN 2\n{rules}");
            for (dic, initial_ok) in [("2\nalpha/B\nbeta/EK\n", true), ("2\nalpha/BK\nbeta/E\n", false)] {
                let d = AffixDict::from_hunspell(&aff, dic, &[]).unwrap();
                let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
                for d in [&d, &back] {
                    assert!(d.check("alphabeta"));
                    assert_eq!(d.check("Alphabeta"), initial_ok, "{dic}{rules}");
                    assert_eq!(d.can_suggest("Alphabeta"), initial_ok, "{dic}{rules}");
                    assert_eq!(d.check("ALPHABETA"), initial_ok, "{dic}{rules}");
                }
            }
            for flag in ['F', 'N'] {
                let aff = format!("{aff}FORBIDDENWORD F\nNOSUGGEST N\n");
                let dic = format!("3\nalpha/B\nbeta/EK\nalphabeta/{flag}\n");
                let d = AffixDict::from_hunspell(&aff, &dic, &[]).unwrap();
                assert_eq!(d.check("Alphabeta"), flag != 'F');
                assert!(!d.can_suggest("Alphabeta"));
            }
        }
    }

    #[test]
    fn explicit_case_prohibitions_precede_compound_folding() {
        let aff = "SET UTF-8\nFORBIDDENWORD F\nNOSUGGEST N\nCOMPOUNDBEGIN B\nCOMPOUNDEND E\nSFX S N 1\nSFX S 0 s .\n";
        for flag in ['F', 'N'] {
            let dic = format!("3\nalpha/B\nbeta/ES\nAlphabeta/{flag}S\n");
            let d = AffixDict::from_hunspell(aff, &dic, &[]).unwrap();
            let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
            for d in [&d, &back] {
                assert!(d.check("alphabeta") && d.check("alphabetas"));
                for word in ["Alphabeta", "Alphabetas"] {
                    assert_eq!(d.check(word), flag != 'F', "{flag}: {word}");
                    assert!(!d.can_suggest(word), "{flag}: {word}");
                }
            }
        }
    }

    #[test]
    fn nosuggest_derivations_cannot_reenter_through_compounds() {
        let aff = "SET UTF-8\nNOSUGGEST N\nCOMPOUNDMIN 2\nCOMPOUNDBEGIN B\nCOMPOUNDEND E\nSFX S N 1\nSFX S 0 s .\nPFX P N 1\nPFX P 0 pre .\n";
        let d = AffixDict::from_hunspell(aff, "5\nalpha/B\nbeta/ES\nalphabeta/NSP\nprealpha/B\ngamma/B\n", &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        for d in [&d, &back] {
            for word in ["alphabeta", "alphabetas", "prealphabeta", "Alphabeta", "Alphabetas", "ALPHABETA", "ALPHABETAS"] {
                assert!(d.check(word), "NOSUGGEST remains correct: {word}");
                assert!(!d.can_suggest(word), "excluded standalone derivation: {word}");
            }
            assert!(!d.contains_suggestible("alphabetas") && !d.contains_suggestible("prealphabeta"));
            for word in ["beta", "betas", "gammabeta", "gammabetas"] {
                assert!(d.check(word) && d.can_suggest(word), "independent allowed form: {word}");
            }
        }
    }

    #[test]
    fn numeric_table_headers_allow_trailing_comments() {
        let aff = "SET UTF-8\nFLAG long\nCOMPOUNDMIN 2\nCOMPOUNDRULE 1\nCOMPOUNDRULE (AA)(BB)\nCHECKCOMPOUNDPATTERN 1\nCHECKCOMPOUNDPATTERN x y\nICONV 1\nICONV ij ĳ\nOCONV 1\nOCONV ĳ ij\nREP 2\nREP foo bar\nREP hash #\n";
        let dic = "3\nalpha/AA\nbeta/BB\nzĳn\n";
        for header in ["COMPOUNDRULE 1", "CHECKCOMPOUNDPATTERN 1", "ICONV 1", "OCONV 1", "REP 2"] {
            let commented = aff.replace(&format!("{header}\n"), &format!("{header} # original count comment\n"));
            let d = AffixDict::from_hunspell(&commented, dic, &[]).unwrap();
            let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
            for d in [&d, &back] {
                assert!(d.check("alphabeta") && d.check("zijn"), "{header}");
                assert_eq!(d.iconv("1ij"), "1ĳ", "{header}");
                assert_eq!(d.oconv("1ĳ"), "1ij", "{header}");
                assert_eq!(d.patterns.len(), 1, "{header}");
                assert_eq!(d.rep(), &[("foo".to_string(), "bar".to_string()), ("hash".to_string(), "#".to_string())], "{header}");
            }
        }
    }

    #[test]
    fn uppercase_mixed_case_entries_and_affixes_keep_case_controls() {
        for (aff, dic, uppercase_ok) in [
            ("BREAK 0\n", "1\nNoord-Holland\n", true),
            ("BREAK 0\nKEEPCASE K\n", "1\nNoord-Holland/K\n", false),
            ("BREAK 0\nSFX S N 1\nSFX S 0 s .\n", "1\nNoord-Holland/S\n", true),
        ] {
            let d = AffixDict::from_hunspell(aff, dic, &[]).unwrap();
            let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
            for d in [&d, &back] {
                assert!(d.check("Noord-Holland"));
                assert!(!d.contains("NOORD-HOLLAND"), "contains remains exact-case");
                assert_eq!(d.check("NOORD-HOLLAND"), uppercase_ok);
                assert_eq!(d.can_suggest("NOORD-HOLLAND"), uppercase_ok);
                assert!(!d.check("Noord-holland") && !d.check("noord-holland"));
                if aff.contains("SFX") {
                    assert!(d.check("Noord-Hollands") && d.check("NOORD-HOLLANDS"));
                }
            }
        }
    }

    #[test]
    fn uppercase_aliases_keep_affix_literals_conditions_and_chain_limits() {
        for (aff, dic, good, bad) in [
            (
                "PFX P Y 1\nPFX P 0 pre e\nSFX S Y 1\nSFX S l ls l\n",
                "1\neMail/PS\n",
                vec!["eMail", "EMAIL", "eMails", "EMAILS", "preeMail", "preeMails"],
                vec!["email", "Email", "PREEMAIL", "PREEMAILS"],
            ),
            ("SFX S N 1\nSFX S 0 s Holland\n", "1\nNoord-Holland/S\n", vec!["Noord-Hollands", "NOORD-HOLLAND"], vec!["NOORD-HOLLANDS"]),
            ("SFX S N 1\nSFX S 0 S l\n", "1\neMail/S\n", vec!["eMailS", "EMAIL"], vec!["EMAILS"]),
            (
                "SFX S N 1\nSFX S 0 s/T l\nSFX T N 1\nSFX T 0 en s\n",
                "1\neMail/S\n",
                vec!["eMails", "EMAILS", "eMailsen", "EMAILSEN"],
                vec!["EMAILSENEN"],
            ),
        ] {
            let d = AffixDict::from_hunspell(&format!("SET UTF-8\nBREAK 0\n{aff}"), dic, &[]).unwrap();
            let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
            for d in [&d, &back] {
                for word in &good {
                    assert!(d.check(word) && d.can_suggest(word), "{word}: {aff}");
                }
                for word in &bad {
                    assert!(!d.check(word) && !d.can_suggest(word), "{word}: {aff}");
                }
                assert!(!d.contains("EMAIL") && !d.contains_suggestible("EMAIL"));
            }
        }
    }

    #[test]
    fn uppercase_aliases_keep_forbidden_nosuggest_and_keepcase_restrictions() {
        let aff = "SET UTF-8\nBREAK 0\nKEEPCASE K\nFORBIDDENWORD F\nNOSUGGEST N\nSFX S N 1\nSFX S 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "3\neMail/KS\nLaSer/FS\nRaDar/NS\n", &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        for d in [&d, &back] {
            assert!(d.check("eMail") && d.check("eMails"));
            for word in ["EMAIL", "EMAILS", "LASER", "LASERS"] {
                assert!(!d.check(word) && !d.can_suggest(word), "{word}");
            }
            for word in ["RaDar", "RaDars", "RADAR", "RADARS"] {
                assert!(d.check(word) && !d.can_suggest(word), "{word}");
            }
        }
    }

    #[test]
    fn unsupported_and_broken_input_is_an_error() {
        assert!(AffixDict::from_hunspell("FLAG num\n", DIC, &[]).is_err());
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
