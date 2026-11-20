//! Symbol dialog: a recently used symbol picked in an ASCII from: mode stays selected.

use super::symbol::SymbolFrom;
use super::*;

fn app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("one\ntwo")), Default::default())
}

#[test]
fn picking_a_recent_symbol_in_ascii_mode_keeps_it_selected() {
    let a = app();
    let mut f = SymbolForm::new(&a, 0);
    f.set_from(SymbolFrom::AsciiDecimal);
    // Ā has no Windows-1252 byte: the mode switches to Unicode (hex) and Ā stays selected.
    // (Ā is in every Latin text font, so the test needs no symbol font installed.)
    f.pick_recent("Arial", 'Ā');
    assert_eq!(f.selected, Some('Ā'));
    assert_eq!(f.from, SymbolFrom::UnicodeHex);
    // Same font, ASCII mode again: Ā still switches the mode back to Unicode (hex).
    f.set_from(SymbolFrom::AsciiDecimal);
    f.pick_recent("Arial", 'Ā');
    assert_eq!((f.selected, f.from), (Some('Ā'), SymbolFrom::UnicodeHex));
    // ½ has an ASCII code: the mode stays.
    f.set_from(SymbolFrom::AsciiDecimal);
    f.pick_recent("", '½');
    assert_eq!(f.from, SymbolFrom::AsciiDecimal);
    assert_eq!(f.selected, Some('½'));
    assert_eq!(f.code, "189");
}
