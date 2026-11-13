//! A document opened from RTF lays out its lines the way Word does for RTF
//! (`Settings::rtf_justification`, see the layout crate's `para::justify`): the reader sets it for
//! every RTF file that does not say `\jexpand` (see the `jexpand` tests); nothing else sets it.

use super::*;
use wordcraft_doc::Document;

const PLAIN: &str = r"{\rtf1\ansi\deff0{\fonttbl{\f0 Courier New;}}\pard\qj\f0\fs24 Some justified text.\par}";
const PRINTER_METRICS: &str =
    r"{\rtf1\ansi\deff0{\fonttbl{\f0 Courier New;}}\notabind\wraptrsp\nocolbal\sprslnsp\lytprtmet\pard\qj\f0\fs24\expnd0\expndtw-3 Text.\par}";

#[test]
fn every_rtf_document_lays_out_with_rtf_justification() {
    for rtf in [PLAIN, PRINTER_METRICS, r"{\rtf1 Hello}"] {
        assert!(import(rtf.as_bytes()).unwrap().settings.rtf_justification, "{rtf}");
    }
}

#[test]
fn a_new_document_does_not() {
    assert!(!Document::new().settings.rtf_justification);
}

#[test]
fn nothing_is_written_for_it_and_it_comes_back_on_reopening() {
    let d = import(PLAIN.as_bytes()).unwrap();
    let out = export(&d);
    assert!(!out.contains("\\jexpand") && !out.contains("\\jcompress"), "{out}");
    assert!(import(out.as_bytes()).unwrap().settings.rtf_justification);
}
