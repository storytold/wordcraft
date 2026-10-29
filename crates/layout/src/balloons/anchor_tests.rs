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

#[test]
fn anchors_walk_dense_unrelated_markers_in_linear_work() {
    const N: usize = 2_000;
    let (small, big) = (dense_markers(N), dense_markers(crate::tests::SCALE * N));
    let check = |doc: &Document, n| assert_eq!(anchors(doc), vec![(0, Pos::body(0, n * wordcraft_doc::para::OBJ.len_utf8()))]);
    let (s, b, ratio) = crate::tests::cost_ratio(crate::tests::SCALE, || check(&small, N), || check(&big, crate::tests::SCALE * N));
    eprintln!("balloon anchor CPU: {s:.6}s -> {b:.6}s, {ratio:.2}x for 8x markers");
    assert!(ratio < 24.0, "balloon anchor work grew {ratio:.2}x for 8x markers ({s:.6}s -> {b:.6}s)");
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
