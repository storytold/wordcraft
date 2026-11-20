use super::{Sense, Tokens, Ui, semibold, vec2};

pub(super) fn heading(ui: &mut Ui, s: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!(s)).font(semibold(12.5)));
}

/// A choice from `options` ((value, label)) in a drop-down, named by its visible label.
pub(super) fn choice<T: PartialEq + Clone>(ui: &mut Ui, id: &str, value: &mut T, options: &[(T, &str)], label: &egui::Response) {
    let shown = options.iter().find(|(v, _)| v == value).map(|(_, l)| tl!(l).to_string()).unwrap_or_default();
    let combo = craft_ui::ComboBox::from_id_salt(id).selected_text(shown).width(150.0).show_ui(ui, |ui| {
        for (v, l) in options {
            ui.selectable_value(value, v.clone(), tl!(l));
        }
    });
    combo.response.labelled_by(label.id);
}

pub(super) const LEADERS: [(&str, &str); 4] = [("none", "(none)"), ("dot", "........"), ("hyphen", "--------"), ("underscore", "________")];

pub(super) fn leader_char(l: &str) -> Option<char> {
    match l {
        "dot" => Some('.'),
        "hyphen" => Some('-'),
        "underscore" => Some('_'),
        _ => None,
    }
}

/// Sample lines on a white card: (indent level, text, page or empty), with `leader` filling up
/// to right-aligned page numbers (or the page right after the text).
pub(super) fn sample(ui: &mut Ui, lines: &[(usize, String, String)], right: bool, leader: Option<char>) {
    let t = Tokens::get(ui.ctx());
    let row = 17.0;
    let (rect, _) = ui.allocate_exact_size(vec2(260.0, row * lines.len().max(1) as f32 + 10.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 2.0, egui::Color32::WHITE);
    p.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
    let font = egui::FontId::proportional(11.0);
    let ink = egui::Color32::BLACK;
    for (i, (level, text, page)) in lines.iter().enumerate() {
        let y = rect.top() + 5.0 + row * i as f32;
        let x = rect.left() + 8.0 + 12.0 * *level as f32;
        let g = p.layout_no_wrap(text.clone(), font.clone(), ink);
        let w = g.size().x;
        p.galley(egui::pos2(x, y), g, ink);
        if page.is_empty() {
            continue;
        }
        let pg = p.layout_no_wrap(page.clone(), font.clone(), ink);
        let pw = pg.size().x;
        let px = if right { rect.right() - 8.0 - pw } else { x + w + 4.0 };
        p.galley(egui::pos2(px, y), pg, ink);
        if right && let Some(c) = leader {
            let dots = p.layout_no_wrap(c.to_string(), font.clone(), egui::Color32::from_gray(90));
            let step = dots.size().x.max(2.0);
            let mut dx = x + w + 4.0;
            while dx + step < px - 3.0 {
                p.galley(egui::pos2(dx, y), dots.clone(), egui::Color32::from_gray(90));
                dx += step;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Caption

/// Labels every document has.
pub(super) const BUILT_IN_LABELS: [&str; 3] = ["Equation", "Figure", "Table"];

pub(super) const NUMBER_FORMATS: [(&str, &str); 5] =
    [("ARABIC", "1, 2, 3, …"), ("alphabetic", "a, b, c, …"), ("ALPHABETIC", "A, B, C, …"), ("roman", "i, ii, iii, …"), ("ROMAN", "I, II, III, …")];

pub(super) const SEPARATORS: [(&str, &str); 5] =
    [("-", "- (hyphen)"), (".", ". (period)"), (":", ": (colon)"), ("\u{2014}", "\u{2014} (em dash)"), ("\u{2013}", "\u{2013} (en dash)")];
