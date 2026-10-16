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
//! Supported: one-character flags (at most 256 distinct, any Unicode character — `FLAG UTF-8`),
//! flag aliases (`AF`: a number standing for a set of flags), `PFX`/`SFX` with strip, append and
//! condition (`.`, letters, `[abc]`, `[^abc]`), cross products, continuation classes on suffixes
//! (at most two suffixes in a row, as in Hunspell), `FORBIDDENWORD`, `NOSUGGEST`, `KEEPCASE` (the
//! stem is a word only in its own letter case), input conversion (`ICONV`: ligatures such as `ﬁ`
//! read as `fi`) and ignored characters (`IGNORE`), and the suggestion tables `TRY`, `KEY`, `MAP`
//! and `REP`.
//! Morphological data (`AM` aliases, the fields after a stem) is skipped. Stems marked
//! `NEEDAFFIX` are words only with an affix, and compound words and word breaks follow
//! [`compound`]. Rejected: long or numeric flags, `CIRCUMFIX`, `FULLSTRIP`, `COMPLEXPREFIXES`
//! and `CHECKSHARPS`, which change which words are correct in ways this engine doesn't implement.
//!
//! Lookups never panic: stems sit in one string, found through an open-addressing hash table, and
//! a probe allocates only small scratch strings for the candidate roots it tries.

use std::collections::HashMap;
use std::hash::Hasher;

use crate::patterns::{FxHasher, FxMap};

mod compound;

use compound::Compounding;

const MAGIC: &str = "WCAFF1";
/// Longest word (in bytes) looked up; longer input is never a dictionary word.
pub const MAX_WORD_BYTES: usize = 120;
/// Empty slot in the stem hash table.
const EMPTY: u32 = u32::MAX;

/// Directives that change which words are correct in ways this engine doesn't implement.
const UNSUPPORTED: &[&str] = &["CHECKSHARPS", "CIRCUMFIX", "COMPLEXPREFIXES", "FULLSTRIP", "OCONV"];

/// Most distinct flags a dictionary may use: every single-byte flag (the Hungarian dictionary
/// uses 158).
const MAX_FLAGS: usize = 256;

/// A set of flags, one bit per distinct flag character of the dictionary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
struct Mask([u64; MAX_FLAGS / 64]);

impl Mask {
    const EMPTY: Mask = Mask([0; MAX_FLAGS / 64]);

    /// The set holding flag number `i` alone (empty when `i` is out of range).
    fn bit(i: usize) -> Mask {
        let mut m = Mask::EMPTY;
        if let Some(w) = m.0.get_mut(i / 64) {
            *w = 1 << (i % 64);
        }
        m
    }

    fn is_empty(self) -> bool {
        self.0.iter().all(|&w| w == 0)
    }

    /// Do the two sets share a flag?
    fn meets(self, other: Mask) -> bool {
        !(self & other).is_empty()
    }
}

impl std::ops::BitAnd for Mask {
    type Output = Mask;
    fn bitand(mut self, o: Mask) -> Mask {
        for (a, b) in self.0.iter_mut().zip(o.0) {
            *a &= b;
        }
        self
    }
}

impl std::ops::BitOr for Mask {
    type Output = Mask;
    fn bitor(mut self, o: Mask) -> Mask {
        self |= o;
        self
    }
}

impl std::ops::Not for Mask {
    type Output = Mask;
    fn not(mut self) -> Mask {
        for a in self.0.iter_mut() {
            *a = !*a;
        }
        self
    }
}

impl std::ops::BitOrAssign for Mask {
    fn bitor_assign(&mut self, o: Mask) {
        for (a, b) in self.0.iter_mut().zip(o.0) {
            *a |= b;
        }
    }
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

/// One prefix or suffix rule.
#[derive(Clone, Debug)]
struct Rule {
    /// Bit of the rule's flag in the stems' flag masks.
    bit: Mask,
    flag: char,
    cross: bool,
    strip: Box<str>,
    add: Box<str>,
    /// Continuation classes (`SFX A 0 able/B`): flags the derived form carries, licensing
    /// further affixes. Chained application is bounded by [`MAX_CHAIN`].
    cont: Mask,
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

/// One way a word is a stem with affixes: the stem's flags (whatever they are; callers decide
/// what counts) and the rules taken off, the outer suffix first.
#[derive(Clone, Copy, Debug)]
struct Parse<'r> {
    stem: Mask,
    prefix: Option<&'r Rule>,
    suffixes: [Option<&'r Rule>; 2],
}

impl Parse<'_> {
    fn affixed(&self) -> bool {
        self.prefix.is_some() || self.suffixes[0].is_some()
    }

    fn suffix_rules(&self) -> impl Iterator<Item = &Rule> {
        self.suffixes.iter().flatten().copied()
    }

    /// Continuation classes of every affix taken off.
    fn conts(&self) -> Mask {
        self.prefix.iter().copied().chain(self.suffix_rules()).fold(Mask::EMPTY, |m, r| m | r.cont)
    }

    /// Continuation classes of the outermost affix (the outer suffix, else the prefix).
    fn outer_cont(&self) -> Mask {
        self.suffixes[0].or(self.prefix).map_or(Mask::EMPTY, |r| r.cont)
    }
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
    sets: Vec<Mask>,
    /// The `FORBIDDENWORD` flag's bit; stems carrying it are not words (0: none declared).
    forbidden: Mask,
    /// Stems with NOSUGGEST are correct, but none of their forms are suggested.
    no_suggest: Mask,
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
    /// The `KEEPCASE` flag's bit: such stems are words only in their own letter case (0: none).
    keepcase: Mask,
    /// `ICONV`: input conversions applied before a lookup, `(from, to)`.
    iconv: Vec<(String, String)>,
    /// `IGNORE`: characters removed from input words (and from the stems and affixes).
    ignore: Box<[char]>,
    /// Compounding, `NEEDAFFIX` and `BREAK` (see [`compound`]).
    compound: Compounding,
    /// The directives `compound` was built from, kept for [`AffixDict::to_bytes`].
    compound_lines: Vec<String>,
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
    /// The `KEEPCASE` flag.
    keepcase: Option<char>,
    iconv: Vec<(String, String)>,
    ignore: Vec<char>,
    /// `AF` flag aliases: alias `n` (1-based) stands for `aliases[n - 1]`.
    aliases: Vec<String>,
    /// Compounding, `NEEDAFFIX` and `BREAK` directives, one line each (see [`compound`]).
    compound_lines: Vec<String>,
    /// `AM` morphological aliases: alias `n` (1-based) stands for `morph_aliases[n - 1]`.
    morph_aliases: Vec<String>,
}

impl Rules {
    /// Flags as written in a `.dic` entry or a continuation class: an `AF` alias number when the
    /// dictionary declares aliases, otherwise the flag characters themselves.
    fn flags<'a>(&'a self, written: &'a str) -> Result<&'a str, String> {
        if self.aliases.is_empty() || written.is_empty() || !written.bytes().all(|b| b.is_ascii_digit()) {
            return Ok(written);
        }
        written
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| self.aliases.get(i))
            .map(String::as_str)
            .ok_or_else(|| format!("flag alias {written} is not defined"))
    }

    /// `w` without the `IGNORE` characters.
    fn strip_ignored(&self, w: &str) -> String {
        if self.ignore.is_empty() { w.to_string() } else { w.chars().filter(|c| !self.ignore.contains(c)).collect() }
    }
}

/// The fields of an `.aff`/`.dic` line: Hunspell separates them with spaces and tabs only, so a
/// flag byte read as U+0085 or U+00A0 (see [`crate::hunspell::decode`]) stays a flag.
fn split_fields(line: &str) -> impl Iterator<Item = &str> {
    line.split([' ', '\t']).filter(|f| !f.is_empty())
}

/// The one flag character a directive (`FORBIDDENWORD w`) names.
fn one_flag(arg: &str, dir: &str, n: usize) -> Result<char, String> {
    let mut it = arg.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => Ok(c),
        _ => Err(format!("line {}: {dir} needs one flag character", n + 1)),
    }
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
    let mut af_header = false;
    let mut am_header = false;
    let mut iconv_header = false;
    let mut tables = compound::TableHeaders::default();
    for (n, line) in aff.lines().enumerate() {
        let f: Vec<&str> = split_fields(line).collect();
        let Some(&dir) = f.first() else { continue };
        if dir.starts_with('#') {
            continue;
        }
        if compound::DIRECTIVES.contains(&dir) {
            if let Some(l) = tables.line(&f) {
                r.compound_lines.push(l);
            }
            continue;
        }
        let at = |k: usize| f.get(k).copied().unwrap_or("");
        match dir {
            "SET" | "LANG" | "NAME" | "VERSION" | "HOME" | "WORDCHARS" | "MAXNGRAMSUGS" | "MAXDIFF" | "ONLYMAXDIFF" | "NOSPLITSUGS"
            | "SUGSWITHDOTS" | "WARN" | "FORBIDWARN" | "MAXCPDSUGS" => {}
            // Morphological aliases, and flags that only matter for morphological generation
            // and suggestions: the words they mark are correct.
            "SUBSTANDARD" | "LEMMA_PRESENT" => {}
            "AM" => {
                if !am_header && f.len() == 2 && at(1).parse::<usize>().is_ok() {
                    am_header = true;
                } else {
                    r.morph_aliases.push(f.get(1..).unwrap_or_default().join(" "));
                }
            }
            "AF" => {
                if !af_header && f.len() == 2 && at(1).parse::<usize>().is_ok() {
                    af_header = true;
                } else {
                    r.aliases.push(at(1).to_string());
                }
            }
            "ICONV" => {
                if !iconv_header && f.len() == 2 && at(1).parse::<usize>().is_ok() {
                    iconv_header = true;
                } else if f.len() >= 3 {
                    r.iconv.push((at(1).to_string(), at(2).to_string()));
                }
            }
            "IGNORE" => r.ignore = at(1).chars().collect(),
            "KEEPCASE" => r.keepcase = Some(one_flag(at(1), dir, n)?),
            "FLAG" => {
                // Per-character flag modes (`FLAG UTF-8`, as VERO's pt_BR dictionary uses) work
                // with the one-character parsing here; only the multi-character encodings differ.
                if !matches!(at(1), "UTF-8" | "DEFAULT" | "utf-8" | "default" | "UTF8" | "") {
                    return Err(format!("line {}: FLAG {} is not supported (one-character flags only)", n + 1, at(1)));
                }
            }
            "FORBIDDENWORD" => r.forbidden = Some(one_flag(at(1), dir, n)?),
            "NOSUGGEST" => r.no_suggest = Some(one_flag(at(1), dir, n)?),
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
    // Aliases may be declared after the rules that use them; resolve once all are read.
    let resolve = |rules: &mut Vec<RawRule>, r: &Rules| -> Result<(), String> {
        for rule in rules.iter_mut() {
            rule.4 = r.flags(&rule.4)?.to_string();
            rule.2 = r.strip_ignored(&rule.2);
            rule.3 = r.strip_ignored(&rule.3);
        }
        Ok(())
    };
    let (mut pfx, mut sfx) = (std::mem::take(&mut r.prefixes), std::mem::take(&mut r.suffixes));
    resolve(&mut pfx, &r)?;
    resolve(&mut sfx, &r)?;
    (r.prefixes, r.suffixes) = (pfx, sfx);
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
        let vowels = compound::syllable_vowels(&rules.compound_lines);
        let morph_aliases = std::mem::take(&mut rules.morph_aliases);
        let mut stems: Vec<(String, String)> = Vec::new();
        for (n, line) in dic.lines().enumerate() {
            // A tab ends the entry, which may then hold spaces (`ipso facto`, a phrase that is
            // never one word); without a tab the first space does. Morphological fields follow.
            let (tok, morph) = match line.split_once('\t') {
                Some((entry, morph)) => (entry.trim_matches(' '), morph.trim()),
                None => {
                    let line = line.trim_start_matches(' ');
                    line.split_once(' ').map_or((line, ""), |(entry, morph)| (entry, morph.trim()))
                }
            };
            if tok.is_empty() {
                continue;
            }
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
            let word = rules.strip_ignored(&word);
            if word.is_empty() || word.len() > MAX_WORD_BYTES || word.contains(['\n', '/']) {
                continue;
            }
            let mut flags = rules.flags(&flags).map_err(|e| format!(".dic line {}: {e}", n + 1))?.to_string();
            // An inflected form listed as a stem (`falevelek`, lemma `falevél`): its extra
            // syllables are inflection, which the 6–3 rule doesn't count.
            if let Some(v) = &vowels {
                let morph = match morph.parse::<usize>().ok().and_then(|i| i.checked_sub(1)).and_then(|i| morph_aliases.get(i)) {
                    Some(m) => m.as_str(),
                    None => morph,
                };
                // Only inflection: a derivation (`ds:`) forms a new word, which counts.
                if let Some(lemma) = split_fields(morph).find_map(|f| f.strip_prefix("st:"))
                    && !split_fields(morph).any(|f| f.starts_with("ds:"))
                {
                    let count = |s: &str| s.chars().filter(|c| v.contains(c)).count();
                    flags.extend(compound::lemma_flag(count(&word).saturating_sub(count(lemma))));
                }
            }
            stems.push((word, flags));
        }
        AffixDict::build(rules, stems, comments.iter().map(|s| s.to_string()).collect())
    }

    fn build(rules: Rules, mut stems: Vec<(String, String)>, comments: Vec<String>) -> Result<AffixDict, String> {
        // Flags: every one a rule or a stem uses gets a bit.
        let mut flag_chars: Vec<char> = rules.prefixes.iter().chain(&rules.suffixes).map(|r| r.0).collect();
        flag_chars.extend(stems.iter().flat_map(|s| s.1.chars()));
        flag_chars.sort_unstable();
        flag_chars.dedup();
        if flag_chars.len() > MAX_FLAGS {
            return Err(format!("{} distinct flags; at most {MAX_FLAGS} are supported", flag_chars.len()));
        }
        let bit_of = |c: char| flag_chars.iter().position(|&x| x == c).map_or(Mask::EMPTY, Mask::bit);
        let forbidden = rules.forbidden.map_or(Mask::EMPTY, bit_of);
        let keepcase = rules.keepcase.map_or(Mask::EMPTY, bit_of);
        let compound = Compounding::from_lines(&rules.compound_lines, &bit_of)?;
        // `NEEDAFFIX` and `ONLYINCOMPOUND` describe one entry; merged homonyms keep them only
        // when every entry has them (`üzelm` needs an affix, a plain homonym wouldn't).
        let status = compound.need_affix | compound.only_in;
        // Stems sorted by bytes; homonyms (the same word twice) are merged.
        stems.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut d = AffixDict {
            words: String::with_capacity(stems.iter().map(|s| s.0.len()).sum()),
            offs: Vec::with_capacity(stems.len() + 1),
            flag_chars: flag_chars.clone(),
            forbidden,
            no_suggest: rules.no_suggest.map_or(Mask::EMPTY, bit_of),
            keepcase,
            compound,
            compound_lines: rules.compound_lines,
            iconv: rules.iconv,
            ignore: rules.ignore.into_boxed_slice(),
            try_chars: rules.try_chars,
            key: rules.key,
            map: rules.map,
            rep: rules.rep,
            comments,
            ..AffixDict::default()
        };
        d.offs.push(0);
        let mut set_ids: HashMap<Mask, u16> = HashMap::new();
        let mut i = 0;
        while i < stems.len() {
            let Some((word, _)) = stems.get(i) else { break };
            let mut mask = Mask::EMPTY;
            let mut common = status;
            let mut j = i;
            while let Some((w, f)) = stems.get(j)
                && w == word
            {
                let m = f.chars().map(bit_of).fold(Mask::EMPTY, |a, b| a | b);
                mask |= m;
                common = common & m;
                j += 1;
            }
            mask = (mask & !status) | common;
            let next_id = u16::try_from(d.sets.len()).map_err(|_| "too many distinct flag sets".to_string())?;
            let id = *set_ids.entry(mask).or_insert(next_id);
            if usize::from(id) == d.sets.len() {
                d.sets.push(mask);
            }
            d.words.push_str(word);
            d.offs.push(u32::try_from(d.words.len()).map_err(|_| "stems too large".to_string())?);
            d.flags.push(id);
            i = j;
        }
        let mk = |(flag, cross, strip, add, cont, cond): RawRule| -> Result<Rule, String> {
            let cont = cont.chars().fold(Mask::EMPTY, |m, c| m | bit_of(c));
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
    fn flag_chars_for(&self, mask: Mask) -> String {
        self.flag_chars.iter().enumerate().filter(|&(i, _)| mask.meets(Mask::bit(i))).map(|(_, c)| c).collect()
    }

    /// The character of `mask` when it holds exactly one flag.
    fn flag_char(&self, mask: Mask) -> Option<char> {
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

    /// Flag mask of a stem, if `w` is one and is a word (not `FORBIDDENWORD`).
    fn stem_flags(&self, w: &str) -> Option<Mask> {
        self.stem_mask(w).filter(|m| !m.meets(self.forbidden))
    }

    /// Flag mask of a stem, if `w` is one, whatever its flags.
    fn stem_mask(&self, w: &str) -> Option<Mask> {
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
                let id = *self.flags.get(t as usize)?;
                return self.sets.get(usize::from(id)).copied();
            }
            slot = (slot + 1) & mask;
        }
        None
    }

    /// Every way `w` is a stem with affixes, within Hunspell's bounds (a prefix; at most two
    /// suffixes in a row through continuation classes; a prefix with one suffix when the suffix's
    /// continuation licenses the prefix or both allow cross products), handed to `f` until it
    /// returns `true`.
    fn parses(&self, w: &str, f: &mut dyn FnMut(&Parse<'_>) -> bool) -> bool {
        if w.is_empty() || w.len() > MAX_WORD_BYTES {
            return false;
        }
        if let Some(stem) = self.stem_mask(w)
            && f(&Parse { stem, prefix: None, suffixes: [None, None] })
        {
            return true;
        }
        if self.suffix_parses(w, None, f) {
            return true;
        }
        let mut root = String::with_capacity(w.len() + 8);
        for p in &self.prefixes {
            let Some(rest) = w.strip_prefix(&*p.add) else { continue };
            if rest.is_empty() {
                continue;
            }
            root.clear();
            root.push_str(&p.strip);
            root.push_str(rest);
            if !p.cond_matches(&root, false) {
                continue;
            }
            if let Some(stem) = self.stem_mask(&root)
                && stem.meets(p.bit)
                && f(&Parse { stem, prefix: Some(p), suffixes: [None, None] })
            {
                return true;
            }
            if self.suffix_parses(&root, Some(p), f) {
                return true;
            }
        }
        false
    }

    /// The `(base, group)` splits of `w` where a suffix group's appended text ends the word;
    /// the root is `base` plus the group's stripped text.
    fn suffix_splits<'a>(&'a self, w: &'a str) -> impl Iterator<Item = (&'a str, &'a SfxGroup)> + 'a {
        // Each split point from the end: the appended string is `w[b..]`, at most the longest
        // suffix, and something must stay before it.
        let cuts = std::iter::once(w.len()).chain(w.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0));
        cuts.take(self.max_sfx_chars + 1)
            .filter_map(move |b| {
                let (base, add) = (w.get(..b)?, w.get(b..)?);
                Some(self.sfx_index.get(add)?.iter().map(move |g| (base, g)))
            })
            .flatten()
    }

    /// Parses of `w` as a stem with one suffix, or (without a prefix) two in a row.
    fn suffix_parses<'r>(&'r self, w: &str, prefix: Option<&'r Rule>, f: &mut dyn FnMut(&Parse<'_>) -> bool) -> bool {
        let mut root = String::with_capacity(w.len() + 8);
        let mut inner = String::with_capacity(w.len() + 8);
        for (base, g) in self.suffix_splits(w) {
            root.clear();
            root.push_str(base);
            root.push_str(&g.strip);
            for r1 in g.rules.iter().filter_map(|&i| self.suffixes.get(i as usize)) {
                if !r1.cond_matches(&root, true) {
                    continue;
                }
                if let Some(stem) = self.stem_mask(&root)
                    && stem.meets(r1.bit)
                    && prefix.is_none_or(|p| r1.cont.meets(p.bit) || (p.cross && r1.cross && stem.meets(p.bit)))
                    && f(&Parse { stem, prefix, suffixes: [Some(r1), None] })
                {
                    return true;
                }
                if prefix.is_some() {
                    continue;
                }
                // Continuation classes: `root` may itself be a stem with a suffix whose
                // continuation licenses `r1`.
                for (base2, g2) in self.suffix_splits(&root) {
                    inner.clear();
                    inner.push_str(base2);
                    inner.push_str(&g2.strip);
                    for r2 in g2.rules.iter().filter_map(|&i| self.suffixes.get(i as usize)) {
                        if r2.cont.meets(r1.bit)
                            && r2.cond_matches(&inner, true)
                            && let Some(stem) = self.stem_mask(&inner)
                            && stem.meets(r2.bit)
                            && f(&Parse { stem, prefix: None, suffixes: [Some(r1), Some(r2)] })
                        {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Is this parse a word on its own? Not when its stem is `FORBIDDENWORD`, in `deny`, or
    /// `ONLYINCOMPOUND`; not a bare `NEEDAFFIX` stem; not when the outermost affix still needs
    /// another (`NEEDAFFIX` in its continuation) or an affix may only stand inside compounds.
    fn standalone(&self, p: &Parse<'_>, deny: Mask) -> bool {
        let c = &self.compound;
        if p.stem.meets(self.forbidden | deny | c.only_in) {
            return false;
        }
        if !p.affixed() && p.stem.meets(c.need_affix) {
            return false;
        }
        !p.outer_cont().meets(c.need_affix) && !p.conts().meets(c.only_in)
    }

    /// Is `w` (normalized) a word without compounding: a stem or an affixed form of one?
    fn simple(&self, w: &str, deny: Mask) -> bool {
        self.parses(w, &mut |p| self.standalone(p, deny))
    }

    /// Is `w` (normalized, case as written) a word: listed and not forbidden, an affixed form,
    /// or a compound? A stem listed as `FORBIDDENWORD` is wrong however else it could be read.
    fn is_word(&self, w: &str, deny: Mask) -> bool {
        if self.stem_mask(w).is_some_and(|m| m.meets(self.forbidden)) {
            return false;
        }
        self.simple(w, deny) || self.compound_word(w, deny)
    }

    /// Is `w` exactly (case and all) a stem, an affixed form of one or a compound? `ICONV` and
    /// `IGNORE` apply first.
    pub fn contains(&self, w: &str) -> bool {
        self.is_word(&self.normalize(w), Mask::EMPTY)
    }

    /// Exactly spelled dictionary words eligible for suggestions (NOSUGGEST stems excluded).
    pub fn contains_suggestible(&self, w: &str) -> bool {
        self.is_word(&self.normalize(w), self.no_suggest)
    }

    /// `w` after the `ICONV` conversions (at each position the longest match wins) and without
    /// the `IGNORE` characters.
    fn normalize<'a>(&self, w: &'a str) -> std::borrow::Cow<'a, str> {
        if self.iconv.is_empty() && self.ignore.is_empty() {
            return std::borrow::Cow::Borrowed(w);
        }
        let mut out = String::with_capacity(w.len());
        let mut rest = w;
        while let Some(c) = rest.chars().next() {
            let hit = self.iconv.iter().filter(|(from, _)| !from.is_empty() && rest.starts_with(from.as_str())).max_by_key(|(from, _)| from.len());
            match hit {
                Some((from, to)) => {
                    out.extend(to.chars().filter(|c| !self.ignore.contains(c)));
                    rest = rest.get(from.len()..).unwrap_or_default();
                }
                None => {
                    if !self.ignore.contains(&c) {
                        out.push(c);
                    }
                    rest = rest.get(c.len_utf8()..).unwrap_or_default();
                }
            }
        }
        std::borrow::Cow::Owned(out)
    }

    /// Is `w` correct, Hunspell style: as written, or — for a capitalized word or one in capitals
    /// — in lower case (and capitalized), since a sentence may start with any word and headings
    /// may be in capitals. Lower-case input never matches a capitalized stem (`warszawa`), and a
    /// `KEEPCASE` stem matches only as written; a word in capitals with a hyphen may also be an
    /// abbreviation with a lower-case ending (`SMS-ET` as `SMS-et`). Each of these readings may
    /// also be split at its `BREAK` points, its sides then checked as written: `Petőfi-vers` and
    /// `PETŐFI-VERS` are correct, `SÁMBOKI-HAZAY` (`Sámboki-hazay`) isn't.
    pub fn check(&self, w: &str) -> bool {
        self.check_except(w, Mask::EMPTY)
    }

    /// Whether a suggestion is correct in its displayed case and has an eligible stem.
    pub fn can_suggest(&self, w: &str) -> bool {
        self.check_except(w, self.no_suggest)
    }

    fn check_except(&self, w: &str, deny: Mask) -> bool {
        let w = self.normalize(w);
        let mut budget = compound::BREAK_BUDGET;
        self.case_readings(&w).into_iter().any(|(v, case_deny)| {
            let deny = deny | case_deny;
            self.is_word(&v, deny) || self.breaks_ok(&v, deny, &mut budget)
        })
    }

    /// The readings of `w` [`AffixDict::check`] tries, with the flags each must not rely on.
    fn case_readings(&self, w: &str) -> Vec<(String, Mask)> {
        let mut out = vec![(w.to_string(), Mask::EMPTY)];
        let deny = self.keepcase;
        match casing(w) {
            Casing::Lower | Casing::Mixed => {}
            Casing::Initial => out.push((w.to_lowercase(), deny)),
            Casing::Upper => {
                let lower = w.to_lowercase();
                out.push((capitalize(&lower), deny));
                out.push((lower, deny));
                if let Some((head, tail)) = w.rsplit_once('-')
                    && !tail.is_empty()
                {
                    out.push((format!("{head}-{}", tail.to_lowercase()), deny));
                }
            }
        }
        out
    }

    /// Is `w`, as written, correct when split at its `BREAK` points? A listed
    /// `FORBIDDENWORD` stays wrong even when its sides are right (`Rákóczi-híd`).
    fn breaks_ok(&self, w: &str, deny: Mask, budget: &mut u32) -> bool {
        !self.stem_mask(w).is_some_and(|m| m.meets(self.forbidden)) && self.check_breaks(w, deny, budget)
    }

    /// One side of a break: a word as written, or one that breaks again.
    fn side_ok(&self, w: &str, deny: Mask, budget: &mut u32) -> bool {
        self.is_word(w, deny) || self.breaks_ok(w, deny, budget)
    }

    /// Every form of the stem `stem` (itself included), unsorted, possibly with repeats.
    pub fn forms(&self, stem: &str) -> Vec<String> {
        let Some(f) = self.stem_flags(stem) else { return Vec::new() };
        let mut out = vec![stem.to_string()];
        let mut cross_sfx = Vec::new();
        // Suffix forms queued with the continuation classes they carry, for chained expansion.
        let mut chain: Vec<(String, Mask)> = Vec::new();
        for r in &self.suffixes {
            if !f.meets(r.bit) || !r.cond_matches(stem, true) {
                continue;
            }
            let Some(base) = stem.strip_suffix(&*r.strip) else { continue };
            let form = format!("{base}{}", r.add);
            if r.cross {
                cross_sfx.push(form.clone());
            }
            if !r.cont.is_empty() {
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
                    if !flags.meets(r.bit) || !r.cond_matches(&form, true) {
                        continue;
                    }
                    let Some(base) = form.strip_suffix(&*r.strip) else { continue };
                    let next = format!("{base}{}", r.add);
                    if !r.cont.is_empty() {
                        chain.push((next.clone(), r.cont));
                    }
                    out.push(next);
                }
            }
        }
        for r in &self.prefixes {
            if !f.meets(r.bit) || !r.cond_matches(stem, false) {
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
        for (directive, mask) in [("FORBIDDENWORD", self.forbidden), ("NOSUGGEST", self.no_suggest), ("KEEPCASE", self.keepcase)] {
            if !mask.is_empty()
                && let Some(c) = self.flag_char(mask)
            {
                out.push_str(&format!("{directive} {c}\n"));
            }
        }
        for (a, b) in &self.iconv {
            out.push_str(&format!("ICONV {a} {b}\n"));
        }
        if !self.ignore.is_empty() {
            out.push_str(&format!("IGNORE {}\n", self.ignore.iter().collect::<String>()));
        }
        for l in compound::with_table_headers(&self.compound_lines) {
            out.push_str(&l);
            out.push('\n');
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
            let mask = self.flags.get(i).and_then(|&id| self.sets.get(usize::from(id))).copied().unwrap_or_default();
            if !mask.is_empty() {
                raw.push(b'/');
                raw.extend_from_slice(self.flag_chars_for(mask).as_bytes());
            }
            raw.push(b'\n');
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
        let mut tables = compound::TableHeaders::default();
        for line in lines {
            if let Some(c) = line.strip_prefix("# ") {
                comments.push(c.to_string());
                continue;
            }
            let f: Vec<&str> = split_fields(line).collect();
            let at = |k: usize| f.get(k).copied().unwrap_or("");
            if compound::DIRECTIVES.contains(&at(0)) {
                if let Some(l) = tables.line(&f) {
                    rules.compound_lines.push(l);
                }
                continue;
            }
            match at(0) {
                "TRY" => rules.try_chars = at(1).to_string(),
                "KEY" => rules.key = at(1).to_string(),
                directive @ ("FORBIDDENWORD" | "NOSUGGEST" | "KEEPCASE") => {
                    let c = one_flag(at(1), directive, 0).map_err(|_| format!("affix dictionary: bad {directive} in `{line}`"))?;
                    match directive {
                        "FORBIDDENWORD" => rules.forbidden = Some(c),
                        "NOSUGGEST" => rules.no_suggest = Some(c),
                        _ => rules.keepcase = Some(c),
                    }
                }
                "ICONV" if f.len() == 3 => rules.iconv.push((at(1).to_string(), at(2).to_string())),
                "IGNORE" => rules.ignore = at(1).chars().collect(),
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
        // letter, the digits and eight accented letters: 70 flags, more than 64.
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
    fn more_than_128_distinct_flags_load() {
        // The Hungarian dictionary uses 158 single-byte flags: 200 flags here, U+0080 up.
        let flags: String = (0x80u32..0x80 + 200).filter_map(char::from_u32).collect();
        let mut aff = String::from("SET UTF-8\n");
        for (i, c) in flags.chars().enumerate() {
            aff.push_str(&format!("SFX {c} N 1\nSFX {c} 0 x{i} .\n"));
        }
        let half: String = flags.chars().step_by(2).collect();
        let d = AffixDict::from_hunspell(&aff, &format!("1\nszó/{half}\n"), &[]).unwrap();
        assert!(d.contains("szóx0") && d.contains("szóx198") && d.contains("szóx130"));
        assert!(!d.contains("szóx1") && !d.contains("szóx199") && !d.contains("szóx131"));
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.contains("szóx198") && !back.contains("szóx199"));
        let too_many: String = (0x100u32..0x100 + 257).filter_map(char::from_u32).collect();
        assert!(AffixDict::from_hunspell("SET UTF-8\n", &format!("1\nszó/{too_many}\n"), &[]).is_err());
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
    fn flag_aliases_stand_for_flag_sets() {
        // `AF`: `/1` in the `.dic` and in a continuation class means the first alias's flags.
        // Morphological aliases (`AM`) and the fields after a stem are skipped.
        let aff = "SET UTF-8\nAF 2\nAF ab # 1\nAF c # 2\nAM 1\nAM po:noun\nSFX a Y 1\nSFX a 0 ok/2 . 1\nSFX b Y 1\nSFX b 0 at . 1\nSFX c Y 1\nSFX c 0 ban . 1\n";
        let d = AffixDict::from_hunspell(aff, "1\nház/1\t1\n", &[]).unwrap();
        assert!(d.contains("ház") && d.contains("házok") && d.contains("házat") && d.contains("házokban"));
        assert!(!d.contains("házban"), "the stem has a and b, not c");
        assert!(AffixDict::from_hunspell(aff, "1\nház/3\n", &[]).is_err(), "alias 3 is not defined");
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.contains("házokban") && !back.contains("házban"), "aliases are expanded in the compact format");
    }

    #[test]
    fn iconv_and_ignore_normalize_input() {
        // Ligatures read as their letters; `IGNORE` characters vanish from input and stems.
        let aff = "SET UTF-8\nICONV 2\nICONV ﬁ fi\nICONV ﬄ ffl\nIGNORE ()\nSFX a Y 1\nSFX a 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "2\nfine/a\nwaf(fle)/a\n", &[]).unwrap();
        assert!(d.check("ﬁne") && d.check("ﬁnes") && d.check("Fine") && d.contains("ﬁne"));
        assert!(d.check("waffle") && d.check("waﬄes") && d.check("wa(ffle)"));
        assert!(!d.check("ﬁner"));
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.check("ﬁnes") && back.check("waﬄe") && back.check("waf(fle)"));
    }

    #[test]
    fn keepcase_stems_match_only_as_written() {
        let aff = "SET UTF-8\nKEEPCASE k\nSFX a Y 1\nSFX a 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "3\npH/k\nkm/ka\nkot/a\n", &[]).unwrap();
        assert!(d.check("pH") && d.check("km") && d.check("kms"));
        assert!(!d.check("PH") && !d.check("Km") && !d.check("KM") && !d.check("Kms"));
        assert!(d.check("Kot") && d.check("KOTS"), "other stems still follow the sentence and heading case");
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.check("km") && !back.check("Km"));
    }

    #[test]
    fn fields_split_on_spaces_and_tabs_only() {
        // A flag byte read as U+00A0 (no-break space) or U+0085 is a flag, not a separator.
        let aff = "SET UTF-8\nSFX \u{a0} Y 1\nSFX \u{a0} 0 y .\nSFX \u{85} Y 1\nSFX \u{85} 0 em .\n";
        let d = AffixDict::from_hunspell(aff, "1\nkot/\u{a0}\u{85}\n", &[]).unwrap();
        assert!(d.contains("koty") && d.contains("kotem"));
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.contains("koty") && back.contains("kotem"));
    }

    #[test]
    fn unsupported_and_broken_input_is_an_error() {
        assert!(AffixDict::from_hunspell("FLAG long\n", DIC, &[]).is_err());
        assert!(AffixDict::from_hunspell("FLAG num\n", DIC, &[]).is_err());
        assert!(AffixDict::from_hunspell("CIRCUMFIX X\n", DIC, &[]).is_err());
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
