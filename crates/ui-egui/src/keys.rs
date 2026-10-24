//! Keyboard: typing, editing keys and command shortcuts (the user's custom keys from Customize
//! Keyboard first, then the registry's `shortcut` fields).

use egui::{Key, Modifiers};
use serde_json::{Value, json};

use crate::WordApp;

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

/// Events for the focused canvas: text, editing keys, clipboard, IME. While keytips are showing
/// every key belongs to them (letters pick badges, Escape cancels), so nothing here runs.
pub fn canvas_events(app: &mut WordApp, ctx: &egui::Context) {
    if app.ui.keytips != crate::keytips::Phase::Off {
        return;
    }
    let events = ctx.input(|i| i.events.clone());
    if app.session.view.read_mode {
        // Read Mode is for navigation and selection, not typing into the document. The ribbon's
        // Edit Document button or Escape returns to Print Layout before editing.
        for e in &events {
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
    // `DeleteSurrounding` and the text replacing what it erased arrive in the same frame.
    app.canvas.ime_join_text = None;
    // Mod held since an earlier frame (its ModifiersChanged went by then).
    let held = ctx.input(|i| i.modifiers);
    if held.command {
        app.canvas.paste_mods = Some(held);
    }
    for e in events {
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
        // An Alt shortcut (Alt+Shift+D, Alt+F7…) may also send its key's text; it isn't typed.
        match &e {
            egui::Event::Key { pressed: true, .. } => {
                app.canvas.swallow_text = false;
                app.canvas.alt_code_echo = None;
                app.canvas.ime_join_text = None;
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
                    }
                    continue;
                }
                egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                    app.canvas.ime_preedit.clear();
                    app.canvas.ime_preedit_range = None;
                    if !text.is_empty() {
                        let _ = app.run("equation.type", json!({"text": text}));
                    }
                    continue;
                }
                egui::Event::Paste(t) => {
                    let _ = app.run("equation.type", json!({"text": t}));
                    continue;
                }
                egui::Event::Key { key, pressed: true, modifiers, .. } if equation_key(app, *key, *modifiers) => continue,
                _ => {}
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
                    if crate::dialogs_smart_art::type_into(app, &t) {
                        continue;
                    }
                    // Typing with a text box selected types into it.
                    crate::objects::enter_text_box(app);
                    type_text(app, &t);
                }
            }
            egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars, after_chars }) => delete_surrounding(app, before_chars, after_chars),
            egui::Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars }) => {
                app.canvas.ime_preedit = text;
                app.canvas.ime_preedit_range = active_range_chars;
            }
            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                app.canvas.ime_preedit.clear();
                app.canvas.ime_preedit_range = None;
                if !text.is_empty() && !crate::dialogs_smart_art::type_into(app, &text) {
                    type_text(app, &text);
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
                if key == Key::Escape && app.canvas.paste_options.take().is_some() {
                    continue;
                }
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
                if leave_full_screen(app, key, modifiers, true) {
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
fn leave_full_screen(app: &mut WordApp, key: Key, m: Modifiers, canvas: bool) -> bool {
    let screen = crate::chrome::Screen::of(&app.session.view).exit_command();
    let Some(id) = screen.or(app.session.view.full_screen.then_some("view.fullScreen")) else { return false };
    if key != Key::Escape || !m.is_none() || !app.escape_free {
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
fn type_text(app: &mut WordApp, text: &str) {
    if let Some(props) = app.canvas.ime_join_text.take() {
        app.session.join_next_undo();
        app.session.pending = Some(props);
        let overtype = std::mem::replace(&mut app.session.prefs.overtype, false);
        let typed = app.run("text.insert", json!({"text": text})).is_ok();
        app.session.prefs.overtype = overtype;
        if typed {
            app.session.continue_typing();
        }
    } else {
        let _ = app.run("text.insert", json!({"text": text}));
    }
    note_automatic_change(app);
}

/// AutoCorrect or AutoFormat just changed the text around the caret (its own undo step, which
/// closes the typing group). The web's text agent still holds the text as typed and counts its
/// `DeleteSurrounding` from that, so it is asked to start afresh (`canvas::ime_reset`).
fn note_automatic_change(app: &mut WordApp) {
    if !app.session.typing_open() && app.session.undo_label() == Some("AutoFormat") {
        app.canvas.ime_reset = true;
    }
}

/// An input method's `DeleteSurrounding`: erase `before` characters (Unicode scalar values, as
/// the input method counts them) before the caret and `after` after it, as Backspace does over a
/// selection. The web build's text agent (eframe's hidden `<input>`) sends it, then the new
/// text, whenever its value changes other than by an addition at its end (#1480): Backspace
/// and corrections from phone keyboards, and Chrome's caret jump. The desktop builds never get
/// it (egui-winit 0.36 doesn't send it). The counts come from outside: they stop at the
/// paragraph's ends, so a paragraph mark or list number is never erased. The erasure joins an
/// open typing group and the text that follows joins it too ([`type_text`]), so one Undo takes
/// back the typing with its correction.
/// With a selection (the agent starts afresh on any click or editing key, so it doesn't send
/// one then) nothing is erased: the text that follows replaces the selection as typing does.
/// Nor inside an equation, whose text the agent's characters went to.
fn delete_surrounding(app: &mut WordApp, before: usize, after: usize) {
    let s = &app.session;
    // Inside an equation the document's text isn't what the input method typed into.
    if !s.sel.is_collapsed() || s.math.is_some() {
        return;
    }
    let f = s.sel.focus.clone();
    let Some(text) = s.doc.para_at(&f).map(|p| p.text.as_str()) else { return };
    let (Some(head), Some(tail)) = (text.get(..f.off), text.get(f.off..)) else { return };
    let start = f.off.saturating_sub(head.chars().rev().take(before).map(char::len_utf8).sum::<usize>());
    let end = f.off.saturating_add(tail.chars().take(after).map(char::len_utf8).sum::<usize>());
    if start == end {
        return;
    }
    let join = s.typing_open();
    // The formatting typing at the caret would use: that of the erased text (or set for it).
    let props = s.typing_props();
    app.session.sel =
        wordcraft_engine::Selection { anchor: wordcraft_doc::Pos { off: start, ..f.clone() }, focus: wordcraft_doc::Pos { off: end, ..f.clone() } };
    let caret = f;
    if join {
        app.session.join_next_undo();
    }
    if app.run("text.backspace", json!({})).is_ok() {
        app.canvas.ime_join_text = Some(props);
    } else {
        // Refused (a protected document): leave the caret where it was.
        app.session.sel = wordcraft_engine::Selection::caret(caret);
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

/// Command shortcuts when the canvas isn't focused but no text field is either.
pub fn global_shortcuts(app: &mut WordApp, ctx: &egui::Context) {
    // Customize Keyboard records the keys pressed; they never run commands meanwhile.
    if app.canvas.focused || ctx.egui_wants_keyboard_input() || matches!(app.dialog, Some(crate::dialogs::Dialog::CustomizeKeyboard { .. })) {
        return;
    }
    let events = ctx.input(|i| i.events.clone());
    for e in events {
        if let egui::Event::Key { key, pressed: true, modifiers, .. } = e {
            if leave_full_screen(app, key, modifiers, false) {
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
