//! #1690, finding 3: malformed command IDs must never target another comment.
use serde_json::{Value, json};
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::{CmdError, Session};

fn commented() -> Session {
    let mut s = Session::new(Document::new());
    s.run("text.insert", &json!({"text": "Keep this text"})).unwrap();
    s.run("select.text", &json!({"text": "this"})).unwrap();
    assert_eq!(s.run("review.newComment", &json!({"text": "Keep this comment"})).unwrap()["id"], 0);
    s.run("select.text", &json!({"text": "this"})).unwrap();
    s.run("select.collapse", &json!({})).unwrap();
    s
}

fn invalid_ids_leave_state_untouched(command: &str) {
    for id in [json!(-1), json!(0.5), json!(0.0), json!("0"), Value::Null, json!(false), json!(4294967296u64)] {
        for at_comment in [true, false] {
            let mut s = commented();
            if !at_comment {
                s.run("caret.docEnd", &json!({})).unwrap();
                s.run("text.newParagraph", &json!({})).unwrap();
            }
            // Keep a redo step too: a refused command must not clear it.
            s.run("format.bold", &json!({})).unwrap();
            s.run("edit.undo", &json!({})).unwrap();
            assert!(s.can_redo());
            let before = (s.doc.clone(), s.sel.clone(), s.undo_labels(), s.redo_label().map(str::to_owned), s.dirty);
            let result = s.run(command, &json!({"id": id, "text": "Replacement"}));
            assert!(matches!(result, Err(CmdError::Params(_))), "{command}, id={id}, at_comment={at_comment}: {result:?}");
            assert_eq!(
                (s.doc.clone(), s.sel.clone(), s.undo_labels(), s.redo_label().map(str::to_owned), s.dirty),
                before,
                "{command}, id={id}, at_comment={at_comment}"
            );
            let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
            assert_eq!(back.comments.len(), 1);
            let comment = back.comments.get(&0).unwrap();
            assert!(!comment.resolved);
            assert_eq!(back.plain_text(wordcraft_doc::StoryRef::Part(comment.part)), "Keep this comment");
        }
    }
}

#[test]
fn delete_comment_rejects_invalid_ids() {
    invalid_ids_leave_state_untouched("review.deleteComment");
}

#[test]
fn resolve_comment_rejects_invalid_ids() {
    invalid_ids_leave_state_untouched("review.resolveComment");
}

#[test]
fn reply_rejects_invalid_comment_ids() {
    invalid_ids_leave_state_untouched("review.reply");
}

#[test]
fn edit_comment_rejects_invalid_ids() {
    invalid_ids_leave_state_untouched("review.editComment");
}

#[test]
fn delete_all_rejects_an_explicit_invalid_id() {
    let mut s = commented();
    let before = s.doc.clone();
    assert!(matches!(s.run("review.deleteComment", &json!({"all": true, "id": null})), Err(CmdError::Params(_))));
    assert_eq!(s.doc, before);
    for params in [json!({"all": true}), json!({"all": true, "id": 0})] {
        let mut s = commented();
        let before = s.doc.clone();
        s.run("review.deleteComment", &params).unwrap();
        assert!(s.doc.comments.is_empty());
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc, before);
    }
}

#[test]
fn largest_comment_id_is_accepted_without_a_caret_fallback() {
    let mut s = commented();
    let comment = s.doc.comments.remove(&0).unwrap();
    s.doc.comments.insert(u32::MAX, comment);
    for object in &mut s.doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().objects {
        if let wordcraft_doc::para::InlineObject::CommentStart { id } | wordcraft_doc::para::InlineObject::CommentEnd { id } = object {
            *id = u32::MAX;
        }
    }
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("review.resolveComment", &json!({"id": u32::MAX, "value": true})).unwrap();
    assert!(s.doc.comments.get(&u32::MAX).unwrap().resolved);
    s.run("review.editComment", &json!({"id": u32::MAX, "text": "Still here"})).unwrap();
    s.run("review.deleteComment", &json!({"id": u32::MAX})).unwrap();
    assert!(s.doc.comments.is_empty());
    s.run("edit.undo", &json!({})).unwrap();
    assert!(s.doc.comments.contains_key(&u32::MAX));
}

#[test]
fn valid_comment_ids_and_omitted_caret_ids_keep_undo_redo_and_docx_fidelity() {
    for command in ["review.resolveComment", "review.deleteComment", "review.reply", "review.editComment"] {
        let params = json!({"id": 0, "text": "Changed comment"});
        let mut variants = vec![params];
        if matches!(command, "review.resolveComment" | "review.deleteComment") {
            variants.push(json!({}));
        }
        for params in variants {
            let mut s = commented();
            let before = s.doc.clone();
            let depth = s.undo_depth();
            s.run(command, &params).unwrap();
            let after = s.doc.clone();
            assert_ne!(after, before);
            assert_eq!(s.undo_depth(), depth + 1);
            s.run("edit.undo", &json!({})).unwrap();
            assert_eq!(s.doc, before);
            s.run("edit.redo", &json!({})).unwrap();
            assert_eq!(s.doc, after);
            let comments = |doc: &Document| {
                doc.comments
                    .values()
                    .map(|c| (c.author.clone(), c.initials.clone(), c.date.clone(), c.resolved, c.parent, doc.plain_text(StoryRef::Part(c.part))))
                    .collect::<Vec<_>>()
            };
            let mut back = after.clone();
            for _ in 0..2 {
                back = wordcraft_docx::read(&wordcraft_docx::write(&back).unwrap()).unwrap();
                assert_eq!(comments(&back), comments(&after), "{command}");
                assert_eq!(back.plain_text(StoryRef::Body), after.plain_text(StoryRef::Body), "{command}");
            }
        }
    }
}
