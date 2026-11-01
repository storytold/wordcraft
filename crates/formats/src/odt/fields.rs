//! Text fields (ODF 1.3 §7.3–7.7) and the Word field codes WordCraft keeps them as.
//!
//! Read: a field element becomes a field with that code and the element's text as its result:
//! a date or time with its date style as a `\@` picture, a sequence numbering figures or
//! tables as `SEQ`, a chapter field as `STYLEREF` to its heading style, references to
//! bookmarks, reference marks and sequences as `REF`/`PAGEREF`. A fixed date or time
//! (`text:fixed="true"`) is plain text, as is a field with no Word equivalent (variables, a
//! reference to a caption's text…), so nothing it shows is lost. Written: a field whose code
//! has an ODF element is that element around its result; any other field (or one with a code
//! ODF can't hold) is its result as text.

use super::writer::x;
use super::xml::{Attrs, get};

/// Longest bookmark name a reference field keeps; a longer one leaves the field as text.
const MAX_REF_NAME: usize = 1024;

/// ODF element (local name) ↔ Word field code, for the fields that take no argument.
const SIMPLE: &[(&str, &str)] = &[
    ("page-count", "NUMPAGES"),
    ("date", "DATE"),
    ("time", "TIME"),
    ("title", "TITLE"),
    ("subject", "SUBJECT"),
    ("initial-creator", "AUTHOR"),
    ("creator", "LASTSAVEDBY"),
    ("author-name", "USERNAME"),
    ("description", "COMMENTS"),
    ("keywords", "KEYWORDS"),
    ("file-name", "FILENAME"),
    ("word-count", "NUMWORDS"),
    ("character-count", "NUMCHARS"),
    ("creation-date", "CREATEDATE"),
    ("modification-date", "SAVEDATE"),
    ("print-date", "PRINTDATE"),
];

/// The Word field code for the `text:` field element `local` with attributes `a`, or `None`
/// when it is no field WordCraft keeps (then its text is read as text).
pub(super) fn instr_of(local: &str, a: &Attrs, picture: impl Fn(&str) -> Option<String>) -> Option<String> {
    let fixed = get(a, "text:fixed").is_some_and(|v| v.trim() == "true");
    // A name a field code can hold as one word.
    let name = |k: &str| {
        get(a, k)
            .map(str::trim)
            .filter(|n| !n.is_empty() && n.len() <= MAX_REF_NAME && !n.contains(|c: char| c.is_whitespace() || c == '"' || c == '\\'))
    };
    match local {
        // An index entry at a point (§8.1.10): Word's `XE`, its keys the levels above it.
        "alphabetical-index-mark" => xe_instr(get(a, "text:key1"), get(a, "text:key2"), get(a, "text:string-value")?),
        // Numbering figures, tables and the like (§7.4.13): Word's `SEQ` and its number format.
        "sequence" => {
            let seq = name("text:name")?;
            let fmt = match get(a, "style:num-format").map(str::trim) {
                Some("i") => "roman",
                Some("I") => "ROMAN",
                Some("a") => "alphabetic",
                Some("A") => "ALPHABETIC",
                _ => "ARABIC",
            };
            Some(format!("SEQ {seq} \\* {fmt}"))
        }
        // The chapter a page is in (§7.3.8): the nearest heading of its level, by name or number.
        "chapter" => {
            let level = get(a, "text:outline-level").and_then(|l| l.trim().parse::<u32>().ok()).unwrap_or(1).clamp(1, 9);
            let number = matches!(get(a, "text:display").map(str::trim), Some("number" | "plain-number"));
            Some(format!("STYLEREF \"Heading {level}\"{}", if number { " \\n" } else { "" }))
        }
        // References to reference marks (§7.7.5) and to sequences (§7.7.7): to the text or the
        // page; a sequence's label and number is the bookmark over them.
        "reference-ref" | "sequence-ref" => {
            let target = name("text:ref-name")?;
            match (local, get(a, "text:reference-format").map(str::trim)) {
                (_, Some("page")) => Some(format!("PAGEREF {target} \\h")),
                ("reference-ref", Some("text") | None) | ("sequence-ref", Some("category-and-value")) => Some(format!("REF {target} \\h")),
                _ => None,
            }
        }
        // Only the current page's number: "previous" and "next" have no Word equivalent.
        "page-number" => (get(a, "text:select-page").is_none_or(|v| v.trim() == "current")).then(|| {
            // Its own number format, over the page style's (Word's `\*` format switch).
            match get(a, "style:num-format").map(str::trim) {
                Some("1") => "PAGE \\* Arabic".to_string(),
                Some("i") => "PAGE \\* roman".to_string(),
                Some("I") => "PAGE \\* ROMAN".to_string(),
                Some("a") => "PAGE \\* alphabetic".to_string(),
                Some("A") => "PAGE \\* ALPHABETIC".to_string(),
                _ => "PAGE".to_string(),
            }
        }),
        "bookmark-ref" => {
            let name =
                get(a, "text:ref-name").map(str::trim).filter(|n| !n.is_empty() && n.len() <= MAX_REF_NAME && !n.contains(char::is_whitespace))?;
            Some(match get(a, "text:reference-format").map(str::trim) {
                Some("page") => format!("PAGEREF {name} \\h"),
                Some("number" | "number-all-superior" | "number-no-superior") => format!("REF {name} \\r \\h"),
                _ => format!("REF {name} \\h"),
            })
        }
        _ if fixed => None,
        _ => {
            let code = SIMPLE.iter().find(|(e, _)| *e == local).map(|(_, code)| code.to_string())?;
            // A date or time in its date style (§19.473 `style:data-style-name`).
            let pic = get(a, "style:data-style-name").and_then(&picture).filter(|_| code.contains("DATE") || code == "TIME");
            Some(match pic {
                Some(p) => format!("{code} \\@ \"{p}\""),
                None => code,
            })
        }
    }
}

/// The ODF field element for Word field code `instr` around `result` (already escaped), or
/// `None` when ODF has none for it.
pub(super) fn element(instr: &str, result: &str, data_style: Option<&str>) -> Option<String> {
    let mut words = instr.split_whitespace();
    let code = words.next()?.to_ascii_uppercase();
    match code.as_str() {
        "SEQ" => {
            let seq = words.next().filter(|n| !n.starts_with('\\') && n.len() <= MAX_REF_NAME)?;
            let fmt = match switch_arg(instr, '*').as_deref() {
                Some("roman") => "i",
                Some("ROMAN") => "I",
                Some("alphabetic") => "a",
                Some("ALPHABETIC") => "A",
                _ => "1",
            };
            let seq = x(seq);
            Some(format!("<text:sequence text:name=\"{seq}\" text:formula=\"ooow:{seq}+1\" style:num-format=\"{fmt}\">{result}</text:sequence>"))
        }
        // `STYLEREF "Heading 2"` or `STYLEREF 2`: the chapter of that level.
        "STYLEREF" => {
            let target = instr.get(code.len()..)?.trim_start();
            let target = target.strip_prefix('"').and_then(|t| t.split('"').next()).unwrap_or_else(|| target.split_whitespace().next().unwrap_or(""));
            let level = target.trim().strip_prefix("Heading").unwrap_or(target).trim().parse::<u32>().ok().filter(|l| (1..=10).contains(l))?;
            let number = [" \\n", " \\r", " \\w"].iter().any(|s| instr.contains(s));
            let display = if number { "number" } else { "name" };
            Some(format!("<text:chapter text:display=\"{display}\" text:outline-level=\"{level}\">{result}</text:chapter>"))
        }
        "PAGE" => {
            let fmt = match switch_arg(instr, '*').as_deref() {
                Some(f) if f.eq_ignore_ascii_case("arabic") => " style:num-format=\"1\"",
                Some("roman") => " style:num-format=\"i\"",
                Some("ROMAN") => " style:num-format=\"I\"",
                Some("alphabetic") => " style:num-format=\"a\"",
                Some("ALPHABETIC") => " style:num-format=\"A\"",
                _ => "",
            };
            Some(format!("<text:page-number{fmt} text:select-page=\"current\">{result}</text:page-number>"))
        }
        // An index entry: a mark at its place, its first levels the keys (§8.1.10).
        "XE" => {
            let entry = instr.get(code.len()..)?.trim_start();
            let entry = entry.strip_prefix('"').and_then(|t| t.split('"').next())?;
            let mut levels: Vec<&str> = entry.split(':').map(str::trim).filter(|l| !l.is_empty()).collect();
            let value = levels.pop()?;
            let keys: String = levels.iter().take(2).enumerate().map(|(i, k)| format!(" text:key{}=\"{}\"", i + 1, x(k))).collect();
            Some(format!("<text:alphabetical-index-mark text:string-value=\"{}\"{keys}/>", x(value)))
        }
        "REF" | "PAGEREF" => {
            let name = words.next().filter(|n| !n.starts_with('\\') && n.len() <= MAX_REF_NAME)?;
            let rest: Vec<&str> = words.collect();
            let format = if code == "PAGEREF" {
                "page"
            } else if rest.iter().any(|s| s.eq_ignore_ascii_case("\\r") || s.eq_ignore_ascii_case("\\n") || s.eq_ignore_ascii_case("\\w")) {
                "number"
            } else {
                "text"
            };
            Some(format!("<text:bookmark-ref text:reference-format=\"{format}\" text:ref-name=\"{}\">{result}</text:bookmark-ref>", x(name)))
        }
        _ => {
            let (el, _) = SIMPLE.iter().find(|(_, c)| *c == code)?;
            let ds = data_style.map(|d| format!(" style:data-style-name=\"{}\"", x(d))).unwrap_or_default();
            Some(format!("<text:{el}{ds}>{result}</text:{el}>"))
        }
    }
}

/// Most characters of an index entry's level.
const MAX_ENTRY_LEVEL: usize = 255;

/// The `XE` field code of an index entry `value` under keys `key1` and `key2` (its levels, top
/// first). Quotes, which end the entry, and colons, which split levels, are left out of each.
/// `None` when the entry is empty.
pub(super) fn xe_instr(key1: Option<&str>, key2: Option<&str>, value: &str) -> Option<String> {
    let clean = |s: &str| {
        s.chars().filter(|c| !matches!(c, '"' | ':' | '\\') && !c.is_control()).take(MAX_ENTRY_LEVEL).collect::<String>().trim().to_string()
    };
    let value = clean(value);
    if value.is_empty() {
        return None;
    }
    let levels: Vec<String> = [key1, key2].into_iter().flatten().map(clean).filter(|k| !k.is_empty()).chain([value]).collect();
    Some(format!("XE \"{}\"", levels.join(":")))
}

/// The argument of switch `\sw` in field code `instr` (quoted or one word).
fn switch_arg(instr: &str, sw: char) -> Option<String> {
    let at = instr.find(&format!("\\{sw}"))?;
    let rest = instr.get(at + 2..)?.trim_start();
    let arg = match rest.strip_prefix('"') {
        Some(q) => q.split('"').next()?,
        None => rest.split_whitespace().next()?,
    };
    Some(arg.chars().take(64).collect())
}

/// The sequence (`SEQ`) field code `instr` counts, if it is one.
pub(super) fn sequence_name(instr: &str) -> Option<&str> {
    let mut words = instr.split_whitespace();
    words.next().filter(|c| c.eq_ignore_ascii_case("SEQ"))?;
    words.next().filter(|n| !n.starts_with('\\') && n.len() <= MAX_REF_NAME)
}
