//! Where does per-keystroke layout time go on a huge document?
//! Prints LayoutCache hits/misses around a single typed keystroke.
use serde_json::json;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp};

fn main() {
    let path = std::env::args().nth(1).expect("usage: cache_probe doc.docx");
    let mut session = Session::new(wordcraft_doc::Document::new());
    session.run("file.open", &json!({"path": path})).expect("open");
    let mut app = WordApp::new(session, Services::default());

    // Warm up: one layout so shapes are cached.
    let (h0, _m0) = {
        let l = app.session.layout();
        (l.ms, 0)
    };
    let _ = h0;
    let (hits0, misses0) = {
        let c = app.session.cache_stats();
        (c.0, c.1)
    };

    // Type one character in the middle of the first big paragraph.
    let t = std::time::Instant::now();
    app.session.run("text.insert", &json!({"text": "x"})).unwrap();
    let insert_ms = t.elapsed().as_secs_f64() * 1000.0;

    // The layout that follows the keystroke.
    let l = app.session.layout();
    let (hits1, misses1) = app.session.cache_stats();
    println!(
        "{{\"layout_ms_after_keystroke\": {}, \"insert_ms\": {}, \"hits\": {}, \"misses\": {}, \"hits0\": {}, \"misses0\": {}}}",
        l.ms,
        (insert_ms * 10.0).round() / 10.0,
        hits1 - hits0,
        misses1 - misses0,
        hits0,
        misses0
    );
}
