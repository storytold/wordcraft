//! Programmatic control of the running app (agents, tests, MCP bridge).
//!
//! Methods (JSON lines over the host's transport, see `docs/control-protocol.md`):
//! - `engine.execute {command, params, doc?}`: run any command (engine or `ui.*`)
//! - `engine.commands {search?}`: engine and app print commands with enablement and parameters
//! - `document.inspect`: document structure; `ui.inspect`: UI state, canvas geometry, perf
//! - `ui.wheel {x, y, dx?, dy, unit?, phase?}`: a mouse-wheel / trackpad scroll at a window point.
//! - `ui.click {x, y, button?, count?, shift?, cmd?}`, `ui.move {x, y}`, `ui.drag {x, y, toX, toY}`:
//!   real pointer input in screen points (reaches every widget)
//! - `ui.clickText {page, x, y}`: click at a page position (points from the page's top-left)
//! - `ui.key {key, shift?, alt?, cmd?}`, `ui.text {text}`: keyboard input through egui
//! - `ui.screenshot {path?}`: PNG of the window; `ui.render {path?, page?, scale?}`: page PNG
//!   (without `path`, both reply with the PNG as base64 `png`)
//! - `ui.parity`: feature catalog parity
//!
//! `engine.execute`, `document.inspect` and `ui.render` take `doc`, the id of an open document
//! (`view.switchWindows` lists them): they work on that document while the window keeps showing
//! the active one. Without `doc` they work on the active document.

use std::sync::mpsc::Sender;

use craft_ui::capture::CaptureSlot;

use serde_json::{Value, json};

use crate::WordApp;

mod geometry;
mod handler;
mod screenshots;

pub(crate) use geometry::{CaptureGeometry, SurfaceGeometry};
pub(crate) use handler::close_refused;
pub use handler::{Outcome, handle, inspect};
pub use screenshots::save_screenshot;
#[cfg(test)]
use screenshots::{ImageCheck, base64_upper_bound, dimensions_are_a_rounded_variant, png_upper_bound, validate_image, working_set};
#[cfg(test)]
pub(crate) use screenshots::{SCREENSHOT_MAX_IMAGE_BYTES, SCREENSHOT_MAX_REQUESTS};
pub(crate) use screenshots::{Shots, admit_screenshot, drain_screenshots, issue_screenshots};

pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<Value>,
}

impl ControlRequest {
    pub fn new(method: impl Into<String>, params: Value) -> (Self, std::sync::mpsc::Receiver<Value>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { method: method.into(), params, reply: tx }, rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a visible root-surface fixture with the given viewport, scale and texture limit for
    /// screenshot tests.
    fn surface(viewport: egui::Vec2, pixels_per_point: f32, max_texture_side: usize) -> SurfaceGeometry {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, viewport);
        let mut raw = egui::RawInput { screen_rect: Some(rect), max_texture_side: Some(max_texture_side), ..Default::default() };
        raw.viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                inner_rect: Some(rect),
                native_pixels_per_point: Some(pixels_per_point),
                minimized: Some(false),
                occluded: Some(false),
                ..Default::default()
            },
        );
        SurfaceGeometry::from_raw(&raw, &ctx).expect("a visible surface")
    }

    /// Builds a capture geometry fixture for document id 7.
    fn geometry(viewport: egui::Vec2, pixels_per_point: f32, max_texture_side: usize) -> CaptureGeometry {
        CaptureGeometry { surface: surface(viewport, pixels_per_point, max_texture_side), viewport: egui::ViewportId::ROOT, document: 7 }
    }

    /// Creates a black test image of the given size.
    fn image(width: usize, height: usize) -> egui::ColorImage {
        egui::ColorImage::filled([width, height], egui::Color32::BLACK)
    }

    /// The reservation covers every live copy: the image, the RGBA copy the encoder consumes, the
    /// PNG buffer and the base64 reply string. A path reply needs no base64.
    #[test]
    fn screenshot_working_set_counts_image_copy_png_and_base64() {
        let raw = 1_000_000usize;
        let png = png_upper_bound(raw).expect("the PNG bound");
        let base64 = base64_upper_bound(png).expect("the base64 bound");
        assert_eq!(working_set(raw, false), Some(raw + raw + png));
        assert_eq!(working_set(raw, true), Some(raw + raw + png + base64));
        assert!(working_set(raw, true).expect("bounded") > raw * 4, "raw pixels alone are not the peak");
        assert_eq!(base64_upper_bound(0), Some(0));
        assert_eq!(base64_upper_bound(1), Some(4));
        assert_eq!(base64_upper_bound(4), Some(8), "rounded up, as the encoder pads");
        assert_eq!(base64_upper_bound(usize::MAX), None, "overflow is refused, not wrapped");
        assert_eq!(working_set(usize::MAX, true), None);
    }

    /// The 128 MiB per-image budget is a working-set budget, not a raw-pixel budget: a
    /// 3000×3000 inline reply no longer fits even though its raw pixels are only 36 MB, while a
    /// path reply for the same image (no base64) does.
    #[test]
    fn the_per_image_budget_reflects_the_encoding_peak() {
        let fits = geometry(egui::vec2(2000.0, 2000.0), 1.0, 16_384);
        assert!(fits.reserved_bytes(true).expect("bounded") <= SCREENSHOT_MAX_IMAGE_BYTES);

        let too_big_inline = geometry(egui::vec2(3000.0, 3000.0), 1.0, 16_384);
        assert!(too_big_inline.reserved_bytes(true).expect("bounded") > SCREENSHOT_MAX_IMAGE_BYTES);
        assert!(too_big_inline.reserved_bytes(false).expect("bounded") <= SCREENSHOT_MAX_IMAGE_BYTES);
    }

    /// A texture limit must not cap the expected physical size: capping one axis changes the
    /// expected aspect and makes the real callback look stale. Out-of-limit surfaces are rejected
    /// explicitly instead.
    #[test]
    fn expected_size_is_not_capped_by_the_texture_limit() {
        let wide = geometry(egui::vec2(10_000.0, 2_000.0), 1.0, 8_192);
        assert_eq!(wide.expected_size(), Some([10_000, 2_000]));
        let reason = wide.oversize_reason().expect("over the texture limit");
        assert!(reason.contains("texture limit"), "{reason}");

        let fits = geometry(egui::vec2(8_000.0, 2_000.0), 1.0, 8_192);
        assert_eq!(fits.expected_size(), Some([8_000, 2_000]));
        assert_eq!(fits.oversize_reason(), None);
    }

    /// Fractional DPI rounds the host surface and the expected size at different boundaries: a
    /// one-pixel difference on each axis is the same capture, a two-pixel difference is stale.
    #[test]
    fn a_callback_one_pixel_off_the_rounded_size_is_accepted() {
        let g = geometry(egui::vec2(100.4, 100.4), 1.5, 16_384);
        assert_eq!(g.expected_size(), Some([151, 151]));
        assert!(validate_image(&image(152, 150), [151, 151]).is_ok());
        assert!(matches!(validate_image(&image(153, 149), [151, 151]), Err(ImageCheck::Stale(_))));
        assert!(matches!(validate_image(&image(800, 600), [151, 151]), Err(ImageCheck::Stale(_))));
    }

    /// A real resize is stale even when one axis happens to be close: both axes must match within
    /// the one-pixel rounding allowance.
    #[test]
    fn a_two_pixel_change_is_a_resize_not_rounding() {
        assert!(dimensions_are_a_rounded_variant([1000, 1000], [1001, 999]));
        assert!(!dimensions_are_a_rounded_variant([1000, 1000], [1200, 800]));
        assert!(!dimensions_are_a_rounded_variant([1000, 1000], [1002, 999]));
        assert!(!dimensions_are_a_rounded_variant([1000, 1000], [1000, 998]));
    }
}

#[cfg(test)]
mod screenshot_tests {
    use super::save_screenshot;

    struct Cleanup(std::path::PathBuf);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A screenshot path must end in `.png` and is refused otherwise; a PNG is written, while a
    /// symlinked PNG target is not followed or overwritten.
    #[test]
    fn screenshot_path_must_be_png_and_does_not_follow_symlinks() {
        let dir = std::env::temp_dir().join(format!("wc-shot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _cleanup = Cleanup(dir.clone());
        let img = egui::ColorImage::filled([2, 2], egui::Color32::RED);

        let txt = dir.join("shot.txt");
        let r = save_screenshot(&img, txt.to_str(), 101);
        assert_eq!(r["ok"], false);
        assert!(!txt.exists());

        let png = dir.join("shot.png");
        let r = save_screenshot(&img, png.to_str(), 102);
        assert_eq!(r["ok"], true);
        assert!(std::fs::read(&png).unwrap().starts_with(b"\x89PNG"));

        #[cfg(unix)]
        {
            let victim = dir.join("victim.txt");
            std::fs::write(&victim, "keep").unwrap();
            let link = dir.join("link.png");
            std::os::unix::fs::symlink(&victim, &link).unwrap();
            let _ = save_screenshot(&img, link.to_str(), 103);
            assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        }

        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let roots = wordcraft_engine::file_access::AutomationRoots::new(None, Some(&other)).unwrap();
        let outside = dir.join("outside.png");
        let r = wordcraft_engine::file_access::confine(roots.as_ref(), || save_screenshot(&img, outside.to_str(), 104));
        assert_eq!(r["ok"], false);
        assert!(!outside.exists());
    }
}

#[cfg(test)]
mod doc_tests;
