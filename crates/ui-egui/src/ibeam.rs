//! The text pointer over the page on Windows (#1746, #1417, #1126). Windows' I-beam inverts what
//! lies under it, but over the GPU-drawn window it stays white, so it vanishes on white paper.
//! There the system pointer hides wherever it would be the I-beam and an I-beam is drawn in its
//! place: black on light paper, white on dark (as the caret, #299), edged in the other colour so
//! it shows on anything. Other systems' I-beams show on any page and are left alone.

use egui::{Color32, CursorIcon, Id, LayerId, Order, Pos2, Stroke};
use wordcraft_doc::props::Rgb;

/// Whether this system's I-beam is replaced by a drawn one.
pub(crate) const DRAWN: bool = cfg!(target_os = "windows");

/// Once the canvas has chosen its pointer: if it is the I-beam, hide it and draw one at `p` that
/// shows against `bg`. Whether it did.
pub(crate) fn replace(ctx: &egui::Context, p: Pos2, bg: Rgb) -> bool {
    if ctx.output(|o| o.cursor_icon) != CursorIcon::Text || !(p.x.is_finite() && p.y.is_finite()) {
        return false;
    }
    ctx.set_cursor_icon(CursorIcon::None);
    // Above the page and whatever floats over it, as the system pointer would be.
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("wordcraft_ibeam")));
    paint(&painter, p, crate::canvas::caret_contrast(bg));
    true
}

/// The I-beam centred on `p`, in `fill`, edged in the opposite colour. Its strokes are whole
/// physical pixels wide and lie on pixel edges, so at 125% or 150% scaling the fill isn't blurred
/// into the edge (a white I-beam on the dark page read grey).
fn paint(painter: &egui::Painter, p: Pos2, fill: Color32) {
    let ppp = painter.ctx().pixels_per_point();
    let edge = if fill == Color32::BLACK { Color32::WHITE } else { Color32::BLACK };
    // About 1 pt of fill inside about 1 pt of edge on each side, in whole pixels.
    let fill_px = ppp.round().max(1.0);
    let (x, y) = (snap(p.x, ppp, fill_px), snap(p.y, ppp, fill_px));
    let (top, bottom) = (y - 8.0, y + 8.0);
    let bar = |grow: f32| {
        [
            [egui::pos2(x, top), egui::pos2(x, bottom)],
            [egui::pos2(x - 3.0 - grow, top), egui::pos2(x + 3.0 + grow, top)],
            [egui::pos2(x - 3.0 - grow, bottom), egui::pos2(x + 3.0 + grow, bottom)],
        ]
    };
    for seg in bar(1.0) {
        painter.line_segment(seg, Stroke::new(3.0 * fill_px / ppp, edge));
    }
    for seg in bar(0.0) {
        painter.line_segment(seg, Stroke::new(fill_px / ppp, fill));
    }
}

/// `v` (points) moved so a stroke `width_px` physical pixels wide centred on it covers whole
/// pixels at `ppp` pixels per point.
fn snap(v: f32, ppp: f32, width_px: f32) -> f32 {
    let half = width_px / 2.0;
    ((v * ppp - half).round() + half) / ppp
}

#[cfg(test)]
mod tests {
    use crate::WordApp;
    use egui::{Color32, CursorIcon, Shape};

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

    /// Hovers the middle of the first page: the pointer then, and the drawn I-beam's colour if any.
    fn hover_page(h: &mut H) -> (CursorIcon, Option<Color32>) {
        let at = crate::canvas::page_to_screen(h.state_mut(), 0, 300.0, 400.0).unwrap();
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(2);
        let out = h.output();
        let x = super::snap(at.x, 1.0, 1.0);
        let stem = out.shapes.iter().find_map(|c| match &c.shape {
            Shape::LineSegment { points: [a, b], stroke } if a.x == x && b.x == x && stroke.width == 1.0 && (b.y - a.y - 16.0).abs() < 0.01 => {
                Some(stroke.color)
            }
            _ => None,
        });
        (out.platform_output.cursor_icon, stem)
    }

    /// #1746: where the system I-beam would vanish (Windows), it hides over the page and a black
    /// I-beam is drawn on white paper, a white one on the dark page.
    #[test]
    fn a_drawn_i_beam_shows_on_white_and_dark_paper() {
        let mut h = window(true, false);
        let sel = h.state().session.sel.clone();
        assert_eq!(hover_page(&mut h), (CursorIcon::None, Some(Color32::BLACK)), "white paper");
        assert_eq!(h.state().session.sel, sel, "hovering leaves the selection alone");
        let mut h = window(true, true);
        let sel = h.state().session.sel.clone();
        assert_eq!(hover_page(&mut h), (CursorIcon::None, Some(Color32::WHITE)), "dark page");
        assert_eq!(h.state().session.sel, sel, "hovering leaves the selection alone");
    }

    /// #1746 review: at 100%, 125%, 150% and 200% scaling the fill and the edge of a stroke centred
    /// on a snapped position both start and end on whole physical pixels.
    #[test]
    fn strokes_lie_on_whole_pixels() {
        for ppp in [1.0f32, 1.25, 1.5, 2.0] {
            let fill_px = ppp.round().max(1.0);
            for v in [0.0f32, 10.3, 99.5, 400.77] {
                let c = super::snap(v, ppp, fill_px) * ppp;
                for w in [fill_px, 3.0 * fill_px] {
                    let start = c - w / 2.0;
                    assert!((start - start.round()).abs() < 1e-3, "ppp {ppp}, v {v}, width {w}: starts at pixel {start}");
                }
                assert!((c / ppp - v).abs() <= 0.5 / ppp + 1e-3, "stays within half a pixel");
            }
        }
    }

    /// The strokes of the drawn I-beam's stem (16 pt tall, in `at`'s column) after hovering `at`,
    /// edge first, then fill.
    fn stem_strokes(h: &mut H, at: egui::Pos2) -> Vec<egui::Stroke> {
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(2);
        h.output()
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::LineSegment { points: [a, b], stroke } if a.x == b.x && (a.x - at.x).abs() < 2.0 && (b.y - a.y - 16.0).abs() < 0.01 => {
                    Some(*stroke)
                }
                _ => None,
            })
            .collect()
    }

    /// #1746 review: at 125% and 150% scaling the stem's edge and fill are whole physical pixels
    /// wide (a 3 pt and a 1 pt stroke would be 3.75 and 1.25 pixels at 125%, blurring the fill).
    #[test]
    fn the_drawn_i_beam_is_whole_pixels_wide_at_125_and_150_percent() {
        for ppp in [1.25f32, 1.5] {
            let mut h = window_at(true, true, ppp, 1440.0, "Hello WordCraft");
            let at = crate::canvas::page_to_screen(h.state_mut(), 0, 300.0, 400.0).unwrap();
            let strokes = stem_strokes(&mut h, at);
            assert_eq!(strokes.len(), 2, "ppp {ppp}: the stem's edge and fill");
            for s in strokes {
                let px = s.width * ppp;
                assert!((px - px.round()).abs() < 1e-3, "ppp {ppp}: a stroke {px} physical pixels wide");
            }
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
        // The fill is drawn last.
        let fill = |h: &mut H, at: egui::Pos2| stem_strokes(h, at).last().map(|s| s.color);
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
}
