use std::sync::Arc;

use craft::browse::places::{FakeDirectories, FolderCache};
use craft::dialogs::ui::FileChooser;
use craft::host::{ManualJobs, PickRequest, Wake};
use egui::vec2;
use serde_json::json;
use wordcraft_engine::Session;

use crate::{Services, WordApp};

/// #3859: a modal dialog that closes as it opens the browser (Save in "Save changes?" as a new
/// document closes, with no system dialog) leaves the keyboard in the browser's name box: the
/// document doesn't take it back on the next frame.
#[test]
fn a_dialog_closing_into_the_browser_leaves_it_the_keyboard() {
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    a.run("text.insert", json!({"text": "Unsaved"})).unwrap();
    let _ = a.run("file.close", json!({}));
    assert!(a.dialog_host.shows_modal(), "Save Changes prompt opens");

    let mut h = crate::test_harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            crate::test_harness::logic(&ctx, app);
            app.ui(ui);
        },
        a,
    );
    crate::test_harness::frames(&mut h, 4);

    let _ = h.state_mut().run("ui.saveChanges", json!({"answer": "save"}));
    let folders = FolderCache::new(Arc::new(FakeDirectories::new().folder("/d/drafts")), Arc::new(ManualJobs::new()), Wake::none());
    let req = PickRequest::save("document").file_name("Unsaved.docx").fallback(true);
    h.state_mut().dialog_host.open(FileChooser::new(req, None, folders));
    crate::test_harness::frames(&mut h, 4);

    assert!(h.state().dialog_host.is_open::<FileChooser>(), "Save As goes on in the browser");
    assert!(!h.state().canvas.focused(&h.ctx), "document does not take the keyboard back");

    let before_doc = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    h.event(egui::Event::Text("x".into()));
    crate::test_harness::frames(&mut h, 2);

    let after_doc = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert_eq!(after_doc, before_doc, "typing does not leak into the document behind the browser");
}

/// #3859: no keytips behind the browser.
#[test]
fn no_keytips_behind_the_browser() {
    let a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    let mut h = crate::test_harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            crate::test_harness::logic(&ctx, app);
            app.ui(ui);
        },
        a,
    );
    crate::test_harness::frames(&mut h, 4);
    let ctx = h.ctx.clone();
    h.state_mut().canvas.focus_page(&ctx);
    crate::test_harness::frames(&mut h, 2);

    let keytip_badges = |h: &egui_kittest::Harness<'_, WordApp>| -> Vec<String> {
        const TAB_BADGES: [&str; 11] = ["F", "H", "N", "D", "G", "P", "S", "M", "R", "W", "Y"];
        h.output()
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if TAB_BADGES.contains(&t.galley.text()) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect()
    };

    assert!(!h.state().keytips_active, "keytips start off");
    assert!(keytip_badges(&h).is_empty(), "no keytip badges start painted: {:?}", keytip_badges(&h));

    let folders = FolderCache::new(Arc::new(FakeDirectories::new().folder("/d/drafts")), Arc::new(ManualJobs::new()), Wake::none());
    let req = PickRequest::open("document").directory("/d").fallback(true);
    h.state_mut().dialog_host.open(FileChooser::new(req, None, folders));
    crate::test_harness::frames(&mut h, 4);
    assert!(h.state().dialog_host.is_open::<FileChooser>(), "the browser is open");

    h.key_press(egui::Key::F10);
    crate::test_harness::frames(&mut h, 2);
    assert!(!h.state().keytips_active, "F10 does not activate keytips while the browser is open");
    assert!(keytip_badges(&h).is_empty(), "no keytip badges are painted behind the browser: {:?}", keytip_badges(&h));

    // Close the browser and check F10 works as a control.
    h.key_press(egui::Key::Escape);
    crate::test_harness::frames(&mut h, 4);
    assert!(!h.state().dialog_host.is_open::<FileChooser>(), "Escape closes the browser");
    assert!(!h.state().modal_open(), "no modal remains open after closing the browser");

    h.key_press(egui::Key::F10);
    crate::test_harness::frames(&mut h, 2);
    assert!(h.state().keytips_active, "F10 activates keytips once the browser is closed");
    assert!(!keytip_badges(&h).is_empty(), "keytip badges paint once the browser is closed");
}
