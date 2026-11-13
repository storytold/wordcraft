//! `\wraptrsp` (Wrap trailing spaces onto the next line) is read into
//! `Settings::wrap_trail_spaces` and written back once; it changes how Word fits justified lines
//! of an RTF document (the layout crate's `rtf_wraptrsp_squeeze` tests).

use super::*;
use wordcraft_doc::Document;

const WITH: &str = r"{\rtf1\ansi\deff0{\fonttbl{\f0 Courier New;}}\notabind\wraptrsp\nocolbal\sprslnsp\lytprtmet\pard\qj\f0\fs24 Text.\par}";
const ALONE: &str = r"{\rtf1\ansi\deff0{\fonttbl{\f0 Courier New;}}\wraptrsp\pard\qj\f0\fs24 Text.\par}";
const WITHOUT: &str = r"{\rtf1\ansi\deff0{\fonttbl{\f0 Courier New;}}\notabind\nocolbal\pard\qj\f0\fs24 Text.\par}";

#[test]
fn wraptrsp_sets_wrap_trail_spaces() {
    for rtf in [WITH, ALONE] {
        assert!(import(rtf.as_bytes()).unwrap().settings.wrap_trail_spaces, "{rtf}");
    }
    assert!(!import(WITHOUT.as_bytes()).unwrap().settings.wrap_trail_spaces);
    assert!(!Document::new().settings.wrap_trail_spaces);
}

#[test]
fn it_is_written_back_once_and_reads_again() {
    let d = import(ALONE.as_bytes()).unwrap();
    let out = export(&d);
    assert_eq!(out.matches("\\wraptrsp").count(), 1, "{out}");
    assert!(import(out.as_bytes()).unwrap().settings.wrap_trail_spaces);
    let mut off = d.clone();
    off.settings.wrap_trail_spaces = false;
    assert!(!export(&off).contains("\\wraptrsp"));
}
