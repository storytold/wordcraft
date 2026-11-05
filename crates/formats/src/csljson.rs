//! CSL-JSON (`.json`) citation libraries, read for References ▸ Manage Sources.
//!
//! The format is the Citation Style Language project's CSL-JSON 1.0 (the citeproc data schema):
//! a top-level array of items, or an object wrapping them as `{"items": […]}`. Per item `id`
//! becomes the tag, `type` the Word source kind (`journal-article`→article, `book`→book,
//! `chapter`→bookSection, `paper-conference`→conference, `report`→report, `webpage`→website,
//! else `other`), `author`/`editor` arrays of `{family, given, literal}` names join into
//! `"Last, First; …"` (`literal` verbatim), and `issued`'s `date-parts[0][0]` is the year.
//! `title`, `container-title`→journal, `volume`, `issue`, `page`→pages, `publisher`,
//! `publisher-place`→city, `URL`→url, `DOI`→doi (an `https://doi.org/` prefix stripped),
//! `edition` and `accessed` (a string kept as-is, a date-parts array written `yyyy-mm-dd`)
//! fill the rest. Numbers read as their text; unknown fields are ignored.
//!
//! JSON is parsed with `serde_json` (RFC 8259); like the crate's other parsers this one is
//! lenient and never panics on hostile input: input over 16 MiB is refused, anything nested
//! deeper than 32 levels errors, at most 10,000 items are kept, and values are cut off at
//! 4,000 characters. A file with no items is an error, not an empty library.

use serde_json::Value;
use wordcraft_doc::Source;

use crate::{MAX_LIBRARY_BYTES, MAX_NEST, MAX_SOURCES, clamp_value};

/// Import the sources of a CSL-JSON library.
pub fn import(bytes: &[u8]) -> Result<Vec<Source>, String> {
    if bytes.len() > MAX_LIBRARY_BYTES {
        return Err(format!("CSL-JSON library is larger than {} MiB", MAX_LIBRARY_BYTES >> 20));
    }
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    if b.iter().all(|c| c.is_ascii_whitespace()) {
        return Err("no CSL-JSON items found in the file".into());
    }
    check_depth(b)?;
    let top: Value = serde_json::from_slice(b).map_err(|e| format!("not CSL-JSON: {e}"))?;
    let items = match &top {
        Value::Array(items) => items.as_slice(),
        Value::Object(o) => o.get("items").and_then(Value::as_array).map_or(&[][..], Vec::as_slice),
        _ => return Err("not CSL-JSON: expected an array of items or an {\"items\": […]} object".into()),
    };
    let mut out = Vec::new();
    for item in items.iter().take(MAX_SOURCES) {
        if let Value::Object(o) = item {
            out.push(source(o));
        }
    }
    if out.is_empty() {
        return Err("no CSL-JSON items found in the file".into());
    }
    Ok(out)
}

/// Refuse nesting deeper than [`MAX_NEST`] before parsing: `serde_json` would allow 128 levels.
fn check_depth(b: &[u8]) -> Result<(), String> {
    let mut depth = 0usize;
    let mut in_str = false;
    let mut esc = false;
    for &c in b {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else if c == b'"' {
            in_str = true;
        } else if c == b'{' || c == b'[' {
            depth += 1;
            if depth > MAX_NEST {
                return Err(format!("JSON nested deeper than {MAX_NEST} levels"));
            }
        } else if c == b'}' || c == b']' {
            depth = depth.saturating_sub(1);
        }
    }
    Ok(())
}

/// The Word source kind for an item `type`.
fn kind_of(t: &str) -> &'static str {
    match t {
        "journal-article" => "article",
        "book" => "book",
        "chapter" => "bookSection",
        "paper-conference" => "conference",
        "report" => "report",
        "webpage" => "website",
        _ => "other",
    }
}

/// A JSON string or number as text; anything else is nothing.
fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// `author`/`editor` names joined as `Last, First; …`; `literal` names pass through.
fn names(v: Option<&Value>) -> String {
    let Some(arr) = v.and_then(Value::as_array) else { return String::new() };
    let mut out: Vec<String> = Vec::new();
    for n in arr {
        let Some(o) = n.as_object() else { continue };
        let family = text(o.get("family"));
        let given = text(o.get("given"));
        let name = match o.get("literal").and_then(Value::as_str) {
            Some(lit) => lit.to_string(),
            None if family.is_empty() => given,
            None if given.is_empty() => family,
            None => format!("{family}, {given}"),
        };
        if !name.trim().is_empty() {
            out.push(name);
        }
    }
    out.join("; ")
}

/// The parts of a CSL date's first `date-parts` array, as text (`[[2020, 3, 15]]` → 2020, 03, 15).
fn date_parts(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_object)
        .and_then(|o| o.get("date-parts"))
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_array)
        .map_or(Vec::new(), |p| {
            p.iter().take(3).enumerate().map(|(i, x)| if i == 0 { text(Some(x)) } else { format!("{:0>2}", text(Some(x))) }).collect()
        })
}

/// A source from one item; unknown fields are ignored.
fn source(o: &serde_json::Map<String, Value>) -> Source {
    Source {
        tag: clamp_value(crate::sanitize_tag(&text(o.get("id")))),
        kind: kind_of(&text(o.get("type"))).into(),
        author: clamp_value(names(o.get("author"))),
        editor: clamp_value(names(o.get("editor"))),
        year: clamp_value(year(o.get("issued"))),
        title: clamp_value(text(o.get("title"))),
        journal: clamp_value(text(o.get("container-title"))),
        volume: clamp_value(text(o.get("volume"))),
        issue: clamp_value(text(o.get("issue"))),
        pages: clamp_value(text(o.get("page"))),
        publisher: clamp_value(text(o.get("publisher"))),
        city: clamp_value(text(o.get("publisher-place"))),
        url: clamp_value(text(o.get("URL"))),
        doi: clamp_value(doi(o.get("DOI"))),
        edition: clamp_value(text(o.get("edition"))),
        accessed: clamp_value(accessed(o.get("accessed"))),
        ..Default::default()
    }
}

/// `issued`'s year, the first date-part.
fn year(v: Option<&Value>) -> String {
    date_parts(v).first().cloned().unwrap_or_default()
}

/// `accessed`: a string stays as-is, a date object becomes `yyyy-mm-dd`.
fn accessed(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(_)) => date_parts(v).join("-"),
        _ => String::new(),
    }
}

/// A DOI with its `https://doi.org/` (or http) prefix stripped.
fn doi(v: Option<&Value>) -> String {
    let d = text(v);
    ["https://doi.org/", "http://doi.org/"].iter().find_map(|p| d.strip_prefix(p)).unwrap_or(&d).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-written library exercising every mapped field.
    const LIB: &str = r#"[
  {
    "id": "doe2020",
    "type": "journal-article",
    "author": [{"family": "Doe", "given": "Jane"}, {"family": "Smith", "given": "John Q."}],
    "editor": [{"literal": "Editor, Anon"}],
    "issued": {"date-parts": [[2020, 3, 15]]},
    "title": "A study",
    "container-title": "Journal of Things",
    "volume": 12,
    "issue": "3",
    "page": "10-20",
    "publisher": "Big Publisher",
    "publisher-place": "Berlin",
    "URL": "https://example.com/a",
    "DOI": "https://doi.org/10.1000/csl",
    "edition": "2",
    "accessed": {"date-parts": [[2023, 1, 15]]},
    "note": "unknown fields are ignored"
  },
  {
    "id": 42,
    "type": "webpage",
    "author": [{"literal": "World Health Organization"}],
    "title": "Fact sheet",
    "URL": "https://who.int",
    "issued": {"date-parts": [["2021"]]},
    "accessed": "2022-02-02"
  }
]"#;

    /// A source with just tag and kind, for filling in field by field.
    fn src(tag: &str, kind: &str) -> Source {
        Source { tag: tag.into(), kind: kind.into(), ..Default::default() }
    }

    #[test]
    fn golden_array_maps_every_field() {
        let out = import(LIB.as_bytes()).unwrap();
        assert_eq!(out.len(), 2);

        let mut e = src("doe2020", "article");
        e.author = "Doe, Jane; Smith, John Q.".into();
        e.editor = "Editor, Anon".into(); // literal name, verbatim
        e.year = "2020".into(); // issued.date-parts[0][0]
        e.title = "A study".into();
        e.journal = "Journal of Things".into(); // container-title
        e.volume = "12".into(); // a number, read as text
        e.issue = "3".into();
        e.pages = "10-20".into(); // page
        e.publisher = "Big Publisher".into();
        e.city = "Berlin".into(); // publisher-place
        e.url = "https://example.com/a".into();
        e.doi = "10.1000/csl".into(); // https://doi.org/ stripped
        e.edition = "2".into();
        e.accessed = "2023-01-15".into(); // date-parts written yyyy-mm-dd
        assert_eq!(out[0], e);

        let mut e = src("42", "website"); // a numeric id read as text
        e.author = "World Health Organization".into();
        e.title = "Fact sheet".into();
        e.url = "https://who.int".into();
        e.year = "2021".into(); // a string date-part
        e.accessed = "2022-02-02".into(); // a string, kept as-is
        assert_eq!(out[1], e);
    }

    #[test]
    fn an_items_object_wraps_the_library() {
        let wrapped = format!("{{\"items\":{LIB}}}");
        assert_eq!(import(wrapped.as_bytes()).unwrap(), import(LIB.as_bytes()).unwrap());
    }

    #[test]
    fn every_type_maps_to_a_kind() {
        let types = ["journal-article", "book", "chapter", "paper-conference", "report", "webpage", "speech"];
        let kinds = ["article", "book", "bookSection", "conference", "report", "website", "other"];
        let lib = format!("[{}]", types.iter().enumerate().map(|(i, t)| format!(r#"{{"id":"{i}","type":"{t}"}}"#)).collect::<Vec<_>>().join(","));
        let out = import(lib.as_bytes()).unwrap();
        let got: Vec<&str> = out.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(got, kinds);
        // No type at all is "other" too.
        assert_eq!(import(b"[{\"id\":\"x\"}]").unwrap()[0].kind, "other");
    }

    #[test]
    fn names_join_last_comma_first() {
        let lib = r#"[
          {"id":"a","type":"book","author":[
            {"family":"Kowalski","given":"Jan"},
            {"literal":"Research Group"},
            {"given":"Only Given"},
            {"family":"Only Family"},
            {},
            "not a name"
          ]}
        ]"#;
        let out = import(lib.as_bytes()).unwrap();
        assert_eq!(out[0].author, "Kowalski, Jan; Research Group; Only Given; Only Family");
        // No authors, no empty string.
        assert_eq!(import(b"[{\"id\":\"x\"}]").unwrap()[0].author, "");
    }

    #[test]
    fn a_bom_is_stripped() {
        let mut v = b"\xEF\xBB\xBF".to_vec();
        v.extend_from_slice(br#"[{"id":"x","type":"book"}]"#);
        assert_eq!(import(&v).unwrap().len(), 1);
    }

    #[test]
    fn values_are_cut_off_at_4000_characters() {
        let at = format!(r#"[{{"id":"x","title":"{}"}}]"#, "a".repeat(4_000));
        assert_eq!(import(at.as_bytes()).unwrap()[0].title.chars().count(), 4_000);
        let over = format!(r#"[{{"id":"x","title":"{}"}}]"#, "a".repeat(4_100));
        assert_eq!(import(over.as_bytes()).unwrap()[0].title.chars().count(), 4_000);
    }

    #[test]
    fn items_are_capped_at_10000() {
        let lib = format!("[{}]", (0..10_002).map(|i| format!(r#"{{"id":"k{i}","type":"book"}}"#)).collect::<Vec<_>>().join(","));
        let out = import(lib.as_bytes()).unwrap();
        assert_eq!(out.len(), 10_000);
        assert_eq!(out[9_999].tag, "k9999");
    }

    #[test]
    fn nesting_deeper_than_32_errors() {
        let deep = format!("{}{}", "[".repeat(33), "]".repeat(33));
        let err = import(deep.as_bytes()).unwrap_err();
        assert!(err.contains("nested"), "{err}");
        // 32 levels pass the depth check (the failure after is about items, not nesting).
        let edge = format!("{}{}", "[".repeat(32), "]".repeat(32));
        let err = import(edge.as_bytes()).unwrap_err();
        assert!(!err.contains("nested"), "{err}");
    }

    #[test]
    fn junk_never_panics() {
        for junk in [
            &b""[..],
            b"not json",
            b"[",
            b"{\"items\": 3}",
            b"[]",
            b"[1, 2]",
            b"[null, true, \"s\"]",
            b"{\"no\":\"items\"}",
            b"\"\xff\"",
            &(0..=255u8).collect::<Vec<u8>>(),
        ] {
            let _ = import(junk);
        }
        assert_eq!(import(b"").unwrap_err(), "no CSL-JSON items found in the file");
        assert!(import(b"[]").unwrap_err().contains("no CSL-JSON items"));
        assert!(import(b"[1,2]").unwrap_err().contains("no CSL-JSON items"));
        let err = import(&vec![b'x'; (16 << 20) + 1]).unwrap_err();
        assert!(err.contains("16 MiB"), "{err}");
    }
}
