//! Native JSON snapshots for a host that stores complete editor versions.

use wordcraft_engine::Session;

/// Serialize only the native model, without changing dirty state, undo or file identity.
/// Parsing our own output also rejects values (such as a non-finite required float) that
/// serialize but cannot be opened as a native Document.
pub fn native_snapshot(session: &Session) -> Result<Vec<u8>, String> {
    let bytes = wordcraft_engine::io::save_bytes("snapshot.json", &session.doc)?;
    let _: wordcraft_doc::Document = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// An opaque token: include document identity, and keep u64 values out of JS numbers.
pub fn revision(session: &Session, draft_revision: u64) -> String {
    format!("{}:{}:{}", session.document_id(), session.rev(), draft_revision)
}

#[derive(Default)]
pub struct RevisionObserver(Option<(u64, u64, u64)>);

impl RevisionObserver {
    pub fn changed(&mut self, session: &Session, draft_revision: u64) -> bool {
        let current = (session.document_id(), session.rev(), draft_revision);
        if self.0 == Some(current) {
            return false;
        }
        self.0 = Some(current);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::{InlineObject, Part, PartKind};
    use wordcraft_engine::Session;

    #[test]
    fn native_snapshot_preserves_complete_model_and_does_not_mark_saved() {
        let mut session = Session::new(wordcraft_engine::sample::sample_document());
        session.doc.parts.insert(
            600,
            Part {
                kind: PartKind::Comment,
                blocks: vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("Snapshot comment", Default::default()))],
                ..Default::default()
            },
        );
        session.doc.comments.insert(1, wordcraft_doc::Comment { author: "Tester".into(), part: 600, ..Default::default() });
        let image = session.doc.add_media(vec![71; 3_200_000], "png");
        let mut para = wordcraft_doc::Paragraph::with_text("\u{fffc}", Default::default());
        para.objects.push(InlineObject::Image {
            media: image,
            w: 10.,
            h: 10.,
            alt: "Large image".into(),
            float: Default::default(),
            crop: [0.; 4],
            ole: None,
        });
        session.doc.body.push(wordcraft_doc::para_block(para));
        session.run("text.insert", &serde_json::json!({"text":"Edited "})).unwrap();
        let document_id = session.document_id();
        let revision = session.rev();
        let undo_depth = session.undo_depth();
        let before = serde_json::to_value(&session.doc).unwrap();
        let bytes = native_snapshot(&session).unwrap();
        assert!(bytes.len() > 4 * 1024 * 1024);
        let reopened = wordcraft_engine::io::open_bytes("version.json", &bytes).unwrap();
        assert_eq!(serde_json::to_value(&reopened).unwrap(), before);
        assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
        assert!(session.dirty);
        assert_eq!(session.document_id(), document_id);
        assert_eq!(session.rev(), revision);
        assert_eq!(session.undo_depth(), undo_depth);
    }
    #[test]
    fn changes_report_every_revision_even_while_the_document_stays_dirty() {
        let mut session = Session::new(wordcraft_doc::Document::new());
        let mut observer = RevisionObserver::default();
        assert!(observer.changed(&session, 0));
        assert!(!observer.changed(&session, 0));
        session.run("text.insert", &serde_json::json!({"text":"first"})).unwrap();
        assert!(session.dirty);
        assert!(observer.changed(&session, 0));
        let first = revision(&session, 0);
        session.run("text.insert", &serde_json::json!({"text":"second"})).unwrap();
        assert!(session.dirty);
        assert!(observer.changed(&session, 0));
        assert_ne!(revision(&session, 0), first);
        assert!(!observer.changed(&session, 0));
        session.set_document(wordcraft_doc::Document::new());
        assert!(observer.changed(&session, 0));
    }
    #[test]
    fn pending_draft_changes_are_reported_without_serializing_the_document() {
        let session = Session::new(wordcraft_doc::Document::new());
        let mut observer = RevisionObserver::default();
        assert!(observer.changed(&session, 0));
        assert!(observer.changed(&session, 1));
        assert!(observer.changed(&session, 2));
        assert!(!observer.changed(&session, 2));
        assert_ne!(revision(&session, 1), revision(&session, 2));
    }
}
