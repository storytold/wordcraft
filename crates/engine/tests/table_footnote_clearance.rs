use serde_json::json;
use std::sync::Arc;
use wordcraft_doc::{Block, Document, Paragraph, PartKind, Path, Pos, StoryRef, Table, para_block};
use wordcraft_engine::Session;
use wordcraft_layout::Placed;

/// A synthetic two-column table with a note in its thirteenth row and a real footer.
fn fixture(columns: u32, distance: f32, footer_lines: usize, note: &str) -> Session {
    let mut d = Document::new();
    let footer = d
        .add_part(PartKind::Footer, (0..footer_lines).map(|_| para_block(Paragraph::with_text("An ordinary footer.", Default::default()))).collect());
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
        let footer = p
            .footer
            .iter()
            .filter_map(|it| match it {
                Placed::Lines { y, l0, l1, .. } if l1 > l0 => Some(*y),
                _ => None,
            })
            .fold(p.h, f32::min);
        let body = p
            .items
            .iter()
            .filter_map(|it| match it {
                Placed::Lines { story: StoryRef::Body, y, para, l0, l1, .. } if l1 > l0 => {
                    let last = &para.lines[l1 - 1];
                    Some(y + last.top - para.lines[*l0].top + last.height)
                }
                _ => None,
            })
            .fold(0.0, f32::max);
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
        let pages = l
            .pages
            .iter()
            .filter(|p| {
                p.items.iter().any(|it| {
                    matches!(it,
                        Placed::Lines { story: StoryRef::Part(id), .. } if doc.parts.get(id).is_some_and(|p| p.kind == PartKind::Footnote)
                    )
                })
            })
            .count();
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

/// A note near the end of a split cell starts on its reference page, not on an earlier fragment.
#[test]
fn split_rows_reserve_only_the_notes_of_retained_lines() {
    let mut s = fixture(2, 35.45, 1, "The later table note.");
    s.run("caret.set", &json!({"pos": Pos { story: StoryRef::Body, path: Path(vec![0, 0, 0, 0]), off: 0 }})).unwrap();
    s.run("text.insert", &json!({"text": "An ordinary long cell paragraph. ".repeat(180)})).unwrap();
    let reference = s.sel.focus.clone();
    let id = s.run("references.footnote", &json!({"text": "The split cell's note."})).unwrap()["id"].as_u64().unwrap() as u32;
    assert_clear(&mut s);
    let l = s.layout();
    let reference_page = l.caret(&reference).unwrap().page;
    let first_note_page = l
        .pages
        .iter()
        .position(|p| {
            p.items.iter().any(|it| {
                matches!(it,
                    Placed::Lines { story: StoryRef::Part(own), .. } if *own == id
                )
            })
        })
        .unwrap();
    assert!(reference_page > 0, "the large cell must split before its reference");
    assert_eq!(first_note_page, reference_page, "no note on a fragment without its reference");
}

/// Fitting and rejected row trials must not consume list numbers from a note or later body text.
#[test]
fn measuring_table_notes_does_not_advance_list_counters() {
    let mut s = fixture(2, 35.45, 1, "A numbered note paragraph.");
    let note = s.sel.focus.story;
    s.run("para.numbering", &json!({})).unwrap();
    let numbering = s.doc.para(note, &Path::top(0)).unwrap().props.numbering;
    let after = s.doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
    after.props.numbering = numbering;
    after.insert_text(0, "After the table.", &Default::default()).unwrap();
    s.touch();
    let l = s.layout();
    let labels = |story, path: &Path| {
        l.pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|it| match it {
                Placed::Lines { story: own, path: at, para, l0: 0, .. } if *own == story && at == path => {
                    para.label.as_ref().map(|label| label.text.trim().to_string())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(labels(note, &Path::top(0)), ["1."]);
    assert_eq!(labels(StoryRef::Body, &Path::top(1)), ["2."]);
    assert_clear(&mut s);
}

/// A cut through one cell's first line must not hide a smaller fitting cut in another cell.
#[test]
fn notes_in_different_cells_keep_a_fitting_prefix_on_the_page() {
    use wordcraft_doc::{CharProps, props::LineSpacing};
    let mut d = Document::new();
    (d.last_section.page_w, d.last_section.page_h) = (595.3, 841.9);
    (d.last_section.margin_top, d.last_section.margin_bottom) = (28.35, 28.35);
    let mut before = Paragraph::with_text("Before the table.", Default::default());
    before.props.space_after = Some(550.0);
    let mut t = Table::new(1, 2, d.last_section.text_width());
    let mut left = Paragraph::with_text(&"Ordinary cell content. ".repeat(100), Default::default());
    left.props.widow_control = Some(false);
    t.rows[0].cells[0].blocks = vec![para_block(left)];
    let mut right = Paragraph::with_text("Tall", CharProps { size: Some(60.0), ..Default::default() });
    right.props.space_before = Some(60.0);
    right.props.line_spacing = Some(LineSpacing::Exactly(100.0));
    t.rows[0].cells[1].blocks = vec![para_block(right)];
    d.body = vec![para_block(before), Arc::new(Block::Table(t)), para_block(Paragraph::new())];
    let mut s = Session::new(d);
    let left_reference = Pos { story: StoryRef::Body, path: Path(vec![1, 0, 0, 0]), off: 5 };
    s.run("caret.set", &json!({"pos": left_reference})).unwrap();
    s.run("references.footnote", &json!({"text": "The early cell note."})).unwrap();
    s.run("caret.set", &json!({"pos": Pos { story: StoryRef::Body, path: Path(vec![1, 0, 1, 0]), off: 1 }})).unwrap();
    s.run("references.footnote", &json!({"text": "The later cell note. ".repeat(30)})).unwrap();
    assert_eq!(s.layout().caret(&left_reference).unwrap().page, 0, "a smaller prefix fits before the tall cell starts");
    assert_clear(&mut s);
}
