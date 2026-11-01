use super::*;
use wordcraft_doc::para::{Float, InlineObject, ShapeKind, Wrap};
use wordcraft_doc::{Document, Paragraph, PartKind, Pos, para_block};

fn shape(id: u32, w: f32) -> InlineObject {
    InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w,
        h: 2000.0,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Float { wrap: Wrap::InFrontOfText, ..Default::default() },
        story: Some(id),
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    }
}

#[derive(Clone, Copy, Debug)]
enum Graph {
    Shared,
    Varied,
    Linked,
    Long,
}

#[derive(Clone, Copy)]
enum Fit {
    Fixed,
    Auto,
}

fn shared_graph(graph: Graph, fit: Fit) -> (Document, u32) {
    let mut d = Document::from_text("");
    d.styles.default_chr.font = Some("Source Sans 3".into());
    let ids: Vec<_> =
        (0..4).map(|k| d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text(&format!("Level{k}"), Default::default()))])).collect();
    for id in &ids {
        d.parts.get_mut(id).unwrap().body = TextBody { auto_fit: matches!(fit, Fit::Auto), inset: Some([0; 4]), ..Default::default() };
    }
    if matches!(graph, Graph::Long) {
        d.parts.get_mut(&ids[3]).unwrap().blocks = Document::from_text(&"Long story line.\n".repeat(63)).body;
    }
    if matches!(graph, Graph::Linked) {
        let tail = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("", Default::default()))]);
        d.parts.get_mut(&ids[3]).unwrap().body.next = Some(tail);
        d.insert_object(&Pos::new(StoryRef::Part(ids[2]), Path::top(0), 0), shape(tail, 180.0), &Default::default()).unwrap();
    }
    for pair in ids.windows(2) {
        for k in 0..wordcraft_doc::BoxBudget::MAX_NESTED {
            let w = if matches!(graph, Graph::Shared) { 180.0 } else { 120.0 + (k % 3) as f32 * 8.0 };
            d.insert_object(&Pos::new(StoryRef::Part(pair[0]), Path::top(0), 0), shape(pair[1], w), &Default::default()).unwrap();
        }
    }
    d.insert_object(&Pos::body(0, 0), shape(ids[0], 240.0), &Default::default()).unwrap();
    (d, ids[0])
}

fn layout(d: &Document) -> crate::DocLayout {
    crate::layout(d, &mut crate::LayoutCache::new(), &crate::LayoutOptions::default())
}

fn visible(l: &crate::DocLayout) -> HashMap<StoryRef, usize> {
    let mut out = HashMap::new();
    for item in l.pages.iter().flat_map(|p| &p.items) {
        if let Placed::Lines { story, l0, l1, .. } = item {
            *out.entry(*story).or_default() += l1 - l0;
        }
    }
    out
}

#[test]
fn shared_probe_work_is_capped() {
    for graph in [Graph::Shared, Graph::Varied, Graph::Linked, Graph::Long] {
        let (control, _) = shared_graph(graph, Fit::Fixed);
        let expected = layout(&control);
        let (d, root) = shared_graph(graph, Fit::Auto);
        MEASURE_COUNTS.with(|counts| counts.borrow_mut().clear());
        COPIED_BLOCKS.with(|count| count.set(0));
        let l = layout(&d);
        let counts = MEASURE_COUNTS.with(|counts| std::mem::take(&mut *counts.borrow_mut()));
        assert!(counts.contains(&MAX_MEASURE_BOXES), "the hostile graph exhausts measurement work: {counts:?}");
        assert!(counts.len() <= 2 && counts.iter().all(|n| *n <= MAX_MEASURE_BOXES), "{counts:?}");
        assert!(counts.iter().sum::<usize>() <= 2 * MAX_MEASURE_BOXES);
        let copied = COPIED_BLOCKS.with(std::cell::Cell::get);
        let max_blocks = d.parts.values().map(|p| p.blocks.len()).max().unwrap();
        assert!(copied <= counts.iter().sum::<usize>() * max_blocks, "preparation only follows charged work: {copied}");
        assert_eq!(visible(&l), visible(&expected), "probe exhaustion never spends actual text-placement allowance");
        assert_eq!(l.text_boxes, expected.text_boxes);
        assert!(l.text_boxes <= 1 + wordcraft_doc::BoxBudget::MAX_NESTED);
        let frame = l
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .find_map(|it| match it {
                Placed::Object { rect, text_box: Some(id), .. } if *id == root => Some(*rect),
                _ => None,
            })
            .unwrap();
        assert_eq!((frame.w, frame.h), (240.0, 2000.0), "an incomplete probe keeps its saved frame");
        eprintln!("PROBE_WORK graph={graph:?} phases={counts:?} copied_blocks={copied} actual_boxes={}", l.text_boxes);
    }
}

#[test]
fn rejected_probe_copies_no_story() {
    let (d, _) = shared_graph(Graph::Linked, Fit::Auto);
    let mut cache = crate::LayoutCache::new();
    let opts = crate::LayoutOptions::default();
    let mut ctx = Ctx {
        doc: &d,
        hf: Arc::new(crate::headers::HfLinks::new(d.sections())),
        alphas: Default::default(),
        floats_above: HashMap::new(),
        cache: &mut cache,
        opts: &opts,
        counters: Default::default(),
        fields: Default::default(),
        notes_hash: 0,
        numbers: HashMap::new(),
        eq_count: 0,
        boxes: Default::default(),
        chains: Default::default(),
        grid: Default::default(),
        hide_deleted: opts.hides_deleted(StoryRef::Body),
        uses_totals: false,
        track_totals: true,
        pct_base: None,
        quiet_marks: Vec::new(),
        stacked_marks: Vec::new(),
        mirror: false,
        table_keys: Vec::new(),
        pin: None,
        footer_wraps: HashMap::new(),
        header_tables: HashMap::new(),
        frames: Default::default(),
        fitted: HashMap::new(),
        fitted_order: Vec::new(),
        measure_left: 3,
        measure_depth: 1,
        measure_exhausted: false,
    };
    ctx.chains.build(ctx.doc);
    let head = *ctx.chains.chains[0].first().unwrap();
    MEASURE_COUNTS.with(|counts| *counts.borrow_mut() = vec![0]);
    COPIED_BLOCKS.with(|count| count.set(0));
    // Chain copy, head entry and first width consume three units. The next width is rejected.
    assert!(chain_share(&mut ctx, head, 0).unwrap().is_empty());
    assert!(ctx.measure_exhausted);
    assert_eq!(ctx.measure_left, 0);
    assert!(ctx.chains.flows.is_empty(), "partial linked flow must not be cached");
    assert!(!ctx.boxes.inside(), "an interrupted flow leaves its head");
    let copied = COPIED_BLOCKS.with(std::cell::Cell::get);
    assert_eq!(copied, 1);
    let obj = shape(1, 240.0);
    for _ in 0..2000 {
        assert!(chain_share(&mut ctx, head, 0).unwrap().is_empty());
        assert_eq!(fit_size(&mut ctx, &obj, 240.0, 2000.0, 0), (240.0, 2000.0));
    }
    assert_eq!(COPIED_BLOCKS.with(std::cell::Cell::get), copied, "rejected probes never clone Blocks");
    assert_eq!(MEASURE_COUNTS.with(|counts| counts.borrow()[0]), 3);
}

fn with_probe_ctx(d: &Document, f: impl FnOnce(&mut Ctx)) {
    let mut cache = crate::LayoutCache::new();
    let opts = crate::LayoutOptions::default();
    let mut ctx = Ctx {
        doc: d,
        hf: Arc::new(crate::headers::HfLinks::new(d.sections())),
        alphas: Default::default(),
        floats_above: HashMap::new(),
        cache: &mut cache,
        opts: &opts,
        counters: Default::default(),
        fields: Default::default(),
        notes_hash: 0,
        numbers: HashMap::new(),
        eq_count: 0,
        boxes: Default::default(),
        chains: Default::default(),
        grid: Default::default(),
        hide_deleted: opts.hides_deleted(StoryRef::Body),
        uses_totals: false,
        track_totals: true,
        pct_base: None,
        quiet_marks: Vec::new(),
        stacked_marks: Vec::new(),
        mirror: false,
        table_keys: Vec::new(),
        pin: None,
        footer_wraps: HashMap::new(),
        header_tables: HashMap::new(),
        frames: Default::default(),
        fitted: HashMap::new(),
        fitted_order: Vec::new(),
        measure_left: 0,
        measure_depth: 0,
        measure_exhausted: false,
    };
    f(&mut ctx);
}

fn inline_shape(id: u32, w: f32) -> InlineObject {
    let mut obj = shape(id, w);
    if let InlineObject::Shape { float, .. } = &mut obj {
        float.wrap = Wrap::Inline;
    }
    obj
}

#[test]
fn root_none_is_reused() {
    let (mut d, root) = shared_graph(Graph::Shared, Fit::Auto);
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.objects[0] = inline_shape(root, 240.0);
    p.touch();
    let p = d.para(StoryRef::Body, &Path::top(0)).unwrap();
    with_probe_ctx(&d, |ctx| {
        MEASURE_COUNTS.with(|counts| counts.borrow_mut().clear());
        COPIED_BLOCKS.with(|count| count.set(0));
        let first = ctx.para_labelled(p, 500.0, None, &[], None);
        let before = MEASURE_COUNTS.with(|counts| counts.borrow().clone());
        assert_eq!(before, [MAX_MEASURE_BOXES]);
        assert!(ctx.measure_exhausted);
        let copied = COPIED_BLOCKS.with(std::cell::Cell::get);
        for _ in 0..4 {
            let repeated = ctx.para_labelled(p, 500.0, None, &[], None);
            assert!(Arc::ptr_eq(&first, &repeated));
            let measured = ctx.para_measured(p, 500.0, None, None, |pl| (pl.height, pl.height));
            assert_eq!(measured, (first.height, first.height));
        }
        let after = MEASURE_COUNTS.with(|counts| counts.borrow().clone());
        eprintln!(
            "ROOT_PROBE_WORK before={before:?} after={after:?} copied_before={copied} copied_after={}",
            COPIED_BLOCKS.with(std::cell::Cell::get)
        );
        assert_eq!(after, before, "repeated layout and table measurement reuse the exhausted root result");
        assert_eq!(COPIED_BLOCKS.with(std::cell::Cell::get), copied);
        let key = (root, 240.0f32.to_bits());
        assert_eq!(ctx.fitted.get(&key), Some(&None));
        assert_eq!(ctx.fitted_order, [key]);
        assert_eq!(ctx.measure_depth, 0);
        assert!(!ctx.boxes.inside());
        assert_eq!(ctx.boxes.total(), 0);
        assert!(ctx.chains.flows.is_empty());
    });
    let mut fixed = d.clone();
    for part in fixed.parts.values_mut() {
        part.body.auto_fit = false;
    }
    let expected = layout(&fixed);
    let actual = layout(&d);
    assert_eq!(visible(&actual), visible(&expected));
    assert_eq!(actual.text_boxes, expected.text_boxes);
    assert!(actual.text_boxes <= 1 + wordcraft_doc::BoxBudget::MAX_NESTED);
}

#[derive(Clone, Copy, Debug)]
enum Nest {
    ActualBox,
    Probe,
}

pub(crate) fn nested_none_is_not_cached(fitted_boxes: impl Fn(&mut Ctx, &Paragraph) -> Vec<(usize, f32)>) {
    let mut d = Document::from_text("");
    d.styles.default_chr.font = Some("Source Sans 3".into());
    let list = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    let parent = d.add_part(PartKind::TextBox, vec![]);
    let leaf = d.add_part(PartKind::TextBox, Document::from_text("Visible child").body);
    let root = d.add_part(PartKind::TextBox, Document::from_text("Numbered root").body);
    for id in [root, leaf] {
        d.parts.get_mut(&id).unwrap().body = TextBody { auto_fit: true, inset: Some([0; 4]), ..Default::default() };
    }
    d.para_mut(StoryRef::Part(root), &Path::top(0)).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num: list, level: 0 });
    d.insert_object(&Pos::new(StoryRef::Part(root), Path::top(0), 0), inline_shape(leaf, 180.0), &Default::default()).unwrap();
    d.insert_object(&Pos::body(0, 0), inline_shape(root, 240.0), &Default::default()).unwrap();
    let p = d.para(StoryRef::Body, &Path::top(0)).unwrap();
    for nest in [Nest::ActualBox, Nest::Probe] {
        with_probe_ctx(&d, |ctx| {
            match nest {
                Nest::ActualBox => assert!(ctx.boxes.enter(parent)),
                Nest::Probe => ctx.measure_depth = 1,
            }
            ctx.eq_count = 7;
            let before = (ctx.boxes.total(), ctx.measure_depth);
            ctx.measure_left = 1;
            MEASURE_COUNTS.with(|counts| *counts.borrow_mut() = vec![0]);
            assert!(fitted_boxes(ctx, p).is_empty());
            assert!(ctx.measure_exhausted);
            assert_eq!(ctx.measure_left, 0);
            assert_eq!((ctx.boxes.total(), ctx.measure_depth), before);
            assert_eq!(ctx.boxes.inside(), matches!(nest, Nest::ActualBox));
            assert_eq!(ctx.eq_count, 7);
            assert!(ctx.fitted.is_empty(), "exhausted {nest:?} must not poison the fitted cache");
            assert!(ctx.fitted_order.is_empty());
            assert!(ctx.chains.flows.is_empty());
            let mut counters = ctx.counters.clone();
            assert_eq!(counters.next_label(&d.numbering, list, 0).unwrap().0, "1.");
            match nest {
                Nest::ActualBox => ctx.boxes.leave(),
                Nest::Probe => ctx.measure_depth = 0,
            }
            let measured = fitted_boxes(ctx, p);
            assert_eq!(measured.len(), 1, "a fresh root must recover from exhausted {nest:?}");
            assert!(measured[0].1 > 0.0);
            assert!(!ctx.measure_exhausted);
            let key = (root, 240.0f32.to_bits());
            assert_eq!(ctx.fitted.get(&key), Some(&Some(measured[0].1)));
            assert_eq!(ctx.fitted_order, [key]);
            assert_eq!(ctx.eq_count, 7);
            let mut counters = ctx.counters.clone();
            assert_eq!(counters.next_label(&d.numbering, list, 0).unwrap().0, "1.");
            eprintln!(
                "NESTED_PROBE_CACHE context={nest:?} exhausted_uncached=true fresh_height={} phases={:?}",
                measured[0].1,
                MEASURE_COUNTS.with(|counts| counts.borrow().clone())
            );
        });
    }
}
#[test]
fn suppressed_owner_identity() {
    let mut d = Document::from_text("");
    let root = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Owner", Default::default()))]);
    let mut object = shape(root, 40.0);
    if let InlineObject::Shape { kind, story, h, float, fill, .. } = &mut object {
        *kind = ShapeKind::Rectangle;
        *story = None;
        *h = 10.0;
        *fill = Some(wordcraft_doc::fill::Fill::solid(wordcraft_doc::props::Rgb(19, 47, 83)));
        float.h_rel = wordcraft_doc::para::Anchor::Column;
        float.v_rel = wordcraft_doc::para::Anchor::Paragraph;
    }
    let owner = Pos::new(StoryRef::Part(root), Path::top(0), 0);
    d.insert_object(&owner, object, &Default::default()).unwrap();
    with_probe_ctx(&d, |ctx| {
        let blocks = &d.parts.get(&root).unwrap().blocks;
        assert!(ctx.boxes.enter(root));
        let items = lay(ctx, root, blocks, 180.0, TextVert::Horz, 0);
        ctx.boxes.leave();
        assert_eq!(suppressed_rects(ctx, root, &items), BTreeSet::from([owner.clone()]));
        let no_hit: Vec<_> = items.iter().filter(|item| !matches!(item, Placed::Object { .. })).cloned().collect();
        assert!(suppressed_rects(ctx, root, &no_hit).is_empty());
        let no_paint: Vec<_> = items.iter().filter(|item| !matches!(item, Placed::Shape { .. })).cloned().collect();
        assert!(suppressed_rects(ctx, root, &no_paint).is_empty());
        for foreign in [Pos::body(0, 0), Pos::new(owner.story, Path::top(1), 0), Pos::new(owner.story, owner.path.clone(), 1)] {
            let mut changed = items.clone();
            for item in &mut changed {
                match item {
                    Placed::Shape { object: Some(pos), .. } => *pos = foreign.clone(),
                    Placed::Object { story, path, off, .. } => {
                        *story = foreign.story;
                        *path = foreign.path.clone();
                        *off = foreign.off;
                    }
                    _ => {}
                }
            }
            assert!(suppressed_rects(ctx, root, &changed).is_empty(), "{foreign:?}");
        }
    });
    d.para_mut(owner.story, &owner.path).unwrap().objects.push(InlineObject::BookmarkEnd { name: "unmatched".into() });
    with_probe_ctx(&d, |ctx| {
        let blocks = &d.parts.get(&root).unwrap().blocks;
        let items = lay(ctx, root, blocks, 180.0, TextVert::Horz, 0);
        assert!(suppressed_rects(ctx, root, &items).is_empty(), "malformed marker ownership is not classified");
    });
}

fn floating_doc(text: &str) -> (Document, u32, InlineObject) {
    let mut d = Document::from_text("");
    d.styles.default_chr.font = Some("Source Sans 3".into());
    let id = d.add_part(PartKind::TextBox, Document::from_text(text).body);
    d.parts.get_mut(&id).unwrap().body = TextBody { auto_fit: true, inset: Some([0; 4]), ..Default::default() };
    let obj = shape(id, 240.0);
    d.insert_object(&Pos::body(0, 0), obj.clone(), &Default::default()).unwrap();
    (d, id, obj)
}

#[test]
fn floating_none_is_reused() {
    for graph in [Graph::Shared, Graph::Varied, Graph::Long] {
        let (d, root) = shared_graph(graph, Fit::Auto);
        let obj = shape(root, 240.0);
        with_probe_ctx(&d, |ctx| {
            MEASURE_COUNTS.with(|v| v.borrow_mut().clear());
            COPIED_BLOCKS.with(|v| v.set(0));
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), (240.0, 2000.0));
            let before = MEASURE_COUNTS.with(|v| v.borrow().clone());
            let copied = COPIED_BLOCKS.with(std::cell::Cell::get);
            assert_eq!(before, [MAX_MEASURE_BOXES]);
            for _ in 0..=wordcraft_doc::BoxBudget::MAX_NESTED {
                assert!(ctx.boxes.enter(root));
                ctx.boxes.leave();
            }
            let actual_total = ctx.boxes.total();
            assert!(actual_total > wordcraft_doc::BoxBudget::MAX_NESTED);
            for saved in [2000.0, 2100.0, 80.0] {
                assert_eq!(fit_size(ctx, &obj, 240.0, saved, 0), (240.0, saved));
            }
            assert_eq!(MEASURE_COUNTS.with(|v| v.borrow().clone()), before, "one exhausted outer measurement per context");
            assert_eq!(COPIED_BLOCKS.with(std::cell::Cell::get), copied);
            assert_eq!(ctx.boxes.total(), actual_total);
            assert!(!ctx.boxes.inside());
            assert_eq!(ctx.measure_depth, 0);
        });
    }
}

#[test]
fn floating_success_is_reused() {
    let (mut d, _, obj) = floating_doc("Original floating content");
    let list = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num: list, level: 0 });
    let unused = d.add_part(PartKind::TextBox, Document::from_text("Unrelated numbered part").body);
    d.para_mut(StoryRef::Part(unused), &Path::top(0)).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num: list, level: 0 });
    with_probe_ctx(&d, |ctx| {
        MEASURE_COUNTS.with(|v| v.borrow_mut().clear());
        let first = fit_size(ctx, &obj, 240.0, 2000.0, 0);
        let before = MEASURE_COUNTS.with(|v| v.borrow().clone());
        assert!(first.1 < 2000.0);
        assert_eq!(before, [1]);
        for _ in 0..=wordcraft_doc::BoxBudget::MAX_NESTED {
            assert!(ctx.boxes.enter(obj.text_box().unwrap()));
            ctx.boxes.leave();
        }
        let actual_total = ctx.boxes.total();
        assert!(actual_total > wordcraft_doc::BoxBudget::MAX_NESTED);
        for _ in 0..4 {
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), first);
        }
        assert_eq!(MEASURE_COUNTS.with(|v| v.borrow().clone()), before, "unrelated numbering does not disable static fitting reuse");
        assert_eq!(ctx.boxes.total(), actual_total);
    });
}

#[test]
fn floating_active_keeps_guards() {
    let (d, root, obj) = floating_doc("Original cycle guard");
    with_probe_ctx(&d, |ctx| {
        MEASURE_COUNTS.with(|v| v.borrow_mut().clear());
        let expected = fit_size(ctx, &obj, 240.0, 2000.0, 0);
        assert!(expected.1 < 2000.0);
        assert!(ctx.boxes.enter(root));
        let total = ctx.boxes.total();
        ctx.eq_count = 7;
        assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), (240.0, 2000.0), "an active owner must still reject its cycle");
        assert_eq!(ctx.boxes.total(), total);
        assert_eq!(ctx.eq_count, 7);
        assert_eq!(ctx.measure_depth, 0);
        assert!(ctx.boxes.inside());
        ctx.boxes.leave();
        let before = MEASURE_COUNTS.with(|v| v.borrow().clone());
        assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), expected);
        assert_eq!(MEASURE_COUNTS.with(|v| v.borrow().clone()), before, "nested rejection must not overwrite the outer cache");
    });
}

#[test]
fn floating_page_stays_fresh() {
    let (mut d, root, mut obj) = floating_doc("");
    d.insert_object(
        &Pos::new(StoryRef::Part(root), Path::top(0), 0),
        InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() },
        &Default::default(),
    )
    .unwrap();
    if let InlineObject::Shape { w, .. } = &mut obj {
        *w = 12.0;
    }
    with_probe_ctx(&d, |ctx| {
        ctx.fields.page = 1;
        let first = fit_size(ctx, &obj, 12.0, 2000.0, 0);
        ctx.fields.page = 1234567890;
        let later = fit_size(ctx, &obj, 12.0, 2000.0, 0);
        let cold = with_probe_ctx_result(&d, |fresh| {
            fresh.fields.page = 1234567890;
            fit_size(fresh, &obj, 12.0, 2000.0, 0)
        });
        let shown: Vec<_> = ctx.cache.paras.values().flat_map(|p| p.shown.iter().map(|v| v.1.as_str())).collect();
        assert!(shown.contains(&"1") && shown.contains(&"1234567890"), "both page contexts were measured: {shown:?}");
        assert!(first.1 > 0.0);
        assert_eq!(later, cold, "the static scalar key must not reuse a different page's extent");
    });
}

fn with_probe_ctx_result<T>(d: &Document, f: impl FnOnce(&mut Ctx) -> T) -> T {
    let mut result = None;
    with_probe_ctx(d, |ctx| result = Some(f(ctx)));
    result.unwrap()
}

#[test]
fn floating_outline_is_caller() {
    use wordcraft_doc::props::Rgb;
    let (d, _, obj) = floating_doc("Original outline");
    with_probe_ctx(&d, |ctx| {
        let mut narrow = obj.clone();
        let mut wide = obj.clone();
        for (o, width) in [(&mut narrow, 1.0), (&mut wide, 7.0)] {
            if let InlineObject::Shape { stroke, stroke_width, .. } = o {
                *stroke = Some(Rgb(20, 60, 100));
                *stroke_width = width;
            }
        }
        let a = fit_size(ctx, &narrow, 240.0, 2000.0, 0);
        let b = fit_size(ctx, &wide, 240.0, 2000.0, 0);
        assert!((b.1 - a.1 - 6.0).abs() < 0.001, "cache content extent, not caller outline");
        assert_eq!(fit_size(ctx, &narrow, 240.0, 2000.0, 0), a);
    });
}

#[test]
fn floating_direct_is_separate() {
    use wordcraft_doc::para::Anchor;
    use wordcraft_doc::props::Rgb;
    let (mut d, root, direct) = floating_doc("Original ownership");
    let child = InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 20.0,
        h: 20.0,
        fill: Some(wordcraft_doc::fill::Fill::solid(Rgb(25, 73, 137))),
        stroke: None,
        stroke_width: 0.0,
        float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, y: 500.0, ..Default::default() },
        story: None,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    d.insert_object(&Pos::new(StoryRef::Part(root), Path::top(0), 0), child, &Default::default()).unwrap();
    let mut turned = direct.clone();
    turned.float_mut().unwrap().rot = 90.0;
    let a = with_probe_ctx_result(&d, |ctx| fit_size(ctx, &direct, 240.0, 2000.0, 0));
    let b = with_probe_ctx_result(&d, |ctx| fit_size(ctx, &turned, 240.0, 2000.0, 0));
    assert!(a.1 < 500.0 && b.1 >= 520.0);
    for order in [[(&direct, a), (&turned, b)], [(&turned, b), (&direct, a)]] {
        with_probe_ctx(&d, |ctx| {
            for (obj, expected) in order.into_iter().cycle().take(4) {
                assert_eq!(fit_size(ctx, obj, 240.0, 2000.0, 0), expected);
            }
        });
    }
}

#[test]
fn floating_widths_stay_distinct() {
    let text = "Original width-sensitive content ".repeat(8);
    let (d, _, obj) = floating_doc(&text);
    let a = with_probe_ctx_result(&d, |ctx| fit_size(ctx, &obj, 24.0, 2000.0, 0));
    let b = with_probe_ctx_result(&d, |ctx| fit_size(ctx, &obj, 240.0, 2000.0, 0));
    assert!(a.1 > b.1);
    with_probe_ctx(&d, |ctx| {
        for (width, expected) in [(24.0, a), (240.0, b), (24.0, a), (240.0, b)] {
            assert_eq!(fit_size(ctx, &obj, width, 2000.0, 0), expected);
        }
    });
}

#[test]
fn floating_grid_keeps_context() {
    let (d, _, obj) = floating_doc("Original grid context");
    with_probe_ctx(&d, |ctx| {
        MEASURE_COUNTS.with(|v| v.borrow_mut().clear());
        let plain = fit_size(ctx, &obj, 240.0, 2000.0, 0);
        assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), plain);
        assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 1), plain);
        assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 1), plain);
        ctx.grid = crate::para::Grid { line: Some(40.0), ..Default::default() };
        let snapped = fit_size(ctx, &obj, 240.0, 2000.0, 0);
        assert!(snapped.1 > plain.1);
        assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), snapped);
        ctx.grid = Default::default();
        assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), plain);
        assert_eq!(MEASURE_COUNTS.with(|v| v.borrow().clone()), [1, 1, 1], "separate depth/grid contexts each measure once");
    });
}

#[test]
fn floating_nested_keeps_budget() {
    let (mut d, root, obj) = floating_doc("Original outer");
    let leaf = d.add_part(PartKind::TextBox, Document::from_text("Original inline leaf").body);
    d.parts.get_mut(&leaf).unwrap().body = TextBody { auto_fit: true, inset: Some([0; 4]), ..Default::default() };
    d.insert_object(&Pos::new(StoryRef::Part(root), Path::top(0), 0), inline_shape(leaf, 180.0), &Default::default()).unwrap();
    let parent = d.add_part(PartKind::TextBox, vec![]);
    let expected = with_probe_ctx_result(&d, |ctx| fit_size(ctx, &obj, 240.0, 2000.0, 0));
    for nest in [Nest::ActualBox, Nest::Probe] {
        with_probe_ctx(&d, |ctx| {
            match nest {
                Nest::ActualBox => assert!(ctx.boxes.enter(parent)),
                Nest::Probe => ctx.measure_depth = 1,
            }
            let before = (ctx.boxes.total(), ctx.measure_depth);
            ctx.eq_count = 7;
            ctx.measure_left = 1;
            MEASURE_COUNTS.with(|v| *v.borrow_mut() = vec![0]);
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), (240.0, 2000.0));
            assert!(ctx.measure_exhausted);
            assert_eq!((ctx.boxes.total(), ctx.measure_depth), before);
            assert_eq!(ctx.eq_count, 7);
            assert!(ctx.fitted.is_empty() && ctx.fitted_order.is_empty());
            match nest {
                Nest::ActualBox => ctx.boxes.leave(),
                Nest::Probe => ctx.measure_depth = 0,
            }
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), expected, "exhausted nested work must not poison a fresh outer result");
            let counts = MEASURE_COUNTS.with(|v| v.borrow().clone());
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), expected);
            assert_eq!(MEASURE_COUNTS.with(|v| v.borrow().clone()), counts);
        });
    }
}

#[test]
fn floating_nested_is_not_memoed() {
    let (mut d, _, obj) = floating_doc("Original nested context");
    let parent = d.add_part(PartKind::TextBox, vec![]);
    with_probe_ctx(&d, |ctx| {
        MEASURE_COUNTS.with(|v| *v.borrow_mut() = vec![0]);
        assert!(ctx.boxes.enter(parent));
        ctx.measure_left = MAX_MEASURE_BOXES;
        ctx.eq_count = 7;
        let nested = fit_size(ctx, &obj, 240.0, 2000.0, 0);
        assert!(!ctx.measure_exhausted);
        assert_eq!(ctx.boxes.total(), 1);
        assert_eq!(ctx.eq_count, 7);
        ctx.boxes.leave();
        assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), nested);
        let before = MEASURE_COUNTS.with(|v| v.borrow().clone());
        assert_eq!(before, [1, 1], "a successful nested result must not seed the outer cache");
        assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), nested);
        assert_eq!(MEASURE_COUNTS.with(|v| v.borrow().clone()), before);
    });
}

pub(crate) fn floating_inline_state_changes(fitted_boxes: impl Fn(&mut Ctx, &Paragraph) -> Vec<(usize, f32)>) {
    let (mut d, root, obj) = floating_doc("");
    let leaf = d.add_part(PartKind::TextBox, Document::from_text("Original inline state").body);
    d.parts.get_mut(&leaf).unwrap().body = TextBody { auto_fit: true, inset: Some([0; 4]), ..Default::default() };
    for owner in [Pos::new(StoryRef::Part(root), Path::top(0), 0), Pos::body(0, 0)] {
        d.insert_object(&owner, inline_shape(leaf, 180.0), &Default::default()).unwrap();
    }
    let carrier = d.para(StoryRef::Body, &Path::top(0)).unwrap();
    for seed_first in [false, true] {
        with_probe_ctx(&d, |ctx| {
            let first = fit_size(ctx, &obj, 240.0, 2000.0, 0);
            let before = ctx.fitted_order.clone();
            ctx.grid = crate::para::Grid { line: Some(40.0), ..Default::default() };
            assert_eq!(fitted_boxes(ctx, carrier).len(), 1);
            assert!(ctx.fitted_order.starts_with(&before));
            let seeded = ctx.fitted_order.clone();
            let wide = if seed_first { Some(fit_size(ctx, &obj, 240.0, 2000.0, 0)) } else { None };
            ctx.grid = Default::default();
            let later = fit_size(ctx, &obj, 240.0, 2000.0, 0);
            let cold = with_probe_ctx_result(&d, |fresh| {
                fresh.grid = crate::para::Grid { line: Some(40.0), ..Default::default() };
                assert_eq!(fitted_boxes(fresh, carrier).len(), 1);
                fresh.grid = Default::default();
                fit_size(fresh, &obj, 240.0, 2000.0, 0)
            });
            assert!(cold.1 > first.1, "the inline cache is a measurement input: {first:?} {cold:?}");
            assert_eq!(later, cold, "outer memoization must retain existing inline-cache context");
            ctx.grid = crate::para::Grid { line: Some(40.0), ..Default::default() };
            let after = fit_size(ctx, &obj, 240.0, 2000.0, 0);
            if let Some(wide) = wide {
                assert_eq!(after, wide);
            }
            ctx.grid = Default::default();
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), cold);
            assert_eq!(ctx.fitted_order, seeded, "probes retain the persistent prefix");
        });
    }
}
#[test]
fn floating_linked_keeps_context() {
    let (mut d, root, obj) = floating_doc("");
    let head = d.add_part(PartKind::TextBox, Document::from_text("Original linked context").body);
    let tail = d.add_part(PartKind::TextBox, Document::from_text("").body);
    d.parts.get_mut(&head).unwrap().body.next = Some(tail);
    d.insert_object(&Pos::new(StoryRef::Part(root), Path::top(0), 0), inline_shape(head, 180.0), &Default::default()).unwrap();
    d.insert_object(&Pos::body(0, 0), shape(tail, 180.0), &Default::default()).unwrap();
    for seed_first in [false, true] {
        with_probe_ctx(&d, |ctx| {
            if seed_first {
                assert!(chain_share(ctx, head, 0).is_some());
            }
            let first = fit_size(ctx, &obj, 240.0, 2000.0, 0);
            let saved = std::mem::take(&mut ctx.chains);
            ctx.grid = crate::para::Grid { line: Some(40.0), ..Default::default() };
            assert!(chain_share(ctx, head, 0).is_some());
            ctx.grid = Default::default();
            MEASURE_COUNTS.with(|v| v.borrow_mut().clear());
            let temporary = fit_size(ctx, &obj, 240.0, 2000.0, 0);
            let cold = with_probe_ctx_result(&d, |fresh| {
                fresh.grid = crate::para::Grid { line: Some(40.0), ..Default::default() };
                assert!(chain_share(fresh, head, 0).is_some());
                fresh.grid = Default::default();
                fit_size(fresh, &obj, 240.0, 2000.0, 0)
            });
            assert_eq!(temporary, cold);
            ctx.chains = saved;
            MEASURE_COUNTS.with(|v| v.borrow_mut().clear());
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), first);
            assert!(!MEASURE_COUNTS.with(|v| v.borrow().is_empty()), "replaced flow maps cannot reuse a floating extent");
        });
    }
}
