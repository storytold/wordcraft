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
    Session::new(doc)
}

fn reference(kind: NoteKind, id: u32) -> InlineObject {
    InlineObject::NoteRef { kind, id, custom: String::new() }
}

fn marker_heavy(n: usize) -> Session {
    let mut objects: Vec<_> = (0..n).map(|id| InlineObject::BookmarkEnd { name: id.to_string() }).collect();
    objects.push(reference(NoteKind::Footnote, 7));
    marked(objects)
}

fn cpu_nanos() -> Option<u64> {
    std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse().ok()
}

// Linux CPU time avoids host scheduling delays. A missing or non-advancing counter uses
// elapsed time so an unavailable counter cannot turn a sample into zero work.
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

#[test]
fn reference_scan_scales_without_rescanning_object_prefixes() {
    let small = marker_heavy(2_048);
    let large = marker_heavy(16_384);
    let scan = |s: &Session| {
        let refs = note_refs(black_box(s), Some(NoteKind::Footnote));
        assert_eq!(refs.len(), 1);
        black_box(refs);
    };
    scan(&small);
    scan(&large);
    let a = cost(|| scan(&small));
    let b = cost(|| scan(&large));
    eprintln!("note reference walk, 8x input: {a:?} -> {b:?}");
    assert!(b < a.saturating_mul(24), "8x markers must not take quadratic work: {a:?} -> {b:?}");
}

#[test]
fn reference_scan_preserves_kinds_unicode_order_and_missing_objects() {
    let objects = vec![
        reference(NoteKind::Endnote, 1),
        InlineObject::BookmarkEnd { name: "unrelated".into() },
        reference(NoteKind::Footnote, 7),
        reference(NoteKind::Footnote, 7),
        reference(NoteKind::Endnote, 2),
    ];
    let mut s = marked(objects);
    let positions = |offsets: &[usize]| offsets.iter().map(|&off| Pos::body(0, off)).collect::<Vec<_>>();
    assert_eq!(note_refs(&s, None), positions(&[2, 8, 11, 14]));
    assert_eq!(note_refs(&s, Some(NoteKind::Footnote)), positions(&[8, 11]));
    assert_eq!(note_refs(&s, Some(NoteKind::Endnote)), positions(&[2, 14]));
    // An unmatched text marker has no object; an extra object has no position. Neither is
    // synthesized into a reference, and a reference to an absent note part still has its position.
    let p = s.doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.text.push('\u{fffc}');
    p.runs.first_mut().unwrap().len = p.text.len();
    assert_eq!(note_refs(&s, None), positions(&[2, 8, 11, 14]));
    let p = s.doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.text.truncate(14);
    p.runs.first_mut().unwrap().len = p.text.len();
    assert_eq!(note_refs(&s, None), positions(&[2, 8, 11]));
}

#[test]
fn next_footnote_keeps_wraparound_and_empty_document_state() {
    let mut s = marked(vec![reference(NoteKind::Endnote, 1), reference(NoteKind::Footnote, 7), reference(NoteKind::Footnote, 9)]);
    let before = s.doc.clone();
    let rev = s.rev();
    s.sel = Selection::caret(Pos::body(0, 0));
    for off in [5, 8, 5] {
        s.run("references.nextFootnote", &json!({})).unwrap();
        assert_eq!(s.sel, Selection::caret(Pos::body(0, off)));
    }
    assert_eq!(s.doc, before);
    assert_eq!(s.rev(), rev);
    let mut empty = Session::new(Document::from_text("Only text."));
    let selection = empty.sel.clone();
    empty.run("references.nextFootnote", &json!({})).unwrap();
    assert_eq!(empty.sel, selection);
}
