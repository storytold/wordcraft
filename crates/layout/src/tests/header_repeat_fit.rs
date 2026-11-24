//! Synthetic regressions for repeated table header fit on continuation pages and fixed-layout
//! table cell clipping of overflowing inline images.

use super::lay;
use crate::display::{DisplayOptions, Draw, page_display};
use crate::{Placed, StoryRef};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::HeightRule;
use wordcraft_doc::{Block, Document, Paragraph, Path, Table, para_block};

fn inline_picture(w: f32, h: f32) -> InlineObject {
    InlineObject::Image {
        media: "synthetic-inline".into(),
        w,
        h,
        alt: "synthetic".into(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    }
}

/// When consecutive leading `tblHeader` rows alone exceed the page body height (3 x 230 pt =
/// ~691.5 pt > 648 pt Letter body), Word does not repeat them on continuation pages. Without this
/// guard, repeating the oversized header block pushes every body row off the page bottom and spins
/// one page per body row.
#[test]
fn oversized_table_header_block_does_not_repeat_on_continuation_pages() {
    let mut doc = Document::new();
    let mut t = Table::new(12, 1, 468.0);
    for (ri, row) in t.rows.iter_mut().enumerate() {
        row.props.header = ri < 3;
        row.props.height = Some(if ri < 3 { 230.0 } else { 80.0 });
        row.props.height_rule = HeightRule::AtLeast;
        row.cells[0].blocks = vec![para_block(Paragraph::with_text(&format!("Row {ri}"), Default::default()))];
    }
    doc.insert_block(StoryRef::Body, &Path::top(0), Block::Table(t)).unwrap();

    let laid = lay(&doc);
    assert!(laid.pages.len() <= 4, "oversized header block inflated page count to {}", laid.pages.len());

    // Header row 0 must appear exactly once, never repeated on continuation pages.
    let row0_count = laid.pages.iter().flat_map(|p| &p.items).filter(|it| matches!(it, Placed::Cell { row: 0, cell: 0, .. })).count();
    assert_eq!(row0_count, 1, "oversized header row repeated {row0_count} times");

    // Every body row (3..12) must be placed within its page bounds (never pushed off the bottom).
    for body_row in 3..12 {
        let placed = laid.pages.iter().find_map(|p| {
            p.items.iter().find_map(|it| match it {
                Placed::Cell { row, cell: 0, rect, .. } if *row == body_row => Some((rect.y, rect.bottom(), p.body.bottom())),
                _ => None,
            })
        });
        let (y, bottom, page_bottom) = placed.expect("body row must be placed");
        assert!(y < page_bottom && bottom <= page_bottom + 0.5, "body row {body_row} placed off-page: y={y}..{bottom} vs page_bottom={page_bottom}");
    }
}

/// Normal-sized repeated table header rows (fitting comfortably with body rows below them)
/// continue to repeat on continuation pages.
#[test]
fn fitting_table_header_block_still_repeats_on_continuation_pages() {
    let mut doc = Document::new();
    let mut t = Table::new(12, 1, 468.0);
    for (ri, row) in t.rows.iter_mut().enumerate() {
        row.props.header = ri == 0;
        row.props.height = Some(if ri == 0 { 40.0 } else { 80.0 });
        row.props.height_rule = HeightRule::AtLeast;
        row.cells[0].blocks = vec![para_block(Paragraph::with_text(&format!("Row {ri}"), Default::default()))];
    }
    doc.insert_block(StoryRef::Body, &Path::top(0), Block::Table(t)).unwrap();

    let laid = lay(&doc);
    assert!(laid.pages.len() >= 2, "table should span at least two pages");
    let row0_count = laid.pages.iter().flat_map(|p| &p.items).filter(|it| matches!(it, Placed::Cell { row: 0, cell: 0, .. })).count();
    assert_eq!(row0_count, laid.pages.len(), "fitting header row should repeat on every page");
}

/// A fixed-layout (`tblLayout="fixed"`) table cell containing an inline image that extends past
/// the cell's horizontal border clips the cell's lines to the cell rectangle in display and PDF
/// output, while non-overflowing fixed cells and autofit table cells remain unclipped.
#[test]
fn fixed_layout_table_clips_overflowing_inline_image_to_cell() {
    let make_doc = |fixed: bool, img_w: f32, indent_left: f32| {
        let mut doc = Document::new();
        let mut t = Table::new(1, 2, 400.0);
        t.props.fixed = fixed;
        t.grid = vec![100.0, 200.0];
        let mut p = Paragraph::with_text("", Default::default());
        p.props.indent_left = Some(indent_left);
        p.insert_object(0, inline_picture(img_w, 60.0), &Default::default()).unwrap();
        t.rows[0].cells[1].blocks = vec![para_block(p)];
        doc.insert_block(StoryRef::Body, &Path::top(0), Block::Table(t)).unwrap();
        doc
    };

    // 1. Fixed table, 200 pt cell, 190 pt image with 40 pt left indent -> right edge 230 pt > 200 pt -> clipped.
    let doc = make_doc(true, 190.0, 40.0);
    let laid = lay(&doc);
    let cell1 = laid.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Cell { row: 0, cell: 1, rect, clip, .. } => Some((*rect, *clip)),
            _ => None,
        })
        .expect("cell 1 placed");
    assert!(cell1.1, "overflowing inline image in fixed-layout cell must set clip=true");
    assert!((cell1.0.w - 200.0).abs() < 0.5, "fixed cell width preserved: {}", cell1.0.w);

    // Cell 0 (empty) must remain unclipped.
    let cell0_clip = laid.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Cell { row: 0, cell: 0, clip, .. } => Some(*clip),
            _ => None,
        })
        .expect("cell 0 placed");
    assert!(!cell0_clip, "non-overflowing sibling cell must not be clipped");

    // Display list must wrap the inline image in a Draw::Clip matching the cell rectangle.
    let draws = page_display(&doc, &laid.pages[0], &DisplayOptions::default());
    let has_cell_clip = draws.iter().any(|d| match d {
        Draw::Clip { rect, items } => {
            (rect.x - cell1.0.x).abs() < 0.01 && (rect.w - cell1.0.w).abs() < 0.01 && items.iter().any(|it| matches!(it, Draw::Image { .. }))
        }
        _ => false,
    });
    assert!(has_cell_clip, "page_display must emit Draw::Clip at the cell bounds around the overflowing image");

    // 2. Fixed table, fitting 120 pt image with 0 indent -> not clipped.
    let fit_doc = make_doc(true, 120.0, 0.0);
    let fit_laid = lay(&fit_doc);
    let fit_clip = fit_laid.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Cell { row: 0, cell: 1, clip, .. } => Some(*clip),
            _ => None,
        })
        .expect("cell 1 placed");
    assert!(!fit_clip, "fitting inline image in fixed-layout cell must not be clipped");

    // 3. Autofit table (fixed = false) with wide image -> not clipped (table expands instead).
    let auto_doc = make_doc(false, 250.0, 0.0);
    let auto_laid = lay(&auto_doc);
    let auto_clip = auto_laid.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Cell { row: 0, cell: 1, clip, .. } => Some(*clip),
            _ => None,
        })
        .expect("cell 1 placed");
    assert!(!auto_clip, "autofit table cell must not set fixed-table clip");
}
