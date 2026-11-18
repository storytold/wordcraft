//! Responsiveness probe: runs the real WordCraft UI headless (like `ui_shot`, without a window or
//! focus), opens a document, and times the frames that handle typing, scrolling, zooming,
//! ribbon tabs and window resizes. Prints numbers only (JSON), never document text.
//!
//! `cargo run --release -p wordcraft-ui-egui --example perf_probe -- doc.docx [block]`
//!
//! A frame's time is the CPU time of `WordApp::logic` + `ui` for that frame (layout, page
//! rasterisation, widgets). What the user sees adds the wait for the next frame and the
//! presentation on screen (up to one display refresh).

use std::time::Instant;

use serde_json::json;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp};

static READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Applies the harness's pending raw input, runs one full app frame, and returns its wall-clock
/// duration in milliseconds.
fn step(h: &mut egui_kittest::Harness<'_, WordApp>) -> f64 {
    let mut raw = std::mem::take(h.input_mut());
    let ctx = h.ctx.clone();
    h.state_mut().raw_input_hook(&ctx, &mut raw);
    *h.input_mut() = raw;
    let t = Instant::now();
    h.step();
    t.elapsed().as_secs_f64() * 1000.0
}

fn stats(v: &[f64]) -> serde_json::Value {
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
    let app = WordApp::new(session, Services::default());
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
    READY.store(true, std::sync::atomic::Ordering::Relaxed);
    let first: Vec<f64> = (0..6).map(|_| step(&mut h)).collect();
    let mut out =
        json!({"open_ms": (open_ms * 10.0).round() / 10.0, "first_frames_ms": first.iter().map(|x| (x * 10.0).round() / 10.0).collect::<Vec<_>>()});

    // Typing: the caret in the middle of `block` (default: the longest paragraph), 40 characters.
    let doc_block = block.unwrap_or_else(|| {
        let s = &h.state().session;
        s.doc.body.iter().enumerate().filter_map(|(i, b)| b.as_para().map(|p| (i, p.len()))).max_by_key(|(_, l)| *l).map(|(i, _)| i).unwrap_or(0)
    });
    let typing = |h: &mut egui_kittest::Harness<'_, WordApp>, pos: serde_json::Value| -> serde_json::Value {
        h.state_mut().session.run("caret.set", &json!({"pos": pos})).ok();
        h.state_mut().canvas.want_focus = true;
        for _ in 0..3 {
            step(h);
        }
        let mut frames = Vec::new();
        let (mut lay, mut ren) = (Vec::new(), Vec::new());
        for _ in 0..40 {
            h.input_mut().events.push(egui::Event::Text("a".into()));
            frames.push(step(h));
            // the frame after (relayout of a long paragraph may finish there)
            frames.push(step(h));
            lay.push(h.state_mut().session.layout().ms);
            ren.push(h.state().canvas.render_ms);
        }
        let typed = frames.chunks(2).map(|c| c[0] + c[1]).collect::<Vec<_>>();
        json!({"frame_with_key_ms": stats(&frames.iter().step_by(2).copied().collect::<Vec<_>>()), "two_frames_ms": stats(&typed), "layout_ms": stats(&lay), "render_ms": stats(&ren)})
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
    let scroll = |h: &mut egui_kittest::Harness<'_, WordApp>, dy: f32, n: usize| -> serde_json::Value {
        h.state_mut().session.run("caret.docStart", &json!({})).ok();
        for _ in 0..3 {
            step(h);
        }
        let mut f = Vec::new();
        for _ in 0..n {
            h.input_mut().events.push(egui::Event::PointerMoved(egui::pos2(720.0, 500.0)));
            h.input_mut().events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, dy),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
                source: egui::MouseWheelSource::Unknown,
            });
            f.push(step(h));
        }
        json!({"frame_ms": stats(&f), "frames_over_33ms": f.iter().filter(|x| **x > 33.0).count(), "frames_over_16ms": f.iter().filter(|x| **x > 16.7).count()})
    };
    out["scroll_wheel"] = scroll(&mut h, -120.0, 120);
    out["scroll_trackpad"] = scroll(&mut h, -20.0, 300);

    // Zoom, ribbon tabs, resize: the frame after the change.
    let mut zoom = Vec::new();
    for z in [50, 75, 100, 150, 200, 100] {
        h.state_mut().session.run("view.zoom", &json!({"value": z})).ok();
        let a = step(&mut h);
        let b = step(&mut h);
        zoom.push(json!({"zoom": z, "frame_ms": (a * 10.0).round() / 10.0, "next_ms": (b * 10.0).round() / 10.0}));
    }
    out["zoom"] = json!(zoom);
    let mut tabs = Vec::new();
    for t in ["Insert", "Draw", "Design", "Layout", "References", "Mailings", "Review", "View", "Home"] {
        h.state_mut().execute("ui.tab", json!({"tab": t})).ok();
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
        let t = Instant::now();
        h.state_mut().session.run("review.acceptAll", &json!({})).ok();
        let cmd = t.elapsed().as_secs_f64() * 1000.0;
        let f = step(&mut h);
        out["accept_all"] = json!({"command_ms": (cmd * 10.0).round() / 10.0, "frame_ms": (f * 10.0).round() / 10.0});
    }
    println!("{out}");
}
