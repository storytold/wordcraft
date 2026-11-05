use super::only_markers;
use wordcraft_doc::Paragraph;
use wordcraft_doc::para::{InlineObject, NoteKind, OBJ, Run};

#[test]
fn marker_ranges_keep_byte_boundaries_and_object_pairing() {
    let mut objects = vec![
        InlineObject::BookmarkStart { name: "Original".into() },
        InlineObject::BookmarkEnd { name: "Original".into() },
        InlineObject::CommentStart { id: 1 },
        InlineObject::CommentEnd { id: 1 },
        InlineObject::PermStart { id: 2, group: "everyone".into(), editor: String::new() },
        InlineObject::PermEnd { id: 2 },
        InlineObject::FieldStart { instr: "REF Original".into(), locked: false, dirty: false },
        InlineObject::FieldEnd,
        InlineObject::ControlStart { control: Default::default() },
        InlineObject::ControlEnd,
    ];
    let end = 2 + objects.len() * OBJ.len_utf8();
    let text = format!("é{}z{OBJ}", OBJ.to_string().repeat(objects.len()));
    objects.push(InlineObject::NoteRef { kind: NoteKind::Footnote, id: 0, custom: String::new() });
    let mut p = Paragraph { runs: vec![Run { len: text.len(), props: Default::default() }], text, objects, ..Default::default() };
    assert!(only_markers(&p, 0..0));
    assert!(only_markers(&p, 2..end));
    for start in (2..end).step_by(OBJ.len_utf8()) {
        assert!(only_markers(&p, start..end));
        assert!(only_markers(&p, start..start + OBJ.len_utf8()));
    }
    assert!(only_markers(&p, p.len()..p.len()));
    for range in [0..end, 1..1, 3..end, 2..end - 1, end..p.len(), p.len() + 1..p.len() + 1, 0..usize::MAX] {
        assert!(!only_markers(&p, range.clone()), "{range:?}");
    }
    p.objects.pop();
    assert!(!only_markers(&p, end + 1..p.len()), "a marker with no object is not empty");
    p.objects.push(InlineObject::FieldEnd);
    p.objects.push(InlineObject::NoteRef { kind: NoteKind::Endnote, id: 0, custom: String::new() });
    assert!(only_markers(&p, end + 1..p.len()), "an unpaired extra object does not belong to the range");
    p.objects[0] = InlineObject::NoteRef { kind: NoteKind::Endnote, id: 0, custom: String::new() };
    assert!(!only_markers(&p, 2..end));
    assert!(only_markers(&p, 5..end), "a visible object before the range does not belong to it");
}

#[test]
fn marker_only_suffix_scan_does_not_repeat_paragraph_prefix_work() {
    let paragraph = |n: usize| {
        let text = format!("é{}\u{000c}{}", OBJ.to_string().repeat(n), OBJ.to_string().repeat(n));
        let start = 3 + n * OBJ.len_utf8();
        let p = Paragraph {
            runs: vec![Run { len: text.len(), props: Default::default() }],
            text,
            objects: vec![InlineObject::FieldEnd; 2 * n],
            ..Default::default()
        };
        (p, start)
    };
    let (small, small_start) = paragraph(2_000);
    let (big, big_start) = paragraph(16_000);
    let (t1, tk, ratio) = crate::tests::cost_ratio(
        8,
        || assert!(std::hint::black_box(only_markers(&small, small_start..small.len()))),
        || assert!(std::hint::black_box(only_markers(&big, big_start..big.len()))),
    );
    eprintln!("8x markers cost {ratio:.2}x: {t1:.6}s -> {tk:.6}s");
    assert!(ratio < 20.0, "8x markers cost {ratio:.2}x: {t1:.6}s -> {tk:.6}s");
}
