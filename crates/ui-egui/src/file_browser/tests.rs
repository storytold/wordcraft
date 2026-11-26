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
