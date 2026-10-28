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
    bookmark_text_at(doc, name, &start)
}

/// Read a previously found bookmark from the current document, including updated field results.
pub(super) fn bookmark_text_at(doc: &wordcraft_doc::Document, name: &str, start: &Pos) -> Option<String> {
    BookmarkTextIndex::default().text_at(doc, name, start)
}

/// Marker locations shared by textual REF reads during one Update Fields call. Result writes
/// change object strings, not markers; no text or object value is retained in this index.
#[derive(Default)]
pub(super) struct BookmarkTextIndex {
    paragraphs: std::collections::HashMap<(wordcraft_doc::StoryRef, wordcraft_doc::Path), BookmarkParagraph>,
}

impl BookmarkTextIndex {
    pub(super) fn text_at(&mut self, doc: &wordcraft_doc::Document, name: &str, start: &Pos) -> Option<String> {
        let para = doc.para(start.story, &start.path)?;
        let index = self.paragraphs.entry((start.story, start.path.clone())).or_insert_with(|| BookmarkParagraph::new(para));
        let end = index.ends.get(name).and_then(|v| v.get(v.partition_point(|off| *off <= start.off))).copied().unwrap_or(para.len());
        // The existing fragment reader has specific fallback behavior for mismatched marker
        // storage. Keep that behavior rather than interpreting malformed objects differently.
        if index.offsets.len() != para.objects.len() {
            return Some(doc.copy_range(start, &Pos { off: end, ..start.clone() }).plain_text().trim().to_string());
        }
        let text = para.text.get(start.off..end)?;
        let mut object = index.offsets.partition_point(|off| *off < start.off);
        let mut out = String::new();
        for c in text.chars() {
            if c == wordcraft_doc::para::OBJ {
                if let Some(o) = para.objects.get(object) {
                    out.push_str(o.plain_text());
                }
                object += 1;
            } else {
                out.push(c);
            }
        }
        Some(out.trim().to_string())
    }
}

struct BookmarkParagraph {
    offsets: Vec<usize>,
    ends: std::collections::HashMap<String, Vec<usize>>,
}

impl BookmarkParagraph {
    fn new(para: &wordcraft_doc::Paragraph) -> Self {
        let offsets = para.object_offsets();
        let mut ends: std::collections::HashMap<String, Vec<usize>> = Default::default();
        for (&off, o) in offsets.iter().zip(&para.objects) {
            if let InlineObject::BookmarkEnd { name } = o {
                ends.entry(name.clone()).or_default().push(off);
            }
        }
        Self { offsets, ends }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::{Document, Paragraph, StoryRef, para::OBJ, para::Run, para_block, props::CharProps};

    fn paragraph(text: String, objects: Vec<InlineObject>) -> Paragraph {
        let mut p = Paragraph::new();
        p.text = text;
        p.runs = vec![Run { len: p.len(), props: CharProps::default() }];
        p.objects = objects;
        p
    }

    fn start(name: &str) -> InlineObject {
        InlineObject::BookmarkStart { name: name.into() }
    }

    fn end(name: &str) -> InlineObject {
        InlineObject::BookmarkEnd { name: name.into() }
    }

    #[test]
    fn bookmark_text_preserves_first_start_end_and_live_results() {
        let objects = vec![end("Target"), start("Target"), end("Other"), end("Target"), end("Target"), start("Target"), end("Target")];
        let mut d = Document::new();
        d.body = vec![
            para_block(paragraph(format!("{OBJ}é{OBJ}  α{OBJ}β  {OBJ}excluded{OBJ}{OBJ}duplicate{OBJ}"), objects)),
            para_block(paragraph(format!("{OBJ}later duplicate{OBJ}"), vec![start("Target"), end("Target")])),
        ];
        assert_eq!(bookmark_text(&d, "Target").as_deref(), Some("αβ"));
        assert_eq!(bookmark_text(&d, "target"), None, "names are case-sensitive");
        assert_eq!(bookmark_text(&d, "Absent"), None);
        let at = Pos::body(0, OBJ.len_utf8() + 'é'.len_utf8());
        assert_eq!(bookmark_text_at(&d, "Target", &at).as_deref(), Some("αβ"));
        // Text is read now, including fields whose result changed since start positions were found.
        d.body = vec![para_block(paragraph(
            format!("é{OBJ} {OBJ} {OBJ}tail"),
            vec![
                start("Target"),
                InlineObject::Field {
                    instr: "TITLE".into(),
                    result: "Original".into(),
                    locked: false,
                    dirty: false,
                    code: false,
                    form: String::new(),
                },
                end("Target"),
            ],
        ))];
        let at = Pos::body(0, 'é'.len_utf8());
        assert_eq!(bookmark_text_at(&d, "Target", &at).as_deref(), Some("Original"));
        if let InlineObject::Field { result, .. } = &mut d.para_mut(StoryRef::Body, &at.path).unwrap().objects[1] {
            *result = "Changed result".into();
        }
        assert_eq!(bookmark_text_at(&d, "Target", &at).as_deref(), Some("Changed result"));
    }

    #[test]
    fn bookmark_text_preserves_missing_end_and_unpaired_object_fallbacks() {
        let mut d = Document::new();
        d.body = vec![para_block(paragraph(format!("{OBJ}before{OBJ}  tail Ω  "), vec![end("Target"), start("Target")]))];
        assert_eq!(bookmark_text(&d, "Target").as_deref(), Some("tail Ω"), "an earlier end is not a closing end");
        d.body.push(para_block(paragraph(format!("next paragraph{OBJ}"), vec![end("Target")])));
        assert_eq!(bookmark_text(&d, "Target").as_deref(), Some("tail Ω"), "ends in later paragraphs are ignored");
        d.body = vec![para_block(paragraph(format!("{OBJ} empty {OBJ}{OBJ} tail"), vec![start("Target"), end("Other")]))];
        assert_eq!(bookmark_text(&d, "Target").as_deref(), Some("empty  tail"), "markers without objects do not invent ends");
        d.body = vec![para_block(paragraph(format!("{OBJ}tail"), vec![start("Target"), end("Target")]))];
        assert_eq!(bookmark_text(&d, "Target").as_deref(), Some("tail"), "objects without markers do not invent ends");
        d.body = vec![para_block(paragraph(format!("{OBJ}{OBJ} tail"), vec![start("Target"), end("Target")]))];
        assert_eq!(bookmark_text(&d, "Target").as_deref(), Some(""));
    }

    #[test]
    fn bookmark_text_end_lookup_scales_with_unrelated_markers() {
        let document = |n| {
            let mut objects = vec![start("Target")];
            objects.extend(std::iter::repeat_n(start("Other"), n));
            objects.push(end("Target"));
            let mut d = Document::new();
            d.body = vec![para_block(paragraph(format!("é{OBJ} α{}Ω {OBJ}tail", OBJ.to_string().repeat(n)), objects))];
            d
        };
        let (small, big) = (document(2_000), document(16_000));
        let at = Pos::body(0, 'é'.len_utf8());
        let check = |d| assert_eq!(bookmark_text_at(d, "Target", &at).as_deref(), Some("αΩ"));
        check(&small);
        check(&big);
        let measure = |d| {
            let cpu = || std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse::<u64>().ok();
            let mut total = 0_u64;
            let mut calls = 0;
            while total < 10_000_000 && calls < 128 {
                let before = cpu();
                check(d);
                let elapsed = match (before, cpu()) {
                    (Some(a), Some(b)) if b > a => b - a,
                    _ => return None,
                };
                total += elapsed;
                calls += 1;
            }
            Some(total as f64 * 1e-9 / f64::from(calls))
        };
        let (Some(a), Some(b)) = (measure(&small), measure(&big)) else {
            eprintln!("Bookmark text: CPU counter unavailable; semantic checks passed, timing assertion skipped");
            return;
        };
        eprintln!("Bookmark text: small={a:.9}s big={b:.9}s ratio={:.2}", b / a);
        assert!(b < a * 24.0 + 0.005, "8x markers must not take quadratic end lookup work: {a:.9}s -> {b:.9}s");
    }
    #[test]
    fn indexed_bookmark_text_matches_fragment_text_for_objects_and_malformed_storage() {
        let field =
            InlineObject::Field {
                instr: "TITLE".into(), result: "  field β  ".into(), locked: false, dirty: false, code: true, form: String::new()
            };
        let objects = vec![
            end("Target"),
            InlineObject::ControlStart { control: Box::default() },
            start("Target"),
            field,
            InlineObject::PositionalTab { align: Default::default(), relative_to: Default::default(), leader: Default::default() },
            InlineObject::Equation { linear: "x+y".into(), display: false, math: Default::default() },
            InlineObject::Ruby {
                base: "漢字".into(),
                ruby: "かんじ".into(),
                align: Default::default(),
                size: 6.0,
                raise: 8.0,
                base_size: 12.0,
                lang: "ja-JP".into(),
                props: Box::default(),
            },
            InlineObject::Opaque { format: "docx".into(), xml: String::new(), text: "opaque Ω".into() },
            InlineObject::ControlEnd,
            end("Target"),
            end("Target"),
            start("Target"),
            end("Target"),
            start("Other"),
        ];
        let text = format!("é{} tail λ", (0..objects.len()).map(|_| format!(" {OBJ} ψ")).collect::<String>());
        for mismatch in [0, 1, 2] {
            let mut p = paragraph(text.clone(), objects.clone());
            match mismatch {
                1 => p.objects.truncate(3),
                2 => p.objects.push(end("Extra")),
                _ => {}
            }
            let mut d = Document::new();
            d.body = vec![para_block(p)];
            let original = d.clone();
            let p = d.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
            let mut index = BookmarkTextIndex::default();
            for off in p.object_offsets().into_iter().chain([p.len()]) {
                let at = Pos::body(0, off);
                for name in ["Target", "Other", "Missing"] {
                    let end = p
                        .objects_with_offsets()
                        .find_map(|(o, v)| (o > off && matches!(v, InlineObject::BookmarkEnd { name: n } if n == name)).then_some(o))
                        .unwrap_or(p.len());
                    let expected = d.copy_range(&at, &Pos::body(0, end)).plain_text().trim().to_string();
                    assert_eq!(index.text_at(&d, name, &at), Some(expected), "mismatch={mismatch} off={off} name={name}");
                }
            }
            assert_eq!(d, original, "text extraction must not modify the source");
        }
    }

    #[test]
    fn indexed_bookmark_text_reads_changed_result_without_rebuilding_markers() {
        let mut d = Document::new();
        d.body = vec![para_block(paragraph(
            format!("{OBJ}{OBJ}{OBJ}"),
            vec![
                start("Target"),
                InlineObject::Field { instr: "TITLE".into(), result: "Old".into(), locked: false, dirty: true, code: false, form: String::new() },
                end("Target"),
            ],
        ))];
        let at = Pos::body(0, 0);
        let mut index = BookmarkTextIndex::default();
        assert_eq!(index.text_at(&d, "Target", &at).as_deref(), Some("Old"));
        if let InlineObject::Field { result, .. } = &mut d.para_mut(StoryRef::Body, &at.path).unwrap().objects[1] {
            *result = "Changed αβ".into();
        }
        assert_eq!(index.text_at(&d, "Target", &at).as_deref(), Some("Changed αβ"));
        assert_eq!(index.paragraphs.len(), 1);
        assert_eq!(index.text_at(&d, "Target", &Pos::body(1, 0)), None);
    }
}
