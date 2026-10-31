//! Text fields (ODF 1.3 §7.3–7.7) and the Word field codes WordCraft keeps them as.
//!
//! Read: a field element becomes a field with that code and the element's text as its result.
//! A fixed date or time (`text:fixed="true"`) is plain text, as is a field with no Word
//! equivalent (sequences, chapters, variables…), so nothing it shows is lost. Written: a field
//! whose code has an ODF element is that element around its result; any other field (or one
//! with a code ODF can't hold) is its result as text.

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
pub(super) fn instr_of(local: &str, a: &Attrs) -> Option<String> {
    let fixed = get(a, "text:fixed").is_some_and(|v| v.trim() == "true");
    match local {
        // Only the current page's number: "previous" and "next" have no Word equivalent.
        "page-number" => (get(a, "text:select-page").is_none_or(|v| v.trim() == "current")).then(|| "PAGE".to_string()),
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
        _ => SIMPLE.iter().find(|(e, _)| *e == local).map(|(_, code)| code.to_string()),
    }
}

/// The ODF field element for Word field code `instr` around `result` (already escaped), or
/// `None` when ODF has none for it.
pub(super) fn element(instr: &str, result: &str) -> Option<String> {
    let mut words = instr.split_whitespace();
    let code = words.next()?.to_ascii_uppercase();
    match code.as_str() {
        "PAGE" => Some(format!("<text:page-number text:select-page=\"current\">{result}</text:page-number>")),
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
            Some(format!("<text:{el}>{result}</text:{el}>"))
        }
    }
}
