//! Keyboard: typing, editing keys and command shortcuts (from the registry's `shortcut` fields).

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
        Key::Num5 => "5",
        Key::Num8 => "8",
        Key::Num9 => "9",
        Key::F3 => "F3",
        Key::F5 => "F5",
        Key::F7 => "F7",
        Key::F9 => "F9",
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

fn combo(m: Modifiers, key: &str, with_shift: bool) -> String {
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

/// Find and run the command bound to this key. Returns true if handled.
fn dispatch(app: &mut WordApp, key: Key, m: Modifiers) -> bool {
    let Some(name) = key_name(key) else { return false };
    let reg = app.session.registry.clone();
    if let Some(spec) = reg.by_shortcut(&combo(m, name, true)) {
        let _ = app.run(spec.id, json!({}));
        return true;
    }
    // Shift extends caret movement.
    if m.shift
        && let Some(spec) = reg.by_shortcut(&combo(m, name, false))
        && spec.id.starts_with("caret.")
    {
        let _ = app.run(spec.id, json!({"extend": true}));
        return true;
    }
    false
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

/// Events for the focused canvas: text, editing keys, clipboard, IME.
pub fn canvas_events(app: &mut WordApp, ctx: &egui::Context) {
    let events = ctx.input(|i| i.events.clone());
    for e in events {
        // Editing an equation: text and editing keys go into it.
        if app.session.math.is_some() {
            match &e {
                egui::Event::Text(t) => {
                    let m = ctx.input(|i| i.modifiers);
                    if !(m.command || (m.ctrl && !cfg!(target_os = "macos"))) && !t.is_empty() && t.chars().all(|c| !c.is_control()) {
                        let _ = app.run("equation.type", json!({"text": t}));
                    }
                    continue;
                }
                egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                    app.canvas.ime_preedit.clear();
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
                if m.command || (m.ctrl && !cfg!(target_os = "macos")) {
                    continue;
                }
                if t.chars().all(|c| !c.is_control()) && !t.is_empty() {
                    let _ = app.run("text.insert", json!({"text": t}));
                }
            }
            egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => app.canvas.ime_preedit = text,
            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                app.canvas.ime_preedit.clear();
                if !text.is_empty() {
                    let _ = app.run("text.insert", json!({"text": text}));
                }
            }
            egui::Event::Paste(t) => {
                let id = if ctx.input(|i| i.modifiers.shift && i.modifiers.alt) { "edit.pasteText" } else { "edit.paste" };
                let _ = app.run(id, json!({"text": t}));
            }
            egui::Event::Copy | egui::Event::Cut => {
                let id = if matches!(e, egui::Event::Cut) { "edit.cut" } else { "edit.copy" };
                if let Ok(r) = app.run(id, json!({}))
                    && let Some(t) = r.get("text").and_then(Value::as_str)
                    && !t.is_empty()
                {
                    ctx.copy_text(t.to_string());
                }
            }
            egui::Event::Key { key, pressed: true, modifiers, .. } => {
                if key == Key::Escape && app.session.painter.is_some() {
                    app.session.painter = None;
                    continue;
                }
                if key == Key::Escape && matches!(app.session.sel.focus.story, wordcraft_doc::StoryRef::Part(_)) && app.session.sel.is_collapsed() {
                    let _ = app.run("insert.closeHeader", json!({}));
                    continue;
                }
                dispatch(app, key, modifiers);
            }
            _ => {}
        }
    }
}

/// Command shortcuts when the canvas isn't focused but no text field is either.
pub fn global_shortcuts(app: &mut WordApp, ctx: &egui::Context) {
    if app.canvas.focused || ctx.egui_wants_keyboard_input() {
        return;
    }
    let events = ctx.input(|i| i.events.clone());
    for e in events {
        if let egui::Event::Key { key, pressed: true, modifiers, .. } = e
            && (modifiers.command || matches!(key, Key::F3 | Key::F5 | Key::F7 | Key::F9 | Key::F12))
        {
            dispatch(app, key, modifiers);
        }
    }
}
