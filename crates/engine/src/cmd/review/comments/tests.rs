use super::*;
use std::hint::black_box;
use std::time::{Duration, Instant};
use wordcraft_doc::{Document, Path};

fn marked(objects: Vec<InlineObject>) -> Session {
    let mut doc = Document::from_text("é tail");
    let p = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.text = format!("é{} tail", "\u{fffc}".repeat(objects.len()));
    p.runs.first_mut().unwrap().len = p.text.len();
    p.objects = objects;
    let part = doc.try_add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("Original comment.", Default::default()))]).unwrap();
    doc.comments.insert(7, Comment { author: "Original author".into(), part, ..Default::default() });
    Session::new(doc)
}

fn marker_heavy(n: usize) -> Session {
    let mut objects: Vec<_> = (0..n).map(|id| InlineObject::BookmarkEnd { name: id.to_string() }).collect();
    objects.push(InlineObject::CommentStart { id: 7 });
    marked(objects)
}

// CPU time avoids host scheduling delays on Linux. Missing, unchanged or decreasing counters
// use elapsed time; a missing counter must never turn a sample into zero work.
fn cpu_nanos() -> Option<u64> {
    std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse().ok()
}

fn cost(mut work: impl FnMut()) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let start = Instant::now();
        let cpu = cpu_nanos();
        let mut rounds = 0;
        loop {
            work();
            rounds += 1;
            if start.elapsed() >= Duration::from_millis(10) {
                break;
            }
        }
        let used = cpu.zip(cpu_nanos()).and_then(|(a, b)| b.checked_sub(a)).filter(|n| *n > 0);
        best = best.min(used.map(Duration::from_nanos).unwrap_or_else(|| start.elapsed()) / rounds);
    }
    best
}

fn scales(mut small: impl FnMut(), mut large: impl FnMut()) {
    small();
    large();
    let a = cost(small);
    let b = cost(large);
    eprintln!("comment walk, 8x input: {a:?} -> {b:?}");
    assert!(b < a.saturating_mul(24), "8x input must not take quadratic work: {a:?} -> {b:?}");
}

#[test]
fn listing_markers_scales_without_rescanning_prefixes() {
    let a = marker_heavy(2_048);
    let b = marker_heavy(16_384);
    scales(
        || {
            black_box(comment_list(black_box(&a)));
        },
        || {
            black_box(comment_list(black_box(&b)));
        },
    );
}

#[test]
fn caret_targeting_scales_without_rescanning_prefixes() {
    let mut a = marker_heavy(2_048);
    let mut b = marker_heavy(16_384);
    for s in [&mut a, &mut b] {
        s.sel = Selection::caret(s.doc.end_of(StoryRef::Body));
    }
    scales(
        || {
            assert_eq!(black_box(comment_at_caret(black_box(&a))), Some(7));
        },
        || {
            assert_eq!(black_box(comment_at_caret(black_box(&b))), Some(7));
        },
    );
}

#[test]
fn anchor_lookup_scales_without_rescanning_prefixes() {
    let mut a = marker_heavy(2_048);
    let mut b = marker_heavy(16_384);
    let absent = BTreeSet::from([999]);
    scales(
        || {
            remove_anchors(black_box(&mut a), &absent).unwrap();
        },
        || {
            remove_anchors(black_box(&mut b), &absent).unwrap();
        },
    );
}

#[test]
fn unanchored_membership_scales_without_searching_the_growing_list() {
    fn unanchored(n: u32) -> Session {
        let mut s = Session::new(Document::new());
        for id in 0..n {
            s.doc.comments.insert(id, Comment::default());
        }
        s
    }
    let a = unanchored(2_048);
    let b = unanchored(16_384);
    scales(
        || {
            black_box(comment_list(black_box(&a)));
        },
        || {
            black_box(comment_list(black_box(&b)));
        },
    );
}

#[test]
fn list_keeps_duplicate_and_missing_ids_and_numeric_unanchored_order() {
    let mut s = marked(vec![InlineObject::CommentStart { id: 7 }, InlineObject::CommentStart { id: 999 }, InlineObject::CommentStart { id: 7 }]);
    s.doc.comments.insert(4, Comment::default());
    s.doc.comments.insert(2, Comment::default());
    let original = s.doc.clone();
    assert_eq!(comment_list(&s), vec![(7, Some(Pos::body(0, 2))), (999, Some(Pos::body(0, 5))), (7, Some(Pos::body(0, 8))), (2, None), (4, None)]);
    assert_eq!(s.doc, original);
    // Extra objects without text markers and markers without objects stay unreachable.
    s.doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().objects.push(InlineObject::CommentStart { id: 2 });
    assert_eq!(comment_list(&s).last(), Some(&(4, None)));
    s.doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().objects.truncate(1);
    assert_eq!(comment_list(&s), vec![(7, Some(Pos::body(0, 2))), (2, None), (4, None)]);
}

#[test]
fn caret_targeting_keeps_nested_duplicate_and_first_anchor_fallback() {
    let mut s = marked(vec![
        InlineObject::CommentEnd { id: 9 },
        InlineObject::CommentStart { id: 7 },
        InlineObject::CommentStart { id: 8 },
        InlineObject::CommentStart { id: 7 },
        InlineObject::CommentEnd { id: 7 },
    ]);
    for (off, expected) in [(1, None), (2, None), (5, Some(7)), (8, Some(8)), (11, Some(7)), (14, Some(8)), (17, Some(8))] {
        s.sel = Selection::caret(Pos::body(0, off));
        assert_eq!(comment_range_at_caret(&s), expected, "offset {off}");
    }
    s.sel = Selection::caret(Pos::body(0, 0));
    assert_eq!(comment_at_caret(&s), Some(9), "first end marker is the fallback even without a matching start");
}

#[test]
fn anchor_removal_keeps_unrelated_objects_text_and_unreachable_extras() {
    let mut s = marked(vec![
        InlineObject::BookmarkEnd { name: "keep".into() },
        InlineObject::CommentStart { id: 7 },
        InlineObject::CommentEnd { id: 8 },
        InlineObject::CommentEnd { id: 7 },
    ]);
    s.doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().objects.push(InlineObject::CommentStart { id: 7 });
    remove_anchors(&mut s, &BTreeSet::from([7])).unwrap();
    let p = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!(p.text, "é\u{fffc}\u{fffc} tail");
    assert_eq!(
        p.objects,
        vec![InlineObject::BookmarkEnd { name: "keep".into() }, InlineObject::CommentEnd { id: 8 }, InlineObject::CommentStart { id: 7 }]
    );
    assert!(s.doc.comments.contains_key(&7), "the scan helper only removes reachable anchors");
}
