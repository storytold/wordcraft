use super::*;

fn harness(text: &str, pane: bool, active: bool) -> (egui_kittest::Harness<'static, WordApp>, u32) {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Review this")), Default::default());
    app.run("select.text", json!({"text": "this"})).unwrap();
    let id = app.run("review.newComment", json!({"text": text})).unwrap()["id"].as_u64().unwrap() as u32;
    mount(app, id, pane, active)
}

/// Pressing or releasing Ctrl/Cmd while the pointer stays still must update the final
/// window cursor, including balloons painted before the canvas chooses its own cursor.
#[test]
fn comment_link_cursor_tracks_the_modifier_without_clicking() {
    let text = "این صفحه: https://example.com";
    let modifiers = egui::Modifiers { ctrl: !cfg!(target_os = "macos"), mac_cmd: cfg!(target_os = "macos"), command: true, ..Default::default() };
    for (pane, active) in [(true, false), (false, false), (false, true)] {
        let (mut h, _) = harness(text, pane, active);
        let at = glyph_at(&h, text, "https://");
        h.event(egui::Event::PointerMoved(at));
        h.step();
        h.event(egui::Event::ModifiersChanged(modifiers));
        h.step();
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::PointingHand, "pane={pane}, active={active}");
        assert!(h.output().platform_output.commands.is_empty(), "hovering never opens a link");
        if pane || active {
            h.event(egui::Event::PointerMoved(glyph_at(&h, text, "صفحه")));
            h.step();
            assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Text, "plain comment text keeps its editing cursor");
            h.event(egui::Event::PointerMoved(at));
            h.step();
            assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::PointingHand);
        }
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.step();
        if pane || active {
            assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Text, "releasing the modifier restores editing");
        }
    }
}

fn mount(mut app: WordApp, id: u32, pane: bool, active: bool) -> (egui_kittest::Harness<'static, WordApp>, u32) {
    app.run("view.commentsPane", json!({"value": pane})).unwrap();
    if active {
        app.canvas.balloon = Some(id);
    }
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1600.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    steps(&mut h, 6);
    (h, id)
}

#[test]
fn ordinary_comment_clicks_still_edit_and_unsafe_links_never_open() {
    let text = "https://example.com file:///tmp/example javascript:alert(1) custom:open";
    for (pane, active) in [(true, false), (false, true)] {
        let (mut h, id) = harness(text, pane, active);
        let at = glyph_at(&h, text, "https://");
        assert!(click(&mut h, at, egui::Modifiers::NONE).is_empty());
        h.event(egui::Event::Text("Edited ".into()));
        steps(&mut h, 2);
        leave_field(&mut h);
        assert!(comment_text(h.state(), id).contains("Edited "), "ordinary clicks retain editing");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(comment_text(h.state(), id), text, "one undo restores the original comment");
        steps(&mut h, 2);
        for needle in ["file:///", "javascript:", "custom:"] {
            let at = glyph_at(&h, text, needle);
            assert!(click(&mut h, at, egui::Modifiers::COMMAND).is_empty());
        }
    }
}

#[test]
fn reply_links_open_in_the_pane_and_balloon_thread() {
    let text = "Reply: https://example.com/reply";
    for (pane, active) in [(true, false), (false, false), (false, true)] {
        let (mut h, id) = harness("Original", pane, active);
        h.state_mut().run("review.reply", json!({"id": id, "text": text})).unwrap();
        steps(&mut h, 3);
        let before = h.state().session.doc.clone();
        let at = glyph_at(&h, text, "https://");
        assert_eq!(click(&mut h, at, egui::Modifiers::COMMAND), ["https://example.com/reply"]);
        assert_eq!(h.state().session.doc, before);
    }
}

/// Opening an imported label must leave its target and all comment formatting untouched.
/// Exercise the formats that preserve editable comments, and read the saved file again.
#[test]
fn imported_comment_links_keep_their_targets_after_open_and_save() {
    use wordcraft_doc::{Paragraph, para_block, props::CharProps};
    let (h, id) = harness("Original", false, false);
    let mut doc = h.state().session.doc.clone();
    let part = doc.comments[&id].part;
    let mut p = Paragraph::with_text("مراجعه: ", Default::default());
    p.append(Paragraph::with_text("Project website", CharProps { link: Some("https://example.com/project".into()), ..Default::default() }));
    doc.set_story(StoryRef::Part(part), vec![para_block(p)]).unwrap();
    for ext in ["docx", "odt", "rtf"] {
        let name = format!("comments.{ext}");
        let save = |doc: &wordcraft_doc::Document| wordcraft_engine::io::save_bytes(&name, doc).unwrap();
        let open = |bytes: &[u8]| wordcraft_engine::io::open_bytes(&name, bytes).unwrap();
        let loaded = open(&save(&doc));
        let id = *loaded.comments.keys().next().expect("the imported comment");
        let part = loaded.comments[&id].part;
        let text = loaded.plain_text(StoryRef::Part(part));
        for (pane, active) in [(true, false), (false, false), (false, true)] {
            let (mut h, _) = mount(WordApp::new(wordcraft_engine::Session::new(loaded.clone()), Default::default()), id, pane, active);
            let at = glyph_at(&h, &text, "Project");
            assert_eq!(click(&mut h, at, egui::Modifiers::COMMAND), ["https://example.com/project"], "{ext}, pane={pane}, active={active}");
            assert_eq!(h.state().session.doc, loaded);
            assert!(!h.state().session.dirty, "following an imported link is not an edit");
            let reopened = open(&save(&h.state().session.doc));
            let content = crate::comment_links::CommentText::new(&reopened, reopened.comments[&id].part);
            let job = content.job(regular(12.0), Color32::BLACK, Color32::BLUE, 300.0);
            assert!(
                job.sections
                    .iter()
                    .any(|s| s.format.underline.width > 0.0 && job.text.get(s.byte_range.start.0..s.byte_range.end.0) == Some("Project website")),
                "{ext} keeps the linked label"
            );
        }
    }
}

/// Locate an actual painted glyph, including wrapped and bidirectional rows. The pane's
/// copy is the rightmost one when the balloon also shows the same text.
fn glyph_at(h: &egui_kittest::Harness<'_, WordApp>, text: &str, needle: &str) -> Pos2 {
    let index = text[..text.find(needle).unwrap()].chars().count();
    h.output()
        .shapes
        .iter()
        .filter_map(|shape| {
            let egui::Shape::Text(shape_text) = &shape.shape else { return None };
            if shape_text.galley.text() != text {
                return None;
            }
            let mut n = index;
            for row in &shape_text.galley.rows {
                if let Some(glyph) = row.glyphs.get(n) {
                    let at = shape_text.pos + row.pos.to_vec2() + glyph.logical_rect().center().to_vec2();
                    return shape.clip_rect.contains(at).then_some(at);
                }
                n = n.checked_sub(row.char_count_including_newline().0)?;
            }
            None
        })
        .max_by(|a, b| a.x.total_cmp(&b.x))
        .expect("the requested comment glyph is visible")
}

fn click(h: &mut egui_kittest::Harness<'_, WordApp>, at: Pos2, modifiers: egui::Modifiers) -> Vec<String> {
    let mut urls = Vec::new();
    h.event(egui::Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::ModifiersChanged(modifiers));
        h.step();
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers });
        h.step();
        urls.extend(h.output().platform_output.commands.iter().filter_map(|c| match c {
            egui::OutputCommand::OpenUrl(url) => Some(url.url.clone()),
            _ => None,
        }));
    }
    urls
}

/// #6693: both comment surfaces, including an unselected margin card, open web and mail
/// links through the same modifier-click used by document hyperlinks.
#[test]
fn comment_links_open_in_the_pane_and_both_balloon_states() {
    let text = "این صفحه: https://example.com/path?q=1#top\nEmail mailto:review@example.com";
    for (pane, active, protected) in [(true, false, false), (false, false, false), (false, true, false), (true, false, true), (false, true, true)] {
        let (mut h, _) = harness(text, pane, active);
        if protected {
            h.state_mut().session.doc.settings.protection = Some("readOnly".into());
            steps(&mut h, 2);
        }
        let before = h.state().session.doc.clone();
        for target in ["https://example.com/path?q=1#top", "mailto:review@example.com"] {
            let at = glyph_at(&h, text, target);
            assert_eq!(click(&mut h, at, egui::Modifiers::COMMAND), [target], "pane={pane}, active={active}, protected={protected}");
            assert_eq!(h.state().session.doc, before, "following a comment link never edits the document");
        }
        if protected {
            h.event(egui::Event::Text("Refused edit".into()));
            steps(&mut h, 2);
            assert!(!crate::panes::has_comment_drafts(h.state()));
            assert_eq!(h.state().session.doc, before);
        }
    }
}
