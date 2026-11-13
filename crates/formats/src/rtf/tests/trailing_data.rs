//! Bytes after the `}` that closes `{\rtf1 …}` are not part of the document (RTF 1.9.1, "RTF Syntax"):
//! Word ignores them, so they add no text, paragraphs or breaks.

use super::*;
use wordcraft_doc::StoryRef;

const DOC: &str = "{\\rtf1\\ansi{\\fonttbl{\\f0 Arial;}}\\f0 Hello\\par\n{\\b nested} world\\par\n}";

/// The body's paragraph count and text.
fn body(bytes: &[u8]) -> (usize, String) {
    let d = import(bytes).unwrap();
    (d.body.len(), d.plain_text(StoryRef::Body))
}

fn with_tail(tail: &[u8]) -> Vec<u8> {
    let mut v = DOC.as_bytes().to_vec();
    v.extend_from_slice(tail);
    v
}

#[test]
fn nothing_after_the_brace_still_imports_everything() {
    let (_, text) = body(DOC.as_bytes());
    assert!(text.contains("Hello"), "{text:?}");
    assert!(text.contains("nested world"), "{text:?}");
}

#[test]
fn text_after_the_closing_brace_is_ignored() {
    let want = body(DOC.as_bytes());
    let got = body(&with_tail(b"\nJUNK after end\n"));
    assert!(!got.1.contains("JUNK"), "{:?}", got.1);
    assert_eq!(got, want);
}

#[test]
fn raw_control_bytes_after_the_closing_brace_add_no_break() {
    // The real file's tail: replacement characters, control bytes and a 0x0E.
    let want = body(DOC.as_bytes());
    let got = body(&with_tail(b"\n\xef\xbf\xbd8\x11$DM\xef\xbf\xbd\x17q\xef\xbf\xbd|\xef\xbf\xbd\xef\xbf\xbd\x0e"));
    assert_eq!(got, want);
    assert!(!got.1.contains(['\u{c}', '\u{e}']), "{:?}", got.1);
}

#[test]
fn a_second_rtf_group_after_the_document_is_ignored() {
    let want = body(DOC.as_bytes());
    let got = body(&with_tail(b"{\\rtf1\\ansi more text\\par}"));
    assert!(!got.1.contains("more text"), "{:?}", got.1);
    assert_eq!(got, want);
}

// PR #5658 review: the document ends where its outer group first closes (RTF 1.9.1, "RTF
// Syntax": the file is one group); nothing after that brace is read, groups included.

#[test]
fn a_group_after_the_closing_brace_is_ignored() {
    let (_, text) = body(b"{\\rtf1\\ansi Hello}{\\b AFTER}");
    assert!(text.contains("Hello"), "{text:?}");
    assert!(!text.contains("AFTER"), "{text:?}");
    let want = body(DOC.as_bytes());
    assert_eq!(body(&with_tail(b"{\\b junk}")), want);
}

#[test]
fn a_stray_brace_ends_the_document_where_it_closes_the_outer_group() {
    let (_, text) = body(b"{\\rtf1\\ansi First\\par}}Second\\par}");
    assert!(text.contains("First"), "{text:?}");
    assert!(!text.contains("Second"), "{text:?}");
    let (_, text) = body(b"{\\rtf1\\ansi{\\b Bold}} Tail{\\i x}}");
    assert!(text.contains("Bold"), "{text:?}");
    assert!(!text.contains("Tail") && !text.contains('x'), "{text:?}");
}

#[test]
fn bin_data_is_skipped_by_its_count_as_the_parser_reads_it() {
    // Three raw `}` bytes inside \bin data are data, not the document's end.
    let (_, text) = body(b"{\\rtf1\\ansi Before {\\*\\blob\\bin3 }}}}After\\par}JUNK");
    assert!(text.contains("Before") && text.contains("After"), "{text:?}");
    assert!(!text.contains("JUNK"), "{text:?}");
    // The parser reads at most 10 digits: `\bin0000000000` skips nothing, `1 ` is text and the
    // next `}` closes the document.
    let (_, text) = body(b"{\\rtf1\\ansi A\\bin00000000001 }TAIL}");
    assert!(text.contains('A'), "{text:?}");
    assert!(!text.contains("TAIL"), "{text:?}");
    // `\binary` is another word, not `\bin`: it skips no bytes.
    let (_, text) = body(b"{\\rtf1\\ansi X\\binary{\\*\\y}} Z}");
    assert!(text.contains('X'), "{text:?}");
    assert!(!text.contains('Z'), "{text:?}");
}

#[test]
fn escaped_braces_and_backslashes_are_text_not_the_end() {
    let (_, text) = body(b"{\\rtf1\\ansi a\\{b\\}c\\\\d\\par}{\\b junk}");
    assert!(text.contains("a{b}c\\d"), "{text:?}");
    assert!(!text.contains("junk"), "{text:?}");
}

#[test]
fn control_words_after_the_closing_brace_add_nothing() {
    let want = body(DOC.as_bytes());
    assert_eq!(body(&with_tail(b"\\par\\page\\sect more\\par")), want);
}
