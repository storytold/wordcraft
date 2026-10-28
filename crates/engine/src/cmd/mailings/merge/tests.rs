use super::*;
use wordcraft_doc::{
    para::{OBJ, Run},
    props::CharProps,
};

fn raw_paragraph(text: &str, props: CharProps) -> Paragraph {
    let mut p = Paragraph::new();
    p.text = text.into();
    p.runs = vec![Run { len: p.len(), props }];
    p
}

fn field(instr: &str) -> InlineObject {
    InlineObject::Field { instr: instr.into(), result: "cached".into(), locked: false, dirty: false, code: false, form: String::new() }
}

fn recipients() -> Session {
    let mut s = Session::new(Document::new());
    s.run("mailings.recipients", &json!({"rows": [{"Name": "Ada"}, {"Name": "Bo"}]})).unwrap();
    s
}

/// Use advancing thread CPU time where available; missing or nonadvancing counters use elapsed time.
fn cost(f: impl FnOnce()) -> f64 {
    let cpu = || std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse::<u64>().ok();
    let (start, before) = (std::time::Instant::now(), cpu());
    f();
    match (before, cpu()) {
        (Some(a), Some(b)) if b > a => (b - a) as f64 * 1e-9,
        _ => start.elapsed().as_secs_f64(),
    }
}

#[test]
fn merge_discovery_scales_with_unrelated_markers() {
    let dense = |n| {
        let mut p = raw_paragraph(&format!("{OBJ}é{}漢", OBJ.to_string().repeat(n)), CharProps::default());
        p.objects = vec![field("MERGEFIELD Name")];
        p.objects.extend(vec![InlineObject::BookmarkStart { name: "Keep".into() }; n]);
        vec![para_block(p)]
    };
    let s = recipients();
    let (small, large) = (dense(2_000), dense(16_000));
    let measure = |input: &Blocks, n| {
        let mut blocks = input.clone();
        let mut row = 0;
        let time = cost(|| assert!(merge_blocks(&s, &mut blocks, &mut row, 2, 0)));
        let Block::Para(p) = &*blocks[0] else { panic!() };
        assert_eq!(p.text, format!("Adaé{}漢", OBJ.to_string().repeat(n)));
        assert_eq!(p.objects.len(), n);
        assert!(p.objects.iter().all(|o| matches!(o, InlineObject::BookmarkStart { name } if name == "Keep")));
        assert_eq!(row, 0);
        time
    };
    measure(&small, 2_000);
    measure(&large, 16_000);
    let (mut short, mut long) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..5 {
        short = short.min((0..8).map(|_| measure(&small, 2_000)).sum::<f64>() / 8.0);
        long = long.min(measure(&large, 16_000));
    }
    let ratio = long / short.max(1e-9);
    assert!(ratio < 24.0, "merge object discovery: {short:.6}s -> {long:.6}s, {ratio:.2}x for 8x markers");
}

#[test]
fn merge_discovery_keeps_unicode_rules_and_mismatched_objects() {
    let s = recipients();
    let mut p = raw_paragraph(&format!("é{OBJ}|{OBJ}|{OBJ}|{OBJ}|{OBJ}|{OBJ}漢"), CharProps { bold: Some(true), ..Default::default() });
    p.objects = vec![
        InlineObject::BookmarkStart { name: "Keep".into() },
        field("MERGEFIELD Name"),
        field("NEXT"),
        field("MERGEFIELD Name"),
        InlineObject::FieldStart { instr: "MERGEFIELD Name".into(), locked: false, dirty: false },
        InlineObject::FieldEnd,
        field("SKIPIF { MERGEFIELD Name } = \"Bo\""),
    ];
    // The surplus SKIPIF has no text marker and must not run.
    let mut blocks = vec![para_block(p)];
    let mut row = 0;
    assert!(merge_blocks(&s, &mut blocks, &mut row, 2, 0));
    let Block::Para(p) = &*blocks[0] else { panic!() };
    assert_eq!(p.text, format!("é{OBJ}|Ada||Bo|{OBJ}|{OBJ}漢"));
    assert_eq!(row, 1);
    assert_eq!(p.objects.len(), 4);
    assert!(p.runs.iter().all(|r| r.props.bold == Some(true)));
    assert!(matches!(p.objects.last(), Some(InlineObject::Field { instr, .. }) if instr.starts_with("SKIPIF")));

    let mut p = raw_paragraph(&format!("é{OBJ}漢{OBJ}"), CharProps::default());
    p.objects = vec![field("MERGEFIELD Name")];
    let mut blocks = vec![para_block(p)];
    assert!(merge_blocks(&s, &mut blocks, &mut 0, 2, 0));
    let Block::Para(p) = &*blocks[0] else { panic!() };
    assert_eq!(p.text, format!("éAda漢{OBJ}"));
    assert!(p.objects.is_empty());
}
