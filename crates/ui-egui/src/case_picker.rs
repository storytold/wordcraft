//! The Change Case picker: one list for Shift+F3, the ribbon, the mini toolbar and the
//! right-click menu. The document changes only when a row is chosen, so "Original" is the text
//! as it was when the picker opened.

use egui::{Id, Key, Modifiers, Order, Pos2, Rect, vec2};
use serde_json::json;
use wordcraft_engine::cmd::format::{CaseMode, next_mode_at_selection};

use crate::WordApp;

const ROWS: usize = 1 + 5;
const WIDTH: f32 = 216.0;
const ESTIMATED_H: f32 = 224.0;
const GAP: f32 = 6.0;

/// The open picker, kept on the canvas beside the other transient page state.
pub struct Picker {
    /// 0 is Original; 1..=5 follows [`CaseMode::ALL`].
    selected: usize,
    opened: wordcraft_engine::Selection,
    document: u64,
    /// Skip the outside-click test on the frame the picker is first drawn.
    opened_frame: Option<u64>,
}

impl Picker {
    fn new(app: &WordApp) -> Self {
        let next = next_mode_at_selection(&app.session);
        let selected = 1 + CaseMode::ALL.iter().position(|m| *m == next).unwrap_or(0);
        Picker { selected, opened: app.session.sel.clone(), document: app.session.document_id(), opened_frame: None }
    }
}

/// Whether the picker is open.
pub fn is_open(app: &WordApp) -> bool {
    app.canvas.case_picker.is_some()
}

/// Open the picker, or move to the next row when it is already open.
pub fn open(app: &mut WordApp) {
    if app.canvas.case_picker.is_some() {
        next(app);
        return;
    }
    if !app.has_document() || app.session.view.read_mode || !app.dialog_host.is_empty() || app.ui.backstage || app.keytips_active {
        return;
    }
    app.canvas.case_picker = Some(Picker::new(app));
    if let Some(ctx) = app.ctx.clone() {
        app.canvas.focus_page(&ctx);
    } else {
        app.canvas.focus_pending = true;
    }
}

fn next(app: &mut WordApp) {
    if let Some(p) = app.canvas.case_picker.as_mut() {
        p.selected = (p.selected + 1) % ROWS;
        if let Some(ctx) = app.ctx.clone() {
            app.canvas.focus_page(&ctx);
        } else {
            app.canvas.focus_pending = true;
        }
    }
}

fn close(app: &mut WordApp) {
    app.canvas.case_picker = None;
    if let Some(ctx) = app.ctx.clone() {
        app.canvas.focus_page(&ctx);
    } else {
        app.canvas.focus_pending = true;
    }
}

fn apply(app: &mut WordApp, row: usize) {
    let mode = row.checked_sub(1).and_then(|i| CaseMode::ALL.get(i).copied());
    close(app);
    if let Some(mode) = mode {
        let _ = app.run("format.changeCase", json!({"mode": mode.as_str()}));
    }
}

/// What a key does while the picker is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Up,
    Down,
    Home,
    End,
    Apply,
    Cancel,
    Next,
}

fn action(key: Key, m: Modifiers) -> Option<Action> {
    if is_shift_f3(key, m) {
        return Some(Action::Next);
    }
    // Command/Alt keys belong to the app. Shift may still be down after Shift+F3.
    if m.command || m.ctrl || m.alt || m.mac_cmd {
        return None;
    }
    Some(match key {
        Key::ArrowUp => Action::Up,
        Key::ArrowDown => Action::Down,
        Key::Home => Action::Home,
        Key::End => Action::End,
        Key::Enter => Action::Apply,
        Key::Escape => Action::Cancel,
        _ => return None,
    })
}

/// Handle the picker's keys before the canvas sees them. Keys the picker doesn't own close it
/// and continue to the document, so typing and Shift+Arrow behave normally.
fn is_shift_f3(key: Key, m: Modifiers) -> bool {
    key == Key::F3 && m.shift && !m.command && !m.ctrl && !m.alt && !m.mac_cmd
}

pub fn keys(app: &mut WordApp, ctx: &egui::Context) {
    if !is_open(app) {
        // Shift+F3 opens the picker in the UI (#3483). Consume the key so the no-mode
        // engine command does not also cycle the text.
        let pressed = ctx
            .input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, modifiers, .. } if is_shift_f3(*key, *modifiers))));
        if pressed {
            open(app);
            ctx.input_mut(|i| {
                i.events.retain(|e| !matches!(e, egui::Event::Key { key, pressed: true, modifiers, .. } if is_shift_f3(*key, *modifiers)));
            });
        }
        return;
    }
    // A dialog, the Backstage, Read Mode or another document invalidates the picker.
    if !app.has_document()
        || app.session.view.read_mode
        || !app.dialog_host.is_empty()
        || app.ui.backstage
        || app.session.document_id() != app.canvas.case_picker.as_ref().map(|p| p.document).unwrap_or_default()
        || app.session.sel != app.canvas.case_picker.as_ref().map(|p| p.opened.clone()).unwrap_or_default()
    {
        close(app);
        return;
    }

    let mut events = ctx.input(|i| i.events.clone());
    let mut keep = Vec::with_capacity(events.len());
    let mut chosen = None;
    let mut cancel = false;
    for event in events.drain(..) {
        let egui::Event::Key { key, pressed: true, modifiers, .. } = &event else {
            match &event {
                egui::Event::Text(_) | egui::Event::Ime(_) => {
                    // Typing closes the picker and goes to the document.
                    close(app);
                }
                egui::Event::WindowFocused(false) => close(app),
                _ => {}
            }
            keep.push(event);
            continue;
        };
        match action(*key, *modifiers) {
            Some(Action::Up) => {
                if let Some(p) = app.canvas.case_picker.as_mut() {
                    p.selected = if p.selected == 0 { ROWS - 1 } else { p.selected - 1 };
                }
            }
            Some(Action::Down) | Some(Action::Next) => next(app),
            Some(Action::Home) => {
                if let Some(p) = app.canvas.case_picker.as_mut() {
                    p.selected = 0;
                }
            }
            Some(Action::End) => {
                if let Some(p) = app.canvas.case_picker.as_mut() {
                    p.selected = ROWS - 1;
                }
            }
            Some(Action::Apply) => {
                chosen = app.canvas.case_picker.as_ref().map(|p| p.selected);
            }
            Some(Action::Cancel) => cancel = true,
            None => {
                // Another key closes the picker and is handled normally (Shift+Arrow included).
                if !matches!(
                    *key,
                    Key::ShiftLeft
                        | Key::ShiftRight
                        | Key::ControlLeft
                        | Key::ControlRight
                        | Key::AltLeft
                        | Key::AltRight
                        | Key::SuperLeft
                        | Key::SuperRight
                ) {
                    close(app);
                }
                keep.push(event);
            }
        }
    }
    ctx.input_mut(|i| i.events = keep);
    if let Some(row) = chosen {
        apply(app, row);
    } else if cancel {
        close(app);
    }
}

/// Remove the picker's keys from the frame's raw input before the runtime and the canvas see them,
/// so Enter and Shift+F3 do not also run a document command (#3483).
pub fn filter_raw(app: &mut WordApp, raw: &mut egui::RawInput) {
    let mut chosen = None;
    let mut cancel = false;
    raw.events.retain(|event| {
        let egui::Event::Key { key, pressed: true, modifiers, .. } = event else { return true };
        if !is_open(app) {
            if is_shift_f3(*key, *modifiers) {
                open(app);
                return false;
            }
            return true;
        }
        match action(*key, *modifiers) {
            Some(Action::Up) => {
                if let Some(p) = app.canvas.case_picker.as_mut() {
                    p.selected = if p.selected == 0 { ROWS - 1 } else { p.selected - 1 };
                }
                false
            }
            Some(Action::Down) | Some(Action::Next) => {
                next(app);
                false
            }
            Some(Action::Home) => {
                if let Some(p) = app.canvas.case_picker.as_mut() {
                    p.selected = 0;
                }
                false
            }
            Some(Action::End) => {
                if let Some(p) = app.canvas.case_picker.as_mut() {
                    p.selected = ROWS - 1;
                }
                false
            }
            Some(Action::Apply) => {
                chosen = app.canvas.case_picker.as_ref().map(|p| p.selected);
                false
            }
            Some(Action::Cancel) => {
                cancel = true;
                false
            }
            None => true,
        }
    });
    if let Some(row) = chosen {
        apply(app, row);
    } else if cancel {
        close(app);
    }
}

/// Draw the picker above the page. Called after the canvas so its Foreground layer sits over the
/// ribbon and the mini toolbar.
pub fn show(app: &mut WordApp, ctx: &egui::Context) {
    let Some((selected, opened, document, opened_frame)) =
        app.canvas.case_picker.as_ref().map(|p| (p.selected, p.opened.clone(), p.document, p.opened_frame))
    else {
        return;
    };
    if !app.has_document()
        || app.session.view.read_mode
        || !app.dialog_host.is_empty()
        || app.ui.backstage
        || app.session.document_id() != document
        || app.session.sel != opened
    {
        close(app);
        return;
    }

    let anchor = if let Some((at, line_h)) = crate::canvas::caret_screen(app) {
        Some((at, line_h))
    } else {
        app.canvas.mini_anchor.map(|r| (r.left_bottom(), 0.0))
    };
    let Some((at, line_h)) = anchor else {
        close(app);
        return;
    };
    let viewport = ctx.content_rect();
    let pos = place(at, line_h, viewport);

    let frame = ctx.cumulative_frame_nr();
    let first_draw = opened_frame.is_none();
    if first_draw && let Some(p) = app.canvas.case_picker.as_mut() {
        p.opened_frame = Some(frame);
    }

    let mut chosen = None;
    let shown = egui::Area::new(Id::unique("wc_change_case")).order(Order::Foreground).fixed_pos(pos).constrain_to(viewport).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_min_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(egui::RichText::new(tl!("Change Case")).strong());
            ui.separator();
            for row in 0..ROWS {
                let label = if row == 0 { tl!("Original") } else { tl!(CaseMode::ALL[row - 1].label()) };
                if ui.selectable_label(selected == row, label).clicked() {
                    chosen = Some(row);
                }
                if row == 0 {
                    ui.separator();
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("↑↓");
                ui.label(tl!("Enter"));
                ui.label(tl!("Apply"));
                ui.label("Esc");
                ui.label(tl!("Cancel"));
            });
        });
    });

    if let Some(row) = chosen {
        apply(app, row);
        return;
    }
    if !first_draw
        && ctx.input(|i| i.pointer.any_pressed())
        && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| !shown.response.rect.contains(p))
    {
        close(app);
    }
}

/// Put the picker below the caret, above it when the bottom is tight, and inside the viewport.
fn place(at: Pos2, line_h: f32, viewport: Rect) -> Pos2 {
    let mut pos = at + vec2(0.0, line_h + GAP);
    if pos.y + ESTIMATED_H > viewport.bottom() && at.y - ESTIMATED_H - GAP >= viewport.top() {
        pos.y = at.y - ESTIMATED_H - GAP;
    }
    pos.x = pos.x.clamp(viewport.left() + 4.0, (viewport.right() - WIDTH - 4.0).max(viewport.left() + 4.0));
    pos.y = pos.y.clamp(viewport.top() + 4.0, (viewport.bottom() - ESTIMATED_H - 4.0).max(viewport.top() + 4.0));
    pos
}

#[cfg(test)]
mod tests {
    use super::{Action, ROWS, action, place};
    use egui::{Key, Modifiers, Rect, pos2, vec2};

    #[test]
    fn picker_keys_leave_shift_arrows_alone() {
        assert_eq!(action(Key::F3, Modifiers::SHIFT), Some(Action::Next));
        assert_eq!(action(Key::ArrowDown, Modifiers::NONE), Some(Action::Down));
        assert_eq!(action(Key::ArrowUp, Modifiers::NONE), Some(Action::Up));
        assert_eq!(action(Key::Enter, Modifiers::NONE), Some(Action::Apply));
        assert_eq!(action(Key::Escape, Modifiers::NONE), Some(Action::Cancel));
        assert_eq!(action(Key::ArrowDown, Modifiers::SHIFT), None);
        assert_eq!(action(Key::F3, Modifiers::CTRL | Modifiers::SHIFT), None);
    }

    #[test]
    fn picker_rows_include_original_and_five_modes() {
        assert_eq!(ROWS, 6);
    }

    #[test]
    fn picker_flips_above_near_the_bottom() {
        let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let below = place(pos2(100.0, 100.0), 10.0, viewport);
        assert!(below.y > 100.0);
        let above = place(pos2(100.0, 560.0), 10.0, viewport);
        assert!(above.y < 560.0);
        assert!(viewport.contains_rect(Rect::from_min_size(above, vec2(super::WIDTH, super::ESTIMATED_H))));
    }
}
