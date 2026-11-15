//! Typing and deleting.

use serde_json::Value;
use wordcraft_doc::para::{COLUMN_BREAK, NB_HYPHEN, NBSP, OBJ, PAGE_BREAK, SOFT_HYPHEN};
use wordcraft_doc::props::{Border, BorderStyle, NumRef};
use wordcraft_doc::{Block, ListKind, Path, Pos, StoryRef, Table};

use super::{delete_selection, delete_selection_effect, non_mac, sel_result, split_para, type_text, type_text_effect};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

mod accent;
mod autoformat;
mod delete;
mod enter_tab;
mod unicode;

pub use accent::Accent;
use accent::{not_in_equation, wait_for_letter};
use autoformat::insert;
use delete::{backspace, delete, delete_sentence_back, delete_word_back, delete_word_fwd};
pub use enter_tab::MAX_TYPED_PARAGRAPHS;
use enter_tab::{back_tab, new_paragraph, tab};
pub use unicode::{alt_code_char, oem_code_page, system_code_pages, system_oem_code_page, windows_code_pages};
use unicode::{ins_char, insert_code, toggle_unicode};

/// The text-entry commands: typing, Enter, breaks, Tab, deleting and typed symbols.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("text.insert", "Type Text", "Editing", insert).params(r#"{"text": string, "raw"?: bool, "paragraphs"?: bool} (paragraphs: each line a paragraph, one undo step, at most 10000 lines)"#),
        CommandSpec::new("text.newParagraph", "New Paragraph", "Editing", new_paragraph).key("Enter"),
        CommandSpec::new("text.lineBreak", "Line Break", "Editing", |s, _| ins_char(s, '\n')).key("Shift+Enter"),
        CommandSpec::new("text.pageBreak", "Page Break", "Insert › Pages", |s, _| ins_char(s, PAGE_BREAK)).key("Mod+Enter"),
        CommandSpec::new("text.columnBreak", "Column Break", "Layout › Breaks", |s, _| ins_char(s, COLUMN_BREAK)).key("Mod+Shift+Enter"),
        CommandSpec::new("text.tab", "Tab", "Editing", tab).key("Tab"),
        CommandSpec::new("text.backTab", "Shift+Tab", "Editing", back_tab).key("Shift+Tab"),
        // A tab character even in a table cell, where Tab moves to the next cell (#3137); ⌥Tab on macOS.
        CommandSpec::new("text.insertTab", "Tab Character", "Editing", |s, _| ins_char(s, '\t'))
            .key(if cfg!(target_os = "macos") { "Alt+Tab" } else { "Mod+Tab" }),
        CommandSpec::new("text.backspace", "Backspace", "Editing", backspace).key("Backspace"),
        CommandSpec::new("text.delete", "Delete", "Editing", delete).key("Delete"),
        CommandSpec::new("text.deleteWordBack", "Delete Previous Word", "Editing", delete_word_back).key("Mod+Backspace / Alt+Backspace"),
        CommandSpec::new("text.deleteSentenceBack", "Delete Previous Sentence", "Editing", delete_sentence_back).key("Mod+Shift+Backspace"),
        CommandSpec::new("text.deleteWordForward", "Delete Next Word", "Editing", delete_word_fwd).key("Mod+Delete / Alt+Delete"),
        CommandSpec::new("text.nbsp", "Nonbreaking Space", "Insert › Symbols", |s, _| ins_char(s, NBSP)).key("Mod+Shift+Space"),
        CommandSpec::new("text.nbHyphen", "Nonbreaking Hyphen", "Insert › Symbols", |s, _| ins_char(s, NB_HYPHEN)).key("Mod+Shift+-"),
        CommandSpec::new("text.optionalHyphen", "Optional Hyphen", "Insert › Symbols", |s, _| ins_char(s, SOFT_HYPHEN)).key("Mod+-"),
        // Alt+X on Windows and Linux; macOS has no default (Option+X types a character there).
        CommandSpec::new("text.toggleUnicode", "Toggle Unicode Character", "Editing", toggle_unicode).key(if cfg!(target_os = "macos") {
            ""
        } else {
            "Alt+X"
        }),
        // Alt+numpad codes: the shell collects the keypad digits while Alt is held (#961).
        CommandSpec::new("text.typeCode", "Type Character Code", "Editing", insert_code)
            .params(r#"{"code": string (decimal digits as typed on the keypad, e.g. "0233"), "ansi"?: 874 | 1250–1258 (the leading-0 code page, default 1252), "oem"?: 437 | 737 | 850 | 852 | 857 | 866 (default from the locale)}"#),
        CommandSpec::new("text.emDash", "Em Dash", "Insert › Symbols", |s, _| ins_char(s, '—')).key("Mod+Alt+-"),
        // On macOS Option+G / R / 2 / ; type these directly, so Word for Mac has no Cmd shortcut.
        CommandSpec::new("text.copyright", "Copyright Sign", "Insert › Symbols", |s, _| ins_char(s, '©')).key(non_mac("Mod+Alt+C")),
        CommandSpec::new("text.registered", "Registered Sign", "Insert › Symbols", |s, _| ins_char(s, '®')).key(non_mac("Mod+Alt+R")),
        CommandSpec::new("text.trademark", "Trademark Sign", "Insert › Symbols", |s, _| ins_char(s, '™')).key(non_mac("Mod+Alt+T")),
        CommandSpec::new("text.ellipsis", "Ellipsis", "Insert › Symbols", |s, _| ins_char(s, '…')).key(non_mac("Mod+Alt+.")),
        // Word's international-character keys (#6328): an accent key, then the letter (Ctrl+'
        // then e types é). Windows and Linux only: macOS types these with Option dead keys. The
        // keys are written as egui names them on a US layout: Ctrl+Shift+^ arrives as
        // Ctrl+Shift+6 (egui has no ^ key); Ctrl+Shift+: is the colon, or the ; key it is on.
        accent_key("text.accentGrave", "Grave Accent (à)", Accent::Grave, non_mac("Mod+`")),
        accent_key("text.accentAcute", "Acute Accent (é)", Accent::Acute, non_mac("Mod+'")),
        accent_key("text.accentCircumflex", "Circumflex (ê)", Accent::Circumflex, non_mac("Mod+Shift+6")),
        accent_key("text.accentTilde", "Tilde (ñ)", Accent::Tilde, non_mac("Mod+Shift+`")),
        accent_key("text.accentDiaeresis", "Diaeresis (ü)", Accent::Diaeresis, non_mac("Mod+Shift+: / Mod+Shift+;")),
        accent_key("text.accentRing", "Ring (å)", Accent::Ring, non_mac("Mod+Shift+2")),
        accent_key("text.accentLigature", "Ligature (æ, œ, ß)", Accent::Ligature, non_mac("Mod+Shift+7")),
        accent_key("text.accentCedilla", "Cedilla (ç)", Accent::Cedilla, non_mac("Mod+,")),
        accent_key("text.accentStroke", "Stroke (ø)", Accent::Stroke, non_mac("Mod+/")),
        // Alt+Ctrl+Shift+? and !. With Ctrl+Alt (AltGr on Windows) a US layout names no
        // character for the key, so Windows reports the / and 1 keys themselves.
        CommandSpec::new("text.invertedQuestion", "Inverted Question Mark", "Insert › Symbols", |s, _| ins_char(s, '¿'))
            .key(non_mac("Mod+Alt+Shift+? / Mod+Alt+Shift+/")),
        CommandSpec::new("text.invertedExclamation", "Inverted Exclamation Mark", "Insert › Symbols", |s, _| ins_char(s, '¡'))
            .key(non_mac("Mod+Alt+Shift+! / Mod+Alt+Shift+1")),
    ]
}

/// An accent key's command: the next character typed with `text.insert` at the same selection
/// takes the accent ([`Accent::compose`]). It changes nothing itself, so typing stays one undo
/// step. Not in an equation.
fn accent_key(id: &'static str, label: &'static str, accent: Accent, key: &'static str) -> CommandSpec {
    let run: fn(&mut Session, &Value) -> CmdResult = match accent {
        Accent::Grave => |s, _| wait_for_letter(s, Accent::Grave),
        Accent::Acute => |s, _| wait_for_letter(s, Accent::Acute),
        Accent::Circumflex => |s, _| wait_for_letter(s, Accent::Circumflex),
        Accent::Tilde => |s, _| wait_for_letter(s, Accent::Tilde),
        Accent::Diaeresis => |s, _| wait_for_letter(s, Accent::Diaeresis),
        Accent::Ring => |s, _| wait_for_letter(s, Accent::Ring),
        Accent::Ligature => |s, _| wait_for_letter(s, Accent::Ligature),
        Accent::Cedilla => |s, _| wait_for_letter(s, Accent::Cedilla),
        Accent::Stroke => |s, _| wait_for_letter(s, Accent::Stroke),
    };
    CommandSpec::new(id, label, "Editing", run)
        .key(key)
        .params("{} (the next character typed by text.insert takes the accent where it applies, e.g. é; any other is typed as it is)")
        .pure()
        .when(not_in_equation)
}
