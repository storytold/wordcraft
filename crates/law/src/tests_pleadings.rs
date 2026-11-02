//! `law.pleadings.new` and the firm details: the skeleton each pleading type writes, how it
//! behaves with undo, Track Changes, .docx and PDF, and the firm details it signs with.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use wordcraft_doc::props::{BorderStyle, VAlign};
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::{CmdError, Session};

use super::tests::{err, law, run};
use crate::cmd::store;
use crate::pleadings::{NOT_BLANK, PROFILE, join_names, long_date};
use crate::{MODULE, install};

fn firm() -> Value {
    json!({
        "firmName": "Example Law Firm LLC",
        "address1": "100 Main Street",
        "address2": "Suite 2",
        "city": "Kirksville",
        "state": "MO",
        "zip": "63501",
        "phone": "660-555-0100",
        "fax": "660-555-0101",
        "email": "attorney@example.com",
        "attorneyName": "Jane Q. Lawyer",
        "barNumber": "12345"
    })
}

/// A Motion to Withdraw, every param fixed.
fn motion() -> Value {
    json!({
        "type": "motion",
        "title": "Motion for Withdrawal of Attorney",
        "county": "ADAIR",
        "caseNumber": "26AR-DR00001",
        "firstSide": {"names": ["JANE DOE"], "role": "Petitioner"},
        "secondSide": {"names": ["JOHN DOE"], "role": "Respondent"},
        "weRepresent": "first",
        "service": "efileCounsel",
        "filingDate": "2026-10-15",
        "firm": firm()
    })
}

fn of_type(kind: &str) -> Value {
    let mut v = motion();
    v["type"] = json!(kind);
    v
}

/// A blank document with the pleading written.
fn pleading(v: Value) -> (Session, Value) {
    let mut s = law();
    let r = run(&mut s, "law.pleadings.new", v);
    (s, r)
}

/// Every body paragraph's text, in order (table cells included).
fn paras(doc: &Document) -> Vec<String> {
    doc.para_paths(StoryRef::Body).iter().filter_map(|p| doc.para(StoryRef::Body, p)).map(|p| p.plain_text()).collect()
}

fn findings(s: &mut Session, for_export: bool) -> Vec<Value> {
    let r = run(s, "law.checks.run", json!({"forExport": for_export, "store": false}));
    r["findings"].as_array().cloned().unwrap_or_default()
}

fn caption_table(doc: &Document) -> &wordcraft_doc::Table {
    doc.body.iter().find_map(|b| b.as_table()).expect("the caption table")
}

#[test]
fn each_type_writes_its_sections_in_order_and_passes_required_sections() {
    for kind in crate::pleadings::TYPES {
        let (mut s, r) = pleading(of_type(kind));
        let want: Vec<&str> = if kind == "petition" {
            vec!["caption", "title", "body", "signature"]
        } else {
            vec!["caption", "title", "body", "signature", "certificateOfService"]
        };
        assert_eq!(r["sections"], json!(want), "{kind}");
        assert_eq!(r["profile"], json!(PROFILE));
        assert_eq!(r["type"], json!(kind));
        // Recorded like law.profiles.apply, so the checks need no params.
        let f = findings(&mut s, false);
        assert!(f.is_empty(), "{kind}: {f:?}");
    }
    // The check does look: the profile alone, with no skeleton, is missing its sections.
    let mut s = super::tests::apply_session(PROFILE, "motion");
    let f = findings(&mut s, false);
    assert!(f.iter().any(|x| x["checkId"] == json!("requiredSections")), "{f:?}");
}

#[test]
fn a_motion_reads_exactly_as_the_firms_template() {
    let (s, _) = pleading(motion());
    let want = [
        "IN THE CIRCUIT COURT OF ADAIR COUNTY",
        "STATE OF MISSOURI",
        "JANE DOE",
        "Petitioner,",
        "v.",
        "JOHN DOE",
        "Respondent.",
        "Case No. 26AR-DR00001",
        "",
        "Motion for Withdrawal of Attorney",
        "COMES NOW JANE DOE, Petitioner, by and through counsel, and in support of this motion states as follows:",
        "[Grounds.]",
        "[Grounds.]",
        "[Grounds.]",
        "WHEREFORE, Petitioner prays this Court [state the relief requested] and for such other and further relief as the Court deems just and proper.",
        "Respectfully Submitted,",
        "Example Law Firm LLC",
        "100 Main Street",
        "Suite 2",
        "Kirksville, MO 63501",
        "660-555-0100 - ph",
        "660-555-0101 - fax",
        "attorney@example.com",
        "Attorney for Petitioner",
        "By: /s/ Jane Q. Lawyer",
        "Jane Q. Lawyer, MO Bar No. 12345",
        "Certificate of Service",
        "I hereby certify that on this 15th day of October, 2026, an electronic copy of the foregoing was sent through the Missouri e-Filing system to counsel of record.",
        "/s/ Jane Q. Lawyer",
        "Jane Q. Lawyer",
    ];
    assert_eq!(paras(&s.doc), want);
    // Each paragraph carries its profile style, the lead words alone are bold, and the
    // numbered paragraphs are a list: number at 42 pt, text at 72 pt.
    let style_of = |text: &str| -> String {
        let doc = &s.doc;
        let p = doc.para_paths(StoryRef::Body).into_iter().filter_map(|p| doc.para(StoryRef::Body, &p).cloned()).find(|p| p.plain_text() == text);
        let id = p.and_then(|p| p.props.style).unwrap_or_default();
        doc.styles.get(&id).map(|st| st.name.clone()).unwrap_or(id)
    };
    for (text, style) in [
        ("IN THE CIRCUIT COURT OF ADAIR COUNTY", "Court"),
        ("STATE OF MISSOURI", "Court State"),
        ("JANE DOE", "Caption"),
        ("Case No. 26AR-DR00001", "Caption"),
        ("Motion for Withdrawal of Attorney", "Title"),
        ("[Grounds.]", "Numbered"),
        ("Respectfully Submitted,", "Signature"),
        ("Certificate of Service", "Certificate Heading"),
        ("/s/ Jane Q. Lawyer", "Certificate"),
    ] {
        assert_eq!(style_of(text), style, "{text}");
    }
    let body: Vec<_> = s.doc.para_paths(StoryRef::Body).into_iter().filter_map(|p| s.doc.para(StoryRef::Body, &p).cloned()).collect();
    let comes = body.iter().find(|p| p.plain_text().starts_with("COMES NOW")).expect("COMES NOW");
    let bold: String = comes.run_ranges().filter(|(_, c)| c.bold == Some(true)).map(|(r, _)| comes.text[r].to_string()).collect();
    assert_eq!(bold, "COMES NOW");
    let numbered: Vec<_> = body.iter().filter(|p| p.plain_text() == "[Grounds.]").collect();
    assert!(numbered.iter().all(|p| p.props.numbering.is_some_and(|n| n.num != 0)), "numbered");
    assert!(numbered.windows(2).all(|w| w[0].props.numbering.map(|n| n.num) == w[1].props.numbering.map(|n| n.num)), "one list");
    assert!(numbered.iter().all(|p| p.props.indent_left == Some(72.0) && p.props.indent_first == Some(-30.0)));
}

#[test]
fn filing_dates_are_written_long_with_ordinals() {
    for (day, want) in [
        (1, "1st"),
        (2, "2nd"),
        (3, "3rd"),
        (4, "4th"),
        (11, "11th"),
        (12, "12th"),
        (13, "13th"),
        (21, "21st"),
        (22, "22nd"),
        (23, "23rd"),
        (31, "31st"),
    ] {
        assert_eq!(long_date((2026, 10, day)), format!("{want} day of October, 2026"));
    }
    // A date that doesn't exist is refused.
    let mut v = motion();
    v["filingDate"] = json!("2026-02-30");
    let mut s = law();
    assert!(err(&mut s, "law.pleadings.new", v).to_string().contains("filingDate"));
}

#[test]
fn several_parties_are_listed_and_joined() {
    let mut v = motion();
    v["firstSide"]["names"] = json!(["JANE DOE", "JIM DOE"]);
    let (s, _) = pleading(v);
    let text = paras(&s.doc);
    let jane = text.iter().position(|t| t == "JANE DOE").expect("first name");
    assert_eq!(text.get(jane + 1).map(String::as_str), Some("JIM DOE"), "one line each");
    assert!(text.iter().any(|t| t.starts_with("COMES NOW JANE DOE and JIM DOE, Petitioner,")), "{text:?}");
    let three = ["A".to_string(), "B".to_string(), "C".to_string()];
    assert_eq!(join_names(&three), "A, B, and C");
    assert_eq!(join_names(&three[..2]), "A and B");
}

#[test]
fn usps_service_adds_the_mailing_sentence_and_needs_party_and_address() {
    let mut v = motion();
    v["service"] = json!("efileUsps");
    v["servedParty"] = json!("John Doe");
    v["servedAddress"] = json!("1 Elm Street, Macon, MO 63552");
    let (s, _) = pleading(v.clone());
    let want = "I hereby certify that on this 15th day of October, 2026, an electronic copy of the foregoing was sent through the Missouri e-Filing system to counsel of record. A copy of this Motion for Withdrawal of Attorney was sent to John Doe postage pre-paid via USPS to last known address of 1 Elm Street, Macon, MO 63552, on today's date.";
    assert!(paras(&s.doc).iter().any(|t| t == want));
    for missing in ["servedParty", "servedAddress"] {
        let mut v = v.clone();
        v.as_object_mut().expect("params").remove(missing);
        let mut s = law();
        let e = err(&mut s, "law.pleadings.new", v);
        assert!(matches!(&e, CmdError::Params(m) if m.contains(missing)), "{e}");
    }
}

#[test]
fn a_petition_has_no_certificate_and_a_verification_only_when_asked() {
    let mut v = of_type("petition");
    v["title"] = json!("Petition for Dissolution of Marriage");
    v["caseNumber"] = json!("");
    v.as_object_mut().expect("params").remove("service");
    let (s, r) = pleading(v.clone());
    let text = paras(&s.doc);
    assert!(!text.iter().any(|t| t == "Certificate of Service"));
    assert!(!text.iter().any(|t| t == "VERIFICATION"));
    assert!(text.iter().any(|t| t == "Case No. ____________"), "no case number yet");
    assert!(
        text.iter()
            .any(|t| t == "COMES NOW JANE DOE, Petitioner, by and through counsel, and for Petition for Dissolution of Marriage states as follows:")
    );
    assert!(text.iter().any(|t| t == "Grant such other and further relief as the Court deems just and proper."));
    assert_eq!(r["sections"].as_array().map(Vec::len), Some(4));
    v["verified"] = json!(true);
    let (mut s, r) = pleading(v);
    let text = paras(&s.doc);
    assert_eq!(r["sections"].as_array().and_then(|a| a.last()).cloned(), Some(json!("verification")));
    assert!(text.iter().any(|t| t == "VERIFICATION"));
    assert!(text.iter().any(|t| t == "JANE DOE, Petitioner"));
    assert!(text.iter().any(|t| t == "County of Adair"));
    let f = findings(&mut s, false);
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn a_document_with_text_is_refused_and_left_alone() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "Draft notes"}));
    let before = s.doc.clone();
    let steps = s.undo_depth();
    let e = err(&mut s, "law.pleadings.new", motion());
    assert!(matches!(&e, CmdError::Params(m) if m == NOT_BLANK), "{e}");
    assert_eq!(s.doc, before, "nothing changed");
    assert_eq!(s.undo_depth(), steps);
}

#[test]
fn one_undo_takes_the_whole_skeleton_and_the_profile_away() {
    let mut s = law();
    let before = s.doc.clone();
    let steps = s.undo_depth();
    run(&mut s, "law.pleadings.new", motion());
    assert_eq!(s.undo_depth(), steps + 1, "one step");
    assert_eq!(s.undo_label(), Some("New Pleading"));
    s.run("edit.undo", &json!({})).expect("undo");
    assert_eq!(s.doc, before);
}

#[test]
fn with_track_changes_on_the_skeleton_is_tracked() {
    let mut s = law();
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "law.pleadings.new", motion());
    let tracked =
        s.doc.para_paths(StoryRef::Body).iter().filter_map(|p| s.doc.para(StoryRef::Body, p)).any(|p| p.runs.iter().any(|r| r.props.ins.is_some()));
    assert!(tracked, "the typing is tracked");
    run(&mut s, "review.rejectAll", json!({}));
    // ponytail: core has no tracked table insertion, so the empty caption table stays.
    assert!(s.doc.plain_text(StoryRef::Body).trim().is_empty(), "{:?}", s.doc.plain_text(StoryRef::Body));
}

#[test]
fn the_caption_table_has_one_rule_and_a_62_38_split() {
    let (s, _) = pleading(motion());
    let t = caption_table(&s.doc);
    assert_eq!((t.rows.len(), t.rows[0].cells.len()), (1, 2));
    let b = t.props.borders.expect("table borders");
    let rule = b.inside_v.expect("the vertical rule");
    assert_eq!((rule.style, rule.width), (BorderStyle::Single, 1.5));
    for side in [b.top, b.bottom, b.left, b.right] {
        assert_eq!(side.map(|x| x.style), Some(BorderStyle::None));
    }
    assert!(t.rows[0].cells.iter().all(|c| c.props.borders.is_none()), "no cell borders");
    let total: f32 = t.grid.iter().sum();
    assert!((t.grid[0] / total - 0.62).abs() < 0.001 && (t.grid[1] / total - 0.38).abs() < 0.001, "{:?}", t.grid);
    assert!((total - 468.0).abs() < 0.01, "the text width");
    assert_eq!(t.rows[0].cells[0].props.margins, Some([8.0, 0.0, 8.0, 20.0]));
    assert_eq!(t.rows[0].cells[1].props.margins, Some([8.0, 20.0, 8.0, 0.0]));
    assert!(t.rows[0].cells.iter().all(|c| c.props.valign == Some(VAlign::Center)));
}

#[test]
fn a_docx_round_trip_keeps_table_styles_and_text() {
    let (s, _) = pleading(motion());
    let bytes = wordcraft_engine::io::save_bytes("pleading.docx", &s.doc).expect("save");
    let doc = wordcraft_engine::io::open_bytes("pleading.docx", &bytes).expect("reopen");
    assert_eq!(paras(&doc), paras(&s.doc));
    let t = caption_table(&doc);
    assert_eq!(t.props.borders.and_then(|b| b.inside_v).map(|r| r.width), Some(1.5));
    assert_eq!(t.rows[0].cells[0].props.margins, Some([8.0, 0.0, 8.0, 20.0]));
    let title = doc.styles.find("Title").expect("Title");
    assert_eq!((title.chr.bold, title.chr.caps, title.chr.size), (Some(true), Some(true), Some(12.0)));
    let sig = doc.styles.find("Signature").expect("Signature");
    assert_eq!(sig.para.indent_left, Some(234.0));
    assert!(doc.styles.find("Court").is_some() && doc.styles.find("Certificate Heading").is_some());
}

#[test]
fn the_motion_exports_to_a_searchable_pdf() {
    let (mut s, _) = pleading(motion());
    let f = findings(&mut s, true);
    assert!(!f.iter().any(|x| x["checkId"] == json!("textSearchablePdf")), "{f:?}");
}

#[test]
fn firm_details_are_saved_checked_and_used() {
    let mut s = law();
    // Checked before anything is saved.
    let mut unknown = firm();
    unknown["website"] = json!("example.com");
    assert!(err(&mut s, "law.firm.set", unknown).to_string().contains("website"));
    let mut long = firm();
    long["firmName"] = json!("x".repeat(201));
    assert!(err(&mut s, "law.firm.set", long).to_string().contains("firmName"));
    run(&mut s, "law.firm.set", firm());
    assert_eq!(run(&mut s, "law.firm.get", json!({})), firm());
    // A pleading without `firm` signs with the saved details.
    let mut v = motion();
    v.as_object_mut().expect("params").remove("firm");
    let (doc, _) = pleading(v.clone());
    assert!(paras(&doc.doc).iter().any(|t| t == "Jane Q. Lawyer, MO Bar No. 12345"));
    // With none saved, it names what is missing.
    store().set_firm(BTreeMap::new()).expect("clear");
    assert_eq!(run(&mut s, "law.firm.get", json!({})), json!({}));
    let mut s = law();
    let e = err(&mut s, "law.pleadings.new", v);
    assert!(matches!(&e, CmdError::Params(m) if m.contains("`firm`")), "{e}");
    // A firm block missing a needed field names the field.
    let mut v = motion();
    v["firm"].as_object_mut().expect("firm").remove("barNumber");
    assert!(err(&mut s, "law.pleadings.new", v).to_string().contains("firm.barNumber"));
}

#[test]
fn firm_details_are_a_file_beside_the_profiles_and_not_a_profile() {
    let dir = super::tests::temp_dir("firm");
    let mut store = crate::store::Store::new(Some(dir.clone()));
    let saved = crate::pleadings::firm_fields(&firm()).expect("firm");
    store.set_firm(saved.clone()).expect("save");
    assert!(dir.join("firm.json").exists());
    let reopened = crate::store::Store::new(Some(dir.clone()));
    assert_eq!(reopened.firm(), &saved);
    assert_eq!(reopened.list().len(), crate::store::bundled().len(), "firm.json is not a profile");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn with_lawcraft_off_the_pleading_commands_are_not_there() {
    let mut s = Session::new(Document::new());
    assert_eq!(install(&mut s), 0);
    for id in ["law.pleadings.new", "law.firm.get", "law.firm.set"] {
        assert!(matches!(s.run(id, &motion()), Err(CmdError::Unknown(_))), "{id}");
    }
    s.prefs.modules.insert(MODULE.into(), true);
    install(&mut s);
    s.prefs.modules.insert(MODULE.into(), false);
    install(&mut s);
    assert!(matches!(s.run("law.pleadings.new", &json!({})), Err(CmdError::Unknown(_))), "and gone again when turned off");
}

#[test]
fn no_params_asks_the_interface_for_the_dialog() {
    let mut s = law();
    let r = run(&mut s, "law.pleadings.new", json!({}));
    assert_eq!(r, json!({"pending": "newPleading"}));
    assert_eq!(s.ui_requests, vec![json!({"addin": "law", "open": "newPleading"})]);
    assert!(s.doc.plain_text(StoryRef::Body).is_empty(), "nothing written");
}

#[test]
fn the_profile_ships_verified_and_the_appellate_ones_are_unchanged() {
    let mut s = law();
    let list = run(&mut s, "law.profiles.list", json!({}));
    let p = list.as_array().expect("list").iter().find(|p| p["id"] == json!(PROFILE)).cloned().expect("listed");
    assert_eq!(p["docs"], json!(["answer", "motion", "notice", "petition"]));
    let profile = store().get(PROFILE).cloned().expect("bundled");
    assert!(profile.verified);
    assert_eq!(profile.jurisdiction, "Missouri");
    assert!(crate::profile::problems(&profile).is_empty(), "{:?}", crate::profile::problems(&profile));
    // FNV-1a of the files as they shipped before this profile: not one byte changed.
    let fnv = |b: &[u8]| b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, c| (h ^ u64::from(*c)).wrapping_mul(0x0100_0000_01b3));
    let missouri = include_bytes!("../profiles/missouri-appellate-brief.json");
    let federal = include_bytes!("../profiles/federal-appellate-brief.json");
    assert_eq!((missouri.len(), fnv(missouri)), (25_791, 0xbfb3_daa0_1124_9dae));
    assert_eq!((federal.len(), fnv(federal)), (11_588, 0x1dae_1ea8_9700_bceb));
}

#[test]
fn a_match_style_must_name_a_style() {
    let mut p = store().get(PROFILE).cloned().expect("bundled");
    if let Some(sec) = p.docs.get_mut("motion").and_then(|d| d.sections.first_mut()) {
        sec.match_style = Some("Nope".into());
    }
    let problems = crate::profile::problems(&p);
    assert!(problems.iter().any(|m| m.contains("matchStyle `Nope`")), "{problems:?}");
}
