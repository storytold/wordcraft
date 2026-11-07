use serde_json::json;
use std::sync::Arc;
use wordcraft_doc::{Block, Document, Paragraph, PartKind, Path, Pos, StoryRef, Table, para_block};
use wordcraft_engine::Session;
use wordcraft_layout::Placed;

/// A synthetic two-column table with a note in its thirteenth row and a real footer.
fn fixture(columns: u32, distance: f32, footer_lines: usize, note: &str) -> Session {
    let mut d = Document::new();
    let footer = d.add_part(PartKind::Footer, (0..footer_lines).map(|_| para_block(Paragraph::with_text("An ordinary footer.", Default::default()))).collect());
    let s = &mut d.last_section;
    (s.page_w, s.page_h) = (595.3, 841.9);
    (s.margin_top, s.margin_bottom, s.margin_left, s.margin_right) = (28.35, 28.35, 28.35, 28.35);
    s.columns.count = columns;
    s.columns.space = 14.15;
    s.footer = distance;
    s.footers.default = Some(footer);
    let mut t = Table::new(24, 2, (s.text_width() - (columns - 1) as f32 * 14.15) / columns as f32);
    for row in &mut t.rows {
        for cell in &mut row.cells {
            cell.blocks = vec![para_block(Paragraph::with_text(&"Ordinary table cell content. ".repeat(5), Default::default()))];
        }
    }
    d.body = vec![Arc::new(Block::Table(t)), para_block(Paragraph::new())];
    let mut s = Session::new(d);
    s.run("caret.set", &json!({"pos": Pos { story: StoryRef::Body, path: Path(vec![0, 12, 0, 0]), off: 20 }})).unwrap();
    s.run("references.footnote", &json!({"text": note})).unwrap();
    s
}

/// Notes and their separator must fit above the actual footer, regardless of nominal margins.
fn assert_clear(s: &mut Session) {
    let doc = s.doc.clone();
    let l = s.layout();
    let mut pieces = 0;
    for (pi, p) in l.pages.iter().enumerate() {
        let footer = p.footer.iter().filter_map(|it| match it {
            Placed::Lines { y, l0, l1, .. } if l1 > l0 => Some(*y),
            _ => None,
        }).fold(p.h, f32::min);
        let body = p.items.iter().filter_map(|it| match it {
            Placed::Lines { story: StoryRef::Body, y, para, l0, l1, .. } if l1 > l0 => {
                let last = &para.lines[l1 - 1];
                Some(y + last.top - para.lines[*l0].top + last.height)
            }
            _ => None,
        }).fold(0.0, f32::max);
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Part(id), y, para, l0, l1, .. } = it
                && doc.parts.get(id).is_some_and(|part| part.kind == PartKind::Footnote)
                && l1 > l0
            {
                let last = &para.lines[l1 - 1];
                let bottom = y + last.top - para.lines[*l0].top + last.height;
                assert!(bottom <= footer + 0.05 && bottom <= p.h + 0.05, "page {pi}: note bottom {bottom} below footer {footer} / page {}", p.h);
                assert!(*y >= body - 0.05, "page {pi}: note top {y} overlaps body bottom {body}");
                pieces += 1;
            }
        }
    }
    assert!(pieces > 0, "the note must be laid out");
}

/// A long table note continues in readable pieces instead of overlapping either body column.
#[test]
fn long_table_notes_continue_above_multiline_footers() {
    for columns in [1, 2] {
        let mut s = fixture(columns, 60.0, 3, &"More editable note content. ".repeat(400));
        assert_clear(&mut s);
        let doc = s.doc.clone();
        let l = s.layout();
        let pages = l.pages.iter().filter(|p| p.items.iter().any(|it| matches!(it,
            Placed::Lines { story: StoryRef::Part(id), .. } if doc.parts.get(id).is_some_and(|p| p.kind == PartKind::Footnote)
        ))).count();
        assert!(pages > 1, "the overlong note must continue");
    }
}

/// Editing notes, history and DOCX round trips use the same reservation in warm and cold layouts.
#[test]
fn editing_table_notes_and_docx_round_trips_preserve_clearance() {
    let mut s = fixture(2, 35.45, 3, "A short footnote.");
    assert_clear(&mut s);
    s.run("text.insert", &json!({"text": " More editable note content. ".repeat(100)})).unwrap();
    assert_clear(&mut s);
    assert_clear(&mut Session::new(s.doc.clone()));
    s.run("edit.undo", &json!({})).unwrap();
    assert_clear(&mut s);
    s.run("edit.redo", &json!({})).unwrap();
    assert_clear(&mut s);
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    let reopened = wordcraft_docx::read(&bytes).unwrap();
    let reopened_note = reopened.parts.iter().find(|(_, p)| p.kind == PartKind::Footnote).unwrap().0;
    assert_eq!(s.doc.plain_text(s.sel.focus.story), reopened.plain_text(StoryRef::Part(*reopened_note)));
    assert_clear(&mut Session::new(reopened));
}

/// Leaf #99: reserving the note after placing its table row must not push it under the footer.
#[test]
fn table_footnotes_stay_above_the_footer() {
    for distance in [14.0, 28.35, 35.45, 60.0] {
        let mut s = fixture(2, distance, 1, "A short footnote.");
        assert_clear(&mut s);
    }
}
