//! Ribbon groups that shrink to fit the window (#662), the way Word's ribbon does: when a tab is
//! wider than the window, groups give up their small buttons' labels (compact), then fold into a
//! single drop-down button that opens the whole group (collapsed), starting from the right.
//!
//! Each frame [`begin`] plans every group's level from the widths the groups measured on earlier
//! frames ([`record`]) and the width there is; [`super::widgets::group_titled`] reads its level
//! with [`level`]. A width not measured yet is estimated, and the plan settles once it is. The
//! ribbon still scrolls if even fully collapsed groups don't fit.

use std::collections::HashMap;

use egui::{Context, Id};

/// The group as designed.
pub const FULL: u8 = 0;
/// Small buttons without their labels (tooltips keep the names).
pub const COMPACT: u8 = 1;
/// One drop-down button holding the whole group.
pub const COLLAPSED: u8 = 2;

/// Estimated width of a collapsed group (its button and divider) before it is measured.
const COLLAPSED_W: f32 = 64.0;

/// One group of a tab in drawing order: its title and measured widths at each level.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupWidths {
    pub title: String,
    pub widths: [Option<f32>; 3],
}

/// Each group's level so that the row fits `avail`: groups shrink from the right, first all to
/// compact, then to collapsed, stopping as soon as the row fits.
pub fn plan(groups: &[GroupWidths], avail: f32) -> HashMap<String, u8> {
    let full = |g: &GroupWidths| g.widths[0].unwrap_or(0.0);
    let width = |g: &GroupWidths, level: u8| -> f32 {
        match level {
            FULL => full(g),
            COMPACT => g.widths[1].unwrap_or(full(g)),
            _ => g.widths[2].unwrap_or(COLLAPSED_W.min(full(g))),
        }
    };
    let mut levels = vec![FULL; groups.len()];
    let mut total: f32 = groups.iter().map(full).sum();
    for target in [COMPACT, COLLAPSED] {
        for i in (0..groups.len()).rev() {
            if total <= avail {
                break;
            }
            let (Some(g), Some(level)) = (groups.get(i), levels.get_mut(i)) else { continue };
            total += width(g, target) - width(g, *level);
            *level = target;
        }
    }
    // Shrinking a wide group (Styles) can make room the groups shrunk before it no longer need,
    // and a group of large buttons gains nothing collapsed: give back whatever still fits.
    for (g, level) in groups.iter().zip(levels.iter_mut()) {
        while *level > FULL {
            let wider = *level - 1;
            let grow = width(g, wider) - width(g, *level);
            let next = total + grow;
            if next.is_nan() || next > avail {
                break;
            }
            total = next;
            *level = wider;
        }
    }
    groups.iter().zip(levels).filter(|(_, l)| *l != FULL).map(|(g, l)| (g.title.clone(), l)).collect()
}

/// What this frame's ribbon is drawing.
#[derive(Clone, Default)]
struct Frame {
    tab: String,
    levels: HashMap<String, u8>,
    drawn: Vec<String>,
}

/// Widths measured so far: (tab, group, level) → width, and each tab's group order.
#[derive(Clone, Default)]
struct Measured {
    widths: HashMap<(String, String, u8), f32>,
    order: HashMap<String, Vec<String>>,
}

fn frame_id() -> Id {
    Id::new("ribbon_fit_frame")
}
fn measured_id() -> Id {
    Id::new("ribbon_fit_measured")
}
fn compact_id() -> Id {
    Id::new("ribbon_fit_compact")
}

/// Start drawing `tab` in a row `avail` points wide: plan the groups' levels.
pub fn begin(ctx: &Context, tab: &str, avail: f32) {
    let m: Measured = ctx.data(|d| d.get_temp(measured_id())).unwrap_or_default();
    let groups: Vec<GroupWidths> = m
        .order
        .get(tab)
        .map(|titles| {
            titles
                .iter()
                .map(|t| GroupWidths {
                    title: t.clone(),
                    widths: [FULL, COMPACT, COLLAPSED].map(|l| m.widths.get(&(tab.to_string(), t.clone(), l)).copied()),
                })
                .collect()
        })
        .unwrap_or_default();
    let levels = if groups.iter().all(|g| g.widths[0].is_some()) { plan(&groups, avail) } else { HashMap::new() };
    ctx.data_mut(|d| d.insert_temp(frame_id(), Frame { tab: tab.to_string(), levels, drawn: Vec::new() }));
}

/// The level group `title` draws at this frame.
pub fn level(ctx: &Context, title: &str) -> u8 {
    ctx.data(|d| d.get_temp::<Frame>(frame_id())).and_then(|f| f.levels.get(title).copied()).unwrap_or(FULL)
}

/// Group `title` drew at `level`, `width` points wide.
pub fn record(ctx: &Context, title: &str, level: u8, width: f32) {
    let Some(mut f) = ctx.data(|d| d.get_temp::<Frame>(frame_id())) else { return };
    let mut m: Measured = ctx.data(|d| d.get_temp(measured_id())).unwrap_or_default();
    if width.is_finite() && width > 0.0 {
        m.widths.insert((f.tab.clone(), title.to_string(), level), width);
    }
    f.drawn.push(title.to_string());
    ctx.data_mut(|d| {
        d.insert_temp(frame_id(), f);
        d.insert_temp(measured_id(), m);
    });
}

/// The row is drawn: remember its group order for the next frame's plan.
pub fn end(ctx: &Context) {
    let Some(f) = ctx.data(|d| d.get_temp::<Frame>(frame_id())) else { return };
    let mut m: Measured = ctx.data(|d| d.get_temp(measured_id())).unwrap_or_default();
    let changed = m.order.get(&f.tab) != Some(&f.drawn);
    m.order.insert(f.tab.clone(), f.drawn);
    ctx.data_mut(|d| d.insert_temp(measured_id(), m));
    if changed {
        // A new order (another tab, a group appeared): plan again with it next frame.
        ctx.request_repaint();
    }
}

/// Whether the group being drawn is compact (small buttons drop their labels).
pub fn compact(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(compact_id())).unwrap_or(false)
}

/// Mark the group being drawn compact or not.
pub fn set_compact(ctx: &Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(compact_id(), on));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(title: &str, full: f32, compact: Option<f32>, collapsed: Option<f32>) -> GroupWidths {
        GroupWidths { title: title.into(), widths: [Some(full), compact, collapsed] }
    }

    #[test]
    fn everything_full_when_it_fits() {
        let gs = [g("A", 100.0, Some(80.0), Some(60.0)), g("B", 100.0, Some(80.0), Some(60.0))];
        assert!(plan(&gs, 200.0).is_empty());
    }

    #[test]
    fn rightmost_groups_compact_first_then_collapse() {
        let gs = [g("A", 100.0, Some(80.0), Some(60.0)), g("B", 100.0, Some(80.0), Some(60.0)), g("C", 100.0, Some(80.0), Some(60.0))];
        // 300 → C compact (280) fits 285.
        assert_eq!(plan(&gs, 285.0), HashMap::from([("C".to_string(), COMPACT)]));
        // All compact is 240; then C collapses (220) to fit 230.
        let p = plan(&gs, 230.0);
        assert_eq!((p.get("A"), p.get("B"), p.get("C")), (Some(&COMPACT), Some(&COMPACT), Some(&COLLAPSED)));
        // Nothing fits: everything collapsed (the row scrolls).
        assert!(plan(&gs, 10.0).values().all(|l| *l == COLLAPSED));
    }

    #[test]
    fn groups_that_still_fit_are_given_back() {
        // A wide gallery group in the middle and big-button groups that don't shrink.
        let gs = [
            g("Font", 300.0, Some(250.0), Some(60.0)),
            g("Styles", 500.0, Some(500.0), Some(60.0)),
            g("Voice", 60.0, Some(60.0), Some(60.0)),
            g("Editor", 60.0, Some(60.0), Some(60.0)),
        ];
        // 920 → 800: Styles collapses; Font, Voice and Editor stay as designed.
        assert_eq!(plan(&gs, 800.0), HashMap::from([("Styles".to_string(), COLLAPSED)]));
    }

    #[test]
    fn unmeasured_widths_are_estimated() {
        let gs = [g("A", 200.0, None, None), g("B", 200.0, None, None)];
        // Compact unknown counts as full, so the planner collapses B (estimated 64) and A, which
        // then fits as designed, is given back.
        assert_eq!(plan(&gs, 300.0), HashMap::from([("B".to_string(), COLLAPSED)]));
        // Hostile numbers don't panic.
        let odd = [g("X", f32::NAN, None, None), g("Y", f32::INFINITY, Some(f32::NAN), None)];
        let _ = plan(&odd, 100.0);
        let _ = plan(&odd, f32::NAN);
    }
}
