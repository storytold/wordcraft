use super::{Event, Id, TextEdit, Ui, pending_id, text_field};

/// #4404 review: a menu pick reaches the field only on the frame after it. One the field wasn't
/// drawn for then (its tab left in between) is dropped, not replayed when the field comes back.
#[test]
fn a_pick_the_field_missed_is_dropped() {
    let id = Id::unique("field_menu_test");
    let field = |ui: &mut Ui, text: &mut String| {
        let label = ui.label("Find what:");
        text_field(ui, TextEdit::singleline(text), id, None).labelled_by(label.id);
    };
    let mut h = egui_kittest::Harness::builder().build_ui_state(field, String::from("old"));
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.run_steps(2);
    assert!(h.ctx.memory(|m| m.has_focus(id)), "the field has the keyboard, so only the frame check drops the pick");
    let long_ago = h.ctx.cumulative_frame_nr().saturating_sub(5);
    h.ctx.data_mut(|d| d.insert_temp(pending_id(id), Some((Event::Paste("x".into()), long_ago))));
    h.run_steps(2);
    assert_eq!(h.state(), "old");
    assert!(h.ctx.data(|d| d.get_temp::<Option<(Event, u64)>>(pending_id(id))).flatten().is_none(), "and it is gone");
}
