//! RIS (`.ris`) citation libraries, read for References ▸ Manage Sources.
//!
//! RIS is the Research Information Systems tag format kept by Clarivate (EndNote): one
//! `TAG  - value` per line (two-character tag, two spaces, hyphen — spacing tolerated), a record
//! opened by `TY  - type` and closed by `ER  -`, leniently flushed at end of file. Tags other
//! than the mapped ones are ignored; lines that are not tag lines are ignored. `AU` lines are
//! one author each and pass through (RIS names are `Last, First` in practice), `SP`/`EP` join
//! into `pages` with an en dash, `PY` gives a 4-digit year with `DA` as fallback, and `T2`, `JO`
//! and `JF` are all read as the journal. `TY` maps to the Word source kind (`JOUR`→article,
//! `BOOK`→book, `CHAP`→bookSection, `CONF`→conference, `RPRT`/`THES`→report,
//! `ELEC`/`EJOUR`/`WEBS`→website, else `other`). RIS carries no key, so the tag stays empty and
//! the engine generates one.
//!
//! Bytes decode as UTF-8 first (BOM stripped), then as latin-1 byte-preserving (Mendeley exports
//! both). Like the crate's other parsers this one is lenient and never panics on hostile input:
//! input over 16 MiB is refused, at most 10,000 records are kept, and values are cut off at
//! 4,000 characters.

use wordcraft_doc::Source;

use crate::{MAX_LIBRARY_BYTES, MAX_SOURCES, clamp_value, utf8_or_latin1};

/// Import the sources of an RIS library.
pub fn import(bytes: &[u8]) -> Result<Vec<Source>, String> {
    if bytes.len() > MAX_LIBRARY_BYTES {
        return Err(format!("RIS library is larger than {} MiB", MAX_LIBRARY_BYTES >> 20));
    }
    let text = utf8_or_latin1(bytes);
    let mut out = Vec::new();
    let mut rec: Option<Rec> = None; // a record is open from its TY to its ER (or the end)
    for line in text.lines() {
        if out.len() >= MAX_SOURCES {
            break;
        }
        let Some((tag, value)) = tag_line(line) else { continue };
        let tag = tag.to_ascii_uppercase();
        match tag.as_str() {
            "TY" => {
                flush(rec.take(), &mut out);
                rec = Some(Rec { ty: value.to_string(), ..Default::default() });
            }
            "ER" => flush(rec.take(), &mut out),
            _ => {
                if let Some(r) = rec.as_mut() {
                    set_tag(r, &tag, value);
                }
            }
        }
    }
    flush(rec, &mut out);
    if out.is_empty() {
        return Err("no RIS records found in the file".into());
    }
    Ok(out)
}

/// A record's tags while it is being read.
#[derive(Default)]
struct Rec {
    ty: String,
    authors: Vec<String>,
    title: String,
    journal: String,
    volume: String,
    issue: String,
    start: String,
    end: String,
    year: String,
    date: String, // DA, the year's fallback
    publisher: String,
    city: String,
    url: String,
    doi: String,
    edition: String,
}

/// A line as a two-character tag and its value: `TAG  - value`, spacing around the `-`
/// tolerated; anything else is not a tag line.
fn tag_line(line: &str) -> Option<(&str, &str)> {
    let tag = line.get(..2)?;
    if !tag.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let rest = line.get(2..).unwrap_or("").trim_start();
    let value = rest.strip_prefix('-')?.trim();
    Some((tag, value))
}

/// The Word source kind for a `TY` value, case-insensitive.
fn kind_of(ty: &str) -> &'static str {
    match ty.to_ascii_uppercase().as_str() {
        "JOUR" => "article",
        "BOOK" => "book",
        "CHAP" => "bookSection",
        "CONF" => "conference",
        "RPRT" | "THES" => "report",
        "ELEC" | "EJOUR" | "WEBS" => "website",
        _ => "other",
    }
}

/// The first run of four digits, if the value holds one (RIS years are `2020` or `2020/03/01`).
fn four_digits(v: &str) -> String {
    let mut run = String::new();
    for c in v.chars() {
        if c.is_ascii_digit() {
            run.push(c);
            if run.len() == 4 {
                return run;
            }
        } else {
            run.clear();
        }
    }
    String::new()
}

/// Store a tag's value; the first line wins when a tag repeats.
fn set_tag(r: &mut Rec, tag: &str, value: &str) {
    let set = |slot: &mut String| {
        if slot.is_empty() {
            *slot = value.to_string();
        }
    };
    match tag {
        "AU" if !value.is_empty() => r.authors.push(value.to_string()),
        "TI" | "T1" => set(&mut r.title),
        "T2" | "JO" | "JF" => set(&mut r.journal),
        "VL" => set(&mut r.volume),
        "IS" => set(&mut r.issue),
        "SP" => set(&mut r.start),
        "EP" => set(&mut r.end),
        "PY" => {
            if r.year.is_empty() {
                r.year = four_digits(value);
            }
        }
        "DA" => set(&mut r.date),
        "PB" => set(&mut r.publisher),
        "CY" => set(&mut r.city),
        "UR" => set(&mut r.url),
        "DO" => set(&mut r.doi),
        "ET" => set(&mut r.edition),
        _ => {}
    }
}

/// Turn a finished record into a source, if it holds one.
fn flush(rec: Option<Rec>, out: &mut Vec<Source>) {
    let Some(r) = rec else { return };
    if out.len() >= MAX_SOURCES {
        return;
    }
    let s = Source {
        kind: kind_of(&r.ty).into(),
        author: clamp_value(r.authors.join("; ")),
        title: clamp_value(r.title),
        journal: clamp_value(r.journal),
        volume: clamp_value(r.volume),
        issue: clamp_value(r.issue),
        pages: pages(&r.start, &r.end),
        year: clamp_value(if r.year.is_empty() { four_digits(&r.date) } else { r.year }),
        publisher: clamp_value(r.publisher),
        city: clamp_value(r.city),
        url: clamp_value(r.url),
        doi: clamp_value(r.doi),
        edition: clamp_value(r.edition),
        ..Default::default()
    };
    let empty = [&s.title, &s.journal, &s.volume, &s.pages, &s.year, &s.publisher, &s.city, &s.url, &s.doi, &s.edition, &s.issue];
    if s.author.is_empty() && empty.iter().all(|f| f.is_empty()) {
        return; // a TY with nothing else is not a source
    }
    out.push(s);
}

/// `SP`/`EP` as pages: both join with an en dash, either alone is the pages.
fn pages(start: &str, end: &str) -> String {
    clamp_value(match (start.is_empty(), end.is_empty()) {
        (false, false) => format!("{start}–{end}"),
        (false, true) => start.to_string(),
        (true, false) => end.to_string(),
        (true, true) => String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-written library exercising every mapped tag; the last record has no `ER`.
    const LIB: &str = "TY  - JOUR\n\
AU  - Doe, Jane\n\
AU  - Smith, John\n\
TI  - A study of things\n\
T2  - Journal of Things\n\
JO  - Journal of Things, second\n\
AB  - Abstracts are ignored.\n\
VL  - 12\n\
IS  - 3\n\
SP  - 100\n\
EP  - 110\n\
PY  - 2020\n\
PB  - Big Publisher\n\
CY  - Berlin\n\
UR  - https://example.com/a\n\
DO  - 10.1000/ris\n\
ER  - \n\
\n\
TY  - CHAP\n\
TI  - A chapter\n\
T2  - The Big Book\n\
AU  - Alpha, Bob\n\
ET  - 2\n\
PY  - 2019/05/01\n\
SP  - 5\n\
ER  - \n\
TY  - WEBS\n\
TI  - A page\n\
UR  - https://example.com/page\n\
DA  - 2018/03/04\n";

    /// A source with an empty tag (the engine generates one), for filling in field by field.
    fn src(kind: &str) -> Source {
        Source { kind: kind.into(), ..Default::default() }
    }

    #[test]
    fn golden_library_maps_every_tag() {
        let out = import(LIB.as_bytes()).unwrap();
        assert_eq!(out.len(), 3);

        let mut e = src("article");
        e.author = "Doe, Jane; Smith, John".into(); // one AU per line, joined with "; "
        e.title = "A study of things".into();
        e.journal = "Journal of Things".into(); // T2 wins, the later JO is ignored
        e.volume = "12".into();
        e.issue = "3".into();
        e.pages = "100–110".into(); // SP + EP with an en dash
        e.year = "2020".into();
        e.publisher = "Big Publisher".into();
        e.city = "Berlin".into();
        e.url = "https://example.com/a".into();
        e.doi = "10.1000/ris".into();
        assert_eq!(out[0], e);

        let mut e = src("bookSection");
        e.title = "A chapter".into();
        e.journal = "The Big Book".into();
        e.author = "Alpha, Bob".into();
        e.edition = "2".into();
        e.year = "2019".into(); // 4-digit year out of "2019/05/01"
        e.pages = "5".into(); // SP alone
        assert_eq!(out[1], e);

        let mut e = src("website"); // WEBS, no ER: flushed at end of file
        e.title = "A page".into();
        e.url = "https://example.com/page".into();
        e.year = "2018".into(); // DA fallback, PY absent
        assert_eq!(out[2], e);
    }

    #[test]
    fn every_type_maps_to_a_kind() {
        let tys = ["JOUR", "BOOK", "CHAP", "CONF", "RPRT", "THES", "ELEC", "EJOUR", "WEBS", "NEWS", "ABST"];
        let kinds = ["article", "book", "bookSection", "conference", "report", "report", "website", "website", "website", "other", "other"];
        let lib = tys.iter().map(|t| format!("TY  - {t}\nTI  - t\nER  - \n")).collect::<String>();
        let out = import(lib.as_bytes()).unwrap();
        let got: Vec<&str> = out.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(got, kinds);
        // Types are matched without case.
        assert_eq!(import(b"TY  - jour\nTI  - t\nER  - \n").unwrap()[0].kind, "article");
    }

    #[test]
    fn latin1_bytes_and_a_bom_decode() {
        // 0xFC/0xE4 are invalid UTF-8: the file decodes byte-preserving as latin-1.
        let lib = b"TY  - BOOK\r\nAU  - M\xfcller, Hans\r\nTI  - Ein B\xfcch\r\nER  - \r\n";
        let out = import(lib).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].author, "Müller, Hans");
        assert_eq!(out[0].title, "Ein Büch");
        // A UTF-8 BOM does not break the first tag.
        let bom = b"\xEF\xBB\xBF";
        let mut v = bom.to_vec();
        v.extend_from_slice(b"TY  - JOUR\nTI  - t\nER  - \n");
        assert_eq!(import(&v).unwrap().len(), 1);
    }

    #[test]
    fn a_record_starts_at_ty() {
        // Tags before the first TY belong to no record and are dropped.
        let out = import(b"AU  - Nobody\nTI  - stray\nTY  - JOUR\nTI  - kept\nER  - \n").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "kept");
        assert_eq!(out[0].author, "");
        // A TY with nothing else carries no source.
        assert_eq!(import(b"TY  - JOUR\nER  - \n").unwrap_err(), "no RIS records found in the file");
    }

    #[test]
    fn values_are_cut_off_at_4000_characters() {
        let at = format!("TY  - JOUR\nTI  - {}\nER  - \n", "a".repeat(4_000));
        assert_eq!(import(at.as_bytes()).unwrap()[0].title.chars().count(), 4_000);
        let over = format!("TY  - JOUR\nTI  - {}\nER  - \n", "a".repeat(4_050));
        assert_eq!(import(over.as_bytes()).unwrap()[0].title.chars().count(), 4_000);
    }

    #[test]
    fn records_are_capped_at_10000() {
        let rec = "TY  - JOUR\nTI  - t\nER  - \n";
        let lib = rec.repeat(10_002);
        let out = import(lib.as_bytes()).unwrap();
        assert_eq!(out.len(), 10_000);
    }

    #[test]
    fn input_over_16mib_is_refused() {
        let err = import(&vec![b'x'; (16 << 20) + 1]).unwrap_err();
        assert!(err.contains("16 MiB"), "{err}");
    }

    #[test]
    fn junk_never_panics() {
        for junk in [&b""[..], b"not ris at all", b"XX no dash", b"TY", b"TY  - \nno close", b"\x00\x01\x02", &(0..=255u8).collect::<Vec<u8>>()] {
            let _ = import(junk);
        }
        assert_eq!(import(b"").unwrap_err(), "no RIS records found in the file");
        assert!(import(b"hello world\n").is_err());
        // Odd spacing still reads as a tag line.
        assert_eq!(import(b"TY-JOUR\nTI - t\nER-\n").unwrap()[0].title, "t");
    }
}
