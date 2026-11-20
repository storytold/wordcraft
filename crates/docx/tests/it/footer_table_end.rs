//! A footer may end in a table: saving must not add a line that moves it upward.
use std::io::{Cursor, Read};
use std::sync::Arc;
use wordcraft_doc::{Block, Document, Paragraph, PartKind, Table, para_block};

/// Write and reopen the in-memory synthetic document without filesystem fixtures.
fn roundtrip(doc: &Document) -> Document {
    wordcraft_docx::read(&wordcraft_docx::write(doc).unwrap()).unwrap()
}

/// Two consecutive saves preserve a terminal footer table without adding flow height.
#[test]
fn table_only_footer_does_not_gain_a_paragraph_on_save() {
    let mut doc = Document::from_text("Synthetic body");
    let id = doc.add_part(PartKind::Footer, vec![Arc::new(Block::Table(Table::new(2, 2, 300.0)))]);
    doc.last_section.footers.default = Some(id);
    for _ in 0..2 {
        doc = roundtrip(&doc);
        let part = &doc.parts[&doc.last_section.footers.default.unwrap()];
        assert_eq!(part.blocks.len(), 1, "an added empty paragraph changes footer height and placement");
        assert!(matches!(&*part.blocks[0], Block::Table(_)));
    }
}

/// A paragraph explicitly present after the table keeps its spacing on save.
#[test]
fn explicit_footer_paragraph_after_a_table_is_preserved() {
    let mut doc = Document::from_text("Synthetic body");
    let mut ending = Paragraph::new();
    ending.props.space_after = Some(7.0);
    let id = doc.add_part(PartKind::Footer, vec![Arc::new(Block::Table(Table::new(1, 1, 300.0))), para_block(ending)]);
    doc.last_section.footers.default = Some(id);
    let read = roundtrip(&doc);
    let part = &read.parts[&read.last_section.footers.default.unwrap()];
    assert_eq!(part.blocks.len(), 2);
    assert_eq!(part.blocks[1].as_para().unwrap().props.space_after, Some(7.0));
}

/// Inspect written XML so reader/model fallback paragraphs cannot hide a missing writer branch.
#[test]
fn an_empty_footer_still_gets_a_paragraph() {
    let mut doc = Document::from_text("Synthetic body");
    let id = doc.add_part(PartKind::Footer, vec![]);
    doc.last_section.footers.default = Some(id);
    // add_part normalizes empty stories; clear that fallback to reach the writer branch.
    doc.parts.get_mut(&id).unwrap().blocks.clear();
    let bytes = wordcraft_docx::write(&doc).unwrap();
    let mut package = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    package.by_name("word/footer1.xml").unwrap().read_to_string(&mut xml).unwrap();
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut paragraphs = 0;
    loop {
        match reader.read_event().unwrap() {
            quick_xml::events::Event::Start(element) | quick_xml::events::Event::Empty(element) if element.name().as_ref() == b"w:p" => {
                paragraphs += 1
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }
    assert_eq!(paragraphs, 1, "the serialized empty footer must contain its paragraph");
}

/// Omitting a footer-root ending must not change the required ending inside nested cells.
#[test]
fn table_cells_still_end_in_a_paragraph() {
    let mut doc = Document::from_text("Synthetic body");
    let mut outer = Table::new(1, 1, 300.0);
    outer.rows[0].cells[0].blocks = vec![Arc::new(Block::Table(Table::new(1, 1, 200.0)))];
    let id = doc.add_part(PartKind::Footer, vec![Arc::new(Block::Table(outer))]);
    doc.last_section.footers.default = Some(id);
    let read = roundtrip(&doc);
    let part = &read.parts[&read.last_section.footers.default.unwrap()];
    let Block::Table(table) = &*part.blocks[0] else { panic!("table") };
    assert_eq!(table.rows[0].cells[0].blocks.len(), 2);
    assert!(table.rows[0].cells[0].blocks.last().unwrap().as_para().is_some());
}

/// Inspect actual footer-root block elements without relying on reader fallback paragraphs.
fn written_footer_blocks(doc: &Document) -> Vec<String> {
    let bytes = wordcraft_docx::write(doc).unwrap();
    let mut package = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    package.by_name("word/footer1.xml").unwrap().read_to_string(&mut xml).unwrap();
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut depth = 0;
    let mut blocks = Vec::new();
    loop {
        match reader.read_event().unwrap() {
            quick_xml::events::Event::Start(element) => {
                if depth == 1 {
                    blocks.push(String::from_utf8(element.name().as_ref().to_vec()).unwrap());
                }
                depth += 1;
            }
            quick_xml::events::Event::Empty(element) if depth == 1 => {
                blocks.push(String::from_utf8(element.name().as_ref().to_vec()).unwrap());
            }
            quick_xml::events::Event::End(_) => depth -= 1,
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }
    blocks
}

/// Footer tables with no writable rows need a real XML paragraph, even before the reader runs.
#[test]
fn skipped_footer_tables_get_a_fallback_paragraph() {
    let mut no_rows = Table::new(1, 1, 300.0);
    no_rows.rows.clear();
    let mut no_cells = Table::new(2, 2, 300.0);
    for row in &mut no_cells.rows {
        row.cells.clear();
    }
    for tables in [vec![no_rows.clone()], vec![no_cells.clone()], vec![no_rows, no_cells]] {
        let mut doc = Document::from_text("Synthetic body");
        let blocks = tables.into_iter().map(|t| Arc::new(Block::Table(t))).collect();
        let id = doc.add_part(PartKind::Footer, blocks);
        doc.last_section.footers.default = Some(id);
        for _ in 0..2 {
            assert_eq!(written_footer_blocks(&doc), ["w:p"]);
            doc = roundtrip(&doc);
        }
    }
}

/// A skipped table beside a real table must not introduce an extra line or footer height.
#[test]
fn skipped_tables_do_not_add_spacing_to_a_real_footer_table() {
    let mut skipped = Table::new(1, 1, 300.0);
    skipped.rows[0].cells.clear();
    let real = Table::new(1, 1, 300.0);
    for tables in [vec![real.clone(), skipped.clone()], vec![skipped.clone(), real.clone()], vec![skipped.clone(), real, skipped]] {
        let mut doc = Document::from_text("Synthetic body");
        let blocks = tables.into_iter().map(|t| Arc::new(Block::Table(t))).collect();
        let id = doc.add_part(PartKind::Footer, blocks);
        doc.last_section.footers.default = Some(id);
        for _ in 0..2 {
            assert_eq!(written_footer_blocks(&doc), ["w:tbl"]);
            doc = roundtrip(&doc);
        }
    }
}

/// An explicit paragraph remains the only root block when a later empty-row table is skipped.
#[test]
fn skipped_table_after_a_footer_paragraph_does_not_add_a_paragraph() {
    let mut doc = Document::from_text("Synthetic body");
    let mut skipped = Table::new(1, 1, 300.0);
    skipped.rows.clear();
    let mut para = Paragraph::new();
    para.props.space_after = Some(7.0);
    let id = doc.add_part(PartKind::Footer, vec![para_block(para), Arc::new(Block::Table(skipped))]);
    doc.last_section.footers.default = Some(id);
    assert_eq!(written_footer_blocks(&doc), ["w:p"]);
    let read = roundtrip(&doc);
    let part = &read.parts[&read.last_section.footers.default.unwrap()];
    assert_eq!(part.blocks.len(), 1);
    assert_eq!(part.blocks[0].as_para().unwrap().props.space_after, Some(7.0));
}
