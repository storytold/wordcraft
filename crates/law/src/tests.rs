//! LawCraft's tests: the commands, the store's verification rules, applying a profile (through
//! WordCraft's own commands, checked in a saved and reopened .docx) and hostile input.

use serde_json::{Value, json};
use std::path::PathBuf;
use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::section::LineNumberRestart;
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::layout::Placed;
use wordcraft_engine::{CmdError, Session};

use crate::cmd::store;
use crate::profile::{Profile, SCHEMA};
use crate::{MODULE, install, specs};

pub(crate) const MISSOURI: &str = "missouri-appellate-brief";
pub(crate) const FEDERAL: &str = "federal-appellate-brief";

/// A session with the module on, over a blank document.
pub(crate) fn law() -> Session {
    let mut s = Session::new(Document::new());
    s.prefs.modules.insert(MODULE.into(), true);
    assert_eq!(install(&mut s), specs().len(), "every law command is installed");
    s
}

pub(crate) fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

pub(crate) fn err(s: &mut Session, id: &str, v: Value) -> CmdError {
    s.run(id, &v).expect_err(&format!("{id} should have failed"))
}

/// A blank document with the profile's document type applied, saved and reopened as .docx, so
/// what the profile did is checked the way a user sees it.
fn applied(id: &str, doc_type: &str) -> Document {
    let mut s = law();
    run(&mut s, "law.profiles.apply", json!({"id": id, "doc": doc_type}));
    roundtrip(&s)
}

pub(crate) fn roundtrip(s: &Session) -> Document {
    let bytes = wordcraft_engine::io::save_bytes("law.docx", &s.doc).expect("save a .docx");
    wordcraft_engine::io::open_bytes("law.docx", &bytes).expect("reopen it")
}

/// Page labels as layout and the table of contents use them (section prefix + page number).
fn layout_page_labels(s: &mut Session) -> Vec<String> {
    let l = s.layout();
    let sections = s.doc.sections();
    l.pages.iter().map(|p| sections.get(p.section).map(|(_, sp)| sp.format_page_number(p.number)).unwrap_or_else(|| p.number.to_string())).collect()
}

/// The text a page's footer actually shows, with its fields filled in for that page (the prefix is
/// literal text, the page number is the laid-out PAGE field's text).
fn rendered_footer(s: &mut Session, page: usize) -> String {
    let l = s.layout();
    let Some(p) = l.pages.get(page) else { return String::new() };
    let mut out = String::new();
    for it in &p.footer {
        let Placed::Lines { story, path, para, l0, l1, .. } = it else { continue };
        let src = s.doc.para(*story, path);
        for li in *l0..*l1 {
            let Some(line) = para.lines.get(li) else { continue };
            for k in line.c0..line.c1 {
                if let Some((_, t, _)) = para.shown.iter().find(|(sk, _, _)| *sk == k) {
                    out.push_str(t);
                } else if let (Some(c), Some(src)) = (para.clusters.get(k), src)
                    && let Some(t) = src.text.get(c.start..c.end)
                {
                    out.push_str(t);
                }
            }
        }
    }
    out
}

fn session_from_doc(doc: Document) -> Session {
    let mut s = Session::new(doc);
    s.prefs.modules.insert(MODULE.into(), true);
    install(&mut s);
    s
}

/// The document's only section.
fn section(doc: &Document) -> wordcraft_doc::section::SectionProps {
    doc.sections().into_iter().next().map(|(_, s)| s.clone()).expect("a section")
}

/// A style by name.
fn style<'a>(doc: &'a Document, name: &str) -> &'a wordcraft_doc::styles::Style {
    doc.styles.find(name).unwrap_or_else(|| panic!("no style `{name}`"))
}

/// A profile in the shape `law.profiles.create` takes it.
pub(crate) fn profile_json(id: &str) -> Value {
    json!({
        "schema": SCHEMA,
        "id": id,
        "name": "Test court profile",
        "jurisdiction": "Test",
        "docs": {
            "brief": {
                "page": {"size": "Letter", "top": 90, "bottom": 90, "left": 90, "right": 90},
                "styles": [{"name": "Normal", "chr": {"size": 12}, "para": {"lineSpacing": {"rule": "multiple", "value": 1}}}],
                "sections": [{"id": "cover", "name": "Cover"}],
                "source": "Test Rule 1"
            }
        }
    })
}

#[test]
fn the_module_is_off_until_the_user_turns_it_on() {
    let mut s = Session::new(Document::new());
    assert!(!s.module_enabled(MODULE));
    assert_eq!(install(&mut s), 0, "nothing is installed while it is off");
    assert!(matches!(s.run("law.profiles.list", &json!({})), Err(CmdError::Unknown(id)) if id == "law.profiles.list"));
    // On, the same command answers.
    s.prefs.modules.insert(MODULE.into(), true);
    assert_eq!(install(&mut s), specs().len());
    let list = run(&mut s, "law.profiles.list", json!({}));
    let ids: Vec<&str> = list.as_array().expect("a list").iter().filter_map(|p| p["id"].as_str()).collect();
    assert!(ids.contains(&MISSOURI) && ids.contains(&FEDERAL), "{ids:?}");
}

#[test]
fn the_profiles_that_ship_are_listed_and_verified() {
    let mut s = law();
    let list = run(&mut s, "law.profiles.list", json!({}));
    let list: Vec<&Value> = list.as_array().expect("a list").iter().filter(|p| p["id"] == json!(MISSOURI) || p["id"] == json!(FEDERAL)).collect();
    assert_eq!(list.len(), 2, "Missouri and federal");
    for p in list {
        assert!(p["docs"].as_array().is_some_and(|d| !d.is_empty()));
    }
    for p in crate::store::bundled() {
        assert!(p.verified, "{} ships verified", p.id);
    }
    let missouri = store().get(MISSOURI).expect("the Missouri profile").clone();
    assert_eq!(missouri.jurisdiction, "Missouri");
    let federal = store().get(FEDERAL).expect("the federal profile").clone();
    assert_eq!(federal.jurisdiction, "Federal (FRAP)");
    // Every rule records where it comes from and when it took effect.
    for p in store().list() {
        for (doc_type, spec) in &p.docs {
            assert!(!spec.source.trim().is_empty(), "{}: {doc_type} has no source", p.id);
            for c in &spec.checks {
                assert!(!c.source.trim().is_empty(), "{}: {doc_type}: a check has no source", p.id);
            }
        }
    }
    // Each profile holds the rules LAWCRAFT.md names.
    let brief = |id: &str| store().get(id).expect("the profile").docs.get("brief").expect("a brief").clone();
    let words = |id: &str| -> u32 { brief(id).checks.iter().find(|c| c.kind == "wordLimit").expect("a word limit").limit.expect("its limit") };
    assert_eq!(words(MISSOURI), 31_000, "Missouri's initial brief");
    assert_eq!(
        store()
            .get(MISSOURI)
            .expect("the profile")
            .docs
            .get("respondentBrief")
            .expect("respondent's brief")
            .checks
            .iter()
            .find(|c| c.kind == "wordLimit")
            .and_then(|c| c.limit),
        Some(27_900)
    );
    assert_eq!(words(FEDERAL), 13_000, "FRAP's principal brief");
    assert_eq!(store().get(FEDERAL).expect("the profile").docs.get("reply").expect("a reply brief").checks[0].limit, Some(6_500));
    let sections = |id: &str| -> Vec<String> { brief(id).sections.iter().map(|s| s.name.clone()).collect() };
    assert_eq!(
        sections(MISSOURI),
        vec![
            "Cover",
            "Table of Contents",
            "Table of Authorities",
            "Jurisdictional Statement",
            "Statement of Facts",
            "Points Relied On",
            "Argument",
            "Conclusion",
            "Certificate of Compliance",
            "Certificate of Service"
        ]
    );
}

#[test]
fn a_profile_comes_back_by_id() {
    let mut s = law();
    let p = run(&mut s, "law.profiles.get", json!({"id": MISSOURI}));
    assert_eq!(p["id"], json!(MISSOURI));
    assert_eq!(p["docs"]["brief"]["page"]["size"], json!("Letter"));
    assert_eq!(err(&mut s, "law.profiles.get", json!({"id": "no-such-court"})).to_string(), "no court profile `no-such-court`");
    assert_eq!(err(&mut s, "law.profiles.get", json!({})).to_string(), "bad parameters: `id` (string) is required");
}

#[test]
fn an_agent_created_profile_is_unverified() {
    let mut s = law();
    // Asked for verified, saved unverified: a person approves profiles, not a command.
    let mut asked = profile_json("test-agent-court");
    asked["verified"] = json!(true);
    let saved = run(&mut s, "law.profiles.create", asked.clone());
    assert_eq!(saved["verified"], json!(false));
    assert_eq!(saved["name"], json!("Test court profile"));
    assert_eq!(store().get("test-agent-court").map(|p| p.verified), Some(false));
    // A second one of the same id is refused, not overwritten.
    assert!(err(&mut s, "law.profiles.create", asked).to_string().contains("already exists"));
    // Creating a second, different one works.
    let other = run(&mut s, "law.profiles.create", profile_json("test-agent-court-2"));
    assert_eq!(other["id"], json!("test-agent-court-2"));
    assert_eq!(other["verified"], json!(false));
}

#[test]
fn updating_a_profile_makes_it_unverified_again() {
    let mut s = law();
    // A profile a person has approved, then an agent changes.
    run(&mut s, "law.profiles.create", profile_json("test-update-court"));
    assert_eq!(run(&mut s, "law.profiles.approve", json!({"id": "test-update-court"}))["verified"], json!(true));
    let updated = run(&mut s, "law.profiles.update", json!({"id": "test-update-court", "name": "Test court profile (mine)"}));
    assert_eq!(updated["name"], json!("Test court profile (mine)"));
    assert_eq!(updated["verified"], json!(false), "a changed profile is unverified again");
    assert_eq!(updated["docs"]["brief"]["page"]["size"], json!("Letter"), "the rest of the profile stayed");
    assert_eq!(updated["jurisdiction"], json!("Test"));
    assert_eq!(store().get("test-update-court").map(|p| p.verified), Some(false));
    // An unknown id is an error, not a new profile.
    assert!(err(&mut s, "law.profiles.update", json!({"id": "test-unknown", "name": "x"})).to_string().contains("no court profile"));
}

#[test]
fn approving_a_profile_is_a_persons_action() {
    let mut s = law();
    run(&mut s, "law.profiles.create", profile_json("test-approve-court"));
    assert_eq!(store().get("test-approve-court").map(|p| p.verified), Some(false));
    let approved = run(&mut s, "law.profiles.approve", json!({"id": "test-approve-court"}));
    assert_eq!(approved["verified"], json!(true));
    assert_eq!(store().get("test-approve-court").map(|p| p.verified), Some(true));
    // Approving a profile that isn't there is an error.
    assert!(err(&mut s, "law.profiles.approve", json!({"id": "test-nothing"})).to_string().contains("no court profile"));
}

#[test]
fn automation_cannot_approve_or_import_verified_profiles() {
    let mut s = law();
    run(&mut s, "law.profiles.create", profile_json("test-auto-court"));
    let dir = temp_dir("auto");
    let verified_file = dir.join("verified.json");
    run(&mut s, "law.profiles.approve", json!({"id": "test-auto-court"}));
    run(&mut s, "law.profiles.export", json!({"id": "test-auto-court", "path": verified_file.to_string_lossy()}));
    wordcraft_engine::file_access::invoke_automation(|| {
        let e = err(&mut s, "law.profiles.approve", json!({"id": "test-auto-court"})).to_string();
        assert!(e.contains("outside automation"), "{e}");
        let imported = run(&mut s, "law.profiles.import", json!({"path": verified_file.to_string_lossy(), "verified": true}));
        assert_eq!(imported["verified"], json!(false), "automation cannot grant verification");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// A confined agent session (filesystem roots in force, without the automation marker) must not
/// turn an import into verification — neither from the file's own flag nor from an explicit
/// override — and the store must hold the profile unverified.
#[test]
fn a_confined_agent_cannot_import_a_verified_profile() {
    let mut s = law();
    let dir = temp_dir("confined-import");
    let read_root = dir.join("in");
    std::fs::create_dir_all(&read_root).expect("the read root");
    let file = read_root.join("verified.json");
    // A verified profile written by a person, inside what will become the read root.
    run(&mut s, "law.profiles.create", profile_json("test-confined-court"));
    run(&mut s, "law.profiles.approve", json!({"id": "test-confined-court"}));
    run(&mut s, "law.profiles.export", json!({"id": "test-confined-court", "path": file.to_string_lossy()}));
    let text = std::fs::read_to_string(&file).expect("the exported file");
    assert!(text.contains("\"verified\": true"), "the file carries the verified flag: {text}");

    let roots = wordcraft_engine::file_access::AutomationRoots::new(Some(read_root.as_path()), None).expect("roots").expect("roots");
    wordcraft_engine::file_access::confine(Some(&roots), || {
        assert!(wordcraft_engine::file_access::automation_confined());
        assert!(!wordcraft_engine::file_access::automation_invoked(), "roots alone confine the agent");
        // The file's own `verified: true` does not survive...
        let imported = run(&mut s, "law.profiles.import", json!({"path": file.to_string_lossy()}));
        assert_eq!(imported["verified"], json!(false), "a confined agent cannot import a verified profile");
        // ...and the explicit override does not grant it either...
        let imported = run(&mut s, "law.profiles.import", json!({"path": file.to_string_lossy(), "verified": true}));
        assert_eq!(imported["verified"], json!(false), "an explicit override is refused under confinement");
        // ...and what the store holds is unverified.
        assert_eq!(store().get("test-confined-court").map(|p| p.verified), Some(false));
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missouri_appendix_pages_are_numbered_from_a1() {
    let mut s = apply_session(MISSOURI, "appendix");
    let sect = section(&s.doc);
    assert_eq!(sect.page_num_start, Some(1));
    assert_eq!(sect.page_num_prefix.as_deref(), Some("A"));
    let footer = sect.footers.default.expect("a footer");
    let text = s.doc.plain_text(wordcraft_doc::StoryRef::Part(footer));
    assert_eq!(text, "A1", "appendix pages are numbered A1, A2…: {text:?}");
    assert_eq!(layout_page_labels(&mut s)[0], "A1", "layout page 1");
    assert_eq!(rendered_footer(&mut s, 0), "A1", "rendered footer on page 1");
    run(&mut s, "text.pageBreak", json!({}));
    let labels = layout_page_labels(&mut s);
    assert!(labels.len() >= 2, "at least two pages: {labels:?}");
    assert_eq!(labels[0], "A1");
    assert_eq!(labels[1], "A2", "layout page 2");
    assert_eq!(rendered_footer(&mut s, 1), "A2", "rendered footer on page 2");
    let doc = roundtrip(&s);
    let sect = section(&doc);
    assert_eq!(sect.page_num_start, Some(1), "page numbering restarts at 1 in a saved .docx");
    assert_eq!(sect.page_num_prefix.as_deref(), Some("A"), "the section prefix survives .docx");
    let footer = sect.footers.default.expect("a footer after round-trip");
    let text = doc.plain_text(wordcraft_doc::StoryRef::Part(footer));
    assert_eq!(text, "A1", "the A prefix survives round-trip: {text:?}");
    let mut reopened = session_from_doc(doc);
    let after = layout_page_labels(&mut reopened);
    assert_eq!(after[0], "A1", "layout after .docx round-trip");
    assert_eq!(after[1], "A2", "layout page 2 after .docx round-trip");
    assert_eq!(rendered_footer(&mut reopened, 0), "A1", "rendered footer after .docx round-trip");
    assert_eq!(rendered_footer(&mut reopened, 1), "A2", "rendered footer page 2 after .docx round-trip");
}

/// Creating an appendix opens it in place of the brief, so an interface asks to save the brief
/// first, as it does for File › New.
#[test]
fn law_appendix_create_asks_to_save_the_brief_first() {
    let spec = crate::specs().into_iter().find(|c| c.id == "law.appendix.create").expect("the command");
    assert!(spec.replaces_document);
    assert!(!crate::specs().into_iter().find(|c| c.id == "law.appendix.apply").expect("the command").replaces_document);
}

#[test]
fn law_appendix_create_opens_a_numbered_appendix_with_toc() {
    let mut s = apply_session(MISSOURI, "brief");
    let created = run(&mut s, "law.appendix.create", json!({}));
    assert_eq!(created["profile"], json!(MISSOURI));
    assert_eq!(created["doc"], json!("appendix"));
    assert_eq!(s.doc.custom_prop(crate::checks::PROP_DOC_TYPE), Some("appendix"));
    let sect = section(&s.doc);
    assert_eq!(sect.page_num_prefix.as_deref(), Some("A"));
    assert_eq!(s.doc.plain_text(StoryRef::Part(sect.footers.default.expect("footer"))), "A1");
    // The TOC is complete as created: the skeleton's own headings are listed with A-numbers,
    // without a separate update command.
    let body = s.doc.plain_text(StoryRef::Body);
    assert!(body.contains("Appendix cover\tA1"), "the cover heading is in the created TOC: {body}");
    assert!(body.contains("Exhibits\tA1"), "the exhibits heading is in the created TOC: {body}");
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.pageBreak", json!({}));
    run(&mut s, "text.insert", json!({"text": "Exhibit A"}));
    run(&mut s, "para.style", json!({"style": "Appendix Heading"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "references.updateToc", json!({}));
    let body = s.doc.plain_text(StoryRef::Body);
    assert!(body.contains("Exhibit A"), "{body}");
    assert!(body.contains("Exhibit A\tA2"), "TOC lists the exhibit on appendix page A2: {body}");
    let findings = run(&mut s, "law.checks.run", json!({"store": false}));
    assert!(findings["findings"].as_array().is_some_and(|f| f.is_empty()), "{findings}");
}

#[test]
fn law_appendix_apply_restyles_the_open_document() {
    let mut s = law();
    run(&mut s, "law.appendix.apply", json!({"id": MISSOURI}));
    assert_eq!(s.doc.custom_prop(crate::checks::PROP_DOC_TYPE), Some("appendix"));
    assert_eq!(section(&s.doc).page_num_prefix.as_deref(), Some("A"));
    let e = err(&mut s, "law.appendix.apply", json!({"id": MISSOURI, "doc": "brief"})).to_string();
    assert!(e.contains("appendix") && e.contains("addendum"), "{e}");
}

#[test]
fn federal_addendum_pages_use_add_prefix() {
    let mut s = apply_session(FEDERAL, "addendum");
    let sect = section(&s.doc);
    assert_eq!(sect.page_num_prefix.as_deref(), Some("Add. "));
    let text = s.doc.plain_text(StoryRef::Part(sect.footers.default.expect("footer")));
    assert_eq!(text, "Add. 1", "federal addendum numbering: {text:?}");
    assert_eq!(rendered_footer(&mut s, 0), "Add. 1", "rendered federal addendum footer");
    let doc = roundtrip(&s);
    let sect = section(&doc);
    assert_eq!(sect.page_num_prefix.as_deref(), Some("Add. "));
    let text = doc.plain_text(StoryRef::Part(sect.footers.default.expect("footer")));
    assert_eq!(text, "Add. 1");
    assert_eq!(rendered_footer(&mut session_from_doc(doc), 0), "Add. 1", "rendered federal addendum footer after .docx round-trip");
}

#[test]
fn forward_style_references_are_applied() {
    let mut s = law();
    let id = "test-forward-styles";
    let p = json!({
        "schema": SCHEMA,
        "id": id,
        "name": "Forward refs",
        "jurisdiction": "Test",
        "docs": {
            "brief": {
                "page": {"size": "Letter"},
                "styles": [
                    {"name": "Caption", "basedOn": "Normal", "next": "Body", "para": {"lineSpacing": {"rule": "multiple", "value": 1}}},
                    {"name": "Body", "basedOn": "Normal", "next": "Body", "chr": {"size": 12}}
                ],
                "sections": [{"id": "cover", "name": "Cover"}],
                "source": "Test"
            }
        }
    });
    run(&mut s, "law.profiles.create", p);
    run(&mut s, "law.profiles.apply", json!({"id": id, "doc": "brief"}));
    let caption = style(&s.doc, "Caption");
    let body_id = s.doc.styles.find("Body").expect("Body").id.clone();
    assert_eq!(caption.next.as_deref(), Some(body_id.as_str()));
    assert_eq!(caption.based_on.as_deref(), Some("Normal"));
}

#[test]
fn validating_reports_every_problem_and_saves_nothing() {
    let mut s = law();
    let ok = run(&mut s, "law.profiles.validate", profile_json("test-never-saved"));
    assert_eq!(ok["ok"], json!(true));
    assert_eq!(ok["id"], json!("test-never-saved"));
    assert!(store().get("test-never-saved").is_none(), "validating has no side effects");

    let bad = json!({
        "schema": SCHEMA,
        "id": "test/bad",
        "name": "",
        "jurisdiction": "Test",
        "docs": {
            "brief": {
                "page": {"size": "Letter"},
                "styles": [{"name": "Body", "basedOn": "Nowhere"}],
                "sections": [{"id": "cover", "name": "Cover", "style": "Missing"}],
                "checks": [{"kind": "wordLimit", "limit": 0}, {"kind": "nonsense"}],
                "source": ""
            }
        }
    });
    let e = err(&mut s, "law.profiles.validate", bad).to_string();
    for want in [
        "may hold only letters, digits",
        "`name` is empty",
        "basedOn `Nowhere`",
        "style `Missing`",
        "limit 0 is outside",
        "kind `nonsense`",
        "docs.brief.source is empty",
    ] {
        assert!(e.contains(want), "{want} not reported in:\n{e}");
    }
    // An unknown schema version is refused outright, not guessed at.
    let future = json!({"schema": SCHEMA + 1, "id": "x", "name": "X", "jurisdiction": "Y", "docs": {}});
    assert!(err(&mut s, "law.profiles.validate", future).to_string().contains("unsupported schema version 2"));
    // And a file can be validated by path, with nothing saved.
    let path = write_temp("test-validate.json", &profile_json("test-validate-file").to_string());
    let from_file = run(&mut s, "law.profiles.validate", json!({"path": path.to_string_lossy()}));
    assert_eq!(from_file["id"], json!("test-validate-file"));
    assert!(store().get("test-validate-file").is_none(), "validating a file saves nothing");
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_profile_file_can_be_exported_and_imported() {
    let mut s = law();
    run(&mut s, "law.profiles.create", profile_json("test-export-court"));
    let dir = temp_dir("export");
    let path = dir.join("test-export-court.json");
    run(&mut s, "law.profiles.export", json!({"id": "test-export-court", "path": path.to_string_lossy()}));
    assert!(path.exists());
    // The file is a profile file: it validates, and importing it brings it back.
    assert_eq!(run(&mut s, "law.profiles.validate", json!({"path": path.to_string_lossy()}))["id"], json!("test-export-court"));
    let back = run(&mut s, "law.profiles.import", json!({"path": path.to_string_lossy()}));
    assert_eq!(back["id"], json!("test-export-court"));
    // Importing a verified profile keeps it verified: the file is the person's own data.
    run(&mut s, "law.profiles.approve", json!({"id": "test-export-court"}));
    let approved = dir.join("approved.json");
    run(&mut s, "law.profiles.export", json!({"id": "test-export-court", "path": approved.to_string_lossy()}));
    run(&mut s, "law.profiles.update", json!({"id": "test-export-court", "name": "not the approved one any more"}));
    assert_eq!(store().get("test-export-court").map(|p| p.verified), Some(false));
    let restored = run(&mut s, "law.profiles.import", json!({"path": approved.to_string_lossy()}));
    assert_eq!(restored["verified"], json!(true), "the file's own flag is kept");
    assert_eq!(restored["name"], json!("Test court profile"));
    // A bundled profile exported also comes back verified when a person imports it.
    let bundled = dir.join("bundled.json");
    run(&mut s, "law.profiles.export", json!({"id": FEDERAL, "path": bundled.to_string_lossy()}));
    let federal = run(&mut s, "law.profiles.import", json!({"path": bundled.to_string_lossy()}));
    assert_eq!(federal["verified"], json!(true));
    assert_eq!(federal["id"], json!(FEDERAL));
    // `verified` overrides the file's flag when it is given (outside automation).
    let unverified = run(&mut s, "law.profiles.import", json!({"path": bundled.to_string_lossy(), "verified": false}));
    assert_eq!(unverified["verified"], json!(false));
    // A file that isn't a profile is an error that says why.
    let broken = dir.join("broken.json");
    std::fs::write(&broken, "{\"schema\": 99}").expect("write");
    assert!(err(&mut s, "law.profiles.import", json!({"path": broken.to_string_lossy()})).to_string().contains("unsupported schema version 99"));
    assert!(err(&mut s, "law.profiles.export", json!({"id": MISSOURI})).to_string().contains("`path` (string) is required"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn applying_the_missouri_brief_gives_a_missouri_brief() {
    let doc = applied(MISSOURI, "brief");
    // Page: US Letter, margins of at least an inch.
    let sect = section(&doc);
    assert_eq!((sect.page_w, sect.page_h), (612.0, 792.0), "8.5 x 11");
    assert_eq!((sect.margin_top, sect.margin_bottom, sect.margin_left, sect.margin_right), (72.0, 72.0, 72.0, 72.0), "1 inch");
    // No line numbers in an appellate brief.
    assert_eq!(sect.line_numbers, None);
    // Styles: Times New Roman 13 pt, one and a half spaced, with the single-spaced exceptions.
    let normal = style(&doc, "Normal");
    assert_eq!(normal.chr.font.as_deref(), Some("Times New Roman"));
    assert_eq!(normal.chr.size, Some(13.0));
    assert_eq!(normal.chr.size, Some(13.0));
    assert_eq!(normal.para.line_spacing, Some(LineSpacing::Multiple(1.5)));
    for name in ["Caption", "Heading", "Signature", "Certificate"] {
        assert_eq!(style(&doc, name).para.line_spacing, Some(LineSpacing::Multiple(1.0)), "{name} is single spaced");
    }
    assert_eq!(
        style(&doc, "Caption").next.as_deref().map(|n| doc.styles.find(n).map(|s| s.name.as_str())),
        Some(Some("Body")),
        "Caption is followed by Body"
    );
    assert_eq!(style(&doc, "Block Quote").para.indent_left, Some(72.0));
    // Enter after a heading, a quotation or a certificate goes back to the body style.
    for name in ["Body", "Heading", "Block Quote", "Signature", "Certificate"] {
        assert_eq!(style(&doc, name).next.as_deref(), Some("Body"), "{name}");
    }
    // The caption is its own section with its fields, and the skeleton is in order.
    let profile = store().get(MISSOURI).expect("the profile").clone();
    let brief = profile.docs.get("brief").expect("the brief");
    assert_eq!(brief.sections.first().map(|s| s.id.as_str()), Some("cover"));
    assert!(brief.sections.first().and_then(|s| s.fields.first()).is_some(), "the caption holds fields");
    assert_eq!(brief.sections.iter().filter(|s| s.style.as_deref() == Some("Certificate")).count(), 2);
    // The document's own paragraph keeps its style: applying a profile doesn't restyle the text.
    let para = doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).expect("the first paragraph");
    assert_eq!(para.props.style, None);
}

#[test]
fn applying_missouri_reply_brief_heading_is_single_spaced() {
    let doc = applied(MISSOURI, "reply");
    assert_eq!(
        style(&doc, "Heading").para.line_spacing,
        Some(LineSpacing::Multiple(1.0)),
        "reply brief headings are single spaced like the initial brief"
    );
}

/// Every heading style in every bundled profile is single spaced: the court rules allow headings
/// (and captions, quotes and certificates) to be single spaced even where the body is 1.5 or
/// double spaced. A heading that only inherits its parent's line spacing would come back 1.5 or
/// double spaced here — this is the round-trip assertion for all of them, not just one.
#[test]
fn every_bundled_heading_stays_single_spaced() {
    let profiles: Vec<(String, Vec<(String, Vec<String>)>)> = crate::store::bundled()
        .iter()
        .map(|p| {
            let docs = p.docs.iter().map(|(doc_type, spec)| (doc_type.clone(), spec.styles.iter().map(|st| st.name.clone()).collect())).collect();
            (p.id.clone(), docs)
        })
        .collect();
    for (id, docs) in profiles {
        for (doc_type, names) in docs {
            let headings: Vec<String> = names.into_iter().filter(|n| n.contains("Heading")).collect();
            if headings.is_empty() {
                continue;
            }
            let doc = applied(&id, &doc_type);
            for name in headings {
                assert_eq!(
                    style(&doc, &name).para.line_spacing,
                    Some(LineSpacing::Multiple(1.0)),
                    "{id}/{doc_type}: heading `{name}` must be single spaced after a .docx round trip"
                );
            }
        }
    }
}

#[test]
fn cover_numbered_turns_different_first_page_on_and_off() {
    let id = "test-cover-numbered";
    let mut s = law();
    let mut p = profile_json(id);
    p["docs"]["brief"]["pageNumbers"] = json!({"position": "bottom", "align": "center", "xOfY": false, "coverNumbered": false});
    run(&mut s, "law.profiles.create", p);
    // `coverNumbered: false`: the cover is its own, un-numbered first page.
    let applied = run(&mut s, "law.profiles.apply", json!({"id": id}));
    assert!(
        applied["applied"].as_array().is_some_and(|a| a.iter().any(|x| x == "layout.differentFirstPage")),
        "coverNumbered goes through WordCraft's own layout command: {applied}"
    );
    assert!(section(&roundtrip(&s)).title_page, "coverNumbered false turns a different first page on");
    // Flip the setting to `coverNumbered: true` and apply again: every page is numbered now, so
    // different-first-page is turned back off through the same command, and it survives a .docx.
    let mut p = profile_json(id);
    p["docs"]["brief"]["pageNumbers"] = json!({"position": "bottom", "align": "center", "xOfY": false, "coverNumbered": true});
    run(&mut s, "law.profiles.update", json!({"id": id, "docs": p["docs"].clone()}));
    let applied = run(&mut s, "law.profiles.apply", json!({"id": id}));
    assert!(
        applied["applied"].as_array().is_some_and(|a| a.iter().any(|x| x == "layout.differentFirstPage")),
        "coverNumbered true turns it off through the layout command: {applied}"
    );
    assert!(!section(&roundtrip(&s)).title_page, "coverNumbered applies numbering on page 1 too");
}

#[test]
fn applying_the_federal_brief_gives_a_federal_brief() {
    let doc = applied(FEDERAL, "brief");
    let sect = section(&doc);
    assert_eq!((sect.page_w, sect.page_h), (612.0, 792.0), "8.5 x 11");
    assert_eq!((sect.margin_top, sect.margin_bottom, sect.margin_left, sect.margin_right), (72.0, 72.0, 72.0, 72.0), "1 inch");
    assert_eq!(sect.line_numbers, None);
    // FRAP 32(a)(4)(A): proportionally spaced type at 14 point or larger.
    let normal = style(&doc, "Normal");
    assert_eq!(normal.chr.size, Some(14.0));
    assert_eq!(normal.chr.font.as_deref(), Some("Times New Roman"));
    assert_eq!(normal.para.line_spacing, Some(LineSpacing::Multiple(2.0)), "double spaced");
    // Page numbers at the bottom of the page, including the cover.
    let s = apply_session(FEDERAL, "brief");
    let numbers = section(&s.doc);
    assert!(!numbers.title_page, "the cover page is numbered too");
    let footer = numbers.footers.default.expect("a footer");
    let text = s.doc.plain_text(StoryRef::Part(footer));
    assert!(text.contains('1'), "the page number field's result is in the footer: {text:?}");
}

#[test]
fn applying_the_federal_brief_replaces_a_monospaced_normal_style() {
    let mut s = law();
    let normal = s.doc.styles.styles.iter_mut().find(|st| st.name == "Normal").expect("Normal");
    normal.chr.font = Some("Courier New".into());
    run(&mut s, "law.profiles.apply", json!({"id": FEDERAL, "doc": "brief"}));
    let doc = roundtrip(&s);
    let normal = style(&doc, "Normal");
    assert_eq!(normal.chr.font.as_deref(), Some("Times New Roman"), "proportional type from the profile");
    assert_eq!(normal.chr.size, Some(14.0));
}

#[test]
fn a_profile_with_line_numbers_turns_them_on_and_off() {
    // A pleading on numbered lines, the way trial-court paper is.
    let id = "test-line-numbers-court";
    let mut s = law();
    let mut p = profile_json(id);
    p["docs"]["brief"]["lineNumbers"] = json!({"enabled": true, "value": "restartPage", "start": 3, "countBy": 2, "distance": 18});
    run(&mut s, "law.profiles.create", p);
    let doc = applied(id, "brief");
    let numbers = section(&doc).line_numbers.expect("line numbers");
    assert_eq!(numbers.restart, LineNumberRestart::Page);
    assert_eq!(numbers.start, 3);
    assert_eq!(numbers.count_by, 2);
    assert_eq!(numbers.distance, 18.0);
    // A court that doesn't number lines leaves none, even in a document that had them.
    let mut s = apply_session(id, "brief");
    run(&mut s, "law.profiles.apply", json!({"id": MISSOURI, "doc": "brief"}));
    assert_eq!(section(&s.doc).line_numbers, None, "an appellate brief isn't numbered");
}

#[test]
fn applying_a_profile_needs_a_document_type() {
    let mut s = law();
    // The Missouri profile covers several document types, so the caller picks one.
    let e = err(&mut s, "law.profiles.apply", json!({"id": MISSOURI})).to_string();
    assert!(e.contains("`doc` is required"), "{e}");
    assert!(e.contains("brief"), "{e}");
    assert!(e.contains("petition"), "{e}");
    // One document type goes without saying.
    run(&mut s, "law.profiles.create", profile_json("test-one-doc-court"));
    assert!(s.run("law.profiles.apply", &json!({"id": "test-one-doc-court"})).is_ok());
    // An unverified profile is still usable.
    assert_eq!(store().get("test-one-doc-court").map(|p| p.verified), Some(false));
    assert!(s.run("law.profiles.apply", &json!({"id": "test-one-doc-court"})).is_ok());
    // A document type the profile doesn't have is an error that lists the ones it does.
    assert!(err(&mut s, "law.profiles.apply", json!({"id": MISSOURI, "doc": "letter"})).to_string().contains("no document type `letter`"));
    // So is a profile that isn't there.
    assert!(err(&mut s, "law.profiles.apply", json!({"id": "test-nothing", "doc": "brief"})).to_string().contains("no court profile"));
}

#[test]
fn applying_a_small_page_is_refused_rather_than_clamped() {
    let mut s = law();
    let mut p = profile_json("test-tiny-court");
    p["docs"]["brief"]["page"] = json!({"size": "Letter", "top": 400, "bottom": 400});
    run(&mut s, "law.profiles.create", p);
    let e = err(&mut s, "law.profiles.apply", json!({"id": "test-tiny-court"})).to_string();
    assert!(e.contains("layout.margins"), "{e}");
    // The document is left as it was.
    assert_eq!(section(&s.doc).margin_top, 72.0);
}

#[test]
fn applying_a_profile_goes_through_wordcrafts_own_commands() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "An opening line."}));
    let blank = section(&s.doc).clone();
    assert_eq!(blank.footers.default, None, "no page numbers yet");
    let steps = s.undo_depth();
    run(&mut s, "law.profiles.apply", json!({"id": MISSOURI, "doc": "brief"}));
    // The whole application is one undo step, named after the command — not one per command.
    assert_eq!(s.undo_depth(), steps + 1, "one step, not one per command");
    assert_eq!(s.undo_label(), Some("Apply Court Profile"));
    assert_eq!(style(&s.doc, "Normal").chr.size, Some(13.0));
    // Undo puts the document back as it was: the blank document was already US Letter with
    // inch margins, so it is the styles and the page numbering that change.
    s.run("edit.undo", &json!({})).expect("undo");
    assert_eq!(section(&s.doc), blank);
    assert_ne!(style(&s.doc, "Normal").chr.size, Some(13.0), "the document's own style sizes are back");
    // Redo applies it again.
    s.run("edit.redo", &json!({})).expect("redo");
    assert_eq!(style(&s.doc, "Normal").chr.size, Some(13.0));
    assert!(section(&s.doc).footers.default.is_some(), "and the page numbers are back");
}

#[test]
fn every_law_command_survives_hostile_parameters() {
    let junk = [
        json!(null),
        json!({}),
        json!({"id": 5, "path": [], "doc": {"a": 1}}),
        json!({"id": "../../etc/passwd", "path": "/nonexistent/wordcraft-law/x.json"}),
        // A number that doesn't fit, a document type that isn't a string, a style that isn't one.
        json!({"schema": 1e308, "docs": {"brief": 5}, "styles": "x"}),
        json!({"id": "x", "docs": {"../../x": {"page": {"size": {"a": 1}}}}}),
    ];
    for spec in specs() {
        for j in &junk {
            let mut s = law();
            let _ = s.run(spec.id, j);
            let _ = s.layout();
            s.clamp_selection();
        }
    }
}

#[test]
fn hostile_files_are_refused_not_fatal() {
    let mut s = law();
    let dir = temp_dir("files");
    // A file that isn't a profile at all.
    let junk = dir.join("junk.json");
    std::fs::write(&junk, "x".repeat(64 * 1024)).expect("write");
    let huge = dir.join("huge.json");
    std::fs::write(&huge, "x".repeat((crate::store::MAX_PROFILE_BYTES as usize) + 1)).expect("write");
    assert!(err(&mut s, "law.profiles.import", json!({"path": huge.to_string_lossy()})).to_string().contains("larger than"));
    let e = err(&mut s, "law.profiles.import", json!({"path": junk.to_string_lossy()})).to_string();
    assert!(e.contains("not valid JSON"), "{e}");
    // An empty profile (no document types).
    let empty = dir.join("empty.json");
    std::fs::write(&empty, r#"{"schema": 1, "id": "x", "name": "X", "jurisdiction": "Y", "docs": {}}"#).expect("write");
    assert!(err(&mut s, "law.profiles.import", json!({"path": empty.to_string_lossy()})).to_string().contains("`docs` is empty"));
    // A folder, a missing file and a path with nowhere to write are all errors, never a panic.
    assert!(s.run("law.profiles.import", &json!({"path": dir.to_string_lossy()})).is_err());
    assert!(s.run("law.profiles.import", &json!({"path": dir.join("nope.json").to_string_lossy()})).is_err());
    // Export makes the folders it needs, but a path under a file can't be written.
    assert!(s.run("law.profiles.export", &json!({"id": MISSOURI, "path": dir.join("deep/dir/x.json").to_string_lossy()})).is_ok());
    assert!(s.run("law.profiles.export", &json!({"id": MISSOURI, "path": dir.join("junk.json").join("x.json").to_string_lossy()})).is_err());
    // A profile that would be a path traversal never gets a file name of its own.
    let escape = json!({"schema": 1, "id": "../escaped", "name": "X", "jurisdiction": "Y", "docs": {"brief": {"source": "r"}}});
    assert!(err(&mut s, "law.profiles.create", escape).to_string().contains("may hold only"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_profile_json_is_refused_not_fatal() {
    let bad = [
        "",
        "null",
        "[]",
        "{}",
        r#"{"schema": 1}"#,
        r#"{"schema": "1", "id": "x"}"#,
        r#"{"schema": 1, "id": "x", "name": "x", "jurisdiction": "x", "docs": {"brief": {"page": {"width": 1e400, "height": 1}}}}"#,
        r#"{"schema": 1, "id": "x", "name": "x", "jurisdiction": "x", "docs": {"../x": {}}}"#,
        r#"{"schema": 1, "id": "x", "name": "x", "jurisdiction": "x", "docs": {"brief": {"styles": [{"name": "A", "basedOn": "B"}, {"name": "B", "basedOn": "A"}]}}}"#,
    ];
    for text in bad {
        let r = Profile::from_json(text);
        if let Ok(p) = &r {
            assert_eq!(crate::profile::problems(p), Vec::<String>::new(), "{text} parsed but has problems");
        }
    }
    // A circular `basedOn` is reported rather than hanging the style order.
    let r = Profile::from_json(bad.last().expect("the last one")).expect_err("a cycle is not valid");
    assert!(r.join(" ").contains("circular"), "{r:?}");
}

#[test]
fn proptest_any_json_either_parses_or_explains_itself() {
    use proptest::prelude::*;
    let base = profile_json("proptest-base");
    proptest!(|(
        text in "\\PC{0,200}",
        top in proptest::option::of(-2000f32..3000f32),
        position in proptest::option::of(0u8..5),
        // Nested values: a character size, a paragraph line spacing, a line-number mode, a
        // section id, a check kind and a word limit all pass through their own checkers.
        size in proptest::option::of(-8f32..6000f32),
        spacing in proptest::option::of(0.0f32..200.0f32),
        spacing_rule in proptest::option::of(0u8..3),
        line_mode in proptest::option::of(0u8..6),
        section_id in "\\PC{0,24}",
        check_kind in "\\PC{0,24}",
        limit in proptest::option::of(0u32..20_000_000),
    )| {
        // Arbitrary bytes: either a profile, or a non-empty list of problems — never a panic.
        match Profile::from_json(&text) {
            Ok(p) => proptest::prop_assert!(crate::profile::problems(&p).is_empty()),
            Err(msgs) => proptest::prop_assert!(!msgs.is_empty()),
        }
        // A valid profile with one nested field at a time replaced by a random value. This drives
        // the nested checkers (page, page numbers, character and paragraph props, line numbers,
        // sections, checks) with hostile numbers and strings.
        let mut v = base.clone();
        if let Some(t) = top {
            v["docs"]["brief"]["page"]["top"] = json!(t);
        }
        if let Some(i) = position {
            let pos = ["top", "bottom", "left", "sideways", ""][i as usize % 5];
            v["docs"]["brief"]["pageNumbers"] = json!({"position": pos, "align": "center", "xOfY": false});
        }
        if let Some(sz) = size {
            v["docs"]["brief"]["styles"][0]["chr"]["size"] = json!(sz);
        }
        if let Some(sp) = spacing {
            let rule = ["multiple", "atLeast", "exactly"][spacing_rule.unwrap_or(0) as usize % 3];
            v["docs"]["brief"]["styles"][0]["para"]["lineSpacing"] = json!({"rule": rule, "value": sp});
        }
        if let Some(m) = line_mode {
            let mode = ["none", "continuous", "restartPage", "restartSection", "bogus", ""][m as usize % 6];
            v["docs"]["brief"]["lineNumbers"] = json!({"enabled": true, "value": mode});
        }
        v["docs"]["brief"]["sections"][0]["id"] = json!(section_id);
        v["docs"]["brief"]["checks"] = match limit {
            Some(n) => json!([{"kind": "wordLimit", "limit": n, "source": "Rule 1"}]),
            None => json!([{"kind": check_kind, "source": "Rule 1"}]),
        };
        match Profile::from_json(&v.to_string()) {
            Ok(p) => proptest::prop_assert!(crate::profile::problems(&p).is_empty()),
            Err(msgs) => {
                proptest::prop_assert!(!msgs.is_empty());
                // A plainly out-of-range type size is reported by name: the mutation reached the
                // nested character-properties checker rather than being swallowed.
                if size.is_some_and(|s| !(1.0..=4096.0).contains(&s)) {
                    proptest::prop_assert!(msgs.iter().any(|m| m.contains("size")), "size problem missing: {msgs:?}");
                }
            }
        }
    });
}

#[test]
fn validation_reports_bad_nested_fields_in_structurally_valid_profiles() {
    let mut s = law();
    let mut p = profile_json("nested-mutate");
    p["docs"]["brief"]["styles"][0]["basedOn"] = json!("MissingStyle");
    let e = err(&mut s, "law.profiles.validate", p).to_string();
    assert!(e.contains("MissingStyle"), "{e}");
    let mut p = profile_json("nested-page");
    p["docs"]["brief"]["page"]["top"] = json!(-1);
    let e = err(&mut s, "law.profiles.validate", p).to_string();
    assert!(e.contains("top"), "{e}");
}

/// A session with a profile applied, for the checks that look at the document.
pub(crate) fn apply_session(id: &str, doc_type: &str) -> Session {
    let mut s = law();
    run(&mut s, "law.profiles.apply", json!({"id": id, "doc": doc_type}));
    s
}

pub(crate) fn temp_dir(what: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wordcraft-law-{what}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temp folder");
    dir
}

fn write_temp(name: &str, text: &str) -> PathBuf {
    let dir = temp_dir("validate");
    let path = dir.join(name);
    std::fs::write(&path, text).expect("write the file");
    path
}
