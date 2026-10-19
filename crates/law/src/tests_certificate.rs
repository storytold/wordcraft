//! Certificate of compliance: word count parity, refresh on export, and DOCX round trip.

use serde_json::json;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Block, Document, Paragraph, Path, StoryRef, Table};
use wordcraft_engine::Session;

use crate::certificate::FIELD_WORDCOUNT;
use crate::checks;
use crate::cmd::store;
use crate::specs;
use crate::tests_checks::write_skeleton;

use super::tests::{FEDERAL, MISSOURI, apply_session, err, law, profile_json, roundtrip, run, temp_dir};

fn cert_wordcount_in_doc(doc: &Document) -> Option<u32> {
    for path in doc.para_paths(StoryRef::Body) {
        let para = doc.para(StoryRef::Body, &path)?;
        for obj in &para.objects {
            if let InlineObject::Field { instr, result, .. } = obj
                && instr == FIELD_WORDCOUNT
            {
                return result.parse().ok();
            }
        }
    }
    None
}

fn compliance(s: &mut Session, id: &str, doc_type: &str) -> serde_json::Value {
    run(s, "law.certificate.compliance", json!({"id": id, "doc": doc_type}))
}

#[test]
fn certificate_word_count_matches_the_word_limit_check() {
    for (id, doc_type) in [(MISSOURI, "brief"), (FEDERAL, "brief")] {
        let profile = store().get(id).expect("profile").clone();
        let mut s = apply_session(id, doc_type);
        write_skeleton(&mut s, &profile, doc_type);
        let r = compliance(&mut s, id, doc_type);
        let from_cert = r["wordCount"].as_u64().expect("wordCount") as u32;
        let from_check = checks::word_count(&s.doc, &profile, doc_type).expect("count");
        assert_eq!(from_cert, from_check, "{id}/{doc_type}");
        assert_eq!(cert_wordcount_in_doc(&s.doc), Some(from_check));
    }
}

#[test]
fn editing_then_save_refreshes_the_certificate_count() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    compliance(&mut s, MISSOURI, "brief");
    // Type at the very start, before the first section heading, so the added words are counted and
    // the certificate's current count must actually move.
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "text.insert", json!({"text": "extra extra extra words added before the certificate.\r"}));
    let after = checks::word_count(&s.doc, &profile, "brief").expect("count");
    let dir = super::tests::temp_dir("cert-save");
    let path = dir.join("brief.docx");
    run(&mut s, "file.save", json!({"path": path.to_string_lossy()}));
    let after_field = cert_wordcount_in_doc(&s.doc).expect("field");
    assert_eq!(after_field, after, "export refresh must update the certificate field");
    // The saved artifact itself carries the refreshed count, not just the in-memory document.
    let bytes = std::fs::read(&path).expect("read the saved file");
    let reopened = wordcraft_engine::io::open_bytes("brief.docx", &bytes).expect("reopen");
    assert_eq!(cert_wordcount_in_doc(&reopened), Some(after), "the saved .docx holds the refreshed count");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn certificate_field_survives_docx_round_trip() {
    let profile = store().get(FEDERAL).expect("profile").clone();
    let mut s = apply_session(FEDERAL, "brief");
    write_skeleton(&mut s, &profile, "brief");
    compliance(&mut s, FEDERAL, "brief");
    let doc = roundtrip(&s);
    assert!(doc.plain_text(StoryRef::Body).contains("Certificate of Compliance"));
    assert!(cert_wordcount_in_doc(&doc).is_some(), "LAWWORDCOUNT survives .docx");
}

#[test]
fn over_limit_still_inserts_and_returns_the_word_limit_finding() {
    let mut p = serde_json::to_value(store().get(MISSOURI).expect("profile")).expect("json");
    p["id"] = json!("test-cert-over");
    p["docs"]["brief"]["checks"] = json!([{
        "kind": "wordLimit",
        "limit": 5,
        "excludeSections": ["cover", "tableOfContents", "tableOfAuthorities", "certificateOfCompliance", "certificateOfService"],
        "source": "Test Rule"
    }]);
    p["docs"]["brief"]["certificateOfCompliance"] = json!({
        "template": "I certify that this brief contains {wordCount} words and complies with the limit of {wordLimit} words ({rule}).",
        "source": "Test Rule"
    });
    let mut s = law();
    run(&mut s, "law.profiles.create", p);
    run(&mut s, "law.profiles.apply", json!({"id": "test-cert-over", "doc": "brief"}));
    let profile = store().get("test-cert-over").expect("profile").clone();
    write_skeleton(&mut s, &profile, "brief");
    let r = compliance(&mut s, "test-cert-over", "brief");
    let findings = r["findings"].as_array().expect("findings");
    assert!(findings.iter().any(|f| f["checkId"] == "wordLimit"), "{findings:?}");
    assert!(s.doc.plain_text(StoryRef::Body).contains("I certify"));
}

#[test]
fn update_fields_refreshes_lawwordcount() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    compliance(&mut s, MISSOURI, "brief");
    run(&mut s, "text.insert", json!({"text": "more counted words here.\r"}));
    let want = checks::word_count(&s.doc, &profile, "brief").expect("count");
    run(&mut s, "references.updateFields", json!({}));
    assert_eq!(cert_wordcount_in_doc(&s.doc), Some(want));
}

#[test]
fn compliance_and_refresh_commands_are_installed() {
    let ids: Vec<&str> = specs().iter().map(|c| c.id).collect();
    assert!(ids.contains(&"law.certificate.compliance"));
    assert!(ids.contains(&"law.certificate.refresh"));
}

fn insert_top_level_table_before_heading(s: &mut Session, heading: &str) {
    let paths = s.doc.para_paths(StoryRef::Body);
    let idx = paths
        .iter()
        .position(|p| s.doc.para(StoryRef::Body, p).is_some_and(|para| para.plain_text().trim().eq_ignore_ascii_case(heading)))
        .expect("heading");
    let block = paths[idx].0.first().copied().unwrap_or(0) as usize;
    s.doc.insert_block(StoryRef::Body, &Path::top(block), Block::Table(Table::new(1, 1, 200.0))).expect("insert table");
    s.touch();
}

fn certificate_para_follows_heading(doc: &Document, heading: &str) -> bool {
    let paths = doc.para_paths(StoryRef::Body);
    let hi = paths
        .iter()
        .position(|p| doc.para(StoryRef::Body, p).is_some_and(|para| para.plain_text().trim().eq_ignore_ascii_case(heading)))
        .expect("heading");
    paths.get(hi + 1).and_then(|p| doc.para(StoryRef::Body, p)).is_some_and(|para| {
        para.plain_text().contains("certify")
            || para.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, .. } if instr == FIELD_WORDCOUNT))
    })
}

#[test]
fn certificate_inserts_after_heading_when_a_table_precedes_the_section() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    insert_top_level_table_before_heading(&mut s, "Certificate of Compliance");
    compliance(&mut s, MISSOURI, "brief");
    assert!(certificate_para_follows_heading(&s.doc, "Certificate of Compliance"));
    compliance(&mut s, MISSOURI, "brief");
    assert!(s.doc.plain_text(StoryRef::Body).contains("certify"));
    assert_eq!(cert_wordcount_in_doc(&s.doc), Some(checks::word_count(&s.doc, &profile, "brief").expect("count")));
}

/// P1 regression: the command given explicit parameters on a document that never had a profile
/// applied must remember the binding, so a later field rebuild and the export hook still refresh.
#[test]
fn certificate_with_explicit_parameters_refreshes_on_an_unapplied_document() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "Certificate of Compliance\r"}));
    compliance(&mut s, MISSOURI, "brief");
    assert_eq!(s.doc.custom_prop(checks::PROP_PROFILE), Some(MISSOURI), "the chosen profile is persisted");
    assert_eq!(s.doc.custom_prop(checks::PROP_DOC_TYPE), Some("brief"), "the chosen document type is persisted");
    assert!(cert_wordcount_in_doc(&s.doc).is_some(), "the certificate carries a live word-count field");
    // Add counted words before the certificate, then rebuild fields: the hook must find the binding.
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "text.insert", json!({"text": "extra counted words added before the certificate.\r"}));
    let rebuilt = checks::word_count(&s.doc, &profile, "brief").expect("count");
    run(&mut s, "references.updateFields", json!({}));
    assert_eq!(cert_wordcount_in_doc(&s.doc), Some(rebuilt), "a field rebuild refreshes the persisted binding");
    // Edit counted text again *after* the last field rebuild, so the export hook—not the rebuild—is
    // what must refresh the saved field; the in-memory field is deliberately stale beforehand.
    run(&mut s, "text.insert", json!({"text": "and even more counted words after the rebuild.\r"}));
    let want = checks::word_count(&s.doc, &profile, "brief").expect("count");
    assert_ne!(want, rebuilt, "the extra words change the count");
    assert_ne!(cert_wordcount_in_doc(&s.doc), Some(want), "the field is stale before export");
    // The pre-export refresh must still find the binding too, and the saved artifact must carry it.
    let dir = temp_dir("cert-explicit-export");
    let path = dir.join("brief.docx");
    run(&mut s, "file.save", json!({"path": path.to_string_lossy()}));
    let bytes = std::fs::read(&path).expect("read the saved file");
    let reopened = wordcraft_engine::io::open_bytes("brief.docx", &bytes).expect("reopen");
    assert_eq!(cert_wordcount_in_doc(&reopened), Some(want), "the exported count is refreshed, never stale");
    let _ = std::fs::remove_dir_all(&dir);
}

fn top_level_heading_block(doc: &Document, name: &str) -> usize {
    doc.para_paths(StoryRef::Body)
        .iter()
        .find_map(|p| {
            (p.depth() == 0 && doc.para(StoryRef::Body, p).is_some_and(|para| para.plain_text().trim().eq_ignore_ascii_case(name)))
                .then(|| p.0.first().copied().unwrap_or(0) as usize)
        })
        .unwrap_or_else(|| panic!("no top-level section heading `{name}`"))
}

/// Move a two-block skeleton section (its heading and body paragraph) to sit immediately before
/// the `before` heading, producing an out-of-order document.
fn move_section_before_heading(s: &mut Session, section: &str, before: &str) {
    let start = top_level_heading_block(&s.doc, section);
    let mut moved = Vec::new();
    for _ in 0..2 {
        moved.push(s.doc.remove_block(StoryRef::Body, &Path::top(start)).expect("remove a section block"));
    }
    let dest = top_level_heading_block(&s.doc, before);
    for (k, block) in moved.into_iter().enumerate() {
        s.doc.insert_block(StoryRef::Body, &Path::top(dest + k), block).expect("insert a section block");
    }
    s.touch();
}

/// P1 regression: replacement is bounded at the next recognized profile section whatever its
/// profile order. An out-of-order document (an earlier section such as `Argument` after the
/// certificate) must keep that section and its content across a refresh — it must not be deleted
/// along with the old certificate, which is what happened when only the sections *after* the
/// certificate in profile order ended the range.
#[test]
fn refresh_preserves_an_out_of_order_section_that_follows_the_certificate() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    compliance(&mut s, MISSOURI, "brief");
    // Give `Argument` a unique body, then move the whole section to sit right after the
    // certificate section, before `Certificate of Service`: a reordered document.
    let paths = s.doc.para_paths(StoryRef::Body);
    let arg_heading = paths
        .iter()
        .position(|p| s.doc.para(StoryRef::Body, p).is_some_and(|q| q.plain_text().trim().eq_ignore_ascii_case("Argument")))
        .expect("Argument heading");
    let arg_body = paths.get(arg_heading + 1).cloned().expect("Argument body paragraph");
    *s.doc.para_mut(StoryRef::Body, &arg_body).expect("an argument paragraph") =
        Paragraph::with_text("The argument content that must survive.", CharProps::default());
    s.touch();
    move_section_before_heading(&mut s, "Argument", "Certificate of Service");
    // Rebuild the certificate. With the bug, everything from the certificate to the next
    // *following* profile section (`Certificate of Service`) — including `Argument` — was deleted.
    compliance(&mut s, MISSOURI, "brief");
    let text = s.doc.plain_text(StoryRef::Body);
    assert!(text.contains("Argument"), "the reordered Argument heading survives: {text}");
    assert!(text.contains("The argument content that must survive."), "the reordered section's content survives: {text}");
    // A second refresh must not remove it either (the previous certificate body is replaced in
    // place).
    compliance(&mut s, MISSOURI, "brief");
    let text = s.doc.plain_text(StoryRef::Body);
    assert!(text.contains("The argument content that must survive."), "the reordered section survives a second refresh: {text}");
    assert_eq!(cert_wordcount_in_doc(&s.doc), Some(checks::word_count(&s.doc, &profile, "brief").expect("count")));
}

fn insert_table_with_headings(s: &mut Session, headings: &[&str]) {
    let mut t = Table::new(headings.len(), 1, 400.0);
    for (r, heading) in headings.iter().enumerate() {
        if let Some(cell) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(0)) {
            cell.blocks = vec![Block::Para(Paragraph::with_text(heading, CharProps::default())).into()];
        }
    }
    let at = s.doc.story(StoryRef::Body).map(|b| b.len()).unwrap_or(0);
    s.doc.insert_block(StoryRef::Body, &Path::top(at), Block::Table(t)).expect("insert table");
    s.touch();
}

/// P2 regression: a section heading inside a table has no top-level place for the certificate.
/// The command must refuse before touching the document, and repeating it must not leave a stray
/// certificate (or a duplicate) outside the table.
#[test]
fn certificate_is_refused_when_the_section_heading_is_inside_a_table() {
    let mut s = apply_session(MISSOURI, "brief");
    insert_table_with_headings(&mut s, &["Certificate of Compliance", "Certificate of Service"]);
    let before = s.doc.plain_text(StoryRef::Body);
    let e = err(&mut s, "law.certificate.compliance", json!({"id": MISSOURI, "doc": "brief"})).to_string();
    assert!(e.contains("table"), "a nested heading is refused: {e}");
    assert!(cert_wordcount_in_doc(&s.doc).is_none(), "nothing was inserted");
    assert_eq!(s.doc.plain_text(StoryRef::Body), before, "the document is unchanged");
    let e = err(&mut s, "law.certificate.compliance", json!({"id": MISSOURI, "doc": "brief"})).to_string();
    assert!(e.contains("table"), "repeated runs still refuse: {e}");
    assert_eq!(s.doc.plain_text(StoryRef::Body), before, "no duplicate certificate appears after repeated runs");
}

#[test]
fn explicit_profile_parameters_must_match_applied_metadata() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    let e = err(&mut s, "law.certificate.compliance", json!({"id": FEDERAL, "doc": "brief"})).to_string();
    assert!(e.contains("applied"), "{e}");
    let e = err(&mut s, "law.certificate.compliance", json!({"id": MISSOURI, "doc": "appendix"})).to_string();
    assert!(e.contains("document type"), "{e}");
}
#[test]
fn export_succeeds_when_profile_has_no_word_limit_and_no_certificate_field() {
    let mut s = law();
    let mut p = profile_json("test-no-word-limit");
    p["docs"]["brief"]["checks"] = json!([{"kind": "noHyperlinks", "source": "Test Rule"}]);
    run(&mut s, "law.profiles.create", p);
    run(&mut s, "law.profiles.apply", json!({"id": "test-no-word-limit", "doc": "brief"}));
    run(&mut s, "text.insert", json!({"text": "Body text without a certificate.\r"}));
    let dir = temp_dir("cert-export-no-wl");
    let path = dir.join("brief.docx");
    run(&mut s, "file.save", json!({"path": path.to_string_lossy()}));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Exporting a document with a profile applied and no certificate changes nothing: no undo step,
/// still saved.
#[test]
fn export_without_a_certificate_leaves_the_document_as_it_was() {
    let mut s = apply_session(MISSOURI, "brief");
    run(&mut s, "text.insert", json!({"text": "Body text without a certificate.\r"}));
    let dir = temp_dir("cert-export-noop");
    run(&mut s, "file.save", json!({"path": dir.join("brief.docx").to_string_lossy()}));
    assert!(!s.dirty);
    let undo = s.undo_labels();
    run(&mut s, "file.exportPdf", json!({"path": dir.join("brief.pdf").to_string_lossy()}));
    assert!(!s.dirty, "exporting doesn't mark it changed");
    assert_eq!(s.undo_labels(), undo, "and leaves no undo step");
    let _ = std::fs::remove_dir_all(&dir);
}

/// One Certificate of Compliance is one undo step.
#[test]
fn inserting_the_certificate_is_one_undo_step() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    let before = s.undo_labels().len();
    compliance(&mut s, MISSOURI, "brief");
    assert_eq!(s.undo_labels().len(), before + 1, "{:?}", s.undo_labels());
}

/// With Track Changes on, replacing the certificate is a tracked change: Reject All brings the old
/// certificate back, Accept All keeps the new one.
#[test]
fn replacing_the_certificate_is_tracked_while_tracking() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    for answer in ["review.rejectAll", "review.acceptAll"] {
        let mut s = apply_session(MISSOURI, "brief");
        write_skeleton(&mut s, &profile, "brief");
        compliance(&mut s, MISSOURI, "brief");
        let old_count = cert_wordcount_in_doc(&s.doc);
        run(&mut s, "caret.docStart", json!({}));
        run(&mut s, "text.insert", json!({"text": "extra extra extra words.\r"}));
        let before = s.doc.plain_text(StoryRef::Body);
        run(&mut s, "review.trackChanges", json!({"value": true}));
        let new_count = compliance(&mut s, MISSOURI, "brief")["wordCount"].as_u64().expect("count") as u32;
        assert_ne!(Some(new_count), old_count, "the count moved");
        assert!(!s.doc.revisions.is_empty(), "recorded as revisions");
        run(&mut s, answer, json!({}));
        // One certificate either way, nothing else changed; its word count is a live field, so
        // it shows the document's current count whichever certificate stays.
        let digits = |t: &str| t.chars().filter(|c| !c.is_ascii_digit()).collect::<String>();
        assert_eq!(digits(&s.doc.plain_text(StoryRef::Body)), digits(&before), "{answer}");
        assert_eq!(s.doc.plain_text(StoryRef::Body).matches("I certify").count(), 1, "{answer}");
        let _ = (old_count, new_count);
    }
}
