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
