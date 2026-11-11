//! Word's keys for international characters on Windows and Linux (#6328): an accent key
//! (Ctrl+' for the acute, Ctrl+` for the grave…) waits for the next typed character, which then
//! takes the accent (Ctrl+' then e types é). A character the accent doesn't apply to is typed as
//! it is. macOS has none of these: its Option dead keys do the same job.

use std::borrow::Cow;

use serde_json::{Value, json};

use super::{CmdResult, Session};

/// An accent key waiting for its letter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent {
    /// Ctrl+`: à è ì ò ù.
    Grave,
    /// Ctrl+': á é í ó ú ý, and ð.
    Acute,
    /// Ctrl+Shift+^: â ê î ô û.
    Circumflex,
    /// Ctrl+Shift+~: ã ñ õ.
    Tilde,
    /// Ctrl+Shift+: : ä ë ï ö ü ÿ.
    Diaeresis,
    /// Ctrl+Shift+@: å.
    Ring,
    /// Ctrl+Shift+&: æ œ ß.
    Ligature,
    /// Ctrl+, : ç.
    Cedilla,
    /// Ctrl+/ : ø.
    Stroke,
}

impl Accent {
    /// The accent's name in command results.
    pub fn name(self) -> &'static str {
        match self {
            Accent::Grave => "grave",
            Accent::Acute => "acute",
            Accent::Circumflex => "circumflex",
            Accent::Tilde => "tilde",
            Accent::Diaeresis => "diaeresis",
            Accent::Ring => "ring",
            Accent::Ligature => "ligature",
            Accent::Cedilla => "cedilla",
            Accent::Stroke => "stroke",
        }
    }

    /// The character `c` becomes with this accent, if the accent applies to it. Capitals take
    /// the capital accented letter, as in Word; ß has no capital here.
    pub fn compose(self, c: char) -> Option<char> {
        let table: &[(char, char)] = match self {
            Accent::Grave => {
                &[('a', 'à'), ('e', 'è'), ('i', 'ì'), ('o', 'ò'), ('u', 'ù'), ('A', 'À'), ('E', 'È'), ('I', 'Ì'), ('O', 'Ò'), ('U', 'Ù')]
            }
            Accent::Acute => &[
                ('a', 'á'),
                ('e', 'é'),
                ('i', 'í'),
                ('o', 'ó'),
                ('u', 'ú'),
                ('y', 'ý'),
                ('A', 'Á'),
                ('E', 'É'),
                ('I', 'Í'),
                ('O', 'Ó'),
                ('U', 'Ú'),
                ('Y', 'Ý'),
                ('d', 'ð'),
                ('D', 'Ð'),
            ],
            Accent::Circumflex => {
                &[('a', 'â'), ('e', 'ê'), ('i', 'î'), ('o', 'ô'), ('u', 'û'), ('A', 'Â'), ('E', 'Ê'), ('I', 'Î'), ('O', 'Ô'), ('U', 'Û')]
            }
            Accent::Tilde => &[('a', 'ã'), ('n', 'ñ'), ('o', 'õ'), ('A', 'Ã'), ('N', 'Ñ'), ('O', 'Õ')],
            Accent::Diaeresis => &[
                ('a', 'ä'),
                ('e', 'ë'),
                ('i', 'ï'),
                ('o', 'ö'),
                ('u', 'ü'),
                ('y', 'ÿ'),
                ('A', 'Ä'),
                ('E', 'Ë'),
                ('I', 'Ï'),
                ('O', 'Ö'),
                ('U', 'Ü'),
                ('Y', 'Ÿ'),
            ],
            Accent::Ring => &[('a', 'å'), ('A', 'Å')],
            Accent::Ligature => &[('a', 'æ'), ('A', 'Æ'), ('o', 'œ'), ('O', 'Œ'), ('s', 'ß')],
            Accent::Cedilla => &[('c', 'ç'), ('C', 'Ç')],
            Accent::Stroke => &[('o', 'ø'), ('O', 'Ø')],
        };
        table.iter().find(|(from, _)| *from == c).map(|(_, to)| *to)
    }

    /// `text` with its first character accented, when the accent applies to it.
    pub fn compose_text(self, text: &str) -> Option<String> {
        let mut chars = text.chars();
        let first = self.compose(chars.next()?)?;
        Some(std::iter::once(first).chain(chars).collect())
    }
}

/// An accent key: the next character typed at this selection takes the accent. Changes nothing
/// in the document, so the typing undo step stays open and the accented letter joins it.
pub(super) fn wait_for_letter(s: &mut Session, accent: Accent) -> CmdResult {
    s.accent_prefix = Some((accent, s.sel.clone()));
    Ok(json!({"accent": accent.name()}))
}

/// The accent keys can't type into an equation (its own keys handle typing there).
pub(super) fn not_in_equation(s: &Session) -> Option<&'static str> {
    s.math.is_some().then_some("not available in an equation")
}

impl Session {
    /// The accent key waiting for its letter, if any.
    pub fn accent_waiting(&self) -> Option<Accent> {
        self.accent_prefix.as_ref().map(|(a, _)| *a)
    }

    /// Forget a waiting accent key (Escape, or a key that types nothing).
    pub fn cancel_accent(&mut self) {
        self.accent_prefix = None;
    }

    /// The params command `id` runs with, given a waiting accent key: `text.insert` takes it and
    /// accents its first character; any other edit, caret move or selection forgets it.
    pub(crate) fn accent_params<'a>(&mut self, id: &str, mutates: bool, params: &'a Value) -> Cow<'a, Value> {
        if id == "text.insert" {
            return self.take_accent_for(params).map_or(Cow::Borrowed(params), Cow::Owned);
        }
        if mutates || id.starts_with("caret.") || id.starts_with("select.") {
            self.accent_prefix = None;
        }
        Cow::Borrowed(params)
    }

    /// `text.insert`'s params with a waiting accent applied to the first character typed (taken
    /// either way). `None` when nothing changes: no accent waits, the selection moved since the
    /// accent key (a click, another window), or the accent doesn't apply to the character.
    fn take_accent_for(&mut self, params: &Value) -> Option<Value> {
        let (accent, at) = self.accent_prefix.take()?;
        if at != self.sel {
            return None;
        }
        let text = accent.compose_text(params.get("text")?.as_str()?)?;
        let mut composed = params.clone();
        composed.as_object_mut()?.insert("text".into(), Value::String(text));
        Some(composed)
    }
}
