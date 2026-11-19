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
    // The card is a sheet of paper: its fill and text are the page's, not the interface's; its edge
    // takes the interface ink. Its metrics are the old dialog's card: 260 pt wide at the sample step's
    // height, 11 pt text on 17 pt rows (closer when the rows need it), 8 pt margins and a 12 pt indent
    // a level.
    let paper = egui::Color32::WHITE;
    let ink = egui::Color32::BLACK;
    let faint = ink.gamma_multiply(0.6);
    Picture::painter(label.to_string(), ThumbAspect::new(65, 12), move |cx, rect| {
        let p = &cx.painter;
        let count = lines.len().max(1) as f32;
        let row = ((rect.height() - 6.0).max(1.0) / count).min(17.0);
        let hairline = 1.0 / p.ctx().pixels_per_point();
        p.rect_filled(rect, 2_u8, paper);
        p.rect_stroke(rect, 2_u8, egui::Stroke::new(hairline, cx.ink().gamma_multiply(0.4)), egui::StrokeKind::Inside);
        let font = egui::FontId::proportional((row * 0.8).min(11.0));
        let (pad, indent, gap) = (8.0, 12.0, 3.0);
        let top = rect.top() + (rect.height() - row * count) / 2.0;
        for (i, (level, text, page)) in lines.iter().enumerate() {
            let x = rect.left() + pad + indent * *level as f32;
            let g = p.layout_no_wrap(text.clone(), font.clone(), ink);
            let (w, y) = (g.size().x, top + row * i as f32 + (row - g.size().y) / 2.0);
            p.galley(egui::pos2(x, y), g, ink);
            if page.is_empty() {
                continue;
            }
            let pg = p.layout_no_wrap(page.clone(), font.clone(), ink);
            let pw = pg.size().x;
            let px = if right { rect.right() - pad - pw } else { x + w + 4.0 };
            p.galley(egui::pos2(px, y), pg, ink);
            if right && let Some(c) = leader {
                let dots = p.layout_no_wrap(c.to_string(), font.clone(), faint);
                let step = dots.size().x.max(2.0);
                let mut dx = x + w + 4.0;
                while dx + step < px - gap {
                    p.galley(egui::pos2(dx, y), dots.clone(), faint);
                    dx += step;
                }
            }
        }
    })
    .extent(PictureSize::Sample(ThumbSize::Medium))
}
