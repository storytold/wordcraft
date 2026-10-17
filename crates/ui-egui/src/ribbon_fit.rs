//! Ribbon groups that shrink to fit the window (#662), the way Word's ribbon does: when a tab is
//! wider than the window, groups give up their small buttons' labels and the Styles gallery shows
//! fewer tiles (compact), then fold into a single drop-down button that opens the whole group
//! (collapsed), starting from the right.
//!
//! Each frame [`begin`] plans every group's level from the widths the groups measured on earlier
//! frames ([`record`]) and the width there is; [`super::widgets::group_titled`] reads its level
//! with [`level`]. Groups are known by their position on the tab and their title, so two custom
//! groups with the same name keep their own widths. A group drawn as designed also estimates its
//! compact width from what its controls would give up ([`add_saving`]), so the planner can choose
//! a compact group over a collapsed one before it has ever been drawn compact. A width not measured
//! yet is estimated, and the plan settles once it is. Widths are measured again after the interface
//! language or scale changes. The ribbon still scrolls if even fully collapsed groups don't fit.

use std::collections::HashMap;

use egui::{Context, Id};

/// The group as designed.
pub const FULL: u8 = 0;
/// Small buttons without their labels (tooltips keep the names), a shorter Styles gallery.
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

/// Each group's level (by position) so that the row fits `avail`: groups shrink from the right,
/// first all to compact, then to collapsed, stopping as soon as the row fits. Then whatever room is
/// left is given back, from the left.
pub fn plan(groups: &[GroupWidths], avail: f32) -> Vec<u8> {
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
            let next = total + width(g, wider) - width(g, *level);
            if !next.is_finite() || next > avail {
                break;
            }
            total = next;
            *level = wider;
        }
    }
    levels
}

/// What this frame's ribbon is drawing.
#[derive(Clone, Default)]
struct Frame {
    tab: String,
    /// The plan, by position, and the titles it was made for.
    titles: Vec<String>,
    levels: Vec<u8>,
    drawn: Vec<String>,
}

/// Widths measured so far: (tab, position, group, level) → width, and each tab's group order.
#[derive(Clone, Default)]
struct Measured {
    /// The interface language and scale the widths were measured at.
    signature: String,
    widths: HashMap<(String, usize, String, u8), f32>,
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
fn saving_id() -> Id {
    Id::new("ribbon_fit_saving")
}
fn active_id() -> Id {
    Id::new("ribbon_fit_active")
}

/// What the measured widths depend on besides the groups themselves.
fn signature(ctx: &Context) -> String {
    format!("{}:{}", crate::i18n::current().code(), ctx.pixels_per_point())
}

/// Start drawing `tab` in a row `avail` points wide: plan the groups' levels.
pub fn begin(ctx: &Context, tab: &str, avail: f32) {
    let sig = signature(ctx);
    let (titles, groups) = ctx.data_mut(|d| {
        let m = d.get_temp_mut_or_default::<Measured>(measured_id());
        if m.signature != sig {
            // Another language or scale: labels have new widths, so measure again.
            *m = Measured { signature: sig, ..Default::default() };
        }
        let titles = m.order.get(tab).cloned().unwrap_or_default();
        let groups: Vec<GroupWidths> = titles
            .iter()
            .enumerate()
            .map(|(i, t)| GroupWidths {
                title: t.clone(),
                widths: [FULL, COMPACT, COLLAPSED].map(|l| m.widths.get(&(tab.to_string(), i, t.clone(), l)).copied()),
            })
            .collect();
        (titles, groups)
    });
    let levels = if groups.iter().all(|g| g.widths[0].is_some()) { plan(&groups, avail) } else { Vec::new() };
    ctx.data_mut(|d| {
        d.insert_temp(frame_id(), Frame { tab: tab.to_string(), titles, levels, drawn: Vec::new() });
        d.insert_temp(active_id(), true);
    });
}

/// The level the next group, `title`, draws at this frame.
pub fn level(ctx: &Context, title: &str) -> u8 {
    ctx.data(|d| {
        let f = d.get_temp::<Frame>(frame_id())?;
        let i = f.drawn.len();
        // Planned for another group here (one was added or removed): draw it as designed.
        if f.titles.get(i).map(String::as_str) != Some(title) {
            return None;
        }
        f.levels.get(i).copied()
    })
    .unwrap_or(FULL)
}

/// Group `title` drew at `level`, `width` points wide.
pub fn record(ctx: &Context, title: &str, level: u8, width: f32) {
    ctx.data_mut(|d| {
        let saving = d.remove_temp::<f32>(saving_id()).unwrap_or(0.0);
        // Only groups on the ribbon row between `begin` and `end` are planned.
        if !d.get_temp::<bool>(active_id()).unwrap_or(false) {
            return;
        }
        let f = d.get_temp_mut_or_default::<Frame>(frame_id());
        let i = f.drawn.len();
        f.drawn.push(title.to_string());
        let tab = f.tab.clone();
        if !(width.is_finite() && width > 0.0) {
            return;
        }
        let m = d.get_temp_mut_or_default::<Measured>(measured_id());
        m.widths.insert((tab.clone(), i, title.to_string(), level), width);
        // Drawn as designed: until it is drawn compact, its compact width is what its labels and
        // gallery tiles would give back.
        if level == FULL && saving.is_finite() && saving > 0.0 {
            m.widths.entry((tab, i, title.to_string(), COMPACT)).or_insert((width - saving).max(COLLAPSED_W));
        }
    });
}

/// The row is drawn: remember its group order for the next frame's plan.
pub fn end(ctx: &Context) {
    let changed = ctx.data_mut(|d| {
        d.insert_temp(active_id(), false);
        let drawn = std::mem::take(&mut d.get_temp_mut_or_default::<Frame>(frame_id()).drawn);
        let tab = d.get_temp_mut_or_default::<Frame>(frame_id()).tab.clone();
        let m = d.get_temp_mut_or_default::<Measured>(measured_id());
        let changed = m.order.get(&tab) != Some(&drawn);
        if changed {
            m.order.insert(tab, drawn);
        }
        changed
    });
    if changed {
        // A new order (another tab, a group appeared): plan again with it next frame.
        ctx.request_repaint();
    }
}

/// Whether the group being drawn is compact (small buttons drop their labels).
pub fn compact(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(compact_id())).unwrap_or(false)
}

/// Mark the group being drawn compact or not, and start counting what it could give up.
pub fn set_compact(ctx: &Context, on: bool) {
    ctx.data_mut(|d| {
        d.insert_temp(compact_id(), on);
        d.remove_temp::<f32>(saving_id());
    });
}

/// The group's controls are drawn: what follows isn't compact (what they could give up is kept).
pub fn end_compact(ctx: &Context) {
    ctx.data_mut(|d| d.insert_temp(compact_id(), false));
}

/// A control of a group drawn as designed would be `w` points narrower in a compact group.
pub fn add_saving(ctx: &Context, w: f32) {
    if w.is_finite() && w > 0.0 && !compact(ctx) {
        ctx.data_mut(|d| *d.get_temp_mut_or_default::<f32>(saving_id()) += w);
    }
}

/// The level this frame planned for the first group called `title` (tests).
#[cfg(test)]
pub(crate) fn planned(ctx: &Context, title: &str) -> Option<u8> {
    ctx.data(|d| {
        let f = d.get_temp::<Frame>(frame_id())?;
        let i = f.titles.iter().position(|t| t == title)?;
        Some(f.levels.get(i).copied().unwrap_or(FULL))
    })
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
        assert_eq!(plan(&gs, 200.0), vec![FULL, FULL]);
    }

    #[test]
    fn rightmost_groups_compact_first_then_collapse() {
        let gs = [g("A", 100.0, Some(80.0), Some(60.0)), g("B", 100.0, Some(80.0), Some(60.0)), g("C", 100.0, Some(80.0), Some(60.0))];
        // 300 → C compact (280) fits 285.
        assert_eq!(plan(&gs, 285.0), vec![FULL, FULL, COMPACT]);
        // All compact is 240; then C collapses (220) to fit 230.
        assert_eq!(plan(&gs, 230.0), vec![COMPACT, COMPACT, COLLAPSED]);
        // Nothing fits: everything collapsed (the row scrolls).
        assert!(plan(&gs, 10.0).iter().all(|l| *l == COLLAPSED));
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
        assert_eq!(plan(&gs, 800.0), vec![FULL, COLLAPSED, FULL, FULL]);
    }

    /// #1285: the Home tab's shape at 1440 pt. The Styles gallery trimmed to fewer tiles (its
    /// compact width) fits, so it stays a gallery and no group folds.
    #[test]
    fn trimmed_gallery_keeps_styles_open() {
        let gs = [
            g("Clipboard", 130.0, Some(70.0), Some(60.0)),
            g("Font", 330.0, Some(290.0), Some(60.0)),
            g("Paragraph", 300.0, Some(280.0), Some(60.0)),
            g("Styles", 520.0, Some(290.0), Some(60.0)),
            g("Editing", 80.0, Some(40.0), Some(60.0)),
            g("Voice", 60.0, Some(60.0), Some(60.0)),
        ];
        let p = plan(&gs, 1250.0);
        assert_eq!(p, vec![FULL, FULL, FULL, COMPACT, FULL, FULL]);
        // Styles as designed (compact = full, the old behaviour) would have to fold.
        let mut wide = gs.clone();
        if let Some(s) = wide.get_mut(3) {
            s.widths[1] = Some(520.0);
        }
        assert_eq!(plan(&wide, 1250.0).get(3), Some(&COLLAPSED));
    }

    #[test]
    fn same_titles_keep_their_own_levels() {
        let gs = [g("New Group", 300.0, Some(300.0), Some(60.0)), g("New Group", 100.0, Some(100.0), Some(60.0))];
        assert_eq!(plan(&gs, 200.0), vec![COLLAPSED, FULL]);
    }

    #[test]
    fn unmeasured_widths_are_estimated() {
        let gs = [g("A", 200.0, None, None), g("B", 200.0, None, None)];
        // Compact unknown counts as full, so the planner collapses B (estimated 64) and A, which
        // then fits as designed, is given back.
        assert_eq!(plan(&gs, 300.0), vec![FULL, COLLAPSED]);
        // Hostile numbers don't panic.
        let odd = [g("X", f32::NAN, None, None), g("Y", f32::INFINITY, Some(f32::NAN), None)];
        let _ = plan(&odd, 100.0);
        let _ = plan(&odd, f32::NAN);
    }
}

/// The real ribbon in the app: the Styles gallery at 1440 pt and a collapsed group's drop-down.
#[cfg(test)]
mod ui_tests {
    use super::*;
    use crate::WordApp;
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    fn harness(width: f32) -> egui_kittest::Harness<'static, WordApp> {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), crate::Services::default());
        app.run("ui.tab", json!({"tab": "Home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(width, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        // Measure, plan, measure the planned levels, plan again.
        h.run_steps(8);
        h
    }

    /// Whether a control called `label` is on screen (a popup laid out off screen doesn't count).
    fn shown(h: &egui_kittest::Harness<'static, WordApp>, label: &str) -> bool {
        h.query_all_by_label(label).any(|n| n.rect().min.y >= 0.0)
    }

    fn click_shown(h: &egui_kittest::Harness<'static, WordApp>, label: &str) {
        let n = h.get_all_by_label(label).find(|n| n.rect().min.y >= 0.0);
        assert!(n.is_some(), "{label} is shown");
        if let Some(n) = n {
            n.click();
        }
    }

    /// #1285: at 1440 pt the Home tab keeps its Styles gallery (trimmed) instead of folding it.
    #[test]
    fn home_at_1440_keeps_the_styles_gallery() {
        let h = harness(1440.0);
        let styles = planned(&h.ctx, "Styles");
        assert!(styles.is_some(), "the Home tab was planned");
        assert_ne!(styles, Some(COLLAPSED), "Styles folded at 1440 pt");
    }

    /// #1285: a collapsed group's drop-down stays open while its controls are used, and they run
    /// their commands: a button, then an item of a menu inside it.
    #[test]
    fn collapsed_group_stays_open_and_runs_its_commands() {
        let mut h = harness(760.0);
        assert_eq!(planned(&h.ctx, "Font"), Some(COLLAPSED), "Font folds at 760 pt");
        h.state_mut().run("text.insert", json!({"text": "quiet words"})).unwrap();
        h.state_mut().run("select.all", json!({})).unwrap();
        h.get_by_role_and_label(Role::Button, "Font").click();
        h.run_steps(3);
        let size = |h: &mut egui_kittest::Harness<'static, WordApp>| {
            h.state_mut().session.run("format.state", &json!({})).ok().and_then(|v| v.get("size").and_then(serde_json::Value::as_f64))
        };
        let before = size(&mut h);
        click_shown(&h, "Increase Font Size");
        h.run_steps(3);
        assert_ne!(size(&mut h), before, "Increase Font Size ran");
        assert!(shown(&h, "Change Case"), "the drop-down stays open after a click inside");
        click_shown(&h, "Change Case");
        h.run_steps(3);
        // The item's accessible name carries its shortcut.
        h.get_by_label_contains("UPPERCASE").click();
        h.run_steps(3);
        let text = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
        assert!(!text.chars().any(|c| c.is_lowercase()), "UPPERCASE ran");
        assert!(shown(&h, "Increase Font Size"), "a menu inside counts as inside");
        h.get_by_role_and_label(Role::Button, "Font").click();
        h.run_steps(3);
        assert!(!shown(&h, "Increase Font Size"), "the button closes it again");
    }
}
