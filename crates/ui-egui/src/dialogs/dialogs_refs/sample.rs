//! The sample card of the Table of Contents and Table of Figures dialogs.

use craft::ui::display::{Picture, PictureSize};
use craft::ui::list::{ThumbAspect, ThumbSize};

/// Tab leaders offered: (value, label).
pub(crate) const LEADERS: [(&str, &str); 4] = [("none", "(none)"), ("dot", "........"), ("hyphen", "--------"), ("underscore", "________")];

/// The character a leader value repeats, `None` for no leader.
pub(crate) fn leader_char(l: &str) -> Option<char> {
    match l {
        "dot" => Some('.'),
        "hyphen" => Some('-'),
        "underscore" => Some('_'),
        _ => None,
    }
}

/// Sample lines on a white card: (indent level, text, page or empty), with `leader` filling up to
/// right-aligned page numbers (or the page right after the text).
pub(crate) fn sample_card(label: &str, lines: Vec<(usize, String, String)>, right: bool, leader: Option<char>) -> Picture<'static> {
    let rows = u16::try_from(lines.len().max(1)).unwrap_or(u16::MAX);
    // The card is a sheet of paper: its fill and text are the page's, not the interface's; its edge
    // takes the interface ink.
    let paper = egui::Color32::WHITE;
    let ink = egui::Color32::BLACK;
    let faint = ink.gamma_multiply(0.6);
    Picture::painter(label.to_string(), ThumbAspect::new(16, rows.saturating_add(1)), move |cx, rect| {
        let p = &cx.painter;
        let row = rect.height() / (lines.len().max(1) as f32 + 0.6);
        let radius = (row * 0.15).round().clamp(0.0, 255.0) as u8;
        let hairline = 1.0 / p.ctx().pixels_per_point();
        p.rect_filled(rect, radius, paper);
        p.rect_stroke(rect, radius, egui::Stroke::new(hairline, cx.ink().gamma_multiply(0.4)), egui::StrokeKind::Inside);
        let font = egui::FontId::proportional(row * 0.65);
        let pad = row * 0.5;
        let gap = pad * 0.25;
        for (i, (level, text, page)) in lines.iter().enumerate() {
            let y = rect.top() + pad * 0.6 + row * i as f32;
            let x = rect.left() + pad + row * 0.7 * *level as f32;
            let g = p.layout_no_wrap(text.clone(), font.clone(), ink);
            let w = g.size().x;
            p.galley(egui::pos2(x, y), g, ink);
            if page.is_empty() {
                continue;
            }
            let pg = p.layout_no_wrap(page.clone(), font.clone(), ink);
            let pw = pg.size().x;
            let px = if right { rect.right() - pad - pw } else { x + w + pad * 0.5 };
            p.galley(egui::pos2(px, y), pg, ink);
            if right && let Some(c) = leader {
                let dots = p.layout_no_wrap(c.to_string(), font.clone(), faint);
                let step = dots.size().x.max(gap);
                let mut dx = x + w + pad * 0.5;
                while dx + step < px - gap {
                    p.galley(egui::pos2(dx, y), dots.clone(), faint);
                    dx += step;
                }
            }
        }
    })
    .extent(PictureSize::Sample(ThumbSize::Medium))
}
