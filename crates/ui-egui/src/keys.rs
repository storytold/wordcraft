//! Keyboard: typing, editing keys and command shortcuts (the user's custom keys from Customize
//! Keyboard first, then the registry's `shortcut` fields).

use egui::{Key, Modifiers};
use serde_json::{Value, json};

use crate::WordApp;
use wordcraft_engine::ImeEditOutcome;

/// The most pre-edit text a UI pass will hold and paint. A composition is a word or a phrase;
/// anything larger is a malformed or hostile event, and laying it out every frame would cost.
const MAX_IME_PREEDIT_BYTES: usize = wordcraft_doc::ime_text::MAX_IME_TEXT_BYTES;

/// The largest IME commit inserted in one event. Real commits are short; a multi-megabyte one is
/// refused rather than turned into document text without a bound.
const MAX_IME_COMMIT_BYTES: usize = 64 * 1024;

/// The registry's name for a key.
pub fn key_name(k: Key) -> Option<&'static str> {
    Some(match k {
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        Key::Enter => "Enter",
        Key::Tab => "Tab",
        Key::Backspace => "Backspace",
        Key::Delete => "Delete",
        Key::Escape => "Escape",
        Key::Space => "Space",
        Key::Insert => "Insert",
        Key::Equals | Key::Plus => "=",
        Key::Minus => "-",
        Key::Period => ".",
        Key::Comma => ",",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Num0 => "0",
        Key::Num1 => "1",
        Key::Num2 => "2",
        Key::Num3 => "3",
        Key::Num4 => "4",
        Key::Num5 => "5",
        Key::Num6 => "6",
        Key::Num7 => "7",
        Key::Num8 => "8",
        Key::Num9 => "9",
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        Key::A => "A",
        Key::B => "B",
        Key::C => "C",
        Key::D => "D",
        Key::E => "E",
        Key::F => "F",
        Key::G => "G",
        Key::H => "H",
        Key::I => "I",
        Key::J => "J",
        Key::K => "K",
        Key::L => "L",
        Key::M => "M",
        Key::N => "N",
        Key::O => "O",
        Key::P => "P",
        Key::Q => "Q",
        Key::R => "R",
        Key::S => "S",
        Key::T => "T",
        Key::U => "U",
        Key::V => "V",
        Key::W => "W",
        Key::X => "X",
        Key::Y => "Y",
        Key::Z => "Z",
        _ => return None,
    })
}

/// A key with its modifiers as the registry writes shortcuts (`Mod+Shift+K`).
pub fn combo(m: Modifiers, key: &str, with_shift: bool) -> String {
    let mut s = String::new();
    if m.command {
        s.push_str("Mod+");
    }
    if m.ctrl && !m.command {
        s.push_str("Ctrl+");
    }
    if m.mac_cmd && m.ctrl {
        s.push_str("Ctrl+");
    }
    if m.alt {
        s.push_str("Alt+");
    }
    if m.shift && with_shift {
        s.push_str("Shift+");
    }
    s.push_str(key);
    s
}

/// Shortcuts for interface commands (`ui.*`), which aren't in the engine's keymap: Alt+Q moves
/// to "Tell me" command search, as in Word for Windows (#819). Not on macOS, where Option+Q types
/// a character. On macOS ⌘⌥R collapses or expands the ribbon, as in Word for Mac.
fn ui_shortcut(key: Key, m: Modifiers) -> Option<(&'static str, Value)> {
    let alt_only = m.alt && !m.command && !m.ctrl && !m.mac_cmd && !m.shift;
    let mac_cmd_alt = cfg!(target_os = "macos") && m.mac_cmd && m.alt && !m.ctrl && !m.shift;
    match key {
        Key::Q if alt_only && !cfg!(target_os = "macos") => Some(("ui.dialog", json!({"name": "commands"}))),
        Key::R if mac_cmd_alt => Some(("ui.collapseRibbon", json!({}))),
        // Ctrl+F1 collapses or expands the ribbon, as in Word for Windows (#1678). Not on macOS,
        // where Ctrl+F1 is the system's keyboard-access switch.
        Key::F1 if m.ctrl && !m.alt && !m.shift && !m.mac_cmd && !cfg!(target_os = "macos") => Some(("ui.collapseRibbon", json!({}))),
        _ => None,
    }
}

/// Find and run the command bound to this key. Returns true if handled.
fn dispatch(app: &mut WordApp, key: Key, m: Modifiers) -> bool {
    let reg = app.session.registry.clone();
    let name = key_name(key);
    let bound = name.and_then(|n| app.session.keymap.resolve(&reg, &combo(m, n, true)));
    // Ctrl+F1 collapses the ribbon only when the user hasn't given it to a command (#1678); the
    // other interface shortcuts (Alt+Q, ⌘⌥R) keep their place ahead of the keymap.
    if let Some((id, params)) = ui_shortcut(key, m).filter(|_| key != Key::F1 || bound.is_none()) {
        let _ = app.run(id, params);
        return true;
    }
    if let Some(spec) = bound {
        // Option+Shift+- types an em dash on macOS; Word's Alt+Shift+- (Collapse) only works in
        // Outline view, so elsewhere the key stays typing (#1933).
        if m.alt && !crate::widgets::enabled(app, spec.id) {
            return false;
        }
        let _ = app.run(spec.id, json!({}));
        return true;
    }
    let Some(name) = name else { return false };
    // Shift extends caret movement; Shift+Backspace still erases (#707).
    if m.shift
        && let Some(spec) = app.session.keymap.resolve(&reg, &combo(m, name, false))
        && let Some(params) = shift_variant_params(spec.id)
    {
        let _ = app.run(spec.id, params);
        return true;
    }
    false
}

/// How a command bound to a plain key runs when Shift is also held and nothing is bound to the
/// shifted combo: caret moves extend the selection, Backspace erases as usual (#707).
fn shift_variant_params(id: &str) -> Option<Value> {
    if id.starts_with("caret.") {
        Some(json!({"extend": true}))
    } else if id == "text.backspace" {
        Some(json!({}))
    } else {
        None
    }
}

/// Shift+Delete, which cuts the selection and does nothing without one, as in Word for Windows
/// (#1669). On Windows the windowing layer already sends it as a Cut; elsewhere it arrives as the
/// key, and no shortcut was bound to it.
fn is_shift_delete(key: Key, m: Modifiers) -> bool {
    key == Key::Delete && m.shift && !m.command && !m.ctrl && !m.alt && !m.mac_cmd
}

/// Shift+F10 opens the context menu, as the Menu key does (#821). (The Menu key itself doesn't
/// reach the app: the windowing layer drops it.)
fn is_context_menu_key(key: Key, m: Modifiers) -> bool {
    key == Key::F10 && m.shift && !m.command && !m.alt && !m.ctrl
}

/// F1–F12: shortcuts on these work without the canvas focused.
fn is_function_key(key: Key) -> bool {
    matches!(key, Key::F1 | Key::F2 | Key::F3 | Key::F4 | Key::F5 | Key::F6 | Key::F7 | Key::F8 | Key::F9 | Key::F10 | Key::F11 | Key::F12)
}

/// Keys while editing inside an equation. Returns true when handled.
fn equation_key(app: &mut WordApp, key: Key, m: Modifiers) -> bool {
    if m.command || m.alt || (m.ctrl && !cfg!(target_os = "macos")) {
        return false;
    }
    let (id, params) = match key {
        Key::ArrowLeft => ("equation.move", json!({"dir": "left"})),
        Key::ArrowRight => ("equation.move", json!({"dir": "right"})),
        Key::ArrowUp => ("equation.move", json!({"dir": "up"})),
        Key::ArrowDown => ("equation.move", json!({"dir": "down"})),
        Key::Home => ("equation.move", json!({"dir": "home"})),
        Key::End => ("equation.move", json!({"dir": "end"})),
        Key::Tab => ("equation.move", json!({"dir": if m.shift { "prev" } else { "next" }})),
        Key::Backspace => ("equation.backspace", json!({})),
        Key::Delete => ("equation.delete", json!({})),
        Key::Enter => ("equation.enter", json!({})),
        Key::Escape => ("equation.exit", json!({})),
        _ => return false,
    };
    let _ = app.run(id, params);
    true
}

/// Whether releasing `key` with `modifiers` is the plain Paste shortcut (Mod+V or the Paste
/// key) — not Mod+Shift+V (Paste Formatting) or Mod+Alt+V (Paste Special) — so a clipboard
/// picture may be pasted on it (#45).
fn is_paste_release(key: Key, modifiers: egui::Modifiers) -> bool {
    (modifiers.command || key == Key::Paste) && !modifiers.alt && !modifiers.shift
}

/// The modifiers a V release stands for: its own, or, when Mod went up before V and no V press
/// came (egui-winit keeps Mod+V presses), those held with Mod.
fn paste_release_modifiers(released: Modifiers, press_seen: bool, held: Option<Modifiers>) -> Modifiers {
    if released.command || press_seen { released } else { held.unwrap_or(released) }
}

/// Whether `key` is a Shift, Ctrl, Alt or Super key, whose own presses don't end a Mod+V.
fn is_modifier_key(key: Key) -> bool {
    matches!(
        key,
        Key::ShiftLeft | Key::ShiftRight | Key::ControlLeft | Key::ControlRight | Key::AltLeft | Key::AltRight | Key::SuperLeft | Key::SuperRight
    )
}

/// Whether releasing `key` with `modifiers` is Mod+Alt+V (Paste Special).
fn is_paste_special_release(key: Key, modifiers: egui::Modifiers) -> bool {
    key == Key::V && modifiers.command && modifiers.alt && !modifiers.shift
}

/// The adjacent replacement a refused byte-counted equation deletion must discard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EquationReplacementDrop {
    /// No deletion is waiting for a replacement.
    None,
    /// The next event may be the replacement: one `Commit`, or a `Text` starting a pair.
    Awaiting,
    /// The replacement's `Text` was dropped; its immediately following `Commit` also belongs
    /// to that same platform replacement.
    TextDropped,
}

/// Events for the focused canvas: text, editing keys, clipboard, IME. While keytips are showing
/// every key belongs to them (letters pick badges, Escape cancels), so nothing here runs.
pub fn canvas_events(app: &mut WordApp, ctx: &egui::Context) {
    if app.ui.keytips != crate::keytips::Phase::Off {
        return;
    }
    let events = ctx.input(|i| i.events.clone());
    let window = app.windows.shown();
    if app.session.view.read_mode {
        // Read Mode is for navigation and selection, not typing into the document. The ribbon's
        // Edit Document button or Escape returns to Print Layout before editing.
        for e in &events {
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
    let mut drop_equation_replacement = EquationReplacementDrop::None;
    // A body/equation replacement may have invalidated a live preedit before this batch (or
    // an earlier event can do it below). Its already queued IME replacement belongs to the old
    // target. This is separate from the outgoing interrupt, which is observed after mutation.
    // Keep the quarantine local to this batch: a later untagged commit can be fresh input.
    let mut interrupted_preedit = std::mem::take(&mut app.canvas.ime_interrupted_preedit);
    let mut interrupted_replacement = EquationReplacementDrop::None;
    // Mod held since an earlier frame (its ModifiersChanged went by then).
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
        interrupted_preedit |= std::mem::take(&mut app.canvas.ime_interrupted_preedit);
        let stale_composition_event = if interrupted_preedit {
            match &e {
                egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { .. } | egui::ImeEvent::DeleteSurroundingBytes { .. }) => {
                    interrupted_replacement = EquationReplacementDrop::Awaiting;
                    true
                }
                egui::Event::Ime(egui::ImeEvent::Commit(_)) => {
                    interrupted_preedit = false;
                    interrupted_replacement = EquationReplacementDrop::None;
                    true
                }
                egui::Event::Text(_) if matches!(interrupted_replacement, EquationReplacementDrop::Awaiting) => {
                    interrupted_replacement = EquationReplacementDrop::TextDropped;
                    true
                }
                egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) if text.is_empty() => true,
                // A new marked composition or ordinary input is not the old target's queued
                // replacement. Commit-only input without an interrupted preedit is never gated.
                egui::Event::Ime(egui::ImeEvent::Preedit { .. })
                | egui::Event::Text(_)
                | egui::Event::Paste(_)
                | egui::Event::Key { pressed: true, .. } => {
                    interrupted_preedit = false;
                    interrupted_replacement = EquationReplacementDrop::None;
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
            continue;
        }
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
        // The replacement is the event immediately after the refused deletion: one `Commit`,
        // or the adjacent `Text` + `Commit` pair some platforms use. Any intervening event is
        // unrelated and clears the state before it is handled.
        let drop_event = match (drop_equation_replacement, &e) {
            (EquationReplacementDrop::None, _) => false,
            (EquationReplacementDrop::Awaiting, egui::Event::Text(_)) => {
                drop_equation_replacement = EquationReplacementDrop::TextDropped;
                true
            }
            (EquationReplacementDrop::Awaiting, egui::Event::Ime(egui::ImeEvent::Commit(_))) => {
                drop_equation_replacement = EquationReplacementDrop::None;
                true
            }
            (EquationReplacementDrop::Awaiting, _) => {
                drop_equation_replacement = EquationReplacementDrop::None;
                false
            }
            (EquationReplacementDrop::TextDropped, egui::Event::Ime(egui::ImeEvent::Commit(_))) => {
                drop_equation_replacement = EquationReplacementDrop::None;
                true
            }
            (EquationReplacementDrop::TextDropped, _) => {
                drop_equation_replacement = EquationReplacementDrop::None;
                false
            }
        };
        if drop_event {
            app.canvas.ime_preedit.clear();
            app.canvas.ime_preedit_range = None;
            app.canvas.ime_interrupt_now = true;
            continue;
        }
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
            egui::Event::Text(_) if std::mem::take(&mut app.canvas.swallow_text) => continue,
            egui::Event::Text(t) => {
                if let Some(echo) = app.canvas.alt_code_echo.take()
                    && alt_code_echoed(app, &echo, t)
                {
                    continue;
                }
            }
            // Focus elsewhere (another window) ends an Alt code unfinished.
            egui::Event::WindowFocused(false) => app.canvas.alt_code.clear(),
            _ => {}
        }
        if alt_codes && app.session.math.is_none() && alt_code_event(app, &e) {
            continue;
        }
        if let egui::Event::Paste(t) = &e {
            app.canvas.pasted = true;
            // Copied picture files arrive as their paths: insert the pictures (#45).
            if app.session.math.is_none() && !ctx.input(|i| i.modifiers.alt) && crate::paste_picture::paste_named_files(app, t) {
                continue;
            }
        }
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
            continue;
        }
        // Editing an equation: text and editing keys go into it.
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
                    continue;
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
                            continue;
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
                    continue;
                }
                // A phone keyboard's or macOS replacement inside an equation: erase the counted
                // units on each side with one transactional equation command. The tree cannot
                // provide a surrounding snapshot; an engine revision proves a real commit and the
                // platform is never handed fabricated surroundings.
                egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars, after_chars }) => {
                    delete_in_equation(app, *before_chars, *after_chars);
                    continue;
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
                        drop_equation_replacement = EquationReplacementDrop::Awaiting;
                        app.canvas.ime_interrupt_now = true;
                    }
                    continue;
                }
                egui::Event::Paste(t) => {
                    if app.run("equation.type", json!({"text": t})).is_err() {
                        app.canvas.ime_interrupt_now = true;
                    }
                    continue;
                }
                egui::Event::Key { key, pressed: true, modifiers, .. } if equation_key(app, *key, *modifiers) => continue,
                // Anything else between the deletion and its replacement (a pre-edit, a key
                // release, focus, a pointer event) means the next equation typing command is a
                // new edit, not the immediate replacement. Drop the promise; a command consumes
                // it in `Session::run` even when it is not a typing command.
                _ => app.session.clear_ime_joins(),
            }
        }
        match e {
            egui::Event::Text(t) => {
                let m = ctx.input(|i| i.modifiers);
                let is_cmd = if cfg!(target_os = "macos") { m.command } else { (m.command || m.ctrl) && !m.alt };
                if is_cmd {
                    continue;
                }
                if t.chars().all(|c| !c.is_control()) && !t.is_empty() {
                    match crate::dialogs_smart_art::type_into(app, &t) {
                        crate::dialogs_smart_art::SmartArtTypeResult::Changed => {
                            note_ime_application_change(app);
                            continue;
                        }
                        crate::dialogs_smart_art::SmartArtTypeResult::ConsumedNoChange => continue,
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
            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
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
            egui::Event::Key { key, pressed: true, modifiers, .. } => {
                if key == Key::Escape && app.session.painter.is_some() {
                    app.session.painter = None;
                    continue;
                }
                // The lasso selection and Ink Replay take Delete and Escape first.
                if crate::ink::key(app, key) {
                    continue;
                }
                // Escape puts the pen or eraser down (back to selecting).
                if key == Key::Escape && app.session.view.draw.mode != wordcraft_engine::cmd::draw::DrawMode::Select {
                    app.canvas.ink = None;
                    let _ = app.run("draw.select", json!({}));
                    continue;
                }
                if key == Key::Escape && crate::table_pen::stop(app) {
                    continue;
                }
                // File › Options › Advanced: the Insert key switches overtype only when asked to.
                if key == Key::Insert && modifiers.is_none() {
                    if app.session.prefs.insert_key_overtype {
                        let _ = app.run("text.overtype", json!({}));
                    }
                    continue;
                }
                if crate::objects::key(app, key, modifiers) {
                    continue;
                }
                // Shift+F10, Windows' keyboard way to the context menu (#821).
                if is_context_menu_key(key, modifiers) {
                    app.canvas.context_menu_from_keys = true;
                    continue;
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
                    continue;
                }
                if key == Key::Escape && matches!(app.session.sel.focus.story, wordcraft_doc::StoryRef::Part(_)) && app.session.sel.is_collapsed() {
                    let _ = app.run("insert.closeHeader", json!({}));
                    continue;
                }
                if is_shift_delete(key, modifiers) {
                    if !app.session.sel.is_collapsed()
                        && let Ok(r) = app.run("edit.cut", json!({}))
                    {
                        crate::system_clipboard::put_result(app, ctx, &r);
                    }
                    continue;
                }
                if leave_full_screen(app, ctx, key, modifiers, true) {
                    continue;
                }
                app.canvas.swallow_text = dispatch(app, key, modifiers) && modifiers.alt;
            }
            _ => {}
        }
    }
}

/// Escape leaves Focus or Full Screen View (#1678), then Full Screen (#980), once nothing nearer
/// took it: a pen, a text box, a header, a dialog, a menu or a popup, a drag on the page, and in
/// the document (`canvas`) a selection, which Escape collapses first (`select.collapse`), as in a
/// normal window.
fn leave_full_screen(app: &mut WordApp, ctx: &egui::Context, key: Key, m: Modifiers, canvas: bool) -> bool {
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

/// Alt+numpad character codes are a Windows convention; on macOS Option+digit types its own
/// characters (¡ ™ £…), so they're off there (also in a browser on a Mac).
fn alt_codes_supported(ctx: &egui::Context) -> bool {
    !cfg!(target_os = "macos") && ctx.os() != egui::os::OperatingSystem::Mac
}

/// The digit a key types for an Alt code.
///
/// Word counts only numeric-keypad digits. egui 0.36 reports keypad and top-row digits alike
/// (`Key::Num0`…`Num9`, with the same `physical_key`), and eframe passes no winit key location
/// on, so the top-row digits compose codes too (they type nothing else with Alt alone; Ctrl+Alt+1
/// and keytip digits are untouched).
fn alt_code_digit(key: Key) -> Option<char> {
    let n = match key {
        Key::Num0 => 0,
        Key::Num1 => 1,
        Key::Num2 => 2,
        Key::Num3 => 3,
        Key::Num4 => 4,
        Key::Num5 => 5,
        Key::Num6 => 6,
        Key::Num7 => 7,
        Key::Num8 => 8,
        Key::Num9 => 9,
        _ => return None,
    };
    char::from_digit(n, 10)
}

/// Alt+numpad entry (#961): digits typed with Alt alone held collect; releasing Alt types the
/// character their code names (`text.typeCode`: 0233 → é, 233 → Θ in code page 437). Returns
/// whether the event was part of it (and isn't handled otherwise).
fn alt_code_event(app: &mut WordApp, e: &egui::Event) -> bool {
    let alt_only = |m: &Modifiers| m.alt && !m.ctrl && !m.shift && !m.command && !m.mac_cmd;
    match e {
        egui::Event::Key { key, pressed: true, repeat, modifiers, .. } if alt_only(modifiers) => match alt_code_digit(*key) {
            // A held digit's auto-repeat adds nothing (as on Windows).
            Some(_) if *repeat => {
                app.canvas.swallow_text = true;
                true
            }
            Some(d) => {
                // Codes beyond seven digits name nothing; stop collecting rather than grow.
                if app.canvas.alt_code.len() < 8 {
                    app.canvas.alt_code.push(d);
                }
                // The digit's text (Linux sends it even with Alt held) isn't typed.
                app.canvas.swallow_text = true;
                true
            }
            None => {
                app.canvas.alt_code.clear();
                false
            }
        },
        egui::Event::Key { key: Key::AltLeft | Key::AltRight, pressed: false, .. } => {
            finish_alt_code(app);
            false
        }
        egui::Event::ModifiersChanged(m) if !m.alt => {
            finish_alt_code(app);
            false
        }
        egui::Event::Key { pressed: true, .. } => {
            // Another key with Ctrl/Shift held (or without Alt) ends the entry unfinished.
            app.canvas.alt_code.clear();
            false
        }
        _ => false,
    }
}

/// What an Alt code typed, for the platform's own composed character that may follow.
#[derive(Clone, Debug)]
pub(crate) struct AltCodeEcho {
    /// The character typed.
    typed: String,
    /// The system composes Alt codes itself (Windows): whatever it sends is the code's character.
    system_composes: bool,
    /// The code named a byte of the system's code pages (1–255), so the system's character wins
    /// over ours: it knows the code pages (DBCS ones too) better than our locale guess.
    byte_code: bool,
}

/// Alt released: type the collected code's character, if any.
fn finish_alt_code(app: &mut WordApp) {
    let code = std::mem::take(&mut app.canvas.alt_code);
    if code.is_empty() {
        return;
    }
    crate::objects::enter_text_box(app);
    let system_composes = cfg!(target_os = "windows");
    let mut params = json!({"code": code});
    if system_composes {
        // Windows: the code pages of the system locale (Polish 1250/852, Russian 1251/866…).
        let (ansi, oem) = wordcraft_engine::cmd::text::system_code_pages();
        params = json!({"code": code, "ansi": ansi, "oem": oem});
        // The system's composed character comes as text after the digits' swallowed text.
        app.canvas.swallow_text = false;
    }
    if let Ok(r) = app.run("text.typeCode", params) {
        let byte_code = code.trim_start_matches('0').parse::<u32>().is_ok_and(|v| v <= 255);
        app.canvas.alt_code_echo = r.get("char").and_then(Value::as_str).map(|c| AltCodeEcho { typed: c.to_string(), system_composes, byte_code });
    }
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

/// The bounded visible-logical window around the body selection, cached by `(revision, cursor,
/// anchor)` so an idle pass does not walk a long paragraph again. Every edit bumps `rev`.
fn ime_window(app: &mut WordApp) -> Option<wordcraft_doc::ime_text::ImeWindow> {
    let rev = app.session.rev();
    let cursor = app.session.sel.focus.clone();
    let anchor = app.session.sel.anchor.clone();
    if let Some((r, c, a, w)) = &app.canvas.ime_window_cache
        && *r == rev
        && *c == cursor
        && *a == anchor
    {
        return w.clone();
    }
    let window = app.session.doc.ime_surrounding(&cursor, &anchor, wordcraft_doc::ime_text::MAX_IME_TEXT_BYTES);
    app.canvas.ime_window_cache = Some((rev, cursor, anchor, window.clone()));
    window
}

/// Observe the settled editor at the end of a UI pass and publish its IME descriptor.
///
/// The tracker is scoped by native viewport. A closed viewport releases its state; a pass without
/// a visible focused editor reports an unfocused observation so a later focus starts afresh.
pub(crate) fn observe_ime(app: &mut WordApp, ctx: &egui::Context, geometry: Option<(egui::Rect, egui::Rect)>, interrupt: bool) {
    let viewport = ctx.viewport_id();
    // Drain an application-origin change before the final observation. The signal survives later
    // commands until here (or until a receipt publishes), so an edit after the receipt still
    // makes the frame `Other`.
    if app.session.take_pending_ime_application_change() {
        note_ime_application_change(app);
    }
    if app.canvas.ime_viewport != Some(viewport) {
        if let Some(old) = app.canvas.ime_viewport {
            app.canvas.ime_tracker.end_viewport(old);
        }
        app.canvas.ime_viewport = Some(viewport);
    }
    let frame = ctx.cumulative_frame_nr_for(viewport);
    let pass = ctx.cumulative_pass_nr_for(viewport);
    let focused = app.canvas.focused && geometry.is_some();
    let update = if focused {
        let target = ime_target(app);
        if app.session.math.is_some() {
            // An equation's structured tree has no reversible text mapping yet: report an
            // explicit Unavailable, never fabricated surroundings. Its stable session token is
            // the replacement epoch, so caret motion keeps one context. The engine revision lets
            // an unavailable receipt prove a committed edit.
            let mut observation = egui::ImeObservation::new(frame, pass, target);
            if let Some(rev) = app.session.ime_state_revision() {
                observation = observation.unavailable_revision(rev);
            }
            app.canvas.ime_tracker.observe(viewport, Some(observation))
        } else {
            let window = ime_window(app);
            if let Some(w) = &window {
                let committed = Some(egui::CommittedText::new(&w.text, w.cursor, w.anchor));
                app.canvas.ime_tracker.observe(viewport, Some(egui::ImeObservation::new(frame, pass, target).committed(committed)))
            } else {
                // A wide selection or cross-story caret has no single surrounding window. Its
                // engine revision orders unavailable observations; no text is fabricated.
                let mut observation = egui::ImeObservation::new(frame, pass, target);
                if let Some(rev) = app.session.ime_state_revision() {
                    observation = observation.unavailable_revision(rev);
                }
                app.canvas.ime_tracker.observe(viewport, Some(observation))
            }
        }
    } else {
        // No editor has the keyboard: a pre-edit painted from the old focus must not linger.
        app.canvas.ime_preedit.clear();
        app.canvas.ime_preedit_range = None;
        app.canvas.ime_tracker.observe(viewport, None)
    };
    if let (Some(update), Some((rect, cursor_rect))) = (update, geometry) {
        ctx.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                owner: update.owner,
                context_generation: update.context_generation,
                purpose: update.purpose,
                rect,
                cursor_rect,
                should_interrupt_composition: interrupt,
                reset_request: None,
                change_cause: update.change_cause,
                surrounding_text: update.surrounding_text,
            });
        });
    }
}

/// The immutable IME owner for the editor that currently has the keyboard.
///
/// The document identity and story are stable across edits; an equation adds its editing-session
/// token as the replacement epoch because its document position moves with the text.
fn ime_target(app: &WordApp) -> egui::ImeTarget {
    let document = app.session.document_id();
    if let Some(math) = &app.session.math {
        egui::ImeTarget::new(egui::Id::unique(("wordcraft-ime-equation", document)), math.session)
    } else {
        // A body replaced under the same document id (setText, version restore, compare,
        // Undo/Redo) advances the epoch, so the platform clears the old composition.
        egui::ImeTarget::new(egui::Id::unique(("wordcraft-ime-body", document, app.session.sel.focus.story)), app.session.ime_body_epoch())
    }
}

/// The bounded committed snapshot used by IME receipts.
///
/// An equation has no reversible tree-to-text mapping, and a selection across blocks has no
/// single visible paragraph, so both are honestly unavailable rather than fabricated.
fn ime_stamp(app: &mut WordApp) -> egui::ImeMutationStamp {
    let unavailable = |app: &mut WordApp| match app.session.ime_state_revision() {
        Some(rev) => egui::ImeMutationStamp::unavailable_at_revision(rev),
        None => egui::ImeMutationStamp::unavailable(),
    };
    if app.session.math.is_some() {
        return unavailable(app);
    }
    match ime_window(app) {
        Some(w) => egui::ImeMutationStamp::from_committed(&w.text, w.cursor, w.anchor, None),
        None => unavailable(app),
    }
}

/// Record a successful accepted IME commit/delete for the current frame and viewport.
fn note_ime_receipt(app: &mut WordApp, before: egui::ImeMutationStamp, after: egui::ImeMutationStamp) {
    let Some(ctx) = app.ctx.clone() else { return };
    let viewport = ctx.viewport_id();
    let frame = ctx.cumulative_frame_nr_for(viewport);
    let target = ime_target(app);
    app.canvas.ime_tracker.note_input_method_apply(viewport, frame, target, before, after);
}

/// Record an application-origin edit in the current frame; `Other` dominates the frame.
fn note_ime_application_change(app: &mut WordApp) {
    let Some(ctx) = app.ctx.clone() else { return };
    let viewport = ctx.viewport_id();
    let frame = ctx.cumulative_frame_nr_for(viewport);
    let target = ime_target(app);
    app.canvas.ime_tracker.note_application_change(viewport, frame, target);
}

/// Whether the stamp pair proves the transition an IME receipt needs.
/// The receipt decision, kept pure so tests can exercise every revision/availability case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImeProof {
    /// Classified no-op with an unchanged stamp: leave an earlier receipt as the frame's cause.
    NoMutation,
    /// A committed change the stamps cannot prove (or an automatic rewrite): `Other`.
    UnprovenMutation,
    /// A classified content change whose exact stamp transition is proven.
    Certified,
}

/// Wraps egui's unavailable-revision accessor for an IME mutation stamp.
fn unavailable_revision(stamp: &egui::ImeMutationStamp) -> Option<u64> {
    stamp.unavailable_revision()
}

/// Whether a pair of IME mutation stamps proves a transition: changed surrounding text, an
/// increase between unavailable revisions, or a mixed transition whose unavailable endpoint
/// carries a revision.
fn stamp_transition_proven(before: &egui::ImeMutationStamp, after: &egui::ImeMutationStamp) -> bool {
    use egui::output::IMESurroundingText;
    match (before.surrounding_text(), after.surrounding_text()) {
        (IMESurroundingText::Available(a), IMESurroundingText::Available(b)) => a != b,
        (IMESurroundingText::Unavailable, IMESurroundingText::Unavailable) => {
            matches!((unavailable_revision(before), unavailable_revision(after)), (Some(a), Some(b)) if a < b)
        }
        // A mixed transition has one unavailable endpoint; only that endpoint carries a revision.
        (IMESurroundingText::Unavailable, IMESurroundingText::Available(_)) | (IMESurroundingText::Available(_), IMESurroundingText::Unavailable) => {
            let unavailable = if matches!(before.surrounding_text(), IMESurroundingText::Unavailable) { before } else { after };
            unavailable_revision(unavailable).is_some()
        }
    }
}

/// Classifies an IME outcome and stamp pair as no mutation, unproven mutation or certified. Only a
/// classified, untransformed content change with a proven stamp transition is certified.
fn classify_ime_proof(outcome: ImeEditOutcome, before: &egui::ImeMutationStamp, after: &egui::ImeMutationStamp) -> ImeProof {
    if !outcome.classified {
        return if outcome.state_changed { ImeProof::UnprovenMutation } else { ImeProof::NoMutation };
    }
    if outcome.transformed {
        return ImeProof::UnprovenMutation;
    }
    if outcome.state_changed && !outcome.content_changed {
        return ImeProof::UnprovenMutation;
    }
    if outcome.content_changed {
        return if stamp_transition_proven(before, after) { ImeProof::Certified } else { ImeProof::UnprovenMutation };
    }
    if before == after { ImeProof::NoMutation } else { ImeProof::UnprovenMutation }
}

/// Publish at most one receipt for a successful IME command, or force `Other` when the engine
/// outcome and the bounded stamps cannot prove a pure content change.
fn publish_ime_receipt(app: &mut WordApp, before: egui::ImeMutationStamp, after: egui::ImeMutationStamp, outcome: ImeEditOutcome) {
    if app.ctx.is_none() {
        return;
    }
    // A successful application-origin command since the last observation poisons the frame, even
    // when the available text stamp is unchanged (formatting-only changes).
    if app.session.take_pending_ime_application_change() {
        note_ime_application_change(app);
        return;
    }
    match classify_ime_proof(outcome, &before, &after) {
        ImeProof::NoMutation => {}
        ImeProof::UnprovenMutation => note_ime_application_change(app),
        ImeProof::Certified => note_ime_receipt(app, before, after),
    }
}

/// AutoCorrect or AutoFormat just changed the text around the caret. The web's text agent still
/// holds the text as typed and counts its `DeleteSurrounding` from that, so it is asked to start
/// afresh (`canvas::ime_reset`). The caller passes the engine's per-command outcome; an undo label
/// from an earlier step cannot make an unrelated literal commit look transformed.
fn note_automatic_change(app: &mut WordApp, outcome: ImeEditOutcome) {
    if outcome.transformed {
        app.canvas.ime_reset = true;
    }
}

/// An input method's `DeleteSurrounding`: erase `before` characters (Unicode scalar values, as
/// the input method counts them) before the caret and `after` after it, as Backspace does over a
/// selection. The web build's text agent (eframe's hidden `<input>`) sends it, then the new
/// text, whenever its value changes other than by an addition at its end (#1480): Backspace
/// and corrections from phone keyboards, and Chrome's caret jump. The desktop builds never get
/// it (egui-winit 0.36 doesn't send it).
///
/// The counts are over the visible logical text: tracked deletions are hidden and inline objects
/// contribute their plain-text result. They exclude the selection, which is preserved, and any
/// pre-edit (which never lives in the model here). Erasure joins an open typing group and the
/// text that follows joins it too ([`type_text`]), so one Undo takes back the typing with its
/// correction. Neither path runs inside an equation, whose text is not the document's.
fn delete_surrounding(app: &mut WordApp, before: usize, after: usize) {
    delete_surrounding_common(app, before, after, wordcraft_doc::ime_text::DeleteUnit::Chars);
}

/// An input method's byte-counted `DeleteSurroundingBytes` (macOS and Wayland): erase the UTF-8
/// range it names, then the following `Commit` replaces it. Both counts round inward at a
/// character boundary (a start forward, an end backward), so a count that ends inside a
/// character leaves that character whole. The selection and paragraph marks stay outside the
/// range.
fn delete_surrounding_bytes(app: &mut WordApp, before: usize, after: usize) {
    delete_surrounding_common(app, before, after, wordcraft_doc::ime_text::DeleteUnit::Bytes);
}

/// Refuse a body surrounding request and discard the composition it invalidates.
///
/// The platform has already changed the text it believed was around the caret, so its snapshot is
/// stale: interrupt it, unpaint the local pre-edit, and drop every replacement promise (body and
/// equation) that could otherwise join a later edit into an earlier deletion.
fn refuse_ime_request(app: &mut WordApp) {
    app.canvas.ime_preedit.clear();
    app.canvas.ime_preedit_range = None;
    app.canvas.ime_join_text = None;
    app.session.clear_ime_joins();
    app.canvas.ime_interrupt_now = true;
}

/// A signed stored-length change: negative when the paragraph shrank, positive only when a
/// restored content-control placeholder grew in place of the erased text.
fn signed_len_delta(before: usize, after: usize) -> Option<i64> {
    let after = i64::try_from(after).ok()?;
    let before = i64::try_from(before).ok()?;
    after.checked_sub(before)
}

/// Move a stored UTF-8 offset by a signed paragraph-length delta.
fn shift_offset(off: usize, shift: i64) -> Option<usize> {
    let off = i64::try_from(off).ok()?;
    usize::try_from(off.checked_add(shift)?).ok()
}

/// The end of a placeholder `restore_placeholder` just inserted at or before `deleted_start`.
///
/// `restore_placeholder` sets the caret to the control's content start before inserting. Finding
/// that exact state proves the growth came from a placeholder insertion at a position before the
/// erased range, so the stored suffix really moved by the measured signed delta.
fn restored_placeholder_end(s: &wordcraft_engine::Session, deleted_start: usize) -> Option<usize> {
    let at = &s.sel.focus;
    let r = s.doc.control_at(at)?;
    if !r.control.showing_placeholder || r.content_start() != *at || r.start.path != r.end.path {
        return None;
    }
    let start = r.content_start().off;
    if start > deleted_start {
        return None;
    }
    let placeholder = r.control.placeholder_shown();
    let end = start.checked_add(placeholder.len())?;
    // The caret alone is not proof: a control that was already showing its placeholder and had
    // that text deleted also leaves the caret at the content start. Require the restored
    // placeholder text itself to occupy the content from the start.
    let para = s.doc.para_at(at)?;
    (!placeholder.is_empty() && para.text.get(start..end) == Some(placeholder)).then_some(end)
}

/// The exact state a refused IME delete-surrounding transaction restores.
struct ImeRollback {
    snapshot: Option<wordcraft_engine::EditSnapshot>,
    original: wordcraft_engine::Selection,
    pending: Option<wordcraft_doc::props::CharProps>,
    goal_x: Option<f32>,
}

impl ImeRollback {
    fn apply(&mut self, app: &mut WordApp) {
        let Some(snapshot) = self.snapshot.take() else {
            return;
        };
        app.session.restore(snapshot);
        app.session.sel = self.original.clone();
        app.session.pending = self.pending.clone();
        app.session.goal_x = self.goal_x;
        refuse_ime_request(app);
    }
}

/// Whether the edit is a classified content and state change that is not a transform.
fn ime_edit_satisfied(app: &mut WordApp) -> bool {
    let outcome = app.session.ime_edit_outcome();
    outcome.classified && outcome.content_changed && outcome.state_changed && !outcome.transformed
}

/// Delete one IME-surrounding range. Returns the paragraph's signed length change and the end of
/// a placeholder the deletion restored, or `None` after rolling back the whole transaction.
fn ime_delete_side(
    app: &mut WordApp,
    rollback: &mut ImeRollback,
    cursor: &wordcraft_doc::Pos,
    start: usize,
    end: usize,
    join_undo: bool,
) -> Option<(i64, Option<usize>)> {
    if start >= end {
        rollback.apply(app);
        return None;
    }
    let Some(len_before) = app.session.doc.para_at(cursor).map(|p| p.len()) else {
        rollback.apply(app);
        return None;
    };
    // A control already showing its placeholder cannot have received a new one from this
    // deletion; its unchanged text must not be mistaken for a restored placeholder.
    let showing_before =
        app.session.doc.control_at(&wordcraft_doc::Pos { off: end, ..cursor.clone() }).is_some_and(|r| r.control.showing_placeholder);
    app.session.sel = wordcraft_engine::Selection {
        anchor: wordcraft_doc::Pos { off: start, ..cursor.clone() },
        focus: wordcraft_doc::Pos { off: end, ..cursor.clone() },
    };
    if join_undo {
        app.session.join_next_undo();
    }
    if app.run_ime("text.backspace", json!({})).is_err() || !ime_edit_satisfied(app) {
        rollback.apply(app);
        return None;
    }
    // A tracked delete of text this author inserted removes it from the stored paragraph;
    // other tracked deletions only mark it. A content control emptied by the deletion gains
    // its placeholder again, which can make the paragraph longer. Measure the signed change
    // and require proof for growth: only a placeholder restored at or before the erased
    // range can move the suffix right.
    let Some(len_after) = app.session.doc.para_at(cursor).map(|p| p.len()) else {
        rollback.apply(app);
        return None;
    };
    let Some(delta) = signed_len_delta(len_before, len_after) else {
        rollback.apply(app);
        return None;
    };
    let restored = restored_placeholder_end(&app.session, start).filter(|_| !showing_before);
    if delta > 0 && restored.is_none() {
        // Growth that cannot be attributed to a placeholder insertion at/before the erased
        // range has no provable suffix mapping: refuse the whole transaction.
        rollback.apply(app);
        return None;
    }
    // Keep the restored placeholder's end: for a caret it is the provable insertion point,
    // also when a tracked deletion keeps the erased bytes in the stored paragraph.
    Some((delta, restored))
}

/// The final caret/selection offsets after one or two surrounding deletions. A restored
/// placeholder end is the only provable position when one side inserted it; otherwise both
/// stored offsets move by the paragraph's signed length change.
fn ime_surrounding_offsets(
    del_anchor: usize,
    del_focus: usize,
    shift: i64,
    first_placeholder_end: Option<usize>,
    second_placeholder_end: Option<usize>,
) -> Option<(usize, usize)> {
    if let Some(end) = second_placeholder_end {
        // A selection spanning the emptied control cannot be remapped through a placeholder
        // insertion without guessing; only a caret has a provable position.
        return (del_anchor == del_focus).then_some((end, end));
    }
    if let (Some(end), true) = (first_placeholder_end, del_anchor == del_focus) {
        // The first side restored the placeholder; its end is the caret's provable position even
        // while an after-side deletion is applied beyond the control.
        return Some((end, end));
    }
    Some((shift_offset(del_anchor, shift)?, shift_offset(del_focus, shift)?))
}

/// Both delete-surrounding paths: one visible-logical mapping for the query and the deletion.
fn delete_surrounding_common(app: &mut WordApp, before: usize, after: usize, unit: wordcraft_doc::ime_text::DeleteUnit) {
    if before == 0 && after == 0 {
        return;
    }
    // A new surrounding request supersedes any replacement promised by an earlier one.
    app.session.clear_ime_joins();
    if app.session.math.is_some() {
        refuse_ime_request(app);
        return;
    }
    let cursor = app.session.sel.focus.clone();
    let anchor = app.session.sel.anchor.clone();
    if cursor.story != anchor.story || cursor.path != anchor.path {
        refuse_ime_request(app);
        return;
    }
    let Some(del) = app.session.doc.ime_delete_surrounding(&cursor, &anchor, before, after, unit) else {
        refuse_ime_request(app);
        return;
    };
    if del.before.is_none() && del.after.is_none() {
        refuse_ime_request(app);
        return;
    }
    let join = app.session.typing_open();
    let props = app.session.typing_props();
    let original = app.session.sel.clone();
    let pending_before = app.session.pending.clone();
    let goal_x_before = app.session.goal_x;
    let before_stamp = ime_stamp(app);
    // Both sides are one IME request: if the second half is refused (Restrict Editing, a
    // content control, an already-deleted tracked range, a classified no-op…), the first half
    // must go back too. The document, selection, undo stack, typing group, dirty flag, pending
    // formatting, `goal_x` and every pending join are restored or cleared. Deliberately not
    // restored: `last_command`, macro recording, Go Back marks, `also_selected`, `typing`/
    // `autocap` and canvas preedit (a refusal clears it).
    let snapshot = app.session.edit_snapshot();
    let mut rollback = ImeRollback { snapshot: Some(snapshot), original, pending: pending_before, goal_x: goal_x_before };
    // Stored offsets of the suffix (and of the original selection) move by the paragraph's
    // signed length change. A placeholder can make that positive; subtracting a removed-byte
    // count cannot describe that, and would leave the second range and the caret inside the
    // placeholder.
    let mut shift = 0i64;
    let mut deleted = false;
    let mut first_placeholder_end = None;
    if let Some(range) = del.before.clone() {
        let Some((delta, restored)) = ime_delete_side(app, &mut rollback, &cursor, range.start, range.end, join) else {
            return;
        };
        deleted = true;
        first_placeholder_end = restored;
        shift = delta;
    }
    let mut second_placeholder_end = None;
    if let Some(range) = del.after {
        let Some(start) = shift_offset(range.start, shift) else {
            rollback.apply(app);
            return;
        };
        let Some(end) = shift_offset(range.end, shift) else {
            rollback.apply(app);
            return;
        };
        let in_bounds = app
            .session
            .doc
            .para_at(&cursor)
            .is_some_and(|p| start < end && end <= p.len() && p.text.is_char_boundary(start) && p.text.is_char_boundary(end));
        if !in_bounds {
            rollback.apply(app);
            return;
        }
        // The mapped end is in the paragraph as the first side left it; a control already
        // showing its placeholder there is not one this side restores.
        let Some((_, restored)) = ime_delete_side(app, &mut rollback, &cursor, start, end, deleted || join) else {
            return;
        };
        deleted = true;
        second_placeholder_end = restored;
    }
    let Some((anchor_off, focus_off)) = ime_surrounding_offsets(del.anchor, del.focus, shift, first_placeholder_end, second_placeholder_end) else {
        rollback.apply(app);
        return;
    };
    let valid =
        app.session.doc.para_at(&cursor).is_some_and(|p| {
            anchor_off <= p.len() && focus_off <= p.len() && p.text.is_char_boundary(anchor_off) && p.text.is_char_boundary(focus_off)
        });
    if !valid {
        rollback.apply(app);
        return;
    }
    app.session.sel = wordcraft_engine::Selection {
        anchor: wordcraft_doc::Pos { off: anchor_off, ..cursor.clone() },
        focus: wordcraft_doc::Pos { off: focus_off, ..cursor },
    };
    if deleted {
        // Only the next `text.insert` at this exact selection may join the deletion; any
        // intervening ordinary command consumes and discards the promise and owns its own Undo
        // checkpoint.
        app.session.join_next_ime_text_replacement();
        app.canvas.ime_join_text = Some(props);
        let after_stamp = ime_stamp(app);
        let outcome = ImeEditOutcome { classified: true, content_changed: true, state_changed: true, transformed: false, selection_changed: true };
        publish_ime_receipt(app, before_stamp, after_stamp, outcome);
    }
}

/// An input method's `DeleteSurrounding` inside an equation: one bounded transactional command.
///
/// The structured tree has no surrounding snapshot; the engine revision on the unavailable stamp
/// proves the commit. Any refusal leaves the document untouched and interrupts the composition.
/// `Session::run` has already consumed the pending undo joins before dispatch and may have
/// recorded the attempted no-op command in the macro/Repeat; those generic side effects are not
/// rolled back.
fn delete_in_equation(app: &mut WordApp, before: usize, after: usize) {
    if app.session.math.is_none() {
        return;
    }
    // A new surrounding request supersedes any earlier equation deletion's promised join,
    // whether or not this one can run. Clear the painted composition before every refusal,
    // too: it belongs to a snapshot the interrupt is about to discard.
    app.session.clear_ime_joins();
    app.canvas.ime_preedit.clear();
    app.canvas.ime_preedit_range = None;
    if before > wordcraft_engine::MAX_EQUATION_DELETE_UNITS || after > wordcraft_engine::MAX_EQUATION_DELETE_UNITS {
        app.canvas.ime_interrupt_now = true;
        return;
    }
    if before == 0 && after == 0 {
        return;
    }
    if app.session.typing_open() {
        app.session.join_next_undo();
    }
    let before_stamp = ime_stamp(app);
    match app.run_ime("equation.deleteSurrounding", json!({"before": before, "after": after})) {
        Ok(_) => {
            // The replacement the platform commits next belongs to the same edit as the
            // deletion, exactly like the body path's `ime_join_text`. This is an equation-only
            // promise: an unrelated command consumes it without joining, so it cannot merge
            // into this deletion's undo step.
            app.session.join_next_equation_edit();
            let outcome = app.session.ime_edit_outcome();
            let after_stamp = ime_stamp(app);
            publish_ime_receipt(app, before_stamp, after_stamp, outcome);
        }
        Err(_) => {
            app.canvas.ime_interrupt_now = true;
        }
    }
}
/// Text arriving right after an Alt code typed `echo` (no key pressed between): whether it was
/// the platform's composed character and is handled. The same character isn't typed twice. On
/// Windows the text is always the composed character: for a code up to 255 it replaces ours (the
/// system knows its code pages), above 255 ours stays (Windows reduces it to a byte; Word types
/// the Unicode character).
fn alt_code_echoed(app: &mut WordApp, echo: &AltCodeEcho, text: &str) -> bool {
    if echo.typed == text {
        return true;
    }
    if !echo.system_composes {
        return false;
    }
    if echo.byte_code && text.chars().count() == 1 {
        let s = &app.session;
        let f = s.sel.focus.clone();
        let start = f.off.checked_sub(echo.typed.len());
        let ours =
            start.filter(|_| s.sel.is_collapsed()).filter(|st| s.doc.para_at(&f).and_then(|p| p.text.get(*st..f.off)) == Some(echo.typed.as_str()));
        if let Some(start) = ours {
            app.session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos { off: start, ..f.clone() }, focus: f };
            let _ = app.run("text.insert", json!({"text": text}));
        }
    }
    true
}

/// Whether a key runs its shortcut with the canvas unfocused (after a ribbon click, say): Mod and
/// Alt shortcuts (Alt+Q reaches Search from anywhere, as in Word, #819) and the function keys.
fn is_global_shortcut(key: Key, m: Modifiers) -> bool {
    m.command || m.alt || is_function_key(key)
}

/// Command shortcuts when the canvas isn't focused and no text field is: nothing has the
/// keyboard, or a button or a pane's title bar does. A focused widget in an open dialog or popup
/// keeps the keys, so Ctrl+Z on a dialog button can't undo the document behind it.
pub fn global_shortcuts(app: &mut WordApp, ctx: &egui::Context) {
    // Customize Keyboard records the keys pressed; they never run commands meanwhile.
    if app.canvas.focused
        || ctx.text_edit_focused()
        || (ctx.egui_wants_keyboard_input() && crate::panes::overlay_open(app, ctx))
        || matches!(app.dialog, Some(crate::dialogs::Dialog::CustomizeKeyboard { .. }))
    {
        return;
    }
    let events = ctx.input(|i| i.events.clone());
    let window = app.windows.shown();
    for e in events {
        // The rest of the keys were pressed in a window that closed or went to the back.
        if app.windows.shown() != window {
            break;
        }
        if let egui::Event::Key { key, pressed: true, modifiers, .. } = e {
            if leave_full_screen(app, ctx, key, modifiers, false) {
                continue;
            }
            if is_global_shortcut(key, modifiers) {
                dispatch(app, key, modifiers);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an IME edit outcome test fixture with `selection_changed` false.
    fn outcome(classified: bool, content: bool, state: bool, transformed: bool) -> ImeEditOutcome {
        ImeEditOutcome { classified, content_changed: content, state_changed: state, transformed, selection_changed: false }
    }

    /// The central gate: a classified no-op leaves an earlier receipt alone, while any
    /// unprovable mutation forces `Other`.
    #[test]
    fn ime_receipt_gate_distinguishes_noop_from_unproven_mutation() {
        let same = egui::ImeMutationStamp::from_committed("ab", 2, 2, None);
        assert_eq!(classify_ime_proof(outcome(true, false, false, false), &same, &same), ImeProof::NoMutation);
        let changed = egui::ImeMutationStamp::from_committed("abx", 3, 3, None);
        assert_eq!(classify_ime_proof(outcome(true, true, true, false), &same, &changed), ImeProof::Certified);
        assert_eq!(classify_ime_proof(outcome(true, true, true, true), &same, &changed), ImeProof::UnprovenMutation);
        assert_eq!(classify_ime_proof(outcome(false, false, true, false), &same, &changed), ImeProof::UnprovenMutation);
        assert_eq!(classify_ime_proof(outcome(true, true, true, false), &same, &same), ImeProof::UnprovenMutation);
    }

    /// Unavailable receipts need both checked revisions and a strict increase; mixed transitions
    /// need the unavailable endpoint's revision.
    #[test]
    fn ime_receipt_gate_checks_unavailable_revisions() {
        let ten = egui::ImeMutationStamp::unavailable_at_revision(10);
        let eleven = egui::ImeMutationStamp::unavailable_at_revision(11);
        let missing = egui::ImeMutationStamp::unavailable();
        let content = outcome(true, true, true, false);
        assert_eq!(classify_ime_proof(content, &ten, &eleven), ImeProof::Certified);
        assert_eq!(classify_ime_proof(content, &eleven, &ten), ImeProof::UnprovenMutation);
        assert_eq!(classify_ime_proof(content, &ten, &ten), ImeProof::UnprovenMutation);
        assert_eq!(classify_ime_proof(content, &missing, &eleven), ImeProof::UnprovenMutation);
        let available = egui::ImeMutationStamp::from_committed("abx", 3, 3, None);
        assert_eq!(classify_ime_proof(content, &ten, &available), ImeProof::Certified);
        assert_eq!(classify_ime_proof(content, &missing, &available), ImeProof::UnprovenMutation);
        assert_eq!(classify_ime_proof(content, &available, &ten), ImeProof::Certified);
        assert_eq!(classify_ime_proof(content, &available, &missing), ImeProof::UnprovenMutation);
    }

    /// A revision-only unavailable change is a changed observation, not a no-op.
    #[test]
    fn ime_receipt_gate_treats_revision_only_changes_as_unproven() {
        let ten = egui::ImeMutationStamp::unavailable_at_revision(10);
        let eleven = egui::ImeMutationStamp::unavailable_at_revision(11);
        assert_eq!(classify_ime_proof(outcome(true, false, false, false), &ten, &eleven), ImeProof::UnprovenMutation);
    }

    /// An exhausted engine allocator emits `Unavailable` without a revision. It can never prove
    /// a content change; a classified no-op still leaves an earlier receipt alone.
    #[test]
    fn ime_receipt_gate_rejects_exhausted_unavailable_stamps() {
        let exhausted = egui::ImeMutationStamp::unavailable();
        assert_eq!(classify_ime_proof(outcome(true, true, true, false), &exhausted, &exhausted), ImeProof::UnprovenMutation);
        assert_eq!(classify_ime_proof(outcome(true, false, false, false), &exhausted, &exhausted), ImeProof::NoMutation);
    }

    /// #1678: Ctrl+F1 collapses or expands the ribbon (not on macOS); plain F1 stays Help's.
    #[test]
    fn ctrl_f1_collapses_the_ribbon() {
        let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
        let expect = if cfg!(target_os = "macos") { None } else { Some(("ui.collapseRibbon", json!({}))) };
        assert_eq!(ui_shortcut(Key::F1, ctrl), expect);
        assert_eq!(ui_shortcut(Key::F1, Modifiers::NONE), None);
        assert_eq!(ui_shortcut(Key::F1, Modifiers { shift: true, ..ctrl }), None);
    }

    /// #1678: a Ctrl+F1 the user binds runs their command instead of collapsing the ribbon.
    #[test]
    fn a_bound_ctrl_f1_beats_the_ribbon_toggle() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.session.keymap = wordcraft_engine::keymap::KeyMap::from_value(&json!({"assigned": {"Mod+F1": "view.focus"}}));
        assert!(dispatch(&mut a, Key::F1, Modifiers { ctrl: true, command: true, ..Default::default() }));
        assert!(a.session.view.focus_mode && !a.ui.ribbon_collapsed);
    }

    /// #1678 review: only Ctrl+F1 gives way to a binding; a user's Alt+Q binding leaves Alt+Q
    /// opening the command search.
    #[test]
    fn a_bound_alt_q_still_opens_command_search() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.session.keymap = wordcraft_engine::keymap::KeyMap::from_value(&json!({"assigned": {"Alt+Q": "view.focus"}}));
        assert!(dispatch(&mut a, Key::Q, Modifiers { alt: true, ..Default::default() }));
        if cfg!(target_os = "macos") {
            // Option+Q is no interface shortcut on macOS, so the binding runs.
            assert!(a.session.view.focus_mode);
        } else {
            assert!(!a.session.view.focus_mode, "the binding doesn't run");
            assert!(matches!(a.dialog, Some(crate::dialogs::Dialog::Commands { .. })), "Alt+Q opens the command search");
        }
    }

    #[test]
    fn shift_backspace_still_erases() {
        assert_eq!(shift_variant_params("text.backspace"), Some(json!({})));
        assert_eq!(shift_variant_params("caret.left"), Some(json!({"extend": true})));
        assert_eq!(shift_variant_params("text.delete"), None);
    }

    #[test]
    fn alt_q_opens_command_search() {
        let m = |alt, shift, command| egui::Modifiers { alt, shift, command, ..Default::default() };
        let tell_me = Some(("ui.dialog", json!({"name": "commands"})));
        if cfg!(target_os = "macos") {
            assert_eq!(ui_shortcut(Key::Q, m(true, false, false)), None, "Option+Q types a character");
        } else {
            assert_eq!(ui_shortcut(Key::Q, m(true, false, false)), tell_me);
        }
        assert_eq!(ui_shortcut(Key::Q, m(true, true, false)), None, "Alt+Shift+Q");
        assert_eq!(ui_shortcut(Key::Q, m(true, false, true)), None, "Mod+Alt+Q");
        assert_eq!(ui_shortcut(Key::Q, m(false, false, false)), None, "a plain Q");
        assert_eq!(ui_shortcut(Key::W, m(true, false, false)), None);
    }

    #[test]
    fn cmd_option_r_collapses_the_ribbon_on_macos() {
        let m = |mac_cmd, alt, shift| egui::Modifiers { mac_cmd, command: mac_cmd, alt, shift, ..Default::default() };
        let collapse = Some(("ui.collapseRibbon", json!({})));
        assert_eq!(ui_shortcut(Key::R, m(true, true, false)), if cfg!(target_os = "macos") { collapse } else { None });
        assert_eq!(ui_shortcut(Key::R, m(true, false, false)), None, "Mod+R aligns right");
        assert_eq!(ui_shortcut(Key::R, m(false, true, false)), None, "Option+R types a character");
        assert_eq!(ui_shortcut(Key::R, m(true, true, true)), None, "Mod+Alt+Shift+R");
    }

    #[test]
    fn only_plain_paste_releases_paste_a_clipboard_picture() {
        let m = |command, shift, alt| egui::Modifiers { command, shift, alt, ..Default::default() };
        assert!(is_paste_release(Key::V, m(true, false, false)), "Mod+V");
        assert!(is_paste_release(Key::Paste, m(false, false, false)), "the Paste key");
        assert!(!is_paste_release(Key::V, m(true, true, false)), "Mod+Shift+V is Paste Formatting");
        assert!(!is_paste_release(Key::V, m(true, false, true)), "Mod+Alt+V is Paste Special");
        assert!(!is_paste_release(Key::V, m(false, false, false)), "a plain V");
        assert!(is_paste_special_release(Key::V, m(true, false, true)), "Mod+Alt+V opens Paste Special");
        assert!(!is_paste_special_release(Key::V, m(true, true, true)) && !is_paste_special_release(Key::V, m(true, false, false)));
    }

    /// An empty document whose clipboard holds a 4 × 4 picture.
    fn picture_app() -> WordApp {
        use crate::paste_picture::ClipboardPicture;
        let services = crate::Services {
            clipboard_picture: Some(Box::new(|| Some(ClipboardPicture::Pixels { width: 4, height: 4, rgba: vec![200; 4 * 4 * 4] }))),
            ..Default::default()
        };
        WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services)
    }

    /// How many pictures the document holds (the Selection Pane's list).
    fn pictures(a: &mut WordApp) -> usize {
        a.session.run("arrange.selectionPane", &json!({})).unwrap().as_array().map(|x| x.len()).unwrap_or(0)
    }

    /// One canvas frame with `modifiers` held, then `events`.
    fn mods_frame(ctx: &egui::Context, a: &mut WordApp, modifiers: Modifiers, events: Vec<egui::Event>) {
        let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
        ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| canvas_events(a, ui.ctx())).drop_without_applying_deltas();
    }

    /// A press or release of `key` with `modifiers`.
    fn key_event(key: Key, pressed: bool, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }
    }

    /// Left Ctrl+V with one hand often lets Ctrl up a moment before V. egui-winit keeps the
    /// Mod+V press, so the canvas sees only Ctrl's release, then V's without Mod: still a paste.
    #[test]
    fn mod_v_released_mod_first_pastes_the_clipboard_picture() {
        let ctx = egui::Context::default();
        let mut a = picture_app();
        let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
        mods_frame(&ctx, &mut a, ctrl, vec![key_event(Key::ControlLeft, true, ctrl)]);
        // The Mod+V press: egui-winit sends no event for it.
        mods_frame(&ctx, &mut a, ctrl, vec![]);
        mods_frame(&ctx, &mut a, Modifiers::NONE, vec![key_event(Key::ControlLeft, false, Modifiers::NONE)]);
        mods_frame(&ctx, &mut a, Modifiers::NONE, vec![key_event(Key::V, false, Modifiers::NONE)]);
        assert_eq!(pictures(&mut a), 1);

        // A plain V afterwards (its press arrives) isn't a paste.
        mods_frame(&ctx, &mut a, Modifiers::NONE, vec![key_event(Key::V, true, Modifiers::NONE)]);
        mods_frame(&ctx, &mut a, Modifiers::NONE, vec![key_event(Key::V, false, Modifiers::NONE)]);
        assert_eq!(pictures(&mut a), 1);

        // Nor is Ctrl+B, Ctrl up, then a V release whose press never came (focus moved in).
        mods_frame(&ctx, &mut a, ctrl, vec![key_event(Key::B, true, ctrl)]);
        mods_frame(&ctx, &mut a, Modifiers::NONE, vec![key_event(Key::ControlLeft, false, Modifiers::NONE), egui::Event::WindowFocused(true)]);
        mods_frame(&ctx, &mut a, Modifiers::NONE, vec![key_event(Key::V, false, Modifiers::NONE)]);
        assert_eq!(pictures(&mut a), 1);

        // Mod+Shift+V released Mod first stays Paste Formatting, not a picture paste.
        let ctrl_shift = Modifiers { shift: true, ..ctrl };
        mods_frame(&ctx, &mut a, ctrl_shift, vec![]);
        mods_frame(&ctx, &mut a, Modifiers::SHIFT, vec![key_event(Key::ControlLeft, false, Modifiers::SHIFT)]);
        mods_frame(&ctx, &mut a, Modifiers::SHIFT, vec![key_event(Key::V, false, Modifiers::SHIFT)]);
        assert_eq!(pictures(&mut a), 1);

        // Released V first, as before, still pastes.
        let mut a = picture_app();
        mods_frame(&ctx, &mut a, ctrl, vec![]);
        mods_frame(&ctx, &mut a, ctrl, vec![key_event(Key::V, false, ctrl)]);
        assert_eq!(pictures(&mut a), 1);
    }

    /// Ctrl down, the kept Mod+V press, Ctrl up and V up can all arrive in one frame: the frame's
    /// modifiers no longer have Mod, but the ModifiersChanged events in between did.
    #[test]
    fn mod_v_within_one_frame_pastes_the_clipboard_picture() {
        let ctx = egui::Context::default();
        let mut a = picture_app();
        let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
        let events = vec![
            egui::Event::ModifiersChanged(ctrl),
            key_event(Key::ControlLeft, true, ctrl),
            egui::Event::ModifiersChanged(Modifiers::NONE),
            key_event(Key::ControlLeft, false, Modifiers::NONE),
            key_event(Key::V, false, Modifiers::NONE),
        ];
        ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| canvas_events(&mut a, ui.ctx())).drop_without_applying_deltas();
        assert_eq!(pictures(&mut a), 1);
    }

    /// A V release keeps its own modifiers unless Mod went up first and no V press arrived.
    #[test]
    fn paste_release_modifiers_fall_back_to_those_held_with_mod() {
        let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
        assert_eq!(paste_release_modifiers(ctrl, false, None), ctrl, "Mod still down");
        assert_eq!(paste_release_modifiers(Modifiers::NONE, false, Some(ctrl)), ctrl, "Mod up first, press kept by egui-winit");
        assert_eq!(paste_release_modifiers(Modifiers::NONE, true, Some(ctrl)), Modifiers::NONE, "a plain V's press arrived");
        assert_eq!(paste_release_modifiers(Modifiers::NONE, false, None), Modifiers::NONE);
    }

    #[test]
    fn shift_delete_is_only_shift_and_delete() {
        let m = |shift, ctrl, alt| egui::Modifiers { shift, ctrl, command: ctrl, alt, ..Default::default() };
        assert!(is_shift_delete(Key::Delete, m(true, false, false)));
        assert!(!is_shift_delete(Key::Delete, m(false, false, false)), "Delete alone erases");
        assert!(!is_shift_delete(Key::Delete, m(true, true, false)), "Ctrl+Shift+Delete");
        assert!(!is_shift_delete(Key::Delete, m(true, false, true)), "Alt+Shift+Delete");
        assert!(!is_shift_delete(Key::Backspace, m(true, false, false)), "Shift+Backspace erases (#707)");
    }

    #[test]
    fn shift_f10_is_the_context_menu_key() {
        let m = |shift, ctrl, alt| egui::Modifiers { shift, ctrl, command: ctrl, alt, ..Default::default() };
        assert!(is_context_menu_key(Key::F10, m(true, false, false)));
        assert!(!is_context_menu_key(Key::F10, m(false, false, false)), "F10 alone shows keytips");
        assert!(!is_context_menu_key(Key::F10, m(true, true, false)), "Ctrl+Shift+F10");
        assert!(!is_context_menu_key(Key::F9, m(true, false, false)), "Shift+F9 is a field key");
    }

    #[test]
    fn alt_shortcuts_work_without_the_canvas_focused() {
        let m = |command, alt, shift| egui::Modifiers { command, alt, shift, ..Default::default() };
        assert!(is_global_shortcut(Key::Q, m(false, true, false)), "Alt+Q (#819)");
        assert!(is_global_shortcut(Key::D, m(false, true, true)), "Alt+Shift+D");
        assert!(is_global_shortcut(Key::B, m(true, false, false)), "Mod+B");
        assert!(is_global_shortcut(Key::F7, m(false, false, false)), "F7");
        assert!(!is_global_shortcut(Key::Q, m(false, false, false)), "a plain letter is typing");
        assert!(!is_global_shortcut(Key::Q, m(false, false, true)), "Shift+letter is typing");
    }

    /// Alt held, keypad 0 2 3 3, Alt released: é (й on a Russian system) is typed, the digits
    /// aren't, and keytips stay off (#961). Linux sends each digit's text too; Windows may send the
    /// composed character.
    #[test]
    fn alt_numpad_code_types_its_character_without_keytips() {
        if cfg!(target_os = "macos") {
            return;
        }
        let doc_text = |a: &WordApp| a.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
        let ctx = egui::Context::default();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        // Windows reads codes in the system locale's code pages (0233 is é in 1252 and 1250, й in
        // 1251); elsewhere the ANSI page is 1252 and the OEM page the locale's own. The app records
        // the locale when it starts, so ask after `WordApp::new`.
        let (ansi, oem_page) = if cfg!(target_os = "windows") {
            wordcraft_engine::cmd::text::system_code_pages()
        } else {
            (1252, wordcraft_engine::cmd::text::system_oem_code_page())
        };
        let typed = wordcraft_engine::cmd::text::alt_code_char("0233", ansi, oem_page).unwrap_or('?');
        let alt = Modifiers { alt: true, ..Default::default() };
        let key = |key, pressed, modifiers| egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers };
        let frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                crate::keytips::logic(a, ui.ctx());
                canvas_events(a, ui.ctx());
            })
            .drop_without_applying_deltas();
        };
        frame(&mut a, vec![egui::Event::ModifiersChanged(alt), key(Key::AltLeft, true, alt)]);
        for (k, t) in [(Key::Num0, "0"), (Key::Num2, "2"), (Key::Num3, "3"), (Key::Num3, "3")] {
            frame(&mut a, vec![key(k, true, alt), egui::Event::Text(t.into()), key(k, false, alt)]);
        }
        assert_eq!(doc_text(&a).trim_end(), "", "nothing typed while Alt is held");
        frame(&mut a, vec![key(Key::AltLeft, false, alt), egui::Event::ModifiersChanged(Modifiers::NONE)]);
        assert_eq!(doc_text(&a).trim_end(), typed.to_string());
        assert_eq!(a.ui.keytips, crate::keytips::Phase::Off, "an Alt code doesn't open keytips");
        // The platform's own composed character isn't typed twice; the next typing is.
        frame(&mut a, vec![egui::Event::Text(typed.to_string())]);
        frame(&mut a, vec![key(Key::E, true, Modifiers::NONE), egui::Event::Text("é".into())]);
        assert_eq!(doc_text(&a).trim_end(), format!("{typed}é"));
        // Without the leading zero: the system locale's OEM code page (233 is Θ in 437, Ú in 850).
        frame(&mut a, vec![egui::Event::ModifiersChanged(alt), key(Key::AltLeft, true, alt)]);
        for k in [Key::Num2, Key::Num3, Key::Num3] {
            frame(&mut a, vec![key(k, true, alt), key(k, false, alt)]);
        }
        frame(&mut a, vec![key(Key::AltLeft, false, alt), egui::Event::ModifiersChanged(Modifiers::NONE)]);
        let oem = wordcraft_engine::cmd::text::alt_code_char("233", ansi, oem_page);
        assert_eq!(doc_text(&a).trim_end(), format!("{typed}é{}", oem.unwrap_or('?')));
        // A bare Alt tap still opens keytips.
        frame(&mut a, vec![egui::Event::ModifiersChanged(alt), key(Key::AltLeft, true, alt)]);
        frame(&mut a, vec![key(Key::AltLeft, false, alt), egui::Event::ModifiersChanged(Modifiers::NONE)]);
        assert_eq!(a.ui.keytips, crate::keytips::Phase::Tabs);
    }

    /// #2388: AltGr (Right Alt on a Polish layout under Wayland) arrives as the Right Alt key
    /// followed by a modifiers event without Alt (it's Level 3 Shift). That event read as Alt
    /// let go opened keytips while AltGr was still held, and the next letter went to them
    /// instead of typing ś, ł, ż. A bare AltGr tap doesn't open keytips either; a Right Alt
    /// that is Alt (US layout) still does.
    #[test]
    fn altgr_types_its_characters_without_keytips() {
        if cfg!(target_os = "macos") {
            return;
        }
        let doc_text = |a: &WordApp| a.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
        let ctx = egui::Context::default();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let none = Modifiers::NONE;
        let key = |key, pressed, modifiers| egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers };
        let frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                crate::keytips::logic(a, ui.ctx());
                canvas_events(a, ui.ctx());
            })
            .drop_without_applying_deltas();
        };
        let altgr_down = || vec![key(Key::AltRight, true, none), egui::Event::ModifiersChanged(none)];
        let altgr_up = || vec![key(Key::AltRight, false, none), egui::Event::ModifiersChanged(none)];
        // Held through three letters, one frame per event batch.
        frame(&mut a, altgr_down());
        assert_eq!(a.ui.keytips, crate::keytips::Phase::Off, "AltGr down doesn't open keytips");
        for (k, t) in [(Key::S, "ś"), (Key::L, "ł"), (Key::Z, "ż")] {
            frame(&mut a, vec![key(k, true, none), egui::Event::Text(t.into())]);
            frame(&mut a, vec![key(k, false, none)]);
            assert_eq!(a.ui.keytips, crate::keytips::Phase::Off, "{t}");
        }
        frame(&mut a, altgr_up());
        assert_eq!(doc_text(&a).trim_end(), "śłż");
        assert_eq!(a.ui.keytips, crate::keytips::Phase::Off);
        // All in one frame.
        frame(&mut a, [altgr_down(), vec![key(Key::A, true, none), egui::Event::Text("ą".into()), key(Key::A, false, none)], altgr_up()].concat());
        assert_eq!(doc_text(&a).trim_end(), "śłżą");
        assert_eq!(a.ui.keytips, crate::keytips::Phase::Off);
        // A bare AltGr tap, in two frames or one.
        frame(&mut a, altgr_down());
        frame(&mut a, altgr_up());
        assert_eq!(a.ui.keytips, crate::keytips::Phase::Off, "an AltGr tap isn't an Alt tap");
        frame(&mut a, [altgr_down(), altgr_up()].concat());
        assert_eq!(a.ui.keytips, crate::keytips::Phase::Off);
        // Right Alt that is Alt still opens keytips.
        let alt = Modifiers { alt: true, ..Default::default() };
        frame(&mut a, vec![key(Key::AltRight, true, none), egui::Event::ModifiersChanged(alt)]);
        frame(&mut a, vec![key(Key::AltRight, false, alt), egui::Event::ModifiersChanged(none)]);
        assert_eq!(a.ui.keytips, crate::keytips::Phase::Tabs);
    }

    fn alt_frame_app() -> (egui::Context, WordApp) {
        let a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        (egui::Context::default(), a)
    }

    fn alt_frame(ctx: &egui::Context, a: &mut WordApp, events: Vec<egui::Event>) {
        ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| canvas_events(a, ui.ctx())).drop_without_applying_deltas();
    }

    fn alt_key(key: Key, pressed: bool, repeat: bool) -> egui::Event {
        egui::Event::Key { key, physical_key: Some(key), pressed, repeat, modifiers: Modifiers { alt: true, ..Default::default() } }
    }

    /// Option+Shift+- types an em dash outside Outline view, where Word's Alt+Shift+- (Collapse)
    /// can't run, so its shortcut doesn't swallow the character (#1933).
    #[test]
    fn disabled_alt_shortcut_leaves_the_typed_character() {
        let (ctx, mut a) = alt_frame_app();
        let alt_shift = Modifiers { alt: true, shift: true, ..Default::default() };
        let key = |pressed| egui::Event::Key { key: Key::Minus, physical_key: Some(Key::Minus), pressed, repeat: false, modifiers: alt_shift };
        alt_frame(&ctx, &mut a, vec![key(true), egui::Event::Text("\u{2014}".into()), key(false)]);
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "\u{2014}");
    }

    /// Holding a digit with Alt adds it once: its auto-repeat presses don't (#961).
    #[test]
    fn alt_code_ignores_auto_repeat() {
        if cfg!(target_os = "macos") {
            return;
        }
        let (ctx, mut a) = alt_frame_app();
        alt_frame(&ctx, &mut a, vec![alt_key(Key::Num6, true, false), alt_key(Key::Num6, true, true), alt_key(Key::Num6, true, true)]);
        alt_frame(&ctx, &mut a, vec![alt_key(Key::Num5, true, false)]);
        assert_eq!(a.canvas.alt_code, "65");
        alt_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(Modifiers::NONE)]);
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "A");
    }

    /// Switching to another window mid-code drops the digits: releasing Alt later types nothing.
    #[test]
    fn alt_code_is_dropped_when_the_window_loses_focus() {
        if cfg!(target_os = "macos") {
            return;
        }
        let (ctx, mut a) = alt_frame_app();
        alt_frame(&ctx, &mut a, vec![alt_key(Key::Num6, true, false), alt_key(Key::Num5, true, false)]);
        alt_frame(&ctx, &mut a, vec![egui::Event::WindowFocused(false)]);
        assert_eq!(a.canvas.alt_code, "");
        alt_frame(&ctx, &mut a, vec![egui::Event::WindowFocused(true), egui::Event::ModifiersChanged(Modifiers::NONE)]);
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "");
    }

    /// Windows composes Alt codes with the system's code pages: for a code up to 255 its
    /// character replaces ours (Polish Alt+0185 is ą, not ¹); above 255 Windows reduces the code
    /// to a byte, so ours stays and its text isn't typed (#961).
    #[test]
    fn system_composed_alt_code_character_wins_up_to_255() {
        let (ctx, mut a) = alt_frame_app();
        let text = |a: &WordApp| a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end().to_string();
        let _ = a.run("text.typeCode", json!({"code": "0185", "ansi": 1252}));
        a.canvas.alt_code_echo = Some(AltCodeEcho { typed: "¹".into(), system_composes: true, byte_code: true });
        alt_frame(&ctx, &mut a, vec![egui::Event::Text("ą".into())]);
        assert_eq!(text(&a), "ą");
        let _ = a.run("text.typeCode", json!({"code": "8364"}));
        a.canvas.alt_code_echo = Some(AltCodeEcho { typed: "€".into(), system_composes: true, byte_code: false });
        alt_frame(&ctx, &mut a, vec![egui::Event::Text("|".into())]);
        assert_eq!(text(&a), "ą€");
        // Elsewhere a different text after the code is ordinary typing.
        a.canvas.alt_code_echo = Some(AltCodeEcho { typed: "€".into(), system_composes: false, byte_code: false });
        alt_frame(&ctx, &mut a, vec![egui::Event::Text("x".into())]);
        assert_eq!(text(&a), "ą€x");
    }

    fn body(a: &WordApp) -> String {
        a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end_matches(['\r', '\n']).to_string()
    }

    fn surround(before_chars: usize, after_chars: usize) -> egui::Event {
        egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars, after_chars })
    }

    /// A real body replacement invalidates the old preedit even if its text is unchanged.
    /// Its queued replacement must not edit the new body or add an Undo checkpoint.
    #[test]
    fn ime_interrupted_body_preedit_drops_queued_replacement_before_mutation() {
        for replacement in [
            vec![egui::Event::Ime(egui::ImeEvent::Commit("stale".into()))],
            vec![surround(1, 1), egui::Event::Ime(egui::ImeEvent::Commit("stale".into()))],
            vec![surround_bytes(1, 1), egui::Event::Ime(egui::ImeEvent::Commit("stale".into()))],
            vec![surround_bytes(1, 1), egui::Event::Text("stale".into()), egui::Event::Ime(egui::ImeEvent::Commit("stale".into()))],
        ] {
            let (ctx, mut a) = alt_frame_app();
            a.run("document.setText", json!({"text": "ABCDE"})).unwrap();
            a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 2));
            alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: "に".into(), active_range_chars: None })]);
            assert_eq!(a.canvas.ime_preedit, "に");
            a.run("document.setText", json!({"text": "ABCDE"})).unwrap();
            a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 2));
            let depth = a.session.undo_depth();
            alt_frame(&ctx, &mut a, replacement);
            assert_eq!(body(&a), "ABCDE", "queued events cannot replace text in the new body");
            assert_eq!(a.session.sel.focus.off, 2, "not even the new caret moves");
            assert_eq!(a.session.undo_depth(), depth, "rejection happens before any transaction");
            assert!(a.canvas.ime_preedit.is_empty());
            assert!(a.canvas.ime_interrupt_now);
        }
    }

    /// Exiting the equation through a real key in the same raw batch invalidates its preedit
    /// before the following body deletion/commit, as does an external target change.
    #[test]
    fn ime_interrupted_equation_preedit_drops_queued_body_replacement() {
        for in_batch in [false, true] {
            let (ctx, mut a) = alt_frame_app();
            a.run("insert.equation", json!({"linear": "abc"})).unwrap();
            alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: "に".into(), active_range_chars: None })]);
            assert_eq!(a.canvas.ime_preedit, "に");
            let mut events = Vec::new();
            if in_batch {
                events.push(egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
            } else {
                a.run("equation.exit", json!({})).unwrap();
            }
            let before = body(&a);
            let depth = a.session.undo_depth();
            events.extend([surround(1, 0), egui::Event::Ime(egui::ImeEvent::Commit("stale".into()))]);
            alt_frame(&ctx, &mut a, events);
            assert!(a.session.math.is_none(), "the equation exit itself still takes effect");
            assert_eq!(body(&a), before, "the equation's queued replacement cannot edit the body");
            assert_eq!(a.session.undo_depth(), depth, "neither rejected event creates an Undo step");
        }
    }

    /// Untagged commits are legitimate without an interrupted composition, after a fresh
    /// preedit, after the one stale replacement, and after an empty intervening batch.
    #[test]
    fn ime_interrupted_preedit_does_not_suppress_fresh_input() {
        for fresh_case in 0..4 {
            let (ctx, mut a) = alt_frame_app();
            a.run("document.setText", json!({"text": "ABCDE"})).unwrap();
            if fresh_case != 0 {
                alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: "old".into(), active_range_chars: None })]);
            }
            a.run("document.setText", json!({"text": "ABCDE"})).unwrap();
            a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 2));
            let depth = a.session.undo_depth();
            let mut events = Vec::new();
            match fresh_case {
                1 => events.push(egui::Event::Ime(egui::ImeEvent::Preedit { text: "fresh".into(), active_range_chars: None })),
                2 => events.push(egui::Event::Ime(egui::ImeEvent::Commit("stale".into()))),
                3 => alt_frame(&ctx, &mut a, vec![]),
                _ => {}
            }
            events.push(egui::Event::Ime(egui::ImeEvent::Commit("X".into())));
            alt_frame(&ctx, &mut a, events);
            assert_eq!(body(&a), "ABXCDE", "fresh case {fresh_case} must remain eligible");
            assert_eq!(a.session.undo_depth(), depth + 1);
            a.run("edit.undo", json!({})).unwrap();
            assert_eq!(body(&a), "ABCDE");
        }
    }

    /// The web's text agent keeps what it typed and, when its value changes other than by an
    /// addition at the end, sends `DeleteSurrounding` for the changed tail and then the new text
    /// (#1480). The document follows the agent's value instead of repeating it.
    #[test]
    fn delete_surrounding_from_the_web_text_agent_does_not_repeat_text() {
        let (ctx, mut a) = alt_frame_app();
        // The trace from #1480: the agent's caret jumped to 0 after `м`, so it reported `иabм`
        // and `риabм` as its whole new tail.
        alt_frame(&ctx, &mut a, vec![egui::Event::Text("a".into())]);
        alt_frame(&ctx, &mut a, vec![egui::Event::Text("b".into())]);
        alt_frame(&ctx, &mut a, vec![egui::Event::Text("м".into())]);
        alt_frame(&ctx, &mut a, vec![surround(3, 0), egui::Event::Text("иabм".into())]);
        alt_frame(&ctx, &mut a, vec![surround(4, 0), egui::Event::Text("риabм".into())]);
        assert_eq!(body(&a), "риabм");
        // A correction (`teh` -> `the`): erase three characters, type the replacement.
        alt_frame(&ctx, &mut a, vec![egui::Event::Text(" teh".into())]);
        alt_frame(&ctx, &mut a, vec![surround(3, 0), egui::Event::Text("the".into())]);
        assert_eq!(body(&a), "риabм the");
        // A plain deletion (Android Backspace): no text follows, or an empty one.
        alt_frame(&ctx, &mut a, vec![surround(1, 0), egui::Event::Text(String::new())]);
        assert_eq!(body(&a), "риabм th");
        alt_frame(&ctx, &mut a, vec![surround(2, 0)]);
        assert_eq!(body(&a), "риabм ");
    }

    /// Builds an `ImeEvent::DeleteSurroundingBytes` event with the given before and after byte counts.
    fn surround_bytes(before_bytes: usize, after_bytes: usize) -> egui::Event {
        egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes, after_bytes })
    }

    /// macOS and Wayland IME replacement counts UTF-8 bytes and must delete exactly that range.
    /// A byte count into the middle of a character is refused rather than slicing the string.
    #[test]
    fn delete_surrounding_bytes_replaces_exact_utf8_and_joins_one_undo_step() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "aé👍b"})).unwrap();
        // The caret is before `b`, after the four-byte emoji.
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 7));
        alt_frame(&ctx, &mut a, vec![surround_bytes(4, 0), egui::Event::Text("x".into())]);
        assert_eq!(body(&a), "aéxb");
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body(&a), "aé👍b", "delete plus commit is one undo step");

        // A range that starts inside the emoji is not a UTF-8 boundary: nothing changes.
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 7));
        alt_frame(&ctx, &mut a, vec![surround_bytes(3, 0)]);
        assert_eq!(body(&a), "aé👍b");
        // Beyond the paragraph is refused too, and never joins the next paragraph.
        alt_frame(&ctx, &mut a, vec![surround_bytes(0, 99)]);
        assert_eq!(body(&a), "aé👍b");
        let depth = a.session.undo_depth();
        for (before, after) in [(99, 0), (usize::MAX, 0), (0, usize::MAX)] {
            alt_frame(&ctx, &mut a, vec![surround_bytes(before, after)]);
            assert_eq!(body(&a), "aé👍b", "out-of-range counts must not erase a prefix");
            assert_eq!(a.session.undo_depth(), depth);
        }
    }

    /// A combining accent is its own UTF-8 range; replacing it does not erase the base letter.
    /// `document.setText` leaves the caret at the start, so the test moves it past `e` + accent.
    #[test]
    fn delete_surrounding_bytes_replaces_a_combining_mark_and_accepts_empty_commit() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "e\u{301}"})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, "e\u{301}".len()));
        alt_frame(&ctx, &mut a, vec![surround_bytes(2, 0), egui::Event::Text(String::new())]);
        assert_eq!(body(&a), "e");
        // A deletion-only batch and a 0/0 batch stay harmless.
        alt_frame(&ctx, &mut a, vec![surround_bytes(1, 0)]);
        assert_eq!(body(&a), "");
        alt_frame(&ctx, &mut a, vec![surround_bytes(0, 0)]);
        assert_eq!(body(&a), "");
    }

    /// The real macOS/Wayland path is `ImeEvent::Commit`, not `Event::Text`: the byte deletion
    /// and the committed replacement are one edit, and Undo restores the original text.
    #[test]
    fn ime_commit_replaces_delete_surrounding_bytes_in_one_undo_step() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "e\u{301}"})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, "e\u{301}".len()));
        alt_frame(&ctx, &mut a, vec![surround_bytes("e\u{301}".len(), 0), egui::Event::Ime(egui::ImeEvent::Commit("é".into()))]);
        assert_eq!(body(&a), "é", "the combining sequence is replaced by the IME's committed text");
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body(&a), "e\u{301}", "delete plus Commit is one undo step");
    }

    /// A platform may deliver the `Commit` that replaces a `DeleteSurrounding` in a later raw
    /// batch than the deletion. The undo join belongs to the deletion transaction, not to the
    /// frame: the commit still shares its checkpoint, so one Undo restores the original input.
    #[test]
    fn ime_commit_in_a_later_frame_joins_the_delete_surrounding_undo_step() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "teh"})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 3));
        let depth = a.session.undo_depth();

        alt_frame(&ctx, &mut a, vec![surround_bytes(3, 0)]);
        assert_eq!(body(&a), "");
        assert_eq!(a.session.undo_depth(), depth + 1, "the deletion is one step");

        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("the".into()))]);
        assert_eq!(body(&a), "the", "the later commit replaces the erased text");
        assert_eq!(a.session.undo_depth(), depth + 1, "the later commit joins the deletion's checkpoint");
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body(&a), "teh", "one Undo restores the original input");
    }

    /// An accepted IME commit can also be empty: it finishes composition without adding text.
    #[test]
    fn ime_empty_commit_finishes_without_inserting() {
        let (ctx, mut a) = alt_frame_app();
        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit(String::new()))]);
        assert_eq!(body(&a), "");
    }

    /// `after_chars` erases after the caret; both counts are characters, not bytes.
    #[test]
    fn delete_surrounding_erases_characters_on_both_sides_of_the_caret() {
        let (ctx, mut a) = alt_frame_app();
        a.run("text.insert", json!({"text": "мир дом"})).unwrap();
        for _ in 0..3 {
            a.run("caret.left", json!({})).unwrap();
        }
        // The caret is after `мир `: two before (`р `) and one after (`д`).
        alt_frame(&ctx, &mut a, vec![surround(2, 1)]);
        assert_eq!(body(&a), "миом");
        alt_frame(&ctx, &mut a, vec![egui::Event::Text("X".into())]);
        assert_eq!(body(&a), "миXом", "the caret stays where the deletion was");
    }

    /// The counts come from outside: larger than the paragraph they stop at its ends, never
    /// joining paragraphs or panicking.
    #[test]
    fn delete_surrounding_stays_inside_the_paragraph() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "one\ntwo\nthree"})).unwrap();
        a.run("caret.docStart", json!({})).unwrap();
        // After the `t` of `two`.
        for _ in 0..5 {
            a.run("caret.right", json!({})).unwrap();
        }
        alt_frame(&ctx, &mut a, vec![surround(usize::MAX, usize::MAX)]);
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "one\n\nthree");
        alt_frame(&ctx, &mut a, vec![surround(5, 5), egui::Event::Text("2".into())]);
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "one\n2\nthree");
    }

    /// The agent's rewrite belongs to the typing it corrects: one Undo takes back the typing
    /// and its correction together, not a half-erased word.
    #[test]
    fn delete_surrounding_joins_the_typing_undo_step() {
        let (ctx, mut a) = alt_frame_app();
        alt_frame(&ctx, &mut a, vec![egui::Event::Text("teh".into())]);
        alt_frame(&ctx, &mut a, vec![surround(3, 0), egui::Event::Text("the".into())]);
        assert_eq!(body(&a), "the");
        // Typing goes on in the same group, not one step per keystroke.
        alt_frame(&ctx, &mut a, vec![egui::Event::Text(" more".into())]);
        alt_frame(&ctx, &mut a, vec![surround(4, 0), egui::Event::Text("mores".into())]);
        alt_frame(&ctx, &mut a, vec![egui::Event::Text("t".into())]);
        assert_eq!(body(&a), "the morest");
        assert_eq!(a.session.undo_label(), Some("Typing"));
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body(&a), "");
    }

    /// The rewritten text keeps the formatting of the text it replaces: a bold `helo` corrected
    /// to `hello` stays bold, as typing over a word does.
    #[test]
    fn delete_surrounding_keeps_the_replaced_texts_formatting() {
        let (ctx, mut a) = alt_frame_app();
        a.run("text.insert", json!({"text": "plain "})).unwrap();
        a.run("format.bold", json!({})).unwrap();
        alt_frame(&ctx, &mut a, vec![egui::Event::Text("helo".into())]);
        alt_frame(&ctx, &mut a, vec![surround(4, 0), egui::Event::Text("hello".into())]);
        assert_eq!(body(&a), "plain hello");
        let f = a.session.sel.focus.clone();
        let para = a.session.doc.para_at(&f).expect("a paragraph");
        let bold: Vec<bool> = para.text.char_indices().map(|(i, _)| para.props_of_char(i).bold == Some(true)).collect();
        assert_eq!(bold, [false, false, false, false, false, false, true, true, true, true, true], "{bold:?}");
    }

    /// Overtype replaces characters after the caret when typing, but the text an input method
    /// sends to replace what it erased only replaces that.
    #[test]
    fn delete_surrounding_text_does_not_overtype() {
        let (ctx, mut a) = alt_frame_app();
        a.run("text.insert", json!({"text": "texXY"})).unwrap();
        for _ in 0..2 {
            a.run("caret.left", json!({})).unwrap();
        }
        a.run("file.options", json!({"overtype": true})).unwrap();
        alt_frame(&ctx, &mut a, vec![surround(3, 0), egui::Event::Text("Texas".into())]);
        assert_eq!(body(&a), "TexasXY");
    }

    /// A key pressed between the erasure and the text ends the input method's rewrite: the text
    /// is ordinary typing, its own undo step.
    #[test]
    fn delete_surrounding_rewrite_ends_at_a_key_press() {
        let (ctx, mut a) = alt_frame_app();
        alt_frame(&ctx, &mut a, vec![egui::Event::Text("ab".into())]);
        let key = egui::Event::Key { key: Key::F20, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        alt_frame(&ctx, &mut a, vec![surround(1, 0), key, egui::Event::Text("X".into())]);
        assert_eq!(body(&a), "aX");
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body(&a), "a");
    }

    /// An automatic change while typing (AutoCorrect `(c)` -> `©`) changes the text behind the
    /// web's text agent, so its later counts would be wrong: the page asks it to start afresh.
    #[test]
    fn automatic_change_asks_the_input_method_to_start_afresh() {
        let (ctx, mut a) = alt_frame_app();
        alt_frame(&ctx, &mut a, vec![egui::Event::Text("Word".into())]);
        alt_frame(&ctx, &mut a, vec![egui::Event::Text(" ".into())]);
        assert!(!a.canvas.ime_reset, "plain typing keeps the agent's text");
        alt_frame(&ctx, &mut a, vec![egui::Event::Text("(c)".into())]);
        alt_frame(&ctx, &mut a, vec![egui::Event::Text(" ".into())]);
        assert_eq!(body(&a), "Word \u{a9} ");
        assert!(a.canvas.ime_reset, "AutoCorrect changed the text");
    }

    /// Both sides of one `DeleteSurrounding` are one request: when the second `text.backspace`
    /// is refused (here by a content control locked against editing), the first half is rolled
    /// back and no rewrite or receipt is promised.
    #[test]
    fn a_refused_second_delete_surrounding_rolls_the_request_back() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "ab"})).unwrap();
        // Wrap `b` in a rich-text control that refuses edits, leaving `a` editable.
        a.session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(0, 2) };
        a.run("developer.richText", json!({"tag": "locked"})).unwrap();
        a.run("developer.properties", json!({"control": "locked", "lockEdit": true})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 1));
        let depth = a.session.undo_depth();
        let pending = wordcraft_doc::props::CharProps { bold: Some(true), ..Default::default() };
        a.session.pending = Some(pending.clone());
        a.session.goal_x = Some(123.0);
        alt_frame(&ctx, &mut a, vec![surround(1, 1)]);
        assert_eq!(body(&a), "ab", "the applied left side is rolled back with the refused right one");
        assert_eq!(a.session.undo_depth(), depth, "no half-erasure is left in Undo");
        assert!(a.canvas.ime_join_text.is_none(), "no text rewrite is promised");
        assert!(a.canvas.ime_interrupt_now, "the platform's stale surrounding snapshot is discarded");
        assert_eq!(a.session.sel.focus.off, 1, "the caret is back where the request found it");
        assert_eq!(a.session.pending, Some(pending), "pending formatting is restored");
        assert_eq!(a.session.goal_x, Some(123.0), "the Up/Down goal column is restored");
    }

    /// A successful body `DeleteSurrounding` followed in the same raw batch by a commit refused
    /// by the locked content at the caret must leave no trace but the deletion: the replacement's
    /// pending formatting is rolled back, the composition is interrupted, no undo step or
    /// fabricated AutoFormat change is recorded, and the caret stays at the deletion.
    ///
    /// Forms mode makes the `za` control fillable and everything outside it protected. Deleting
    /// its final `a` is allowed; the following commit starts at the locked `X` control's content
    /// start, which typing moves beside the control (as Word does at a content-control edge) and
    /// into protected text, where Restrict Editing refuses it. The deletion therefore stands and
    /// the replacement does not.
    #[test]
    fn a_refused_ime_commit_after_a_successful_delete_restores_pending_and_interrupts() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "zaXY"})).unwrap();
        a.session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 0), focus: wordcraft_doc::Pos::body(0, 2) };
        a.run("developer.richText", json!({"tag": "fill"})).unwrap();
        a.run("select.text", json!({"text": "X"})).unwrap();
        a.run("developer.richText", json!({"tag": "locked"})).unwrap();
        a.run("developer.properties", json!({"control": "locked", "lockEdit": true})).unwrap();
        a.run("review.restrict", json!({"mode": "forms"})).unwrap();
        let locked =
            a.session.doc.control_ranges(wordcraft_doc::StoryRef::Body).into_iter().find(|r| r.control.tag == "locked").expect("the locked control");
        a.session.sel = wordcraft_engine::Selection::caret(locked.content_start());
        a.session.pending = None;
        a.canvas.ime_reset = false;
        let _ = a.session.take_pending_ime_application_change();
        let depth = a.session.undo_depth();
        alt_frame(&ctx, &mut a, vec![surround(1, 0), egui::Event::Ime(egui::ImeEvent::Commit("\"".into()))]);
        assert_eq!(body(&a), "zXY", "the locked replacement is refused, the earlier deletion stands");
        assert_eq!(a.session.undo_depth(), depth + 1, "only the successful deletion added an undo step");
        let locked = a
            .session
            .doc
            .control_ranges(wordcraft_doc::StoryRef::Body)
            .into_iter()
            .find(|r| r.control.tag == "locked")
            .expect("the locked control remains");
        assert_eq!(a.session.sel.focus, locked.content_start(), "the caret stays where the deletion left it");
        assert_eq!(a.session.sel.anchor, locked.content_start());
        assert_eq!(a.session.pending, None, "the refused replacement leaves no pending formatting from the erased text");
        assert!(a.canvas.ime_join_text.is_none(), "the refused replacement promises no rewrite");
        assert!(a.canvas.ime_interrupt_now, "the platform's stale surrounding snapshot is interrupted");
        assert!(!a.canvas.ime_reset, "a refused commit is not an AutoFormat change");
        assert!(!a.session.take_pending_ime_application_change(), "no application-origin receipt is fabricated");
    }

    /// Track Changes can remove the author's own insertion from the stored text. The right-hand
    /// range and the final caret then move left by what was actually removed: the second side
    /// must erase the character the platform named, not the one that took its stored offset.
    #[test]
    fn tracked_delete_surrounding_shifts_past_its_own_insertion() {
        let (ctx, mut a) = alt_frame_app();
        a.run("review.trackChanges", json!({"value": true})).unwrap();
        a.run("text.insert", json!({"text": "abXY"})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 2));
        alt_frame(&ctx, &mut a, vec![surround(1, 1)]);
        assert_eq!(body(&a), "aY", "the own insertion `b` and the following `X` are erased");
        assert_eq!(a.session.sel.focus.off, 1, "the caret points after the stored deletion");
        assert_eq!(a.session.sel.anchor.off, 1);
    }

    /// Track Changes plus a refusal: the first side really removes this author's own insertion
    /// from the stored paragraph, then a locked control refuses the second side. The whole
    /// request is rolled back, including the shifted caret and undo stack.
    #[test]
    fn tracked_own_insertion_then_a_refused_side_rolls_the_whole_request_back() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "aXY"})).unwrap();
        // Lock the following `X`; inserting `b` before it under Track Changes is this author's own
        // insertion, which a tracked delete removes from the stored text instead of marking.
        a.session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(0, 2) };
        a.run("developer.richText", json!({"tag": "locked"})).unwrap();
        a.run("developer.properties", json!({"control": "locked", "lockEdit": true})).unwrap();
        a.run("review.trackChanges", json!({"value": true})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 1));
        a.run("text.insert", json!({"text": "b"})).unwrap();
        assert_eq!(body(&a), "abXY");
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 2));
        let depth = a.session.undo_depth();
        let revisions = a.session.doc.revisions.len();
        let pending = wordcraft_doc::props::CharProps { bold: Some(true), ..Default::default() };
        a.session.pending = Some(pending.clone());
        a.session.goal_x = Some(123.0);
        alt_frame(&ctx, &mut a, vec![surround(1, 1)]);
        assert_eq!(body(&a), "abXY", "the first side's own-insertion deletion is rolled back with the refused second side");
        assert_eq!(a.session.undo_depth(), depth, "the half-erasure leaves no undo step");
        assert_eq!(a.session.doc.revisions.len(), revisions, "revision metadata is restored too");
        assert!(a.canvas.ime_join_text.is_none(), "no rewrite is promised");
        assert!(a.canvas.ime_interrupt_now, "the platform's stale surrounding snapshot is discarded");
        assert_eq!(a.session.sel.focus.off, 2, "the caret is back where the request found it");
        assert_eq!(a.session.pending, Some(pending), "pending formatting is restored");
        assert_eq!(a.session.goal_x, Some(123.0), "the Up/Down goal column is restored");
    }

    /// Selects a body range, wraps it in a rich-text content control with `tag`, and sets its
    /// placeholder.
    fn wrap_in_control(a: &mut WordApp, start: usize, end: usize, tag: &str, placeholder: &str) {
        a.session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, start), focus: wordcraft_doc::Pos::body(0, end) };
        a.run("developer.richText", json!({"tag": tag})).unwrap();
        a.run("developer.properties", json!({"control": tag, "placeholder": placeholder})).unwrap();
    }

    /// Finds the body content control with `tag`, panicking if it is absent.
    fn control(a: &WordApp, tag: &str) -> wordcraft_doc::ControlRange {
        a.session
            .doc
            .control_ranges(wordcraft_doc::StoryRef::Body)
            .into_iter()
            .find(|r| r.control.tag == tag)
            .unwrap_or_else(|| panic!("the {tag} control remains"))
    }

    /// Emptying a control restores a placeholder longer than the erased text. The right-hand
    /// range and the caret must move by the *signed* length change: the `X` after the control is
    /// erased (not a placeholder character), the caret lands at the placeholder's end, and the
    /// commit replaces the placeholder in the same undo step as the deletion.
    #[test]
    fn delete_surrounding_maps_after_text_and_caret_past_a_growing_placeholder() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "Name: AdaX"})).unwrap();
        wrap_in_control(&mut a, 6, 9, "name", "Enter the full name");
        let r = control(&a, "name");
        let placeholder = r.control.placeholder_shown().to_string();
        assert!(placeholder.len() > "Ada".len(), "the fixture grows the paragraph: {placeholder:?}");
        a.session.sel = wordcraft_engine::Selection::caret(r.end.clone());
        let depth = a.session.undo_depth();

        alt_frame(&ctx, &mut a, vec![surround(3, 1)]);
        assert_eq!(body(&a), format!("Name: {placeholder}"), "the following `X` is erased, not a placeholder character");
        let end = control(&a, "name").content_start().off + placeholder.len();
        assert_eq!((a.session.sel.anchor.off, a.session.sel.focus.off), (end, end), "the caret is at the placeholder's end, never inside it");
        assert_eq!(a.session.undo_depth(), depth + 1, "both erased sides are one undo step");

        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("N".into()))]);
        assert_eq!(body(&a), "Name: N", "the commit replaces the restored placeholder");
        assert_eq!(a.session.undo_depth(), depth + 1, "the replacement joins the deletion");
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body(&a), "Name: AdaX", "one Undo restores both erased sides and the replacement");
    }

    /// A placeholder shorter than the erased text also has to map the suffix and caret by the
    /// signed delta; subtracting a removed-byte count alone describes only this direction.
    #[test]
    fn delete_surrounding_maps_a_shrinking_placeholder_and_after_text() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "Name: AdaX"})).unwrap();
        wrap_in_control(&mut a, 6, 9, "name", "N");
        let r = control(&a, "name");
        a.session.sel = wordcraft_engine::Selection::caret(r.end.clone());
        let depth = a.session.undo_depth();

        alt_frame(&ctx, &mut a, vec![surround(3, 1)]);
        assert_eq!(body(&a), "Name: N", "the shorter placeholder replaces `Ada`, and `X` still goes");
        let end = control(&a, "name").content_start().off + 1;
        assert_eq!((a.session.sel.anchor.off, a.session.sel.focus.off), (end, end));
        assert_eq!(a.session.undo_depth(), depth + 1);

        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("Z".into()))]);
        assert_eq!(body(&a), "Name: Z");
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body(&a), "Name: AdaX", "one Undo restores the original value and the following text");
    }

    /// The after side alone can empty a control and restore a longer placeholder. Its caret
    /// must map to the restored placeholder's end, not to the old pre-deletion offset, and the
    /// text beyond the after-side range stays put.
    #[test]
    fn delete_surrounding_maps_a_placeholder_restored_by_the_after_side() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "aXY"})).unwrap();
        wrap_in_control(&mut a, 1, 2, "name", "Enter the value");
        let r = control(&a, "name");
        let placeholder = r.control.placeholder_shown().to_string();
        assert!(placeholder.len() > 1);
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 1));
        let depth = a.session.undo_depth();

        alt_frame(&ctx, &mut a, vec![surround(0, 1)]);
        assert_eq!(body(&a), format!("a{placeholder}Y"), "only the one after-side `X` is erased; `Y` is beyond the range");
        let end = control(&a, "name").content_start().off + placeholder.len();
        assert_eq!((a.session.sel.anchor.off, a.session.sel.focus.off), (end, end));
        assert_eq!(a.session.undo_depth(), depth + 1);

        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("Q".into()))]);
        assert_eq!(body(&a), "aQY", "the commit replaces the placeholder and leaves the following `Y`");
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body(&a), "aXY", "one Undo restores the emptied control, the following text and the commit");
    }

    /// A refused body request clears the locally painted pre-edit and every replacement join.
    /// The earlier successful deletion stays, but the following commit gets its own Undo step.
    #[test]
    fn a_refused_body_delete_clears_preedit_and_joins() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "ab"})).unwrap();
        wrap_in_control(&mut a, 1, 2, "locked", "b");
        a.run("developer.properties", json!({"control": "locked", "lockEdit": true})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 1));
        let depth = a.session.undo_depth();

        alt_frame(
            &ctx,
            &mut a,
            vec![surround(1, 0), egui::Event::Ime(egui::ImeEvent::Preedit { text: "に".into(), active_range_chars: None }), surround(0, 1)],
        );
        assert_eq!(body(&a), "b", "the editable `a` is erased; the locked control's `b` is not");
        assert!(a.canvas.ime_preedit.is_empty(), "a refused request must not leave its pre-edit painted");
        assert_eq!(a.canvas.ime_preedit_range, None);
        assert!(a.canvas.ime_join_text.is_none(), "no replacement formatting is promised");
        assert!(a.canvas.ime_interrupt_now, "the platform's stale surroundings are interrupted");
        assert_eq!(a.session.undo_depth(), depth + 1, "only the accepted first deletion has a step");

        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
        assert_eq!(body(&a), "Xb");
        assert_eq!(a.session.undo_depth(), depth + 2, "the cleared join gives the commit its own checkpoint");
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body(&a), "b", "Undo takes back only the commit, not the earlier deletion");
    }

    /// A body deletion promises its join only to the immediate matching replacement. An ordinary
    /// mutating command in between consumes the promise and keeps its own Undo checkpoint.
    #[test]
    fn body_delete_join_does_not_leak_to_an_intervening_command() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "abcd"})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 4));
        let base = a.session.undo_depth();

        alt_frame(&ctx, &mut a, vec![surround(1, 0)]);
        assert_eq!(body(&a), "abc");
        assert_eq!(a.session.undo_depth(), base + 1, "the deletion is its own step");

        a.run("format.bold", json!({})).unwrap();
        assert_eq!(body(&a), "abc", "the intervening command changes no text");
        assert_eq!(a.session.undo_depth(), base + 2, "an intervening ordinary command owns its checkpoint");

        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
        assert_eq!(body(&a), "abcX", "the replacement still applies");
        assert_eq!(a.session.undo_depth(), base + 3, "the replacement no longer joins across the ordinary command");
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body(&a), "abc", "Undo takes back only the commit, not the deletion or the Bold step");
    }

    /// A cross-block selection has no single surrounding snapshot. Refusing the request must
    /// also interrupt the composition instead of leaving the platform's hidden text stale.
    #[test]
    fn cross_block_delete_surrounding_is_refused_and_interrupts() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "one\ntwo"})).unwrap();
        a.session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(1, 1) };
        alt_frame(&ctx, &mut a, vec![surround(1, 0)]);
        assert_eq!(body(&a), "one\ntwo", "nothing is deleted across blocks");
        assert!(a.canvas.ime_interrupt_now, "the refused request interrupts the composition");
    }

    /// An IME replacement inside an equation erases the counted characters with the equation's
    /// commands; a byte count (which has no mapping into the tree) deletes nothing. A refusal
    /// drops only the replacement immediately paired with it: the next `Commit`, or an adjacent
    /// `Text` + `Commit`. A later key, pre-edit or direct commit in the same raw batch is fresh
    /// input and applies; so does one in a later frame.
    #[test]
    fn ime_equation_byte_delete_drops_only_its_immediate_replacement() {
        let (ctx, mut a) = alt_frame_app();
        a.run("insert.equation", json!({"linear": "x+y"})).unwrap();
        let linear = |a: &mut WordApp| a.run("equation.get", json!({})).unwrap().get("linear").and_then(|v| v.as_str()).unwrap_or("").to_string();
        assert_eq!(linear(&mut a), "x+y");
        alt_frame(&ctx, &mut a, vec![surround(1, 0)]);
        assert_eq!(linear(&mut a), "x+", "the character before the caret in the equation is erased");
        let base = linear(&mut a);

        // A direct commit in a later frame is fresh input, not the stale replacement.
        a.canvas.ime_interrupt_now = false;
        alt_frame(&ctx, &mut a, vec![surround_bytes(1, 0)]);
        assert_eq!(linear(&mut a), base, "a byte count is not guessed into equation characters");
        assert!(a.canvas.ime_interrupt_now, "the refused byte deletion interrupts the composition");
        a.canvas.ime_interrupt_now = false;
        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("D".into()))]);
        assert_eq!(linear(&mut a), format!("{base}D"), "a later direct commit applies without a pre-edit");

        // The immediately following commit belongs to the refused replacement and is dropped;
        // the next commit in the same raw batch is fresh input and applies.
        let after_pair = linear(&mut a);
        a.canvas.ime_interrupt_now = false;
        alt_frame(
            &ctx,
            &mut a,
            vec![surround_bytes(1, 0), egui::Event::Ime(egui::ImeEvent::Commit("Z".into())), egui::Event::Ime(egui::ImeEvent::Commit("Y".into()))],
        );
        assert_eq!(linear(&mut a), format!("{after_pair}Y"), "only the adjacent stale commit is dropped");
        let after_commit = linear(&mut a);

        // A platform that routes the replacement through `Text` before its `Commit` sends the
        // adjacent pair; both belong to the refused deletion, and a later commit still applies.
        a.canvas.ime_interrupt_now = false;
        alt_frame(
            &ctx,
            &mut a,
            vec![
                surround_bytes(1, 0),
                egui::Event::Text("Q".into()),
                egui::Event::Ime(egui::ImeEvent::Commit("Q2".into())),
                egui::Event::Ime(egui::ImeEvent::Commit("R".into())),
            ],
        );
        assert_eq!(linear(&mut a), format!("{after_commit}R"), "the adjacent Text+Commit pair is stale, the next commit is not");

        // Any intervening event ends the pair: a key then its Text is ordinary fresh input.
        let after_commit = linear(&mut a);
        a.canvas.ime_interrupt_now = false;
        let key = egui::Event::Key { key: Key::F20, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        alt_frame(&ctx, &mut a, vec![surround_bytes(1, 0), key, egui::Event::Text("K".into())]);
        assert_eq!(linear(&mut a), format!("{after_commit}K"), "a key between the deletion and text clears the suppression");

        // A pre-edit likewise ends the stale pair; its commit is fresh.
        let after_commit = linear(&mut a);
        a.canvas.ime_interrupt_now = false;
        alt_frame(
            &ctx,
            &mut a,
            vec![
                surround_bytes(1, 0),
                egui::Event::Ime(egui::ImeEvent::Preedit { text: "p".into(), active_range_chars: None }),
                egui::Event::Ime(egui::ImeEvent::Commit("P".into())),
            ],
        );
        assert_eq!(linear(&mut a), format!("{after_commit}P"), "a fresh composition in the same batch is accepted");
    }

    /// An over-cap equation deletion is still a new surrounding request: it discards the
    /// composition that was painted for the snapshot it refuses, and interrupts it.
    #[test]
    fn over_cap_equation_delete_clears_the_painted_preedit() {
        let (ctx, mut a) = alt_frame_app();
        a.run("insert.equation", json!({"linear": "abc"})).unwrap();
        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: "p".into(), active_range_chars: None })]);
        assert_eq!(a.canvas.ime_preedit, "p");
        a.canvas.ime_interrupt_now = false;
        alt_frame(&ctx, &mut a, vec![surround(wordcraft_engine::MAX_EQUATION_DELETE_UNITS + 1, 0)]);
        assert_eq!(body(&a), "abc", "the over-cap request deletes nothing");
        assert!(a.canvas.ime_preedit.is_empty(), "the refused request's pre-edit is not left painted");
        assert_eq!(a.canvas.ime_preedit_range, None);
        assert!(a.canvas.ime_interrupt_now, "the platform's stale snapshot is interrupted");
    }

    /// An over-cap equation deletion also clears a previous equation deletion's promised join,
    /// so the next commit cannot silently merge into that earlier undo step.
    #[test]
    fn over_cap_equation_delete_clears_the_previous_replacement_join() {
        let (ctx, mut a) = alt_frame_app();
        a.run("insert.equation", json!({"linear": "abcd"})).unwrap();
        alt_frame(&ctx, &mut a, vec![surround(1, 0), surround(wordcraft_engine::MAX_EQUATION_DELETE_UNITS + 1, 0)]);
        assert_eq!(body(&a), "abc", "the first, valid deletion stands");
        assert!(a.canvas.ime_interrupt_now, "the over-cap request interrupts the composition");
        let depth = a.session.undo_depth();
        a.canvas.ime_interrupt_now = false;
        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
        assert_eq!(body(&a), "abcX");
        assert_eq!(a.session.undo_depth(), depth + 1, "the cleared join gives the commit its own undo step");
    }

    /// The transactional equation delete and the following commit are one undo step, like the
    /// body's `DeleteSurrounding` plus its replacement text.
    #[test]
    fn ime_equation_delete_and_commit_are_one_undo_step() {
        let (ctx, mut a) = alt_frame_app();
        a.run("insert.equation", json!({"linear": "abcd"})).unwrap();
        alt_frame(&ctx, &mut a, vec![surround(1, 0), egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
        assert_eq!(body(&a), "abcX");
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body(&a), "abcd", "one Undo restores the deleted unit and the commit together");
    }

    /// An equation deletion promises a join only to its immediate replacement. An unrelated
    /// command in between consumes the promise without joining, so it gets its own undo step
    /// instead of being merged into the deletion.
    #[test]
    fn ime_equation_delete_join_does_not_leak_to_an_unrelated_command() {
        let (ctx, mut a) = alt_frame_app();
        a.run("insert.equation", json!({"linear": "abcd"})).unwrap();
        let base = a.session.undo_depth();
        alt_frame(&ctx, &mut a, vec![surround(1, 0)]);
        assert_eq!(body(&a), "abc");
        assert_eq!(a.session.undo_depth(), base + 1, "the deletion is its own undo step");

        a.run("equation.justify", json!({"jc": "right"})).unwrap();
        assert_eq!(a.session.undo_depth(), base + 2, "an unrelated command keeps its own checkpoint");

        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
        assert_eq!(body(&a), "abcX");
        assert_eq!(a.session.undo_depth(), base + 3, "the replacement no longer joins across the unrelated command");

        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(body(&a), "abc", "Undo takes back only the commit, not the deletion and the unrelated command");
    }

    /// Hostile pre-edit and commit sizes are refused before they reach the canvas or the model.
    #[test]
    fn oversized_ime_events_are_ignored() {
        let (ctx, mut a) = alt_frame_app();
        let huge = "a".repeat(MAX_IME_PREEDIT_BYTES + 1);
        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: huge, active_range_chars: Some(0..1) })]);
        assert!(a.canvas.ime_preedit.is_empty(), "an oversized pre-edit is not painted");
        assert_eq!(a.canvas.ime_preedit_range, None);
        let huge = "b".repeat(MAX_IME_COMMIT_BYTES + 1);
        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit(huge))]);
        assert_eq!(body(&a), "", "an oversized commit is not inserted");
        // The equation cap is lower than the byte cap: an over-cap commit is refused before the
        // command, with no prefix committed and the composition interrupted.
        a.run("insert.equation", json!({})).unwrap();
        a.canvas.ime_interrupt_now = false;
        let long = "c".repeat(wordcraft_engine::MAX_EQUATION_TYPE_CHARS + 1);
        alt_frame(&ctx, &mut a, vec![egui::Event::Ime(egui::ImeEvent::Commit(long))]);
        assert_eq!(body(&a), "", "no equation prefix is committed");
        assert!(a.canvas.ime_interrupt_now, "the refused commit starts the composition afresh");
    }

    /// A pre-edit belongs to the editor that had focus: losing it drops the painted composition.
    #[test]
    fn observe_ime_drops_a_preedit_without_focus() {
        let ctx = egui::Context::default();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.canvas.ime_preedit = "に".into();
        a.canvas.ime_preedit_range = Some(0..1);
        a.canvas.focused = false;
        let r = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(10.0, 10.0));
        observe_ime(&mut a, &ctx, Some((r, r)), false);
        assert!(a.canvas.ime_preedit.is_empty(), "no composition is kept for an unfocused editor");
        assert_eq!(a.canvas.ime_preedit_range, None);
    }

    #[test]
    fn typing_with_smart_art_selected_types_into_an_item() {
        let ctx = egui::Context::default();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("insert.smartArt", json!({"layout": "matrix", "items": ["Ada", "Bea"]})).unwrap();
        for t in ["x", "y"] {
            ctx.run_ui(egui::RawInput { events: vec![egui::Event::Text(t.into())], ..Default::default() }, |ui| canvas_events(&mut a, ui.ctx()))
                .drop_without_applying_deltas();
        }
        let g = wordcraft_engine::cmd::smart_art::selected_smart_art(&a.session).expect("the graphic stays");
        assert_eq!(g.items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["Adaxy", "Bea"]);
        assert!(a.session.view.smart_art_pane, "the Text Pane opens");
        a.run("edit.undo", json!({})).unwrap();
        let g = wordcraft_engine::cmd::smart_art::selected_smart_art(&a.session).expect("still there");
        assert_eq!(g.items[0].text, "Ada", "one undo step for the typing");
    }
}
