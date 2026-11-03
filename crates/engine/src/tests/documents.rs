//! Several open documents in one session: File › New and Open open beside the others, each
//! document keeps its own text, selection, undo and find state, and work addressed to a document
//! by id lands in that document whichever one is active.

use serde_json::{Value, json};
use wordcraft_doc::StoryRef;

use crate::Session;

fn s() -> Session {
    Session::new(wordcraft_doc::Document::new())
}

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn text(s: &Session) -> String {
    s.doc.plain_text(StoryRef::Body)
}

fn ids(s: &Session) -> Vec<u64> {
    s.documents().iter().map(|d| d.id).collect()
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wordcraft-engine-docs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn new_opens_another_document_and_each_keeps_its_own_text_and_undo() {
    let mut s = s();
    let a = s.document_id();
    run(&mut s, "text.insert", json!({"text": "Alpha"}));
    run(&mut s, "file.new", json!({}));
    let b = s.document_id();
    assert_ne!(a, b);
    assert_eq!(ids(&s), vec![a, b], "opening order");
    let docs = s.documents();
    assert_eq!(docs.iter().map(|d| (d.title.as_str(), d.active)).collect::<Vec<_>>(), vec![("Document1", false), ("Document2", true)]);
    assert!(docs[0].dirty && !docs[1].dirty);
    assert_eq!(text(&s), "", "the new document is active");
    run(&mut s, "text.insert", json!({"text": "Beta"}));

    s.activate(a).unwrap();
    assert_eq!(text(&s), "Alpha");
    assert_eq!(s.document_id(), a, "ids stay with their documents");
    s.activate(a).unwrap();
    s.activate(b).unwrap();
    assert_eq!(text(&s), "Beta");
    run(&mut s, "file.new", json!({}));
    assert_eq!(s.documents()[2].title, "Document3");
}

/// Undo goes with its document: edit A, edit B, back to A, Undo takes A's edit back and leaves
/// B alone; Redo puts it back. Find state stays with its document too.
#[test]
fn undo_and_find_stay_with_their_document_across_switches() {
    let mut s = s();
    let a = s.document_id();
    run(&mut s, "text.insert", json!({"text": "Alpha"}));
    s.find.query = "Alpha".into();
    let b = s.add_document(wordcraft_doc::Document::new());
    assert_eq!(s.find.query, "", "a new document has its own find state");
    run(&mut s, "text.insert", json!({"text": "Beta"}));
    s.find.query = "Beta".into();

    s.activate(a).unwrap();
    assert_eq!(s.find.query, "Alpha");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "");
    assert_eq!(s.with_document(b, |s| (text(s), s.undo_depth(), s.find.query.clone())).unwrap(), ("Beta".to_string(), 1, "Beta".to_string()));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(text(&s), "Alpha");
    s.activate(b).unwrap();
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "");
    assert_eq!(s.with_document(a, |s| text(s)).unwrap(), "Alpha");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn open_replaces_an_untouched_blank_document_and_otherwise_opens_beside() {
    let dir = temp_dir("open");
    let one = dir.join("one.txt");
    let two = dir.join("two.txt");
    std::fs::write(&one, "One").unwrap();
    std::fs::write(&two, "Two").unwrap();

    let mut s = s();
    assert!(s.is_untouched_blank());
    let r = run(&mut s, "file.open", json!({"path": one.to_string_lossy()}));
    assert_eq!(r["alreadyOpen"], false);
    assert_eq!(s.documents().len(), 1, "the untouched blank document was replaced");
    let first = s.document_id();
    assert_eq!(s.documents()[0].title, "one");

    run(&mut s, "file.open", json!({"path": two.to_string_lossy()}));
    assert_eq!(s.documents().len(), 2, "a document with a file stays open");
    let second = s.document_id();
    assert_eq!(text(&s), "Two");

    // The same file again, by another spelling: it becomes the active document.
    let again = dir.join(".").join("one.txt");
    let r = run(&mut s, "file.open", json!({"path": again.to_string_lossy()}));
    assert_eq!((r["alreadyOpen"].as_bool(), r["id"].as_u64()), (Some(true), Some(first)));
    assert_eq!((s.document_id(), s.documents().len()), (first, 2));
    assert_eq!(s.find_open(&two), Some(second));

    // A failed open changes nothing.
    assert!(s.run("file.open", &json!({"path": dir.join("missing.docx").to_string_lossy()})).is_err());
    assert_eq!((ids(&s), s.document_id()), (vec![first, second], first));

    // An edited blank document isn't replaced.
    let mut s = self::s();
    run(&mut s, "text.insert", json!({"text": "x"}));
    run(&mut s, "text.backspace", json!({}));
    assert!(!s.is_untouched_blank());
    run(&mut s, "file.open", json!({"path": one.to_string_lossy()}));
    assert_eq!(s.documents().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn close_activates_the_previous_document_and_the_last_leaves_a_blank_one() {
    let mut s = s();
    let a = s.document_id();
    let b = s.add_document(wordcraft_doc::Document::from_text("B"));
    let c = s.add_document(wordcraft_doc::Document::from_text("C"));
    s.activate(b).unwrap();
    s.close_document(b).unwrap();
    assert_eq!((ids(&s), s.document_id()), (vec![a, c], a), "the one opened before it");
    s.close_document(c).unwrap();
    s.close_document(a).unwrap();
    assert_eq!(s.documents().len(), 1);
    assert!(![a, b, c].contains(&s.document_id()));
    assert!(s.is_untouched_blank());
    assert!(s.close_document(a).is_err(), "closed documents are gone");

    // Closing the first makes the next one active.
    let d = s.document_id();
    let e = s.add_document(wordcraft_doc::Document::new());
    s.activate(d).unwrap();
    s.close_document(d).unwrap();
    assert_eq!(s.document_id(), e);
}

#[test]
fn close_document_refuses_unsaved_changes_unless_forced() {
    let mut s = s();
    let a = s.document_id();
    run(&mut s, "text.insert", json!({"text": "Draft"}));
    let b = s.add_document(wordcraft_doc::Document::new());
    assert!(s.run("file.closeDocument", &json!({"id": a})).is_err());
    assert_eq!(ids(&s), vec![a, b], "still open");
    let r = run(&mut s, "file.closeDocument", json!({}));
    assert_eq!((r["closed"].as_u64(), r["active"].as_u64()), (Some(b), Some(a)));
    let r = run(&mut s, "file.closeDocument", json!({"id": a, "force": true}));
    assert_eq!(r["documents"].as_array().map(Vec::len), Some(1));
    assert!(s.is_untouched_blank());
    for bad in [json!({"id": -1}), json!({"id": "x"}), json!({"id": u64::MAX}), json!({"id": 1.5}), json!({"id": []})] {
        assert!(s.run("file.closeDocument", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.documents().len(), 1);
}

/// Work addressed to a document by id lands in it, whichever document is active, and the active
/// one is left as it was.
#[test]
fn with_document_works_on_the_given_document_and_keeps_the_active_one() {
    let mut s = s();
    let a = s.document_id();
    run(&mut s, "text.insert", json!({"text": "Alpha"}));
    let b = s.add_document(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "Beta"}));
    let (b_rev, b_undo) = (s.rev(), s.undo_depth());

    let r = s.with_document(a, |s| {
        s.run("text.insert", &json!({"text": " more"})).unwrap();
        s.run("edit.advancedFind", &json!({})).ok();
        s.document_id()
    });
    assert_eq!(r.unwrap(), a);
    assert_eq!(s.document_id(), b);
    assert_eq!((text(&s), s.rev(), s.undo_depth()), ("Beta".to_string(), b_rev, b_undo), "B untouched");
    assert_eq!(s.with_document(a, |s| text(s)).unwrap(), "Alpha more");
    assert!(s.ui_requests.iter().all(|q| q.get("open").is_none() || q["document"] == a), "requests name their document: {:?}", s.ui_requests);
    assert!(s.with_document(u64::MAX, |_| ()).is_err());

    // Closing the active document inside leaves the one it ran on active.
    s.with_document(a, |s| s.close_document(b)).unwrap().unwrap();
    assert_eq!(ids(&s), vec![a]);
    assert_eq!(s.document_id(), a);
}

/// A Read Aloud reading names its document: switching documents stops it, and turning citation
/// skipping on in another document doesn't restart it at the first document's positions there.
#[test]
fn read_aloud_belongs_to_its_document() {
    let mut s = s();
    let a = s.document_id();
    run(&mut s, "text.insert", json!({"text": "One sentence. Another one."}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "review.readAloud", json!({}));
    assert_eq!(s.read_aloud.document, Some(a));
    let b = s.add_document(wordcraft_doc::Document::from_text("Other text."));
    assert_eq!(s.read_aloud.state(), crate::speech::State::Stopped, "the reading of A stopped");
    run(&mut s, "readAloud.skipCitations", json!({"value": false}));
    assert_eq!(s.read_aloud.document, Some(a), "nothing restarted in B");
    run(&mut s, "review.readAloud", json!({}));
    assert_eq!(s.read_aloud.document, Some(b));
}

#[test]
fn switch_windows_lists_and_activates() {
    let mut s = s();
    let a = s.document_id();
    let b = s.add_document(wordcraft_doc::Document::new());
    let r = run(&mut s, "view.switchWindows", json!({}));
    let list = r["documents"].as_array().unwrap();
    assert_eq!(
        list.iter().map(|d| (d["id"].as_u64(), d["title"].as_str(), d["active"].as_bool())).collect::<Vec<_>>(),
        vec![(Some(a), Some("Document1"), Some(false)), (Some(b), Some("Document2"), Some(true))]
    );
    assert!(list[0].get("path").is_some() && list[0].get("dirty").is_some());
    run(&mut s, "view.switchWindows", json!({"id": a}));
    assert_eq!(s.document_id(), a);
    for bad in [json!({"id": 0}), json!({"id": u64::MAX}), json!({"id": -5}), json!({"id": "1"}), json!({"id": 1e300}), json!({"id": {}})] {
        assert!(s.run("view.switchWindows", &bad).is_err(), "{bad}");
        assert_eq!(s.document_id(), a);
    }
    let junk = [json!(null), json!([]), json!({"id": null}), json!({"text": 5})];
    for id in ["view.switchWindows", "file.closeDocument", "file.new"] {
        for j in &junk {
            let _ = s.run(id, j);
            s.clamp_selection();
            let _ = s.layout();
        }
    }
    assert!(!s.documents().is_empty());
}

#[test]
fn document_ids_are_unique_across_sessions_and_replacements() {
    let mut one = s();
    let two = s();
    assert_ne!(one.document_id(), two.document_id());
    let before = one.document_id();
    one.set_document(wordcraft_doc::Document::new());
    assert_ne!(one.document_id(), before);
    assert_eq!(ids(&one), vec![one.document_id()], "the replacement takes its place in the list");
}

/// A Start screen's stand-in document gives back its name: the first document a person makes is
/// `Document1`, the next `Document2`, and after closing the last one, New makes `Document2`
/// again only if no other document has taken it.
#[test]
fn a_stand_in_document_gives_back_its_untitled_name() {
    let mut s = s();
    let placeholder = s.document_id();
    s.release_untitled_name();
    run(&mut s, "file.new", json!({}));
    s.close_document(placeholder).unwrap();
    assert_eq!(s.documents()[0].title, "Document1");
    run(&mut s, "file.new", json!({}));
    assert_eq!(s.documents()[1].title, "Document2");
    // A document with work keeps its name.
    run(&mut s, "text.insert", json!({"text": "Work"}));
    s.release_untitled_name();
    run(&mut s, "file.new", json!({}));
    assert_eq!(s.documents()[2].title, "Document3");
}

/// Two windows on one document: each keeps its own caret and zoom; a caret inside text another
/// window deleted is clamped, so typing there lands at the end of what is left; Undo in either
/// window undoes the document's last edit, and typing in one window never joins the other's
/// undo step.
#[test]
fn two_views_of_one_document_keep_their_own_caret_and_share_undo() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello world"}));
    let mut b = s.copy_view();
    let a_caret = s.sel.focus.clone();

    // Window B: zoom in; its caret is after "world".
    let mut a = s.take_view();
    s.put_view(b);
    run(&mut s, "view.zoom", json!({"value": 150}));
    b = s.take_view();
    s.put_view(a);
    assert_eq!(s.view.zoom, 1.0, "window A keeps its zoom");

    // Window A deletes " world", where B's caret is.
    run(&mut s, "select.text", json!({"text": " world"}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(text(&s).trim(), "Hello");
    a = s.take_view();
    s.put_view(b);
    assert!((s.view.zoom - 1.5).abs() < 1e-6, "window B keeps its zoom");
    assert_eq!(s.sel.focus.off, 5, "B's caret is clamped into what is left");
    run(&mut s, "text.insert", json!({"text": "!"}));
    assert_eq!(text(&s).trim(), "Hello!");

    // Undo in A undoes B's typing; Undo in B then undoes A's delete.
    b = s.take_view();
    s.put_view(a);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s).trim(), "Hello");
    a = s.take_view();
    s.put_view(b);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s).trim(), "Hello world");

    // Typing in A, then B, then A again: three undo steps.
    b = s.take_view();
    s.put_view(a);
    s.sel = crate::Selection::caret(a_caret);
    run(&mut s, "text.insert", json!({"text": "x"}));
    a = s.take_view();
    s.put_view(b);
    run(&mut s, "text.insert", json!({"text": "y"}));
    b = s.take_view();
    s.put_view(a);
    run(&mut s, "text.insert", json!({"text": "z"}));
    run(&mut s, "edit.undo", json!({}));
    assert!(!text(&s).contains('z') && text(&s).contains('x') && text(&s).contains('y'), "{}", text(&s));
    drop(b);
}

/// An input method's promise that the next insertion joins its replacement belongs to the
/// window that made it: another window on the document typing at the same place gets an undo
/// step of its own.
#[test]
fn input_method_joins_stay_with_their_window() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    s.join_next_ime_text_replacement();
    let b = s.copy_view();
    let a = s.take_view();
    s.put_view(b);
    run(&mut s, "text.insert", json!({"text": "y"}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s).trim(), "Hello", "B's typing is its own undo step");
    drop(a);
}
