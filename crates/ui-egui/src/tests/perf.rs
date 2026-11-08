//! Scrolling performance with a very large table (#811).

use std::time::Instant;

use wordcraft_doc::table::{Cell, Table};
use wordcraft_doc::{Block, Document, Path, StoryRef};

use crate::{Services, WordApp};

/// A document with one `rows × cols` table, every cell holding a few words.
pub(crate) fn big_table_doc(rows: usize, cols: usize) -> Document {
    let mut d = Document::from_text("Before the table\nAfter the table");
    let mut t = Table::new(rows, cols, 468.0);
    for (r, row) in t.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            let w = cell.props.width;
            *cell = Cell::with_text(&format!("R{r} C{c} value {}", r * cols + c));
            cell.props.width = w;
        }
    }
    let _ = d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(t));
    d
}

fn harness(doc: Document) -> egui_kittest::Harness<'static, WordApp> {
    let a = WordApp::new(wordcraft_engine::Session::new(doc), Services::default());
    egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    )
}

/// Queues a pointer move to (720, 500) and a vertical point-unit wheel event.
fn wheel(h: &mut egui_kittest::Harness<'static, WordApp>, dy: f32) {
    h.input_mut().events.push(egui::Event::PointerMoved(egui::pos2(720.0, 500.0)));
    h.input_mut().events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
        source: egui::MouseWheelSource::Unknown,
    });
}

/// `cargo test -p wordcraft-ui-egui --release big_table_scroll_bench -- --ignored --nocapture`
/// (`WORDCRAFT_BENCH_DOCX=path` times that file instead of the synthetic table).
#[test]
#[ignore]
fn big_table_scroll_bench() {
    let doc = match std::env::var("WORDCRAFT_BENCH_DOCX") {
        Ok(p) => wordcraft_engine::io::open_path(std::path::Path::new(&p)).unwrap(),
        Err(_) => big_table_doc(2000, 8),
    };
    let mut s = wordcraft_engine::Session::new(doc.clone());
    let t = Instant::now();
    let l = s.layout();
    eprintln!("layout: {:.1} ms, {} pages", t.elapsed().as_secs_f64() * 1e3, l.pages.len());
    let t = Instant::now();
    let _ = s.export_layout();
    eprintln!("export_layout: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    if let Some(p) = l.pages.get(1) {
        let t = Instant::now();
        let _ = wordcraft_render::render_page(&s.doc, p, 1.5, &crate::canvas::screen_render_options());
        eprintln!("render_page: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    }
    let mut h = harness(doc);
    let t = Instant::now();
    h.run_steps(3);
    eprintln!("first frames: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    for _ in 0..10 {
        h.step();
    }
    let mut idle = Vec::new();
    for _ in 0..10 {
        let t = Instant::now();
        h.step();
        idle.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let mut steps = Vec::new();
    for _ in 0..40 {
        wheel(&mut h, -120.0);
        let t = Instant::now();
        h.step();
        steps.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let stat = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        (v.get(v.len() / 2).copied().unwrap_or(0.0), v.last().copied().unwrap_or(0.0))
    };
    eprintln!("idle frame: median/max {:.2?}", stat(&mut idle));
    eprintln!("scroll frame: median/max {:.2?}", stat(&mut steps));
    eprintln!("scroll offset after: {:?}", h.state().canvas.scroll_offset);
}

/// Benchmark simulating an extended interactive session on Baguio-Benguet docx
#[test]
#[ignore]
fn long_open_baguio_bench() {
    let doc = match std::env::var("WORDCRAFT_BENCH_DOCX") {
        Ok(p) => wordcraft_engine::io::open_path(std::path::Path::new(&p)).unwrap(),
        Err(_) => big_table_doc(200, 8),
    };
    let mut h = harness(doc);
    let l = h.state_mut().session.layout();
    eprintln!("Baguio pages count: {}", l.pages.len());
    for (i, p) in l.pages.iter().enumerate().take(3) {
        let lines_count = p.items.iter().filter(|it| matches!(it, wordcraft_layout::Placed::Lines { .. })).count();
        let other_count = p.items.len() - lines_count;
        eprintln!("Page {}: {} lines, {} other items, w={}, h={}", i, lines_count, other_count, p.w, p.h);
    }
    h.run_steps(3);

    // Warm-up (allows progressive idle prewarming for documents under 24 pages)
    for _ in 0..15 {
        h.step();
    }

    let mut phase_times = Vec::new();
    let mut max_times = Vec::new();

    // Phase 1: 100 idle frames (simulating sitting open for 100 caret ticks)
    let mut idle_times = Vec::new();
    for _ in 0..100 {
        let t = Instant::now();
        h.step();
        idle_times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    idle_times.sort_by(|a, b| a.total_cmp(b));
    phase_times.push(idle_times[idle_times.len() / 2]);
    max_times.push(*idle_times.last().unwrap());

    // Phase 2: 100 down motions ('j')
    let mut j_times = Vec::new();
    for _ in 0..100 {
        h.input_mut().events.push(egui::Event::Key {
            key: egui::Key::J,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let t = Instant::now();
        h.step();
        j_times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    j_times.sort_by(|a, b| a.total_cmp(b));
    phase_times.push(j_times[j_times.len() / 2]);
    max_times.push(*j_times.last().unwrap());

    // Phase 3: 100 word motions ('w')
    let mut w_times = Vec::new();
    for _ in 0..100 {
        h.input_mut().events.push(egui::Event::Key {
            key: egui::Key::W,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let t = Instant::now();
        h.step();
        w_times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    w_times.sort_by(|a, b| a.total_cmp(b));
    phase_times.push(w_times[w_times.len() / 2]);
    max_times.push(*w_times.last().unwrap());

    // Phase 4: 50 wheel scroll steps down and 50 up
    let mut scroll_times = Vec::new();
    for i in 0..100 {
        wheel(&mut h, if i < 50 { -200.0 } else { 200.0 });
        let t = Instant::now();
        h.step();
        scroll_times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    scroll_times.sort_by(|a, b| a.total_cmp(b));
    phase_times.push(scroll_times[scroll_times.len() / 2]);
    max_times.push(*scroll_times.last().unwrap());

    // Phase 5: 100 more idle frames after heavy navigation
    let mut late_idle_times = Vec::new();
    for _ in 0..100 {
        let t = Instant::now();
        h.step();
        late_idle_times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    late_idle_times.sort_by(|a, b| a.total_cmp(b));
    phase_times.push(late_idle_times[late_idle_times.len() / 2]);
    max_times.push(*late_idle_times.last().unwrap());

    eprintln!(
        "Phases (median ms): idle1={:.2}, j={:.2}, w={:.2}, scroll={:.2}, idle2={:.2}",
        phase_times[0], phase_times[1], phase_times[2], phase_times[3], phase_times[4]
    );
    eprintln!(
        "Phases (max ms): idle1={:.2}, j={:.2}, w={:.2}, scroll={:.2}, idle2={:.2}",
        max_times[0], max_times[1], max_times[2], max_times[3], max_times[4]
    );
    eprintln!("Textures count: {}", h.state().canvas.texture_count());
}

/// #811: scrolling through one very large table was slow because every frame fingerprinted every
/// item of every page (thousands of cell borders a page) for the page texture cache, and every
/// page ever shown kept its texture. Now a frame lays nothing out again, fingerprints a page once
/// per layout and only when it is on screen, and keeps textures only near the view.
#[test]
fn scrolling_a_big_table_fingerprints_only_pages_coming_into_view() {
    let mut h = harness(big_table_doc(600, 8));
    h.run_steps(4);
    let layout = h.state_mut().session.layout();
    let pages = layout.pages.len();
    assert!(pages > 30, "the table runs over many pages ({pages})");
    let first = h.state().canvas.page_keys.hashed;
    assert!(first > 0 && first < 6, "only the pages on screen are fingerprinted, not all {pages}: {first}");
    // Idle frames do no per-page work.
    h.run_steps(3);
    assert_eq!(h.state().canvas.page_keys.hashed, first, "idle frames fingerprint nothing");
    // Scroll a long way down: each newly shown page is fingerprinted once.
    let mut max_textures = 0;
    for _ in 0..60 {
        wheel(&mut h, -400.0);
        h.step();
        max_textures = max_textures.max(h.state().canvas.texture_count());
    }
    let offset = h.state().canvas.scroll_offset.y;
    assert!(offset > 20_000.0, "scrolled through many pages: {offset}");
    let after = std::sync::Arc::clone(&h.state_mut().session.layout());
    assert!(std::sync::Arc::ptr_eq(&layout, &after), "scrolling never lays the document out again");
    let hashed = h.state().canvas.page_keys.hashed;
    // 60 steps of 400 pt cross about 24'000 / 1'056 ≈ 23 pages (at 100%, 1.33 px a point).
    assert!(hashed < pages && hashed <= first + 30, "pages fingerprinted: {hashed} of {pages}");
    // Textures stay bounded to the pages near the view.
    assert!(max_textures <= 12, "textures kept while scrolling: {max_textures}");
    assert!(h.state().canvas.texture_count() >= 1, "the pages in view stay rendered");
}
