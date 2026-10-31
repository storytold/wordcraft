use std::net::TcpListener;

use serde_json::{Value, json};
use wordcraft_doc::para::OBJ;
use wordcraft_doc::{Document, InlineObject, Path, Pos, StoryRef};
use wordcraft_engine::{Selection, Session};

use crate::client::{self, Options};
use crate::fields;
use crate::wire::{self, Call, Frame};
use crate::{Bridge, Command, Headless, ZoteroError};

mod limits;

const CITE: &str = r#"ITEM CSL_CITATION {"citationID":"k1","properties":{"formattedCitation":"(Doe, 2020)"},"citationItems":[{"id":7}]}"#;
const BIBL: &str = r#"BIBL {"uncited":[],"omitted":[],"custom":[]} CSL_BIBLIOGRAPHY"#;
const PREFS: &str = r#"<data data-version="3" zotero-version="7.0"><session id="s1"/><style id="http://www.zotero.org/styles/apa" hasBibliography="1" bibliographyStyleHasBeenSet="1"/><prefs><pref name="fieldType" value="ReferenceMark"/></prefs></data>"#;

struct Z {
    b: Bridge,
    s: Session,
    h: Headless,
}

impl Z {
    /// A session over `text`, caret at byte `caret` of the first paragraph.
    fn new(text: &str, caret: usize) -> Z {
        let mut s = Session::new(Document::from_text(text));
        s.sel = Selection::caret(Pos::body(0, caret));
        Z { b: Bridge::new(), s, h: Headless::default() }
    }
    fn call(&mut self, method: &str, args: Value) -> Result<Value, String> {
        let args = match args {
            Value::Array(a) => a,
            v => vec![v],
        };
        self.b.handle(&mut self.s, &mut self.h, &Call::new(method, args))
    }
    fn ok(&mut self, method: &str, args: Value) -> Value {
        self.call(method, args).unwrap_or_else(|e| panic!("{method}: {e}"))
    }
    fn begin(&mut self) {
        let r = self.ok("Application_getActiveDocument", json!([3]));
        assert_eq!(r[0], 3);
    }
    fn text(&self) -> String {
        self.s.doc.plain_text(StoryRef::Body)
    }
    /// Insert a citation the way Zotero does after the user picks an item.
    fn cite(&mut self, note: i64, rich: &str) -> u64 {
        let r = self.ok("Document_insertField", json!([1, "ReferenceMark", note]));
        assert_eq!(r[1], "");
        let id = r[0].as_u64().unwrap();
        self.ok("Field_setCode", json!([1, id, CITE]));
        self.ok("Field_setText", json!([1, id, rich, true]));
        id
    }
}

fn italic_at(d: &Document, story: StoryRef, path: &Path, needle: &str) -> bool {
    let p = d.para(story, path).unwrap();
    let off = p.text.find(needle).unwrap();
    p.props_of_char(off).italic == Some(true)
}

#[test]
fn add_citation_like_zotero_does() {
    let mut z = Z::new("Hello world", 5);
    z.begin();
    assert_eq!(z.ok("Document_getDocumentData", json!([1])), "");
    z.ok("Document_setDocumentData", json!([1, PREFS]));
    assert_eq!(z.ok("Document_canInsertField", json!([1, "ReferenceMark"])), true);
    assert_eq!(z.ok("Document_cursorInField", json!([1, "ReferenceMark"])), Value::Null);
    let id = z.cite(0, r"{\rtf  (Doe, {\i{}2020})}");
    let f = z.ok("Document_getFields", json!([1, "ReferenceMark"]));
    assert_eq!(f, json!([[id], [CITE], [0]]));
    assert_eq!(z.ok("Field_getText", json!([1, id])), " (Doe, 2020)");
    z.ok("Document_complete", json!([1]));
    assert!(z.b.completed);

    assert_eq!(z.text(), "Hello (Doe, 2020) world");
    assert!(italic_at(&z.s.doc, StoryRef::Body, &Path::top(0), "2020"));
    let r = z.s.doc.field_ranges(StoryRef::Body);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].instr, format!("ADDIN ZOTERO_{CITE}"));
    // The caret ends after the citation.
    assert_eq!(z.s.sel.focus, Pos { off: r[0].end.off + OBJ.len_utf8(), ..r[0].end.clone() });
    // Preferences are stored the Word way, and read back the way Zotero wrote them.
    assert!(z.s.doc.custom_prop("ZOTERO_PREF_1").unwrap().starts_with("<data"));
    let stored: String = (1..=3).filter_map(|i| z.s.doc.custom_prop(&format!("ZOTERO_PREF_{i}"))).collect();
    assert!(stored.contains(r#"name="fieldType" value="Field""#));
    assert!(z.s.doc.custom_props.iter().all(|p| p.value.chars().count() <= fields::PREF_CHUNK));
    assert_eq!(z.ok("Document_getDocumentData", json!([1])), PREFS);

    // The whole transaction is one undo step.
    assert_eq!(z.s.undo_label(), Some("Zotero"));
    assert!(z.s.undo());
    assert_eq!(z.text(), "Hello world");
    assert!(z.s.doc.custom_props.is_empty());
    assert!(!z.s.can_undo());
}

#[test]
fn citations_survive_docx_and_zotero_finds_them_again() {
    let mut z = Z::new("Hello world", 5);
    z.begin();
    let id = z.cite(0, r"{\rtf (Doe, {\i 2020})}");
    z.ok("Document_complete", json!([1]));
    let bytes = wordcraft_docx::write(&z.s.doc).unwrap();
    let mut z2 = Z { b: Bridge::new(), s: Session::new(wordcraft_docx::read(&bytes).unwrap()), h: Headless::default() };
    z2.begin();
    let f = z2.ok("Document_getFields", json!([1, "ReferenceMark"]));
    assert_eq!(f[1], json!([CITE]));
    assert_ne!(f[0][0], Value::Null);
    let _ = id;
    // Caret inside the citation: Zotero edits that one.
    let fr = z2.s.doc.field_ranges(StoryRef::Body);
    z2.s.sel = Selection::caret(Pos { off: fr[0].start.off + OBJ.len_utf8() + 2, ..fr[0].start.clone() });
    let hit = z2.ok("Document_cursorInField", json!([1, "ReferenceMark"]));
    assert_eq!(hit, json!([f[0][0], CITE, 0]));
    z2.s.sel = Selection::caret(Pos::body(0, 0));
    assert_eq!(z2.ok("Document_cursorInField", json!([1, "ReferenceMark"])), Value::Null);
}

#[test]
fn bibliography_spans_paragraphs_with_its_style() {
    let mut z = Z::new("Text.", 5);
    z.begin();
    let r = z.ok("Document_insertField", json!([1, "ReferenceMark", 0]));
    let id = r[0].as_u64().unwrap();
    z.ok("Field_setCode", json!([1, id, BIBL]));
    z.ok("Document_setBibliographyStyle", json!([1, -720, 720, 480, 240, [], 0]));
    z.ok("Field_setText", json!([1, id, r"{\rtf Doe, J. (2020). {\i A Title}.\par Roe, R. (2019). Other.\par }", true]));
    z.ok("Document_complete", json!([1]));

    assert_eq!(z.text(), "Text.Doe, J. (2020). A Title.\nRoe, R. (2019). Other.");
    let r = z.s.doc.field_ranges(StoryRef::Body);
    assert_eq!(r.len(), 1);
    assert_eq!((r[0].start.path.clone(), r[0].end.path.clone()), (Path::top(0), Path::top(1)));
    assert!(italic_at(&z.s.doc, StoryRef::Body, &Path::top(0), "A Title"));
    for i in 0..2 {
        assert_eq!(z.s.doc.para(StoryRef::Body, &Path::top(i)).unwrap().props.style.as_deref(), Some("Bibliography"));
    }
    let st = z.s.doc.styles.get("Bibliography").unwrap();
    assert_eq!((st.para.indent_left, st.para.indent_first, st.para.space_after), (Some(36.0), Some(-36.0), Some(12.0)));
    // Refresh: Zotero rewrites the text; the field keeps its range.
    z.begin();
    let f = z.ok("Document_getFields", json!([1, "ReferenceMark"]));
    let id = f[0][0].as_u64().unwrap();
    z.ok("Field_setText", json!([1, id, r"{\rtf Only one.}", true]));
    z.ok("Document_complete", json!([1]));
    assert_eq!(z.text(), "Text.Only one.");
}

#[test]
fn note_styles_put_citations_in_footnotes() {
    let mut z = Z::new("One. Two.", 4);
    z.begin();
    let a = z.cite(1, r"{\rtf Doe, {\i Title}, 2020.}");
    z.s.sel = Selection::caret(Pos::body(0, z.s.doc.para(StoryRef::Body, &Path::top(0)).unwrap().len()));
    let b = z.cite(1, r"{\rtf Ibid.}");
    let f = z.ok("Document_getFields", json!([1, "ReferenceMark"]));
    assert_eq!(f[0], json!([a, b]));
    assert_eq!(f[2], json!([1, 2]));
    let notes: Vec<u32> = z.s.doc.parts.keys().copied().collect();
    assert_eq!(notes.len(), 2);
    let note_text = z.s.doc.plain_text(StoryRef::Part(notes[0]));
    assert!(note_text.ends_with("Doe, Title, 2020."), "{note_text:?}");
    // Deleting a note citation deletes its note.
    z.ok("Field_delete", json!([1, a]));
    assert_eq!(z.s.doc.parts.len(), 1);
    let f = z.ok("Document_getFields", json!([1, "ReferenceMark"]));
    assert_eq!(f, json!([[b], [CITE], [1]]));
    z.ok("Document_complete", json!([1]));
}

#[test]
fn remove_codes_keeps_text_and_select_delete_work() {
    let mut z = Z::new("A B", 1);
    z.begin();
    let a = z.cite(0, "(Doe 2020)");
    z.s.sel = Selection::caret(Pos::body(0, z.s.doc.para(StoryRef::Body, &Path::top(0)).unwrap().len()));
    let b = z.cite(0, "(Roe 2019)");
    z.ok("Field_select", json!([1, b]));
    assert_eq!(z.s.selected_text(), "(Roe 2019)");
    // A selection Zotero made to show a citation is undone when it finishes.
    z.ok("Document_complete", json!([1]));
    assert!(z.s.sel.is_collapsed());
    z.begin();
    z.ok("Field_removeCode", json!([1, a]));
    assert_eq!(z.text(), "A(Doe 2020) B(Roe 2019)");
    assert_eq!(fields::list(&z.s.doc).len(), 1);
    // The surviving id still names the surviving field.
    assert_eq!(z.ok("Field_getText", json!([1, b])), "(Roe 2019)");
    z.ok("Field_delete", json!([1, b]));
    assert_eq!(z.text(), "A(Doe 2020) B");
    assert!(z.call("Field_getText", json!([1, b])).is_err());
}

#[test]
fn placeholders_become_fields() {
    let mut z = Z::new("Note: ", 6);
    z.begin();
    z.ok(
        "Document_insertText",
        json!([1, r#"<p>As shown <a href="https://www.zotero.org/?p1">(Doe)</a> and <a href="https://www.zotero.org/?p2">(Roe)</a>.</p>"#]),
    );
    assert_eq!(z.text(), "Note: As shown (Doe) and (Roe).");
    let r = z.ok("Document_convertPlaceholdersToFields", json!([1, ["p1", "p2"], 0, "ReferenceMark", 2]));
    let ids = r[0].as_array().unwrap().clone();
    assert_eq!(ids.len(), 2);
    for (k, id) in ids.iter().enumerate() {
        z.ok("Field_setCode", json!([1, id, CITE]));
        assert_eq!(z.ok("Field_getText", json!([1, id])), ["(Doe)", "(Roe)"][k]);
    }
    assert_eq!(z.text(), "Note: As shown (Doe) and (Roe).");
    let p = z.s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert!(p.runs.iter().all(|r| r.props.link.is_none()));
    assert!(z.call("Document_convertPlaceholdersToFields", json!([1, ["nope"], 0, "ReferenceMark", 1])).is_err());
}

#[test]
fn alerts_activate_and_unknown_calls() {
    let mut z = Z::new("x", 0);
    z.begin();
    assert_eq!(z.ok("Document_displayAlert", json!([1, "Continue?", 2, 3])), 2);
    assert_eq!(z.ok("Document_displayAlert", json!([1, "OK?", 1, 1])), 1);
    assert_eq!(z.h.alerts, ["Continue?", "OK?"]);
    assert_eq!(z.ok("Document_activate", json!([1])), Value::Null);
    assert_eq!(z.ok("Document_convert", json!([1, [], [], [], 0])), Value::Null);
    assert!(z.call("Document_exportDocument", json!([1, "ReferenceMark", ""])).is_err());
    assert!(z.call("Nope_nope", json!([])).is_err());
    // Nothing changed, so no undo step.
    assert!(!z.s.can_undo());
}

#[test]
fn citations_only_in_text_and_notes() {
    let mut z = Z::new("x", 0);
    let hid = z.s.doc.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::new())]);
    z.s.sel = Selection::caret(z.s.doc.start_of(StoryRef::Part(hid)));
    z.begin();
    assert_eq!(z.ok("Document_canInsertField", json!([1, "ReferenceMark"])), false);
    assert!(z.call("Document_insertField", json!([1, "ReferenceMark", 0])).is_err());
}

#[test]
fn hostile_calls_never_panic() {
    let junk =
        [json!(null), json!(-1), json!(1e300), json!("x"), json!([]), json!({}), json!(u64::MAX), json!(i64::MIN), json!([[1, 2], "a"]), json!(true)];
    let methods = [
        "Application_getActiveDocument",
        "Document_displayAlert",
        "Document_activate",
        "Document_canInsertField",
        "Document_setDocumentData",
        "Document_getDocumentData",
        "Document_cursorInField",
        "Document_insertField",
        "Document_insertText",
        "Document_getFields",
        "Document_convert",
        "Document_convertPlaceholdersToFields",
        "Document_setBibliographyStyle",
        "Document_complete",
        "Field_delete",
        "Field_select",
        "Field_removeCode",
        "Field_setText",
        "Field_getText",
        "Field_setCode",
        "Field_convert",
    ];
    let mut z = Z::new("Hello world\nsecond", 3);
    z.begin();
    let id = z.cite(0, "(x)");
    for m in methods {
        for j in &junk {
            for n in 0..8 {
                let mut args: Vec<Value> = (0..n).map(|_| j.clone()).collect();
                let _ = z.call(m, Value::Array(args.clone()));
                // And with a real field id in the id slot.
                if n >= 2 {
                    if let Some(a) = args.get_mut(1) {
                        *a = json!(id);
                    }
                    let _ = z.call(m, Value::Array(args));
                }
            }
        }
    }
    // The document is still sound.
    z.s.clamp_selection();
    let _ = wordcraft_docx::write(&z.s.doc).unwrap();
}

// ---- wire ----

#[test]
fn frames_round_trip_and_bad_frames_are_errors() {
    let mut buf = Vec::new();
    wire::write_frame(&mut buf, 7, b"[1]").unwrap();
    assert_eq!(&buf[..8], &[0, 0, 0, 7, 0, 0, 0, 3]);
    let f = wire::read_frame(&mut buf.as_slice()).unwrap();
    assert_eq!(f, Some(Frame { txid: 7, payload: b"[1]".to_vec() }));
    assert_eq!(wire::read_frame(&mut [].as_slice()).unwrap(), None);
    assert!(wire::read_frame(&mut [0u8, 0, 0].as_slice()).is_err());
    // Length over the cap, and a truncated payload.
    assert!(matches!(wire::read_frame(&mut [0u8, 0, 0, 1, 0xFF, 0xFF, 0xFF, 0xFF].as_slice()), Err(ZoteroError::Protocol(_))));
    assert!(wire::read_frame(&mut [0u8, 0, 0, 1, 0, 0, 0, 9, b'x'].as_slice()).is_err());
}

#[test]
fn calls_parse_both_shapes() {
    assert_eq!(Call::parse(br#"["Application_getActiveDocument", 3]"#).unwrap(), Call::new("Application_getActiveDocument", vec![json!(3)]));
    assert_eq!(Call::parse(br#"["Field_setText", [1, 2, "x", true]]"#).unwrap().args.len(), 4);
    assert_eq!(Call::parse(br#"["Document_complete"]"#).unwrap().args.len(), 0);
    for bad in [&b"{}"[..], b"[]", b"[1]", b"nope", b"\xFF"] {
        assert!(Call::parse(bad).is_err());
    }
    assert_eq!(wire::encode_reply(&Err("boom".into())), b"ERR:boom");
    assert_eq!(wire::encode_reply(&Ok(json!([3, 1]))), b"[3,1]");
}

/// A fake Zotero: accepts one connection, checks the command, makes `calls`, and returns the
/// replies it got.
fn fake_zotero(expect_command: &'static str, calls: Vec<Value>) -> (std::net::SocketAddr, std::thread::JoinHandle<Vec<String>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let h = std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let f = wire::read_frame(&mut s).unwrap().unwrap();
        assert_eq!(f.txid, 0);
        let cmd: Value = serde_json::from_slice(&f.payload).unwrap();
        assert_eq!(cmd, json!({"command": expect_command, "templateVersion": 1}));
        let mut replies = Vec::new();
        for (i, c) in calls.into_iter().enumerate() {
            let tx = 100 + i as u32;
            wire::write_frame(&mut s, tx, &serde_json::to_vec(&c).unwrap()).unwrap();
            let r = wire::read_frame(&mut s).unwrap().unwrap();
            assert_eq!(r.txid, tx);
            replies.push(String::from_utf8(r.payload).unwrap());
        }
        replies
    });
    (addr, h)
}

#[test]
fn whole_session_over_a_socket() {
    let calls = vec![
        json!(["Application_getActiveDocument", [3]]),
        json!(["Document_getDocumentData", [1]]),
        json!(["Document_setDocumentData", [1, PREFS]]),
        json!(["Document_cursorInField", [1, "ReferenceMark"]]),
        json!(["Document_insertField", [1, "ReferenceMark", 0]]),
        json!(["Field_setCode", [1, 1, CITE]]),
        json!(["Field_setText", [1, 1, r"{\rtf (Doe, {\i 2020})}", true]]),
        json!(["Not_a_method", []]),
        json!(["Document_complete", [1]]),
    ];
    let (addr, server) = fake_zotero("addEditCitation", calls);
    let mut session = Session::new(Document::from_text("Cite here."));
    session.sel = Selection::caret(Pos::body(0, 9));
    let opts = Options { addr, ..Options::default() };
    let out = client::run_on_session(&opts, Command::AddEditCitation, &mut Bridge::new(), &mut session, &mut Headless::default()).unwrap();
    let replies = server.join().unwrap();
    assert!(out.completed);
    assert_eq!(out.calls, 9);
    assert_eq!(out.errors.len(), 1);
    assert_eq!(replies[0], "[3,1]");
    assert_eq!(replies[1], r#""""#);
    assert_eq!(replies[3], "null");
    assert_eq!(replies[4], r#"[1,"",0]"#);
    assert!(replies[7].starts_with("ERR:"));
    assert_eq!(replies[8], "null");
    assert_eq!(session.doc.plain_text(StoryRef::Body), "Cite here(Doe, 2020).");
}

#[test]
fn cancelled_session_and_no_zotero() {
    // Zotero closes the connection without completing (the user cancelled its dialog).
    let (addr, server) = fake_zotero("refresh", vec![json!(["Application_getActiveDocument", [3]])]);
    let mut session = Session::new(Document::from_text("x"));
    let opts = Options { addr, ..Options::default() };
    let out = client::run_on_session(&opts, Command::Refresh, &mut Bridge::new(), &mut session, &mut Headless::default()).unwrap();
    server.join().unwrap();
    assert!(!out.completed);
    assert_eq!(out.calls, 1);

    // Nothing listening.
    let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let opts = Options { addr: free, ..Options::default() };
    let r = client::run_command(&opts, Command::Refresh, &mut |_| Ok(Value::Null));
    assert!(matches!(r, Err(ZoteroError::NotRunning(_))));
}

#[test]
fn command_names() {
    for c in Command::ALL {
        assert_eq!(Command::from_name(c.wire_name()), Some(c));
    }
    assert_eq!(Command::from_name("ADDEDITCITATION"), Some(Command::AddEditCitation));
    assert_eq!(Command::from_name("exportDocument"), None);
}

#[test]
fn prefs_chunking_and_codes() {
    let mut d = Document::new();
    d.set_custom_prop("Other", "keep");
    let long: String = "é".repeat(600);
    fields::set_prefs(&mut d, &long);
    assert_eq!(d.custom_prop("ZOTERO_PREF_3").map(|v| v.chars().count()), Some(90));
    assert_eq!(fields::get_prefs(&d), long);
    fields::set_prefs(&mut d, "short");
    assert_eq!(d.custom_prop("ZOTERO_PREF_2"), None);
    assert_eq!(d.custom_prop("Other"), Some("keep"));
    assert_eq!(fields::zotero_code("  addin zotero_ITEM x "), Some("ITEM x"));
    assert_eq!(fields::zotero_code("ADDIN EN.CITE"), None);
    assert_eq!(fields::zotero_code("ADD"), None);
    let _ = InlineObject::FieldEnd;
}

mod props {
    use proptest::prelude::*;

    use crate::wire::{Call, read_frame};

    proptest! {
        #[test]
        fn random_frames_and_payloads_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = read_frame(&mut bytes.as_slice());
            let _ = Call::parse(&bytes);
        }
    }
}

fn note_count(d: &Document, kind: wordcraft_doc::PartKind) -> usize {
    d.parts.values().filter(|p| p.kind == kind).count()
}

#[test]
fn switching_to_a_note_style_moves_citations_into_footnotes_and_back() {
    use wordcraft_doc::PartKind;
    let mut z = Z::new("One. Two.", 4);
    z.begin();
    let a = z.cite(0, r"{\rtf (Doe, {\i 2020})}");
    let end = z.s.doc.para(StoryRef::Body, &Path::top(0)).unwrap().len();
    z.s.sel = Selection::caret(Pos::body(0, end));
    let b = z.cite(0, "(Roe 2019)");
    z.ok("Document_complete", json!([1]));
    let before = z.text();
    assert_eq!(before, "One.(Doe, 2020) Two.(Roe 2019)");

    // To footnotes (as Zotero does when the style becomes a note style).
    z.begin();
    z.ok("Document_convert", json!([1, [a, b], ["ReferenceMark", "ReferenceMark"], [1, 1], 2]));
    assert_eq!(z.text().replace(OBJ, ""), "One. Two.");
    assert_eq!(note_count(&z.s.doc, PartKind::Footnote), 2);
    let f = z.ok("Document_getFields", json!([1, "ReferenceMark"]));
    assert_eq!(f[0], json!([a, b]), "ids survive the move");
    assert_eq!(f[2], json!([1, 2]));
    let r = fields::list(&z.s.doc);
    let StoryRef::Part(pid) = r[0].start.story else { panic!("not in a note") };
    assert!(italic_at(&z.s.doc, StoryRef::Part(pid), &Path::top(0), "2020"));
    assert_eq!(z.ok("Field_getText", json!([1, a])), "(Doe, 2020)");
    z.ok("Document_complete", json!([1]));

    // One to an endnote.
    z.begin();
    z.ok("Field_convert", json!([1, b, "ReferenceMark", 2]));
    assert_eq!((note_count(&z.s.doc, PartKind::Footnote), note_count(&z.s.doc, PartKind::Endnote)), (1, 1));
    let f = z.ok("Document_getFields", json!([1, "ReferenceMark"]));
    assert_eq!(f[0], json!([a, b]));
    assert_eq!(f[2], json!([1, 1]));
    z.ok("Document_complete", json!([1]));

    // Back to the text: the notes go, the citations sit where their marks were.
    z.begin();
    z.ok("Document_convert", json!([1, [a, b], ["ReferenceMark", "ReferenceMark"], 0, 2]));
    assert_eq!(z.text(), before);
    assert!(z.s.doc.parts.is_empty());
    assert!(italic_at(&z.s.doc, StoryRef::Body, &Path::top(0), "2020"));
    assert_eq!(z.ok("Document_getFields", json!([1, "ReferenceMark"]))[2], json!([0, 0]));
    // Converting to where it already is changes nothing.
    let rev = z.s.rev();
    z.ok("Field_convert", json!([1, a, "ReferenceMark", 0]));
    assert_eq!(z.s.rev(), rev);
    z.ok("Document_complete", json!([1]));

    // Each transaction was one undo step.
    z.s.undo();
    assert_eq!(note_count(&z.s.doc, PartKind::Endnote), 1);
    z.s.undo();
    assert_eq!(note_count(&z.s.doc, PartKind::Footnote), 2);
}

#[test]
fn a_note_with_other_text_stays_when_its_citation_leaves() {
    let mut z = Z::new("Text.", 5);
    z.begin();
    let a = z.cite(1, "Doe, Title.");
    // The author adds a comment to the note.
    let pid = *z.s.doc.parts.keys().next().unwrap();
    let p = z.s.doc.para_mut(StoryRef::Part(pid), &Path::top(0)).unwrap();
    let n = p.len();
    p.insert_text(n, " See also chapter 2.", &Default::default()).unwrap();
    z.ok("Field_convert", json!([1, a, "ReferenceMark", 0]));
    assert_eq!(z.s.doc.parts.len(), 1);
    assert!(z.s.doc.plain_text(StoryRef::Part(pid)).ends_with("See also chapter 2."));
    assert_eq!(z.ok("Document_getFields", json!([1, "ReferenceMark"]))[2], json!([0]));
    assert!(z.text().ends_with("Doe, Title."));
    assert!(z.call("Document_convert", json!([1, "x", [], [], 0])).is_err());
    assert!(z.call("Document_convert", json!([1, [999], [], [1], 1])).is_err());
}

#[test]
fn a_note_with_a_picture_stays_when_its_citation_leaves() {
    let mut z = Z::new("Text.", 5);
    z.begin();
    let a = z.cite(1, "Doe, Title.");
    let pid = *z.s.doc.parts.keys().next().unwrap();
    let p = z.s.doc.para_mut(StoryRef::Part(pid), &Path::top(0)).unwrap();
    let n = p.len();
    let image = InlineObject::Image { media: "m".into(), w: 1.0, h: 1.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None };
    p.insert_object(n, image, &Default::default()).unwrap();
    z.ok("Field_convert", json!([1, a, "ReferenceMark", 0]));
    assert_eq!(z.s.doc.parts.len(), 1);
    assert!(z.s.doc.para(StoryRef::Part(pid), &Path::top(0)).unwrap().objects.iter().any(|o| matches!(o, InlineObject::Image { .. })));
    assert!(z.text().ends_with("Doe, Title."));
}

/// Zotero reads a new bibliography field back before filling it, and unlinks it when it is
/// empty (it takes that for a bibliography the user deleted). Seen with Zotero 10: Add/Edit
/// Bibliography did nothing.
#[test]
fn a_new_field_is_never_empty_so_zotero_keeps_a_new_bibliography() {
    let mut z = Z::new("Text. ", 6);
    z.begin();
    let r = z.ok("Document_insertField", json!([1, "ReferenceMark", 0]));
    let id = r[0].as_u64().unwrap();
    z.ok("Field_setCode", json!([1, id, "TEMP"]));
    z.ok("Field_setCode", json!([1, id, "BIBL {} CSL_BIBLIOGRAPHY"]));
    let text = z.ok("Field_getText", json!([1, id]));
    assert!(!text.as_str().unwrap().is_empty(), "Zotero would unlink this bibliography");
    // Zotero's result replaces the placeholder.
    z.ok("Field_setText", json!([1, id, r"{\rtf Doe, J. (2020).\par}", true]));
    z.ok("Document_complete", json!([1]));
    assert_eq!(z.text(), "Text. Doe, J. (2020).");
}

/// #898: a click just after a citation puts the caret before its hidden field end. Zotero must
/// see that caret outside the citation (as in Word), and a citation added there goes after the
/// first one, not inside its result.
#[test]
fn a_citation_added_just_after_another_is_not_nested_in_it() {
    let mut z = Z::new("Cats purr ", 10);
    z.begin();
    z.cite(0, "(Doe 2020)");
    let first = z.s.doc.field_ranges(StoryRef::Body)[0].end.clone();
    z.s.sel = Selection::caret(first);
    assert_eq!(z.ok("Document_cursorInField", json!([1, "ReferenceMark"])), Value::Null);
    z.cite(0, "(Roe 2019)");
    z.ok("Document_complete", json!([1]));
    assert_eq!(z.text(), "Cats purr (Doe 2020)(Roe 2019)");
    let r = z.s.doc.field_ranges(StoryRef::Body);
    assert_eq!(r.iter().map(|f| f.depth).collect::<Vec<_>>(), [0, 0]);
}

// #1323: Zotero's edits obey Restrict Editing, content-control locks and Track Changes, the same
// rules commands obey.

/// Everything Zotero could change: every story's text with its formatting, the preferences, the
/// styles.
fn snapshot(d: &Document) -> String {
    format!("{:?}|{:?}|{:?}|{:?}", d.body, d.parts, d.custom_props, d.styles)
}

/// A session over `text` with one citation already in it (at byte `caret`), its id, and the
/// transaction finished.
fn with_citation(text: &str, caret: usize) -> (Z, u64) {
    let mut z = Z::new(text, caret);
    z.begin();
    z.ok("Document_setDocumentData", json!([1, PREFS]));
    let id = z.cite(0, "(Doe 2020)");
    z.ok("Document_complete", json!([1]));
    (z, id)
}

/// Every editing call Zotero makes, against field `id`; each must be refused. The document
/// preferences aren't content: they follow the restriction alone (`prefs_refused`), refused
/// unless the document has regions the user may edit.
fn assert_every_edit_refused(z: &mut Z, id: u64, prefs_refused: bool) {
    let before = snapshot(&z.s.doc);
    z.begin();
    let refused = [
        ("Document_insertField", json!([1, "ReferenceMark", 0])),
        ("Document_insertText", json!([1, "x"])),
        ("Document_setDocumentData", json!([1, "<data/>"])),
        ("Document_setBibliographyStyle", json!([1, 0, 720, 240, 240, [], 0])),
        ("Document_convert", json!([1, [id], 1, [1]])),
        ("Field_setText", json!([1, id, "(Roe 2019)", false])),
        ("Field_setCode", json!([1, id, "ITEM CSL_CITATION {}"])),
        ("Field_removeCode", json!([1, id])),
        ("Field_delete", json!([1, id])),
    ];
    for (method, args) in refused {
        if method == "Document_setDocumentData" && !prefs_refused {
            continue;
        }
        assert!(z.call(method, args).is_err(), "{method} went through");
        assert_eq!(snapshot(&z.s.doc), before, "{method} changed the document");
    }
    // Reading still works, and the field id is still good.
    assert_eq!(z.ok("Field_getText", json!([1, id])), "(Doe 2020)");
    z.ok("Document_complete", json!([1]));
    assert_eq!(snapshot(&z.s.doc), before);
}

#[test]
fn read_only_protection_refuses_every_zotero_edit() {
    let (mut z, id) = with_citation("Hello world", 5);
    z.s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    assert_every_edit_refused(&mut z, id, true);
}

#[test]
fn comments_protection_refuses_every_zotero_edit() {
    let (mut z, id) = with_citation("Hello world", 5);
    z.s.run("review.restrict", &json!({"mode": "comments"})).unwrap();
    assert_every_edit_refused(&mut z, id, true);
}

#[test]
fn forms_protection_refuses_zotero_edits_outside_fillable_controls() {
    let (mut z, id) = with_citation("Hello world. Name here.", 5);
    z.s.run("select.text", &json!({"text": "Name here"})).unwrap();
    z.s.run("developer.plainText", &json!({"tag": "name"})).unwrap();
    z.s.run("review.restrict", &json!({"mode": "forms"})).unwrap();
    z.s.sel = Selection::caret(Pos::body(0, 0));
    assert_every_edit_refused(&mut z, id, false);
}

#[test]
fn read_only_exception_takes_a_citation_but_the_rest_refuses_one() {
    let mut z = Z::new("Locked intro.\nFill in here.\nLocked end.", 0);
    z.s.run("select.text", &json!({"text": "Fill in here."})).unwrap();
    z.s.run("review.restrict", &json!({"exception": "add"})).unwrap();
    z.s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    // Outside the exception: refused, nothing changes.
    let before = snapshot(&z.s.doc);
    z.s.sel = Selection::caret(Pos::body(0, 6));
    z.begin();
    assert!(z.call("Document_insertField", json!([1, "ReferenceMark", 0])).is_err());
    assert_eq!(snapshot(&z.s.doc), before);
    z.ok("Document_complete", json!([1]));
    // Inside it: the citation goes in, and Zotero can fill it.
    z.s.run("select.text", &json!({"text": "here"})).unwrap();
    z.s.sel = Selection::caret(z.s.sel.ordered().0);
    z.begin();
    let id = z.cite(0, "(Doe 2020) ");
    z.ok("Document_complete", json!([1]));
    assert_eq!(z.text(), "Locked intro.\nFill in (Doe 2020) here.\nLocked end.");
    // A citation inside the exception can be refreshed, but not moved out of it into a note.
    z.begin();
    z.ok("Field_setText", json!([1, id, "(Doe 2021) ", false]));
    assert_eq!(z.text(), "Locked intro.\nFill in (Doe 2021) here.\nLocked end.");
    assert!(z.call("Document_convert", json!([1, [id], 1, [1]])).is_err());
    z.ok("Document_complete", json!([1]));
    assert_eq!(z.text(), "Locked intro.\nFill in (Doe 2021) here.\nLocked end.");
}

#[test]
fn locked_content_controls_refuse_zotero_edits() {
    // A citation inside a control, which is then locked against edits.
    let (mut z, id) = with_citation("Hello world", 5);
    let len = z.s.doc.para(StoryRef::Body, &Path::top(0)).unwrap().len();
    z.s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, len) };
    z.s.run("developer.richText", &json!({"tag": "frozen"})).unwrap();
    z.s.run("developer.properties", &json!({"control": "frozen", "lockEdit": true})).unwrap();
    let before = snapshot(&z.s.doc);
    z.begin();
    for (method, args) in [
        ("Field_setText", json!([1, id, "(Roe 2019)", false])),
        ("Field_setCode", json!([1, id, "ITEM CSL_CITATION {}"])),
        ("Field_removeCode", json!([1, id])),
        ("Field_delete", json!([1, id])),
        ("Document_convert", json!([1, [id], 1, [1]])),
    ] {
        assert!(z.call(method, args).is_err(), "{method} went through");
        assert_eq!(snapshot(&z.s.doc), before, "{method} changed the document");
    }
    // A new citation inside the locked control: refused too.
    z.s.run("select.text", &json!({"text": "world"})).unwrap();
    z.s.sel = Selection::caret(z.s.sel.ordered().0);
    assert!(z.call("Document_insertField", json!([1, "ReferenceMark", 0])).is_err());
    assert!(z.call("Document_insertText", json!([1, "x"])).is_err());
    z.ok("Document_complete", json!([1]));
    assert_eq!(snapshot(&z.s.doc), before);
}

/// Track Changes on: each Zotero edit is a revision, and Reject All brings the text back.
fn tracked_edits_are_revisions(protect: bool) {
    let track = |z: &mut Z| {
        if protect {
            z.s.run("review.restrict", &json!({"mode": "trackedChanges"})).unwrap();
        } else {
            z.s.run("review.trackChanges", &json!({"value": true})).unwrap();
        }
    };
    let reject_all = |z: &mut Z| {
        if protect {
            z.s.run("review.restrict", &json!({"stop": true})).unwrap();
        }
        z.s.run("review.rejectAll", &json!({})).unwrap();
    };
    // Inserting a citation.
    let mut z = Z::new("Hello world", 5);
    track(&mut z);
    z.begin();
    let id = z.cite(0, " (Doe 2020)");
    z.ok("Document_complete", json!([1]));
    assert_eq!(z.s.doc.final_text(StoryRef::Body), "Hello (Doe 2020) world");
    assert!(z.s.doc.settings.track_changes);
    let p = z.s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    let off = p.text.find("Doe").unwrap();
    assert!(p.props_of_char(off).ins.is_some(), "the citation is not a tracked insertion");
    let _ = id;
    reject_all(&mut z);
    assert_eq!(z.text(), "Hello world");
    assert!(fields::list(&z.s.doc).is_empty());

    // Refreshing a citation, removing its code, deleting it.
    for method in ["Field_setText", "Field_removeCode", "Field_delete"] {
        let (mut z, id) = with_citation("Hello world", 5);
        let original = z.text();
        track(&mut z);
        z.begin();
        match method {
            "Field_setText" => {
                z.ok(method, json!([1, id, "(Roe 2019)", false]));
                // Zotero reads back the citation as it now reads, not the deleted text.
                assert_eq!(z.ok("Field_getText", json!([1, id])), "(Roe 2019)");
                assert_eq!(z.s.doc.final_text(StoryRef::Body), "Hello(Roe 2019) world");
            }
            _ => {
                z.ok(method, json!([1, id]));
                // A field whose markers are deleted is gone for Zotero.
                assert!(fields::list(&z.s.doc).is_empty(), "{method}");
                assert!(z.ok("Document_getFields", json!([1, "ReferenceMark"]))[0].as_array().unwrap().is_empty(), "{method}");
            }
        }
        z.ok("Document_complete", json!([1]));
        assert!(!z.s.doc.revisions.is_empty(), "{method} left no revision");
        assert_eq!(z.text().replace("(Roe 2019)", ""), original, "{method}: the old text is kept as a deletion");
        reject_all(&mut z);
        assert_eq!(z.text(), original, "{method}");
        assert_eq!(fields::list(&z.s.doc).len(), 1, "{method}");
    }
}

#[test]
fn track_changes_records_zotero_edits_as_revisions() {
    tracked_edits_are_revisions(false);
}

#[test]
fn tracked_changes_protection_records_zotero_edits_as_revisions() {
    tracked_edits_are_revisions(true);
}

/// Review of #1323: text Zotero inserts where Track Changes just marked text deleted is an
/// insertion only, never deleted too (Accept All keeps it).
#[test]
fn tracked_inserted_text_never_takes_a_deletion_mark() {
    let mut z = Z::new("Hello world", 0);
    z.s.run("review.trackChanges", &json!({"value": true})).unwrap();
    z.s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 5) };
    z.begin();
    z.ok("Document_insertText", json!([1, "Note"]));
    z.ok("Document_complete", json!([1]));
    let p = z.s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    let off = p.text.find("Note").unwrap();
    let props = p.props_of_char(off);
    assert!(props.ins.is_some() && props.del.is_none(), "{props:?}");
    assert_eq!(z.s.doc.final_text(StoryRef::Body), "Note world");
    z.s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(z.text(), "Note world");
}

#[test]
fn citation_at_document_end_stays_closed() {
    // #898: citing at the very end, typing after it, saving and reloading must all keep
    // the field's end marker in place.
    let mut z = Z::new("Hello", 5);
    z.begin();
    let id = z.cite(0, r"{\rtf  (Doe, {\i{}2020})}");
    z.ok("Document_complete", json!([1]));
    // Type right after the citation, at the very end of the document.
    z.s.run("text.insert", &json!({"text": "!"})).unwrap();
    assert_eq!(z.text(), "Hello (Doe, 2020)!");
    let ranges = z.s.doc.field_ranges(StoryRef::Body);
    assert_eq!(ranges.len(), 1);
    assert!(z.s.sel.focus.off > ranges[0].end.off, "typed text lands outside the field");
    let f = z.ok("Document_getFields", json!([1, "ReferenceMark"]));
    assert_eq!(f, json!([[id], [CITE], [0]]));
    // Save and reload: the end marker must survive the round trip.
    let bytes = wordcraft_engine::io::save_bytes("probe.docx", &z.s.doc).unwrap();
    let back = wordcraft_engine::io::open_bytes("probe.docx", &bytes).unwrap();
    assert_eq!(back.field_ranges(StoryRef::Body).len(), 1);
}
