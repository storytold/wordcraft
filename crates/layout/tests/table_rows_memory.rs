//! Laying out a long table holds its rows once: they move onto the pages instead of being
//! copied there while the laid-out table lives on until the last row is placed.

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use wordcraft_doc::{Block, Document, Paragraph, Path, StoryRef, Table, para_block};
use wordcraft_layout::{LayoutCache, LayoutOptions, layout};

#[test]
fn a_long_table_is_not_held_twice_while_it_is_placed() {
    let _prof = dhat::Profiler::builder().testing().build();
    let mut t = Table::new(1000, 4, 468.0);
    t.rows[0].props.header = true;
    for (r, row) in t.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            cell.blocks = vec![para_block(Paragraph::with_text(&format!("Row {r}, cell {c}"), Default::default()))];
        }
    }
    let mut d = Document::new();
    d.insert_block(StoryRef::Body, &Path::top(0), Block::Table(t)).unwrap();
    let mut cache = LayoutCache::new();
    let before = dhat::HeapStats::get();
    let l = layout(&d, &mut cache, &LayoutOptions::default());
    let after = dhat::HeapStats::get();
    assert!(l.pages.len() > 20, "{}", l.pages.len());
    // What the layout and the cache keep, and what laying out needed on top of that.
    let kept = after.curr_bytes - before.curr_bytes;
    let extra = after.max_bytes - after.curr_bytes;
    assert!(extra < kept / 20, "kept {kept} bytes, needed {extra} more while laying out");
}
