//! Large documents stay responsive (#554): work is counted (blocks placed, paragraphs laid
//! out), not timed, so these hold on a busy machine. Incremental pagination must give exactly the
//! pages a full layout does.

use serde_json::json;
use wordcraft_doc::{Block, Path, Pos, StoryRef};

use crate::{Selection, Session};

/// A session on the synthetic book, laid out once, spelling off (export layouts don't proof).
fn book(pages: usize) -> Session {
    let mut s = Session::new(crate::sample::large_document(pages));
    s.view.proofing = false;
    let _ = s.layout();
    s
}

/// The body paragraph of plain text nearest the start of page `page`.
fn text_para_on(s: &mut Session, page: usize) -> usize {
    let lay = s.layout();
    let from = lay.pages.get(page).map(|p| p.first_block).unwrap_or(0);
    let n = s.doc.body.len();
    let ok = |i: usize| matches!(s.doc.body.get(i).map(|b| &**b), Some(Block::Para(p)) if p.len() > 40 && p.objects.is_empty());
    (from..n).find(|&i| ok(i)).or_else(|| (0..from).rev().find(|&i| ok(i))).unwrap()
}

fn caret_in(s: &mut Session, block: usize, off: usize) {
    s.sel = Selection::caret(Pos { story: StoryRef::Body, path: Path::top(block), off });
}

/// The pages as text, for comparing two layouts.
fn pages_dump(l: &wordcraft_layout::DocLayout) -> Vec<String> {
    l.pages.iter().map(|p| format!("{:?}|{:?}|{:?}|{}|{}", p.items, p.header, p.footer, p.number, p.first_block)).collect()
}

/// The session's (incremental) layout is the one a full layout gives.
#[track_caller]
fn assert_same_as_full(s: &mut Session, what: &str) {
    let inc = s.layout();
    let full = s.export_layout();
    assert_eq!(inc.pages.len(), full.pages.len(), "{what}: page count");
    let (a, b) = (pages_dump(&inc), pages_dump(&full));
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        assert!(x == y, "{what}: page {} differs from a full layout", i + 1);
    }
    assert_eq!(inc.index.len(), full.index.len(), "{what}: index");
}

#[test]
fn typing_in_a_long_document_relays_out_only_near_the_edit() {
    let mut s = book(120);
    let blocks = s.doc.body.len();
    let pages = s.layout().pages.len();
    assert!(pages >= 100, "{pages} pages");
    for page in [0, pages / 2, pages - 1] {
        let i = text_para_on(&mut s, page);
        caret_in(&mut s, i, 10);
        for key in 0..3 {
            s.run("text.insert", &json!({"text": "x"})).unwrap();
            let (_, misses) = s.layout_cache_stats();
            let l = s.layout();
            // One paragraph laid out again, a handful of blocks placed: the edited page, and the
            // next pages until they come out as before.
            let laid = s.layout_cache_stats().1 - misses;
            assert!((1..=2).contains(&laid), "page {page}: {laid} paragraphs laid out");
            // The first key after the caret moved also places the paragraph the typed word left again
            // (its squiggle is back), so the span between the two is placed once.
            assert!(key == 0 || l.blocks_placed * 10 < blocks, "page {page}: {} of {blocks} blocks placed", l.blocks_placed);
        }
        assert_same_as_full(&mut s, &format!("typing on page {}", page + 1));
    }
}

#[test]
fn incremental_layout_matches_a_full_layout_after_any_edit() {
    let mut s = book(40);
    let pages = s.layout().pages.len();
    let mid = text_para_on(&mut s, pages / 2);
    type Step = (&'static str, serde_json::Value);
    let cases: Vec<(&str, usize, Vec<Step>)> = vec![
        ("a new paragraph", mid, vec![("text.newParagraph", json!({}))]),
        ("a page break", mid, vec![("text.pageBreak", json!({}))]),
        ("joining paragraphs", mid, vec![("text.backspace", json!({}))]),
        ("a long insertion", mid, vec![("text.insert", json!({"text": "many more words ".repeat(60)}))]),
        ("a heading style", mid, vec![("styles.apply", json!({"style": "Heading1"}))]),
        ("a footnote", mid, vec![("references.footnote", json!({"text": "A new note."}))]),
        ("bold", mid, vec![("select.text", json!({"text": "the"})), ("format.bold", json!({}))]),
        ("margins", mid, vec![("layout.margins", json!({"preset": "narrow"}))]),
        ("typing on the first page", 2, vec![("text.insert", json!({"text": "words to push lines down ".repeat(8)}))]),
        ("typing on the last page", s.doc.body.len() - 1, vec![("text.insert", json!({"text": "a closing remark"}))]),
    ];
    for (what, block, steps) in cases {
        let at = block.min(s.doc.body.len() - 1);
        caret_in(&mut s, at, 0);
        s.clamp_selection();
        for (id, p) in steps {
            s.run(id, &p).unwrap_or_else(|e| panic!("{what}: {id} failed: {e}"));
        }
        assert_same_as_full(&mut s, what);
        // Undo comes back to the earlier pages too.
        s.run("edit.undo", &json!({})).unwrap();
        assert_same_as_full(&mut s, &format!("undoing {what}"));
    }
}

#[test]
fn many_pictures_and_tables_lay_out_without_shaping_them_again() {
    // A second pass over an unchanged-but-relaid document hits the paragraph cache everywhere,
    // table cells and keep-with-next lookahead included.
    let mut s = book(60);
    let i = text_para_on(&mut s, 30);
    caret_in(&mut s, i, 5);
    s.run("text.insert", &json!({"text": "y"})).unwrap();
    // Force a full pass (the incremental one would skip most blocks).
    let doc = s.doc.clone();
    let mut cache = wordcraft_layout::LayoutCache::new();
    let opts = wordcraft_layout::LayoutOptions::default();
    let _ = wordcraft_layout::layout(&doc, &mut cache, &opts);
    let misses = cache.misses;
    let mut d2 = doc.clone();
    d2.body = d2.body.iter().map(|b| std::sync::Arc::new((**b).clone())).collect();
    let l = wordcraft_layout::layout(&d2, &mut cache, &opts);
    assert_eq!(l.blocks_placed, d2.body.len());
    assert_eq!(cache.misses, misses, "a full pass over the same paragraphs lays none out again");
}
