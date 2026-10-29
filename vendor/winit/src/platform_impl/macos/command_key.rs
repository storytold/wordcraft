//! The character a key types while ⌘ is held.
//!
//! A keyboard layout has its own table for ⌘. "Dvorak – QWERTY ⌘" types Dvorak letters but uses
//! QWERTY ones while ⌘ is held, and non-Latin layouts can give Latin letters there, so shortcuts
//! stay where users expect them. `charactersIgnoringModifiers` ignores that table.

/// `UCKeyTranslate`'s `modifierKeyState`: Carbon's `EventModifiers` (`cmdKey`, `shiftKey`)
/// shifted right by 8.
pub(super) fn uc_modifier_state(shift: bool, command: bool) -> u32 {
    const CMD_KEY: u32 = 1 << 8;
    const SHIFT_KEY: u32 = 1 << 9;
    let mut modifiers = 0;
    if command {
        modifiers |= CMD_KEY;
    }
    if shift {
        modifiers |= SHIFT_KEY;
    }
    (modifiers >> 8) & 0xff
}

/// Whether text from a layout's table names the key; otherwise the usual rules apply.
pub(super) fn is_key_text(text: &str) -> bool {
    !text.is_empty() && !text.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_state_uses_carbon_bits() {
        assert_eq!(uc_modifier_state(false, false), 0);
        assert_eq!(uc_modifier_state(false, true), 1);
        assert_eq!(uc_modifier_state(true, true), 3);
    }

    #[test]
    fn control_characters_are_not_key_text() {
        assert!(is_key_text("v"));
        assert!(is_key_text("V"));
        assert!(!is_key_text(""));
        assert!(!is_key_text("\u{16}"));
    }
}
