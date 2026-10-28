use super::*;
use wordcraft_doc::para::{OBJ, Run};
use wordcraft_doc::{Document, Paragraph, Path, para_block};

fn markers(objects: Vec<InlineObject>) -> Paragraph {
    let mut p = Paragraph::new();
    p.text = format!("é{}tail", OBJ.to_string().repeat(objects.len()));
    p.runs = vec![Run { len: p.len(), props: CharProps::default() }];
    p.objects = objects;
    p
}

fn dense(n: usize, unique: bool) -> Document {
    let mut doc = Document::new();
    doc.body = vec![para_block(markers(
        (0..n).map(|i| InlineObject::BookmarkStart { name: if unique { format!("Mark{i:06}") } else { "Other".into() } }).collect(),
    ))];
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
fn bookmark_cost_measures_work_when_cpu_counters_do_not_advance() {
    let mut s = Session::new(dense(64, true));
    for readings in [[Some(0), Some(0)], [Some(10), Some(10)], [Some(10), Some(5)], [None, None], [Some(10), None]] {
        let mut readings = readings.into_iter();
        let cost = cpu_cost_with(|| readings.next().flatten(), &mut || {
            let result = link_targets(&mut s, &json!({})).unwrap();
            assert_eq!(result["bookmarks"].as_array().unwrap().len(), 64);
            std::hint::black_box(result);
        });
        assert!(cost > 0.0, "completed bookmark work must have a positive elapsed cost: {cost}");
    }
}

fn linear(mut small: impl FnMut(), mut big: impl FnMut()) {
    small();
    big();
    let (mut t1, mut t8) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..5 {
        t1 = t1.min(cpu_cost(&mut || (0..8).for_each(|_| small())) / 8.0);
        t8 = t8.min(cpu_cost(&mut big));
    }
    let ratio = t8 / t1.max(1e-9);
    eprintln!("bookmark work: small={t1:.6}s big={t8:.6}s ratio={ratio:.2}");
    assert!(ratio < 24.0, "8x markers should not cost quadratic work: {ratio:.2}x");
}

#[test]
fn bookmark_target_names_scale_linearly() {
    let check = |s: &mut Session, n| {
        let result = link_targets(s, &json!({})).unwrap();
        let names = result["bookmarks"].as_array().unwrap();
        assert_eq!(names.len(), n);
        assert_eq!(names[0], "Mark000000");
        assert_eq!(names[n - 1], format!("Mark{:06}", n - 1));
        std::hint::black_box(result);
    };
    let mut small = Session::new(dense(2_000, true));
    let mut big = Session::new(dense(16_000, true));
    linear(|| check(&mut small, 2_000), || check(&mut big, 16_000));
}

#[test]
fn bookmark_removal_scan_scales_linearly() {
    let mut small = Session::new(dense(2_000, false));
    let mut big = Session::new(dense(16_000, false));
    linear(|| assert!(!remove_bookmark(&mut small, "Missing").unwrap()), || assert!(!remove_bookmark(&mut big, "Missing").unwrap()));
}

#[test]
fn bookmark_span_scan_scales_linearly() {
    let document = |n| {
        let mut doc = dense(n, false);
        let p = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        p.objects[0] = InlineObject::BookmarkStart { name: "Target".into() };
        p.objects[n - 1] = InlineObject::BookmarkEnd { name: "Target".into() };
        doc
    };
    let small = document(2_000);
    let big = document(16_000);
    let check = |doc: &Document, n: usize| {
        assert_eq!(bookmark_span(doc, "Target", &Pos::body(0, 2)), (Pos::body(0, 5), Pos::body(0, 2 + (n - 1) * OBJ.len_utf8())));
    };
    linear(|| check(&small, 2_000), || check(&big, 16_000));
}

#[test]
fn bookmark_targets_keep_first_visible_names_in_body_order() {
    let mut doc = Document::new();
    doc.body = vec![para_block(markers(
        ["Zulu", "_Hidden", "Alpha", "Zulu", "alpha", "École", "_Hidden", "Alpha"]
            .into_iter()
            .map(|name| InlineObject::BookmarkStart { name: name.into() })
            .collect(),
    ))];
    let original = doc.clone();
    let mut s = Session::new(doc);
    let before = s.sel.clone();
    let result = s.run("link.targets", &json!({})).unwrap();
    assert_eq!(result, json!({"top": true, "headings": [], "bookmarks": ["Zulu", "Alpha", "alpha", "École"]}));
    assert_eq!(s.doc, original);
    assert_eq!(s.sel, before);
    assert!(s.undo_labels().is_empty());
}

#[test]
fn bookmark_span_keeps_first_later_end_and_missing_end_fallback() {
    let mut doc = Document::new();
    doc.body = vec![
        para_block(markers(vec![InlineObject::BookmarkEnd { name: "Target".into() }, InlineObject::BookmarkStart { name: "Target".into() }])),
        para_block(markers(vec![InlineObject::BookmarkEnd { name: "Other".into() }, InlineObject::BookmarkEnd { name: "Target".into() }])),
        para_block(markers(vec![InlineObject::BookmarkEnd { name: "Target".into() }])),
    ];
    assert_eq!(bookmark_range(&doc, "Target"), Some((Pos::body(0, 8), Pos::body(1, 5))));
    assert_eq!(bookmark_span(&doc, "Missing", &Pos::body(0, 5)), (Pos::body(0, 8), Pos::body(0, 8)));
    assert!(bookmark_range(&doc, "Missing").is_none());
    // Unpaired U+FFFC markers have no associated object and cannot become an endpoint.
    let p = doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
    p.objects.truncate(1);
    doc.body.truncate(2);
    assert_eq!(bookmark_range(&doc, "Target"), Some((Pos::body(0, 8), Pos::body(0, 8))));
}
