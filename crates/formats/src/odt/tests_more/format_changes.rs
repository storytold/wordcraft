//! Tracked formatting changes (ODF 1.3 §5.5.7 `text:format-change`, marked by
//! `text:change-start`/`-end`): the text keeps a formatting revision with its author and date.
//! ODF records no old formatting, so the old is the text's own (accepting keeps it, rejecting
//! changes nothing); WordCraft's formatting revisions are written as format changes.

use crate::odt::tests::{NS_T, content_of, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::RevisionKind;

const NS_DC: &str = r#"xmlns:dc="http://purl.org/dc/elements/1.1/""#;

/// Each run with a formatting revision: (its text, the revision's author, whether it is bold).
fn changed(d: &Document) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    for b in d.body.iter() {
        let Some(p) = b.as_para() else { continue };
        for (r, c) in p.run_ranges() {
            if let Some(ch) = &c.fmt_change {
                let rev = d.revisions.get(ch.rev as usize).expect("its revision");
                assert_eq!(rev.kind, RevisionKind::Format);
                out.push((p.text.get(r).unwrap_or("").to_string(), rev.author.clone(), c.bold == Some(true)));
            }
        }
    }
    out
}

/// LibreOffice's own (a document it saved).
fn open() -> Document {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_DC}><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style></office:automatic-styles><office:body><office:text><text:tracked-changes><text:changed-region text:id="ct1"><text:format-change><office:change-info><dc:creator>Ada</dc:creator><dc:date>2026-11-01T11:29:13</dc:date></office:change-info></text:format-change></text:changed-region></text:tracked-changes><text:p>Some <text:change-start text:change-id="ct1"/><text:span text:style-name="T1">words</text:span><text:change-end text:change-id="ct1"/> that turned bold.</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

#[test]
fn a_format_change_opens_as_a_formatting_revision() {
    assert_eq!(changed(&open()), [("words".to_string(), "Ada".to_string(), true)]);
}

#[test]
fn format_changes_round_trip() {
    let d = open();
    let bytes = export(&d).unwrap();
    let c = content_of(&bytes);
    assert!(c.contains("<text:format-change>") && c.contains("<text:change-start "), "{c}");
    // ODF 1.3 names a changed region by `xml:id` (the change marks refer to it).
    assert!(c.contains(r#"<text:changed-region xml:id="ct1" text:id="ct1">"#), "{c}");
    assert_eq!(changed(&import(&bytes).unwrap()), changed(&d));
}

#[test]
fn a_region_named_by_xml_id_alone_opens() {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_DC}><office:body><office:text><text:tracked-changes><text:changed-region xml:id="r1"><text:format-change><office:change-info><dc:creator>Bo</dc:creator><dc:date>2026-01-01T00:00:00</dc:date></office:change-info></text:format-change></text:changed-region></text:tracked-changes><text:p><text:change-start text:change-id="r1"/>shown<text:change-end text:change-id="r1"/></text:p></office:text></office:body></office:document-content>"#
    );
    assert_eq!(changed(&import(&package(&content, None)).unwrap()), [("shown".to_string(), "Bo".to_string(), false)]);
}

#[test]
fn hostile_format_changes_do_not_panic() {
    for body in [
        r#"<text:p><text:change-start text:change-id="nope"/>x<text:change-end text:change-id="nope"/></text:p>"#,
        r#"<text:p><text:change-start text:change-id="ct1"/>never ends</text:p><text:p>next</text:p>"#,
        r#"<text:p><text:change-end text:change-id="ct1"/>ends first</text:p>"#,
    ] {
        let content = format!(
            r#"<office:document-content {NS_T} {NS_DC}><office:body><office:text><text:tracked-changes><text:changed-region text:id="ct1"><text:format-change><office:change-info><dc:creator>A</dc:creator><dc:date>2026-01-01T00:00:00</dc:date></office:change-info></text:format-change></text:changed-region></text:tracked-changes>{body}</office:text></office:body></office:document-content>"#
        );
        let d = import(&package(&content, None)).unwrap();
        let _ = export(&d).unwrap();
    }
}
