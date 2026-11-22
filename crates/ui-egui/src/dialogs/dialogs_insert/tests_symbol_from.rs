//! Symbol dialog, Symbols tab: Word's "from:" list (Unicode (hex), ASCII (decimal), ASCII (hex)),
//! the Shortcut key line and the AutoCorrect… button.

use super::symbol::{SymbolFrom, char_from_code, code_text, from_showing, grid_chars, symbol_autocorrect, symbol_shortcut};
use super::*;

use SymbolFrom::{AsciiDecimal, AsciiHex, UnicodeHex};

fn app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default())
}

fn code(c: char, from: SymbolFrom) -> Option<String> {
    code_text(c, from)
}

#[test]
fn the_from_list_is_words() {
    let labels: Vec<&str> = SymbolFrom::ALL.iter().map(|f| f.label()).collect();
    assert_eq!(labels, ["Unicode (hex)", "ASCII (decimal)", "ASCII (hex)"]);
    assert_eq!(SymbolFrom::default(), UnicodeHex);
    let a = app();
    assert_eq!(SymbolForm::new(&a, 0).from, UnicodeHex);
}

#[test]
fn character_codes_by_mode() {
    assert_eq!((code('©', UnicodeHex), code('©', AsciiDecimal), code('©', AsciiHex)), (Some("00A9".into()), Some("169".into()), Some("A9".into())));
    assert_eq!((code('€', UnicodeHex), code('€', AsciiDecimal), code('€', AsciiHex)), (Some("20AC".into()), Some("128".into()), Some("80".into())));
    assert_eq!((code('A', AsciiDecimal), code('A', AsciiHex)), (Some("65".into()), Some("41".into())));
    assert_eq!((code('♥', AsciiDecimal), code('♥', AsciiHex)), (None, None), "♥ is not in the code page");
    assert_eq!(code('😀', UnicodeHex), Some("1F600".into()));
}

#[test]
fn typed_codes_select_by_mode() {
    assert_eq!(char_from_code("169", AsciiDecimal), Some('©'));
    assert_eq!(char_from_code("0169", AsciiDecimal), Some('©'));
    assert_eq!(char_from_code("80", AsciiHex), Some('€'));
    assert_eq!(char_from_code("81", AsciiHex), None, "0x81 is undefined in Windows-1252");
    assert_eq!(char_from_code("300", AsciiDecimal), None);
    for t in ["2665", "U+2665", " 2665 "] {
        assert_eq!(char_from_code(t, UnicodeHex), Some('♥'), "{t:?}");
    }
}

#[test]
fn the_shortcut_key_line() {
    assert_eq!(symbol_shortcut('♥', UnicodeHex, None, Some("Alt+X")), "2665, Alt+X");
    assert_eq!(symbol_shortcut('½', UnicodeHex, None, Some("Alt+X")), "00BD, Alt+X");
    assert_eq!(symbol_shortcut('½', AsciiDecimal, None, Some("Alt+X")), "Alt+0189");
    assert_eq!(symbol_shortcut('½', AsciiHex, None, Some("Alt+X")), "Alt+0189");
    assert_eq!(symbol_shortcut('€', AsciiHex, None, Some("Alt+X")), "Alt+0128");
    assert_eq!(symbol_shortcut('©', UnicodeHex, Some("Ctrl+Alt+C"), Some("Alt+X")), "Ctrl+Alt+C", "a command's own key wins");
    assert_eq!(symbol_shortcut('♥', AsciiDecimal, None, Some("Alt+X")), "");
}

#[test]
fn ascii_modes_show_the_code_page_in_byte_order() {
    let chars = ['A', '©', 'Ÿ', '€', '♥'];
    assert_eq!(grid_chars(&chars, AsciiDecimal), vec!['A', '€', 'Ÿ', '©']);
    assert_eq!(grid_chars(&chars, AsciiHex), vec!['A', '€', 'Ÿ', '©']);
    assert_eq!(grid_chars(&chars, UnicodeHex), chars.to_vec());
}

#[test]
fn autocorrect_opens_with_the_symbol_in_with() {
    let mut a = app();
    let id = symbol_autocorrect(&mut a, '♥').expect("AutoCorrect opens");
    let host =
        a.dialog_host.get_mut_by_id::<crate::dialogs::host::FormHost<crate::dialogs_review::AutoCorrectForm>>(id).expect("the AutoCorrect form");
    let f = host.form();
    assert_eq!((f.tab, f.from.as_str(), f.to.as_str()), (0, "", "♥"));
}

#[test]
fn a_recent_symbol_with_no_ascii_code_switches_to_unicode() {
    assert_eq!(from_showing('♥', AsciiDecimal), UnicodeHex);
    assert_eq!(from_showing('♥', AsciiHex), UnicodeHex);
    assert_eq!(from_showing('½', AsciiDecimal), AsciiDecimal);
    assert_eq!(from_showing('€', AsciiHex), AsciiHex);
    assert_eq!(from_showing('♥', UnicodeHex), UnicodeHex);
}

#[test]
fn the_unicode_line_names_the_toggle_unicode_key() {
    assert_eq!(symbol_shortcut('♥', UnicodeHex, None, Some("Alt+X")), "2665, Alt+X");
    assert_eq!(symbol_shortcut('♥', UnicodeHex, None, Some("Ctrl+Shift+U")), "2665, Ctrl+Shift+U");
    assert_eq!(symbol_shortcut('♥', UnicodeHex, None, None), "");
    assert_eq!(symbol_shortcut('½', AsciiDecimal, None, None), "Alt+0189");
    assert_eq!(symbol_shortcut('♥', UnicodeHex, Some("Ctrl+Alt+H"), None), "Ctrl+Alt+H");
}
