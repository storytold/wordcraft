//! Ribbon sections that adapt to the window (#662), the way Word's ribbon does: each section keeps
//! its natural rows while the card has room for it, and the first section that doesn't fit — and
//! every one after it — becomes one row: an icon, the section's name and a chevron opening the whole
//! section. Word measured at 125 % keeps Clipboard and Font natural at 819 pt, adds Paragraph at
//! 960 pt, and keeps the label band below every section at both levels.
//!
//! Each frame [`begin`] plans every section's level from the widths the sections measured on
//! earlier frames ([`record`]) and the width there is; [`super::widgets::group_titled`] reads its
//! level with [`level`]. Sections are known by their position on the tab and their title, so two
//! custom groups with the same name keep their own widths. A width not measured yet takes no room,
//! and the plan settles once it is. Widths are measured again after the interface language or scale
//! changes. The ribbon still scrolls if even the one-row sections don't fit.

use std::collections::HashMap;

use egui::{Context, Id};

/// The group as designed: every control in its natural rows.
pub const FULL: u8 = 0;
/// One row holding a drop-down button with the group's name, opening the whole group.
pub const COMPACT: u8 = 1;

/// Room the card keeps past its last section. Word's measured breaks leave the last natural
/// section well clear of the right edge (at 1024 × 768 only Clipboard and Font keep their rows),
/// and without this a section fits by a hair and loses the break Word has there.
const EDGE_ROOM: f32 = 39.0;

/// One group of a tab in drawing order: its title and measured widths at each level.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupWidths {
    pub title: String,
    pub widths: [Option<f32>; 2],
}

/// Each group's level (by position) from the width `avail`: sections keep their natural rows while
/// the card has room for them, and the first one that doesn't fit marks the rest as one-row
/// drop-downs. Word measured at 125 % keeps just Clipboard and Font natural at 819 pt, adds
/// Paragraph at 960 pt and Styles further on; a section after the break never becomes natural
/// again, however narrow the drop-downs are.
pub fn plan(groups: &[GroupWidths], avail: f32) -> Vec<u8> {
    let mut levels = vec![COMPACT; groups.len()];
    let mut used = 0.0;
    for (g, level) in groups.iter().zip(levels.iter_mut()) {
        let full = g.widths[FULL as usize].unwrap_or(0.0);
        // Unmeasured or hostile widths take no room; the plan settles once they are measured.
        if !(full.is_finite() && full >= 0.0) {
            *level = FULL;
            continue;
        }
        if used + full > avail {
            break;
        }
        used += full;
        *level = FULL;
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

/// Stable storage id for the ribbon fit plan of the current frame.
fn frame_id() -> Id {
    Id::unique("ribbon_fit_frame")
}
/// Stable storage id for widths measured from earlier ribbon frames.
fn measured_id() -> Id {
    Id::unique("ribbon_fit_measured")
}
/// Stable storage id for whether this frame's ribbon fit plan is active.
fn active_id() -> Id {
    Id::unique("ribbon_fit_active")
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
                widths: [FULL, COMPACT].map(|l| m.widths.get(&(tab.to_string(), i, t.clone(), l)).copied()),
            })
            .collect();
        (titles, groups)
    });
    let levels = if groups.iter().all(|g| g.widths[0].is_some()) { plan(&groups, avail - EDGE_ROOM) } else { Vec::new() };
    ctx.data_mut(|d| {
        d.insert_temp(frame_id(), Frame { tab: tab.to_string(), titles, levels, drawn: Vec::new() });
        d.insert_temp(active_id(), true);
    });
}

/// How many groups of this ribbon row are drawn so far: the next group's place, which tells apart
/// groups that share a title (a custom group named like a built-in one).
pub fn position(ctx: &Context) -> usize {
    ctx.data(|d| d.get_temp::<Frame>(frame_id()).map(|f| f.drawn.len())).unwrap_or(0)
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

    fn g(title: &str, full: f32, compact: Option<f32>) -> GroupWidths {
        GroupWidths { title: title.into(), widths: [Some(full), compact] }
    }

    #[test]
    fn everything_full_when_it_fits() {
        let gs = [g("A", 100.0, Some(96.0)), g("B", 100.0, Some(96.0))];
        assert_eq!(plan(&gs, 200.0), vec![FULL, FULL]);
    }

    /// Word's measured rule: sections keep their natural rows until one doesn't fit, and that one
    /// and every section after it are one row, however much room the later ones would give back.
    #[test]
    fn the_first_section_that_does_not_fit_marks_itself_and_the_rest_one_row() {
        let gs = [g("A", 100.0, Some(60.0)), g("B", 100.0, Some(60.0)), g("C", 100.0, Some(60.0))];
        assert_eq!(plan(&gs, 300.0), vec![FULL, FULL, FULL]);
        assert_eq!(plan(&gs, 285.0), vec![FULL, FULL, COMPACT]);
        assert_eq!(plan(&gs, 150.0), vec![FULL, COMPACT, COMPACT]);
        assert_eq!(plan(&gs, 50.0), vec![COMPACT, COMPACT, COMPACT]);
    }

    /// Home's shape at the widths Word was measured at: Clipboard 150 and Font 420 fit 819 pt, the
    /// 380 pt Paragraph then breaks the row; at 960 pt Paragraph fits and Styles is the break.
    #[test]
    fn home_follows_words_measured_breaks() {
        let gs = [
            g("Clipboard", 150.0, Some(70.0)),
            g("Font", 420.0, Some(100.0)),
            g("Paragraph", 380.0, Some(95.0)),
            g("Styles", 520.0, Some(90.0)),
            g("Editing", 80.0, Some(65.0)),
            g("Voice", 60.0, Some(60.0)),
            g("Editor", 60.0, Some(60.0)),
        ];
        assert_eq!(plan(&gs, 819.0), vec![FULL, FULL, COMPACT, COMPACT, COMPACT, COMPACT, COMPACT]);
        assert_eq!(plan(&gs, 960.0), vec![FULL, FULL, FULL, COMPACT, COMPACT, COMPACT, COMPACT]);
    }

    #[test]
    fn unmeasured_widths_take_no_room() {
        let gs = [g("A", 0.0, None), g("B", 0.0, None)];
        assert_eq!(plan(&gs, 300.0), vec![FULL, FULL]);
        // Hostile numbers don't panic.
        let odd = [g("X", f32::NAN, None), g("Y", f32::INFINITY, Some(f32::NAN))];
        let _ = plan(&odd, 100.0);
        let _ = plan(&odd, f32::NAN);
    }
}

/// The real ribbon in the app: Home's natural rows and a one-row section's drop-down.
#[cfg(test)]
mod ui_tests {
    use super::*;
    use crate::WordApp;
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    fn harness(width: f32, height: f32, ppp: f32) -> egui_kittest::Harness<'static, WordApp> {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), crate::Services::default());
        app.run("ui.tab", json!({"tab": "Home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(width, height)).with_pixels_per_point(ppp).build_ui_state(
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

    /// Home's sections in drawing order, each at its planned level.
    fn home_levels(h: &egui_kittest::Harness<'static, WordApp>) -> Vec<(String, u8)> {
        ["Clipboard", "Font", "Paragraph", "Paragraph Tools", "Styles", "Editing", "Voice", "Editor"]
            .into_iter()
            .filter_map(|title| planned(&h.ctx, title).map(|level| (title.to_string(), level)))
            .collect()
    }

    /// Word's measured rule at 125 %: 1024 × 768 draws Clipboard and Font in their rows and every
    /// later section as one row; 1200 × 768 (960 pt) adds Paragraph; wider windows keep adding.
    #[test]
    fn home_sections_are_natural_then_one_row_at_1024_and_wider() {
        let h = harness(819.0, 614.0, 1.25);
        let levels = home_levels(&h);
        assert_eq!(planned(&h.ctx, "Clipboard"), Some(FULL), "Clipboard keeps its rows: {levels:?}");
        assert_eq!(planned(&h.ctx, "Font"), Some(FULL), "Font keeps its rows: {levels:?}");
        assert_eq!(planned(&h.ctx, "Paragraph"), Some(COMPACT), "Paragraph is one row at 819 pt: {levels:?}");
        assert_eq!(planned(&h.ctx, "Styles"), Some(COMPACT), "Styles is one row at 819 pt: {levels:?}");

        let h = harness(960.0, 614.4, 1.25);
        let levels = home_levels(&h);
        assert_eq!(planned(&h.ctx, "Paragraph"), Some(FULL), "Paragraph gains its rows at 1200 px: {levels:?}");
        assert_eq!(planned(&h.ctx, "Styles"), Some(COMPACT), "Styles is still one row at 1200 px: {levels:?}");

        // 1228 × 768 pt: no natural section follows a one-row one, wherever the break falls.
        let h = harness(1228.0, 768.0, 1.25);
        let levels = home_levels(&h);
        let first_compact = levels.iter().position(|(_, level)| *level == COMPACT);
        if let Some(i) = first_compact {
            assert!(levels[i..].iter().all(|(_, level)| *level == COMPACT), "natural after one row: {levels:?}");
        }
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

    /// #1285: at 1440 pt the Home tab keeps its wide sections' natural rows.
    #[test]
    fn home_at_1440_keeps_the_wide_sections() {
        let h = harness(1440.0, 900.0, 1.0);
        let levels = home_levels(&h);
        assert_eq!(planned(&h.ctx, "Clipboard"), Some(FULL), "{levels:?}");
        assert_eq!(planned(&h.ctx, "Font"), Some(FULL), "{levels:?}");
        assert_eq!(planned(&h.ctx, "Paragraph"), Some(FULL), "{levels:?}");
        assert_eq!(planned(&h.ctx, "Styles"), Some(FULL), "Styles is a gallery at 1440 pt: {levels:?}");
        let first_compact = levels.iter().position(|(_, level)| *level == COMPACT);
        if let Some(i) = first_compact {
            assert!(levels[i..].iter().all(|(_, level)| *level == COMPACT), "natural after one row: {levels:?}");
        }
    }

    /// #1285: a one-row section's drop-down stays open while its controls are used, and they run
    /// their commands: a button, then an item of a menu inside it.
    #[test]
    fn compact_group_stays_open_and_runs_its_commands() {
        let mut h = harness(420.0, 700.0, 1.0);
        assert_eq!(planned(&h.ctx, "Font"), Some(COMPACT), "Font is one row at 420 pt");
        h.state_mut().run("text.insert", json!({"text": "quiet words"})).unwrap();
        h.state_mut().run("select.all", json!({})).unwrap();
        h.get_by_role_and_label(Role::Button, "Font").click();
        h.run_steps(3);
        let sizes = |h: &egui_kittest::Harness<'static, WordApp>| {
            let s = &h.state().session;
            s.doc
                .para_paths(wordcraft_doc::StoryRef::Body)
                .iter()
                .filter_map(|path| s.doc.para(wordcraft_doc::StoryRef::Body, path))
                .flat_map(|p| p.run_ranges().map(|(_, c)| s.doc.styles.resolve_char(p.props.style.as_deref(), c).size))
                .collect::<Vec<_>>()
        };
        let before = sizes(&h);
        assert!(before.windows(2).any(|sizes| sizes[0] != sizes[1]), "the fixture has mixed sizes");
        click_shown(&h, "Increase Font Size");
        h.run_steps(3);
        let after = sizes(&h);
        assert_eq!(after.len(), before.len());
        assert!(after.windows(2).any(|sizes| sizes[0] != sizes[1]), "mixed sizes stay mixed");
        assert!(after.iter().zip(&before).all(|(after, before)| after > before), "Increase Font Size grew each run without flattening mixed sizes");
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
