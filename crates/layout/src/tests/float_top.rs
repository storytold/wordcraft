//! An empty paragraph after a document's leading floating table takes no space before. Word's PDFs
//! of docxide-pdf's `leading-empty-before-float-0` and `-12` put the text after the empty anchor at
//! the same place for 0 pt and 12 pt before.

use super::lay;
use crate::{Arc, DocLayout, LayoutCache, LayoutOptions, Placed, StoryRef, ViewMode, layout};
use wordcraft_doc::para::{Anchor, InlineObject};
use wordcraft_doc::props::TableFloat;
use wordcraft_doc::{Block, Document, Paragraph, Table, para_block};

/// Positions are compared to a hundredth of a point; a kept 12 pt space before shows as at least
/// 11.99 pt more.
const EPS: f32 = 0.01;
const KEPT: f32 = 12.0 - EPS;

/// What the paragraph after the table holds.
enum Content<'a> {
    Empty,
    /// Nothing but this zero-width marker.
    Marker(&'a InlineObject),
    Text(&'static str),
}

/// The page and top of body block `block`'s lines.
fn line_at(l: &DocLayout, block: u32) -> (usize, f32) {
    l.pages
        .iter()
        .enumerate()
        .find_map(|(pi, pg)| {
            pg.items.iter().find_map(|i| match i {
                Placed::Lines { story: StoryRef::Body, path, y, .. } if path.0 == [block] => Some((pi, *y)),
                _ => None,
            })
        })
        .unwrap_or_else(|| panic!("no lines laid out for body block {block}"))
}

/// Where body block `block`'s lines start (on whatever page).
fn line_y(d: &Document, block: u32) -> f32 {
    line_at(&lay(d), block).1
}

/// A floating table `table` wide (or none), then a paragraph of `content` with `before`, then "After".
fn document(table: Option<f32>, content: &Content<'_>, before: f32) -> Document {
    let mut p = match content {
        Content::Empty => Paragraph::new(),
        Content::Text(text) => Paragraph::with_text(text, Default::default()),
        Content::Marker(obj) => {
            let mut p = Paragraph::new();
            p.insert_object(0, (*obj).clone(), &Default::default()).unwrap();
            p
        }
    };
    p.props.space_before = Some(before);
    let mut d = Document::new();
    d.body = vec![para_block(p), para_block(Paragraph::with_text("After", Default::default()))];
    if let Some(w) = table {
        let mut t = Table::new(1, 1, w);
        t.rows[0].cells[0].blocks = vec![para_block(Paragraph::with_text("Table", Default::default()))];
        t.props.float = Some(TableFloat { h_rel: Anchor::Margin, v_rel: Anchor::Paragraph, ..Default::default() });
        d.body.insert(0, Arc::new(Block::Table(t)));
    }
    d
}

#[test]
fn an_empty_paragraph_after_a_floating_table_at_the_page_top_takes_no_space_before() {
    let empty = line_y(&document(Some(200.0), &Content::Empty, 12.0), 2);
    let plain = line_y(&document(None, &Content::Empty, 0.0), 1);
    assert!((empty - plain).abs() < EPS, "after the empty anchor at {empty}, with no table nor space before at {plain}");
    // One with text keeps it, and so does an empty first paragraph with no table before it.
    assert!(line_y(&document(Some(200.0), &Content::Text("Text"), 12.0), 2) > empty + KEPT);
    assert!(line_y(&document(None, &Content::Empty, 12.0), 1) > plain + KEPT);
}

#[test]
fn a_paragraph_of_markers_only_after_the_leading_float_takes_no_space_before() {
    let empty = line_y(&document(Some(200.0), &Content::Empty, 12.0), 2);
    let markers = [
        InlineObject::BookmarkStart { name: "_GoBack".into() },
        InlineObject::CommentStart { id: 1 },
        InlineObject::PermStart { id: 1, group: "everyone".into(), editor: String::new() },
    ];
    for m in markers {
        let y = line_y(&document(Some(200.0), &Content::Marker(&m), 12.0), 2);
        assert!((y - empty).abs() < EPS, "{m:?}: after it at {y}, after an empty anchor at {empty}");
    }
}

#[test]
fn an_empty_paragraph_below_a_full_width_leading_float_takes_no_space_before() {
    // Not checked against Word (no fixture has it): the rule doesn't look at whether the paragraph
    // sits beside the table, so below one as wide as the text it drops its space before too.
    let full = |before| line_y(&document(Some(468.0), &Content::Empty, before), 2);
    assert!((full(12.0) - full(0.0)).abs() < EPS, "after it with 12 pt before at {}, with none at {}", full(12.0), full(0.0));
}

#[test]
fn web_view_and_a_floating_table_on_a_later_page_keep_the_space_before() {
    // Web view lays floating tables out in the text: the paragraph after one keeps its space before.
    let web = |before| {
        let opts = LayoutOptions { view: ViewMode::Web, web_width: 500.0, ..Default::default() };
        line_at(&layout(&document(Some(200.0), &Content::Empty, before), &mut LayoutCache::new(), &opts), 2).1
    };
    assert!(web(12.0) > web(0.0) + KEPT, "web view: after it with 12 pt before at {}, with none at {}", web(12.0), web(0.0));
    // A filler paragraph whose space after leaves less room than the table needs: the table goes to
    // the top of page two with its empty anchor, which isn't the document's leading float.
    let body_h = lay(&Document::new()).pages[0].body.h;
    let later = |before| {
        let mut d = document(Some(200.0), &Content::Empty, before);
        let mut filler = Paragraph::with_text("Filler", Default::default());
        filler.props.space_after = Some(body_h - 20.0);
        d.body.insert(0, para_block(filler));
        let l = lay(&d);
        assert_eq!(line_at(&l, 2).0, 1, "the empty anchor is on page two");
        line_at(&l, 3).1
    };
    assert!(later(12.0) > later(0.0) + KEPT, "later page: after it with 12 pt before at {}, with none at {}", later(12.0), later(0.0));
}
