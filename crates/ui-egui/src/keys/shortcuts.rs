use super::{Key, Modifiers};

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
pub fn ui_shortcut(key: Key, m: Modifiers) -> Option<(&'static str, serde_json::Value)> {
    let alt_only = m.alt && !m.command && !m.ctrl && !m.mac_cmd && !m.shift;
    let mac_cmd_alt = cfg!(target_os = "macos") && m.mac_cmd && m.alt && !m.ctrl && !m.shift;
    match key {
        Key::Q if alt_only && !cfg!(target_os = "macos") => Some(("ui.dialog", serde_json::json!({"name": "commands"}))),
        Key::R if mac_cmd_alt => Some(("ui.collapseRibbon", serde_json::json!({}))),
        // Ctrl+F1 collapses or expands the ribbon, as in Word for Windows (#1678). Not on macOS,
        // where Ctrl+F1 is the system's keyboard-access switch.
        Key::F1 if m.ctrl && !m.alt && !m.shift && !m.mac_cmd && !cfg!(target_os = "macos") => Some(("ui.collapseRibbon", serde_json::json!({}))),
        _ => None,
    }
}
