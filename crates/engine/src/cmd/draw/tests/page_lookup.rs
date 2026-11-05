use super::super::{PlacedInk, ink_on};
use crate::Session;
use std::sync::Arc;
use wordcraft_doc::freeform::{Freeform, InkTool};
use wordcraft_doc::para::{Float, InlineObject, OBJ, ShapeKind, Wrap};
use wordcraft_doc::{CharProps, Document, Paragraph, Path, Pos, StoryRef, para_block};
use wordcraft_geom::{Rect, Spin};
use wordcraft_layout::{DocLayout, Page, Placed};

fn stroke(order: u32) -> InlineObject {
    InlineObject::Shape {
        kind: ShapeKind::Freeform,
        w: 20.0,
        h: 10.0,
        fill: None,
        stroke: Some(wordcraft_doc::Rgb(12, 34, 56)),
        stroke_width: 2.0,
        float: Float { wrap: Wrap::InFrontOfText, ..Default::default() },
        story: None,
        freeform: Some(Arc::new(Freeform { order: Some(order), ..Freeform::ink(InkTool::Pen, 20.0, 10.0, vec![[1.0, 1.0], [19.0, 9.0]]) })),
        effects: Default::default(),
        extra: Default::default(),
    }
}

fn placed(story: StoryRef, path: Path, off: usize, x: f32) -> Placed {
    Placed::Object {
        rect: Rect::new(x, 30.0, 20.0, 10.0),
        spin: Spin::default(),
        story,
        path,
        off,
        text_box: None,
        wrap: Wrap::InFrontOfText,
        origin: Default::default(),
    }
}

fn orders(ink: &[PlacedInk]) -> Vec<u32> {
    ink.iter().map(|k| k.freeform.order.unwrap()).collect()
}

#[test]
fn page_ink_keeps_item_order_duplicates_story_paths_and_unmatched_storage() {
    let mut d = Document::new();
    let mut p = Paragraph::new();
    p.text = format!("é{OBJ}{OBJ}{OBJ}漢{OBJ}");
    p.runs = vec![wordcraft_doc::Run { len: p.text.len(), props: CharProps::default() }];
    p.objects = vec![InlineObject::BookmarkStart { name: "original".into() }, stroke(11), stroke(22)];
    d.body = vec![para_block(p.clone())];
    let h = d.add_part(wordcraft_doc::PartKind::Header, vec![para_block(p.clone())]);
    let mut table = wordcraft_doc::Table::new(1, 1, 100.0);
    p.objects.push(stroke(33));
    p.objects.push(stroke(99)); // Surplus storage has no character and is not placed.
    table.rows[0].cells[0].blocks = vec![para_block(p)];
    d.body.push(Arc::new(wordcraft_doc::Block::Table(table)));
    let cell = Path(vec![1, 0, 0, 0]);
    let mut items = vec![
        placed(StoryRef::Body, Path::top(0), 8, 50.0),
        placed(StoryRef::Part(h), Path::top(0), 5, 60.0),
        placed(StoryRef::Body, cell.clone(), 14, 70.0),
        placed(StoryRef::Body, Path::top(0), 5, 80.0),
        placed(StoryRef::Body, Path::top(0), 8, 90.0),
    ];
    items.extend([0, 1, 2, 3, 4, 11, 14, usize::MAX].map(|off| placed(StoryRef::Body, Path::top(0), off, 0.0)));
    items.push(placed(StoryRef::Part(999), Path::top(0), 0, 0.0));
    let l = DocLayout { pages: vec![Page { items, ..Default::default() }], ..Default::default() };
    let mut s = Session::new(d);
    let original = s.doc.clone();
    let got = ink_on(&s, &l, 0);
    assert_eq!(orders(&got), [22, 11, 33, 11, 22]);
    assert_eq!(got.iter().map(|k| k.rect.x).collect::<Vec<_>>(), [50.0, 60.0, 70.0, 80.0, 90.0]);
    assert_eq!(got[1].pos, Pos::new(StoryRef::Part(h), Path::top(0), 5));
    assert_eq!(got[2].pos.path, cell);
    assert!(got.iter().all(|k| k.width == 2.0 && k.color == Some(wordcraft_doc::Rgb(12, 34, 56))));
    assert!(ink_on(&s, &l, 1).is_empty());
    s.view.hide_ink = true;
    assert_eq!(orders(&ink_on(&s, &l, 0)), orders(&got), "discovery retains hidden ink; callers decide visibility");
    assert_eq!(s.doc, original);
}

#[test]
fn page_ink_lookup_work_does_not_multiply_with_paragraph_objects() {
    fn fixture(n: usize) -> (Session, DocLayout) {
        let mut d = Document::new();
        let mut p = Paragraph::new();
        p.text = OBJ.to_string().repeat(n);
        p.runs = vec![wordcraft_doc::Run { len: p.text.len(), props: CharProps::default() }];
        p.objects = (0..n).map(|i| stroke(i as u32)).collect();
        d.body = vec![para_block(p)];
        let items = (0..n).map(|i| placed(StoryRef::Body, Path::top(0), i * OBJ.len_utf8(), 50.0)).collect();
        (Session::new(d), DocLayout { pages: vec![Page { items, ..Default::default() }], ..Default::default() })
    }
    // Scheduling delays do not count. Without an advancing Linux thread CPU clock, the
    // semantic assertions still run and only the scaling check is skipped.
    fn ran() -> Option<u64> {
        std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse().ok()
    }
    fn cost(mut f: impl FnMut()) -> Option<f64> {
        let before = ran()?;
        for _ in 0..32 {
            f();
        }
        let elapsed = ran()?.checked_sub(before)?;
        (elapsed > 0).then_some(elapsed as f64 / 32.0)
    }
    let (small, a) = fixture(250);
    let (big, b) = fixture(2000);
    assert_eq!(ink_on(&small, &a, 0).len(), 250);
    assert_eq!(ink_on(&big, &b, 0).len(), 2000);
    let mut t1 = f64::INFINITY;
    let mut t8 = f64::INFINITY;
    for _ in 0..5 {
        let Some(small_cost) = cost(|| {
            for _ in 0..8 {
                std::hint::black_box(ink_on(&small, &a, 0));
            }
        }) else {
            return;
        };
        let Some(big_cost) = cost(|| {
            std::hint::black_box(ink_on(&big, &b, 0));
        }) else {
            return;
        };
        t1 = t1.min(small_cost / 8.0);
        t8 = t8.min(big_cost);
    }
    let ratio = t8 / t1;
    eprintln!("page ink CPU ratio={ratio:.2} for 8x objects");
    assert!(ratio < 24.0, "page ink lookup repeats paragraph-prefix work: {ratio:.2}x CPU for 8x objects");
}
