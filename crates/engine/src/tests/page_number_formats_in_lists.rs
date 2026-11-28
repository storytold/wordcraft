//! Generated lists and page cross-references write a page as its footer shows it: in the number
//! format of the page's section (`w:pgNumType w:fmt`), as Word does (#3856). The front matter is
//! numbered i, ii; the body restarts at 1.

use serde_json::{Value, json};
use wordcraft_doc::StoryRef;
use wordcraft_doc::para::InlineObject;

use crate::Session;

fn run(s: &mut Session, id: &str, v: Value) {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"));
}

/// Page i: "Front"; page ii: "Preface" (Heading 1); next-page section break; page 1: "Chapter one"
/// (Heading 1). Section 1 lowerRoman from 1, section 2 decimal from 1.
fn two_sections() -> Session {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Front\nPreface\nChapter one"));
    for h in ["Preface", "Chapter one"] {
        run(&mut s, "select.text", json!({"text": h}));
        run(&mut s, "styles.apply", json!({"style": "Heading 1"}));
    }
    run(&mut s, "select.text", json!({"text": "Preface"}));
    run(&mut s, "caret.home", json!({}));
    run(&mut s, "layout.break", json!({"kind": "page"}));
    run(&mut s, "select.text", json!({"text": "Chapter one"}));
    run(&mut s, "caret.home", json!({}));
    run(&mut s, "layout.break", json!({"kind": "nextPage"}));
    run(&mut s, "select.text", json!({"text": "Chapter one"}));
    run(&mut s, "layout.pageNumberFormat", json!({"format": "decimal", "start": 1}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "layout.pageNumberFormat", json!({"format": "lowerRoman", "start": 1}));
    s
}

fn styled(s: &Session, prefix: &str) -> Vec<String> {
    s.doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .filter(|p| p.props.style.as_deref().is_some_and(|st| st.starts_with(prefix) && st != "TOCHeading"))
        .map(|p| p.plain_text())
        .collect()
}

fn field_results(s: &Session, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        for o in &s.doc.para(StoryRef::Body, &path).unwrap().objects {
            if let InlineObject::Field { instr, result, .. } = o
                && instr.trim_start().starts_with(name)
            {
                out.push(result.clone());
            }
        }
    }
    out
}

#[test]
fn toc_page_numbers_follow_the_section_format() {
    let mut s = two_sections();
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.toc", json!({"levels": 1, "hyperlinks": false, "rightAlign": false}));
    assert_eq!(styled(&s, "TOC1"), ["Preface ii", "Chapter one 1"]);
}

#[test]
fn toc_pageref_results_follow_the_section_format() {
    let mut s = two_sections();
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.toc", json!({"levels": 1, "hyperlinks": true, "rightAlign": false}));
    assert_eq!(field_results(&s, "PAGEREF"), ["ii", "1"]);
}

#[test]
fn toc_upper_roman_and_letters() {
    let mut s = two_sections();
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "layout.pageNumberFormat", json!({"format": "upperRoman", "start": 4}));
    run(&mut s, "select.text", json!({"text": "Chapter one"}));
    run(&mut s, "layout.pageNumberFormat", json!({"format": "lowerLetter", "start": 3}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.toc", json!({"levels": 1, "hyperlinks": false, "rightAlign": false}));
    // Preface is the section's 2nd page: 4 + 1 = 5 = V; Chapter one starts at 3 = c.
    assert_eq!(styled(&s, "TOC1"), ["Preface V", "Chapter one c"]);
}

#[test]
fn index_pages_follow_the_section_format() {
    let mut s = two_sections();
    run(&mut s, "select.text", json!({"text": "Preface"}));
    run(&mut s, "references.markEntry", json!({"entry": "Preface"}));
    run(&mut s, "select.text", json!({"text": "Chapter one"}));
    run(&mut s, "references.markEntry", json!({"entry": "Chapter"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "references.index", json!({"columns": 1, "rightAlign": false}));
    let text = s.doc.plain_text(StoryRef::Body);
    // The index is the last thing in the document: its lines start with the entry.
    let entry = |name: &str| text.lines().rfind(|l| l.starts_with(name)).unwrap_or_default().to_string();
    let preface = entry("Preface");
    assert!(preface.ends_with("ii"), "index line {preface:?} in {text:?}");
    let chapter = entry("Chapter");
    assert!(chapter.ends_with('1') && chapter != "Chapter one", "index line {chapter:?} in {text:?}");
}

#[test]
fn page_cross_reference_follows_the_section_format() {
    let mut s = two_sections();
    run(&mut s, "caret.docEnd", json!({}));
    // Before this change the field was inserted with "1", not even the decimal 2: find out which
    // page it reads, too.
    run(&mut s, "insert.crossReference", json!({"to": "heading", "target": "Preface", "show": "page"}));
    assert_eq!(field_results(&s, "PAGEREF"), ["ii"]);
}

#[test]
fn update_fields_still_agree() {
    // F9 already formats PAGEREF by the section; inserting must give the same as updating.
    let mut s = two_sections();
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.toc", json!({"levels": 1, "hyperlinks": true, "rightAlign": false}));
    let inserted = field_results(&s, "PAGEREF");
    crate::cmd::references::update_fields_in(&mut s, true).unwrap();
    assert_eq!(field_results(&s, "PAGEREF"), inserted);
}

// PR #5971 review: a decimal-only guard, `1, 1` across two sections, index page ranges and
// `_Ref` bookmark reuse.

/// "Front" / page break / "Preface" (Heading 1) / page break / "Chapter one" (Heading 1), one
/// decimal section: pages 1, 2, 3.
fn decimal_only() -> Session {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Front\nPreface\nChapter one"));
    for h in ["Preface", "Chapter one"] {
        run(&mut s, "select.text", json!({"text": h}));
        run(&mut s, "styles.apply", json!({"style": "Heading 1"}));
        run(&mut s, "caret.home", json!({}));
        run(&mut s, "layout.break", json!({"kind": "page"}));
    }
    s
}

/// The last line of the body's text that starts with `name` (the index is at the end).
fn index_line(s: &Session, name: &str) -> String {
    let text = s.doc.plain_text(StoryRef::Body);
    text.lines().rfind(|l| l.starts_with(name)).unwrap_or_default().to_string()
}

/// The body paragraph whose text contains `text`, and the byte offset where `text` starts in it.
fn find(s: &Session, text: &str) -> (wordcraft_doc::Path, usize) {
    s.doc
        .para_paths(StoryRef::Body)
        .into_iter()
        .find_map(|path| s.doc.para(StoryRef::Body, &path).and_then(|p| p.text.find(text)).map(|o| (path, o)))
        .unwrap_or_else(|| panic!("no paragraph with {text:?}"))
}

fn add_mark(s: &mut Session, path: &wordcraft_doc::Path, off: usize, obj: InlineObject) {
    let at = wordcraft_doc::Pos { story: StoryRef::Body, path: path.clone(), off };
    s.doc.insert_object(&at, obj, &wordcraft_doc::CharProps::default()).unwrap();
}

#[test]
fn decimal_only_documents_keep_plain_page_numbers() {
    let mut s = decimal_only();
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.toc", json!({"levels": 1, "hyperlinks": false, "rightAlign": false}));
    assert_eq!(styled(&s, "TOC1"), ["Preface 2", "Chapter one 3"]);

    let mut s = decimal_only();
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.toc", json!({"levels": 1, "hyperlinks": true, "rightAlign": false}));
    assert_eq!(field_results(&s, "PAGEREF"), ["2", "3"]);

    let mut s = decimal_only();
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "insert.crossReference", json!({"to": "heading", "target": "Preface", "show": "page"}));
    assert_eq!(field_results(&s, "PAGEREF"), ["2"]);

    let mut s = decimal_only();
    run(&mut s, "select.text", json!({"text": "Preface"}));
    run(&mut s, "references.markEntry", json!({"entry": "Preface"}));
    run(&mut s, "select.text", json!({"text": "Chapter one"}));
    run(&mut s, "references.markEntry", json!({"entry": "Chapter"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "references.index", json!({"columns": 1, "rightAlign": false}));
    let (preface, chapter) = (index_line(&s, "Preface"), index_line(&s, "Chapter"));
    assert!(preface.ends_with('2') && !preface.ends_with("12"), "{preface:?}");
    assert!(chapter.ends_with('3') && chapter != "Chapter one", "{chapter:?}");
}

/// The same entry on page 1 of two sections lists both pages: the index merges pages by layout
/// page, not by the number shown.
#[test]
fn the_same_label_on_two_pages_is_listed_twice() {
    let mut s = two_sections();
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "layout.pageNumberFormat", json!({"format": "decimal", "start": 1}));
    for t in ["Front", "Chapter one"] {
        run(&mut s, "select.text", json!({"text": t}));
        run(&mut s, "references.markEntry", json!({"entry": "Topic"}));
    }
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "references.index", json!({"columns": 1, "rightAlign": false}));
    let line = index_line(&s, "Topic");
    assert!(line.ends_with("1, 1"), "{line:?}");
}

/// An index entry for bookmark `Span` (`XE \r`), the bookmark from "Preface" (page ii) to `end`.
fn range_entry(end: Option<(&str, bool)>) -> String {
    let mut s = two_sections();
    let (path, off) = find(&s, "Preface");
    // The end first, so the start's offset stays put when both are in one paragraph.
    if let Some((text, at_end_of_para)) = end {
        let (epath, eoff) = find(&s, text);
        let eoff = if at_end_of_para { s.doc.para(StoryRef::Body, &epath).unwrap().len() } else { eoff + text.len() };
        add_mark(&mut s, &epath, eoff, InlineObject::BookmarkEnd { name: "Span".into() });
    }
    add_mark(&mut s, &path, off, InlineObject::BookmarkStart { name: "Span".into() });
    run(&mut s, "select.text", json!({"text": "Front"}));
    run(&mut s, "references.markEntry", json!({"entry": "Span", "bookmark": "Span"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "references.index", json!({"columns": 1, "rightAlign": false}));
    index_line(&s, "Span")
}

#[test]
fn an_index_range_across_sections_uses_each_ends_format() {
    let line = range_entry(Some(("Chapter one", true)));
    assert!(line.ends_with("ii\u{2013}1"), "{line:?}");
}

#[test]
fn an_index_range_on_one_page_is_one_page() {
    let line = range_entry(Some(("Preface", false)));
    assert!(line.ends_with("ii") && !line.contains('\u{2013}'), "{line:?}");
}

#[test]
fn an_index_range_without_its_end_is_the_start_page() {
    let line = range_entry(None);
    assert!(line.ends_with("ii") && !line.contains('\u{2013}'), "{line:?}");
}

/// A heading that starts with a page break and already has a `_Ref` bookmark around it from
/// offset 0: a page cross-reference reuses it and reads the heading's page.
#[test]
fn a_ref_bookmark_before_the_page_break_is_reused() {
    let mut s = two_sections();
    let (path, _) = find(&s, "Preface");
    let len = s.doc.para(StoryRef::Body, &path).unwrap().len();
    add_mark(&mut s, &path, len, InlineObject::BookmarkEnd { name: "_Ref000000007".into() });
    add_mark(&mut s, &path, 0, InlineObject::BookmarkStart { name: "_Ref000000007".into() });
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "insert.crossReference", json!({"to": "heading", "target": "Preface", "show": "page"}));
    let refs: Vec<String> = s.doc.bookmarks().into_iter().map(|b| b.0).filter(|n| n.starts_with("_Ref")).collect();
    assert_eq!(refs, ["_Ref000000007"], "no second _Ref bookmark");
    assert_eq!(field_results(&s, "PAGEREF"), ["ii"]);
}
