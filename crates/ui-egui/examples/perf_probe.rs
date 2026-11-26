//! Responsiveness probe: runs the real WordCraft UI headless (like `ui_shot`, without a window or
//! focus), opens a document, and times the frames that handle typing, scrolling, zooming,
//! ribbon tabs and window resizes. Prints numbers only (JSON), never document text.
//!
//! `cargo run --release -p wordcraft-ui-egui --example perf_probe -- doc.docx [block]`
//!
//! A frame's time is the CPU time of one harness step (`WordApp::logic` + `ui`: layout, page
//! rasterisation, widgets). What the user sees adds the wait for the next frame and the
//! presentation on screen (up to one display refresh). Input goes in through control requests,
//! as an agent sends it.

use std::sync::mpsc::Sender;
use std::time::Instant;

use craft::automation::control::{ControlChannel, ControlRequest};
use serde_json::{Value, json};
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp};

static READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

type Harness<'a> = egui_kittest::Harness<'a, WordApp>;

/// Queues a control request; the app answers it in the logic phase of a following frame.
fn request(tx: &Sender<ControlRequest>, method: &str, params: Value) {
    let (req, _reply) = ControlRequest::new(method, params);
    tx.send(req).expect("the app is running");
}

/// Queues a command through `engine.execute`.
fn exec(tx: &Sender<ControlRequest>, command: &str, params: Value) {
    request(tx, "engine.execute", json!({"command": command, "params": params}));
}

/// Hands the app's frame input to the runtime, runs one full app frame and returns its wall-clock
/// duration in milliseconds.
fn step(h: &mut Harness<'_>) -> f64 {
    let ctx = h.ctx.clone();
    craft::ui::shell::embed::before_input(&ctx, |out| h.state_mut().frame_input(out));
    let t = Instant::now();
    h.step();
    t.elapsed().as_secs_f64() * 1000.0
}

fn stats(v: &[f64]) -> Value {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let at = |q: f64| s.get(((s.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0.0);
    json!({"n": s.len(), "med": (at(0.5) * 10.0).round() / 10.0, "p95": (at(0.95) * 10.0).round() / 10.0, "max": (at(1.0) * 10.0).round() / 10.0})
}

/// Opens the document named on the command line and prints JSON timings for start-up, typing,
/// scrolling, zooming, ribbon tabs and window resizes.
fn main() {
    let path = std::env::args().nth(1).expect("usage: perf_probe doc.docx [block]");
    let block: Option<usize> = std::env::args().nth(2).and_then(|b| b.parse().ok());
    let t_open = Instant::now();
    let mut session = Session::new(wordcraft_doc::Document::new());
    session.run("file.open", &json!({"path": path})).expect("open");
    let open_ms = t_open.elapsed().as_secs_f64() * 1000.0;
    let (tx, rx) = std::sync::mpsc::channel::<ControlRequest>();
    let app = WordApp::new(session, Services::default()).with_control(ControlChannel::new(rx));
    let mut h =
        egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_pixels_per_point(2.0).with_max_steps(1_000_000).build_ui_state(
            |ui, app: &mut WordApp| {
                if !READY.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
    // PLAN(app-hub): craft-ui `testing` feature and craft-automation in dev-dependencies
    // PLAN(shell-w11): assumed `wordcraft_ui_egui::ui_fonts` and `i18n::Lang::EN`
    if let Err(error) =
        craft::ui::shell::embed::install(&h.ctx, craft::ui::testing::install_spec(), &wordcraft_ui_egui::ui_fonts(wordcraft_ui_egui::i18n::Lang::EN))
    {
        eprintln!("installing the UI runtime failed: {error:?}");
        std::process::exit(1);
    }
    READY.store(true, std::sync::atomic::Ordering::Relaxed);
    let first: Vec<f64> = (0..6).map(|_| step(&mut h)).collect();
    let mut out =
        json!({"open_ms": (open_ms * 10.0).round() / 10.0, "first_frames_ms": first.iter().map(|x| (x * 10.0).round() / 10.0).collect::<Vec<_>>()});

    // Typing: the caret in the middle of `block` (default: the longest paragraph), 40 characters.
    let doc_block = block.unwrap_or_else(|| {
        let s = &h.state().session;
        s.doc.body.iter().enumerate().filter_map(|(i, b)| b.as_para().map(|p| (i, p.len()))).max_by_key(|(_, l)| *l).map(|(i, _)| i).unwrap_or(0)
    });
    let typing = |h: &mut Harness<'_>, pos: Value| -> Value {
        // One click gives the page the keyboard; the caret is then put where the probe types.
        request(&tx, "ui.click", json!({"x": 720.0, "y": 500.0}));
        for _ in 0..3 {
            step(h);
        }
        exec(&tx, "caret.set", json!({"pos": pos}));
        let ctx = h.ctx.clone();
        h.state_mut().canvas.focus_page(&ctx);
        for _ in 0..3 {
            step(h);
        }
        let mut frames = Vec::new();
        let mut lay = Vec::new();
        for _ in 0..40 {
            request(&tx, "ui.type", json!({"text": "a"}));
            frames.push(step(h));
            // the frame after (relayout of a long paragraph may finish there)
            frames.push(step(h));
            lay.push(h.state_mut().session.layout().ms);
        }
        let typed = frames.chunks(2).map(|c| c[0] + c[1]).collect::<Vec<_>>();
        json!({"frame_with_key_ms": stats(&frames.iter().step_by(2).copied().collect::<Vec<_>>()), "two_frames_ms": stats(&typed), "layout_ms": stats(&lay)})
    };
    let mid = h.state().session.doc.body.get(doc_block).and_then(|b| b.as_para()).map(|p| p.len() / 2).unwrap_or(0);
    // a char boundary at or before the middle
    let mid = {
        let p = h.state().session.doc.body.get(doc_block).and_then(|b| b.as_para()).map(|p| p.plain_text()).unwrap_or_default();
        (0..=mid.min(p.len())).rev().find(|i| p.is_char_boundary(*i)).unwrap_or(0)
    };
    out["typing_long_para"] = typing(&mut h, json!({"story": "body", "path": [doc_block], "off": mid}));
    out["typing_para_len"] = json!(h.state().session.doc.body.get(doc_block).and_then(|b| b.as_para()).map(|p| p.len()).unwrap_or(0));
    let table = h.state().session.doc.body.iter().position(|b| matches!(&**b, wordcraft_doc::Block::Table(_)));
    if let Some(t) = table {
        out["typing_table"] = typing(&mut h, json!({"story": "body", "path": [t, 0, 0, 0], "off": 0}));
    }

    // Scrolling: mouse wheel (120 pt notches) and trackpad (20 pt gestures) over the canvas.
    let scroll = |h: &mut Harness<'_>, dy: f32, n: usize| -> Value {
        exec(&tx, "caret.docStart", json!({}));
        for _ in 0..3 {
            step(h);
        }
        let mut f = Vec::new();
        for _ in 0..n {
            request(&tx, "ui.wheel", json!({"x": 720.0, "y": 500.0, "dy": dy}));
            f.push(step(h));
        }
        json!({"frame_ms": stats(&f), "frames_over_33ms": f.iter().filter(|x| **x > 33.0).count(), "frames_over_16ms": f.iter().filter(|x| **x > 16.7).count()})
    };
    out["scroll_wheel"] = scroll(&mut h, -120.0, 120);
    out["scroll_trackpad"] = scroll(&mut h, -20.0, 300);

    // Zoom, ribbon tabs, resize: the frame after the change.
    let mut zoom = Vec::new();
    for z in [50, 75, 100, 150, 200, 100] {
        exec(&tx, "view.zoom", json!({"value": z}));
        let a = step(&mut h);
        let b = step(&mut h);
        zoom.push(json!({"zoom": z, "frame_ms": (a * 10.0).round() / 10.0, "next_ms": (b * 10.0).round() / 10.0}));
    }
    out["zoom"] = json!(zoom);
    let mut tabs = Vec::new();
    for t in ["Insert", "Draw", "Design", "Layout", "References", "Mailings", "Review", "View", "Home"] {
        exec(&tx, "ui.tab", json!({"tab": t}));
        tabs.push(step(&mut h));
    }
    out["ribbon_tab_ms"] = stats(&tabs);
    let mut rs = Vec::new();
    for w in (800..=1440).rev().step_by(40).chain((800..=1440).step_by(40)) {
        h.set_size(egui::vec2(w as f32, 900.0));
        rs.push(step(&mut h));
    }
    out["resize_ms"] = stats(&rs);
    // Accept all (if the document has tracked changes).
    if !h.state().session.doc.revisions.is_empty() {
        exec(&tx, "review.acceptAll", json!({}));
        let f = step(&mut h);
        out["accept_all"] = json!({"frame_ms": (f * 10.0).round() / 10.0});
    }
    println!("{out}");
}
