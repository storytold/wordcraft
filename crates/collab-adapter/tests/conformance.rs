//! The five conformance cases of the first adapter increment.
//!
//! `docs/collab/adapters.md` names them for the increment that lands before the desktop and web
//! wiring move onto the seam: two clients converging, a reconnect, a structural edit against a
//! text edit on the same scope, a viewer refused, and a 100-page document's round-trip. They run
//! against the in-process relay (`MockRelay`), which is the same seam the real backend implements.

use std::sync::Arc;

use wordcraft_collab_adapter::edit::{Edit, EditError};
use wordcraft_collab_adapter::ids::BlockId;
use wordcraft_collab_adapter::relay::MockRelay;
use wordcraft_collab_adapter::seam::{RefusalCode, Role, Session};
use wordcraft_collab_adapter::session::DocSession;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Block, Document, Paragraph, StoryRef};

fn document(texts: &[&str]) -> Document {
    let mut doc = Document::new();
    doc.body.clear();
    for text in texts {
        doc.body.push(Arc::new(Block::Para(Paragraph::with_text(text, CharProps::default()))));
    }
    doc
}

fn body_text(doc: &Document) -> Vec<String> {
    doc.body.iter().map(|block| block.as_para().map(Paragraph::plain_text).unwrap_or_default()).collect::<Vec<_>>()
}

fn attach(relay: &MockRelay, peer: &str, role: Role, base: &Document) -> DocSession {
    let client = relay.connect(peer);
    DocSession::attach(peer, role, base.clone(), Box::new(client)).expect("the peer joins")
}

fn pages(doc: &Document) -> usize {
    wordcraft_engine::Session::new(doc.clone()).layout().pages.len()
}

/// One line of the long document: long enough that two of them fill a line of body text, which is
/// what makes 3000 of them run past a hundred pages.
fn long_line(index: usize) -> String {
    format!(
        "Paragraph {index} of a long document carries a sentence long enough that two of them fill a line of body text, the way a page of ordinary prose does."
    )
}

#[test]
fn two_clients_converge_and_a_late_joiner_catches_up() {
    let base = document(&["First", "Second", "Third", "Fourth"]);
    let ids = wordcraft_collab_adapter::ids::IdMap::bind(&base, 0);
    let (first, second) = (ids.id_at(0).expect("an id"), ids.id_at(1).expect("an id"));
    let third = ids.id_at(2).expect("an id");
    let relay = MockRelay::new(base.clone());
    let mut alice = attach(&relay, "alice", Role::Editor, &base);
    let mut bob = attach(&relay, "bob", Role::Editor, &base);

    alice.submit(Edit::InsertText { block: first, at: 0, text: "Alice: ".into() }).expect("alice types");
    assert_eq!(bob.sync().expect("bob syncs"), 1);
    bob.submit(Edit::InsertText { block: second, at: 0, text: "Bob: ".into() }).expect("bob types");
    alice.sync().expect("alice syncs");

    // Both address the same paragraph from the same version: the second submission is refused
    // as stale, syncs and lands, so the session agrees on one order.
    alice.submit(Edit::InsertText { block: third, at: 0, text: "a".into() }).expect("alice types first");
    let stale = bob.submit(Edit::InsertText { block: third, at: 0, text: "b".into() }).expect_err("bob's base is behind");
    assert_eq!(stale.code, RefusalCode::Stale);
    bob.sync().expect("bob catches up");
    bob.submit(Edit::InsertText { block: third, at: 0, text: "b".into() }).expect("the retry lands");

    // A peer that joins afterwards replays the whole journal from the base document.
    let mut carol = attach(&relay, "carol", Role::Editor, &base);
    assert_eq!(carol.sync().expect("nothing new"), 0);
    alice.sync().expect("alice syncs");
    bob.sync().expect("bob syncs");

    let text = alice.text();
    assert_eq!(text, bob.text());
    assert_eq!(text, carol.text());
    assert_eq!(body_text(&relay.document().expect("the relay document")), body_text(&alice.doc));
    assert_eq!(body_text(&alice.doc), ["Alice: First", "Bob: Second", "baThird", "Fourth"]);
    assert_eq!(relay.head(), 4, "one sequence number per accepted edit");
    assert_eq!(relay.journal_len(), 4, "the journal keeps every accepted edit");
}

#[test]
fn a_reconnect_replays_the_journal_it_missed() {
    let base = document(&["one", "two", "three"]);
    let ids = wordcraft_collab_adapter::ids::IdMap::bind(&base, 0);
    let blocks: Vec<BlockId> = (0..3).filter_map(|index| ids.id_at(index)).collect();
    let relay = MockRelay::new(base.clone());
    let mut alice = attach(&relay, "alice", Role::Editor, &base);
    let mut bob = attach(&relay, "bob", Role::Editor, &base);

    alice.submit(Edit::InsertText { block: blocks[0], at: 0, text: "A".into() }).expect("alice types");
    bob.sync().expect("bob syncs");

    // Alice's socket goes away; bob keeps working on the same document.
    bob.submit(Edit::InsertText { block: blocks[1], at: 3, text: "!".into() }).expect("bob types");
    bob.submit(Edit::InsertText { block: blocks[2], at: 0, text: "B".into() }).expect("bob types again");
    assert_eq!(bob.version(), 3);

    // She comes back with the version she had applied, and only the tail is replayed.
    alice.reconnect(Box::new(relay.connect("alice"))).expect("alice reconnects");
    assert_eq!(alice.version(), 3);
    assert_eq!(alice.text(), bob.text());
    assert_eq!(body_text(&alice.doc), ["Aone", "two!", "Bthree"]);
    assert_eq!(relay.journal_len(), 3, "the reconnect applied the tail, it did not resubmit it");

    // A peer asking for a point the journal cannot serve is told to resync, not handed a gap.
    let mut ahead = relay.connect("late");
    let refused = ahead.join(Role::Editor, Some(99)).expect_err("a version the relay never reached");
    assert_eq!(refused.code, RefusalCode::Resync);
}

#[test]
fn a_structural_edit_against_a_text_edit_has_one_outcome_on_every_replica() {
    let base = document(&["one", "two", "three"]);
    let ids = wordcraft_collab_adapter::ids::IdMap::bind(&base, 0);
    let removed = ids.id_at(1).expect("an id");
    let relay = MockRelay::new(base.clone());
    let mut alice = attach(&relay, "alice", Role::Editor, &base);
    let mut bob = attach(&relay, "bob", Role::Editor, &base);

    // Bob types into the paragraph; the text is sequenced first.
    bob.submit(Edit::InsertText { block: removed, at: 0, text: "typed ".into() }).expect("bob types");
    alice.sync().expect("alice syncs");

    // Alice removes the paragraph the text was in: the structural edit wins visibility.
    alice.submit(Edit::RemoveParagraph { block: removed }).expect("alice removes it");
    bob.sync().expect("bob applies the removal");

    // Bob's next edit for the same scope arrives after the acknowledged removal: it is refused,
    // the block does not come back, and the text he typed is handed back to him.
    let after = bob.submit(Edit::InsertText { block: removed, at: 0, text: "still here ".into() }).expect_err("the scope is gone");
    assert_eq!(after.code, RefusalCode::Gone);
    let recovered = bob.recovered();
    assert_eq!(recovered.len(), 1, "the refused text is recoverable");
    assert_eq!(recovered[0].text, "still here ");
    assert_eq!(recovered[0].block, removed);

    assert_eq!(alice.text(), bob.text());
    assert_eq!(body_text(&alice.doc), ["one", "three"]);
    assert!(!bob.text().contains("still here"), "the removed scope never resurrects the text");
    assert!(!alice.text().contains("typed"), "the removal dropped the paragraph's text on every replica");
    assert_eq!(body_text(&relay.document().expect("the relay document")), ["one", "three"]);
    // A second attempt with the same stale id is refused again, never applied to another block.
    let again = bob.submit(Edit::DeleteRange { block: removed, from: 0, to: 1 }).expect_err("gone");
    assert_eq!(again.code, RefusalCode::Gone);
}

#[test]
fn a_viewer_is_refused_at_the_seam_and_at_the_backend() {
    let base = document(&["one"]);
    let ids = wordcraft_collab_adapter::ids::IdMap::bind(&base, 0);
    let block = ids.id_at(0).expect("an id");
    let relay = MockRelay::new(base.clone());
    let mut viewer = attach(&relay, "viewer", Role::Viewer, &base);
    let mut editor = attach(&relay, "editor", Role::Editor, &base);

    let refused = viewer.submit(Edit::InsertText { block, at: 0, text: "x".into() }).expect_err("a viewer does not write");
    assert_eq!(refused.code, RefusalCode::ReadOnly);
    assert_eq!(viewer.version(), 0);
    assert_eq!(relay.head(), 0, "nothing was sequenced for the refused edit");
    assert_eq!(viewer.text(), "one");

    // The adapter is not the only gate: a submission that bypasses it is refused by the relay.
    let mut raw = relay.connect("raw-viewer");
    raw.join(Role::Viewer, None).expect("the raw peer joins");
    let at_backend = raw.submit(Edit::InsertText { block, at: 0, text: "x".into() }, 0).expect_err("the relay refuses a viewer");
    assert_eq!(at_backend.code, RefusalCode::ReadOnly);

    // Presence is allowed without edit permission and is latest-wins per peer.
    viewer.set_caret(Some("para 1")).expect("a viewer shows a caret");
    viewer.set_caret(Some("para 1, word 3")).expect("the new position replaces the old one");
    let peers = editor.peers();
    let seen = peers.iter().find(|peer| peer.id == "viewer").expect("the viewer is in the participant list");
    assert_eq!(seen.role, Role::Viewer);
    assert_eq!(seen.at.as_deref(), Some("para 1, word 3"));
    assert!(!viewer.text().contains('x'));

    editor.submit(Edit::InsertText { block, at: 0, text: "edit ".into() }).expect("an editor writes");
    assert_eq!(editor.text(), "edit one");
}

#[test]
fn a_hundred_page_document_round_trips_through_docx_with_its_unmodelled_part() {
    // A part the model does not read: a custom XML part is related from the main part, kept
    // verbatim on open and written back unchanged. Nothing in the adapter re-encodes it.
    const PART: &str = "customXml/item1.xml";
    let probe: &[u8] = b"<?xml version=\"1.0\"?><craft:probe xmlns:craft=\"urn:craft:probe\">keep me byte for byte</craft:probe>";

    // The file the session opens: a hundred-page document and the part beside it.
    let bytes = docx_with_part(3000, PART, probe);
    let base = wordcraft_docx::read(&bytes).expect("the fixture reads");
    assert!(base.passthrough.contains_key(PART), "the reader keeps the part it does not model");
    let ids = wordcraft_collab_adapter::ids::IdMap::bind(&base, 0);
    assert_eq!(ids.len(), 3000, "every paragraph got a session id");
    let first = ids.id_at(0).expect("an id");
    let middle = ids.id_at(1500).expect("an id");
    let last = ids.id_at(2999).expect("an id");

    let relay = MockRelay::new(base.clone());
    let mut alice = attach(&relay, "alice", Role::Editor, &base);
    let mut bob = attach(&relay, "bob", Role::Editor, &base);

    alice.submit(Edit::InsertText { block: first, at: 0, text: "Alice was here. ".into() }).expect("alice types");
    bob.sync().expect("bob syncs");
    bob.submit(Edit::InsertText { block: middle, at: 0, text: "Bob was here. ".into() }).expect("bob types");
    alice.sync().expect("alice syncs");
    bob.submit(Edit::InsertText { block: last, at: 0, text: "Bob was here too. ".into() }).expect("bob types");
    alice.sync().expect("alice syncs");

    // Pagination is a property of the document, not of the session: every replica computes the
    // same page count, and it is still a hundred-page document.
    let alice_pages = pages(&alice.doc);
    assert!(alice_pages >= 100, "the document paginates {alice_pages} pages, not a hundred");
    assert_eq!(alice_pages, pages(&bob.doc), "the same pagination on both replicas");
    assert_eq!(alice_pages, pages(&relay.document().expect("the relay document")));

    // The saved file opens again with the edited text, the same pagination and the part byte for
    // byte — on both replicas, whose documents converged.
    for replica in [&alice.doc, &bob.doc] {
        let saved = wordcraft_docx::write(replica).expect("the document writes");
        let reopened = wordcraft_docx::read(&saved).expect("the document reads back");
        assert_eq!(reopened.plain_text(StoryRef::Body), replica.plain_text(StoryRef::Body));
        assert_eq!(pages(&reopened), alice_pages);
        let part = reopened.passthrough.get(PART).expect("the custom XML part survived the save");
        assert_eq!(part.as_slice(), probe);
    }
    assert_eq!(bob.text(), alice.text());
    assert!(alice.text().contains("Alice was here. "));
    assert!(alice.text().contains("Bob was here too. "));

    // An edit naming a block that never existed is refused without touching the document.
    let unknown = wordcraft_collab_adapter::edit::apply(
        &mut alice.doc,
        &mut alice.ids,
        &Edit::InsertText { block: BlockId(u32::MAX), at: 0, text: "x".into() },
    );
    assert_eq!(unknown, Err(EditError::Gone));
}

/// A `.docx` package built by hand: `paragraphs` paragraphs of [`long_line`] and one custom XML
/// part related from the main part, with the content types both need.
fn docx_with_part(paragraphs: usize, part: &str, probe: &[u8]) -> Vec<u8> {
    let mut body = String::new();
    for index in 0..paragraphs {
        body.push_str("<w:p><w:r><w:t xml:space=\"preserve\">");
        body.push_str(&long_line(index));
        body.push_str("</w:t></w:r></w:p>");
    }
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr/></w:body></w:document>"
    );
    let content_types = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/{part}\" ContentType=\"application/vnd.openxmlformats-officedocument.customXml+xml\"/></Types>"
    );
    let root = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>";
    let document_rels = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml\" Target=\"../{part}\"/></Relationships>"
    );
    let mut zw = craft_zip::Writer::new();
    for (name, bytes) in [
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", root.as_bytes()),
        ("word/document.xml", document.as_bytes()),
        ("word/_rels/document.xml.rels", document_rels.as_bytes()),
        (part, probe),
    ] {
        zw.add(name, bytes, craft_zip::Compression::Deflated).expect("the fixture packs");
    }
    zw.finish().expect("the fixture closes")
}
