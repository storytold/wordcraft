use super::alt_code::{alt_code_echoed, alt_code_event, alt_codes_supported};
use super::ime::{
    MAX_IME_COMMIT_BYTES, MAX_IME_PREEDIT_BYTES, delete_in_equation, delete_surrounding, delete_surrounding_bytes, ime_stamp, note_automatic_change,
    note_ime_application_change, publish_ime_receipt,
};
use super::shortcuts::{
    EquationReplacementDrop, dispatch, equation_key, is_context_menu_key, is_modifier_key, is_paste_release, is_paste_special_release,
    is_shift_delete, paste_release_modifiers,
};
use super::{Key, Modifiers, WordApp, json};

/// Events for the focused canvas: text, editing keys, clipboard, IME. While keytips are showing
/// every key belongs to them (letters pick badges, Escape cancels), so nothing here runs.
pub fn canvas_events(app: &mut WordApp, ctx: &egui::Context) {
    if app.ui.keytips != crate::keytips::Phase::Off {
        return;
    }
    let events = ctx.input(|i| i.events.clone());
    let window = app.windows.shown();
    if app.session.view.read_mode {
        read_mode_events(app, ctx, &events, window);
        return;
    }
    let alt_codes = alt_codes_supported(ctx);
    // The erased text's formatting is carried only for a replacement in this raw batch. The
    // undo join belongs to the deletion transaction instead: the engine's scoped
    // `ime_text_join` survives frames until its matching `text.insert` or any other command,
    // key or new request consumes or clears it, so a platform that delivers the `Commit` in a
    // later batch still shares the deletion's Undo checkpoint.
    app.canvas.ime_join_text = None;
    // A refused byte-counted equation deletion suppresses only the replacement immediately
    // paired with it. egui-winit pushes `DeleteSurroundingBytes` immediately before the
    // `Commit` from the same winit IME event, so one adjacent event is the replacement. Some
    // platforms route the same replacement through a `Text` followed by its `Commit`; that
    // adjacent pair is the only exception. Any other event is fresh input: clearing the state
    // lets a later key, pre-edit or direct commit in this same raw batch apply. This is local,
    // not canvas state: an untagged event in a later frame cannot be told apart from fresh
    // input, so it must never be dropped.
    // A body/equation replacement may have invalidated a live preedit before this batch (or
    // an earlier event can do it below). Its already queued IME replacement belongs to the old
    // target. This is separate from the outgoing interrupt, which is observed after mutation.
    // Keep the quarantine local to this batch: a later untagged commit can be fresh input.
    // Mod held since an earlier frame (its ModifiersChanged went by then).
    let mut batch = Batch {
        drop_equation_replacement: EquationReplacementDrop::None,
        interrupted_preedit: std::mem::take(&mut app.canvas.ime_interrupted_preedit),
        interrupted_replacement: EquationReplacementDrop::None,
    };
    let held = ctx.input(|i| i.modifiers);
    if held.command {
        app.canvas.paste_mods = Some(held);
    }
    for e in events {
        // A key closed the window (Mod+W) or brought another forward: the frame's other keys
        // were typed in this one and are dropped.
        if app.windows.shown() != window {
            break;
        }
        if stale_composition(app, &e, &mut batch) {
            continue;
        }
        note_key_state(app, &e);
        if drop_refused_replacement(app, &e, &mut batch)
            || prefilter(app, ctx, &e, alt_codes)
            || paste_release(app, &e)
            || equation_event(app, ctx, &e, &mut batch)
        {
            continue;
        }
        handle_event(app, ctx, e);
    }
}

/// State carried across the events of one raw batch.
struct Batch {
    drop_equation_replacement: EquationReplacementDrop,
    interrupted_preedit: bool,
    interrupted_replacement: EquationReplacementDrop,
}

/// Read Mode is for navigation and selection, not typing into the document.
fn read_mode_events(app: &mut WordApp, ctx: &egui::Context, events: &[egui::Event], window: u64) {
    // Read Mode is for navigation and selection, not typing into the document. The ribbon's
    // Edit Document button or Escape returns to Print Layout before editing.
    for e in events {
        if app.windows.shown() != window {
            break;
        }
        match e {
            // Not while a dialog or menu that takes Escape is open.
            egui::Event::Key { key: Key::Escape, pressed: true, .. } if app.escape_free => {
                // Focus or Full Screen first (#1678), then Read Mode.
                let id = crate::chrome::Screen::of(&app.session.view).exit_command().unwrap_or("view.printLayout");
                let _ = app.run(id, json!({"value": false}));
                break;
            }
            egui::Event::Key { key: Key::PageDown | Key::ArrowRight, pressed: true, modifiers, .. } if modifiers.is_none() => {
                crate::canvas::turn_read_page(app, 1.0)
            }
            egui::Event::Key { key: Key::PageUp | Key::ArrowLeft, pressed: true, modifiers, .. } if modifiers.is_none() => {
                crate::canvas::turn_read_page(app, -1.0)
            }
            egui::Event::Copy => {
                let selected = app.session.selected_text();
                if !selected.is_empty() {
                    ctx.copy_text(selected);
                }
            }
            _ => {}
        }
    }
}

/// An event that is the queued replacement for an interrupted pre-edit: dropped.
fn stale_composition(app: &mut WordApp, e: &egui::Event, b: &mut Batch) -> bool {
    let Batch { interrupted_preedit, interrupted_replacement, .. } = b;
    *interrupted_preedit |= std::mem::take(&mut app.canvas.ime_interrupted_preedit);
    let stale_composition_event = if *interrupted_preedit {
        match &e {
            egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { .. } | egui::ImeEvent::DeleteSurroundingBytes { .. }) => {
                *interrupted_replacement = EquationReplacementDrop::Awaiting;
                true
            }
            egui::Event::Ime(egui::ImeEvent::Commit(_)) => {
                *interrupted_preedit = false;
                *interrupted_replacement = EquationReplacementDrop::None;
                true
            }
            egui::Event::Text(_) if matches!(*interrupted_replacement, EquationReplacementDrop::Awaiting) => {
                *interrupted_replacement = EquationReplacementDrop::TextDropped;
                true
            }
            egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) if text.is_empty() => true,
            // A new marked composition or ordinary input is not the old target's queued
            // replacement. Commit-only input without an interrupted preedit is never gated.
            egui::Event::Ime(egui::ImeEvent::Preedit { .. })
            | egui::Event::Text(_)
            | egui::Event::Paste(_)
            | egui::Event::Key { pressed: true, .. } => {
                *interrupted_preedit = false;
                *interrupted_replacement = EquationReplacementDrop::None;
                false
            }
            _ => false,
        }
    } else {
        false
    };
    if stale_composition_event {
        app.session.clear_ime_joins();
        app.canvas.ime_join_text = None;
        return true;
    }
    false
}

/// Which presses reached the canvas, and the modifiers held with Mod, in event order.
fn note_key_state(app: &mut WordApp, e: &egui::Event) {
    // Which presses reached the canvas, and the modifiers held with Mod, in event order: Mod
    // can go down and up within one frame, leaving the frame's modifiers without it.
    match &e {
        egui::Event::ModifiersChanged(m) if m.command => app.canvas.paste_mods = Some(*m),
        egui::Event::Key { key: Key::V | Key::Paste, pressed: true, .. } => app.canvas.v_pressed = true,
        egui::Event::Key { key, pressed: true, modifiers, .. } if !is_modifier_key(*key) => {
            app.canvas.paste_mods = modifiers.command.then_some(*modifiers);
        }
        egui::Event::WindowFocused(_) => {
            app.canvas.v_pressed = false;
            app.canvas.paste_mods = None;
        }
        _ => {}
    }
}

/// The replacement paired with a refused equation deletion is dropped.
fn drop_refused_replacement(app: &mut WordApp, e: &egui::Event, b: &mut Batch) -> bool {
    // The replacement is the event immediately after the refused deletion: one `Commit`,
    // or the adjacent `Text` + `Commit` pair some platforms use. Any intervening event is
    // unrelated and clears the state before it is handled.
    let drop_event = match (b.drop_equation_replacement, e) {
        (EquationReplacementDrop::None, _) => false,
        (EquationReplacementDrop::Awaiting, egui::Event::Text(_)) => {
            b.drop_equation_replacement = EquationReplacementDrop::TextDropped;
            true
        }
        (EquationReplacementDrop::Awaiting, egui::Event::Ime(egui::ImeEvent::Commit(_))) => {
            b.drop_equation_replacement = EquationReplacementDrop::None;
            true
        }
        (EquationReplacementDrop::Awaiting, _) => {
            b.drop_equation_replacement = EquationReplacementDrop::None;
            false
        }
        (EquationReplacementDrop::TextDropped, egui::Event::Ime(egui::ImeEvent::Commit(_))) => {
            b.drop_equation_replacement = EquationReplacementDrop::None;
            true
        }
        (EquationReplacementDrop::TextDropped, _) => {
            b.drop_equation_replacement = EquationReplacementDrop::None;
            false
        }
    };
    if drop_event {
        app.canvas.ime_preedit.clear();
        app.canvas.ime_preedit_range = None;
        app.canvas.ime_interrupt_now = true;
        return true;
    }
    false
}

/// Alt shortcut text, Alt codes and pasted picture files take their events before typing.
fn prefilter(app: &mut WordApp, ctx: &egui::Context, e: &egui::Event, alt_codes: bool) -> bool {
    // An Alt shortcut (Alt+Shift+D, Alt+F7…) may also send its key's text; it isn't typed.
    match &e {
        egui::Event::Key { pressed: true, .. } => {
            app.canvas.swallow_text = false;
            app.canvas.alt_code_echo = None;
            app.canvas.ime_join_text = None;
            // A key between a deletion and its text means that text is fresh input, not the
            // replacement the deletion promised (even when the key runs no command).
            app.session.clear_ime_text_join();
        }
        egui::Event::Text(_) if std::mem::take(&mut app.canvas.swallow_text) => return true,
        egui::Event::Text(t) => {
            if let Some(echo) = app.canvas.alt_code_echo.take()
                && alt_code_echoed(app, &echo, t)
            {
                return true;
            }
        }
        // Focus elsewhere (another window) ends an Alt code unfinished.
        egui::Event::WindowFocused(false) => app.canvas.alt_code.clear(),
        _ => {}
    }
    if alt_codes && app.session.math.is_none() && alt_code_event(app, e) {
        return true;
    }
    if let egui::Event::Paste(t) = &e {
        app.canvas.pasted = true;
        // Copied picture files arrive as their paths: insert the pictures (#45).
        if app.session.math.is_none() && !ctx.input(|i| i.modifiers.alt) && crate::paste_picture::paste_named_files(app, t) {
            return true;
        }
    }
    // egui-winit 0.37 (outside Wayland) answers Mod+V on a clipboard with no text but a
    // picture with this event and sends no V release, so the release below never comes.
    // Paste through `edit.paste`, which reads the picture through the host like the ribbon's
    // Paste; the event's own pixels are left alone.
    if let egui::Event::PasteImage(_) = &e {
        // Pasted here: a V release, if a platform still sends one, mustn't paste again.
        app.canvas.pasted = true;
        if app.session.math.is_none() && app.services.clipboard_picture.is_some() {
            let _ = app.run("edit.paste", json!({}));
        }
        return true;
    }
    false
}

/// Mod+V with no clipboard text sends only the key's release: paste the picture then.
fn paste_release(app: &mut WordApp, e: &egui::Event) -> bool {
    // Desktop: Mod+V with no text on the clipboard sends no paste event, only the key's
    // release; paste the clipboard's picture then (#45). A paste event before the release
    // means it was text. (The web reads pasted pictures itself.) Mod released before V still
    // pastes: a release with no V press seen was Mod+V.
    if let egui::Event::Key { key: key @ (Key::V | Key::Paste), pressed: false, modifiers, .. } = &e {
        let press_seen = std::mem::take(&mut app.canvas.v_pressed);
        let modifiers = &paste_release_modifiers(*modifiers, press_seen, app.canvas.paste_mods.take());
        let paste_keys = is_paste_release(*key, *modifiers);
        let pasted = std::mem::take(&mut app.canvas.pasted);
        if !pasted && paste_keys && app.session.math.is_none() && app.services.clipboard_picture.is_some() {
            let _ = app.run("edit.paste", json!({}));
        }
        // Mod+Alt+V with no text on the clipboard: Paste Special still opens, listing what
        // is there (a picture) or saying what it holds (#687, #833).
        if !pasted && is_paste_special_release(*key, *modifiers) && app.session.math.is_none() && app.services.clipboard_picture.is_some() {
            let _ = app.run("edit.pasteSpecial", json!({}));
        }
        return true;
    }
    false
}

/// Editing an equation: text and editing keys go into it.
fn equation_event(app: &mut WordApp, ctx: &egui::Context, e: &egui::Event, b: &mut Batch) -> bool {
    if app.session.math.is_some() {
        match &e {
            egui::Event::Text(t) => {
                let m = ctx.input(|i| i.modifiers);
                let is_cmd = if cfg!(target_os = "macos") { m.command } else { (m.command || m.ctrl) && !m.alt };
                if !is_cmd && !t.is_empty() && t.chars().all(|c| !c.is_control()) {
                    let _ = app.run("equation.type", json!({"text": t}));
                } else {
                    // Ignored text is not the replacement a deletion promised.
                    app.session.clear_ime_joins();
                }
                return true;
            }
            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                app.canvas.ime_preedit.clear();
                app.canvas.ime_preedit_range = None;
                if !text.is_empty() {
                    if text.len() > MAX_IME_COMMIT_BYTES || text.chars().count() > wordcraft_engine::MAX_EQUATION_TYPE_CHARS {
                        // Refuse the whole commit (no prefix in either equation mode) and ask
                        // the platform to start the composition afresh. It is not the
                        // replacement for a preceding deletion.
                        log::warn!("ignoring an oversized IME equation commit of {} bytes", text.len());
                        app.session.clear_ime_joins();
                        app.canvas.ime_interrupt_now = true;
                        return true;
                    }
                    let before_stamp = ime_stamp(app);
                    match app.run_ime("equation.type", json!({"text": text})) {
                        Ok(_) => {
                            let outcome = app.session.ime_edit_outcome();
                            let after_stamp = ime_stamp(app);
                            publish_ime_receipt(app, before_stamp, after_stamp, outcome);
                        }
                        Err(_) => {
                            // The equation refused the commit (it may be gone): start the
                            // composition afresh instead of leaving one it believes landed.
                            app.canvas.ime_interrupt_now = true;
                        }
                    }
                } else {
                    // An empty commit is not the replacement for a preceding deletion.
                    app.session.clear_ime_joins();
                }
                return true;
            }
            // A phone keyboard's or macOS replacement inside an equation: erase the counted
            // units on each side with one transactional equation command. The tree cannot
            // provide a surrounding snapshot; an engine revision proves a real commit and the
            // platform is never handed fabricated surroundings.
            egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars, after_chars }) => {
                delete_in_equation(app, *before_chars, *after_chars);
                return true;
            }
            // Byte counts have no reversible mapping into a structured equation, so an
            // exact replacement cannot be performed without guessing at tree units. Refuse
            // the whole request, clear the composition and drop the replacement text that
            // the platform sends for the deletion it believes it made. Inserting it beside
            // the untouched equation would duplicate the old text.
            egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes, after_bytes }) => {
                if *before_bytes != 0 || *after_bytes != 0 {
                    log::debug!("refusing byte delete-surrounding inside an equation");
                    app.canvas.ime_preedit.clear();
                    app.canvas.ime_preedit_range = None;
                    // This refusal is a new request, not the replacement promised by an
                    // earlier equation deletion.
                    app.session.clear_ime_joins();
                    b.drop_equation_replacement = EquationReplacementDrop::Awaiting;
                    app.canvas.ime_interrupt_now = true;
                }
                return true;
            }
            egui::Event::Paste(t) => {
                if app.run("equation.type", json!({"text": t})).is_err() {
                    app.canvas.ime_interrupt_now = true;
                }
                return true;
            }
            egui::Event::Key { key, pressed: true, modifiers, .. } if equation_key(app, *key, *modifiers) => return true,
            // Anything else between the deletion and its replacement (a pre-edit, a key
            // release, focus, a pointer event) means the next equation typing command is a
            // new edit, not the immediate replacement. Drop the promise; a command consumes
            // it in `Session::run` even when it is not a typing command.
            _ => app.session.clear_ime_joins(),
        }
    }
    false
}

/// A committed IME string.
fn ime_commit(app: &mut WordApp, text: String) {
    app.canvas.ime_preedit.clear();
    app.canvas.ime_preedit_range = None;
    if !text.is_empty() {
        if text.len() <= MAX_IME_COMMIT_BYTES {
            match crate::dialogs_smart_art::type_into(app, &text) {
                crate::dialogs_smart_art::SmartArtTypeResult::Changed => {
                    app.session.clear_ime_text_join();
                    app.canvas.ime_join_text = None;
                    note_ime_application_change(app);
                }
                crate::dialogs_smart_art::SmartArtTypeResult::ConsumedNoChange => {
                    app.session.clear_ime_text_join();
                    app.canvas.ime_join_text = None;
                }
                crate::dialogs_smart_art::SmartArtTypeResult::NotHandled => {
                    type_text(app, &text, true);
                }
            }
        } else {
            log::warn!("ignoring an IME commit of {} bytes", text.len());
            app.session.clear_ime_text_join();
            app.canvas.ime_join_text = None;
            app.canvas.ime_interrupt_now = true;
        }
    } else {
        // An empty commit is not the replacement a preceding deletion promised.
        app.session.clear_ime_text_join();
        app.canvas.ime_join_text = None;
    }
}

/// A key press the canvas handles (shortcut, ink, objects, clipboard keys).
fn key_press(app: &mut WordApp, ctx: &egui::Context, key: Key, modifiers: Modifiers) {
    if key == Key::Escape && app.session.painter.is_some() {
        app.session.painter = None;
        return;
    }
    // The lasso selection and Ink Replay take Delete and Escape first.
    if crate::ink::key(app, key) {
        return;
    }
    // Escape puts the pen or eraser down (back to selecting).
    if key == Key::Escape && app.session.view.draw.mode != wordcraft_engine::cmd::draw::DrawMode::Select {
        app.canvas.ink = None;
        let _ = app.run("draw.select", json!({}));
        return;
    }
    if key == Key::Escape && crate::table_pen::stop(app) {
        return;
    }
    // File › Options › Advanced: the Insert key switches overtype only when asked to.
    if key == Key::Insert && modifiers.is_none() {
        if app.session.prefs.insert_key_overtype {
            let _ = app.run("text.overtype", json!({}));
        }
        return;
    }
    if crate::objects::key(app, key, modifiers) {
        return;
    }
    // Shift+F10, Windows' keyboard way to the context menu (#821).
    if is_context_menu_key(key, modifiers) {
        app.canvas.context_menu_from_keys = true;
        return;
    }
    // Escape in a text box (or WordArt) selects the box itself, so Delete then removes
    // it (#1538). The anchor position is just after the box's object character.
    if key == Key::Escape
        && crate::canvas::in_text_box(app)
        && let wordcraft_doc::StoryRef::Part(id) = app.session.sel.focus.story
        && let Some(end) = app.session.doc.text_box_anchor(id)
        && let Some(off) = end.off.checked_sub(wordcraft_doc::para::OBJ.len_utf8())
    {
        let start = wordcraft_doc::Pos { off, ..end.clone() };
        let _ = app.run("select.range", json!({"anchor": start, "focus": end}));
        return;
    }
    if key == Key::Escape && matches!(app.session.sel.focus.story, wordcraft_doc::StoryRef::Part(_)) && app.session.sel.is_collapsed() {
        let _ = app.run("insert.closeHeader", json!({}));
        return;
    }
    if is_shift_delete(key, modifiers) {
        if !app.session.sel.is_collapsed()
            && let Ok(r) = app.run("edit.cut", json!({}))
        {
            crate::system_clipboard::put_result(app, ctx, &r);
        }
        return;
    }
    if leave_full_screen(app, ctx, key, modifiers, true) {
        return;
    }
    app.canvas.swallow_text = dispatch(app, key, modifiers) && modifiers.alt;
}

/// Everything else: typed text, IME, clipboard and key presses.
fn handle_event(app: &mut WordApp, ctx: &egui::Context, e: egui::Event) {
    match e {
        egui::Event::Text(t) => {
            let m = ctx.input(|i| i.modifiers);
            let is_cmd = if cfg!(target_os = "macos") { m.command } else { (m.command || m.ctrl) && !m.alt };
            if is_cmd {
                return;
            }
            if t.chars().all(|c| !c.is_control()) && !t.is_empty() {
                match crate::dialogs_smart_art::type_into(app, &t) {
                    crate::dialogs_smart_art::SmartArtTypeResult::Changed => {
                        note_ime_application_change(app);
                        return;
                    }
                    crate::dialogs_smart_art::SmartArtTypeResult::ConsumedNoChange => return,
                    crate::dialogs_smart_art::SmartArtTypeResult::NotHandled => {}
                }
                // Typing with a text box selected types into it.
                crate::objects::enter_text_box(app);
                type_text(app, &t, false);
            }
        }
        egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars, after_chars }) => delete_surrounding(app, before_chars, after_chars),
        egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes, after_bytes }) => {
            delete_surrounding_bytes(app, before_bytes, after_bytes)
        }
        egui::Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars }) => {
            if text.len() <= MAX_IME_PREEDIT_BYTES {
                app.canvas.ime_preedit = text;
                app.canvas.ime_preedit_range = active_range_chars;
            } else {
                log::warn!("ignoring an IME pre-edit of {} bytes", text.len());
                app.canvas.ime_preedit.clear();
                app.canvas.ime_preedit_range = None;
            }
        }
        egui::Event::Ime(egui::ImeEvent::Commit(text)) => ime_commit(app, text),
        egui::Event::Paste(t) => {
            // Mod+Alt+V arrives as a paste too (the shell reads the clipboard): Paste Special.
            let m = ctx.input(|i| i.modifiers);
            let id = match (m.alt, m.shift) {
                (true, true) => "edit.pasteText",
                (true, false) => "edit.pasteSpecial",
                _ => "edit.paste",
            };
            let _ = app.run(id, json!({"text": t}));
        }
        egui::Event::Copy | egui::Event::Cut => {
            let id = if matches!(e, egui::Event::Cut) { "edit.cut" } else { "edit.copy" };
            // `run` puts the copied text on the system clipboard itself.
            let _ = app.run(id, json!({}));
        }
        egui::Event::Key { key, pressed: true, modifiers, .. } => key_press(app, ctx, key, modifiers),
        _ => {}
    }
}

/// Escape leaves Focus or Full Screen View (#1678), then Full Screen (#980), once nothing nearer
/// took it: a pen, a text box, a header, a dialog, a menu or a popup, a drag on the page, and in
/// the document (`canvas`) a selection, which Escape collapses first (`select.collapse`), as in a
/// normal window.
pub(super) fn leave_full_screen(app: &mut WordApp, ctx: &egui::Context, key: Key, m: Modifiers, canvas: bool) -> bool {
    let screen = crate::chrome::Screen::of(&app.session.view).exit_command();
    let Some(id) = screen.or(app.session.view.full_screen.then_some("view.fullScreen")) else { return false };
    if key != Key::Escape || !m.is_none() || !app.escape_free {
        return false;
    }
    // Escape with a button held (dragging a task pane's title bar, say) cancels the drag only.
    // egui has already dropped the drag itself, so the held button is what is left of it.
    if ctx.input(|i| i.pointer.any_down()) {
        return false;
    }
    let c = &app.canvas;
    if c.obj_drag.is_some() || c.table_resize.is_some() || c.table_drag.is_some() || c.link_from.is_some() {
        return false;
    }
    if canvas && !app.session.sel.is_collapsed() {
        return false;
    }
    let _ = app.run(id, json!({"value": false}));
    true
}

/// Typed text (a key's text or an input method's commit). Text replacing what an input method's
/// `DeleteSurrounding` just erased takes the erased text's formatting, never overtypes what
/// follows, and stays in the typing undo group (#1480).
///
/// `ime` says this text came from an accepted IME commit. Only then is a receipt taken, and only
/// around the insertion itself: a transformation the engine performs (AutoCorrect, AutoFormat)
/// makes the frame [`egui::ImeChangeCause::Other`] instead.
fn type_text(app: &mut WordApp, text: &str, ime: bool) {
    let before = ime.then(|| ime_stamp(app));
    // Where the text lands, so a quote can be checked against what the engine actually inserted.
    let start = app.session.sel.ordered().0;
    let pending_before = app.session.pending.clone();
    let typed = if let Some(props) = app.canvas.ime_join_text.take() {
        // The replacement takes the erased text's formatting. `Session::run` does not restore
        // `pending` on failure (it is not part of the document snapshot), so keep it here: a
        // refused replacement must not leave its formatting waiting for the next ordinary key.
        // The undo join itself is the engine's scoped body-replacement promise, consumed by
        // `Session::run` only for this `text.insert` at the promised selection.
        app.session.pending = Some(props);
        let overtype = std::mem::replace(&mut app.session.prefs.overtype, false);
        let typed =
            if ime { app.run_ime("text.insert", json!({"text": text})).is_ok() } else { app.run("text.insert", json!({"text": text})).is_ok() };
        app.session.prefs.overtype = overtype;
        if typed {
            app.session.continue_typing();
        } else {
            app.session.pending = pending_before;
            // The platform already applied its own deletion to the text it believed was there.
            // Its replacement was refused, so the surrounding snapshot is stale and the
            // deletion's join must not reach a later composition.
            app.session.clear_ime_text_join();
            app.canvas.ime_interrupt_now = true;
        }
        typed
    } else if ime {
        app.run_ime("text.insert", json!({"text": text})).is_ok()
    } else {
        app.run("text.insert", json!({"text": text})).is_ok()
    };
    if !typed {
        return;
    }
    let mut outcome = app.session.ime_edit_outcome();
    // The engine's own outcome is the per-command provenance: `text.insert` compares the paragraph
    // with the one it typed and marks AutoCorrect/list/dash rewrites `transformed`. A smart quote
    // is also checked against the actual inserted text, so smart quotes off stays the IME's own
    // edit. Never infer this from an undo label, which may belong to an earlier step. A refused
    // insert has no provenance at all: it must not fabricate an AutoFormat change or receipt.
    let quote = text == "\"" || text == "'";
    outcome.transformed |= quote && !inserted_literal_at(app, text, &start);
    note_automatic_change(app, outcome);
    if let Some(before) = before {
        let after = ime_stamp(app);
        publish_ime_receipt(app, before, after, outcome);
    }
}

/// Whether the text at `start` is exactly `text` (the engine inserted it unchanged).
///
/// A smart quote, an AutoCorrect replacement or a relocated insert makes this false: the edit is
/// AutoFormat work, not the input method's own change.
fn inserted_literal_at(app: &WordApp, text: &str, start: &wordcraft_doc::Pos) -> bool {
    let Some(para) = app.session.doc.para_at(start) else { return false };
    let Some(end) = start.off.checked_add(text.len()) else { return false };
    para.text.get(start.off..end) == Some(text)
}
