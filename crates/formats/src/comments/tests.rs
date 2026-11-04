use super::*;
use wordcraft_doc::{Block, CharProps, Comment, Paragraph, PartKind, Run, Table, para_block};

fn paragraph(text: String, objects: Vec<InlineObject>) -> Paragraph {
    let len = text.len();
    Paragraph { text, objects, runs: vec![Run { len, props: CharProps::default() }], ..Paragraph::new() }
}

fn comment(doc: &mut Document, id: u32, author: &str, parent: Option<u32>, text: &str) {
    let part = doc.add_part(PartKind::Comment, vec![para_block(Paragraph::with_text(text, CharProps::default()))]);
    doc.comments.insert(id, Comment { author: author.into(), part, parent, ..Default::default() });
}

#[test]
fn marker_walk_preserves_comment_quotes_order_and_unpaired_objects() {
    let mut doc = Document::new();
    comment(&mut doc, 9, " Ada ", None, "First note");
    comment(&mut doc, 3, " ", Some(9), "Reply text");
    comment(&mut doc, 5, "", None, "Second note");
    comment(&mut doc, 7, "Grace", None, "Unanchored");
    comment(&mut doc, 12, "Linus", None, "Cell note");
    doc.body = vec![para_block(paragraph(
        format!("é漢{OBJ}α {OBJ}β{OBJ} γ{OBJ}{OBJ}"),
        vec![
            InlineObject::CommentStart { id: 9 },
            InlineObject::CommentStart { id: 5 },
            InlineObject::CommentEnd { id: 9 },
            InlineObject::CommentEnd { id: 5 },
            // No marker for this object: it must not anchor the unanchored comment.
            InlineObject::BookmarkStart { name: "unrelated".into() },
            InlineObject::CommentStart { id: 7 },
        ],
    ))];
    // Missing object storage must not replay an earlier comment marker.
    doc.body.push(para_block(paragraph(format!("{OBJ}{OBJ}tail"), vec![])));
    let mut table = Table::new(1, 1, 200.0);
    table.rows[0].cells[0].blocks =
        vec![para_block(paragraph(format!("{OBJ}cell{OBJ}"), vec![InlineObject::CommentStart { id: 12 }, InlineObject::CommentEnd { id: 12 }]))];
    doc.body.push(std::sync::Arc::new(Block::Table(table)));
    let before = doc.clone();
    let actual = notes(&doc);
    assert_eq!(
        actual,
        vec![
            Note { author: "Ada".into(), quote: "α β".into(), text: "First note".into(), replies: vec![("Author".into(), "Reply text".into())] },
            Note { author: "Author".into(), quote: "β γ".into(), text: "Second note".into(), replies: vec![] },
            Note { author: "Linus".into(), quote: "cell".into(), text: "Cell note".into(), replies: vec![] },
            Note { author: "Grace".into(), quote: "".into(), text: "Unanchored".into(), replies: vec![] },
        ]
    );
    assert_eq!(doc, before, "export discovery must leave the document unchanged");
}

fn dense_markers(n: usize) -> Document {
    let mut doc = Document::new();
    comment(&mut doc, 9, "Ada", None, "Kept note");
    let mut objects = Vec::with_capacity(2 * n + 2);
    for k in 0..n {
        objects.push(InlineObject::BookmarkStart { name: format!("bookmark{k}") });
        objects.push(InlineObject::BookmarkEnd { name: format!("bookmark{k}") });
    }
    objects.extend([InlineObject::CommentStart { id: 9 }, InlineObject::CommentEnd { id: 9 }]);
    doc.body = vec![para_block(paragraph(format!("{}{OBJ}é漢 quote{OBJ}", OBJ.to_string().repeat(2 * n)), objects))];
    doc
}

// Ignore scheduler delays. An absent, failed, decreasing or nonadvancing counter supplies no
// performance evidence; the semantic assertions below still run on every platform.
fn cpu_cost(mut f: impl FnMut()) -> Option<f64> {
    let ran = || std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse::<u64>().ok();
    let before = ran();
    f();
    match (before, ran()) {
        (Some(a), Some(b)) if b > a => Some((b - a) as f64 * 1e-9),
        _ => None,
    }
}

/// Unrelated markers must add one walk, rather than a fresh paragraph-prefix scan per marker.
#[test]
fn comment_export_discovery_scales_with_unrelated_markers() {
    let (small, big) = (dense_markers(2_000), dense_markers(16_000));
    let expected = vec![Note { author: "Ada".into(), quote: "é漢 quote".into(), text: "Kept note".into(), replies: vec![] }];
    assert_eq!(notes(&small), expected);
    assert_eq!(notes(&big), expected);
    let (mut t1, mut t8) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..3 {
        let a = cpu_cost(|| {
            for _ in 0..8 {
                std::hint::black_box(notes(&small));
            }
        });
        let b = cpu_cost(|| {
            std::hint::black_box(notes(&big));
        });
        if let (Some(a), Some(b)) = (a, b) {
            t1 = t1.min(a / 8.0);
            t8 = t8.min(b);
        }
    }
    if t1.is_finite() && t8.is_finite() {
        let ratio = t8 / t1;
        eprintln!("comment discovery CPU: small={t1:.6}s big={t8:.6}s ratio={ratio:.2}");
        assert!(ratio < 24.0, "8x the markers cost {ratio:.2}x CPU ({t1:.6}s versus {t8:.6}s)");
    } else {
        eprintln!("comment discovery scaling skipped: no advancing thread CPU samples");
    }
}
