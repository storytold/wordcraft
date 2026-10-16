//! Headless UI screenshots: renders the whole WordCraft window offscreen (wgpu), so it works
//! with a locked screen or a hidden window.
//!
//! `cargo run -p wordcraft-ui-egui --example ui_shot -- script.jsonl`
//!
//! The script is JSON lines: control-channel requests (`{"method": …, "params": …}`, see
//! `docs/control-protocol.md`), `{"shot": "/abs/out.png"}` to save the window as PNG, or
//! `{"steps": n}` to run extra frames, or `{"wait": ms}` to keep running frames for a while (for
//! background work such as a Zotero session). The window is 1440×900 pt at 2× and opens the sample
//! document unless the first line is `{"empty": true}`; `{"empty": true, "startScreen": true}`
//! starts the way the desktop app does with nothing to open (File › Home, #766).
//!
//! Two optional keys describe the *display*, which is not the interface zoom:
//! `{"nativePixelsPerPoint": 1.0}` sets the display's native scale (any finite value from 0.5 to 4;
//! anything else, or no key at all, keeps the 2× default) and `{"traceScale": true}` prints one JSON
//! line per capture with the values the harness really reports — the native scale, the interface
//! zoom factor, the effective pixels per point and the viewport in egui points and in device pixels.
//! A 760-point window with `nativePixelsPerPoint: 1` and `ui.interfaceSize` 200 % therefore really
//! draws at zoom 2, while the same width with the 2× default and 100 % draws at zoom 0.5.

use wordcraft_engine::Session;
use wordcraft_ui_egui::{ControlRequest, Services, WordApp};

static READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn main() {
    let path = std::env::args().nth(1).expect("usage: ui_shot script.jsonl");
    let script = std::fs::read_to_string(&path).expect("read script");
    let (tx, rx) = std::sync::mpsc::channel::<ControlRequest>();
    let lines: Vec<serde_json::Value> =
        script.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).expect("json line")).collect();
    let doc = if lines.first().is_some_and(|l| l.get("empty").is_some()) {
        wordcraft_doc::Document::new()
    } else {
        wordcraft_engine::sample::sample_document()
    };
    let mut app = WordApp::new(Session::new(doc), Services::default()).with_control(rx);
    app.integrated_titlebar = true;
    if lines.first().is_some_and(|l| l.get("startScreen").is_some()) {
        app.start_screen();
    }
    let w = lines.iter().find_map(|l| l.get("width").and_then(|v| v.as_f64())).unwrap_or(1440.0) as f32;
    let h = lines.iter().find_map(|l| l.get("height").and_then(|v| v.as_f64())).unwrap_or(900.0) as f32;
    let native = native_pixels_per_point(&lines);
    let trace = trace_scale(&lines);
    let mut harness =
        egui_kittest::Harness::builder().with_size(egui::vec2(w, h)).with_pixels_per_point(native).with_max_steps(1_000_000).wgpu().build_ui_state(
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
    harness.input_mut().max_texture_side = Some(8192);
    READY.store(true, std::sync::atomic::Ordering::Relaxed);
    step_n(&mut harness, 6);
    for l in lines {
        if let Some(p) = l.get("shot").and_then(|v| v.as_str()) {
            step_n(&mut harness, 4);
            if trace {
                println!("{}", scale_trace(&harness, p));
            }
            match harness.render() {
                Ok(img) => {
                    img.save(p).expect("save png");
                    println!("{p}");
                }
                Err(e) => eprintln!("render failed: {e}"),
            }
        } else if let Some(n) = l.get("steps").and_then(|v| v.as_u64()) {
            step_n(&mut harness, n as usize);
        } else if let Some(ms) = l.get("wait").and_then(|v| v.as_u64()) {
            let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
            while std::time::Instant::now() < end {
                step(&mut harness);
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        } else if let Some(m) = l.get("method").and_then(|v| v.as_str()) {
            let (req, reply) = ControlRequest::new(m, l.get("params").cloned().unwrap_or_default());
            tx.send(req).expect("send");
            for _ in 0..60 {
                step(&mut harness);
                if let Ok(r) = reply.try_recv() {
                    let s = r.to_string();
                    // `UI_SHOT_FULL=1` prints whole replies (e.g. `document.inspect` for scripted checks).
                    let cut = if std::env::var_os("UI_SHOT_FULL").is_some() { s.len() } else { s.len().min(300) };
                    println!("{m}: {}", s.get(..cut).unwrap_or(&s));
                    break;
                }
            }
            step_n(&mut harness, 2);
        }
    }
}

/// The display's native scale from the script: any finite value from 0.5 to 4, otherwise the 2×
/// default (an older script with no key behaves exactly as before).
fn native_pixels_per_point(lines: &[serde_json::Value]) -> f32 {
    const DEFAULT: f32 = 2.0;
    lines
        .iter()
        .find_map(|l| l.get("nativePixelsPerPoint").and_then(|v| v.as_f64()))
        .map(|v| v as f32)
        .filter(|v| v.is_finite() && (0.5..=4.0).contains(v))
        .unwrap_or(DEFAULT)
}

/// Whether the script asked for the measured scale trace (`{"traceScale": true}`).
fn trace_scale(lines: &[serde_json::Value]) -> bool {
    lines.iter().any(|l| l.get("traceScale").and_then(|v| v.as_bool()).unwrap_or(false))
}

/// One JSON line with what the harness really reports at a settled capture: the display's native
/// scale, the interface zoom factor, the effective pixels per point, and the viewport in egui points
/// and in device pixels (egui points × the effective pixels per point).
fn scale_trace(harness: &egui_kittest::Harness<'_, WordApp>, shot: &str) -> String {
    let ctx = &harness.ctx;
    let ppp = ctx.pixels_per_point();
    let zoom = ctx.zoom_factor();
    let native = ctx.native_pixels_per_point().unwrap_or(ppp);
    let rect = ctx.input(|i| i.viewport_rect());
    let gui = rect.size();
    let device = egui::vec2(gui.x * ppp, gui.y * ppp);
    serde_json::json!({
        "shot": shot,
        "nativePixelsPerPoint": native,
        "zoomFactor": zoom,
        "pixelsPerPoint": ppp,
        "guiSize": [gui.x, gui.y],
        "deviceSize": [device.x, device.y],
        "viewportMin": [rect.min.x, rect.min.y],
    })
    .to_string()
}

fn step(harness: &mut egui_kittest::Harness<'_, WordApp>) {
    let mut raw = std::mem::take(harness.input_mut());
    harness.state_mut().raw_input_hook(&mut raw);
    *harness.input_mut() = raw;
    harness.step();
}

fn step_n(harness: &mut egui_kittest::Harness<'_, WordApp>, n: usize) {
    for _ in 0..n {
        step(harness);
    }
}

#[cfg(test)]
mod tests {
    use super::{native_pixels_per_point, trace_scale};

    fn lines(v: serde_json::Value) -> Vec<serde_json::Value> {
        vec![v]
    }

    #[test]
    fn the_display_scale_defaults_to_two_and_accepts_only_sane_values() {
        // No key at all: an older script behaves exactly as before.
        assert_eq!(native_pixels_per_point(&lines(serde_json::json!({"width": 760.0}))), 2.0);
        assert_eq!(native_pixels_per_point(&[]), 2.0);
        // A sane value is honoured, including the ends of the accepted range.
        for v in [0.5, 1.0, 1.5, 2.0, 3.0, 4.0] {
            assert_eq!(native_pixels_per_point(&lines(serde_json::json!({"nativePixelsPerPoint": v}))), v as f32, "{v} must be honoured");
        }
        // Nonsense, out-of-range and non-numeric values fall back rather than reaching the harness.
        for v in [0.25, 0.0, -1.0, 4.5, 100.0] {
            assert_eq!(native_pixels_per_point(&lines(serde_json::json!({"nativePixelsPerPoint": v}))), 2.0, "{v} must fall back");
        }
        assert_eq!(native_pixels_per_point(&lines(serde_json::json!({"nativePixelsPerPoint": "1.5"}))), 2.0, "a string is not a scale");
        assert_eq!(native_pixels_per_point(&lines(serde_json::json!({"nativePixelsPerPoint": null}))), 2.0);
    }

    #[test]
    fn the_trace_is_off_unless_the_script_asks_for_it() {
        assert!(!trace_scale(&[]));
        assert!(!trace_scale(&lines(serde_json::json!({"width": 760.0}))));
        assert!(!trace_scale(&lines(serde_json::json!({"traceScale": false}))));
        assert!(!trace_scale(&lines(serde_json::json!({"traceScale": "yes"}))));
        assert!(trace_scale(&lines(serde_json::json!({"traceScale": true}))));
        // Any line may carry it, not only the first.
        assert!(trace_scale(&[serde_json::json!({"width": 760.0}), serde_json::json!({"traceScale": true})]));
    }
}
