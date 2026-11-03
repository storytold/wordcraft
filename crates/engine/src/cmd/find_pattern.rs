//! Word's Find and Replace patterns (#543): special characters (`^p`, `^t`, `^#` …) and the
//! "Use wildcards" language (`?`, `*`, `[a-z]`, `{n,m}`, `@`, `<`, `>`, `( )`), translated to a
//! regular expression for the linear-time `regex` engine, so no pattern can backtrack
//! catastrophically. Replace text understands the same codes plus `^&` (the found text), `^c`
//! (the clipboard) and, with wildcards, `\1`–`\9` (the groups).
//!
//! The search reads a story as text with a paragraph mark (`\r`, [`PARA_MARK`]) between
//! paragraphs when the pattern can match one, so `^p` finds paragraph ends; a paragraph that ends
//! a section ends with [`SECT_MARK`] instead (`^b`).

use regex::escape;

/// A paragraph mark in the searched text.
pub const PARA_MARK: char = '\r';
/// The mark of a paragraph that ends a section (`^b`), in the searched text only.
pub const SECT_MARK: char = '\u{1D}';
/// Longest Find what text, in characters.
pub const MAX_PATTERN: usize = 1024;
/// Largest count in `{n,m}`.
const MAX_COUNT: u32 = 1000;
/// Any character except a paragraph mark (`?`, `^?`, `*`).
const ANY: &str = r"[^\r\x{1D}]";

/// How the Find what text is read.
#[derive(Clone, Copy, Debug, Default)]
pub struct Opts {
    pub match_case: bool,
    pub whole_word: bool,
    pub prefix: bool,
    pub suffix: bool,
    pub ignore_punct: bool,
    pub ignore_space: bool,
    /// Word's "Use wildcards".
    pub wildcards: bool,
    /// A regular expression (WordCraft's own option; wins over wildcards).
    pub regex: bool,
    /// Take the text literally: no `^` codes.
    pub literal: bool,
}

/// A pattern ready for the regex engine.
#[derive(Debug)]
pub struct Compiled {
    pub pattern: String,
    pub case_insensitive: bool,
    /// The pattern can match a paragraph mark, so the search must read across paragraphs.
    pub spans_paras: bool,
}

/// What a `^` code stands for in Find what.
fn find_code(c: char) -> Option<&'static str> {
    Some(match c {
        'p' => r"\r",
        'b' => r"\x{1D}",
        't' => r"\t",
        '?' => ANY,
        '#' => "[0-9]",
        '$' => r"\p{L}",
        '^' => r"\^",
        'l' => r"\n",
        'm' => r"\x{0C}",
        'n' => r"\x{0E}",
        '+' => r"\x{2014}",
        '=' => r"\x{2013}",
        's' => r"\x{A0}",
        '~' => r"\x{2011}",
        '-' => r"\x{AD}",
        'w' => r"[ \t\x{A0}]+",
        'g' => r"\x{FFFC}",
        '%' => "§",
        'v' => "¶",
        _ => return None,
    })
}

/// What a `^` code stands for in Replace with.
fn replace_code(c: char) -> Option<char> {
    Some(match c {
        'p' => PARA_MARK,
        't' => '\t',
        '^' => '^',
        'l' => '\n',
        'm' => '\u{0C}',
        'n' => '\u{0E}',
        '+' => '\u{2014}',
        '=' => '\u{2013}',
        's' => '\u{A0}',
        '~' => '\u{2011}',
        '-' => '\u{AD}',
        '%' => '§',
        'v' => '¶',
        _ => return None,
    })
}

/// A character code (`^13`, `^0151`, `^u8212`) as the character it stands for here.
fn code_char(n: u32) -> Option<char> {
    Some(match n {
        13 => PARA_MARK,
        11 => '\n',
        12 => '\u{0C}',
        14 => '\u{0E}',
        30 => '\u{2011}',
        31 => '\u{AD}',
        n => char::from_u32(n).filter(|c| !c.is_control() || *c == '\t')?,
    })
}

/// A Windows-1252 code as its Unicode scalar: 128–159 hold typographic characters there (151 is
/// an em dash); the five undefined slots and every other code stay as they are.
fn ansi(n: u32) -> u32 {
    const HIGH: [u32; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018,
        0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    n.checked_sub(128).and_then(|k| HIGH.get(k as usize)).copied().unwrap_or(n)
}

/// Read a numeric code after `^` (`13`, `0151`, `u8212`) starting at `chars[i]`; returns the
/// character and how many characters it took.
fn numeric_code(chars: &[char], i: usize) -> Option<(char, usize)> {
    let (start, max) = if chars.get(i) == Some(&'u') { (i + 1, 5) } else { (i, 4) };
    let digits: String = chars.get(start..).unwrap_or(&[]).iter().take(max).take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let n: u32 = digits.parse().ok()?;
    // `^0nnn` is a Windows-1252 (ANSI) code; `^nnn` and `^unnnn` are Unicode.
    let n = if start == i && digits.starts_with('0') { ansi(n) } else { n };
    Some((code_char(n)?, start - i + digits.chars().count()))
}

/// A literal character as a regex atom.
fn lit(c: char) -> String {
    if c == PARA_MARK {
        return r"\r".into();
    }
    escape(c.encode_utf8(&mut [0; 4]))
}

/// Translate Word's Find what text.
pub fn compile(query: &str, o: &Opts) -> Result<Compiled, String> {
    if query.chars().count() > MAX_PATTERN {
        return Err(format!("the Find what text is longer than {MAX_PATTERN} characters"));
    }
    let (pat, spans) = if o.regex {
        (query.to_string(), false)
    } else if o.literal {
        (escape(query), false)
    } else if o.wildcards {
        wildcards(query)?
    } else {
        special(query, o)
    };
    let wrap = |p: String| format!("(?:{p})");
    let pat = if o.wildcards && !o.regex {
        pat
    } else if o.whole_word {
        format!(r"\b{}\b", wrap(pat))
    } else {
        match (o.prefix, o.suffix) {
            (true, true) => format!(r"\b{}\b", wrap(pat)),
            (true, false) => format!(r"\b{}", wrap(pat)),
            (false, true) => format!(r"{}\b", wrap(pat)),
            (false, false) => pat,
        }
    };
    // Word's wildcard search always matches case.
    let case_insensitive = !o.match_case && (!o.wildcards || o.regex);
    Ok(Compiled { pattern: pat, case_insensitive, spans_paras: spans })
}

/// Normal search: literal text with `^` codes, optionally skipping punctuation and white space.
fn special(query: &str, o: &Opts) -> (String, bool) {
    let chars: Vec<char> = query.chars().collect();
    let mut atoms: Vec<String> = Vec::new();
    let mut spans = false;
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        i += 1;
        if c == '^'
            && let Some(&n) = chars.get(i)
        {
            if let Some(code) = find_code(n) {
                spans |= matches!(n, 'p' | 'b');
                atoms.push(code.to_string());
                i += 1;
                continue;
            }
            if let Some((ch, used)) = numeric_code(&chars, i) {
                spans |= ch == PARA_MARK;
                atoms.push(lit(ch));
                i += used;
                continue;
            }
            // Not a code: the caret itself.
        }
        if (o.ignore_punct && is_punct(c)) || (o.ignore_space && is_space(c)) {
            continue;
        }
        atoms.push(lit(c));
    }
    let filler = match (o.ignore_punct, o.ignore_space) {
        (true, true) => r"[\p{P} \t\x{A0}]*",
        (true, false) => r"\p{P}*",
        (false, true) => r"[ \t\x{A0}]*",
        (false, false) => "",
    };
    (atoms.join(filler), spans)
}

fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation() || matches!(c, '\u{2010}'..='\u{2027}' | '\u{00A1}' | '\u{00BF}' | '\u{00AB}' | '\u{00BB}')
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\u{A0}')
}

/// "Use wildcards" search.
fn wildcards(query: &str) -> Result<(String, bool), String> {
    let chars: Vec<char> = query.chars().collect();
    let mut out = String::new();
    let mut spans = false;
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        i += 1;
        match c {
            '?' => out.push_str(ANY),
            '*' => {
                out.push_str(ANY);
                out.push_str("*?");
            }
            '@' => out.push('+'),
            '<' => out.push_str(r"\b{start}"),
            '>' => out.push_str(r"\b{end}"),
            '(' | ')' => out.push(c),
            '{' => {
                let end = chars.get(i..).and_then(|r| r.iter().position(|c| *c == '}')).ok_or("a { needs a closing }")?;
                let body: String = chars.get(i..i + end).unwrap_or(&[]).iter().collect();
                out.push_str(&count(&body)?);
                i += end + 1;
            }
            '[' => {
                let end = chars.get(i..).and_then(|r| r.iter().position(|c| *c == ']')).ok_or("a [ needs a closing ]")?;
                out.push_str(&class(chars.get(i..i + end).unwrap_or(&[]))?);
                i += end + 1;
            }
            '\\' => match chars.get(i) {
                Some(d) if d.is_ascii_digit() => return Err("back-references (\\1) work in Replace with, not in Find what".into()),
                Some(&n) => {
                    out.push_str(&lit(n));
                    i += 1;
                }
                None => out.push_str(r"\\"),
            },
            '^' => {
                let Some(&n) = chars.get(i) else {
                    out.push_str(r"\^");
                    continue;
                };
                if let Some(code) = find_code(n) {
                    spans |= matches!(n, 'p' | 'b');
                    out.push_str(code);
                    i += 1;
                } else if let Some((ch, used)) = numeric_code(&chars, i) {
                    spans |= ch == PARA_MARK;
                    out.push_str(&lit(ch));
                    i += used;
                } else {
                    out.push_str(r"\^");
                }
            }
            c => out.push_str(&lit(c)),
        }
    }
    Ok((out, spans))
}

/// `{n}`, `{n,}`, `{n,m}` (and `{,m}`).
fn count(body: &str) -> Result<String, String> {
    let bad = || format!("{{{body}}} is not a valid count");
    let num = |s: &str| -> Result<Option<u32>, String> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(None);
        }
        let n: u32 = s.parse().map_err(|_| bad())?;
        if n > MAX_COUNT {
            return Err(format!("counts go up to {MAX_COUNT}"));
        }
        Ok(Some(n))
    };
    match body.split_once([',', ';']) {
        None => Ok(format!("{{{}}}", num(body)?.ok_or_else(bad)?)),
        Some((a, b)) => {
            let a = num(a)?.unwrap_or(0);
            match num(b)? {
                Some(b) if b < a => Err(bad()),
                Some(b) => Ok(format!("{{{a},{b}}}")),
                None => Ok(format!("{{{a},}}")),
            }
        }
    }
}

/// `[abc]`, `[a-z]`, `[!x]`: the characters inside are literal.
fn class(inner: &[char]) -> Result<String, String> {
    let (neg, inner) = match inner.split_first() {
        Some(('!', rest)) => (true, rest),
        _ => (false, inner),
    };
    if inner.is_empty() {
        return Err("[ ] needs at least one character".into());
    }
    let mut out = String::from(if neg { r"[^\r\x{1D}" } else { "[" });
    let mut i = 0;
    while let Some(&c) = inner.get(i) {
        let c = if c == '\\' && i + 1 < inner.len() {
            i += 1;
            inner.get(i).copied().unwrap_or(c)
        } else {
            c
        };
        i += 1;
        if inner.get(i) == Some(&'-')
            && let Some(&hi) = inner.get(i + 1)
        {
            if hi < c {
                return Err(format!("[{c}-{hi}] is not in ascending order"));
            }
            out.push_str(&format!("{}-{}", lit(c), lit(hi)));
            i += 2;
        } else {
            out.push_str(&lit(c));
        }
    }
    out.push(']');
    Ok(out)
}

/// A piece of Replace with text.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Lit(String),
    /// `^&`: the text found.
    Found,
    /// `\n` with wildcards, or `$n` / `${n}` / `\n` with a regex: group `n` (0 is the match).
    Group(usize),
    /// `${name}` / `$name` with a regex: a named group, resolved to a [`Piece::Group`] once the
    /// pattern is built ([`resolve_names`]); one left unresolved expands to nothing.
    Named(String),
    /// `^c`: the clipboard.
    Clip,
}

/// Read Replace with text: `^` codes (unless `literal`), `\1`–`\9` with wildcards, and with a
/// regex also `$n`, `${n}`, `${name}`, `$name`, `$$`, `\n` (paragraph mark), `\t` and `\\`.
/// Pushes the pending literal text, if any.
fn flush_literal(cur: &mut String, out: &mut Vec<Piece>) {
    if !cur.is_empty() {
        out.push(Piece::Lit(std::mem::take(cur)));
    }
}

/// The `^` code whose letter is `chars[i]` (`^&`, `^c`, a replace code or a numeric one): pushes
/// its piece and returns the characters it used, or `None` when it isn't a code.
fn caret_code(chars: &[char], i: usize, cur: &mut String, out: &mut Vec<Piece>) -> Option<usize> {
    let &n = chars.get(i)?;
    if n == '&' || n == 'c' {
        flush_literal(cur, out);
        out.push(if n == '&' { Piece::Found } else { Piece::Clip });
        return Some(1);
    }
    if let Some(ch) = replace_code(n) {
        cur.push(ch);
        return Some(1);
    }
    let (ch, used) = numeric_code(chars, i)?;
    cur.push(ch);
    Some(used)
}

pub fn parse_replace(with: &str, o: &Opts) -> Vec<Piece> {
    // With a regex, `special: false` turns off only the `^` codes; group references still expand.
    if o.literal && !o.regex {
        return vec![Piece::Lit(with.to_string())];
    }
    let chars: Vec<char> = with.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        i += 1;
        if o.regex && (c == '$' || c == '\\') {
            match regex_ref(&chars, i, c) {
                Some((Ref::Char(ch), used)) => {
                    cur.push(ch);
                    i += used;
                }
                Some((Ref::Group(pc), used)) => {
                    flush_literal(&mut cur, &mut out);
                    out.extend(pc);
                    i += used;
                }
                // Not a reference or an escape: the character as written.
                None => cur.push(c),
            }
            continue;
        }
        if c == '\\' && o.wildcards {
            match chars.get(i) {
                Some(d) if d.is_ascii_digit() && *d != '0' => {
                    flush_literal(&mut cur, &mut out);
                    out.push(Piece::Group(d.to_digit(10).unwrap_or(0) as usize));
                    i += 1;
                    continue;
                }
                Some(&n) => {
                    cur.push(n);
                    i += 1;
                    continue;
                }
                None => {}
            }
        }
        if c == '^'
            && !o.literal
            && let Some(used) = caret_code(&chars, i, &mut cur, &mut out)
        {
            i += used;
            continue;
        }
        cur.push(c);
    }
    flush_literal(&mut cur, &mut out);
    out
}

/// What a `$` or `\` starts in regex Replace with text.
enum Ref {
    /// An escape: `$$`, `\n`, `\t`, `\\`.
    Char(char),
    /// A group reference; `None` for a number too large to be a group (expands to nothing).
    Group(Option<Piece>),
}

/// A group named by its number or name, as the `regex` crate reads it.
fn group_ref(name: &str) -> Option<Piece> {
    if name.bytes().all(|b| b.is_ascii_digit()) { name.parse().ok().map(Piece::Group) } else { Some(Piece::Named(name.to_string())) }
}

/// Read a regex reference or escape after `c` (`$` or `\`) at `chars[i..]`: what it is and how
/// many characters it uses after `c`, or `None` when `c` is a plain character.
fn regex_ref(chars: &[char], i: usize, c: char) -> Option<(Ref, usize)> {
    let next = *chars.get(i)?;
    if c == '\\' {
        return match next {
            'n' => Some((Ref::Char(PARA_MARK), 1)),
            't' => Some((Ref::Char('\t'), 1)),
            // `\$` and `\^` keep a `$` or `^` as typed (no group, no `^` code).
            '\\' | '$' | '^' => Some((Ref::Char(next), 1)),
            '1'..='9' => Some((Ref::Group(next.to_digit(10).map(|d| Piece::Group(d as usize))), 1)),
            _ => None,
        };
    }
    if next == '$' {
        return Some((Ref::Char('$'), 1));
    }
    let rest = chars.get(i..).unwrap_or_default();
    if next == '{' {
        // `${…}`: everything up to the closing brace; unclosed or empty, the `$` is literal.
        let close = rest.iter().position(|&ch| ch == '}')?;
        let name: String = rest.get(1..close)?.iter().collect();
        if name.is_empty() {
            return None;
        }
        return Some((Ref::Group(group_ref(&name)), close + 1));
    }
    // `$name`: the longest run of letters, digits and `_`.
    let len = rest.iter().take_while(|ch| ch.is_ascii_alphanumeric() || **ch == '_').count();
    if len == 0 {
        return None;
    }
    let name: String = rest.get(..len)?.iter().collect();
    Some((Ref::Group(group_ref(&name)), len))
}

/// Turn named group references into numbered ones with `index` (a name's group number);
/// an unknown name is dropped, so it expands to nothing.
pub fn resolve_names(pieces: Vec<Piece>, index: &dyn Fn(&str) -> Option<usize>) -> Vec<Piece> {
    pieces
        .into_iter()
        .filter_map(|p| match p {
            Piece::Named(n) => index(&n).map(Piece::Group),
            p => Some(p),
        })
        .collect()
}

/// Searched text as it is inserted again: marks become paragraph breaks.
fn as_inserted(s: &str) -> String {
    s.replace(SECT_MARK, "\r")
}

/// Fill in Replace with text for one match: `found` is the matched text, `group(n)` a group's.
pub fn expand(pieces: &[Piece], found: &str, group: &dyn Fn(usize) -> Option<String>, clip: &str) -> String {
    let mut out = String::new();
    for p in pieces {
        match p {
            Piece::Lit(t) => out.push_str(t),
            Piece::Found => out.push_str(&as_inserted(found)),
            Piece::Group(n) => out.push_str(&as_inserted(&group(*n).unwrap_or_default())),
            Piece::Clip => out.push_str(&clip.replace("\r\n", "\r").replace('\n', "\r")),
            Piece::Named(_) => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finds(query: &str, hay: &str) -> bool {
        let c = compile(query, &Opts::default()).unwrap();
        regex::Regex::new(&c.pattern).unwrap().is_match(hay)
    }

    fn replaced(with: &str) -> String {
        expand(&parse_replace(with, &Opts::default()), "", &|_| None, "")
    }

    #[test]
    fn ansi_codes_128_to_159_are_windows_1252() {
        for (code, ch) in [("^0151", '\u{2014}'), ("^0150", '\u{2013}'), ("^0128", '\u{20AC}'), ("^0145", '\u{2018}')] {
            let hay = format!("a{ch}b");
            assert!(finds(code, &hay), "{code} should find {ch:?}");
            assert!(!finds(code, code), "{code} should not match its literal text");
            let w = Opts { wildcards: true, ..Opts::default() };
            let c = compile(code, &w).unwrap();
            assert!(regex::Regex::new(&c.pattern).unwrap().is_match(&hay), "wildcard {code} should find {ch:?}");
        }
        assert_eq!(replaced("x^0151y"), "x\u{2014}y");
        assert_eq!(replaced("^0128"), "\u{20AC}");
        // Unchanged: other codes and the undefined Windows-1252 slots.
        assert!(finds("^13", "a\rb"));
        assert!(finds("^0065", "A"));
        for undefined in ["^0129", "^0141", "^0143", "^0144", "^0157"] {
            assert_eq!(replaced(undefined), undefined);
        }
        // Without the leading zero the code is not read as Windows-1252.
        assert!(!finds("^151", "\u{2014}"));
        assert!(!finds("^150", "\u{2013}"));
    }
}
