use super::{FieldCode, InlineObject, NumFormat, Pos};

/// The summary property a TITLE, SUBJECT, AUTHOR, KEYWORDS or COMMENTS field shows.
pub(super) fn summary_property<'a>(core: &'a wordcraft_doc::CoreProps, field: &str) -> &'a str {
    match field {
        "TITLE" => &core.title,
        "SUBJECT" => &core.subject,
        "AUTHOR" => &core.creator,
        "KEYWORDS" => &core.keywords,
        "COMMENTS" => &core.description,
        _ => "",
    }
}

pub(super) fn set_summary_property(core: &mut wordcraft_doc::CoreProps, field: &str, value: String) {
    match field {
        "TITLE" => core.title = value,
        "SUBJECT" => core.subject = value,
        "AUTHOR" => core.creator = value,
        "KEYWORDS" => core.keywords = value,
        "COMMENTS" => core.description = value,
        _ => {}
    }
}

/// The new value a summary-property field gives its property: the argument after the field's
/// name (`SUBJECT "Quarterly report"`, or one word without quotes: `SUBJECT Draft`), at most 255
/// characters as Word allows.
pub(super) fn new_property_value(instr: &str) -> Option<String> {
    FieldCode::parse(instr).args.first().map(|a| a.chars().take(255).collect())
}

/// A count (NUMWORDS, NUMCHARS, REVNUM) in the field's `\*` number format, digits without one or
/// when the format does not cover the count (see [`super::table_formula::number_format_covers`]),
/// with the field's capitalization switches.
pub(super) fn count_text(instr: &str, n: usize) -> String {
    let fmt = wordcraft_layout::fields::format_switch(instr)
        .filter(|f| crate::cmd::table_formula::number_format_covers(*f, n as f64))
        .unwrap_or(NumFormat::Decimal);
    wordcraft_layout::fields::field_number(instr, fmt, u32::try_from(n).unwrap_or(u32::MAX))
}

/// A field's computed text with its capitalization switches (`\* Upper`, `\* Lower`, `\* Caps`,
/// `\* FirstCap`) applied in order.
pub(super) fn text_result(instr: &str, text: String) -> String {
    wordcraft_layout::fields::apply_case_switch(instr, text)
}

/// What Word's F9 shows for an `IF` whose operator isn't one it knows (`IF 1=1 …` reads `1=1` as
/// the first expression).
pub const IF_UNKNOWN_OP: &str = "Error! Unknown op code for conditional.";

/// What an `IF` field shows when updated (ECMA-376 §17.16.5.30), as measured in Word for Windows:
/// `IF expression1 operator expression2 "true text" "false text"`, the operator one of `=`, `<>`,
/// `<`, `>`, `<=`, `>=` standing apart. Two unquoted numbers compare as numbers (`2 > 10` is
/// false, `1,000 = 1000` true); anything else as text, case-sensitively for `=` and `<>`, and in
/// word order otherwise, lowercase before uppercase (`"apple" < "banana"`, `"z" < "Z"`); with
/// `=` and `<>` the second expression may hold `?` (any one character) and `*` (any run). A
/// missing text is empty. `None` for a code holding a field written out in braces
/// (`IF { PAGE } = 1 …`, as mail merge rules are), which isn't evaluated here: it keeps its
/// result.
pub(super) fn if_result(instr: &str) -> Option<String> {
    let words = if_words(instr);
    if words.iter().any(|(w, quoted)| !quoted && w.contains(['{', '}'])) {
        return None;
    }
    let [left, op, right, rest @ ..] = words.as_slice() else { return Some(String::new()) };
    let ord = match (if_number(left), if_number(right)) {
        (Some(a), Some(b)) => a.partial_cmp(&b),
        _ => None,
    };
    use std::cmp::Ordering::{Equal, Greater, Less};
    let pattern = right.0.contains(['*', '?']);
    let equal = || ord.map_or_else(|| if pattern { wildcard_match(&left.0, &right.0) } else { left.0 == right.0 }, |o| o == Equal);
    let order = || ord.unwrap_or_else(|| word_order(&left.0, &right.0));
    let holds = match op.0.as_str() {
        "=" => equal(),
        "<>" => !equal(),
        "<" => order() == Less,
        ">" => order() == Greater,
        "<=" => order() != Greater,
        ">=" => order() != Less,
        _ => return Some(IF_UNKNOWN_OP.to_string()),
    };
    Some(rest.get(if holds { 0 } else { 1 }).map(|w| w.0.clone()).unwrap_or_default())
}

/// The words of an `IF` code after its name and before its first switch: each a quoted text (the
/// quotes dropped, `true`) or a run of non-blank characters.
fn if_words(instr: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut chars = instr.trim_start().chars().skip_while(|c| !c.is_whitespace()).peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            out.push((chars.by_ref().take_while(|&c| c != '"').collect(), true));
        } else if c == '\\' {
            break;
        } else {
            let mut w = String::new();
            while let Some(&c) = chars.peek().filter(|c| !c.is_whitespace()) {
                w.push(c);
                chars.next();
            }
            out.push((w, false));
        }
    }
    out
}

/// An unquoted `IF` expression as a number (thousands separators allowed).
fn if_number((text, quoted): &(String, bool)) -> Option<f64> {
    if *quoted {
        return None;
    }
    text.replace(',', "").parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Word's text order: letters compared regardless of case first, then lowercase before uppercase.
fn word_order(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| {
        a.chars()
            .zip(b.chars())
            .find(|(x, y)| x != y)
            .map_or(std::cmp::Ordering::Equal, |(x, _)| if x.is_lowercase() { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater })
    })
}

/// Does `text` match `pattern`, `?` standing for any one character and `*` for any run?
fn wildcard_match(text: &str, pattern: &str) -> bool {
    let (t, p): (Vec<char>, Vec<char>) = (text.chars().collect(), pattern.chars().collect());
    // The last `*` seen and where the text stood then, to retry from.
    let (mut ti, mut pi, mut star) = (0usize, 0usize, None::<(usize, usize)>);
    while ti < t.len() {
        match p.get(pi) {
            Some('*') => {
                star = Some((pi, ti));
                pi += 1;
            }
            Some(&c) if c == '?' || c == t[ti] => {
                ti += 1;
                pi += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    pi = sp + 1;
                    ti = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    p[pi.min(p.len())..].iter().all(|&c| c == '*')
}

/// The property name of a `DOCPROPERTY` field: the word (or quoted text) after the field name,
/// before any switch (`DOCPROPERTY "Project Name" \* MERGEFORMAT`).
pub(super) fn doc_property_arg(instr: &str) -> Option<String> {
    let rest = instr.trim().split_once(char::is_whitespace)?.1.trim_start();
    let name = match rest.strip_prefix('"') {
        Some(q) => q.split('"').next().unwrap_or(""),
        None => rest.split([' ', '\\']).next().unwrap_or(""),
    };
    (!name.is_empty()).then(|| name.to_string())
}

/// The value of a document property under the name a `DOCPROPERTY` field uses: a custom property
/// when one has this name, else the built-in property (English names, case-insensitively, as
/// Word's field accepts) from `core`. `None` for a name this application doesn't know.
pub(super) fn doc_property(doc: &wordcraft_doc::Document, core: &wordcraft_doc::CoreProps, name: &str) -> Option<String> {
    if let Some(v) = doc.custom_prop(name) {
        return Some(v.to_string());
    }
    let c = core;
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "title" => c.title.clone(),
        "subject" => c.subject.clone(),
        "author" | "creator" => c.creator.clone(),
        "keywords" => c.keywords.clone(),
        "comments" | "description" => c.description.clone(),
        "category" => c.category.clone(),
        "lastmodifiedby" | "last modified by" | "lastsavedby" | "last saved by" => c.last_modified_by.clone(),
        "revisionnumber" | "revision number" | "revision" => c.revision.to_string(),
        _ => return None,
    })
}

/// A field's `\@` date picture: the quoted text after it (switches may follow), or one word.
pub fn date_picture(instr: &str) -> Option<String> {
    let rest = instr.split_once("\\@")?.1.trim_start();
    let pic = match rest.strip_prefix('"') {
        Some(q) => q.split('"').next().unwrap_or(""),
        None => rest.split_whitespace().next().unwrap_or(""),
    };
    (!pic.is_empty()).then(|| pic.to_string())
}

/// The text a bookmark marks (field results included), up to its end in the same paragraph or
/// the paragraph's end. `None` when there is no such bookmark.
pub fn bookmark_text(doc: &wordcraft_doc::Document, name: &str) -> Option<String> {
    let (_, start) = doc.bookmarks().into_iter().find(|(n, _)| n == name)?;
    let para = doc.para(start.story, &start.path)?;
    let end = para
        .object_offsets()
        .into_iter()
        .find(|o| *o > start.off && matches!(para.object_at(*o), Some(InlineObject::BookmarkEnd { name: n }) if n == name))
        .unwrap_or(para.len());
    Some(doc.copy_range(&start, &Pos { off: end, ..start.clone() }).plain_text().trim().to_string())
}
