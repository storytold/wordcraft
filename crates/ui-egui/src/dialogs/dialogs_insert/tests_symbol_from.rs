//! Symbol dialog, Symbols tab: Word's "from:" list (Unicode (hex), ASCII (decimal), ASCII (hex)),
//! the Shortcut key line and the AutoCorrect… button.

use super::symbol::{SymbolFrom, char_from_code, code_text, grid_chars, symbol_autocorrect, symbol_shortcut};
use super::*;

use SymbolFrom::{AsciiDecimal, AsciiHex, UnicodeHex};

fn app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("one\ntwo")), Default::default())
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
    assert_eq!(symbol_shortcut('♥', UnicodeHex, None), "2665, Alt+X");
    assert_eq!(symbol_shortcut('½', UnicodeHex, None), "00BD, Alt+X");
    assert_eq!(symbol_shortcut('½', AsciiDecimal, None), "Alt+0189");
    assert_eq!(symbol_shortcut('½', AsciiHex, None), "Alt+0189");
    assert_eq!(symbol_shortcut('€', AsciiHex, None), "Alt+0128");
    assert_eq!(symbol_shortcut('©', UnicodeHex, Some("Ctrl+Alt+C")), "Ctrl+Alt+C", "a command's own key wins");
    assert_eq!(symbol_shortcut('♥', AsciiDecimal, None), "");
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
    symbol_autocorrect(&mut a, '♥');
    match a.dialog.take() {
        Some(crate::dialogs::Dialog::Review(d)) => match *d {
            crate::dialogs_review::ReviewDialog::AutoCorrect(f) => {
                assert_eq!((f.tab, f.from.as_str(), f.to.as_str()), (0, "", "♥"));
            }
            d => panic!("not AutoCorrect: {d:?}"),
        },
        d => panic!("no AutoCorrect dialog: {d:?}"),
    }
}
