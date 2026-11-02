//! Court profile checks: compliant and violating documents per bundled profile.

use serde_json::{Value, json};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{CharProps, Document, Paragraph, StoryRef, para_block};
use wordcraft_engine::Session;

use crate::cmd::store;
use crate::profile::Profile;
use crate::specs;

use super::tests::{FEDERAL, MISSOURI, apply_session, err, law, run};

fn findings(s: &mut Session, id: &str, doc_type: &str, for_export: bool) -> Vec<Value> {
    let r = run(s, "law.checks.run", json!({"id": id, "doc": doc_type, "forExport": for_export, "store": false}));
    r["findings"].as_array().cloned().unwrap_or_default()
}

fn has_check(findings: &[Value], check_id: &str) -> bool {
    findings.iter().any(|f| f["checkId"] == json!(check_id))
}

/// Write every section heading from the profile, with body text under each.
pub(crate) fn write_skeleton(s: &mut Session, profile: &Profile, doc_type: &str) {
    let spec = profile.docs.get(doc_type).expect("document type");
    // Not every document type has a `Body` style (the appendix does not); fall back to the first
    // style so the body still carries the profile's formatting.
    let body_style = if spec.styles.iter().any(|st| st.name.eq_ignore_ascii_case("Body")) {
        "Body"
    } else {
        spec.styles.first().map(|st| st.name.as_str()).unwrap_or("Normal")
    };
    for sec in &spec.sections {
        run(s, "text.insert", json!({"text": sec.name}));
        if let Some(st) = &sec.style {
            run(s, "para.style", json!({"style": st}));
        }
        run(s, "text.insert", json!({"text": "\r"}));
        run(s, "styles.apply", json!({"style": body_style}));
        run(s, "text.insert", json!({"text": "Body paragraph for this section."}));
        run(s, "text.insert", json!({"text": "\r"}));
    }
}

#[test]
fn missouri_initial_brief_skeleton_passes_checks() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    let f = findings(&mut s, MISSOURI, "brief", false);
    assert!(f.is_empty(), "expected no findings: {f:?}");
}

#[test]
fn federal_principal_brief_skeleton_passes_checks() {
    let profile = store().get(FEDERAL).expect("profile").clone();
    let mut s = apply_session(FEDERAL, "brief");
    write_skeleton(&mut s, &profile, "brief");
    let f = findings(&mut s, FEDERAL, "brief", false);
    assert!(f.is_empty(), "expected no findings: {f:?}");
}

/// Every document type of every bundled profile: a compliant skeleton reports nothing, on demand
/// and on export (so the export-only checks such as `textSearchablePdf` are covered too).
#[test]
fn every_bundled_document_type_compliant_skeleton_has_no_findings() {
    for id in [MISSOURI, FEDERAL] {
        let profile = store().get(id).expect("profile").clone();
        let doc_types: Vec<String> = profile.doc_types().iter().map(|s| s.to_string()).collect();
        assert!(!doc_types.is_empty(), "{id} has no document types");
        for doc_type in doc_types {
            let mut s = apply_session(id, &doc_type);
            write_skeleton(&mut s, &profile, &doc_type);
            for for_export in [false, true] {
                let f = findings(&mut s, id, &doc_type, for_export);
                assert!(f.is_empty(), "{id}/{doc_type} (export {for_export}): expected no findings: {f:?}");
            }
        }
    }
}

/// The bundled respondent and reply briefs carry the formatting checks too, not just a typeface.
#[test]
fn missouri_respondent_and_reply_checks_are_populated() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    for doc_type in ["respondentBrief", "reply"] {
        let spec = profile.docs.get(doc_type).expect("document type");
        let kinds: Vec<&str> = spec.checks.iter().map(|c| c.kind.as_str()).collect();
        for want in ["wordLimit", "minFontSize", "minMargins", "minLineSpacing", "requiredSections", "noHyperlinks", "textSearchablePdf"] {
            assert!(kinds.contains(&want), "{doc_type} is missing `{want}`: {kinds:?}");
        }
        let word_limit = spec.checks.iter().find(|c| c.kind == "wordLimit").expect("a word limit");
        assert!(!word_limit.exclude_sections.is_empty(), "{doc_type}: the word limit needs counting exclusions");
        let sections = spec.checks.iter().find(|c| c.kind == "requiredSections").expect("required sections");
        assert_eq!(sections.ids.len(), spec.sections.len(), "{doc_type}: every section is required, in order");
    }
}

/// Respondent and reply briefs report the same violations as the initial brief.
#[test]
fn missouri_respondent_and_reply_violations_are_reported() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    for doc_type in ["respondentBrief", "reply"] {
        // Small type.
        let mut s = apply_session(MISSOURI, doc_type);
        write_skeleton(&mut s, &profile, doc_type);
        run(&mut s, "format.size", json!({"size": 10}));
        run(&mut s, "text.insert", json!({"text": "tiny"}));
        assert!(has_check(&findings(&mut s, MISSOURI, doc_type, false), "minFontSize"), "{doc_type}: minFontSize");
        // Tight margins.
        let mut s = apply_session(MISSOURI, doc_type);
        write_skeleton(&mut s, &profile, doc_type);
        assert!(s.run("layout.margins", &json!({"top": 36})).is_ok());
        assert!(has_check(&findings(&mut s, MISSOURI, doc_type, false), "minMargins"), "{doc_type}: minMargins");
        // A required section missing (or out of order).
        let mut s = apply_session(MISSOURI, doc_type);
        run(&mut s, "styles.apply", json!({"style": "Heading"}));
        run(&mut s, "text.insert", json!({"text": "Cover\n"}));
        assert!(has_check(&findings(&mut s, MISSOURI, doc_type, false), "requiredSections"), "{doc_type}: requiredSections");
    }
}

/// Missouri appendix: margins and font checks (no word limit on the exhibit volume).
#[test]
fn missouri_appendix_violations_are_reported() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "appendix");
    write_skeleton(&mut s, &profile, "appendix");
    run(&mut s, "format.size", json!({"size": 10}));
    run(&mut s, "text.insert", json!({"text": "tiny"}));
    assert!(has_check(&findings(&mut s, MISSOURI, "appendix", false), "minFontSize"));
    let mut s = apply_session(MISSOURI, "appendix");
    write_skeleton(&mut s, &profile, "appendix");
    assert!(s.run("layout.margins", &json!({"top": 36})).is_ok());
    assert!(has_check(&findings(&mut s, MISSOURI, "appendix", false), "minMargins"));
    let kinds: Vec<&str> = profile.docs.get("appendix").expect("appendix").checks.iter().map(|c| c.kind.as_str()).collect();
    assert!(!kinds.contains(&"wordLimit"), "appendix has no word limit: {kinds:?}");
}

/// Federal violations beyond the type size: margins, spacing and section order.
#[test]
fn federal_checks_fire_beyond_font_size() {
    let profile = store().get(FEDERAL).expect("profile").clone();
    // Tight margins.
    let mut s = apply_session(FEDERAL, "brief");
    write_skeleton(&mut s, &profile, "brief");
    assert!(s.run("layout.margins", &json!({"top": 36})).is_ok());
    assert!(has_check(&findings(&mut s, FEDERAL, "brief", false), "minMargins"));
    // Single-spaced body below FRAP's double-spacing floor.
    let mut s = apply_session(FEDERAL, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "styles.apply", json!({"style": "Body"}));
    run(&mut s, "para.lineSpacing", json!({"value": 1}));
    run(&mut s, "text.insert", json!({"text": "Tight.\r"}));
    assert!(has_check(&findings(&mut s, FEDERAL, "brief", false), "minLineSpacing"));
    // A required section missing.
    let mut s = apply_session(FEDERAL, "brief");
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.insert", json!({"text": "Argument\n"}));
    assert!(has_check(&findings(&mut s, FEDERAL, "brief", false), "requiredSections"));
}

/// Required sections must be present *and in order*: every section is written, but two are
/// swapped, so the order check is what reports them — not a missing section.
#[test]
fn sections_out_of_order_are_reported() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let spec = profile.docs.get("brief").expect("brief");
    let mut s = apply_session(MISSOURI, "brief");
    // All of the profile's sections, in order, with Argument and Conclusion swapped.
    let mut sections: Vec<&crate::profile::SectionSpec> = spec.sections.iter().collect();
    let (a, c) = (
        sections.iter().position(|x| x.id == "argument").expect("Argument"),
        sections.iter().position(|x| x.id == "conclusion").expect("Conclusion"),
    );
    sections.swap(a, c);
    for sec in sections {
        run(&mut s, "styles.apply", json!({"style": sec.style.clone().unwrap_or_else(|| "Heading".into())}));
        run(&mut s, "text.insert", json!({"text": format!("{}\n", sec.name)}));
    }
    let f = findings(&mut s, MISSOURI, "brief", false);
    assert!(has_check(&f, "requiredSections"), "{f:?}");
}

/// Single-spaced exceptions (caption, heading, block quote, signature, certificate) may be
/// tighter than the body's minimum, but they still may not go below single spacing.
#[test]
fn single_spaced_exceptions_must_still_be_at_least_single_spaced() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    // Compliant: a Heading at single spacing is allowed.
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "para.lineSpacing", json!({"value": 1}));
    run(&mut s, "text.insert", json!({"text": "Heading"}));
    let f = findings(&mut s, MISSOURI, "brief", false);
    assert!(!has_check(&f, "minLineSpacing"), "single spacing is allowed for an exception: {f:?}");

    // Underspaced: a Heading tighter than single spacing is reported.
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "para.lineSpacing", json!({"value": 0.5}));
    run(&mut s, "text.insert", json!({"text": "Heading"}));
    let f = findings(&mut s, MISSOURI, "brief", false);
    assert!(has_check(&f, "minLineSpacing"), "an exception below single spacing is reported: {f:?}");
}

/// The word limit counts the profile's included sections and skips the excluded ones; the
/// boundary is exact.
#[test]
fn word_limit_counts_included_sections_only() {
    // The bundled Missouri brief, with a limit small enough to reach in a test: the same check,
    // the same `excludeSections`, only the number lowered.
    let mut profile = store().get(MISSOURI).expect("profile").clone();
    let brief = profile.docs.get_mut("brief").expect("brief");
    let wl = brief.checks.iter_mut().find(|c| c.kind == "wordLimit").expect("a word limit");
    wl.limit = Some(6);

    // Words under an excluded heading (Cover) do not count.
    let mut s = apply_session(MISSOURI, "brief");
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.insert", json!({"text": "Cover\r"}));
    run(&mut s, "styles.apply", json!({"style": "Body"}));
    run(&mut s, "text.insert", json!({"text": "one two three four five six seven eight\r"}));
    let f = crate::checks::evaluate(&s.doc, &profile, "brief", false, false, None).expect("evaluate");
    assert!(!f.iter().any(|x| x.check_id == "wordLimit"), "excluded words counted: {f:?}");

    // A counted heading (Argument) plus five words is exactly the limit: it passes...
    let mut s = apply_session(MISSOURI, "brief");
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.insert", json!({"text": "Argument\r"}));
    run(&mut s, "styles.apply", json!({"style": "Body"}));
    run(&mut s, "text.insert", json!({"text": "one two three four five\r"}));
    let f = crate::checks::evaluate(&s.doc, &profile, "brief", false, false, None).expect("evaluate");
    assert!(!f.iter().any(|x| x.check_id == "wordLimit"), "exactly at the limit must pass: {f:?}");
    // ...and one more word is over it.
    run(&mut s, "text.insert", json!({"text": "six\r"}));
    let f = crate::checks::evaluate(&s.doc, &profile, "brief", false, false, None).expect("evaluate");
    assert!(f.iter().any(|x| x.check_id == "wordLimit"), "one over the limit must fail: {f:?}");
}

/// The bundled word limits are exercised at their real production boundaries, not a lowered test
/// value: exactly the limit passes, one word more fails, for every bundled limit.
#[test]
fn bundled_word_limits_are_exact_at_their_production_values() {
    let cases = [
        (MISSOURI, "brief", 31_000u32),
        (MISSOURI, "respondentBrief", 27_900),
        (MISSOURI, "reply", 7_750),
        (FEDERAL, "brief", 13_000),
        (FEDERAL, "reply", 6_500),
    ];
    for (id, doc_type, limit) in cases {
        let profile = store().get(id).expect("profile").clone();
        let words = |n: usize| {
            let text = vec!["word"; n].join(" ");
            let mut d = Document::new();
            d.body = vec![para_block(Paragraph::with_text(&text, CharProps::default()))];
            d
        };
        let at = crate::checks::evaluate(&words(limit as usize), &profile, doc_type, false, false, None).expect("evaluate");
        assert!(!at.iter().any(|x| x.check_id == "wordLimit"), "{id}/{doc_type}: exactly {limit} words must pass");
        let over = crate::checks::evaluate(&words(limit as usize + 1), &profile, doc_type, false, false, None).expect("evaluate");
        assert!(over.iter().any(|x| x.check_id == "wordLimit"), "{id}/{doc_type}: {} words must fail", limit + 1);
    }
}

/// A text box under an excluded section (a label on the cover) is left out of the word count, and
/// one under a counted section is counted — text boxes belong to the section that anchors them.
#[test]
fn text_boxes_follow_their_section_for_the_word_limit() {
    let mut profile = store().get(MISSOURI).expect("profile").clone();
    let brief = profile.docs.get_mut("brief").expect("brief");
    let wl = brief.checks.iter_mut().find(|c| c.kind == "wordLimit").expect("a word limit");
    wl.limit = Some(2);

    // Under the excluded Cover: the box's ten words do not count.
    let mut s = apply_session(MISSOURI, "brief");
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.insert", json!({"text": "Cover\r"}));
    run(&mut s, "insert.textBox", json!({"text": "one two three four five six seven eight nine ten"}));
    let f = crate::checks::evaluate(&s.doc, &profile, "brief", false, false, None).expect("evaluate");
    assert!(!f.iter().any(|x| x.check_id == "wordLimit"), "a text box under the excluded cover must not count: {f:?}");

    // Under a counted heading: the box's ten words do count.
    let mut s = apply_session(MISSOURI, "brief");
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.insert", json!({"text": "Argument\r"}));
    run(&mut s, "insert.textBox", json!({"text": "one two three four five six seven eight nine ten"}));
    let f = crate::checks::evaluate(&s.doc, &profile, "brief", false, false, None).expect("evaluate");
    assert!(f.iter().any(|x| x.check_id == "wordLimit"), "a text box under a counted heading must count: {f:?}");
}

#[test]
fn each_missouri_check_fires_on_a_violation() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    // Word limit (use a tiny limit on a test profile)
    let mut s = law();
    let mut p = super::tests::profile_json("test-word-limit");
    p["docs"]["brief"]["checks"] = json!([{"kind": "wordLimit", "limit": 5, "source": "Test Rule"}]);
    run(&mut s, "law.profiles.create", p);
    run(&mut s, "law.profiles.apply", json!({"id": "test-word-limit", "doc": "brief"}));
    run(&mut s, "text.insert", json!({"text": "one two three four five six seven"}));
    assert!(has_check(&findings(&mut s, "test-word-limit", "brief", false), "wordLimit"));
    // Required section missing
    let mut s = apply_session(MISSOURI, "brief");
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.insert", json!({"text": "Cover\n"}));
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "requiredSections"));
    // Hyperlink
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "link"}));
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "noHyperlinks"));
    // Small type
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "format.size", json!({"size": 10}));
    run(&mut s, "text.insert", json!({"text": "tiny"}));
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "minFontSize"));
    // Tight margins
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    assert!(s.run("layout.margins", &json!({"top": 36})).is_ok());
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "minMargins"));
}

#[test]
fn export_runs_checks_and_reports_through_proofing() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "x"}));
    let dir = super::tests::temp_dir("export-checks");
    let path = dir.join("brief.pdf");
    let r = run(&mut s, "file.exportPdf", json!({"path": path.to_string_lossy()}));
    let export_findings = r["findings"].as_array().expect("export findings");
    assert!(export_findings.iter().any(|f| f["checkId"] == "noHyperlinks"));
    let issues = run(&mut s, "review.issues", json!({}));
    let issues = issues.as_array().expect("issues");
    assert!(issues.iter().any(|f| f["kind"] == "lint" && f["checkId"] == "noHyperlinks"));
    // The same findings are the document's lint findings (Review › Proofing reads both).
    let lint = run(&mut s, "law.checks.findings", json!({}));
    assert!(lint.as_array().expect("lint").iter().any(|f| f["checkId"] == "noHyperlinks"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn law_checks_run_is_pure_and_lists_findings() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "x"}));
    // A missing profile id is an error (no applied profile and none given).
    let e = err(&mut s, "law.checks.run", json!({"id": "no-such-court", "doc": "brief"})).to_string();
    assert!(e.contains("no court profile"), "{e}");
    // Running the checks returns the findings and never changes the document.
    let before = serde_json::to_string(&s.doc).expect("serialize");
    let r = run(&mut s, "law.checks.run", json!({"id": MISSOURI, "doc": "brief", "store": false}));
    let after = serde_json::to_string(&s.doc).expect("serialize");
    assert_eq!(before, after, "law.checks.run must not edit the document");
    assert!(r["findings"].as_array().is_some_and(|f| has_check(f, "noHyperlinks")), "{r}");
    assert!(specs().iter().any(|c| c.id == "law.checks.run"));
}

#[test]
fn export_checks_survive_save_and_reopen() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "x"}));
    let bytes = wordcraft_engine::io::save_bytes("brief.docx", &s.doc).expect("save");
    let doc = wordcraft_engine::io::open_bytes("brief.docx", &bytes).expect("open");
    let mut s = law();
    s.set_document(doc);
    let dir = super::tests::temp_dir("reopen-export");
    let path = dir.join("brief.pdf");
    let r = run(&mut s, "file.exportPdf", json!({"path": path.to_string_lossy()}));
    assert!(r["findings"].as_array().expect("findings").iter().any(|f| f["checkId"] == "noHyperlinks"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn later_section_tight_margins_are_reported() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "layout.break", json!({"kind": "nextPage"}));
    run(&mut s, "text.insert", json!({"text": "Second section body\r"}));
    assert!(s.run("layout.margins", &json!({"top": 36})).is_ok());
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "minMargins"));
}

#[test]
fn exact_line_spacing_below_minimum_fails() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "styles.apply", json!({"style": "Body"}));
    run(&mut s, "para.lineSpacing", json!({"exactly": 12}));
    run(&mut s, "text.insert", json!({"text": "Tight body line.\r"}));
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "minLineSpacing"));
}

/// Body text placed in a text box is still body text: a single-spaced box violates Missouri's
/// 1.5-spacing rule and Federal's double-spacing rule, exactly as a single-spaced body paragraph
/// does. Text boxes live in their own part stories, so the check cannot look at the body alone.
#[test]
fn text_box_line_spacing_is_checked() {
    for (id, doc_type) in [(MISSOURI, "brief"), (FEDERAL, "brief")] {
        let profile = store().get(id).expect("profile").clone();
        let mut s = apply_session(id, doc_type);
        write_skeleton(&mut s, &profile, doc_type);
        run(&mut s, "insert.textBox", json!({"text": "A tight box."}));
        run(&mut s, "styles.apply", json!({"style": "Body"}));
        run(&mut s, "para.lineSpacing", json!({"value": 1}));
        let f = findings(&mut s, id, doc_type, false);
        assert!(has_check(&f, "minLineSpacing"), "{id}: a single-spaced text box must be reported: {f:?}");
    }
}

/// The profile's single-spaced exceptions apply inside a text box too: a Caption-styled box at
/// single spacing is allowed, like any other caption.
#[test]
fn text_box_single_spaced_exceptions_are_allowed() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "insert.textBox", json!({"text": "A caption box."}));
    run(&mut s, "styles.apply", json!({"style": "Caption"}));
    run(&mut s, "para.lineSpacing", json!({"value": 1}));
    let f = findings(&mut s, MISSOURI, "brief", false);
    assert!(!has_check(&f, "minLineSpacing"), "a single-spaced caption text box is allowed: {f:?}");
}

#[test]
fn footnote_hyperlink_is_reported() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    let note = StoryRef::Part(run(&mut s, "references.footnote", json!({"text": "See also"}))["id"].as_u64().expect("id") as u32);
    run(&mut s, "caret.set", json!({"pos": {"story": note, "path": [0], "off": 0}}));
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "link"}));
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "noHyperlinks"));
}

#[test]
fn header_and_footer_hyperlinks_are_reported() {
    for cmd in ["insert.header", "insert.footer"] {
        let profile = store().get(MISSOURI).expect("profile").clone();
        let mut s = apply_session(MISSOURI, "brief");
        write_skeleton(&mut s, &profile, "brief");
        assert!(!has_check(&findings(&mut s, MISSOURI, "brief", false), "noHyperlinks"));
        let story = run(&mut s, cmd, json!({}))["story"].as_u64().expect("story") as u32;
        run(&mut s, "caret.set", json!({"pos": {"story": StoryRef::Part(story), "path": [0], "off": 0}}));
        run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "link"}));
        assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "noHyperlinks"), "{cmd}");
    }
}

#[test]
fn text_before_first_heading_counts_toward_word_limit() {
    let mut s = law();
    let mut p = super::tests::profile_json("test-pre-heading");
    p["docs"]["brief"]["checks"] = json!([{"kind": "wordLimit", "limit": 3, "source": "Test"}]);
    run(&mut s, "law.profiles.create", p);
    run(&mut s, "law.profiles.apply", json!({"id": "test-pre-heading", "doc": "brief"}));
    run(&mut s, "text.insert", json!({"text": "one two three four five six\r"}));
    run(&mut s, "text.insert", json!({"text": "Cover\r"}));
    assert!(has_check(&findings(&mut s, "test-pre-heading", "brief", false), "wordLimit"));
}

#[test]
fn wrong_typeface_fails_missouri_check() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "format.font", json!({"name": "Arial"}));
    run(&mut s, "text.insert", json!({"text": "wrong face"}));
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "minFontSize"));
}

#[test]
fn federal_small_type_fails_check() {
    let profile = store().get(FEDERAL).expect("profile").clone();
    let mut s = apply_session(FEDERAL, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "format.size", json!({"size": 10}));
    run(&mut s, "text.insert", json!({"text": "small"}));
    assert!(has_check(&findings(&mut s, FEDERAL, "brief", false), "minFontSize"));
}

#[test]
fn searchable_pdf_passes_on_real_export() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    let f = findings(&mut s, MISSOURI, "brief", true);
    assert!(!has_check(&f, "textSearchablePdf"), "{f:?}");
}

/// A document whose only text is numeric is still a full text layer: no PDF finding.
#[test]
fn numeric_text_pdf_passes_on_real_export() {
    let mut d = Document::new();
    d.body = vec![para_block(Paragraph::with_text("123", CharProps::default()))];
    let bytes = wordcraft_engine::io::save_bytes_opts("export.pdf", &d, &Default::default()).expect("export a PDF");
    assert!(crate::checks::searchability_finding(&bytes, "textSearchablePdf").is_none());
}

#[test]
fn font_only_pdf_bytes_are_not_searchable() {
    let bytes = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/MediaBox[0 0 200 200]/Resources<</Font<</F1 4 0 R>>>>>>endobj\n4 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";
    assert_eq!(wordcraft_pdf::text_searchability(bytes), wordcraft_pdf::TextSearchability::NotSearchable);
}

/// An unsearchable export produces the matching LawCraft finding, not a silent pass.
#[test]
fn unsearchable_pdf_produces_the_text_searchable_finding() {
    let font_only = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/MediaBox[0 0 200 200]/Resources<</Font<</F1 4 0 R>>>>>>endobj\n4 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";
    let f = crate::checks::searchability_finding(font_only, "textSearchablePdf").expect("an unsearchable PDF must produce a finding");
    assert_eq!(f.check_id, "textSearchablePdf");
    assert_eq!(f.severity, "error");
    // A real export carries text, so the same helper says nothing.
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile_missouri(), "brief");
    let bytes = wordcraft_engine::io::save_bytes_opts("export.pdf", &s.doc, &Default::default()).expect("export a PDF");
    assert!(crate::checks::searchability_finding(&bytes, "textSearchablePdf").is_none());
}

/// A dominant scan with a long caption must surface the page-specific LawCraft warning, not pass or
/// downgrade to a document-wide "not searchable" error.
#[test]
fn dominant_scan_with_long_caption_warns_on_the_page() {
    let png = test_png();
    let mut d = Document::new();
    let media = d.add_media(png, "png");
    let mut scan = Paragraph::with_text("", CharProps::default());
    scan.insert_object(
        0,
        InlineObject::Image { media, w: 300.0, h: 400.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    let caption = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty twentyone twentytwo twentythree twentyfour twentyfive";
    d.body = vec![para_block(scan), para_block(Paragraph::with_text(caption, CharProps::default()))];
    let bytes = wordcraft_engine::io::save_bytes_opts("export.pdf", &d, &Default::default()).expect("export a PDF");
    let f = crate::checks::searchability_finding(&bytes, "textSearchablePdf").expect("a dominant scan must not pass");
    assert_eq!(f.severity, "warning");
    assert_eq!(f.message, "Page 1 has scanned content that could not be verified as searchable.");
}

/// An image-only scan (no text anywhere) must warn about the page it is on, not produce the generic
/// document-wide "not searchable" error: the scan is unverified content, and its page is named.
#[test]
fn image_only_scan_warns_on_the_page() {
    let mut d = Document::new();
    let media = d.add_media(test_png(), "png");
    let mut scan = Paragraph::with_text("", CharProps::default());
    scan.insert_object(
        0,
        InlineObject::Image { media, w: 300.0, h: 400.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    d.body = vec![para_block(scan)];
    let bytes = wordcraft_engine::io::save_bytes_opts("export.pdf", &d, &Default::default()).expect("export a PDF");
    let f = crate::checks::searchability_finding(&bytes, "textSearchablePdf").expect("an image-only scan must not pass");
    assert_eq!(f.severity, "warning");
    assert_eq!(f.message, "Page 1 has scanned content that could not be verified as searchable.");
}

/// A scan whose only text is a page-number footer must warn about the page: a page number is
/// furniture, so it never certifies the scanned content as searchable.
#[test]
fn scan_with_only_a_page_number_footer_warns_on_the_page() {
    let mut d = Document::new();
    let media = d.add_media(test_png(), "png");
    let mut scan = Paragraph::with_text("", CharProps::default());
    scan.insert_object(
        0,
        InlineObject::Image { media, w: 300.0, h: 400.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    d.body = vec![para_block(scan)];
    let mut footer = Paragraph::with_text("", CharProps::default());
    footer
        .insert_object(
            0,
            InlineObject::Field { instr: "PAGE".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
            &CharProps::default(),
        )
        .unwrap();
    let id = d.add_part(wordcraft_doc::PartKind::Footer, vec![para_block(footer)]);
    d.last_section.footers.default = Some(id);
    let bytes = wordcraft_engine::io::save_bytes_opts("export.pdf", &d, &Default::default()).expect("export a PDF");
    let f = crate::checks::searchability_finding(&bytes, "textSearchablePdf").expect("a page-number-only scan must not pass");
    assert_eq!(f.severity, "warning");
    assert_eq!(f.message, "Page 1 has scanned content that could not be verified as searchable.");
}

/// A tiny in-memory PNG for building a "scan" (a page-dominating picture) in a test document.
fn test_png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(16, 8, |x, y| image::Rgba([(x * 15) as u8, (y * 30) as u8, 200, 255]));
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
    buf
}

fn profile_missouri() -> Profile {
    store().get(MISSOURI).expect("profile").clone()
}

/// A host that opens a file and *then* installs LawCraft (`Session::new(doc)` → `install`) must
/// still get the applied profile's export checks; it never calls `set_document`.
#[test]
fn install_restores_export_checks_for_an_already_open_document() {
    let profile = profile_missouri();
    let mut built = apply_session(MISSOURI, "brief");
    write_skeleton(&mut built, &profile, "brief");
    run(&mut built, "insert.link", json!({"url": "https://example.com", "text": "x"}));
    let bytes = wordcraft_engine::io::save_bytes("brief.docx", &built.doc).expect("save");
    let doc = wordcraft_engine::io::open_bytes("brief.docx", &bytes).expect("open");

    // A fresh host session over the reopened document, then LawCraft installed (no set_document).
    let mut s = Session::new(doc);
    s.prefs.modules.insert(crate::MODULE.into(), true);
    assert_eq!(crate::install(&mut s), specs().len());

    let dir = super::tests::temp_dir("install-export");
    let path = dir.join("brief.pdf");
    let r = run(&mut s, "file.exportPdf", json!({"path": path.to_string_lossy()}));
    let f = r["findings"].as_array().expect("findings");
    assert!(f.iter().any(|x| x["checkId"] == "noHyperlinks"), "install must restore export checks: {f:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Exporting to a non-PDF format still runs the document checks, but never the PDF-only ones; the
/// findings flow through Review › Proofing like any other.
#[test]
fn non_pdf_export_runs_document_checks_not_pdf_verification() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "x"}));
    let dir = super::tests::temp_dir("export-docx-checks");
    let path = dir.join("brief.docx");
    let r = run(&mut s, "file.save", json!({"path": path.to_string_lossy()}));
    let f = r["findings"].as_array().expect("export findings");
    assert!(f.iter().any(|x| x["checkId"] == "noHyperlinks"), "a non-PDF export still runs document checks: {f:?}");
    assert!(!f.iter().any(|x| x["checkId"] == "textSearchablePdf"), "PDF verification must not run for a .docx export: {f:?}");
    // The findings are surfaced the same way as any other lint finding.
    let issues = run(&mut s, "review.issues", json!({}));
    assert!(issues.as_array().expect("issues").iter().any(|x| x["kind"] == "lint" && x["checkId"] == "noHyperlinks"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Exporting a page as PNG runs the applied profile's checks too, but never the PDF verification:
/// a PNG is not a PDF copy.
#[test]
fn png_export_runs_document_checks_not_pdf_verification() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "x"}));
    let dir = super::tests::temp_dir("export-png-checks");
    let path = dir.join("brief.png");
    let r = run(&mut s, "file.exportPng", json!({"path": path.to_string_lossy()}));
    let f = r["findings"].as_array().expect("export findings");
    assert!(f.iter().any(|x| x["checkId"] == "noHyperlinks"), "a PNG export still runs document checks: {f:?}");
    assert!(!f.iter().any(|x| x["checkId"] == "textSearchablePdf"), "PDF verification must not run for a PNG export: {f:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Printing to PDF is an export too: the applied profile's checks run, and the PDF verification
/// runs with them because the output is a PDF.
#[test]
fn print_pdf_export_runs_checks() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "x"}));
    let dir = super::tests::temp_dir("export-print-checks");
    let path = dir.join("brief.pdf");
    let r = run(&mut s, "file.printPdf", json!({"path": path.to_string_lossy()}));
    let f = r["findings"].as_array().expect("export findings");
    assert!(f.iter().any(|x| x["checkId"] == "noHyperlinks"), "a print-to-PDF still runs the checks: {f:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `law.checks.run` runs the PDF-only check by default (an on-demand check is about a PDF copy)
/// but skips it when the caller names a non-PDF output format.
#[test]
fn checks_run_honours_the_output_format() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "x"}));
    // Default (PDF): the document checks run.
    let pdf = findings(&mut s, MISSOURI, "brief", true);
    assert!(has_check(&pdf, "noHyperlinks"), "{pdf:?}");
    // A named `.docx` output still runs the document checks but not the PDF one.
    let docx = run(&mut s, "law.checks.run", json!({"id": MISSOURI, "doc": "brief", "forExport": true, "format": "docx", "store": false}));
    let docx = docx["findings"].as_array().cloned().unwrap_or_default();
    assert!(has_check(&docx, "noHyperlinks"), "{docx:?}");
    assert!(!has_check(&docx, "textSearchablePdf"), "a .docx output is not verified as a PDF: {docx:?}");
}

/// A federal brief over its word limit is reported, and the bundled limits are FRAP 32(a)(7)(B)'s.
#[test]
fn federal_word_limit_violation_is_reported() {
    let mut profile = store().get(FEDERAL).expect("profile").clone();
    let brief = profile.docs.get_mut("brief").expect("brief");
    let wl = brief.checks.iter_mut().find(|c| c.kind == "wordLimit").expect("a word limit");
    assert_eq!(wl.limit, Some(13_000), "the bundled principal brief limit");
    wl.limit = Some(5);
    let mut s = apply_session(FEDERAL, "brief");
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.insert", json!({"text": "Argument\r"}));
    run(&mut s, "styles.apply", json!({"style": "Body"}));
    run(&mut s, "text.insert", json!({"text": "one two three four five six seven\r"}));
    let f = crate::checks::evaluate(&s.doc, &profile, "brief", false, false, None).expect("evaluate");
    assert!(f.iter().any(|x| x.check_id == "wordLimit"), "over-limit federal principal brief: {f:?}");

    // The reply brief's bundled limit, and a violation of it.
    let reply = profile.docs.get_mut("reply").expect("reply");
    let rl = reply.checks.iter_mut().find(|c| c.kind == "wordLimit").expect("a word limit");
    assert_eq!(rl.limit, Some(6_500), "the bundled reply limit");
    rl.limit = Some(5);
    let mut s = apply_session(FEDERAL, "reply");
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.insert", json!({"text": "Argument\r"}));
    run(&mut s, "styles.apply", json!({"style": "Body"}));
    run(&mut s, "text.insert", json!({"text": "one two three four five six seven\r"}));
    let f = crate::checks::evaluate(&s.doc, &profile, "reply", false, false, None).expect("evaluate");
    assert!(f.iter().any(|x| x.check_id == "wordLimit"), "over-limit federal reply: {f:?}");
}

/// Apply profile A, then B, then Undo: the document is back on A, and so are the export checks,
/// which read the applied profile from the document when they run.
#[test]
fn undoing_a_profile_change_brings_its_checks_back_too() {
    let mut s = law();
    run(&mut s, "law.profiles.apply", json!({"id": MISSOURI, "doc": "brief"}));
    run(&mut s, "law.profiles.apply", json!({"id": FEDERAL, "doc": "brief"}));
    assert_eq!(run(&mut s, "law.checks.run", json!({"forExport": true}))["profile"], json!(FEDERAL));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.custom_prop(crate::checks::PROP_PROFILE), Some(MISSOURI));
    assert_eq!(run(&mut s, "law.checks.run", json!({"forExport": true}))["profile"], json!(MISSOURI));

    // Undo the first one too: no profile, so an export checks nothing and reports no failure.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.custom_prop(crate::checks::PROP_PROFILE), None);
    let findings = s.run_export_checks(&wordcraft_engine::io::ExportOptions::default(), "docx");
    assert!(findings.is_empty(), "{findings:?}");
}

/// Each check keeps its own findings: court checks don't wipe another check's, and the other way.
#[test]
fn court_check_findings_sit_beside_other_checks_findings() {
    let mut s = apply_session(MISSOURI, "brief");
    s.set_lint_findings("law.other", vec![wordcraft_engine::lint::Finding::error("otherRule", "from another check")]);
    run(&mut s, "law.checks.run", json!({}));
    let all = s.lint_findings();
    assert!(all.iter().any(|f| f["checkId"] == json!("otherRule")), "the other check's finding stays");
    let court = run(&mut s, "law.checks.findings", json!({}));
    assert!(court.as_array().is_some_and(|a| !a.is_empty() && a.iter().all(|f| f["checkId"] != json!("otherRule"))), "{court}");
}

/// Turning LawCraft off takes back what it added: its commands, its export checks and its findings.
#[test]
fn turning_lawcraft_off_leaves_nothing_behind() {
    let mut s = apply_session(MISSOURI, "brief");
    run(&mut s, "law.checks.run", json!({}));
    assert!(!s.lint_findings().is_empty());
    s.prefs.modules.remove(crate::MODULE);
    crate::install(&mut s);
    assert!(s.registry.all().iter().all(|c| !c.id.starts_with("law.")), "no law.* command is left");
    assert!(s.lint_findings().is_empty(), "its findings are cleared");
    let findings = s.run_export_checks(&wordcraft_engine::io::ExportOptions::default(), "pdf");
    assert!(findings.is_empty(), "no court check runs on export: {findings:?}");
    // On again: everything comes back, with the document's profile.
    s.prefs.modules.insert(crate::MODULE.into(), true);
    crate::install(&mut s);
    assert!(s.registry.get("law.checks.run").is_some());
    assert!(!s.run_export_checks(&wordcraft_engine::io::ExportOptions::default(), "docx").is_empty());
}
