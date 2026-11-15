//! The text pointer over the page on Windows (#1746, #1417, #1126). Windows' I-beam inverts what
//! lies under it, but over the GPU-drawn window it stays white, so it vanishes on white paper.
//! Supply the contrasting I-beam as a native RGBA cursor: a controlled comparison implicated hiding the OS pointer and painting
//! its replacement in centered Magnifier panning jumps. The mechanism is unknown.
//! Other platforms keep their system pointer. Caret/accessibility geometry is unchanged.

use std::collections::VecDeque;

use egui::{Color32, CursorIcon, CustomCursorImage, Id, Pos2};
use wordcraft_doc::props::Rgb;

/// Whether this system's I-beam needs the contrasting image.
pub(crate) const DRAWN: bool = cfg!(target_os = "windows");

/// Custom images persist in egui's output, so every window must explicitly relinquish ours.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn begin_frame(ctx: &egui::Context) {
    ctx.set_cursor_image(None);
}

/// A later overlay or a pointer exit must take precedence over the canvas's choice.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn finish_frame(ctx: &egui::Context) {
    if ctx.output(|o| o.cursor_icon) != CursorIcon::Text || ctx.input(|i| i.pointer.latest_pos().is_none()) {
        ctx.set_cursor_image(None);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    fill: u16,
    half_height: u16,
    half_bar: u16,
    light: bool,
}

impl Key {
    /// Quantize display scale and contrast so equivalent bitmaps share a cache entry.
    fn new(ppp: f32, color: Color32) -> Option<Self> {
        if !ppp.is_finite() || ppp <= 0.0 {
            return None;
        }
        // Bounded native pixel dimensions; document zoom does not enlarge the mouse pointer.
        let ppp = ppp.clamp(0.5, 8.0);
        Some(Self {
            fill: ppp.round().max(1.0) as u16,
            half_height: (8.0 * ppp).round() as u16,
            half_bar: (3.0 * ppp).round() as u16,
            light: color == Color32::WHITE,
        })
    }
}

#[derive(Clone, Default)]
struct Cache(VecDeque<(Key, CustomCursorImage)>);

/// Once the canvas has chosen Text, supply its native image without changing cursor selection.
/// Keeping Text also gives the backend a stock cursor choice if RGBA validation fails.
/// Native OS creation failure is opaque to this API; it may retain the previous cursor.
pub(crate) fn replace(ctx: &egui::Context, p: Pos2, bg: Rgb) -> bool {
    if ctx.output(|o| o.cursor_icon) != CursorIcon::Text || !(p.x.is_finite() && p.y.is_finite()) {
        ctx.set_cursor_image(None);
        return false;
    }
    let Some(key) = Key::new(ctx.pixels_per_point(), crate::canvas::caret_contrast(bg)) else {
        ctx.set_cursor_image(None);
        return false;
    };
    let image = ctx.data_mut(|data| {
        let cache = data.get_temp_mut_or_default::<Cache>(Id::unique("wordcraft_native_ibeam_cache"));
        if let Some((_, image)) = cache.0.iter().find(|(cached, _)| *cached == key) {
            return image.clone();
        }
        let image = raster(key);
        if cache.0.len() >= 8 {
            cache.0.pop_front();
        }
        cache.0.push_back((key, image.clone()));
        image
    });
    ctx.set_cursor_image(Some(image));
    true
}

/// Original WordCraft geometry, rasterized with solid whole-pixel strokes and a central hotspot.
/// Key is private and bounded by Key::new; the largest image is smaller than 256 x 256 pixels.
fn raster(key: Key) -> CustomCursorImage {
    let fill = i32::from(key.fill);
    let edge = 3 * fill;
    let half_height = i32::from(key.half_height);
    let half_bar = i32::from(key.half_bar);
    let hx = half_bar + edge + 1;
    let hy = half_height + edge + 1;
    let width = (2 * hx + 1) as u16;
    let height = (2 * hy + 1) as u16;
    let mut rgba = vec![0; usize::from(width) * usize::from(height) * 4];
    let foreground = if key.light { 255 } else { 0 };
    let outline = 255 - foreground;
    for (index, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let x = (index % usize::from(width)) as i32 - hx;
        let y = (index / usize::from(width)) as i32 - hy;
        let band = |v: i32, center: i32, thickness: i32| v >= center - thickness / 2 && v < center - thickness / 2 + thickness;
        let inside = |thickness: i32, bar: i32| {
            (band(x, 0, thickness) && y >= -half_height && y <= half_height)
                || ((band(y, -half_height, thickness) || band(y, half_height, thickness)) && x >= -bar && x <= bar)
        };
        let value = if inside(fill, half_bar) {
            foreground
        } else if inside(edge, half_bar + fill) {
            outline
        } else {
            continue;
        };
        pixel.copy_from_slice(&[value, value, value, 255]);
    }
    CustomCursorImage { rgba: rgba.into(), size: [width, height], hotspot: [hx as u16, hy as u16] }
}

#[cfg(test)]
mod tests {
    use crate::WordApp;
    use egui::{Color32, CursorIcon, CustomCursorImage};

    type H = egui_kittest::Harness<'static, WordApp>;

    fn window(drawn: bool, dark: bool) -> H {
        window_at(drawn, dark, 1.0, 1440.0, "Hello WordCraft")
    }

    /// A window at `ppp` physical pixels per point showing `text`.
    fn window_at(drawn: bool, dark: bool, ppp: f32, width: f32, text: &str) -> H {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(text)), Default::default());
        a.canvas.drawn_ibeam = drawn;
        a.session.view.dark_mode = dark;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(width, 900.0)).with_pixels_per_point(ppp).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(3);
        h
    }

    fn image_at(h: &mut H, at: egui::Pos2) -> Option<CustomCursorImage> {
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(2);
        h.output().platform_output.cursor_image.clone()
    }

    fn pixel(image: &CustomCursorImage, x: u16, y: u16) -> [u8; 4] {
        let i = (usize::from(y) * usize::from(image.size[0]) + usize::from(x)) * 4;
        image.rgba[i..i + 4].try_into().unwrap()
    }

    fn center(image: &CustomCursorImage) -> Color32 {
        let [r, g, b, a] = pixel(image, image.hotspot[0], image.hotspot[1]);
        Color32::from_rgba_unmultiplied(r, g, b, a)
    }

    /// Hovers the middle of the first page: native pointer choice and bitmap fill, if any.
    fn hover_page(h: &mut H) -> (CursorIcon, Option<Color32>) {
        let at = crate::canvas::page_to_screen(h.state_mut(), 0, 300.0, 400.0).unwrap();
        let image = image_at(h, at);
        (h.output().platform_output.cursor_icon, image.as_ref().map(center))
    }

    /// #1746's contrast and selection invariants, now using a visible native bitmap.
    #[test]
    fn a_drawn_i_beam_shows_on_white_and_dark_paper() {
        let mut h = window(true, false);
        let sel = h.state().session.sel.clone();
        assert_eq!(hover_page(&mut h), (CursorIcon::Text, Some(Color32::BLACK)), "white paper");
        assert_eq!(h.state().session.sel, sel, "hovering leaves the selection alone");
        let mut h = window(true, true);
        let sel = h.state().session.sel.clone();
        assert_eq!(hover_page(&mut h), (CursorIcon::Text, Some(Color32::WHITE)), "dark page");
        assert_eq!(h.state().session.sel, sel, "hovering leaves the selection alone");
    }

    /// #1746 review: at 100%, 125%, 150% and 200% scaling the fill and the edge of a stroke centred
    /// on a snapped position both start and end on whole physical pixels.
    #[test]
    fn strokes_lie_on_whole_pixels() {
        for ppp in [0.5f32, 1.0, 1.25, 1.5, 2.0, 3.0, 8.0] {
            for color in [Color32::BLACK, Color32::WHITE] {
                let key = super::Key::new(ppp, color).unwrap();
                let image = super::raster(key);
                assert_eq!(image.rgba.len(), usize::from(image.size[0]) * usize::from(image.size[1]) * 4);
                assert_eq!(image.hotspot, [image.size[0] / 2, image.size[1] / 2]);
                assert_eq!(center(&image), color);
                assert!(image.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 0 || p[3] == 255));
                let opposite = if color == Color32::BLACK { [255; 4] } else { [0, 0, 0, 255] };
                assert!(image.rgba.as_chunks::<4>().0.contains(&opposite));
                for x in 0..image.size[0] {
                    assert_eq!(pixel(&image, x, 0)[3], 0);
                    assert_eq!(pixel(&image, x, image.size[1] - 1)[3], 0);
                }
                let fill = (0..image.size[0]).filter(|&x| pixel(&image, x, image.hotspot[1]) == color.to_array()).count();
                assert_eq!(fill, usize::from(key.fill));
            }
        }
    }

    /// #1746 review: at 125% and 150% scaling the stem's edge and fill are whole physical pixels
    /// wide (a 3 pt and a 1 pt stroke would be 3.75 and 1.25 pixels at 125%, blurring the fill).
    #[test]
    fn the_drawn_i_beam_is_whole_pixels_wide_at_125_and_150_percent() {
        for ppp in [1.25f32, 1.5] {
            let mut h = window_at(true, true, ppp, 1440.0, "Hello WordCraft");
            let at = crate::canvas::page_to_screen(h.state_mut(), 0, 300.0, 400.0).unwrap();
            let image = image_at(&mut h, at).unwrap();
            assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::Text);
            let fill = (0..image.size[0]).filter(|&x| pixel(&image, x, image.hotspot[1]) == [255; 4]).count();
            assert_eq!(fill, ppp.round().max(1.0) as usize);
        }
    }

    /// #1746 review: off the shown page (the gap between pages, the seam where Hide White Space
    /// cuts the margins) the I-beam contrasts with the canvas, not the paper. With the dark page
    /// the two differ: white over the page, black over the light canvas.
    #[test]
    fn the_drawn_i_beam_takes_the_canvas_colour_off_the_page() {
        let text = vec!["A line of text to fill the pages."; 120].join("\n");
        let mut h = window_at(true, true, 1.0, 800.0, &text);
        h.state_mut().session.view.zoom = 0.5;
        h.run_steps(3);
        let canvas = crate::theme::Tokens::get(&h.ctx).canvas;
        let off = crate::canvas::caret_contrast(wordcraft_doc::props::Rgb(canvas.r(), canvas.g(), canvas.b()));
        assert_eq!(off, Color32::BLACK, "the light canvas");
        let fill = |h: &mut H, at: egui::Pos2| image_at(h, at).as_ref().map(center);
        let rects = h.state_mut().canvas.page_rects.clone();
        assert!(rects.len() > 1, "two pages");
        let x = rects[0].center().x;
        let gap = egui::pos2(x, (rects[0].max.y + rects[1].min.y) / 2.0);
        assert!(rects[0].max.y < gap.y && gap.y < rects[1].min.y, "a gap between the pages: {rects:?}");
        assert_eq!(fill(&mut h, egui::pos2(x, rects[0].center().y)), Some(Color32::WHITE), "on the dark page");
        assert_eq!(fill(&mut h, gap), Some(off), "between the pages");
        // On the seam where Hide White Space cuts page 1's bottom margin and page 2's top.
        assert!(h.state_mut().run("view.hideWhiteSpace", serde_json::json!({"value": true})).is_ok());
        h.run_steps(3);
        let scale = h.state_mut().canvas.scale;
        let body_bottom = h.state_mut().session.layout().pages[0].body.bottom();
        let band_end = crate::canvas::page_to_screen(h.state_mut(), 0, 300.0, body_bottom).unwrap().y + crate::canvas::WHITE_SPACE_PAD * scale;
        assert_eq!(fill(&mut h, egui::pos2(x, band_end - 20.0)), Some(Color32::WHITE), "on page 1's text area");
        assert_eq!(fill(&mut h, egui::pos2(x, band_end + crate::canvas::SEAM / 2.0)), Some(off), "on the seam over the cut margins");
    }

    /// Elsewhere the system's own I-beam stays.
    #[test]
    fn the_system_i_beam_stays_where_it_shows() {
        let mut h = window(false, false);
        assert_eq!(hover_page(&mut h), (CursorIcon::Text, None));
    }

    /// Leaving the page clears the image and returning reuses its pixels.
    #[test]
    fn native_image_is_cached_and_cleared_on_ribbon_exit_and_backstage() {
        let mut h = window(true, false);
        let at = crate::canvas::page_to_screen(h.state_mut(), 0, 300.0, 400.0).unwrap();
        let first = image_at(&mut h, at).unwrap();
        let second = image_at(&mut h, at + egui::vec2(1.0, 0.0)).unwrap();
        assert!(std::sync::Arc::ptr_eq(&first.rgba, &second.rgba));
        assert!(image_at(&mut h, egui::pos2(75.0, 55.0)).is_none(), "ribbon");
        assert_ne!(h.output().platform_output.cursor_icon, CursorIcon::None);
        let returned = image_at(&mut h, at).unwrap();
        assert!(std::sync::Arc::ptr_eq(&first.rgba, &returned.rgba));
        h.event(egui::Event::PointerGone);
        h.run_steps(2);
        assert!(h.output().platform_output.cursor_image.is_none());
        assert!(image_at(&mut h, at).is_some());
        h.state_mut().ui.backstage = true;
        h.run_steps(2);
        assert!(h.output().platform_output.cursor_image.is_none());
    }

    /// A later resize request wins, and invalid coordinates leave the stock cursor selected.
    #[test]
    fn another_cursor_choice_wins_and_invalid_geometry_is_bounded() {
        let ctx = egui::Context::default();
        ctx.set_cursor_icon(CursorIcon::Text);
        assert!(super::replace(&ctx, egui::pos2(10.0, 10.0), wordcraft_doc::props::Rgb(255, 255, 255)));
        ctx.set_cursor_icon(CursorIcon::ResizeHorizontal);
        super::finish_frame(&ctx);
        assert!(ctx.output(|o| o.cursor_image.is_none()));
        assert_eq!(ctx.output(|o| o.cursor_icon), CursorIcon::ResizeHorizontal);
        ctx.set_cursor_icon(CursorIcon::Text);
        assert!(!super::replace(&ctx, egui::pos2(f32::NAN, 0.0), wordcraft_doc::props::Rgb(0, 0, 0)));
        assert!(ctx.output(|o| o.cursor_image.is_none()));
        assert_eq!(ctx.output(|o| o.cursor_icon), CursorIcon::Text, "invalid geometry retains the native fallback");
        for ppp in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            assert!(super::Key::new(ppp, Color32::BLACK).is_none());
        }
        let largest = super::raster(super::Key::new(f32::MAX, Color32::BLACK).unwrap());
        assert!(largest.size.into_iter().all(|s| s < 256));
    }

    /// A running window must replace stale DPI images, then reuse its baseline bitmap.
    /// This tests egui output, not native Windows DPI messages or Magnifier appearance.
    #[test]
    fn live_dpi_changes_replace_the_bitmap_and_restore_the_cached_baseline() {
        let mut h = window(true, false);
        let at = crate::canvas::page_to_screen(h.state_mut(), 0, 300.0, 400.0).unwrap();
        let first = image_at(&mut h, at).unwrap();
        assert_eq!((first.size, first.hotspot), ([15, 25], [7, 12]));
        for (ppp, size, hotspot) in [(1.25, [17, 29], [8, 14]), (1.5, [25, 39], [12, 19]), (1.0, [15, 25], [7, 12])] {
            // Report display DPI, rather than changing the interface zoom that WordApp owns.
            h.input_mut().viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(ppp);
            h.run_steps(3);
            assert_eq!(h.ctx.pixels_per_point(), ppp);
            let at = crate::canvas::page_to_screen(h.state_mut(), 0, 300.0, 400.0).unwrap();
            let image = image_at(&mut h, at).unwrap();
            assert_eq!((image.size, image.hotspot), (size, hotspot));
            assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::Text);
            assert_eq!(std::sync::Arc::ptr_eq(&first.rgba, &image.rgba), ppp == 1.0);
        }
    }

    /// Evicting scale variants in one window leaves another window independent.
    #[test]
    fn native_bitmap_caches_are_bounded_and_local_to_their_context() {
        let first = egui::Context::default();
        let second = egui::Context::default();
        let bg = wordcraft_doc::props::Rgb(255, 255, 255);
        let at = egui::pos2(20.0, 20.0);
        for i in 0..40 {
            let mut input = egui::RawInput::default();
            input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(0.5 + i as f32 / 5.0);
            first.begin_pass(input);
            first.set_cursor_icon(CursorIcon::Text);
            assert!(super::replace(&first, at, bg));
            first.end_pass().textures_delta.clear();
        }
        let len = |ctx: &egui::Context| {
            ctx.data(|data| data.get_temp::<super::Cache>(egui::Id::unique("wordcraft_native_ibeam_cache")).map_or(0, |cache| cache.0.len()))
        };
        assert_eq!(len(&first), 8);
        assert_eq!(len(&second), 0, "another window/context has no stale bitmap");
        second.set_cursor_icon(CursorIcon::Text);
        assert!(super::replace(&second, at, bg));
        super::begin_frame(&first);
        assert!(first.output(|o| o.cursor_image.is_none()));
        assert!(second.output(|o| o.cursor_image.is_some()), "clearing one window does not clear another");
    }
}
