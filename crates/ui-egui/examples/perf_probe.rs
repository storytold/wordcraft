//! Responsiveness and memory probe: runs the real WordCraft UI headless (like `ui_shot`, without a
//! window or focus), opens a document (or generates a `--synthetic` workload), and times the frames
//! that handle idle rendering, typing, edit/undo bursts, scrolling, zooming, ribbon tabs, and
//! window resizes. Prints numbers only (JSON), never document text.
//!
//! `cargo run --release -p wordcraft-ui-egui --example perf_probe -- [--synthetic | doc.docx] [block]`
//!
//! A frame's time is the CPU time of one harness step (`WordApp::logic` + `ui`: layout, page
//! rasterisation, widgets). What the user sees adds the wait for the next frame and the
//! presentation on screen (up to one display refresh). Input goes in through control requests,
//! as an agent sends it.

use std::sync::mpsc::Sender;
use std::time::Instant;

use craft::automation::control::{ControlChannel, ControlRequest};
use serde_json::{Value, json};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::table::{Cell, Table};
use wordcraft_doc::{Block, Document, Path, Pos, StoryRef};
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp};

static READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

type Harness<'a> = egui_kittest::Harness<'a, WordApp>;

/// Drains `TexturesDelta` every frame instead of letting `egui_kittest::LazyRenderer::Uninitialized`
/// hoard every uploaded page/tile image across the probe run.
struct DrainRenderer;

impl egui_kittest::TestRenderer for DrainRenderer {
    fn handle_delta(&mut self, delta: &mut egui::TexturesDelta) {
        delta.clear();
    }

    fn render(&mut self, _ctx: &egui::Context, _output: &egui::FullOutput) -> Result<image::RgbaImage, String> {
        Err("perf_probe runs headless without framebuffer readback".into())
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Reads Linux `/proc/self/status` for current resident memory (`VmRSS`) and peak (`VmHWM`) in MiB.
fn memory_mb() -> Value {
    #[cfg(target_os = "linux")]
    {
        let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
            return Value::Null;
        };
        let parse_kb = |prefix: &str| -> Option<f64> {
            status.lines().find_map(|line| {
                let rest = line.strip_prefix(prefix)?.trim();
                let kb: f64 = rest.split_whitespace().next()?.parse().ok()?;
                Some(round1(kb / 1024.0))
            })
        };
        json!({"rss_mb": parse_kb("VmRSS:"), "hwm_mb": parse_kb("VmHWM:")})
    }
    #[cfg(not(target_os = "linux"))]
    {
        Value::Null
    }
}

/// Builds a deterministic in-memory workload combining styled paragraphs, a large table, embedded
/// PNG images, and tracked revisions so baselines need no external `.docx` fixture.
fn synthetic_session() -> Session {
    let mut text = String::new();
    let sentence = "The quick brown fox jumps over the lazy dog while layout, styles, numbering, and proofing run across pages.";
    for i in 0..1156 {
        if i == 100 {
            text.push_str(&format!("Paragraph {i}: "));
            for rep in 0..25 {
                if rep > 0 {
                    text.push(' ');
                }
                text.push_str(sentence);
            }
            text.push('\n');
        } else {
            text.push_str(&format!("Paragraph {i}: {sentence}\n"));
        }
    }
    let mut doc = Document::from_text(&text);
    for i in (0..1156).step_by(20) {
        let _ = doc.format_paragraphs(&Pos::body(i, 0), &Pos::body(i, 0), &|p| p.style = Some("Heading1".into()));
    }

    let mut table = Table::new(250, 8, 468.0);
    for (r, row) in table.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            let w = cell.props.width;
            *cell = Cell::with_text(&format!("R{r} C{c} value {}", r * 8 + c));
            cell.props.width = w;
        }
    }
    let _ = doc.insert_block(StoryRef::Body, &Path::top(50), Block::Table(table));

    for img_idx in 0..16_u8 {
        let mut rgba = image::RgbaImage::new(256, 256);
        for (x, y, px) in rgba.enumerate_pixels_mut() {
            *px = image::Rgba([
                img_idx.wrapping_mul(17).wrapping_add((x % 256) as u8),
                img_idx.wrapping_mul(31).wrapping_add((y % 256) as u8),
                200_u8.wrapping_sub(img_idx.wrapping_mul(11)),
                255,
            ]);
        }
        if let Ok(png) = wordcraft_geom::picture::encode_png(&image::DynamicImage::ImageRgba8(rgba)) {
            let key = doc.add_media(png, "png");
            let block_idx = 2 + usize::from(img_idx) * 2;
            let _ = doc.insert_object(
                &Pos::body(block_idx, 0),
                InlineObject::Image {
                    media: key,
                    w: 144.0,
                    h: 144.0,
                    alt: format!("Synthetic image {img_idx}"),
                    float: Default::default(),
                    crop: [0.0; 4],
                    fx: Default::default(),
                    ole: None,
                },
                &Default::default(),
            );
        }
    }

    let mut session = Session::new(doc);
    let _ = session.run("review.trackChanges", &json!({"enabled": true}));
    let _ = session.run("caret.set", &json!({"pos": {"story": "body", "path": [0], "off": 0}}));
    let _ = session.run("text.insert", &json!({"text": "Tracked prefix "}));
    let _ = session.run("review.trackChanges", &json!({"enabled": false}));
    session
}

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
    json!({"n": s.len(), "med": round1(at(0.5)), "p95": round1(at(0.95)), "max": round1(at(1.0))})
}

/// Opens the document named on the command line (or builds `--synthetic`) and prints JSON timings
/// and memory metrics for start-up, idle, typing, edit/undo bursts, scrolling, zooming, ribbon
/// tabs and window resizes.
fn main() {
    let path = std::env::args().nth(1).expect("usage: perf_probe [--synthetic | doc.docx] [block]");
    let block: Option<usize> = std::env::args().nth(2).and_then(|b| b.parse().ok());
    let mem_start = memory_mb();
    let t_open = Instant::now();
    let session = if path == "--synthetic" {
        synthetic_session()
    } else {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("file.open", &json!({"path": path})).expect("open");
        s
    };
    let open_ms = t_open.elapsed().as_secs_f64() * 1000.0;
    let mem_after_open = memory_mb();
    let (tx, rx) = std::sync::mpsc::channel::<ControlRequest>();
    let app = WordApp::new(session, Services::default()).with_control(ControlChannel::new(rx));
    let mut h = egui_kittest::Harness::builder()
        .renderer(DrainRenderer)
        .with_size(egui::vec2(1440.0, 900.0))
        .with_pixels_per_point(2.0)
        .with_max_steps(1_000_000)
        .build_ui_state(
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
    let mem_after_first = memory_mb();

    let idle: Vec<f64> = (0..60).map(|_| step(&mut h)).collect();
    let mem_after_idle = memory_mb();

    let mut out = json!({
        "source": if path == "--synthetic" { "synthetic" } else { "file" },
        "open_ms": round1(open_ms),
        "pages": h.state_mut().session.layout().pages.len(),
        "first_frames_ms": first.iter().map(|x| round1(*x)).collect::<Vec<_>>(),
        "idle_60_frames_ms": stats(&idle),
    });

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
    let mem_after_typing = memory_mb();

    // Edit/Undo burst: 40 discrete paragraph insertions followed by 40 undos.
    let mut edit_burst = Vec::new();
    let mut undo_burst = Vec::new();
    for k in 0..40 {
        h.state_mut().session.close_typing();
        exec(&tx, "text.insert", json!({"text": format!(" {k}")}));
        edit_burst.push(step(&mut h));
    }
    let mem_after_edits = memory_mb();
    for _ in 0..40 {
        exec(&tx, "edit.undo", json!({}));
        undo_burst.push(step(&mut h));
    }
    let mem_after_undo = memory_mb();
    out["edit_burst_40_ms"] = stats(&edit_burst);
    out["undo_burst_40_ms"] = stats(&undo_burst);

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
    let mem_after_scroll = memory_mb();

    // Zoom, ribbon tabs, resize: the frame after the change.
    let mut zoom = Vec::new();
    for z in [50, 75, 100, 150, 200, 100] {
        exec(&tx, "view.zoom", json!({"value": z}));
        let a = step(&mut h);
        let b = step(&mut h);
        zoom.push(json!({"zoom": z, "frame_ms": round1(a), "next_ms": round1(b)}));
    }
    out["zoom"] = json!(zoom);
    let mem_after_zoom = memory_mb();
    let mut tabs = Vec::new();
    for t in ["Insert", "Draw", "Design", "Layout", "References", "Mailings", "Review", "View", "Home"] {
        exec(&tx, "ui.tab", json!({"tab": t}));
        tabs.push(step(&mut h));
    }
    out["ribbon_tab_ms"] = stats(&tabs);
    let mem_after_ribbon = memory_mb();
    let mut rs = Vec::new();
    for w in (800..=1440).rev().step_by(40).chain((800..=1440).step_by(40)) {
        h.set_size(egui::vec2(w as f32, 900.0));
        rs.push(step(&mut h));
    }
    out["resize_ms"] = stats(&rs);
    let mem_after_resize = memory_mb();
    // Accept all (if the document has tracked changes).
    if !h.state().session.doc.revisions.is_empty() {
        exec(&tx, "review.acceptAll", json!({}));
        let f = step(&mut h);
        out["accept_all"] = json!({"frame_ms": round1(f)});
    }
    let mem_end = memory_mb();
    let tex_mgr = h.ctx.tex_manager();
    let tm = tex_mgr.read();
    let tex_bytes: usize = tm.allocated().map(|(_, m)| m.size[0] * m.size[1] * m.bytes_per_pixel).sum();
    out["textures_end"] = json!({"count": tm.num_allocated(), "mb": round1(tex_bytes as f64 / (1024.0 * 1024.0))});
    drop(tm);
    out["memory"] = json!({
        "start": mem_start,
        "after_open": mem_after_open,
        "after_first_frames": mem_after_first,
        "after_idle": mem_after_idle,
        "after_typing": mem_after_typing,
        "after_edits": mem_after_edits,
        "after_undo": mem_after_undo,
        "after_scroll": mem_after_scroll,
        "after_zoom": mem_after_zoom,
        "after_ribbon": mem_after_ribbon,
        "after_resize": mem_after_resize,
        "end": mem_end,
    });
    println!("{out}");
}
