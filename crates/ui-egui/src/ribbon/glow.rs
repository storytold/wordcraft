//! WordCraft's low accent light, using the shared ribbon's recorded group geometry.
use craft::ui::runtime::inspect::Recorded;
use egui::{Color32, Rect, pos2};
pub(super) fn paint(ui: &egui::Ui, recorded: &Recorded) {
    let accent = crate::theme::Tokens::get(ui.ctx()).accent;
    for (index, element) in recorded.elements.iter().enumerate() {
        if element.component != "Row" || !element.facts.as_ref().is_some_and(|f| f.role == "Group" && f.name.is_some()) {
            continue;
        }
        let Some(area) = recorded.rect_in(ui.ctx(), index, ui.layer_id()).filter(|r| r.is_finite()) else { continue };
        let lit = ui.ctx().animate_bool_responsive(element.id.with("wordcraft.group_glow"), ui.rect_contains_pointer(area));
        if lit > 0.0 {
            ui.painter().add(group_glow(area, accent, lit));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_group_glow_is_subtle_and_fades_before_every_edge() {
        for width in [24.0, 120.0, 480.0] {
            for t in [crate::theme::Tokens::light(), crate::theme::Tokens::dark()] {
                let area = Rect::from_min_size(pos2(30.0, 20.0), egui::vec2(width, 90.0));
                let egui::Shape::Mesh(mesh) = group_glow(area, t.accent, 1.0) else { panic!("expected a glow mesh") };
                let bounds = mesh.calc_bounds();
                assert!(area.expand2(egui::vec2(0.0, 2.0)).contains_rect(bounds), "the soft tail stays in the ribbon's bottom padding");
                assert!(bounds.min.x > area.min.x && bounds.max.x < area.max.x);
                assert_eq!(bounds.max.y, area.max.y + 2.0);
                let peak = mesh.vertices.iter().max_by_key(|v| v.color.a()).unwrap();
                assert!(peak.pos.y >= area.max.y - 3.0 && peak.pos.y < area.max.y, "the bright core sits below the label");
                assert_eq!(peak.color.a(), 51, "a visible core adds depth without a solid blue edge");
                assert!(mesh.vertices.iter().any(|v| v.color.a() > 0), "the highlight is visible");
                for v in &mesh.vertices {
                    assert!(v.color.a() <= 51, "no saturated blue stripe, even in the dark theme");
                    if v.pos.x == bounds.min.x || v.pos.x == bounds.max.x || v.pos.y == bounds.min.y || v.pos.y == bounds.max.y {
                        assert_eq!(v.color.a(), 0, "every edge fades to transparent");
                    }
                }
                let egui::Shape::Mesh(faded) = group_glow(area, t.accent, 0.5) else { panic!("expected a glow mesh") };
                assert!(faded.vertices.iter().zip(&mesh.vertices).all(|(a, b)| a.pos == b.pos && a.color.a() <= b.color.a()));
            }
        }
    }
}
/// A low accent glow inset from the group's edges, transparent on every side so neither the
/// ribbon's bottom border nor a neighbouring group cuts it off at its brightest point.
fn group_glow(area: Rect, accent: Color32, strength: f32) -> egui::Shape {
    let color = |alpha: f32| Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), (alpha * strength * 255.0) as u8);
    let inset = (area.width() * 0.1).min(8.0);
    let r = Rect::from_min_max(pos2(area.min.x + inset, area.max.y - 8.0), pos2(area.max.x - inset, area.max.y + 2.0));
    const COLS: u32 = 17;
    const ROWS: [f32; 7] = [0.0, 0.025, 0.10, 0.20, 0.15, 0.05, 0.0];
    let mut mesh = egui::epaint::Mesh::default();
    for (row, alpha) in ROWS.iter().enumerate() {
        let y = egui::lerp(r.min.y..=r.max.y, row as f32 / (ROWS.len() - 1) as f32);
        for col in 0..COLS {
            let u = col as f32 / (COLS - 1) as f32;
            let bell = (1.0 - (2.0 * u - 1.0).powi(2)).powi(2);
            mesh.colored_vertex(pos2(egui::lerp(r.min.x..=r.max.x, u), y), color(alpha * bell));
        }
    }
    for row in 0..(ROWS.len() - 1) as u32 {
        for col in 0..COLS - 1 {
            let i = row * COLS + col;
            mesh.add_triangle(i, i + 1, i + COLS + 1);
            mesh.add_triangle(i, i + COLS + 1, i + COLS);
        }
    }
    egui::Shape::mesh(mesh)
}
