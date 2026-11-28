use super::*;
use wordcraft_doc::{Block, Comment, Path, Table, para_block};

fn push(p: &mut Paragraph, object: InlineObject) {
    p.insert_object(p.len(), object, &CharProps::default()).unwrap();
}

#[test]
fn anchors_keep_first_root_occurrences_and_utf8_table_positions() {
    let mut doc = Document::from_text("Préface Ω\nSecond");
    for (id, parent) in [(40, None), (7, Some(40)), (9, None), (2, None), (3, None)] {
        doc.comments.insert(id, Comment { parent, ..Default::default() });
    }
    let p = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    push(p, InlineObject::BookmarkStart { name: "unrelated".into() });
    push(p, InlineObject::CommentStart { id: 999 }); // no such comment
    push(p, InlineObject::CommentStart { id: 7 }); // reply, not its own balloon
    push(p, InlineObject::CommentEnd { id: 9 });
    let first = p.len();
    push(p, InlineObject::CommentStart { id: 40 });
    push(p, InlineObject::CommentStart { id: 40 }); // first occurrence wins
    let second = p.len();
    push(p, InlineObject::CommentStart { id: 9 });
    let mut table = Table::new(1, 1, 200.0);
    let mut cell = Paragraph::with_text("Cell λ", CharProps::default());
    let in_cell = cell.len();
    push(&mut cell, InlineObject::CommentStart { id: 2 });
    table.rows[0].cells[0].blocks = vec![para_block(cell)];
    doc.body.push(std::sync::Arc::new(Block::Table(table)));
    // Comments in other stories do not acquire body balloons.
    let mut note = Paragraph::with_text("Note", CharProps::default());
    push(&mut note, InlineObject::CommentStart { id: 3 });
    doc.add_part(wordcraft_doc::PartKind::Footnote, vec![para_block(note)]);
    assert_eq!(
        anchors(&doc),
        vec![(40, Pos::body(0, first)), (9, Pos::body(0, second)), (2, Pos::new(StoryRef::Body, Path(vec![2, 0, 0, 0]), in_cell))]
    );
}

fn dense_markers(n: usize) -> Document {
    let mut doc = Document::new();
    doc.comments.insert(0, Comment::default());
    let mut p = Paragraph::with_text("x", CharProps::default());
    p.text = wordcraft_doc::para::OBJ.to_string().repeat(n);
    p.runs[0].len = p.len();
    p.objects = vec![InlineObject::BookmarkEnd { name: "unrelated".into() }; n];
    push(&mut p, InlineObject::CommentStart { id: 0 });
    doc.body = vec![para_block(p)];
    doc
}

/// Asserts `check` on `make(SCALE * n)` takes at most 1.5 * SCALE times the steps it takes on
/// `make(n)`: about 8x for 8x the input when the work is linear, about 64x when it is quadratic.
/// The steps, not a timing, are both counters' together: the paragraph walks, the paragraph list
/// and the path lookups (`wordcraft_doc::work`), and the comment lookups (`crate::work`).
fn assert_linear_anchor_work(what: &str, n: usize, make: impl Fn(usize) -> Document, check: impl Fn(&Document, usize)) {
    let (small, big) = (make(n), make(crate::tests::SCALE * n));
    let steps = |doc: &Document, n| {
        let mut in_doc = 0;
        let in_layout = crate::work::count(|| in_doc = wordcraft_doc::work::count(|| check(doc, n)));
        (in_doc, in_layout)
    };
    let ((sd, sl), (bd, bl)) = (steps(&small, n), steps(&big, crate::tests::SCALE * n));
    let (s, b) = (sd + sl, bd + bl);
    let ratio = b as f64 / s.max(1) as f64;
    eprintln!("{what}: {s} -> {b} steps ({sd} -> {bd} in wordcraft_doc, {sl} -> {bl} in layout), {ratio:.1}x for 8x the input");
    assert!(sd > 0, "{what}: no steps counted in wordcraft_doc, so the walks this test watches no longer count them (wordcraft_doc::work)");
    assert!(sl > 0, "{what}: no steps counted in layout, so the comment lookups this test watches no longer count them (crate::work)");
    assert!(ratio <= 1.5 * crate::tests::SCALE as f64, "{what}: work grew {ratio:.1}x for 8x the input ({s} -> {b} steps)");
}

#[test]
fn anchors_walk_dense_unrelated_markers_in_linear_work() {
    let check = |doc: &Document, n| assert_eq!(anchors(doc), vec![(0, Pos::body(0, n * wordcraft_doc::para::OBJ.len_utf8()))]);
    assert_linear_anchor_work("balloon anchors, dense markers", 2_000, dense_markers, check);
}

/// `n` paragraphs, every other one in a table cell, each holding an unrelated marker, the start
/// of a reply, the start of the comment it answers twice and the start of a missing comment.
fn threads(n: usize) -> Document {
    let mut doc = Document::new();
    doc.body = (0..n as u32)
        .map(|i| {
            let (top, reply) = (2 * i, 2 * i + 1);
            doc.comments.insert(top, Comment::default());
            doc.comments.insert(reply, Comment { parent: Some(top), ..Default::default() });
            let mut p = Paragraph::with_text("x", CharProps::default());
            push(&mut p, InlineObject::BookmarkEnd { name: "unrelated".into() });
            for id in [reply, top, top, u32::MAX] {
                push(&mut p, InlineObject::CommentStart { id });
            }
            if i % 2 == 0 {
                return para_block(p);
            }
            let mut table = Table::new(1, 1, 200.0);
            table.rows[0].cells[0].blocks = vec![para_block(p)];
            std::sync::Arc::new(Block::Table(table))
        })
        .collect();
    doc
}

#[test]
fn anchors_take_linear_work_over_paragraphs_comments_and_replies() {
    let check = |doc: &Document, n: usize| {
        let ids: Vec<u32> = anchors(doc).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, (0..n as u32).map(|i| 2 * i).collect::<Vec<_>>());
    };
    assert_linear_anchor_work("balloon anchors, threads", 500, threads, check);
}

#[test]
fn anchors_keep_the_existing_limit_in_document_order() {
    let mut doc = Document::new();
    let mut p = Paragraph::with_text("x", CharProps::default());
    p.text = wordcraft_doc::para::OBJ.to_string().repeat(MAX_BALLOONS + 1);
    p.runs[0].len = p.len();
    p.objects = (0..=MAX_BALLOONS).map(|id| InlineObject::CommentStart { id: id as u32 }).collect();
    doc.comments = (0..=MAX_BALLOONS).map(|id| (id as u32, Comment::default())).collect();
    doc.body = vec![para_block(p)];
    let got = anchors(&doc);
    assert_eq!(got.len(), MAX_BALLOONS);
    assert_eq!(got.first(), Some(&(0, Pos::body(0, 0))));
    assert_eq!(got.last(), Some(&((MAX_BALLOONS - 1) as u32, Pos::body(0, (MAX_BALLOONS - 1) * wordcraft_doc::para::OBJ.len_utf8()))));
}
