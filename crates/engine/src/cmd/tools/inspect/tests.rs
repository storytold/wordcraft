use super::*;
use std::hint::black_box;
use wordcraft_doc::para::{NoteKind, OBJ, Run};
use wordcraft_doc::props::CharProps;

fn marker_document(n: usize, hidden: bool) -> Document {
    let mut doc = Document::from_text("");
    let p = doc.para_mut(StoryRef::Body, &wordcraft_doc::Path(vec![0])).unwrap();
    p.text = OBJ.to_string().repeat(n);
    p.runs = vec![Run { len: p.len(), props: CharProps::default() }];
    p.objects = vec![InlineObject::BookmarkStart { name: "Original".into() }; n];
    doc.styles.default_chr.hidden = Some(hidden);
    doc
}

// Match the warmed, alternating scaling checks used in layout. Advancing Linux thread CPU
// time excludes scheduler wait; otherwise use the best elapsed sample for each input size.
fn cpu_cost(f: &mut impl FnMut()) -> f64 {
    let ran = || {
        let s = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
        s.split_whitespace().next()?.parse::<u64>().ok()
    };
    cpu_cost_with(ran, f)
}

fn cpu_cost_with(mut ran: impl FnMut() -> Option<u64>, f: &mut impl FnMut()) -> f64 {
    let (t, before) = (std::time::Instant::now(), ran());
    f();
    match (before, ran()) {
        (Some(a), Some(b)) if b > a => (b - a) as f64 * 1e-9,
        _ => t.elapsed().as_secs_f64(),
    }
}

#[test]
fn inspector_cost_measures_work_when_cpu_counters_do_not_advance() {
    let mut s = Session::new(marker_document(64, false));
    for readings in [[Some(0), Some(0)], [Some(10), Some(10)], [Some(10), Some(5)], [None, None], [Some(10), None]] {
        let mut readings = readings.into_iter();
        let cost = cpu_cost_with(|| readings.next().flatten(), &mut || {
            let report = s.run("file.inspect", &json!({})).unwrap();
            assert_eq!(report["found"]["hiddenCount"], 0);
            assert_eq!(report["found"]["unsupported"], json!([]));
            black_box(report);
        });
        assert!(cost > 0.0, "completed inspection work must have a positive elapsed cost: {cost}");
    }
}

fn inspection_cost_is_linear(hidden: bool, alternating: bool) {
    const SCALE: usize = 8;
    let document = |n| {
        let mut doc = marker_document(n, hidden);
        if alternating {
            doc.para_mut(StoryRef::Body, &wordcraft_doc::Path(vec![0])).unwrap().runs =
                (0..n).map(|i| Run { len: OBJ.len_utf8(), props: CharProps { hidden: Some(i % 2 == 0), ..Default::default() } }).collect();
        }
        doc
    };
    let small_doc = document(2_000);
    let big_doc = document(2_000 * SCALE);
    let mut small = Session::new(small_doc.clone());
    let mut big = Session::new(big_doc.clone());
    let check = |s: &mut Session| {
        let report = s.run("file.inspect", &json!({})).unwrap();
        assert_eq!(report["found"]["hiddenCount"], 0);
        assert_eq!(report["found"]["unsupported"], json!([]));
        black_box(report);
    };
    check(&mut small);
    check(&mut big);
    let (mut t1, mut t8) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..5 {
        t1 = t1.min(cpu_cost(&mut || (0..SCALE).for_each(|_| check(&mut small))) / SCALE as f64);
        t8 = t8.min(cpu_cost(&mut || check(&mut big)));
    }
    assert_eq!(small.doc, small_doc);
    assert_eq!(big.doc, big_doc);
    // Eight times as many markers costs about 8x with a walk and 64x with a prefix scan.
    assert!(t8 / t1.max(1e-9) < 24.0, "hidden={hidden}: small={t1}, big={t8}, ratio={}", t8 / t1.max(1e-9));
}

#[test]
fn inspector_unsupported_scan_scales_linearly() {
    inspection_cost_is_linear(false, false);
}

#[test]
fn inspector_hidden_marker_scan_scales_linearly() {
    inspection_cost_is_linear(true, false);
}

#[test]
fn inspector_fragmented_hidden_marker_ranges_scale_linearly() {
    inspection_cost_is_linear(false, true);
}

#[test]
fn hidden_content_distinguishes_markers_from_objects_at_unicode_offsets() {
    let mut p = Paragraph::new();
    p.text = format!("é{OBJ}{OBJ}x{OBJ}");
    p.runs = vec![Run { len: p.len(), props: CharProps::default() }];
    p.objects = vec![
        InlineObject::BookmarkStart { name: "Original".into() },
        InlineObject::NoteRef { kind: NoteKind::Footnote, id: 7, custom: String::new() },
        InlineObject::ControlEnd,
    ];
    for (range, expected) in [(0..2, true), (2..5, false), (5..8, true), (8..9, true), (9..12, false), (2..8, true), (2..2, false), (1..2, false)] {
        let mut offsets = p.objects_with_offsets().filter_map(|(off, object)| (!object.is_marker()).then_some(off)).peekable();
        assert_eq!(has_content(&p, &range, &mut offsets), expected, "{range:?}");
    }
    let mut offsets = p.objects_with_offsets().filter_map(|(off, object)| (!object.is_marker()).then_some(off)).peekable();
    for (range, expected) in [(0..2, true), (2..5, false), (5..8, true), (8..9, true), (9..12, false)] {
        assert_eq!(has_content(&p, &range, &mut offsets), expected, "ordered {range:?}");
    }
    drop(offsets);
    p.objects.truncate(1);
    let mut offsets = p.objects_with_offsets().filter_map(|(off, object)| (!object.is_marker()).then_some(off)).peekable();
    assert!(!has_content(&p, &(5..8), &mut offsets), "an unmatched marker is not content");
}

#[test]
fn inspector_keeps_opaque_report_order_and_excludes_hidden_objects() {
    let mut doc = marker_document(5, false);
    let p = doc.para_mut(StoryRef::Body, &wordcraft_doc::Path(vec![0])).unwrap();
    p.text.insert(0, 'é');
    let opaque = |format: &str| InlineObject::Opaque { format: format.into(), xml: "<w:r><w:vanish/></w:r>".into(), text: "Original".into() };
    p.objects = vec![
        InlineObject::BookmarkStart { name: "Original".into() },
        opaque("first"),
        InlineObject::NoteRef { kind: NoteKind::Footnote, id: 7, custom: String::new() },
        opaque("second"),
        opaque("hidden"),
    ];
    p.runs = vec![
        Run { len: 2 + 4 * OBJ.len_utf8(), props: CharProps::default() },
        Run { len: OBJ.len_utf8(), props: CharProps { hidden: Some(true), ..Default::default() } },
    ];
    let mut s = Session::new(doc.clone());
    let report = s.run("file.inspect", &json!({})).unwrap();
    let entry = |format: &str| json!({"story": "body", "object": "opaque", "format": format, "reason": "hidden text inside an object kept from the source file"});
    assert_eq!(report["found"]["unsupported"], json!([entry("first"), entry("second")]));
    assert_eq!(report["found"]["hiddenCount"], 1);
    assert_eq!(s.doc, doc);
}
