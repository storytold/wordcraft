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
//! Supported: one-character flags (any Unicode character — `FLAG UTF-8`), two-character
//! (`FLAG long`) and numeric (`FLAG num`) flags, at most [`MAX_FLAGS`] distinct; `PFX`/`SFX`
//! with strip, append and condition (`.`, letters, `[abc]`, `[^abc]`), cross products,
//! continuation classes on suffixes (at most two suffixes in a row, as in Hunspell; a prefix
//! flag in a suffix's continuation lets that prefix attach to the suffixed form),
//! `FORBIDDENWORD`, `NEEDAFFIX`, `KEEPCASE`, `FULLSTRIP`, `ICONV`, `NOSUGGEST` (for
//! [`AffixDict::can_suggest`]) and the suggestion tables `TRY`, `KEY`, `MAP` and `REP`. Ignored (they
//! only shape output or compounding hints): `OCONV`, `BREAK`, morphological fields. Rejected: compounding, `AF`/`AM` aliases,
//! `CIRCUMFIX` used by a rule, and the other directives that change which words are correct.
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

/// Directives that change which words are correct in ways this engine doesn't implement.
const UNSUPPORTED: &[&str] = &[
    "AF",
    "AM",
    "CHECKSHARPS",
    "COMPLEXPREFIXES",
    "COMPOUNDBEGIN",
    "COMPOUNDEND",
    "COMPOUNDFLAG",
    "COMPOUNDMIDDLE",
    "COMPOUNDMIN",
    "COMPOUNDPERMITFLAG",
    "COMPOUNDRULE",
    "IGNORE",
    "LEMMA_PRESENT",
    "ONLYINCOMPOUND",
    "PSEUDOROOT",
    "SUBSTANDARD",
];

/// Most distinct flags a dictionary may use (the Dicollecte French dictionary uses about 180).
pub const MAX_FLAGS: usize = 512;

/// A set of flags, one bit per flag index (see [`AffixDict::flag_chars`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
struct FlagSet([u64; MAX_FLAGS / 64]);

impl FlagSet {
    fn single(i: usize) -> FlagSet {
        let mut s = FlagSet::default();
        if let Some(w) = s.0.get_mut(i / 64) {
            *w |= 1 << (i % 64);
        }
        s
    }

    fn has(self, i: usize) -> bool {
        self.0.get(i / 64).is_some_and(|w| w & (1 << (i % 64)) != 0)
    }

    fn is_empty(self) -> bool {
        self.0.iter().all(|&w| w == 0)
    }

    fn union(self, o: FlagSet) -> FlagSet {
        let mut out = self;
        for (a, b) in out.0.iter_mut().zip(o.0) {
            *a |= b;
        }
        out
    }

    fn intersects(self, o: FlagSet) -> bool {
        self.0.iter().zip(o.0).any(|(a, b)| a & b != 0)
    }

    fn contains_all(self, o: FlagSet) -> bool {
        self.0.iter().zip(o.0).all(|(a, b)| a & b == b)
    }

    fn without(self, o: FlagSet) -> FlagSet {
        let mut out = self;
        for (a, b) in out.0.iter_mut().zip(o.0) {
            *a &= !b;
        }
        out
    }
}

/// How an `.aff`/`.dic` pair writes flags (`FLAG` directive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlagMode {
    /// One character per flag (the default, `FLAG UTF-8`).
    Char,
    /// Two characters per flag (`FLAG long`).
    Long,
    /// Decimal numbers separated by commas (`FLAG num`).
    Num,
}

/// Reads flag fields. Long and numeric flags become one private-use character each, so the rest
/// of the engine (and the compact format) deals in one-character flags only.
struct FlagReader {
    mode: FlagMode,
    names: HashMap<String, char>,
}

impl FlagReader {
    fn new() -> FlagReader {
        FlagReader { mode: FlagMode::Char, names: HashMap::new() }
    }

    fn intern(&mut self, name: &str) -> Result<char, String> {
        if let Some(&c) = self.names.get(name) {
            return Ok(c);
        }
        // Supplementary Private Use Area-A: never a letter of a real flag field.
        let c = u32::try_from(self.names.len()).ok().and_then(|n| char::from_u32(0xF0000 + n)).filter(|_| self.names.len() < MAX_FLAGS);
        let c = c.ok_or_else(|| format!("more than {MAX_FLAGS} distinct flags"))?;
        self.names.insert(name.to_string(), c);
        Ok(c)
    }

    /// The flags of a field (`ab`, `S.()`, `12,7`), as one character each.
    fn read(&mut self, field: &str) -> Result<String, String> {
        match self.mode {
            FlagMode::Char => Ok(field.to_string()),
            FlagMode::Long => {
                let chars: Vec<char> = field.chars().collect();
                if !chars.len().is_multiple_of(2) {
                    return Err(format!("long flags `{field}` have an odd length"));
                }
                chars.chunks(2).map(|p| self.intern(&p.iter().collect::<String>())).collect()
            }
            FlagMode::Num => field
                .split(',')
                .filter(|n| !n.is_empty())
                .map(|n| match n.parse::<u16>() {
                    Ok(_) => self.intern(n),
                    Err(_) => Err(format!("numeric flag `{n}` is not a number")),
                })
                .collect(),
        }
    }

    /// Exactly one flag.
    fn one(&mut self, field: &str) -> Result<char, String> {
        let flags = self.read(field)?;
        let mut it = flags.chars();
        match (it.next(), it.next()) {
            (Some(c), None) if !c.is_whitespace() => Ok(c),
            _ => Err(format!("`{field}` is not one flag")),
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
    bit: FlagSet,
    flag: char,
    cross: bool,
    strip: Box<str>,
    add: Box<str>,
    /// Continuation classes (`SFX A 0 able/B`): flags the derived form carries, licensing
    /// further affixes. Chained application is bounded by [`MAX_CHAIN`].
    cont: FlagSet,
    /// The continuation holds `NEEDAFFIX`: the derived form is a word only with another affix.
    needs_affix: bool,
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
    sets: Vec<FlagSet>,
    /// The `FORBIDDENWORD` flag's bit; stems carrying it are not words (empty: none declared).
    forbidden: FlagSet,
    /// The `NEEDAFFIX` flag's bit: a stem carrying it is a word only with an affix.
    need_affix: FlagSet,
    /// The `KEEPCASE` flag's bit: a stem carrying it is a word only in its own case.
    keep_case: FlagSet,
    /// The `NOSUGGEST` flag's bit: a stem carrying it (and its forms) is correct but never
    /// suggested.
    no_suggest: FlagSet,
    /// `FULLSTRIP`: a rule may strip the whole stem (`être` → `suis`).
    full_strip: bool,
    /// `ICONV` input conversions, applied before lookup (`’` → `'`).
    iconv: Vec<(String, String)>,
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
    /// The `FORBIDDENWORD` flag: stems carrying it are listed but never correct.
    forbidden: Option<char>,
    need_affix: Option<char>,
    keep_case: Option<char>,
    no_suggest: Option<char>,
    full_strip: bool,
    iconv: Vec<(String, String)>,
    /// The `CIRCUMFIX` flag, refused when a rule or stem uses it (not implemented).
    circumfix: Option<char>,
}

/// A rule as parsed from text, before flags become bits: `(flag, cross, strip, add, cont, cond)`.
type RawRule = (char, bool, String, String, String, String);

/// `0` means empty in Hunspell's strip and append fields.
fn zero_empty(s: &str) -> String {
    if s == "0" { String::new() } else { s.to_string() }
}

/// Read Hunspell `.aff` text (already decoded to UTF-8). The returned reader decodes the
/// `.dic` flag fields the same way.
fn parse_aff(aff: &str) -> Result<(Rules, FlagReader), String> {
    let mut r = Rules::default();
    let mut flags = FlagReader::new();
    let mut cross: HashMap<(bool, char), bool> = HashMap::new();
    let mut map_header = false;
    let mut rep_header = false;
    let mut iconv_header = false;
    for (n, line) in aff.lines().enumerate() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let Some(&dir) = f.first() else { continue };
        if dir.starts_with('#') {
            continue;
        }
        let at = |k: usize| f.get(k).copied().unwrap_or("");
        let line_err = |e: String| format!("line {}: {e}", n + 1);
        match dir {
            "SET" | "LANG" | "NAME" | "VERSION" | "HOME" | "WORDCHARS" | "MAXNGRAMSUGS" | "MAXDIFF" | "ONLYMAXDIFF" | "NOSPLITSUGS"
            | "SUGSWITHDOTS" | "WARN" | "FORBIDWARN" | "MAXCPDSUGS" | "OCONV" | "BREAK" => {}
            "FLAG" => {
                flags.mode = match at(1) {
                    "UTF-8" | "DEFAULT" | "utf-8" | "default" | "UTF8" | "" => FlagMode::Char,
                    "long" | "LONG" => FlagMode::Long,
                    "num" | "NUM" => FlagMode::Num,
                    other => return Err(line_err(format!("FLAG {other} is not supported"))),
                }
            }
            "FORBIDDENWORD" => r.forbidden = Some(flags.one(at(1)).map_err(line_err)?),
            "NEEDAFFIX" => r.need_affix = Some(flags.one(at(1)).map_err(line_err)?),
            "KEEPCASE" => r.keep_case = Some(flags.one(at(1)).map_err(line_err)?),
            "NOSUGGEST" => r.no_suggest = Some(flags.one(at(1)).map_err(line_err)?),
            "CIRCUMFIX" => r.circumfix = Some(flags.one(at(1)).map_err(line_err)?),
            "FULLSTRIP" => r.full_strip = true,
            "ICONV" => {
                if !iconv_header && f.len() == 2 && at(1).parse::<usize>().is_ok() {
                    iconv_header = true;
                } else if f.len() >= 3 && !at(1).is_empty() {
                    r.iconv.push((at(1).replace('_', " "), at(2).replace('_', " ")));
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
                let flag = flags.one(at(1)).map_err(line_err)?;
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
                let cont = flags.read(cont).map_err(line_err)?;
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
    Ok((r, flags))
}

impl AffixDict {
    /// Build from Hunspell `.aff` and `.dic` text (decoded to UTF-8; the `.dic` count line and
    /// morphological fields are ignored). `comments` (provenance, licence) are kept with the data.
    pub fn from_hunspell(aff: &str, dic: &str, comments: &[&str]) -> Result<AffixDict, String> {
        // Some dictionaries open with a UTF-8 BOM; it must not glue itself to the first
        // directive or turn the `.dic` count line into a stem.
        let aff = aff.strip_prefix('\u{feff}').unwrap_or(aff);
        let dic = dic.strip_prefix('\u{feff}').unwrap_or(dic);
        let (rules, mut reader) = parse_aff(aff)?;
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
            let flags = reader.read(&flags).map_err(|e| format!(".dic line {}: {e}", n + 1))?;
            stems.push((word, flags));
        }
        AffixDict::build(rules, stems, comments.iter().map(|s| s.to_string()).collect())
    }

    fn build(rules: Rules, mut stems: Vec<(String, String)>, comments: Vec<String>) -> Result<AffixDict, String> {
        if let Some(c) = rules.circumfix
            && (rules.prefixes.iter().chain(&rules.suffixes).any(|r| r.4.contains(c)) || stems.iter().any(|s| s.1.contains(c)))
        {
            return Err("CIRCUMFIX is not supported".into());
        }
        // Flags: every one a rule, a stem or a directive uses gets a bit.
        let mut flag_chars: Vec<char> = rules.prefixes.iter().chain(&rules.suffixes).map(|r| r.0).collect();
        flag_chars.extend(rules.prefixes.iter().chain(&rules.suffixes).flat_map(|r| r.4.chars()));
        flag_chars.extend(stems.iter().flat_map(|s| s.1.chars()));
        flag_chars.extend([rules.forbidden, rules.need_affix, rules.keep_case, rules.no_suggest].into_iter().flatten());
        flag_chars.sort_unstable();
        flag_chars.dedup();
        if flag_chars.len() > MAX_FLAGS {
            return Err(format!("{} distinct flags; at most {MAX_FLAGS} are supported", flag_chars.len()));
        }
        let bit_of = |c: char| flag_chars.iter().position(|&x| x == c).map_or(FlagSet::default(), FlagSet::single);
        let set_of = |s: &str| s.chars().fold(FlagSet::default(), |m, c| m.union(bit_of(c)));
        let directive = |c: Option<char>| c.map_or(FlagSet::default(), bit_of);
        let (forbidden, need_affix, keep_case) = (directive(rules.forbidden), directive(rules.need_affix), directive(rules.keep_case));
        let no_suggest = directive(rules.no_suggest);
        // Stems sorted by bytes; homonyms (the same word twice) are merged.
        stems.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut d = AffixDict {
            words: String::with_capacity(stems.iter().map(|s| s.0.len()).sum()),
            offs: Vec::with_capacity(stems.len() + 1),
            flag_chars: flag_chars.clone(),
            forbidden,
            need_affix,
            keep_case,
            no_suggest,
            full_strip: rules.full_strip,
            iconv: rules.iconv,
            try_chars: rules.try_chars,
            key: rules.key,
            map: rules.map,
            rep: rules.rep,
            comments,
            ..AffixDict::default()
        };
        d.offs.push(0);
        let mut set_ids: HashMap<FlagSet, u16> = HashMap::new();
        let mut i = 0;
        while i < stems.len() {
            let Some((word, _)) = stems.get(i) else { break };
            let mut mask = FlagSet::default();
            let mut every_no_suggest = true;
            let mut j = i;
            while let Some((w, f)) = stems.get(j)
                && w == word
            {
                let entry = set_of(f);
                every_no_suggest &= entry.intersects(no_suggest);
                mask = mask.union(entry);
                j += 1;
            }
            // `bar` the unit is NOSUGGEST, `bar` the noun isn't: the word stays suggestible unless
            // every entry spelled like it is marked.
            if !every_no_suggest {
                mask = mask.without(no_suggest);
            }
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
            let cont = set_of(&cont);
            let needs_affix = cont.intersects(need_affix);
            let (strip, add, cond_text) = (strip.into(), add.into(), cond.as_str().into());
            Ok(Rule { bit: bit_of(flag), flag, cross, strip, add, cont, needs_affix, cond: parse_cond(&cond)?, cond_text })
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
    fn flag_chars_for(&self, mask: FlagSet) -> String {
        self.flag_chars.iter().enumerate().filter(|&(i, _)| mask.has(i)).map(|(_, c)| c).collect()
    }

    /// The character of `mask` when it holds exactly one flag.
    fn flag_char(&self, mask: FlagSet) -> Option<char> {
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

    /// Flag mask of a stem, if `w` is one and carries none of the flags in `reject`.
    fn stem_flags(&self, w: &str, reject: FlagSet) -> Option<FlagSet> {
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
                let mask = self.sets.get(usize::from(id)).copied();
                // A stem carrying FORBIDDENWORD (always in `reject`) is listed but is not a
                // word: no direct hit, and (this is the only stem lookup) no affixed form either.
                return mask.filter(|m| !m.intersects(reject));
            }
            slot = (slot + 1) & mask;
        }
        None
    }

    /// The split points of `w` into a base and an appended suffix, from the end: the suffix is at
    /// most the longest one, and something stays before it unless `FULLSTRIP` allows a rule to
    /// replace the whole stem.
    fn cuts<'a>(&self, w: &'a str) -> impl Iterator<Item = (&'a str, &'a str)> + use<'a> {
        let min = usize::from(!self.full_strip);
        std::iter::once(w.len())
            .chain(w.char_indices().rev().map(|(i, _)| i).filter(move |&i| i >= min))
            .take(self.max_sfx_chars + 1)
            .filter_map(|b| Some((w.get(..b)?, w.get(b..)?)))
    }

    /// Does `w` carry flag `bit` — as a stem, or as the form of a suffix rule whose
    /// continuation classes include `bit` (chained suffixation, depth-bounded)?
    fn form_has_flag(&self, w: &str, bit: FlagSet, depth: u8, reject: FlagSet) -> bool {
        if self.stem_flags(w, reject).is_some_and(|f| f.intersects(bit)) {
            return true;
        }
        if depth == 0 {
            return false;
        }
        let mut root = String::with_capacity(w.len() + 8);
        for (base, add) in self.cuts(w) {
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups {
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                if root.is_empty() {
                    continue;
                }
                for r in g.rules.iter().filter_map(|&i| self.suffixes.get(i as usize)) {
                    if r.cont.intersects(bit) && r.cond_matches(&root, true) && self.form_has_flag(&root, r.bit, depth - 1, reject) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Is `w` exactly (case and all) a stem or an affixed form of one?
    pub fn contains(&self, w: &str) -> bool {
        self.contains_with(w, self.forbidden)
    }

    /// May `w` be offered as a suggestion: [`AffixDict::contains`], leaving out the stems marked
    /// `NOSUGGEST` and their forms (currency codes, unit symbols, interjections)?
    pub fn contains_suggestible(&self, w: &str) -> bool {
        self.contains_with(w, self.forbidden.union(self.no_suggest))
    }

    /// [`AffixDict::contains`], with stems carrying a flag of `reject` left out.
    fn contains_with(&self, w: &str, reject: FlagSet) -> bool {
        if w.is_empty() || w.len() > MAX_WORD_BYTES {
            return false;
        }
        if self.stem_flags(w, reject).is_some_and(|f| !f.intersects(self.need_affix)) || self.suffixed(w, FlagSet::default(), reject) {
            return true;
        }
        let mut root = String::with_capacity(w.len() + 8);
        for r in &self.prefixes {
            let Some(rest) = w.strip_prefix(&*r.add) else { continue };
            if rest.is_empty() && !self.full_strip {
                continue;
            }
            root.clear();
            root.push_str(&r.strip);
            root.push_str(rest);
            if root.is_empty() || !r.cond_matches(&root, false) {
                continue;
            }
            // A prefix whose continuation holds NEEDAFFIX needs a suffix too: the cross product.
            if !r.needs_affix && self.form_has_flag(&root, r.bit, INNER_CHAIN, reject) {
                return true;
            }
            if r.cross && self.suffixed(&root, r.bit, reject) {
                return true;
            }
        }
        false
    }

    /// Is `w` a stem with one suffix? With `need` set (a prefix was taken off), the stem must
    /// carry those flags too and the suffix must allow cross products.
    fn suffixed(&self, w: &str, need: FlagSet, reject: FlagSet) -> bool {
        let mut root = String::with_capacity(w.len() + 8);
        let alone = need.is_empty();
        for (base, add) in self.cuts(w) {
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups {
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                if root.is_empty() {
                    continue;
                }
                // Without a prefix, the suffix is the only affix: one needing another won't do.
                let rules = || g.rules.iter().filter_map(|&i| self.suffixes.get(i as usize)).filter(|r| !(alone && r.needs_affix));
                // The stem path: `root` is a stem carrying the rule's flag (and the prefix's,
                // when a cross product is being checked).
                if let Some(f) = self.stem_flags(&root, reject)
                    && f.contains_all(need)
                    && rules().any(|r| f.intersects(r.bit) && (alone || r.cross) && r.cond_matches(&root, true))
                {
                    return true;
                }
                // Continuation classes: `root` need not be a stem carrying this rule's flag —
                // it may be a form that gained the flag from another rule's continuation.
                if alone && rules().any(|r| r.cond_matches(&root, true) && self.form_has_flag(&root, r.bit, INNER_CHAIN, reject)) {
                    return true;
                }
            }
        }
        false
    }

    /// `w` through the `ICONV` table (longest match first, left to right).
    fn input_conv<'a>(&self, w: &'a str) -> std::borrow::Cow<'a, str> {
        if self.iconv.is_empty() || !self.iconv.iter().any(|(from, _)| w.contains(from.as_str())) {
            return std::borrow::Cow::Borrowed(w);
        }
        let mut out = String::with_capacity(w.len());
        let mut rest = w;
        while let Some(c) = rest.chars().next() {
            let best = self.iconv.iter().filter(|(from, _)| !from.is_empty() && rest.starts_with(from.as_str())).max_by_key(|(from, _)| from.len());
            match best {
                Some((from, to)) => {
                    out.push_str(to);
                    rest = rest.get(from.len()..).unwrap_or_default();
                }
                None => {
                    out.push(c);
                    rest = rest.get(c.len_utf8()..).unwrap_or_default();
                }
            }
        }
        std::borrow::Cow::Owned(out)
    }

    /// Is `w` correct, Hunspell style: as written, or — for a capitalized word or one in capitals
    /// — in lower case (and capitalized), since a sentence may start with any word and headings
    /// may be in capitals. Lower-case input never matches a capitalized stem (`warszawa`), and a
    /// `KEEPCASE` stem only matches as written.
    pub fn check(&self, w: &str) -> bool {
        self.check_with(w, self.forbidden)
    }

    /// May `w` be offered as a suggestion: [`AffixDict::check`], leaving out the stems marked
    /// `NOSUGGEST` and their forms, in any case.
    pub fn can_suggest(&self, w: &str) -> bool {
        self.check_with(w, self.forbidden.union(self.no_suggest))
    }

    /// [`AffixDict::check`], with stems carrying a flag of `reject` left out.
    fn check_with(&self, w: &str, reject: FlagSet) -> bool {
        let w = self.input_conv(w);
        let w = w.as_ref();
        if self.contains_with(w, reject) {
            return true;
        }
        let reject = reject.union(self.keep_case);
        match casing(w) {
            Casing::Lower | Casing::Mixed => false,
            Casing::Initial => self.contains_with(&w.to_lowercase(), reject),
            Casing::Upper => {
                let lower = w.to_lowercase();
                self.contains_with(&capitalize(&lower), reject) || self.contains_with(&lower, reject)
            }
        }
    }

    /// Every form of the stem `stem` (itself included unless it needs an affix), unsorted,
    /// possibly with repeats.
    pub fn forms(&self, stem: &str) -> Vec<String> {
        let Some(f) = self.stem_flags(stem, self.forbidden) else { return Vec::new() };
        let mut out = Vec::new();
        if !f.intersects(self.need_affix) {
            out.push(stem.to_string());
        }
        let mut cross_sfx = Vec::new();
        // Suffix forms queued with the continuation classes they carry, for chained expansion.
        let mut chain: Vec<(String, FlagSet)> = Vec::new();
        for r in &self.suffixes {
            if !f.intersects(r.bit) || !r.cond_matches(stem, true) {
                continue;
            }
            let Some(base) = stem.strip_suffix(&*r.strip) else { continue };
            // Stripping the whole stem needs FULLSTRIP, as in the checker.
            let form = format!("{base}{}", r.add);
            if form.is_empty() || (base.is_empty() && !self.full_strip) {
                continue;
            }
            if r.cross {
                cross_sfx.push(form.clone());
            }
            if !r.cont.is_empty() {
                chain.push((form.clone(), r.cont));
            }
            if !r.needs_affix {
                out.push(form);
            }
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
                    if !flags.intersects(r.bit) || !r.cond_matches(&form, true) {
                        continue;
                    }
                    let Some(base) = form.strip_suffix(&*r.strip) else { continue };
                    let next = format!("{base}{}", r.add);
                    if next.is_empty() || (base.is_empty() && !self.full_strip) {
                        continue;
                    }
                    if !r.cont.is_empty() {
                        chain.push((next.clone(), r.cont));
                    }
                    if !r.needs_affix {
                        out.push(next);
                    }
                }
            }
        }
        for r in &self.prefixes {
            if !f.intersects(r.bit) || !r.cond_matches(stem, false) {
                continue;
            }
            let Some(rest) = stem.strip_prefix(&*r.strip) else { continue };
            if rest.is_empty() && !self.full_strip {
                continue;
            }
            if !r.needs_affix {
                out.push(format!("{}{rest}", r.add));
            }
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
        let directives =
            [("FORBIDDENWORD", self.forbidden), ("NEEDAFFIX", self.need_affix), ("KEEPCASE", self.keep_case), ("NOSUGGEST", self.no_suggest)];
        for (name, set) in directives {
            if let Some(c) = self.flag_char(set) {
                out.push_str(&format!("{name} {c}\n"));
            }
        }
        if self.full_strip {
            out.push_str("FULLSTRIP\n");
        }
        for (a, b) in &self.iconv {
            out.push_str(&format!("ICONV {} {}\n", a.replace(' ', "_"), b.replace(' ', "_")));
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
        for line in lines {
            if let Some(c) = line.strip_prefix("# ") {
                comments.push(c.to_string());
                continue;
            }
            let f: Vec<&str> = line.split_whitespace().collect();
            let at = |k: usize| f.get(k).copied().unwrap_or("");
            match at(0) {
                "TRY" => rules.try_chars = at(1).to_string(),
                "KEY" => rules.key = at(1).to_string(),
                "MAP" => rules.map.push(at(1).chars().collect()),
                "REP" => rules.rep.push((at(1).replace('_', " "), at(2).replace('_', " "))),
                name @ ("FORBIDDENWORD" | "NEEDAFFIX" | "KEEPCASE" | "NOSUGGEST") => {
                    let mut it = at(1).chars();
                    let c = match (it.next(), it.next()) {
                        (Some(c), None) if !c.is_whitespace() => c,
                        _ => return Err(format!("affix dictionary: bad {name} in `{line}`")),
                    };
                    match name {
                        "FORBIDDENWORD" => rules.forbidden = Some(c),
                        "NEEDAFFIX" => rules.need_affix = Some(c),
                        "KEEPCASE" => rules.keep_case = Some(c),
                        _ => rules.no_suggest = Some(c),
                    }
                }
                "FULLSTRIP" => rules.full_strip = true,
                "ICONV" => {
                    if !at(1).is_empty() {
                        rules.iconv.push((at(1).replace('_', " "), at(2).replace('_', " ")));
                    }
                }
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
        assert!(AffixDict::from_hunspell("FLAG long\n", "1\nkot/abc\n", &[]).is_err(), "odd-length long flags");
        assert!(AffixDict::from_hunspell("FLAG num\n", "1\nkot/1,x\n", &[]).is_err(), "a numeric flag that isn't a number");
        assert!(AffixDict::from_hunspell("FLAG ucs2\n", DIC, &[]).is_err());
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

    #[test]
    fn long_and_numeric_flags() {
        // `FLAG long` (the Dicollecte French dictionary): two characters per flag.
        let aff = "SET UTF-8\nFLAG long\nSFX S. Y 1\nSFX S. 0 s [^sxz]\nSFX Aa N 1\nSFX Aa 0 ette .\n";
        let d = AffixDict::from_hunspell(aff, "2\nchat/S.Aa\nchien/S.\n", &[]).unwrap();
        assert!(d.contains("chat") && d.contains("chats") && d.contains("chatette") && d.contains("chiens"));
        assert!(!d.contains("chienette"), "chien carries S. only");
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.contains("chats") && back.contains("chatette") && !back.contains("chienette"), "long flags survive a round trip");
        // `FLAG num`: comma-separated numbers.
        let aff = "SET UTF-8\nFLAG num\nSFX 101 Y 1\nSFX 101 0 s .\nSFX 7 N 1\nSFX 7 0 ette .\n";
        let d = AffixDict::from_hunspell(aff, "2\nchat/101,7\nchien/101\n", &[]).unwrap();
        assert!(d.contains("chats") && d.contains("chatette") && d.contains("chiens") && !d.contains("chienette"));
    }

    #[test]
    fn more_than_128_distinct_flags_load() {
        // The French dictionary uses about 180 long flags; 300 here, past the old 128-bit limit.
        let mut aff = String::from("SET UTF-8\nFLAG num\n");
        let mut flags = Vec::new();
        for n in 1..=300 {
            aff.push_str(&format!("SFX {n} N 1\nSFX {n} 0 x{n} .\n"));
            flags.push(n.to_string());
        }
        let d = AffixDict::from_hunspell(&aff, &format!("2\nmot/{}\nautre/300\n", flags.join(",")), &[]).unwrap();
        assert!(d.contains("motx1") && d.contains("motx200") && d.contains("motx300") && d.contains("autrex300"));
        assert!(!d.contains("autrex1") && !d.contains("motx301"));
        let mut too_many = String::from("SET UTF-8\nFLAG num\n");
        for n in 1..=MAX_FLAGS + 1 {
            too_many.push_str(&format!("SFX {n} N 1\nSFX {n} 0 x .\n"));
        }
        assert!(AffixDict::from_hunspell(&too_many, "0\n", &[]).is_err(), "past MAX_FLAGS is an error, not a panic");
    }

    #[test]
    fn need_affix_stems_and_affixes() {
        // French style: the stem needs an affix, and an empty suffix gives the singular.
        let aff = "SET UTF-8\nFLAG long\nNEEDAFFIX ()\nSFX S. Y 2\nSFX S. 0 0 [^sxz]\nSFX S. 0 s [^sxz]\nSFX Zz N 1\nSFX Zz 0 iste/()S. .\n";
        let d = AffixDict::from_hunspell(aff, "3\nAllemagne/S.()\nbleu/()\nart/Zz\n", &[]).unwrap();
        assert!(d.contains("Allemagne") && d.contains("Allemagnes"), "the empty suffix is an affix");
        assert!(!d.contains("bleu"), "a NEEDAFFIX stem with no affix is not a word");
        assert!(d.contains("art") && d.contains("artistes"), "the suffix needing an affix gets one");
        assert!(d.contains("artiste"), "artiste is artist + iste + the empty S. suffix");
        let aff = "SET UTF-8\nFLAG long\nNEEDAFFIX ()\nSFX Zz N 1\nSFX Zz 0 iste/() .\n";
        let d = AffixDict::from_hunspell(aff, "1\nart/Zz\n", &[]).unwrap();
        assert!(!d.contains("artiste"), "nothing follows the suffix that needs an affix");
        let mut forms = d.forms("art");
        forms.sort();
        assert_eq!(forms, ["art"]);
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.contains("art") && !back.contains("artiste"), "NEEDAFFIX survives a round trip");
    }

    #[test]
    fn prefix_flags_from_a_suffix_continuation() {
        // French elision: `SFX S. 0 0/L'` lets the prefix `l'` attach to the suffixed form.
        let aff = "SET UTF-8\nFLAG long\nNEEDAFFIX ()\nPFX L' Y 1\nPFX L' 0 l' [aeiouh]\nSFX S. Y 2\nSFX S. 0 0/L' [^sxz]\nSFX S. 0 s [^sxz]\n";
        let d = AffixDict::from_hunspell(aff, "2\narbre/S.()\nchat/S.()\n", &[]).unwrap();
        assert!(d.contains("arbre") && d.contains("arbres") && d.contains("l'arbre"));
        assert!(!d.contains("l'arbres"), "the plural suffix carries no L'");
        assert!(!d.contains("l'chat"), "the prefix condition still applies");
    }

    #[test]
    fn keep_case_stems_match_only_as_written() {
        let aff = "SET UTF-8\nFLAG long\nKEEPCASE ||\nSFX S. Y 1\nSFX S. 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "2\nkm/||\nchat/S.\n", &[]).unwrap();
        assert!(d.check("km") && !d.check("Km") && !d.check("KM"));
        assert!(d.check("Chats") && d.check("CHATS"), "other stems keep the case variants");
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.check("km") && !back.check("KM"), "KEEPCASE survives a round trip");
    }

    #[test]
    fn full_strip_replaces_the_whole_stem() {
        let aff = "SET UTF-8\nFLAG long\nFULLSTRIP\nSFX Et N 2\nSFX Et être suis être\nSFX Et être es être\n";
        let d = AffixDict::from_hunspell(aff, "1\nêtre/Et\n", &[]).unwrap();
        assert!(d.contains("être") && d.contains("suis") && d.contains("es"));
        assert!(!d.contains("suiss"));
        let mut forms = d.forms("être");
        forms.sort();
        assert_eq!(forms, ["es", "suis", "être"]);
        assert!(AffixDict::from_bytes(&d.to_bytes()).unwrap().contains("suis"), "FULLSTRIP survives a round trip");
        let without = AffixDict::from_hunspell(&aff.replace("FULLSTRIP\n", ""), "1\nêtre/Et\n", &[]).unwrap();
        assert!(!without.contains("suis"), "without FULLSTRIP a rule can't strip the whole stem");
        assert_eq!(without.forms("être"), ["être"], "nor list such a form");
    }

    #[test]
    fn iconv_converts_input_before_lookup() {
        let aff = "SET UTF-8\nICONV 2\nICONV ’ '\nICONV ﬁ fi\n";
        let d = AffixDict::from_hunspell(aff, "2\naujourd'hui\nfin\n", &[]).unwrap();
        assert!(d.check("aujourd’hui") && d.check("aujourd'hui") && d.check("ﬁn") && d.check("Aujourd’hui"));
        assert!(!d.check("aujourd’huy"));
        assert!(AffixDict::from_bytes(&d.to_bytes()).unwrap().check("aujourd’hui"), "ICONV survives a round trip");
        // An ICONV line without operands in compact data must not loop forever.
        let empty = miniz_oxide::deflate::compress_to_vec(b"WCAFF1\nICONV\nWORDS\n\x00fin\n", 6);
        let d = AffixDict::from_bytes(&empty).unwrap();
        assert!(d.check("fin") && !d.check("a"));
    }

    #[test]
    fn circumfix_is_refused_only_when_used() {
        let aff = "SET UTF-8\nFLAG long\nCIRCUMFIX **\nSFX S. Y 1\nSFX S. 0 s .\n";
        assert!(AffixDict::from_hunspell(aff, "1\nchat/S.\n", &[]).is_ok(), "declared but unused: harmless");
        assert!(AffixDict::from_hunspell(aff, "1\nchat/S.**\n", &[]).is_err(), "used: not implemented, so refused");
    }

    #[test]
    fn no_suggest_words_are_correct_but_not_suggestible() {
        // Dicollecte marks currency codes, unit symbols and interjections NOSUGGEST.
        let aff = "SET UTF-8\nFLAG long\nNOSUGGEST --\nSFX S. Y 1\nSFX S. 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "2\nchat/S.\nchf/S.--\n", &[]).unwrap();
        assert!(d.contains("chf") && d.contains("chfs") && d.check("chf"), "still correct");
        assert!(!d.contains_suggestible("chf") && !d.contains_suggestible("chfs"), "never suggested, nor its forms");
        assert!(d.contains_suggestible("chat") && d.contains_suggestible("chats"));
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.contains("chf") && !back.contains_suggestible("chf"), "NOSUGGEST survives a round trip");
    }

    #[test]
    fn no_suggest_on_one_homonym_keeps_the_word_suggestible() {
        // `bar` the unit is NOSUGGEST and KEEPCASE, `bar` the noun is an ordinary word.
        let aff = "SET UTF-8\nFLAG long\nNOSUGGEST --\nKEEPCASE ||\nSFX S. Y 1\nSFX S. 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "3\nbar/S.\nbar/||--\ncgr/||--\n", &[]).unwrap();
        assert!(d.contains_suggestible("bar") && d.contains_suggestible("bars"), "the noun stays suggestible");
        assert!(!d.contains_suggestible("cgr") && d.contains("cgr"), "a symbol with one entry doesn't");
        let back = AffixDict::from_bytes(&d.to_bytes()).unwrap();
        assert!(back.contains_suggestible("bars") && !back.contains_suggestible("cgr"), "both survive a round trip");
    }

    /// `.aff` text built from random pieces with the features the French dictionary uses.
    fn random_aff(mode: u8, directives: &[String], rules: &[(String, String, String, String, String)]) -> String {
        let mode = ["long", "num", "UTF-8"].get(usize::from(mode % 3)).copied().unwrap_or("long");
        let mut aff = format!("SET UTF-8\nFLAG {mode}\nFULLSTRIP\nICONV 2\nICONV ’ '\n");
        for d in directives {
            aff.push_str(d);
            aff.push('\n');
        }
        for (flag, strip, add, cont, cond) in rules {
            let kind = if strip.len() % 2 == 0 { "SFX" } else { "PFX" };
            aff.push_str(&format!("{kind} {flag} Y 1\n{kind} {flag} {strip} {add}/{cont} {cond}\n"));
        }
        aff
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
        fn random_french_style_dictionaries_never_panic(
            mode in 0u8..3,
            directives in proptest::collection::vec("(NEEDAFFIX|KEEPCASE|NOSUGGEST|FORBIDDENWORD|ICONV|CIRCUMFIX) [a-c0-9(),.'’]{0,3}", 0..4),
            rules in proptest::collection::vec(("[A-Ca-c0-9().']{1,2}", "[a-cé0]{1,3}", "[a-cé0']{1,3}", "[A-Ca-c0-9(),.']{0,4}", "[a-cé.\\[\\]^]{1,4}"), 0..8),
            stems in proptest::collection::vec(("[a-cé'’-]{1,6}", "[A-Ca-c0-9(),.']{0,6}"), 0..8),
            word in "[a-cé'’ -]{0,10}",
        ) {
            let aff = random_aff(mode, &directives, &rules);
            let dic: String = std::iter::once(format!("{}\n", stems.len())).chain(stems.iter().map(|(w, f)| format!("{w}/{f}\n"))).collect();
            if let Ok(d) = AffixDict::from_hunspell(&aff, &dic, &[]) {
                let _ = d.check(&word);
                let _ = d.contains_suggestible(&word);
                for stem in d.stems().map(str::to_string).collect::<Vec<_>>() {
                    let _ = d.forms(&stem);
                }
                let _ = crate::fr::suggest_in(&d, &word, 3);
                let back = AffixDict::from_bytes(&d.to_bytes());
                proptest::prop_assert!(back.is_ok(), "{aff}\n{dic}");
            }
        }
    }
}
