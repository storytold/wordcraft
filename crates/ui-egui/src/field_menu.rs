//! The right-click menu of a dialog's text box (#4404): Cut, Copy, Paste and Select All, as the
//! system's own text boxes in Word's dialogs have. egui's text fields have no menu of their own,
//! so a right-click in Find what did nothing and Paste was keyboard-only.
//!
//! Cut, Copy and Paste reach the field as egui's own clipboard events on the next frame, once it
//! has the focus back from the menu: the selection, a single-line field's handling of pasted line
//! breaks, a character limit and undo then behave exactly as with Ctrl+X, Ctrl+C and Ctrl+V.

use egui::text::{CCursor, CCursorRange};
use egui::{Event, Id, PointerButton, Response, TextEdit, Ui};

/// Where a field keeps the selection it had before a right-click.
fn kept_id(id: Id) -> Id {
    id.with("field_menu_kept")
}

/// Where the menu leaves its pick for the next frame.
fn pending_id(id: Id) -> Id {
    id.with("field_menu_pending")
}

/// Add `edit` under `id`, with the menu. `clipboard` reads the system clipboard's text for Paste
/// ([`crate::Services::clipboard_text`]); without it (the web, where only Ctrl+V can read the
/// clipboard) Paste is greyed out.
pub fn text_field(ui: &mut Ui, edit: TextEdit<'_>, id: Id, clipboard: Option<&dyn Fn() -> Option<String>>) -> Response {
    // Last frame's pick, now that the field has the focus again. Only last frame's: a pick the
    // field wasn't drawn for (its tab left in between) is dropped, not replayed later.
    let frame = ui.ctx().cumulative_frame_nr();
    if let Some((event, picked)) = ui.data_mut(|d| d.remove_temp::<Option<(Event, u64)>>(pending_id(id))).flatten()
        && picked.saturating_add(1) == frame
        && ui.memory(|m| m.has_focus(id))
    {
        ui.input_mut(|i| i.events.push(event));
    }
    let mut out = edit.id(id).show(ui);
    let resp = out.response.response.clone();
    // egui moves the caret to a press of any button, so a right-click would lose the selection
    // the menu is for. As on Word's page, a right-click in the selection keeps it and one
    // elsewhere moves the caret there: put that back while the button is down and on its
    // release, which is when the menu opens.
    let right =
        resp.hovered() && ui.input(|i| i.pointer.button_down(PointerButton::Secondary) || i.pointer.button_released(PointerButton::Secondary));
    let mut kept = ui.data(|d| d.get_temp::<CCursorRange>(kept_id(id)));
    if right && ui.input(|i| i.pointer.button_pressed(PointerButton::Secondary)) {
        // On the press egui has just put the caret at the click.
        let at = out.state.cursor.char_range().map(|r| r.primary.index);
        let in_selection = kept.zip(at).is_some_and(|(k, at)| {
            let (start, end) = (k.primary.index.min(k.secondary.index), k.primary.index.max(k.secondary.index));
            start < end && (start..=end).contains(&at)
        });
        if !in_selection {
            kept = out.state.cursor.char_range();
        }
    }
    if right {
        if let Some(kept) = kept {
            out.state.cursor.set_char_range(Some(kept));
            out.state.clone().store(ui.ctx(), id);
            ui.data_mut(|d| d.insert_temp(kept_id(id), kept));
        }
    } else if let Some(range) = out.state.cursor.char_range() {
        ui.data_mut(|d| d.insert_temp(kept_id(id), range));
    }
    let selected = out.state.cursor.char_range().is_some_and(|r| !r.is_empty());
    let len = out.galley.text().chars().count();
    resp.context_menu(|ui| {
        let mut pick = None;
        if ui.add_enabled(selected, egui::Button::new(tl!("Cut"))).clicked() {
            pick = Some(Event::Cut);
        }
        if ui.add_enabled(selected, egui::Button::new(tl!("Copy"))).clicked() {
            pick = Some(Event::Copy);
        }
        if ui.add_enabled(clipboard.is_some(), egui::Button::new(tl!("Paste"))).clicked() {
            // An empty clipboard pastes nothing; the menu still closes.
            pick = clipboard.and_then(|f| f()).filter(|t| !t.is_empty()).map(Event::Paste);
            ui.close();
        }
        ui.separator();
        if ui.button(tl!("Select All")).clicked() {
            let all = CCursorRange::two(CCursor::new(0), CCursor::new(len));
            out.state.cursor.set_char_range(Some(all));
            out.state.clone().store(ui.ctx(), id);
            ui.data_mut(|d| d.insert_temp(kept_id(id), all));
            ui.memory_mut(|m| m.request_focus(id));
            ui.close();
        }
        if let Some(event) = pick {
            ui.data_mut(|d| d.insert_temp(pending_id(id), Some((event, frame))));
            ui.memory_mut(|m| m.request_focus(id));
            ui.close();
        }
    });
    resp
}

#[cfg(test)]
mod tests;
