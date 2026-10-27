use super::*;

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
