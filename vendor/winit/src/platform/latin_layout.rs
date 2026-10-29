//! Latin letters for shortcuts on non-Latin keyboard layouts.

use crate::event::KeyEvent;
use crate::keyboard::Key;

/// Additional methods for the `KeyEvent` that help match shortcuts such as
/// <kbd>Ctrl</kbd>+<kbd>C</kbd> on Russian, Greek, Arabic and other non-Latin layouts.
pub trait KeyEventExtLatinLayout {
    /// What this key types without modifiers on the first Latin layout the user has configured,
    /// when the active layout isn't Latin.
    ///
    /// For example, the key that types `с` on a Russian layout gives `Some(Key::Character("c"))`
    /// when the user also has a US layout, and `Some(Key::Character("j"))` with Dvorak. Letters
    /// are lowercase. A character other than a letter means the key has no Latin letter: with
    /// Dvorak, the key that types `й` gives `'`, so Ctrl on it is no shortcut on Q. This is
    /// `None` on a Latin layout, for keys that type no character on the Latin layout, and when
    /// the user has no Latin layout.
    fn latin_key(&self) -> Option<Key>;

    /// Whether the active layout types Latin letters, such as US, AZERTY or Dvorak.
    ///
    /// This is decided for the whole layout rather than for each key: on a Latin layout, use the
    /// letters it types.
    fn is_latin_layout(&self) -> bool;
}

#[cfg(any(x11_platform, wayland_platform))]
impl KeyEventExtLatinLayout for KeyEvent {
    #[inline]
    fn latin_key(&self) -> Option<Key> {
        self.platform_specific.latin_key.clone()
    }

    #[inline]
    fn is_latin_layout(&self) -> bool {
        self.platform_specific.is_latin_layout
    }
}
