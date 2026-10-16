//! Table of authorities: the finder marks citations with Word's TA fields by category and
//! WordCraft's own TOA fields list them.

use serde_json::json;
use wordcraft_doc::StoryRef;
use wordcraft_doc::para::InlineObject;
use wordcraft_engine::Session;

use super::tests::{law, roundtrip, run};

const CASE: &str = "570 S.W.2d 679 (Mo. banc 1978)";
const STATUTE: &str = "Section 537.080, RSMo";

fn insert(s: &mut Session, text: &str) {
    run(s, "text.insert", json!({"text": text}));
}

fn brief() -> Session {
    let mut s = law();
    insert(&mut s, &format!("The court held in Thummel v. King, {CASE}, that it applies."));
    run(&mut s, "text.newParagraph", json!({}));
    insert(&mut s, &format!("The statute, {STATUTE}, says so."));
    run(&mut s, "text.newParagraph", json!({}));
    insert(&mut s, "Argument");
    run(&mut s, "caret.docStart", json!({}));
    s
}

/// Field instructions in the body, in order.
fn fields(s: &Session) -> Vec<String> {
    let mut out = Vec::new();
    for b in &s.doc.body {
        if let Some(p) = b.as_para() {
            for o in &p.objects {
                if let InlineObject::Field { instr, .. } = o {
                    out.push(instr.clone());
                }
            }
        }
    }
    out
}

#[test]
fn citations_get_word_marks_and_a_table_per_category() {
    let mut s = brief();
    let r = run(&mut s, "law.toa.build", json!({"pack": "missouri"}));
    assert_eq!(r["marked"], json!(2), "{r}");
    assert_eq!(r["inserted"], json!([1, 2]), "{r}");
    let f = fields(&s);
    assert!(f.contains(&format!("TA \\l \"{CASE}\" \\c 1")), "{f:?}");
    assert!(f.contains(&format!("TA \\l \"{STATUTE}\" \\c 2")), "{f:?}");
    assert!(f.contains(&"TOA \\c \"1\" \\p".to_string()) && f.contains(&"TOA \\c \"2\" \\p".to_string()), "{f:?}");
    let t = s.doc.plain_text(StoryRef::Body);
    let cases = t.find("Cases").expect("Cases heading");
    let statutes = t.find("Statutes").expect("Statutes heading");
    assert!(cases < statutes && statutes < t.find("The court held").expect("text"), "the table goes at the caret: {t}");
    assert!(t[cases..statutes].contains(&format!("{CASE}\t1")), "{t}");
    assert!(t[statutes..].contains(&format!("{STATUTE}\t1")), "{t}");
    assert!(!t[cases..statutes].contains(STATUTE), "a statute isn't listed under Cases: {t}");
}

#[test]
fn building_again_updates_without_marking_twice() {
    let mut s = brief();
    run(&mut s, "law.toa.build", json!({"pack": "missouri"}));
    let before = fields(&s);
    let r = run(&mut s, "law.toa.build", json!({"pack": "missouri"}));
    assert_eq!(r["marked"], json!(0), "the table's own lines aren't citations to mark: {r}");
    assert_eq!(r["updated"], json!(true));
    assert_eq!(fields(&s), before);
    // A citation added later is marked and listed by the next build.
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    insert(&mut s, "Also Smith v. Jones, 600 S.W.3d 100 (Mo. App. E.D. 2020).");
    let r = run(&mut s, "law.toa.build", json!({"pack": "missouri"}));
    assert_eq!(r["marked"], json!(1), "{r}");
    assert!(s.doc.plain_text(StoryRef::Body).contains("600 S.W.3d 100 (Mo. App. E.D. 2020)\t1"));
}

#[test]
fn update_table_keeps_the_table_current() {
    let mut s = brief();
    run(&mut s, "law.toa.build", json!({"pack": "missouri"}));
    // Push the citations to page 2: References › Update Table follows them.
    run(&mut s, "select.text", json!({"text": "The court held"}));
    run(&mut s, "caret.home", json!({}));
    run(&mut s, "text.pageBreak", json!({}));
    run(&mut s, "references.updateAuthorities", json!({}));
    let t = s.doc.plain_text(StoryRef::Body);
    assert!(t.contains(&format!("{CASE}\t2")), "{t}");
}

#[test]
fn passim_for_an_authority_on_five_pages_and_off_on_request() {
    let mut s = brief();
    for _ in 0..5 {
        run(&mut s, "caret.docEnd", json!({}));
        run(&mut s, "text.pageBreak", json!({}));
        run(&mut s, "text.newParagraph", json!({}));
        insert(&mut s, &format!("Again, {CASE}."));
    }
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "law.toa.build", json!({"pack": "missouri"}));
    assert!(s.doc.plain_text(StoryRef::Body).contains(&format!("{CASE}\tpassim")));

    let mut s = brief();
    for _ in 0..5 {
        run(&mut s, "caret.docEnd", json!({}));
        run(&mut s, "text.pageBreak", json!({}));
        run(&mut s, "text.newParagraph", json!({}));
        insert(&mut s, &format!("Again, {CASE}."));
    }
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "law.toa.build", json!({"pack": "missouri", "passim": false}));
    let t = s.doc.plain_text(StoryRef::Body);
    assert!(!t.contains("passim") && t.contains(&format!("{CASE}\t1, 2, 3, 4, 5, 6")), "{t}");
}

#[test]
fn marks_and_table_survive_a_docx_round_trip() {
    let mut s = brief();
    run(&mut s, "law.toa.build", json!({"pack": "missouri"}));
    let before = fields(&s);
    let mut back = Session::new(roundtrip(&s));
    assert_eq!(fields(&back), before);
    back.run("references.updateAuthorities", &json!({})).expect("update");
    assert!(back.doc.plain_text(StoryRef::Body).contains(&format!("{CASE}\t1")));
}

#[test]
fn bad_parameters_and_no_citations() {
    let mut s = brief();
    assert!(s.run("law.toa.build", &json!({"pack": "missouri", "at": "end"})).is_err());
    assert!(s.run("law.toa.build", &json!({"pack": "no-such-pack"})).is_err());
    assert!(s.run("law.toa.build", &json!({})).is_err(), "no pack and no profile applied");
    let mut empty = law();
    insert(&mut empty, "Nothing cited here.");
    let r = run(&mut empty, "law.toa.build", json!({"pack": "missouri"}));
    assert_eq!(r["inserted"], json!([]));
    assert!(fields(&empty).is_empty());
}

/// A cases-only table, then the brief's first statute: building again adds a Statutes table
/// instead of only updating the Cases one.
#[test]
fn rebuilding_adds_a_category_the_table_did_not_have() {
    let mut s = law();
    insert(&mut s, &format!("The court held in Thummel v. King, {CASE}, that it applies."));
    run(&mut s, "text.newParagraph", json!({}));
    insert(&mut s, "Argument");
    run(&mut s, "caret.docStart", json!({}));
    let r = run(&mut s, "law.toa.build", json!({"pack": "missouri"}));
    assert_eq!(r["inserted"], json!([1]));
    assert!(!s.doc.plain_text(StoryRef::Body).contains("Statutes"));

    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    insert(&mut s, &format!("The statute, {STATUTE}, says so."));
    let r = run(&mut s, "law.toa.build", json!({"pack": "missouri"}));
    assert_eq!(r["inserted"], json!([2]), "{r}");
    let t = s.doc.plain_text(StoryRef::Body);
    let statutes = t.find("Statutes").expect("a Statutes table");
    assert!(t[statutes..].contains(&format!("{STATUTE}\t1")), "{t}");
    assert!(fields(&s).contains(&"TOA \\c \"2\" \\p".to_string()));
    // Building once more adds nothing new.
    assert_eq!(run(&mut s, "law.toa.build", json!({"pack": "missouri"}))["inserted"], json!([]));
}
