//! Line numbering (`w:lnNumType`, ECMA-376 Part 1 §17.6.8) and the built-in "line number"
//! character style that formats the numbers (#6018).

use std::sync::Arc;

use wordcraft_doc::section::{Columns, LineNumberRestart, LineNumbering};
use wordcraft_doc::{CharProps, Document, Rgb, Style, StyleKind, TextColor};

/// Every line-numbering option and a localised "line number" style (German Word calls its id
/// `Zeilennummer`) survive a save and reopen, beside two columns.
#[test]
fn line_numbering_options_and_style_round_trip() {
    let mut d = Document::from_text("one\ntwo\nthree");
    let numbering = LineNumbering { count_by: 5, start: 3, distance: 21.6, restart: LineNumberRestart::Continuous };
    d.last_section.line_numbers = Some(numbering.clone());
    d.last_section.columns = Columns { count: 2, space: 36.0, separator: false, widths: Vec::new() };
    let red = TextColor::Rgb(Rgb(0xC0, 0, 0));
    Arc::make_mut(&mut d.styles).upsert(Style {
        id: "Zeilennummer".into(),
        name: "Line Number".into(),
        kind: StyleKind::Character,
        chr: CharProps { size: Some(8.0), color: Some(red), italic: Some(true), ..Default::default() },
        ..Default::default()
    });
    let back = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(back.last_section.line_numbers, Some(numbering));
    assert_eq!(back.last_section.columns.count, 2);
    let st = back.styles.find("line number").expect("the line number style");
    assert_eq!(st.id, "Zeilennummer");
    assert_eq!((st.chr.size, st.chr.color, st.chr.italic), (Some(8.0), Some(red), Some(true)));
}
