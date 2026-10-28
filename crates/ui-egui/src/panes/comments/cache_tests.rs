use super::*;

fn app() -> WordApp {
    let mut app = WordApp::new(
        wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&"Original paragraph.\n".repeat(2_000))),
        crate::Services::default(),
    );
    app.run("review.newComment", json!({"text": "Original comment"})).unwrap();
    app
}

fn paint(ctx: &egui::Context, app: &mut WordApp) -> Vec<String> {
    let out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 800.0))), ..Default::default() },
        |ui| comments(app, ui),
    );
    let texts = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect();
    out.drop_without_applying_deltas();
    texts
}

#[test]
fn comments_pane_reuses_unchanged_list_across_repaints() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app();
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let revision = app.session.rev();
    LIST_BUILDS.with(|n| n.set(0));
    for _ in 0..4 {
        assert!(paint(&ctx, &mut app).iter().any(|t| t.contains("Original comment")));
    }
    assert_eq!(app.session.rev(), revision);
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    assert_eq!(LIST_BUILDS.with(|n| n.get()), 1, "unchanged repaints must reuse the ordered comment list");
    app.run("caret.docEnd", json!({})).unwrap();
    paint(&ctx, &mut app);
    assert_eq!(LIST_BUILDS.with(|n| n.get()), 1, "moving the caret does not change the list");
    let list = cached_comments(&mut app);
    drafts(&mut app).edits.insert(0, "Uncommitted draft".into());
    assert!(paint(&ctx, &mut app).iter().any(|t| t.contains("Uncommitted draft")));
    assert!(std::sync::Arc::ptr_eq(&list, &cached_comments(&mut app)), "draft text stays live without rebuilding metadata");
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before, "rendering a draft does not save it");
}

#[test]
fn comments_pane_refreshes_cards_and_anchors_after_edits_and_history() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app();
    paint(&ctx, &mut app);
    let anchor = cached_comments(&mut app)[0].anchor.clone().unwrap();
    app.run("caret.docStart", json!({})).unwrap();
    app.run("text.insert", json!({"text": "Prefix ", "raw": true})).unwrap();
    paint(&ctx, &mut app);
    assert_eq!(cached_comments(&mut app)[0].anchor.as_ref().unwrap().off, anchor.off + 7);
    app.run("edit.undo", json!({})).unwrap();
    paint(&ctx, &mut app);
    assert_eq!(cached_comments(&mut app)[0].anchor, Some(anchor));
    app.run("review.editComment", json!({"id": 0, "text": "Edited comment"})).unwrap();
    assert!(paint(&ctx, &mut app).iter().any(|t| t.contains("Edited comment")));
    app.run("edit.undo", json!({})).unwrap();
    assert!(paint(&ctx, &mut app).iter().any(|t| t.contains("Original comment")));
    app.run("edit.redo", json!({})).unwrap();
    assert!(paint(&ctx, &mut app).iter().any(|t| t.contains("Edited comment")));
    app.run("review.reply", json!({"id": 0, "text": "A reply"})).unwrap();
    paint(&ctx, &mut app);
    let cards = cached_comments(&mut app);
    assert_eq!(cards.iter().map(|c| c.id).collect::<Vec<_>>(), vec![0, 1]);
    assert!(cards[1].reply && cards[1].anchor.is_none());
    app.run("review.resolveComment", json!({"id": 0})).unwrap();
    assert!(paint(&ctx, &mut app).iter().any(|t| t == "Resolved"));
    assert!(cached_comments(&mut app)[0].resolved);
    app.run("review.resolveComment", json!({"id": 0})).unwrap();
    paint(&ctx, &mut app);
    assert!(!cached_comments(&mut app)[0].resolved);
    app.run("review.deleteComment", json!({"id": 0})).unwrap();
    paint(&ctx, &mut app);
    assert!(cached_comments(&mut app).is_empty());
    app.run("edit.undo", json!({})).unwrap();
    paint(&ctx, &mut app);
    assert_eq!(cached_comments(&mut app).len(), 2);
}

#[test]
fn comments_pane_snapshot_matches_command_across_stories() {
    use wordcraft_doc::{InlineObject, StoryRef};
    let mut app = app();
    app.run("insert.header", json!({"text": "Header witness"})).unwrap();
    let header = app.session.sel.focus.story;
    assert!(matches!(header, StoryRef::Part(_)));
    app.run("review.newComment", json!({"text": "Header comment"})).unwrap();
    // A duplicate start and an unanchored reply retain the command's exact order.
    app.session.doc.insert_object(&app.session.sel.focus.clone(), InlineObject::CommentStart { id: 1 }, &Default::default()).unwrap();
    app.run("review.reply", json!({"id": 0, "text": "Reply witness"})).unwrap();
    let expected = app.session.run("review.comments", &json!({})).unwrap();
    let cards = cached_comments(&mut app);
    let actual: Vec<_> = cards
        .iter()
        .map(
            |c| json!({"id": c.id, "author": c.author, "parent": app.session.doc.comments[&c.id].parent, "resolved": c.resolved, "anchor": c.anchor}),
        )
        .collect();
    for (actual, expected) in actual.iter().zip(expected.as_array().unwrap()) {
        for key in ["id", "author", "parent", "resolved", "anchor"] {
            assert_eq!(actual[key], expected[key], "{key}");
        }
    }
    assert_eq!(actual.len(), expected.as_array().unwrap().len());
    assert!(cards.iter().any(|c| c.anchor.as_ref().is_some_and(|p| p.story == header)));
}

#[test]
fn comments_pane_refreshes_on_equal_revision_document_switch_and_replacement() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app();
    let first = app.session.document_id();
    let revision = app.session.rev();
    paint(&ctx, &mut app);
    let second = app.session.add_document(wordcraft_doc::Document::from_text("Other document"));
    app.run("review.newComment", json!({"text": "Other comment"})).unwrap();
    assert_eq!(app.session.rev(), revision, "document identity must distinguish equal revisions");
    assert!(paint(&ctx, &mut app).iter().any(|t| t.contains("Other comment")));
    app.session.switch_document(first).unwrap();
    assert!(paint(&ctx, &mut app).iter().any(|t| t.contains("Original comment")));
    app.session.switch_document(second).unwrap();
    assert!(paint(&ctx, &mut app).iter().any(|t| t.contains("Other comment")));
    app.session.set_document(wordcraft_doc::Document::new());
    paint(&ctx, &mut app);
    assert!(cached_comments(&mut app).is_empty());
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn comments_pane_cache_follows_window_ownership() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app();
    let first = app.windows.shown();
    paint(&ctx, &mut app);
    let weak = std::sync::Arc::downgrade(&app.comment_cache.items);
    app.new_window();
    let second = app.windows.shown();
    assert_ne!(first, second);
    LIST_BUILDS.with(|n| n.set(0));
    for _ in 0..4 {
        for key in [first, second] {
            assert!(app.enter_window(key));
            paint(&ctx, &mut app);
        }
    }
    assert_eq!(LIST_BUILDS.with(|n| n.get()), 1, "the first window keeps its list; only the new window builds");
    assert!(weak.upgrade().is_some());
    app.close_window(first);
    assert_eq!(app.window_count(), 1);
    assert!(weak.upgrade().is_none(), "closing the window releases its list while egui stays alive");
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn comments_pane_cached_cards_keep_buttons_live() {
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    app.run("caret.docEnd", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(400.0, 800.0))
        .with_context_setup(crate::theme::install_fonts)
        .build_ui_state(|ui, app: &mut WordApp| comments(app, ui), app);
    h.run_steps(4);
    let anchor = h.state().comment_cache.items[0].anchor.clone().unwrap();
    h.get_by_label("Go to").click();
    h.run_steps(2);
    assert_eq!(h.state().session.sel.focus, anchor);
    h.get_by_label("Resolve").click();
    h.run_steps(2);
    assert!(h.state().session.doc.comments[&0].resolved);
    h.get_by_label("Reopen").click();
    h.run_steps(2);
    assert!(!h.state().session.doc.comments[&0].resolved);
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert!(h.state().session.doc.comments.is_empty());
}
