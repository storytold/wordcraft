//! An empty paragraph after a document's leading floating table takes no space before. Word's PDFs
//! of docxide-pdf's `leading-empty-before-float-0` and `-12` put the text after the empty anchor at
//! the same place for 0 pt and 12 pt before.

use super::lay;
use crate::{Arc, Placed, StoryRef};
use wordcraft_doc::para::{Anchor, InlineObject};
use wordcraft_doc::props::TableFloat;
use wordcraft_doc::{Block, Document, Paragraph, Table, para_block};

/// Where the lines of body block `block` start on page one.
fn line_y(d: &Document, block: u32) -> f32 {
    lay(d).pages[0]
        .items
        .iter()
        .find_map(|i| match i {
            Placed::Lines { story: StoryRef::Body, path, y, .. } if path.0 == [block] => Some(*y),
            _ => None,
        })
        .unwrap()
}

/// `table` (a floating one) or nothing, then a paragraph of `text` with `before`, then "After".
fn document(table: bool, text: &str, before: f32) -> Document {
    let mut p = Paragraph::with_text(text, Default::default());
    // "#" stands for a bookmark, a zero-width marker.
    if text == "#" {
        p = Paragraph::new();
        p.insert_object(0, InlineObject::BookmarkStart { name: "_GoBack".into() }, &Default::default()).unwrap();
    }
    p.props.space_before = Some(before);
    let mut d = Document::new();
    d.body = vec![para_block(p), para_block(Paragraph::with_text("After", Default::default()))];
    if table {
        let mut t = Table::new(1, 1, 200.0);
        t.rows[0].cells[0].blocks = vec![para_block(Paragraph::with_text("Table", Default::default()))];
        t.props.float = Some(TableFloat { h_rel: Anchor::Margin, v_rel: Anchor::Paragraph, ..Default::default() });
        d.body.insert(0, Arc::new(Block::Table(t)));
    }
    d
}

#[test]
fn an_empty_paragraph_after_a_floating_table_at_the_page_top_takes_no_space_before() {
    let empty = line_y(&document(true, "", 12.0), 2);
    let plain = line_y(&document(false, "", 0.0), 1);
    assert!((empty - plain).abs() < 0.01, "after the empty anchor at {empty}, with no table nor space before at {plain}");
    // A bookmark alone doesn't make it a paragraph with text.
    assert!((line_y(&document(true, "#", 12.0), 2) - plain).abs() < 0.01);
    // One with text keeps it, and so does an empty first paragraph with no table before it.
    assert!(line_y(&document(true, "Text", 12.0), 2) > empty + 11.99);
    assert!(line_y(&document(false, "", 12.0), 1) > plain + 11.99);
}
