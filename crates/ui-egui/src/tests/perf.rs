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
