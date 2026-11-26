//! Scrolling performance with a very large table (#811).

use std::sync::Arc;

use craft::host::{Host, ManualJobs};
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

/// A harness over `doc` whose host runs a job only when the test runs its queue ([`ManualJobs`]): page pictures render
/// on the test thread at the points the test picks, so what is resident never depends on the runner's load.
fn harness(doc: Document) -> (egui_kittest::Harness<'static, WordApp>, Arc<ManualJobs>) {
    let jobs = Arc::new(ManualJobs::new());
    let host = Host::builder(crate::APP).i18n(crate::i18n::catalogs()).lang(craft::i18n::Lang::EN).jobs(jobs.clone()).build();
    let a = WordApp::new(wordcraft_engine::Session::new(doc), Services::default()).with_host(Arc::new(host));
    (crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| a), jobs)
}

/// Queues a pointer move to (720, 500) and a vertical point-unit wheel event that scrolls the view `dy` points.
/// The framework applies the touchpad speed to point scrolling (`WordApp::root_env`), as production's desktop host did
/// before egui read the input (4× at Normal off macOS and the web), so the event carries `dy` over that factor.
fn wheel(h: &mut egui_kittest::Harness<'static, WordApp>, dy: f32) {
    let factor = h.state().ui.touchpad_scroll_speed.touchpad_factor();
    h.input_mut().events.push(egui::Event::PointerMoved(egui::pos2(720.0, 500.0)));
    h.input_mut().events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, dy / factor),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
        source: egui::MouseWheelSource::Unknown,
    });
}

/// #811: scrolling through one very large table was slow because every frame fingerprinted every
/// item of every page (thousands of cell borders a page) for the page texture cache, and every
/// page ever shown kept its texture. Now a frame lays nothing out again, fingerprints a page once
/// per layout and only when it is on screen, and keeps textures only near the view. The pictures render on the
/// host's jobs, which the test runs after each scroll step, so the renders for the view have finished when the next
/// frame uploads them.
#[test]
fn scrolling_a_big_table_fingerprints_only_pages_coming_into_view() {
    let (mut h, jobs) = harness(big_table_doc(400, 8));
    h.run_steps(4);
    let layout = h.state_mut().session.layout();
    let pages = layout.pages.len();
    // The scroll below crosses about 17 pages; the table runs well past that on every platform's fonts (29 pages on
    // macOS, more elsewhere), so the view never reaches its end.
    assert!(pages > 25, "the table runs over many pages ({pages})");
    let first = h.state().canvas.page_keys.hashed;
    assert!(first > 0 && first < 6, "only the pages on screen are fingerprinted, not all {pages}: {first}");
    // Idle frames do no per-page work.
    h.run_steps(3);
    assert_eq!(h.state().canvas.page_keys.hashed, first, "idle frames fingerprint nothing");
    assert!(jobs.waiting() > 0, "the pages in view render on the host's jobs");
    // Scroll a long way down: each newly shown page is fingerprinted once. The renders a step asks for finish before
    // the next step, whose frame uploads them.
    let mut max_textures = 0;
    for _ in 0..9 {
        wheel(&mut h, -2000.0);
        h.step();
        max_textures = max_textures.max(h.state().canvas.resident_pictures);
        jobs.run_all();
    }
    h.step();
    let offset = h.state().canvas.view.offset.1;
    assert!(offset > 16_000.0, "scrolled through many pages: {offset}");
    let after = std::sync::Arc::clone(&h.state_mut().session.layout());
    assert!(std::sync::Arc::ptr_eq(&layout, &after), "scrolling never lays the document out again");
    let hashed = h.state().canvas.page_keys.hashed;
    // 9 steps of 2'000 pt cross about 18'000 / 1'056 ≈ 17 pages (at 100%, 1.33 px a point).
    assert!(hashed < pages && hashed <= first + 26, "pages fingerprinted: {hashed} of {pages}");
    // Textures stay bounded to the pages near the view.
    max_textures = max_textures.max(h.state().canvas.resident_pictures);
    assert!(max_textures <= 12, "textures kept while scrolling: {max_textures}");
    assert!(h.state().canvas.resident_pictures >= 1, "the pages in view stay rendered");
}

/// A harness on `session`'s document, driven as the desktop app is.
fn harness_of(session: wordcraft_engine::Session) -> egui_kittest::Harness<'static, WordApp> {
    let a = WordApp::new(session, Services::default());
    crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| a)
}

/// The body paragraphs to type in, as (what they are, block index): the first, the middle and the last that hold text.
fn typing_spots(doc: &Document) -> Vec<(String, usize)> {
    let texty: Vec<usize> = doc.body.iter().enumerate().filter(|(_, b)| b.as_para().is_some_and(|p| p.len() > 20)).map(|(i, _)| i).collect();
    let longest = doc.body.iter().enumerate().filter_map(|(i, b)| b.as_para().map(|p| (p.len(), i))).max().map(|(_, i)| i);
    let mut spots = Vec::new();
    if let (Some(&first), Some(&last)) = (texty.first(), texty.last()) {
        spots.push(("first paragraph".to_string(), first));
        spots.push(("middle paragraph".to_string(), texty[texty.len() / 2]));
        spots.push(("last paragraph".to_string(), last));
    }
    if let Some(i) = longest {
        spots.push(("longest paragraph".to_string(), i));
    }
    spots
}

/// Typing latency in a long document: the time of the frame that takes a typed letter and the one
/// after it. Run with `cargo test --release -p wordcraft-ui-egui --features craft/screen-testing typing_latency_bench -- --ignored --nocapture`;
/// `WORDCRAFT_BENCH_DOC` names a document (any format the app opens) to type in instead of the generated one.
#[test]
#[ignore = "a benchmark: prints timings"]
fn typing_latency_bench() {
    let mut session = wordcraft_engine::Session::new(Document::new());
    match std::env::var("WORDCRAFT_BENCH_DOC") {
        Ok(path) => {
            session.run("file.open", &serde_json::json!({"path": path})).expect("the document opens");
        }
        Err(_) => {
            let words = ["quick", "brown", "fox", "jumps", "over", "a", "lazy", "dog", "while", "typing"];
            let para = |n: usize, seed: usize| (0..n).map(|i| words[(i * 7 + seed) % words.len()]).collect::<Vec<_>>().join(" ");
            let mut paras: Vec<String> = (0..1500).map(|i| para(30 + i % 90, i)).collect();
            paras[700] = para(3000, 1);
            session = wordcraft_engine::Session::new(Document::from_text(&paras.join("\n")));
        }
    }
    let spots = typing_spots(&session.doc);
    let blocks = session.doc.body.len();
    let mut h = harness_of(session);
    h.run_steps(6);
    let pages = h.state_mut().session.layout().pages.len();
    println!("document: {blocks} blocks, {pages} pages");
    for (label, para_ix) in spots {
        h.state_mut().run("caret.set", serde_json::json!({"pos": {"story": "body", "path": [para_ix], "off": 1}})).unwrap();
        h.run_steps(3);
        let mut passes = (0, 0);
        let (mut first, mut second, mut lay, mut last_pass) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for _ in 0..30 {
            h.input_mut().events.push(egui::Event::Text("a".into()));
            let p0 = h.ctx.cumulative_pass_nr();
            let t = std::time::Instant::now();
            h.step();
            let p1 = h.ctx.cumulative_pass_nr();
            first.push(t.elapsed().as_secs_f64() * 1000.0);
            last_pass.push(h.state().frame_ms);
            let t = std::time::Instant::now();
            h.step();
            second.push(t.elapsed().as_secs_f64() * 1000.0);
            passes = (p1 - p0, h.ctx.cumulative_pass_nr() - p1);
            if std::env::var("WORDCRAFT_BENCH_TAIL").is_ok() {
                let tail: Vec<String> = (0..4)
                    .map(|_| {
                        let t = std::time::Instant::now();
                        h.step();
                        format!("{:.1}", t.elapsed().as_secs_f64() * 1000.0)
                    })
                    .collect();
                println!("  idle steps after: {}", tail.join(" "));
            }
            lay.push(h.state_mut().session.layout().ms);
        }
        let med = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            (v[v.len() / 2], v[v.len() - 1])
        };
        println!(
            "{label}: key frame med/max {:.1?} ms, next frame {:.1?} ms, layout {:.1?} ms, last pass of the key frame {:.1?} ms, passes (key, next) {passes:?}",
            med(&mut first),
            med(&mut second),
            med(&mut lay),
            med(&mut last_pass)
        );
    }
}

/// A target for a sampling profiler: types in a long document for `WORDCRAFT_PROFILE_SECS` seconds.
#[test]
#[ignore = "a profiling target"]
fn typing_profile_target() {
    let secs: u64 = std::env::var("WORDCRAFT_PROFILE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
    let words = ["quick", "brown", "fox", "jumps", "over", "a", "lazy", "dog", "while", "typing"];
    let para = |n: usize, seed: usize| (0..n).map(|i| words[(i * 7 + seed) % words.len()]).collect::<Vec<_>>().join(" ");
    let paras: Vec<String> = (0..1500).map(|i| para(30 + i % 90, i)).collect();
    let mut h = harness_of(wordcraft_engine::Session::new(Document::from_text(&paras.join("\n"))));
    h.run_steps(6);
    h.state_mut().run("caret.set", serde_json::json!({"pos": {"story": "body", "path": [3], "off": 5}})).unwrap();
    h.run_steps(3);
    let mode = std::env::var("WORDCRAFT_PROFILE_MODE").unwrap_or_default();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < end {
        if mode != "idle" {
            h.input_mut().events.push(egui::Event::Text("a".into()));
        }
        h.step();
        if mode != "idle" {
            h.step();
        }
    }
}

/// What the fixed cost of a frame is made of: an idle frame of a one-line document on each ribbon tab, and with the ribbon
/// collapsed. Run like [`typing_latency_bench`].
#[test]
#[ignore = "a benchmark: prints timings"]
fn idle_frame_bench() {
    let mut h = harness_of(wordcraft_engine::Session::new(Document::from_text("One line of text.")));
    h.run_steps(8);
    let time = |h: &mut egui_kittest::Harness<'static, WordApp>| {
        h.run_steps(3);
        let mut v: Vec<f64> = (0..15)
            .map(|_| {
                let t = std::time::Instant::now();
                h.step();
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    for tab in ["Home", "Insert", "Draw", "Design", "Layout", "References", "Mailings", "Review", "View"] {
        h.state_mut().run("ui.tab", serde_json::json!({"tab": tab})).unwrap();
        println!("idle frame, {tab} tab: {:.1} ms", time(&mut h));
    }
    h.state_mut().run("ui.tab", serde_json::json!({"tab": "Home"})).unwrap();
    h.state_mut().run("ui.collapseRibbon", serde_json::json!({})).unwrap();
    println!("idle frame, ribbon collapsed: {:.1} ms", time(&mut h));
}

/// What an edit costs the engine without the interface: the command, then the layout of the edited document. Run like
/// [`typing_latency_bench`].
#[test]
#[ignore = "a benchmark: prints timings"]
fn edit_and_layout_bench() {
    let words = ["quick", "brown", "fox", "jumps", "over", "a", "lazy", "dog", "while", "typing"];
    let para = |n: usize, seed: usize| (0..n).map(|i| words[(i * 7 + seed) % words.len()]).collect::<Vec<_>>().join(" ");
    let paras: Vec<String> = (0..1500).map(|i| para(30 + i % 90, i)).collect();
    let mut s = wordcraft_engine::Session::new(Document::from_text(&paras.join("\n")));
    s.layout();
    s.run("caret.set", &serde_json::json!({"pos": {"story": "body", "path": [700], "off": 1}})).unwrap();
    let (mut edit, mut lay) = (Vec::new(), Vec::new());
    let rounds: usize = std::env::var("WORDCRAFT_BENCH_ROUNDS").ok().and_then(|r| r.parse().ok()).unwrap_or(40);
    for _ in 0..rounds {
        let t = std::time::Instant::now();
        s.run("text.insert", &serde_json::json!({"text": "a"})).unwrap();
        edit.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = std::time::Instant::now();
        s.layout();
        lay.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let med = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    println!("edit command {:.2} ms, layout {:.2} ms (206-page document)", med(&mut edit), med(&mut lay));
}
