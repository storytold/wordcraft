//! BibTeX (`.bib`) citation libraries, read for References ▸ Manage Sources.
//!
//! The entry syntax is Oren Patashnik's ("BibTeXing", 1988): `@type{key, field = value, …}`
//! with `(`…`)` interchangeable with `{`…`}`, values braced `{…{…}…}` (inner braces are content,
//! BibTeX's capitalisation guards), quoted `"…"` (braces guard quotes inside), bare `@string`
//! names and `jan`…`dec` month macros, joined with `#`. `@string` definitions interpolate into
//! later values; `@comment` and `@preamble` are skipped. Type, field and string names are
//! case-insensitive; anything between entries is ignored.
//!
//! The source kind comes from the entry type (`article`→article, `book`→book,
//! `inbook`/`incollection`→bookSection, `inproceedings`/`conference`→conference,
//! `phdthesis`/`mastersthesis`/`masterthesis`/`techreport`→report, `online`/`misc`→website, else
//! `other`); `number` maps to the issue, `address` to the city, `booktitle` to the journal, and
//! `pages` `10--20` becomes `10–20`. The citation tag is the entry key, kept in
//! `[A-Za-z0-9_./:-]` with other runs collapsed to one `-`, so a `CITATION` field instruction
//! survives it.
//!
//! Like the crate's other parsers this one is lenient and never panics on hostile input: input
//! over 16 MiB is refused, at most 10,000 entries are kept, field values are cut off at 4,000
//! characters, and braces nested deeper than 32 error the file. An entry left open at end of
//! file is flushed when it has content.

use std::collections::HashMap;

use wordcraft_doc::Source;

use crate::{
    sanitize_tag, {MAX_LIBRARY_BYTES, MAX_NEST, MAX_SOURCES, MAX_VALUE, clamp_value, utf8_or_latin1},
};

/// Import the sources of a BibTeX library.
pub fn import(bytes: &[u8]) -> Result<Vec<Source>, String> {
    if bytes.len() > MAX_LIBRARY_BYTES {
        return Err(format!("BibTeX library is larger than {} MiB", MAX_LIBRARY_BYTES >> 20));
    }
    let mut p = Parser { b: utf8_or_latin1(bytes).into_bytes(), at: 0, strings: HashMap::new(), out: Vec::new() };
    p.run()?;
    if p.out.is_empty() {
        return Err("no BibTeX entries found in the file".into());
    }
    Ok(p.out)
}

/// The en dash BibTeX's `10--20` page ranges become.
const EN_DASH: &str = "–";

fn depth_err() -> String {
    format!("BibTeX braces nested deeper than {MAX_NEST} levels")
}

/// The word's source kind for an entry type.
fn kind_of(ty: &str) -> &'static str {
    match ty {
        "article" => "article",
        "book" => "book",
        "inbook" | "incollection" => "bookSection",
        "inproceedings" | "conference" => "conference",
        "phdthesis" | "mastersthesis" | "masterthesis" | "techreport" => "report",
        "online" | "misc" => "website",
        _ => "other",
    }
}

/// The entry key as a citation tag: `[A-Za-z0-9_./:-]` kept, other runs one `-`.
/// The `jan`…`dec` macros the standard styles define.
fn month_macro(token: &str) -> Option<&'static str> {
    const MONTHS: [(&str, &str); 12] = [
        ("jan", "January"),
        ("feb", "February"),
        ("mar", "March"),
        ("apr", "April"),
        ("may", "May"),
        ("jun", "June"),
        ("jul", "July"),
        ("aug", "August"),
        ("sep", "September"),
        ("oct", "October"),
        ("nov", "November"),
        ("dec", "December"),
    ];
    MONTHS.iter().find(|(k, _)| k.eq_ignore_ascii_case(token)).map(|(_, v)| *v)
}

/// Set a mapped field, first value wins, `pages` normalised to an en dash range.
/// A name list with BibTeX's top-level ` and ` separators turned into the model's `; `:
/// brace depth guards the word, so an `and` inside a braced name stays, and matching is
/// ASCII-case-insensitive (`AND`, `And`). Each name is trimmed; empty names are dropped.
fn join_names(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let is_and_at = |k: usize| -> bool {
        // `and` (any case) standing alone: whitespace (or the ends) around it.
        k + 3 <= chars.len()
            && chars[k].eq_ignore_ascii_case(&'a')
            && chars[k + 1].eq_ignore_ascii_case(&'n')
            && chars[k + 2].eq_ignore_ascii_case(&'d')
            && (k == 0 || chars[k - 1].is_whitespace())
            && (k + 3 == chars.len() || chars[k + 3].is_whitespace())
    };
    let mut names: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let mut k = 0;
    while k < chars.len() {
        let c = chars[k];
        if c == '{' {
            depth += 1;
            cur.push(c);
        } else if c == '}' {
            depth = depth.saturating_sub(1);
            cur.push(c);
        } else if depth == 0 && is_and_at(k) {
            names.push(cur.trim().to_string());
            cur.clear();
            k += 3;
            continue;
        } else {
            cur.push(c);
        }
        k += 1;
    }
    names.push(cur.trim().to_string());
    names.into_iter().filter(|n| !n.is_empty()).collect::<Vec<_>>().join("; ")
}

fn set_field(src: &mut Source, name: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    // BibTeX separates names with the word `and` ("Doe, Jane and Smith, John"); the model's
    // lists are `Last, First; Last, First`, so top-level separators become `; `. Inside braces
    // an `and` is a name's own words ("{Barnes and Noble}") and stays.
    let joined;
    let value = if matches!(name, "author" | "editor") {
        joined = join_names(value);
        joined.as_str()
    } else {
        value
    };
    let set = |slot: &mut String| {
        if slot.is_empty() {
            *slot = clamp_value(value.to_string());
        }
    };
    match name {
        "author" => set(&mut src.author),
        "title" => set(&mut src.title),
        "year" => set(&mut src.year),
        "journal" | "booktitle" => set(&mut src.journal),
        "volume" => set(&mut src.volume),
        "number" => set(&mut src.issue),
        "pages" if src.pages.is_empty() => src.pages = clamp_value(value.replace("--", EN_DASH)),
        "publisher" => set(&mut src.publisher),
        "address" => set(&mut src.city),
        "url" => set(&mut src.url),
        "doi" => set(&mut src.doi),
        "month" => set(&mut src.month),
        "editor" => set(&mut src.editor),
        "edition" => set(&mut src.edition),
        _ => {}
    }
}

/// Does the source hold anything at all?
fn has_content(s: &Source) -> bool {
    [&s.author, &s.title, &s.year, &s.journal, &s.volume, &s.pages, &s.publisher, &s.city, &s.url, &s.editor, &s.edition, &s.issue, &s.month]
        .iter()
        .any(|f| !f.is_empty())
}

struct Parser {
    b: Vec<u8>,
    at: usize,
    /// `@string` definitions, name lowercased.
    strings: HashMap<String, String>,
    out: Vec<Source>,
}

impl Parser {
    /// Scan `@entry`s to the end of the file or the entry cap.
    fn run(&mut self) -> Result<(), String> {
        while self.out.len() < MAX_SOURCES {
            let at = self.b.get(self.at..).unwrap_or(&[]).iter().position(|c| *c == b'@');
            let Some(i) = at else { break };
            self.at += i + 1;
            let name = self.word();
            self.ws();
            let Some(close) = self.delim() else { continue };
            match name.as_str() {
                "comment" | "preamble" => self.skip_to_close(close)?,
                "string" => self.parse_string(close)?,
                _ => self.parse_entry(close, &name)?,
            }
        }
        Ok(())
    }

    /// A run of ASCII alphanumerics, lowercased.
    fn word(&mut self) -> String {
        let start = self.at;
        while self.b.get(self.at).is_some_and(|c| c.is_ascii_alphanumeric()) {
            self.at += 1;
        }
        self.slice(start, self.at).to_ascii_lowercase()
    }

    fn ws(&mut self) {
        while self.b.get(self.at).is_some_and(|c| c.is_ascii_whitespace()) {
            self.at += 1;
        }
    }

    /// The entry's opening delimiter, consumed: `}` or `)` to close it.
    fn delim(&mut self) -> Option<u8> {
        match self.b.get(self.at) {
            Some(b'{') => {
                self.at += 1;
                Some(b'}')
            }
            Some(b'(') => {
                self.at += 1;
                Some(b')')
            }
            _ => None,
        }
    }

    /// `b[a..b]` as text; cuts are always at ASCII delimiters, so this cannot split a character
    /// (the `unwrap_or`s only guard that promise).
    fn slice(&self, a: usize, b: usize) -> String {
        std::str::from_utf8(self.b.get(a..b).unwrap_or(&[])).unwrap_or_default().to_string()
    }

    /// Skip to the entry's closing delimiter, tracking nested braces and parens; end of file
    /// ends the skip.
    fn skip_to_close(&mut self, close: u8) -> Result<(), String> {
        let mut br = 0usize;
        let mut pa = 0usize;
        while let Some(c) = self.b.get(self.at).copied() {
            match c {
                b'{' => {
                    br += 1;
                    if br > MAX_NEST {
                        return Err(depth_err());
                    }
                }
                b'}' if close == b'}' && br == 0 => {
                    self.at += 1;
                    return Ok(());
                }
                b'}' => br = br.saturating_sub(1),
                b'(' => {
                    pa += 1;
                    if pa > MAX_NEST {
                        return Err(depth_err());
                    }
                }
                b')' if close == b')' && br == 0 && pa == 0 => {
                    self.at += 1;
                    return Ok(());
                }
                b')' => pa = pa.saturating_sub(1),
                _ => {}
            }
            self.at += 1;
        }
        Ok(())
    }

    /// `@string{name = value}`: define `name`, lowercased, for later interpolation.
    fn parse_string(&mut self, close: u8) -> Result<(), String> {
        self.ws();
        let start = self.at;
        while self.b.get(self.at).copied().is_some_and(|c| !name_end(c)) {
            self.at += 1;
        }
        let name = self.slice(start, self.at).to_ascii_lowercase();
        self.ws();
        if name.is_empty() || self.b.get(self.at) != Some(&b'=') {
            return self.skip_to_close(close); // not a definition: drop it
        }
        self.at += 1;
        let value = self.value()?;
        self.strings.insert(name, value);
        self.skip_to_close(close) // tolerate a trailing comma or junk before the close
    }

    /// One entry: the key to the first comma, then `field = value` until the delimiter closes.
    /// An entry left open at end of file is flushed when it has a key or content.
    fn parse_entry(&mut self, close: u8, name: &str) -> Result<(), String> {
        let start = self.at;
        while let Some(c) = self.b.get(self.at).copied() {
            if c == b',' || c == close {
                break;
            }
            self.at += 1;
        }
        let key = self.slice(start, self.at).trim().to_string();
        let mut src = Source { tag: sanitize_tag(&key), kind: kind_of(name).into(), ..Default::default() };
        if self.b.get(self.at) == Some(&b',') {
            self.at += 1;
            self.fields(close, &mut src)?;
        } else if self.b.get(self.at) == Some(&close) {
            self.at += 1;
        }
        if !key.is_empty() || has_content(&src) {
            self.out.push(src);
        }
        Ok(())
    }

    /// `field = value` pairs; anything unexpected is skipped, never fatal.
    fn fields(&mut self, close: u8, src: &mut Source) -> Result<(), String> {
        loop {
            self.ws();
            match self.b.get(self.at).copied() {
                None => return Ok(()), // end of file inside the entry: the caller flushes
                Some(c) if c == close => {
                    self.at += 1;
                    return Ok(());
                }
                Some(b',') => self.at += 1,
                _ => {
                    let start = self.at;
                    while self.b.get(self.at).copied().is_some_and(|c| !name_end(c)) {
                        self.at += 1;
                    }
                    let name = self.slice(start, self.at).to_ascii_lowercase();
                    if self.at == start {
                        self.at += 1; // a stray delimiter: make progress
                    }
                    self.ws();
                    if self.b.get(self.at) == Some(&b'=') {
                        self.at += 1;
                        let value = self.value()?;
                        set_field(src, &name, &value);
                    }
                }
            }
        }
    }

    /// A value: braced `{…}`, quoted `"…"` and bare `@string`/macro tokens, joined with `#`.
    /// Inner braces are content and stay; an unterminated part yields what there is. Only the
    /// first [`MAX_VALUE`] characters are ever kept (the field clamp keeps just those anyway), so
    /// a value cannot amplify past the caps however many `#` parts it has.
    fn value(&mut self) -> Result<String, String> {
        let mut out = String::new();
        let mut len = 0usize; // characters in `out`
        loop {
            self.ws();
            let start = self.at;
            // Parts past the clamp are parsed (so the file advances) but no longer kept.
            let room = MAX_VALUE.saturating_sub(len);
            match self.b.get(self.at).copied() {
                Some(b'{') => {
                    self.at += 1;
                    let mut depth = 1usize;
                    let mut closed = false;
                    while let Some(c) = self.b.get(self.at).copied() {
                        match c {
                            b'{' => {
                                depth += 1;
                                if depth > MAX_NEST {
                                    return Err(depth_err());
                                }
                            }
                            b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    closed = true;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        self.at += 1;
                    }
                    let end = if closed { self.at } else { self.b.len() };
                    if room > 0 {
                        len += push_upto(&mut out, &self.slice(start + 1, end), room);
                    }
                    if closed {
                        self.at += 1;
                    }
                }
                Some(b'"') => {
                    self.at += 1;
                    let mut depth = 0usize; // braces guard quotes inside quotes
                    let mut closed = false;
                    while let Some(c) = self.b.get(self.at).copied() {
                        match c {
                            b'"' if depth == 0 => {
                                closed = true;
                                break;
                            }
                            b'{' => {
                                depth += 1;
                                if depth > MAX_NEST {
                                    return Err(depth_err());
                                }
                            }
                            b'}' => depth = depth.saturating_sub(1),
                            _ => {}
                        }
                        self.at += 1;
                    }
                    let end = if closed { self.at } else { self.b.len() };
                    if room > 0 {
                        len += push_upto(&mut out, &self.slice(start + 1, end), room);
                    }
                    if closed {
                        self.at += 1;
                    }
                }
                Some(c) if !token_end(c) => {
                    while self.b.get(self.at).copied().is_some_and(|c| !token_end(c)) {
                        self.at += 1;
                    }
                    if room > 0 {
                        let token = self.slice(start, self.at);
                        if let Some(v) = self.strings.get(&token.to_ascii_lowercase()) {
                            len += push_upto(&mut out, v, room);
                        } else if let Some(m) = month_macro(&token) {
                            len += push_upto(&mut out, m, room);
                        } else {
                            len += push_upto(&mut out, &token, room);
                        }
                    }
                }
                _ => {} // no value here (a comma, the close, or the end): nothing to add
            }
            self.ws();
            if self.b.get(self.at) != Some(&b'#') {
                break;
            }
            self.at += 1;
        }
        Ok(out)
    }
}

/// Does the byte end a bare token?
fn token_end(c: u8) -> bool {
    c.is_ascii_whitespace() || matches!(c, b',' | b'#' | b'{' | b'}' | b'(' | b')' | b'"')
}

/// Push at most `room` characters of `part`, returning how many that was: a value keeps only
/// its first [`MAX_VALUE`] characters ([`set_field`] clamps to those anyway), so nothing past
/// them is ever observable and a `#` chain cannot amplify past the caps.
fn push_upto(out: &mut String, part: &str, room: usize) -> usize {
    let mut n = 0usize;
    for c in part.chars() {
        if n == room {
            break;
        }
        out.push(c);
        n += 1;
    }
    n
}

/// Does the byte end a field name?
fn name_end(c: u8) -> bool {
    c.is_ascii_whitespace() || matches!(c, b'=' | b',' | b'{' | b'}' | b'(' | b')')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-written library exercising every mapped field and form of value.
    const LIB: &str = r#"% A small library exported from a reference manager. Free text between
% entries is ignored, and an email like jane@example.com starts nothing.
@string{pub = "Journal of Experiments"}
@string{pre = "Aa"}
@string{post = "Bb"}

@article{doe2020,
  author  = {Doe, Jane and Smith, John},
  title   = {A Study of {Braces} and Café},
  journal = pub,
  volume  = 12,
  number  = 3,
  pages   = {10--20},
  month   = mar,
  year    = {2020},
  doi     = {10.1000/doe2020},
  url     = {https://example.com/paper}
}

@Comment{anything in here is skipped}

@preamble{"\newcommand{\noopsort}[1]{}"}

@inproceedings(conf/key!,
  author = "O'Neill, Grace",
  title  = "A {"quoted"} conference talk",
  booktitle = {Proceedings of Things},
  year = 2019
)

@book{knuth84,
  author = {Knuth, Donald E.},
  editor = {Editor, Ed},
  title = {The TeXbook},
  publisher = {Addison-Wesley},
  address = {Reading, MA},
  edition = {2nd},
  year = {1984}
}

@phdthesis{muller:2021,
  author = {Müller, Hans},
  title = {Eine Untersuchung},
  school = {ETH Zürich},
  address = {Zürich},
  year = {2021}
}

@online{web1,
  author = {World Health Organization},
  title = {Fact sheet},
  url = {https://who.int},
  year = {2023}
}

@misc{weird key/name,
  title = pre # "-" # post,
  month = jan
}
"#;

    /// A source with just tag and kind, for filling in field by field.
    fn src(tag: &str, kind: &str) -> Source {
        Source { tag: tag.into(), kind: kind.into(), ..Default::default() }
    }

    #[test]
    fn author_and_editor_split_on_top_level_and() {
        // Top-level `and` (any case, whitespace-bounded) separates names; inside braces it is a
        // name's own words and survives.
        let lib = br#"@book{a, author = {Doe, Jane AND Smith, John AND {Barnes and Noble}}, editor = {Roe, Sam and Lee, Ann}}
@book{b, author = {Solo Author}}"#;
        let out = import(lib).unwrap();
        assert_eq!(out[0].author, "Doe, Jane; Smith, John; {Barnes and Noble}");
        assert_eq!(out[0].editor, "Roe, Sam; Lee, Ann");
        assert_eq!(out[1].author, "Solo Author");
    }

    #[test]
    fn golden_library_maps_every_field() {
        let out = import(LIB.as_bytes()).unwrap();
        assert_eq!(out.len(), 6);

        let mut e = src("doe2020", "article");
        e.author = "Doe, Jane; Smith, John".into(); // top-level `and` → "; "
        e.title = "A Study of {Braces} and Café".into(); // inner braces are content, kept
        e.journal = "Journal of Experiments".into(); // @string interpolation
        e.volume = "12".into(); // bare number value
        e.issue = "3".into(); // number → issue
        e.pages = "10–20".into(); // -- → en dash
        e.month = "March".into(); // month macro
        e.year = "2020".into();
        e.doi = "10.1000/doe2020".into();
        e.url = "https://example.com/paper".into();
        assert_eq!(out[0], e);

        let mut e = src("conf/key-", "conference"); // '!' in the key collapses to '-'
        e.author = "O'Neill, Grace".into(); // quoted value
        e.title = "A {\"quoted\"} conference talk".into(); // braces guard quotes inside quotes
        e.journal = "Proceedings of Things".into(); // booktitle → journal
        e.year = "2019".into();
        assert_eq!(out[1], e);

        let mut e = src("knuth84", "book");
        e.author = "Knuth, Donald E.".into();
        e.editor = "Editor, Ed".into();
        e.title = "The TeXbook".into();
        e.publisher = "Addison-Wesley".into();
        e.city = "Reading, MA".into(); // address → city
        e.edition = "2nd".into();
        e.year = "1984".into();
        assert_eq!(out[2], e);

        let mut e = src("muller:2021", "report"); // phdthesis → report, ':' kept in the tag
        e.author = "Müller, Hans".into();
        e.title = "Eine Untersuchung".into();
        e.city = "Zürich".into(); // school is not a mapped field
        e.year = "2021".into();
        assert_eq!(out[3], e);

        let mut e = src("web1", "website"); // online → website
        e.author = "World Health Organization".into();
        e.title = "Fact sheet".into();
        e.url = "https://who.int".into();
        e.year = "2023".into();
        assert_eq!(out[4], e);

        let mut e = src("weird-key/name", "website"); // misc → website, spaces collapse
        e.title = "Aa-Bb".into(); // # concatenation of two @strings and a literal
        e.month = "January".into();
        assert_eq!(out[5], e);
    }

    #[test]
    fn every_entry_type_maps_to_a_kind() {
        let lib = "@speech{a}\n@chapter{b}\n@dictionary{c}\n@MASTERSTHESIS{d}\n@techreport{e}\n@InCollection{f}";
        let out = import(lib.as_bytes()).unwrap();
        let kinds: Vec<&str> = out.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(kinds, ["other", "other", "other", "report", "report", "bookSection"]);
    }

    #[test]
    fn an_entry_with_only_a_key_still_imports() {
        let out = import(b"@article{empty}").unwrap();
        assert_eq!(out, vec![src("empty", "article")]);
    }

    #[test]
    fn double_braced_values_keep_their_inner_braces() {
        let out = import(b"@misc{a, author = {{Organisation Name}}}").unwrap();
        assert_eq!(out[0].author, "{Organisation Name}");
    }

    #[test]
    fn unterminated_entry_flushes_at_eof() {
        let out = import(b"@article{x, title = {never closed").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "never closed");
        // An entry with no key and no fields before the end is dropped, not flushed.
        assert_eq!(import(b"@article{").unwrap_err(), "no BibTeX entries found in the file");
    }

    #[test]
    fn braces_nested_deeper_than_32_error() {
        let ok = "@article{x, title = {a".to_string() + &"{".repeat(31);
        assert!(import(ok.as_bytes()).is_ok());
        for bad in [
            "@article{x, title = {a".to_string() + &"{".repeat(32),
            "@article{x, title = {".to_string() + &"{".repeat(100_000),
            "@comment{".to_string() + &"{".repeat(64),
        ] {
            let err = import(bad.as_bytes()).unwrap_err();
            assert!(err.contains("nested"), "{err}");
        }
    }

    #[test]
    fn values_are_cut_off_at_4000_characters() {
        let at = "@misc{x, title = {".to_string() + &"a".repeat(4_000) + "}";
        assert_eq!(import(at.as_bytes()).unwrap()[0].title.chars().count(), 4_000);
        let over = "@misc{x, title = {".to_string() + &"a".repeat(4_100) + "}";
        assert_eq!(import(over.as_bytes()).unwrap()[0].title.chars().count(), 4_000);
        // At a character boundary: a multi-byte last character is not split.
        let uni = "@misc{x, title = {".to_string() + &"é".repeat(4_002) + "}";
        assert_eq!(import(uni.as_bytes()).unwrap()[0].title.chars().count(), 4_000);
    }

    #[test]
    fn concatenated_values_stay_bounded() {
        // A small file whose @string is referenced millions of times must not amplify into
        // gigabytes: parts past the 4,000-character clamp are consumed, not kept.
        let lib = "@string{big = {".to_string() + &"x".repeat(16 << 10) + "}}\n@misc{x, title = big" + &" # big".repeat(2_000_000) + "}\n";
        assert!(lib.len() < 16 << 20, "{}", lib.len());
        let started = std::time::Instant::now();
        let out = import(lib.as_bytes()).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title.chars().count(), 4_000);
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    }

    #[test]
    fn entries_are_capped_at_10000() {
        let mut lib = String::new();
        for i in 0..10_005 {
            lib.push_str(&format!("@misc{{k{i}, title = {{t}}}}\n"));
        }
        let out = import(lib.as_bytes()).unwrap();
        assert_eq!(out.len(), 10_000);
        assert_eq!(out[9_999].tag, "k9999");
    }

    #[test]
    fn input_over_16mib_is_refused() {
        let err = import(&vec![b'x'; (16 << 20) + 1]).unwrap_err();
        assert!(err.contains("16 MiB"), "{err}");
        assert!(import(&vec![b'x'; 16 << 20]).is_err());
    }

    #[test]
    fn junk_never_panics() {
        for junk in [
            &b""[..],
            b"hello",
            b"@",
            b"@article",
            b"@article no brace",
            b"@journal-article{x, title = {t}}", // '-' ends the type word: not an entry
            b"@string{",
            b"@string{a}",
            b"@preamble{",
            b"\x00\x01\x02",
            &(0..=255u8).collect::<Vec<u8>>(),
            b"@article{x, title = }",
            b"@article{x, = = =, title = {t}}",
        ] {
            // Whatever comes back is an error or sources; the point is not panicking.
            let _ = import(junk);
        }
        assert_eq!(import(b"").unwrap_err(), "no BibTeX entries found in the file");
        assert!(import(&(0..=255u8).collect::<Vec<u8>>()).is_err());
    }
}
