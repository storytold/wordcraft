//! The adapter that turns the edit events of a field gesture into undo steps of the session.

use craft::behavior::{EditEvents, Value};
use craft::ui::edit::{StepName, UndoMarks};

use crate::WordApp;

impl UndoMarks for WordApp {
    type Mark = u64;

    fn undo_mark(&self) -> u64 {
        self.session.undo_mark()
    }

    fn revert_to(&mut self, mark: u64) {
        self.session.revert_since(mark);
    }

    /// The folded step keeps the first step's own command label: history labels are English command names that the
    /// chrome translates, so the localized `StepName` is not stored.
    fn squash_since(&mut self, mark: u64, _name: &StepName) {
        self.session.merge_undo_since(mark);
    }
}

/// Feeds a field's events to the window's gesture host: a gesture is one undo step, a cancelled one leaves none.
/// `apply` makes the edit for a proposed value.
pub(crate) fn apply_edits<T: Value>(app: &mut WordApp, events: EditEvents<T>, apply: impl Fn(&mut WordApp, T)) {
    // `handle` takes the document by `&mut`, so the host leaves its field for the call.
    // PLAN: assumed `WordApp.edits: GestureHost<WordApp>` reachable as `app.edits` (app-hub, H-1)
    let mut host = std::mem::take(&mut app.edits);
    for event in events {
        host.handle(app, event, &apply);
    }
    app.edits = host;
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::behavior::{CancelReason, EditEvent, GestureId, Mixed};
    use serde_json::json;
    use wordcraft_engine::Session;

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default())
    }

    fn set_margin(app: &mut WordApp, v: Option<f64>) {
        if let Some(v) = v {
            let _ = app.run("layout.margins", json!({"left": v}));
        }
    }

    fn begin(id: GestureId) -> EditEvents<Option<f64>> {
        EditEvents::from(EditEvent::begin(id, Mixed::Same(Some(72.0)), "Margin".into(), None))
    }

    #[test]
    fn a_cancelled_field_gesture_leaves_no_step_and_no_redo() {
        let mut a = app();
        a.run("layout.margins", json!({"left": 80.0})).unwrap();
        a.run("layout.margins", json!({"left": 90.0})).unwrap();
        let (depth, margin) = (a.session.undo_depth(), a.session.doc.last_section.margin_left);

        let id = GestureId::new(1);
        apply_edits(&mut a, begin(id), set_margin);
        apply_edits(&mut a, EditEvents::from(EditEvent::update(id, Some(100.0))), set_margin);
        assert_eq!(a.session.doc.last_section.margin_left, 100.0, "the update is applied as it goes");
        apply_edits(&mut a, EditEvents::from(EditEvent::cancel(id, CancelReason::Escape, None)), set_margin);

        assert_eq!(a.session.undo_depth(), depth, "no step left, none of the earlier ones undone");
        assert_eq!(a.session.doc.last_section.margin_left, margin);
        assert!(!a.session.can_redo());
    }

    #[test]
    fn a_committed_field_gesture_is_one_step() {
        let mut a = app();
        a.run("layout.margins", json!({"left": 80.0})).unwrap();
        let depth = a.session.undo_depth();

        let id = GestureId::new(1);
        apply_edits(&mut a, begin(id), set_margin);
        apply_edits(&mut a, EditEvents::from(EditEvent::update(id, Some(100.0))), set_margin);
        apply_edits(&mut a, EditEvents::from(EditEvent::update(id, Some(110.0))), set_margin);
        apply_edits(&mut a, EditEvents::from(EditEvent::commit(id, Mixed::Same(Some(72.0)), Some(120.0))), set_margin);

        assert_eq!(a.session.undo_depth(), depth + 1);
        assert_eq!(a.session.doc.last_section.margin_left, 120.0);
    }
}
