use super::*;
use wordcraft_doc::para::{Anchor, Float, ShapeKind, Wrap, WrapText};
use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::{PartKind, Pos, TextBody, para_block};

const TEST_WIDTH: f32 = 200.0;
const WRAP_END: f32 = 60.0;

fn area(left: f32, right: f32) -> para::Exclusion {
    para::Exclusion { top: 0.0, bottom: WRAP_END, left, right, top_bottom: false, contour: None, side: WrapText::BothSides, table: false }
}

fn word_doc() -> Document {
    let mut d = Document::from_text("Independent");
    d.styles.default_chr.font = Some("Source Sans 3".into());
    d.styles.default_chr.size = Some(12.0);
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.line_spacing = Some(LineSpacing::Exactly(12.0));
    d
}

fn with_ctx<T>(d: &Document, f: impl FnOnce(&mut Ctx) -> T) -> T {
    let mut cache = LayoutCache::new();
    let opts = LayoutOptions::default();
    let prior_wraps = PriorBodyWraps::new();
    let mut back_wraps = BackBodyWraps::new();
    let mut ctx = Ctx {
        doc: d,
        sections: d.sections(),
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
        mirror: false,
        table_keys: Vec::new(),
        prior_wraps: &prior_wraps,
        back_wraps: &mut back_wraps,
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
    f(&mut ctx)
}

fn line_signature(p: &ParaLayout) -> Vec<(usize, usize, u32, u32, u32)> {
    p.lines.iter().map(|l| (l.start, l.stop, l.top.to_bits(), l.left.to_bits(), l.right.to_bits())).collect()
}

#[test]
fn policy_is_exact_in_warm_cache() {
    let d = word_doc();
    let p = d.para(StoryRef::Body, &Path::top(0)).unwrap();
    let areas = [area(0.0, 180.0)];
    let cold = with_ctx(&d, |ctx| ctx.para_with_wrap(p, TEST_WIDTH, None, &areas, None, &[0]));
    with_ctx(&d, |ctx| {
        let wait = ctx.para_labelled(p, TEST_WIDTH, None, &areas, None);
        let split = ctx.para_with_wrap(p, TEST_WIDTH, None, &areas, None, &[0]);
        assert!(wait.lines[0].top >= WRAP_END);
        assert!(split.lines[0].top < WRAP_END);
        assert!(!Arc::ptr_eq(&wait, &split));
        assert_eq!(line_signature(&split), line_signature(&cold));
        assert_eq!((ctx.cache.misses, ctx.cache.hits), (2, 0));
        let wait_again = ctx.para_labelled(p, TEST_WIDTH, None, &areas, None);
        let split_again = ctx.para_with_wrap(p, TEST_WIDTH, None, &areas, None, &[0]);
        assert!(Arc::ptr_eq(&wait, &wait_again));
        assert!(Arc::ptr_eq(&split, &split_again));
        assert_eq!((ctx.cache.misses, ctx.cache.hits), (2, 2));
        assert_eq!(ctx.cache.used.len(), 2);
    });
}

#[test]
fn rtl_keeps_policy_with_area() {
    for bidi in [false, true] {
        let mut d = word_doc();
        d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.bidi = Some(bidi);
        let p = d.para(StoryRef::Body, &Path::top(0)).unwrap();
        let areas = [area(0.0, 180.0), area(400.0, 500.0)];
        with_ctx(&d, |ctx| {
            let waiting = ctx.para_with_wrap(p, TEST_WIDTH, None, &areas, None, &[1]);
            let split = ctx.para_with_wrap(p, TEST_WIDTH, None, &areas, None, &[0]);
            let invalid = ctx.para_with_wrap(p, TEST_WIDTH, None, &areas, None, &[usize::MAX]);
            let default = ctx.para_labelled(p, TEST_WIDTH, None, &areas, None);
            assert!(waiting.lines[0].top >= WRAP_END, "an unrelated Split area cannot change Wait, rtl={bidi}");
            assert!(split.lines[0].top < WRAP_END, "the active area keeps Split when mirrored, rtl={bidi}");
            assert_eq!(line_signature(&invalid), line_signature(&default));
        });
    }
}

#[test]
fn continuation_reindexes_policy() {
    let mut rel = BodyExclusions::default();
    let mut removed = area(300.0, 350.0);
    removed.top = 80.0;
    removed.bottom = 120.0;
    rel.append_split([removed]);
    rel.append_wait([area(0.0, 40.0)]);
    rel.append_split([area(60.0, 90.0)]);
    assert_eq!(rel.split, [0, 2]);
    let ordinary = (Rect::new(120.0, 0.0, 20.0, 1000.0), false, None, WrapText::BothSides, false);
    let filtered = (Rect::new(900.0, 0.0, 20.0, 1000.0), false, None, WrapText::BothSides, false);
    let here = BodyExclusions::wait(rel_exclusions(&[ordinary, filtered], 0.0, -30.0));
    let next = continued_exclusions(&rel, &here, 30.0, TEST_WIDTH);
    assert_eq!(next.areas.len(), 3);
    assert_eq!(next.split, [1], "retained Split is reindexed; even a tall new area starts Wait");
    assert_eq!((next.areas[0].bottom, next.areas[1].bottom, next.areas[2].top), (30.0, 30.0, 30.0));
    let waiting = BodyExclusions::wait(next.areas.clone());
    assert_ne!(next, waiting, "policy-only changes must trigger the relay guard");
}

#[test]
fn fallback_keeps_pair() {
    let d = word_doc();
    let p = d.para(StoryRef::Body, &Path::top(0)).unwrap();
    let areas = vec![area(0.0, 180.0)];
    with_ctx(&d, |ctx| {
        let wait = ctx.para_labelled(p, TEST_WIDTH, None, &areas, None);
        let split = ctx.para_with_wrap(p, TEST_WIDTH, None, &areas, None, &[0]);
        let mut pl = wait.clone();
        let mut policy = BodyExclusions::wait(areas.clone());
        let mut changed = BodyExclusions::default();
        changed.append_split(areas);
        assert!(accept_wrap_relay(&mut pl, &mut policy, split.clone(), changed.clone(), 0, wait.lines[0].top).is_none());
        assert!(Arc::ptr_eq(&pl, &wait));
        assert!(policy.split.is_empty());
        assert!(accept_wrap_relay(&mut pl, &mut policy, split.clone(), changed.clone(), usize::MAX, 0.0).is_none());
        assert!(Arc::ptr_eq(&pl, &wait));
        assert!(policy.split.is_empty());
        assert_eq!(accept_wrap_relay(&mut pl, &mut policy, split.clone(), changed, 0, split.lines[0].top), Some(0));
        assert!(Arc::ptr_eq(&pl, &split));
        assert_eq!(policy.split, [0]);
    });
}

fn shape(story: Option<u32>, wrap: Wrap, w: f32, h: f32) -> InlineObject {
    InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w,
        h,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Float { wrap, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, ..Default::default() },
        story,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    }
}

#[test]
fn default_nested_stays_wait() {
    let mut d = word_doc();
    let id = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Independent", Default::default()))]);
    d.parts.get_mut(&id).unwrap().body = TextBody { auto_fit: true, inset: Some([0; 4]), ..Default::default() };
    let mut obstacle = shape(None, Wrap::Square, 180.0, WRAP_END);
    if let InlineObject::Shape { kind, .. } = &mut obstacle {
        *kind = ShapeKind::Rectangle;
    }
    d.insert_object(&Pos::new(StoryRef::Part(id), Path::top(0), 0), obstacle, &Default::default()).unwrap();
    d.insert_object(&Pos::body(0, 0), shape(Some(id), Wrap::Inline, TEST_WIDTH, 10.0), &Default::default()).unwrap();
    let p = d.para(StoryRef::Body, &Path::top(0)).unwrap();
    let expected = with_ctx(&d, |ctx| ctx.fitted_boxes(p));
    assert_eq!(expected.len(), 1);
    assert!(expected[0].1 > WRAP_END, "nested default layout waits below its own obstruction");
    with_ctx(&d, |ctx| {
        let split = ctx.para_with_wrap(p, TEST_WIDTH, None, &[area(0.0, 180.0)], None, &[0]);
        assert!(!split.lines.is_empty());
        assert_eq!(ctx.fitted[&(id, TEST_WIDTH.to_bits())], Some(expected[0].1));
        let ordinary = ctx.para_labelled(p, TEST_WIDTH, None, &[area(0.0, 180.0)], None);
        let cold = with_ctx(&d, |fresh| fresh.para_labelled(p, TEST_WIDTH, None, &[area(0.0, 180.0)], None));
        assert_eq!(line_signature(&ordinary), line_signature(&cold));
    });
}

#[derive(Clone, Copy, Debug)]
enum ReplayCase {
    Plain,
    Totals,
    Notes,
    Columns,
    Footer,
    Combined,
}

fn field(instr: &str) -> InlineObject {
    InlineObject::Field { instr: instr.into(), result: "9".into(), locked: false, dirty: false, code: false, form: String::new() }
}

fn replay_doc(case: ReplayCase) -> (Document, Vec<u32>) {
    let text = std::iter::once("Earlier".to_string())
        .chain(std::iter::once("Boundary anchor".to_string()))
        .chain((0..80).map(|i| format!("Body{i:03} original row")))
        .collect::<Vec<_>>()
        .join("\n");
    let mut d = Document::from_text(&text);
    d.styles.default_chr.font = Some("Source Sans 3".into());
    d.styles.default_chr.size = Some(12.0);
    d.last_section.page_w = 595.3;
    d.last_section.page_h = 841.9;
    d.last_section.margin_top = 94.5;
    d.last_section.margin_bottom = 72.0;
    let list = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    for i in 0..d.body.len() {
        let p = d.para_mut(StoryRef::Body, &Path::top(i)).unwrap();
        p.props.space_before = Some(0.0);
        p.props.space_after = Some(0.0);
        p.props.line_spacing = Some(LineSpacing::Exactly(if i == 0 { 80.0 } else { 18.0 }));
        if i >= 2 {
            p.props.numbering = Some(wordcraft_doc::props::NumRef { num: list, level: 0 });
        }
    }
    let id = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Boundary story", Default::default()))]);
    let mut obj = shape(Some(id), Wrap::Square, 400.0, 169.15);
    obj.float_mut().unwrap().y = -300.0;
    d.insert_object(&Pos::body(1, 0), obj, &Default::default()).unwrap();
    if matches!(case, ReplayCase::Totals | ReplayCase::Combined) {
        let p = d.para_mut(StoryRef::Body, &Path::top(4)).unwrap();
        for instr in ["NUMPAGES", "SECTIONPAGES"] {
            p.insert_object(p.len(), field(instr), &Default::default()).unwrap();
        }
    }
    let mut notes = Vec::new();
    if matches!(case, ReplayCase::Notes | ReplayCase::Combined) {
        d.settings.footnote_pr =
            wordcraft_doc::section::NoteProps { num_restart: Some(wordcraft_doc::section::NoteRestart::EachPage), ..Default::default() };
        d.para_mut(StoryRef::Body, &Path::top(70)).unwrap().props.page_break_before = Some(true);
        for (i, text) in [(0, "First note payload."), (70, "Later note payload.")] {
            let id = d.add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text(text, Default::default()))]);
            let off = d.para(StoryRef::Body, &Path::top(i)).unwrap().len();
            d.insert_object(
                &Pos::body(i, off),
                InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id, custom: String::new() },
                &Default::default(),
            )
            .unwrap();
            notes.push(id);
        }
    }
    if matches!(case, ReplayCase::Columns | ReplayCase::Combined) {
        d.last_section.columns = wordcraft_doc::section::Columns { count: 2, space: 36.0, separator: false, widths: Vec::new() };
    }
    if matches!(case, ReplayCase::Footer | ReplayCase::Combined) {
        let mut table = wordcraft_doc::Table::new(1, 1, 90.0);
        table.props.cell_margins = Some([0.0; 4]);
        table.props.float =
            Some(wordcraft_doc::props::TableFloat { y: 140.0, x: 72.0, h_rel: Anchor::Page, v_rel: Anchor::Page, ..Default::default() });
        table.rows[0].props.height = Some(60.0);
        table.rows[0].props.height_rule = wordcraft_doc::props::HeightRule::Exact;
        let id = d.add_part(PartKind::Footer, vec![Arc::new(Block::Table(table))]);
        d.last_section.footers.default = Some(id);
    }
    (d, notes)
}

fn counted_layout(d: &Document, cache: &mut LayoutCache) -> (DocLayout, ReplayStats) {
    REPLAY_STATS.with(|s| *s.borrow_mut() = ReplayStats::default());
    let laid = layout(d, cache, &LayoutOptions::default());
    let stats = REPLAY_STATS.with(|s| s.borrow().clone());
    assert_eq!(stats.passes, 1 + stats.back_retries + stats.total_retries + stats.early_retries + stats.note_retries);
    assert!(stats.back_retries <= MAX_BACK_WRAP_PASSES);
    assert!(stats.total_retries + stats.totals_with_back <= MAX_TOTAL_PASSES);
    assert!(stats.note_retries < MAX_NOTE_PASSES);
    assert!(stats.early_retries + stats.early_with_back <= MAX_REWRAP_PASSES);
    assert!(stats.passes <= MAX_BACK_WRAP_PASSES + MAX_TOTAL_PASSES + MAX_NOTE_PASSES + MAX_REWRAP_PASSES);
    assert_eq!(stats.wrap_attempts, stats.wrap_accepted + stats.wrap_rejected);
    (laid, stats)
}

fn body_signature(l: &DocLayout) -> Vec<(usize, Path, usize, usize, u32, u32)> {
    l.pages
        .iter()
        .enumerate()
        .flat_map(|(page, p)| {
            p.items.iter().filter_map(move |it| match it {
                Placed::Lines { story: StoryRef::Body, path, l0, l1, x, y, .. } => Some((page, path.clone(), *l0, *l1, x.to_bits(), y.to_bits())),
                _ => None,
            })
        })
        .collect()
}

fn assert_visible(d: &Document, l: &DocLayout) {
    let mut labels = Vec::new();
    let mut bytes: HashMap<usize, Vec<u8>> = d
        .body
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b.as_ref() {
            Block::Para(p) => Some((i, vec![0; p.text.len()])),
            _ => None,
        })
        .collect();
    for it in l.pages.iter().flat_map(|p| &p.items) {
        let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, .. } = it else { continue };
        let index = path.0[0] as usize;
        for line in &para.lines[*l0..*l1] {
            for count in &mut bytes.get_mut(&index).unwrap()[line.start..line.stop] {
                *count += 1;
            }
        }
        if *l0 == 0
            && let Some(label) = &para.label
        {
            labels.push(label.text.trim().to_string());
        }
    }
    assert!(bytes.values().all(|row| row.iter().all(|n| *n == 1)), "each body byte remains on one physical line");
    assert_eq!(labels, (1..=80).map(|n| format!("{n}.")).collect::<Vec<_>>(), "replays do not lose or duplicate numbering");
}

#[test]
fn replay_counts_and_state_hold() {
    for case in [ReplayCase::Plain, ReplayCase::Totals, ReplayCase::Notes, ReplayCase::Columns, ReplayCase::Footer, ReplayCase::Combined] {
        let (d, notes) = replay_doc(case);
        let mut cache = LayoutCache::new();
        let (cold, first) = counted_layout(&d, &mut cache);
        let (warm, second) = counted_layout(&d, &mut cache);
        eprintln!("REPLAY_WORK case={case:?} cold={first:?} warm={second:?} pages={}", cold.pages.len());
        assert!(first.back_retries > 0 && second.back_retries > 0);
        assert_eq!(cold.pages.len(), warm.pages.len());
        assert_eq!(body_signature(&cold), body_signature(&warm));
        assert_visible(&d, &cold);
        assert_visible(&d, &warm);
        if matches!(case, ReplayCase::Totals | ReplayCase::Combined) {
            assert!(first.total_retries + first.totals_with_back > 0);
            let shown: Vec<_> = cold
                .pages
                .iter()
                .flat_map(|p| &p.items)
                .filter_map(|it| match it {
                    Placed::Lines { story: StoryRef::Body, path, para, .. } if *path == Path::top(4) => {
                        Some(para.shown.iter().map(|s| s.1.clone()).collect::<Vec<_>>())
                    }
                    _ => None,
                })
                .flatten()
                .collect();
            assert_eq!(shown, vec![cold.pages.len().to_string(); 2]);
        }
        if !notes.is_empty() {
            assert!(first.note_retries > 0);
            let pages: Vec<_> = notes.iter().map(|id| cold.caret(&d.start_of(StoryRef::Part(*id))).unwrap().page).collect();
            assert!(pages[1] > pages[0]);
            let mut marks = HashMap::new();
            for it in cold.pages.iter().flat_map(|p| &p.items) {
                if let Placed::Lines { story: StoryRef::Body, para, .. } = it {
                    for (ci, id) in &para.notes {
                        marks.insert(*id, para.shown.iter().find(|s| s.0 == *ci).map(|s| s.1.clone()));
                    }
                }
            }
            assert_eq!(marks[&notes[0]], Some("1".into()));
            assert_eq!(marks[&notes[1]], Some("1".into()));
        }
        if matches!(case, ReplayCase::Columns | ReplayCase::Combined) {
            let columns: std::collections::HashSet<_> = cold.pages[0]
                .items
                .iter()
                .filter_map(|it| match it {
                    Placed::Lines { story: StoryRef::Body, x, .. } => Some(x.to_bits()),
                    _ => None,
                })
                .collect();
            assert!(columns.len() >= 2, "the fixture traverses both columns");
        }
        if matches!(case, ReplayCase::Footer | ReplayCase::Combined) {
            assert!(cold.pages.iter().all(|p| p.footer.iter().any(|it| matches!(it, Placed::Cell { .. }))));
        }
    }
}

// A later anchor must wrap text already continued onto its page during backward replay.
fn relay_later_doc(anchor: Anchor, wrap: Wrap) -> (Document, u32) {
    let mut d = Document::from_text(&format!("{}\nLater anchor", "original words ".repeat(80)));
    d.settings.compat_mode = 15;
    d.styles.default_chr.font = Some("Source Sans 3".into());
    d.styles.default_chr.size = Some(12.0);
    d.last_section.page_w = 300.0;
    d.last_section.page_h = 300.0;
    d.last_section.margin_left = 30.0;
    d.last_section.margin_right = 30.0;
    d.last_section.margin_top = 30.0;
    d.last_section.margin_bottom = 30.0;
    for i in 0..2 {
        let p = d.para_mut(StoryRef::Body, &Path::top(i)).unwrap();
        p.props.space_before = Some(0.0);
        p.props.space_after = Some(0.0);
        p.props.line_spacing = Some(LineSpacing::Exactly(12.0));
    }
    let id = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Frame", Default::default()))]);
    d.parts.get_mut(&id).unwrap().body.inset = Some([0; 4]);
    let mut o = shape(Some(id), wrap, 100.0, 60.0);
    let float = o.float_mut().unwrap();
    float.v_rel = anchor;
    float.y = -500.0;
    d.insert_object(&Pos::body(1, 0), o, &Default::default()).unwrap();
    (d, id)
}

fn relay_ranges(d: &Document, laid: &DocLayout) {
    for index in 0..2 {
        let p = d.para(StoryRef::Body, &Path::top(index)).unwrap();
        let mut coverage = vec![0; p.len()];
        for item in laid.pages.iter().flat_map(|p| &p.items) {
            let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, .. } = item else { continue };
            if *path != Path::top(index) {
                continue;
            }
            for line in &para.lines[*l0..*l1] {
                for n in &mut coverage[line.start..line.stop] {
                    *n += 1;
                }
            }
        }
        assert!(coverage.iter().all(|n| *n == 1), "body {index}: ranges conserved");
    }
}

fn relay_page_rows(laid: &DocLayout, page: usize) -> Vec<String> {
    let mut out = Vec::new();
    for it in &laid.pages[page].items {
        let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, x, y, .. } = it else { continue };
        for line in &para.lines[*l0..*l1] {
            out.push(format!(
                "{path:?}:{}:{}:{:?}:{:?}:{:?}",
                line.start,
                line.stop,
                x.to_bits(),
                (y + line.top - para.lines[*l0].top).to_bits(),
                line.xs.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
            ));
        }
    }
    out
}

fn assert_later_wrap(anchor: Anchor) {
    let (d, id) = relay_later_doc(anchor, Wrap::Square);
    let mut cache = LayoutCache::new();
    let (laid, stats) = counted_layout(&d, &mut cache);
    let object = laid.find_object(0, |o| o.text_box == Some(id)).unwrap();
    let r = object.rect;
    eprintln!("RELAY_LATER {anchor:?} page={} rect={r:?} stats={stats:?}", object.page);
    assert!(object.page > 0, "the anchor follows a cross-page paragraph");
    assert_eq!((r.y, r.w, r.h), (30.0, 100.0, 60.0));
    assert!(stats.back_retries > 0, "actual backward replay is exercised");
    let mut touched = 0;
    for item in &laid.pages[object.page].items {
        let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, x, y, .. } = item else { continue };
        if *path != Path::top(0) {
            continue;
        }
        assert!(*l0 > 0, "this is a continuation fragment");
        for line in &para.lines[*l0..*l1] {
            let top = y + line.top - para.lines[*l0].top;
            if top >= r.bottom() || top + line.height <= r.y {
                continue;
            }
            touched += 1;
            for (offset, c) in para.clusters[line.c0..line.c1].iter().enumerate() {
                if c.kind != para::ClKind::Text {
                    continue;
                }
                let left = x + line.xs[offset];
                assert!(
                    left + c.adv <= r.x + 0.01 || left >= r.right() - 0.01,
                    "{anchor:?}: continuation text overlaps later frame: byte={} x={left} adv={} frame={r:?}",
                    c.start,
                    c.adv
                );
            }
        }
    }
    assert!(touched > 0, "continuation rows beside the frame are exercised");
    let mut unwrapped = d.clone();
    unwrapped.para_mut(StoryRef::Body, &Path::top(1)).unwrap().objects[0].float_mut().unwrap().wrap = Wrap::InFrontOfText;
    let (plain, _) = counted_layout(&unwrapped, &mut LayoutCache::new());
    assert_eq!(relay_page_rows(&laid, 0), relay_page_rows(&plain, 0), "page-one prefix remains exact");
    relay_ranges(&d, &laid);
    let (warm, _) = counted_layout(&d, &mut cache);
    assert_eq!(body_signature(&laid), body_signature(&warm));
    assert_eq!(warm.find_object(0, |o| o.text_box == Some(id)).unwrap().rect, r);
    assert_eq!(relay_page_rows(&laid, object.page), relay_page_rows(&warm, object.page));
    relay_ranges(&d, &warm);
}

#[test]
fn continued_paragraph_wraps() {
    assert_later_wrap(Anchor::Paragraph);
}

#[test]
fn continued_line_wraps() {
    assert_later_wrap(Anchor::Line);
}

#[test]
fn continued_unsupported_stays() {
    for (anchor, wrap) in [(Anchor::Page, Wrap::Square), (Anchor::Paragraph, Wrap::BehindText), (Anchor::Line, Wrap::InFrontOfText)] {
        let (d, id) = relay_later_doc(anchor, wrap);
        let mut cache = LayoutCache::new();
        let (cold, stats) = counted_layout(&d, &mut cache);
        assert!(cold.find_object(0, |o| o.text_box == Some(id)).unwrap().page > 0);
        assert_eq!(stats.back_retries, 0, "{anchor:?} {wrap:?}: no backward policy");
        relay_ranges(&d, &cold);
        let (warm, _) = counted_layout(&d, &mut cache);
        assert_eq!(body_signature(&cold), body_signature(&warm));
        relay_ranges(&d, &warm);
    }
}

#[test]
fn continued_tall_keeps_split() {
    let mut here = BodyExclusions::default();
    let mut tall = area(40.0, 80.0);
    tall.top = 10.0;
    tall.bottom = 1000.0;
    here.append_split([area(900.0, 1000.0), tall]);
    here.append_wait([area(90.0, 120.0)]);
    let next = continued_exclusions(&BodyExclusions::default(), &here, 30.0, TEST_WIDTH);
    assert_eq!(next.areas.len(), 2);
    assert_eq!(next.split, [0], "filtered Split indices do not change Wait");
    assert!(next.areas.iter().all(|a| a.top == 30.0));
}

#[test]
fn relay_boundary_cache_is_exact() {
    let (d, _) = relay_later_doc(Anchor::Paragraph, Wrap::Square);
    let p = d.para(StoryRef::Body, &Path::top(0)).unwrap();
    with_ctx(&d, |ctx| {
        let plain = ctx.para_labelled(p, 240.0, None, &[], None);
        let index = 20;
        let (resume, top) = (plain.lines[index].start, plain.lines[index].top);
        let areas = [para::Exclusion {
            top,
            bottom: top + 60.0,
            left: 0.0,
            right: 100.0,
            top_bottom: false,
            contour: None,
            side: WrapText::BothSides,
            table: false,
        }];
        let ordinary = ctx.para_with_wrap(p, 240.0, None, &areas, None, &[]);
        let relaid = ctx.para_relay(p, 240.0, None, &areas, None, &[], Some(top));
        eprintln!(
            "RELAY_PREFIX plain={:?} ordinary={:?} relay={:?}",
            line_signature(&plain)[index - 1..index + 1].to_vec(),
            line_signature(&ordinary)[index - 1..index + 1].to_vec(),
            line_signature(&relaid)[index - 1..index + 1].to_vec()
        );
        assert_ne!(line_signature(&plain)[..index], line_signature(&ordinary)[..index], "estimated row height reaches across the boundary");
        assert_eq!(line_signature(&plain)[..index], line_signature(&relaid)[..index]);
        assert_eq!((relaid.lines[index].start, relaid.lines[index].top), (resume, top));
        assert!(!Arc::ptr_eq(&ordinary, &relaid));
        let other = ctx.para_relay(p, 240.0, None, &areas, None, &[], Some(top + 12.0));
        assert!(!Arc::ptr_eq(&other, &relaid), "boundary bits belong to the cache identity");
        let warm = ctx.para_relay(p, 240.0, None, &areas, None, &[], Some(top));
        assert!(Arc::ptr_eq(&warm, &relaid));
        let normal_warm = ctx.para_with_wrap(p, 240.0, None, &areas, None, &[]);
        assert!(Arc::ptr_eq(&normal_warm, &ordinary));
        let mut held = plain.clone();
        let mut rel = BodyExclusions::default();
        assert!(accept_wrap_relay(&mut held, &mut rel, ordinary, BodyExclusions::wait(areas.to_vec()), resume, top).is_none());
        assert!(Arc::ptr_eq(&held, &plain));
        assert_eq!(accept_wrap_relay(&mut held, &mut rel, relaid, BodyExclusions::wait(areas.to_vec()), resume, top), Some(index));
    });
}
