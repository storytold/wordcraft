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

/// The sender fields (§7.3.6 `text:sender-firstname` … §7.3.7 `text:sender-state-or-province`).
const SENDER_PARTS: [&str; 18] = [
    "firstname",
    "lastname",
    "initials",
    "title",
    "position",
    "email",
    "phone-private",
    "fax",
    "company",
    "phone-work",
    "street",
    "city",
    "postal-code",
    "country",
    "state-or-province",
    "country-code",
    "street-number",
    "address",
];

/// `el`, a field element, fixed at what it shows (§19.807 `text:fixed`), for the fields that can
/// be.
/// Field element `el` for locked field `instr` showing `result`, fixed (see [`fixed`]). A date
/// or time is fixed at the value its text shows under its picture (§19.871, §19.880); `None`
/// when that can't be read, as a reader shows a fixed date without a value as today's (then it
/// is written as its text).
pub(super) fn locked(el: &str, instr: &str, result: &str) -> Option<String> {
    let code = instr.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    let attr = match code.as_str() {
        "DATE" => "text:date-value",
        "TIME" => "text:time-value",
        _ => return Some(fixed(el)),
    };
    let value = super::datastyle::picture_of(instr).and_then(|pic| super::datastyle::value_shown(&pic, result))?;
    let el = fixed(el);
    let (tag, rest) = el.find(['>', ' ']).and_then(|at| Some((el.get(..at)?, el.get(at..)?)))?;
    Some(format!("{tag} {attr}=\"{value}\"{rest}"))
}

pub(super) fn fixed(el: &str) -> String {
    with_fixed(el, true)
}

/// `el`, a field element, an author field said not to be fixed (§19.807's default, said:
/// LibreOffice reads an author field without `text:fixed` as fixed, showing what it holds, and
/// writes `false` itself). Other fields are written as LibreOffice writes them, without it.
pub(super) fn unfixed(el: &str) -> String {
    if el.starts_with("<text:author-") { with_fixed(el, false) } else { el.to_string() }
}

fn with_fixed(el: &str, fixed: bool) -> String {
    const FIXABLE: [&str; 6] = ["<text:date", "<text:time", "<text:author-name", "<text:author-initials", "<text:sender-", "<text:file-name"];
    if !FIXABLE.iter().any(|f| el.starts_with(f)) {
        return el.to_string();
    }
    match el.find(['>', ' ']).and_then(|at| Some((el.get(..at)?, el.get(at..)?))) {
        Some((tag, rest)) => format!("{tag} text:fixed=\"{fixed}\"{rest}"),
        None => el.to_string(),
    }
}

/// The Word field code for the `text:` field element `local` with attributes `a`, or `None`
/// when it is no field WordCraft keeps (then its text is read as text).
pub(super) fn instr_of(local: &str, a: &Attrs, picture: impl Fn(&str) -> Option<String>) -> Option<String> {
    // Fixed, unless it holds no value to be fixed at (LibreOffice's templates: a date that shows
    // the day the document is made from them).
    // A name a field code can hold as one word.
    let name = |k: &str| {
        get(a, k)
            .map(str::trim)
            .filter(|n| !n.is_empty() && n.len() <= MAX_REF_NAME && !n.contains(|c: char| c.is_whitespace() || c == '"' || c == '\\'))
    };
    match local {
        // A variable's value (§7.4.5, §7.4.6): what a SET gave its name.
        "variable-get" | "variable-set" => Some(format!("REF {}", name("text:name")?)),
        // A document property's value (§7.5.6 `text:user-defined`, a `meta:user-defined`) or a
        // user field's (§7.4.9 `text:user-field-get`, declared once for the document): Word's
        // `DOCPROPERTY`, the property kept as a custom document property.
        "user-defined" | "user-field-get" => {
            let n = clean_prop_name(get(a, "text:name")?)?;
            Some(format!("DOCPROPERTY \"{n}\""))
        }
        // A citation (§8.1.24 `text:bibliography-mark`): Word's `CITATION` of the source it
        // carries (see [`bib_source`]).
        "bibliography-mark" => bib_tag(get(a, "text:identifier")?).map(|t| format!("CITATION {t}")),
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
        // The sender's details (§7.3.6–7.3.7), from the application's user data: WordCraft's
        // own `SENDER` field code, shown as its last result.
        _ if local.strip_prefix("sender-").is_some_and(|p| SENDER_PARTS.contains(&p)) => local.strip_prefix("sender-").map(|p| format!("SENDER {p}")),
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
        "SENDER" => {
            let part = words.next().filter(|p| SENDER_PARTS.contains(p))?;
            Some(format!("<text:sender-{part}>{result}</text:sender-{part}>"))
        }
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
            let target = instr.trim_start().get(code.len()..)?.trim_start();
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
            let entry = instr.trim_start().get(code.len()..)?.trim_start();
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

/// A custom property's name as a field code can quote it: no quotes or backslashes, no control
/// characters, at most 255 characters. `None` when nothing is left.
pub(super) fn clean_prop_name(n: &str) -> Option<String> {
    let n: String = n.chars().filter(|c| !matches!(c, '"' | '\\') && !c.is_control()).take(255).collect();
    let n = n.trim();
    (!n.is_empty()).then(|| n.to_string())
}

/// The property a `DOCPROPERTY` field code names, if it is one.
pub(super) fn docproperty_name(instr: &str) -> Option<&str> {
    let rest = instr.trim_start().strip_prefix("DOCPROPERTY")?.trim_start();
    match rest.strip_prefix('"') {
        Some(q) => q.split('"').next(),
        None => rest.split_whitespace().next(),
    }
    .map(str::trim)
    .filter(|n| !n.is_empty() && !n.starts_with('\\'))
}

/// Most custom properties read from a document.
pub(super) const MAX_PROPS: usize = 1000;
/// Longest custom property value read, characters.
const MAX_PROP_VALUE: usize = 4096;

/// A custom property of ODF value type `t` (`office:value-type`, `meta:value-type`) and value
/// `v`: as a document's custom property (OOXML variant type, value).
pub(super) fn custom_prop(name: &str, t: Option<&str>, v: &str) -> Option<wordcraft_doc::CustomProp> {
    let name = clean_prop_name(name)?;
    let kind = match t.map(str::trim) {
        Some("float" | "percentage" | "currency") => "r8",
        Some("boolean") => "bool",
        Some("date") => "filetime",
        _ => "lpwstr",
    };
    let mut value: String = v.chars().filter(|c| !c.is_control() || *c == '\n').take(MAX_PROP_VALUE).collect();
    let mut kind = kind;
    if kind == "filetime" {
        // A date and time in UTC, as Word's `filetime` holds it: a bare date at midnight; one
        // that isn't a date is kept as text.
        let v = value.trim();
        let date_ok = v.len() >= 10
            && v.is_char_boundary(10)
            && v.get(..10).is_some_and(|d| d.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() }));
        value = match (date_ok, v.get(10..)) {
            (true, Some("")) => format!("{v}T00:00:00Z"),
            (true, Some(rest)) if rest.ends_with('Z') || rest.contains('+') || rest.rfind('-').is_some() => v.to_string(),
            (true, Some(_)) => format!("{v}Z"),
            _ => {
                kind = "lpwstr";
                v.to_string()
            }
        };
    }
    Some(wordcraft_doc::CustomProp { name, kind: kind.into(), value, link_target: None, odf_user_field: false })
}

/// The custom properties of a package's `meta.xml` (§4.3.2.14 `meta:user-defined`).
pub(super) fn meta_props(xml: &[u8]) -> Vec<wordcraft_doc::CustomProp> {
    let mut out: Vec<wordcraft_doc::CustomProp> = Vec::new();
    let mut open: Option<(String, Option<String>, String)> = None;
    super::events(xml, |ev| match ev {
        super::Ev::Start(q, a) if q == "meta:user-defined" => {
            open = get(&a, "meta:name").map(|n| (n.to_string(), get(&a, "meta:value-type").map(String::from), String::new()));
        }
        super::Ev::Text(t) => {
            if let Some((_, _, v)) = &mut open
                && v.len() < MAX_PROP_VALUE * 4
            {
                v.push_str(&t);
            }
        }
        super::Ev::End(q) if q == "meta:user-defined" => {
            if let Some((n, t, v)) = open.take()
                && out.len() < MAX_PROPS
                && let Some(p) = custom_prop(&n, t.as_deref(), &v)
                && !out.iter().any(|o| o.name == p.name)
            {
                out.push(p);
            }
        }
        _ => {}
    });
    out
}

/// The user fields a package's `content.xml` declares (§7.4.7 `text:user-field-decl`), as custom
/// properties.
pub(super) fn user_fields(xml: &[u8]) -> Vec<wordcraft_doc::CustomProp> {
    let mut out: Vec<wordcraft_doc::CustomProp> = Vec::new();
    super::events(xml, |ev| {
        if let super::Ev::Start(q, a) = ev
            && q == "text:user-field-decl"
            && out.len() < MAX_PROPS
        {
            let t = get(&a, "office:value-type");
            let v = match t.map(str::trim) {
                Some("float" | "percentage" | "currency") => get(&a, "office:value"),
                Some("boolean") => get(&a, "office:boolean-value"),
                Some("date") => get(&a, "office:date-value"),
                _ => get(&a, "office:string-value"),
            }
            .unwrap_or("");
            if let Some(p) = get(&a, "text:name").and_then(|n| custom_prop(n, t, v))
                && !out.iter().any(|o| o.name == p.name)
            {
                out.push(wordcraft_doc::CustomProp { odf_user_field: true, ..p });
            }
        }
    });
    out
}

/// A source's tag as a field code names it: one word (spaces made underscores), at most 64
/// characters, no quotes or backslashes. `None` when nothing is left.
pub(super) fn bib_tag(id: &str) -> Option<String> {
    let t: String =
        id.trim().chars().filter(|c| !matches!(c, '"' | '\\') && !c.is_control()).map(|c| if c.is_whitespace() { '_' } else { c }).take(64).collect();
    (!t.is_empty()).then_some(t)
}

/// ODF bibliography types (§19.817 `text:bibliography-type`) and WordCraft's source kinds.
const BIB_KINDS: [(&str, &str); 10] = [
    ("book", "book"),
    ("inbook", "bookSection"),
    ("incollection", "bookSection"),
    ("article", "article"),
    ("journal", "periodical"),
    ("inproceedings", "conference"),
    ("proceedings", "conference"),
    ("conference", "conference"),
    ("techreport", "report"),
    ("www", "website"),
];

/// The source a `text:bibliography-mark` carries: its identifier as the tag, its type as the
/// kind, and its author, title, year, publisher, address, journal, volume, pages, URL, editor,
/// edition, number (issue) and month. `None` without an identifier.
pub(super) fn bib_source(a: &Attrs) -> Option<wordcraft_doc::Source> {
    let tag = bib_tag(get(a, "text:identifier")?)?;
    let v = |k: &str| get(a, k).map(|s| s.chars().filter(|c| !c.is_control()).take(1024).collect::<String>()).unwrap_or_default();
    let t = get(a, "text:bibliography-type").map(str::trim).unwrap_or("");
    let kind = BIB_KINDS.iter().find(|(o, _)| *o == t).map_or("other", |(_, k)| k).to_string();
    Some(wordcraft_doc::Source {
        tag,
        kind,
        author: v("text:author"),
        title: v("text:title"),
        year: v("text:year"),
        publisher: v("text:publisher"),
        city: v("text:address"),
        journal: v("text:journal"),
        volume: v("text:volume"),
        pages: v("text:pages"),
        url: v("text:url"),
        editor: v("text:editor"),
        edition: v("text:edition"),
        issue: v("text:number"),
        month: v("text:month"),
        ..Default::default()
    })
}

/// A source as a `text:bibliography-mark` around `result` (already escaped).
pub(super) fn bib_mark(s: &wordcraft_doc::Source, result: &str) -> String {
    let t = BIB_KINDS.iter().find(|(_, k)| *k == s.kind).map_or("misc", |(o, _)| o);
    let author = if s.corporate.is_empty() { &s.author } else { &s.corporate };
    let mut a = format!(" text:identifier=\"{}\" text:bibliography-type=\"{t}\"", x(&s.tag));
    for (k, v) in [
        ("author", author),
        ("title", &s.title),
        ("year", &s.year),
        ("publisher", &s.publisher),
        ("address", &s.city),
        ("journal", &s.journal),
        ("volume", &s.volume),
        ("pages", &s.pages),
        ("url", &s.url),
        ("editor", &s.editor),
        ("edition", &s.edition),
        ("number", &s.issue),
        ("month", &s.month),
    ] {
        if !v.is_empty() {
            a.push_str(&format!(" text:{k}=\"{}\"", x(v)));
        }
    }
    format!("<text:bibliography-mark{a}>{result}</text:bibliography-mark>")
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

/// The SET field (ECMA-376 §17.16.5.58) a `text:variable-set` with attributes `a` is: its name and
/// value (§19.383 `office:string-value`, else the number, else the formula without its prefix).
pub(super) fn set_instr(a: &Attrs) -> Option<String> {
    let name = get(a, "text:name")
        .map(str::trim)
        .filter(|n| !n.is_empty() && n.len() <= MAX_REF_NAME && !n.contains(|c: char| c.is_whitespace() || c == '"' || c == '\\'))?;
    let value = get(a, "office:string-value")
        .or_else(|| get(a, "office:value"))
        .or_else(|| get(a, "text:formula").map(|f| f.trim().strip_prefix("ooow:").unwrap_or(f.trim())))
        .unwrap_or_default();
    let value: String = value.chars().take(MAX_FIELD_VALUE).collect();
    Some(format!("SET {name} \"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")))
}

/// Most characters of a variable's value kept.
const MAX_FIELD_VALUE: usize = 10_000;

/// A SET field's name and value, when `instr` is one.
pub(super) fn set_of(instr: &str) -> Option<(&str, String)> {
    let rest = instr.trim_start().strip_prefix("SET")?.strip_prefix(char::is_whitespace)?.trim_start();
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let (name, value) = (rest.get(..end)?, rest.get(end..)?.trim());
    if name.is_empty() {
        return None;
    }
    // A quoted value, its \" and \\ undone; else the words as they are.
    let value = match value.strip_prefix('"') {
        Some(q) => {
            let mut out = String::new();
            let mut it = q.chars();
            while let Some(c) = it.next() {
                match c {
                    '"' => break,
                    '\\' => out.extend(it.next()),
                    c => out.push(c),
                }
            }
            out
        }
        None => value.to_string(),
    };
    Some((name, value))
}

#[cfg(test)]
mod tests {
    /// Word writes field codes with a leading space (` STYLEREF 2 `); the argument is read after
    /// the code all the same.
    #[test]
    fn a_field_code_with_leading_space_keeps_its_argument() {
        let chapter = super::element(" STYLEREF \"Heading 2\" ", "Intro", None).unwrap_or_default();
        assert!(chapter.contains("text:outline-level=\"2\""), "{chapter}");
        assert_eq!(super::element("STYLEREF 2", "Intro", None), super::element("  STYLEREF 2", "Intro", None));
        assert_eq!(super::element(" XE \"Apple\" ", "", None), super::element("XE \"Apple\"", "", None));
        assert!(super::element(" XE \"Apple\" ", "", None).is_some_and(|e| e.contains("Apple")));
    }
}
