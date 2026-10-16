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
        for _ in 0..3 {
            s.run("text.insert", &json!({"text": "x"})).unwrap();
            let (_, misses) = s.layout_cache_stats();
            let l = s.layout();
            // One paragraph laid out again, a handful of blocks placed: the edited page, and the
            // next pages until they come out as before.
            let laid = s.layout_cache_stats().1 - misses;
            assert!((1..=2).contains(&laid), "page {page}: {laid} paragraphs laid out");
            assert!(l.blocks_placed * 10 < blocks, "page {page}: {} of {blocks} blocks placed", l.blocks_placed);
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
            let _ = s.run(id, &p);
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

/// A body paragraph of plain text (no objects) at `i`.
fn plain(s: &Session, i: usize) -> bool {
    matches!(s.doc.body.get(i).map(|b| &**b), Some(Block::Para(p)) if p.len() > 40 && p.objects.is_empty() && p.props.numbering.is_none())
}

#[test]
fn incremental_layout_keeps_no_space_before_after_a_paragraph_ending_in_a_page_break() {
    // #1284: a paragraph ending in a page break (its mark stays with the break) leaves the next
    // paragraph, first on the new page, without its space before. Typing further down that page
    // resumes from the page's first block, so the flag must come back with it.
    let mut s = book(40);
    let pages = s.layout().pages.len();
    let from = text_para_on(&mut s, pages / 2);
    let i = (from..s.doc.body.len()).find(|&i| (0..4).all(|k| plain(&s, i + k))).unwrap();
    caret_in(&mut s, i + 1, 0);
    s.run("para.spacing", &json!({"before": 48})).unwrap();
    // No space after on the paragraph with the break (it would push the new page's text down).
    caret_in(&mut s, i, 0);
    s.run("para.spacing", &json!({"after": 0})).unwrap();
    let len = s.doc.body[i].as_para().unwrap().len();
    caret_in(&mut s, i, len);
    s.run("text.pageBreak", &json!({})).unwrap();
    assert_same_as_full(&mut s, "a page break at the end of a paragraph");
    let lay = s.layout();
    // The page the break starts counts the paragraph with the break as its first block.
    let page = lay.pages.iter().rposition(|p| p.first_block == i).expect("the break starts a page");
    assert!(lay.pages.get(page + 1).is_none_or(|p| p.first_block > i + 3), "the edit is on the same page");
    let top = lay.pages[page].items.iter().find_map(|it| match it {
        wordcraft_layout::Placed::Lines { path, y, .. } if *path == Path::top(i + 1) => Some(*y),
        _ => None,
    });
    let first = lay.pages[page - 1].items.iter().find_map(|it| match it {
        wordcraft_layout::Placed::Lines { story: StoryRef::Body, y, .. } => Some(*y),
        _ => None,
    });
    assert_eq!(top, first, "no space before at the top of the page");
    caret_in(&mut s, i + 3, 5);
    for _ in 0..3 {
        s.run("text.insert", &json!({"text": "x"})).unwrap();
        assert!(s.layout().blocks_placed < s.doc.body.len(), "laid out incrementally");
        assert_same_as_full(&mut s, "typing on the page after the break");
    }
}

#[test]
fn a_note_edited_in_place_is_laid_out_again() {
    // #1284: the pass holds the blocks of headers, notes and text boxes, so a note changed in
    // place (its block not shared, so not copied) is a different block, not stale pages.
    let mut s = book(40);
    let notes: Vec<u32> = s.doc.parts.iter().filter(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(id, _)| *id).collect();
    let id = notes[notes.len() / 3];
    for round in 0..2 {
        let part = s.doc.parts.get_mut(&id).unwrap();
        let block = part.blocks.first_mut().unwrap();
        if let Block::Para(p) = std::sync::Arc::make_mut(block) {
            let at = p.len();
            p.insert_text(at, &" and a much longer note than before".repeat(8), &Default::default()).unwrap();
        }
        s.touch();
        assert_same_as_full(&mut s, &format!("a note changed in place, round {round}"));
    }
}

#[test]
fn incremental_layout_matches_a_full_layout_after_edits_in_other_stories() {
    let mut s = book(40);
    let pages = s.layout().pages.len();
    let mid = text_para_on(&mut s, pages / 2);
    let part_of = |s: &Session, kind: wordcraft_doc::PartKind| {
        let ids: Vec<u32> = s.doc.parts.iter().filter(|(_, p)| p.kind == kind).map(|(id, _)| *id).collect();
        ids[ids.len() / 2]
    };
    let type_in = |s: &mut Session, story: StoryRef, path: Path, text: &str| {
        let off = s.doc.para(story, &path).map(|p| p.len()).unwrap_or(0);
        s.sel = Selection::caret(Pos { story, path, off });
        s.run("text.insert", &json!({"text": text})).unwrap();
    };
    // Typing in a footnote: its page's notes grow.
    let note = part_of(&s, wordcraft_doc::PartKind::Footnote);
    for _ in 0..3 {
        type_in(&mut s, StoryRef::Part(note), Path::top(0), " more words in the note");
        assert_same_as_full(&mut s, "typing in a footnote");
    }
    type_in(&mut s, StoryRef::Part(note), Path::top(0), &" a note now several lines long".repeat(10));
    assert_same_as_full(&mut s, "a long footnote");
    // Typing in the header: two lines push the body down on every page.
    let header = part_of(&s, wordcraft_doc::PartKind::Header);
    type_in(&mut s, StoryRef::Part(header), Path::top(0), &" and a subtitle".repeat(10));
    assert_same_as_full(&mut s, "typing in the header");
    // A table cell.
    let t = (mid..s.doc.body.len()).find(|&i| matches!(s.doc.body.get(i).map(|b| &**b), Some(Block::Table(_)))).unwrap();
    for text in ["x", &" a cell that grows taller".repeat(12)] {
        type_in(&mut s, StoryRef::Body, Path(vec![t as u32, 1, 1, 0]), text);
        assert_same_as_full(&mut s, "typing in a table cell");
    }
    // Renumbering a list: the book's lists all continue one numbering, so taking the first item
    // out renumbers every item after it.
    let item = (0..s.doc.body.len()).find(|&i| s.doc.body[i].as_para().is_some_and(|p| p.props.numbering.is_some())).unwrap();
    caret_in(&mut s, item, 0);
    s.run("para.numbering", &json!({"off": true})).unwrap();
    assert_same_as_full(&mut s, "renumbering a list");
    s.run("edit.undo", &json!({})).unwrap();
    assert_same_as_full(&mut s, "undoing the renumbering");
    // A text box on a middle page, then typing in it.
    caret_in(&mut s, mid, 0);
    s.run("insert.textBox", &json!({"text": "Boxed"})).unwrap();
    assert_same_as_full(&mut s, "a text box");
    let tb = part_of(&s, wordcraft_doc::PartKind::TextBox);
    for text in ["x", &" text that overflows the box".repeat(6)] {
        type_in(&mut s, StoryRef::Part(tb), Path::top(0), text);
        assert_same_as_full(&mut s, "typing in a text box");
    }
}

#[test]
fn incremental_layout_matches_a_full_layout_with_section_breaks() {
    let mut s = book(40);
    let pages = s.layout().pages.len();
    // Section breaks a third and two thirds of the way through, the second continuous.
    for (page, kind) in [(pages / 3, "nextPage"), (2 * pages / 3, "continuous")] {
        let i = text_para_on(&mut s, page);
        caret_in(&mut s, i, 0);
        s.run("layout.break", &json!({"kind": kind})).unwrap();
        assert_same_as_full(&mut s, &format!("a {kind} section break"));
    }
    // Typing in each section, then a change to the middle section's margins.
    for page in [1, pages / 2, pages - 2] {
        let i = text_para_on(&mut s, page);
        caret_in(&mut s, i, 3);
        for _ in 0..3 {
            s.run("text.insert", &json!({"text": "typed "})).unwrap();
            assert_same_as_full(&mut s, &format!("typing on page {} of a sectioned document", page + 1));
        }
        s.run("text.insert", &json!({"text": "enough words to add a line ".repeat(6)})).unwrap();
        assert_same_as_full(&mut s, &format!("adding lines on page {}", page + 1));
    }
    let i = text_para_on(&mut s, pages / 2);
    caret_in(&mut s, i, 0);
    s.run("layout.margins", &json!({"preset": "narrow"})).unwrap();
    assert_same_as_full(&mut s, "the middle section's margins");
}
