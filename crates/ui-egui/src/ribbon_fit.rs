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
const EDGE_ROOM: f32 = 95.0;

/// From this client width Word's Home draws Font and Paragraph in their wide two-row form
/// (measured at 1900 px = 1520 pt at 125 %); below it they keep three rows.
pub const WIDE_MIN: f32 = 1520.0;

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
    /// Whether the card is wide enough for the two-row Home forms.
    wide: bool,
    /// The card width this frame's measured forms belong to.
    card_w: f32,
    /// The plan, by position, and the titles it was made for.
    titles: Vec<String>,
    levels: Vec<u8>,
    drawn: Vec<String>,
}

/// Widths measured so far: (tab, position, group, level, layout form) → width, and each
/// tab's group order. The form is the section's measured row count at the width it drew at
/// (plus the wide-Home flag): a section's natural layout changes at width breakpoints, so a
/// FULL width stored on one side of a breakpoint must not plan the other side.
#[derive(Clone, Default)]
struct Measured {
    /// The interface language and scale the widths were measured at.
    signature: String,
    widths: HashMap<(String, usize, String, u8, String), f32>,
    order: HashMap<String, Vec<String>>,
}

/// The layout form a section's measured width belongs to: its row count from the measured
/// table at this card width, and whether the wide Home form is on (`WIDE_MIN`, which
/// `begin` has not stored in the frame yet, so compare the width directly).
fn form_key(card_w: f32, tab: &str, section: &str) -> String {
    format!("{}:{}", rows_in(tab, section, card_w, 0), card_w >= WIDE_MIN)
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

// Whether tests have asked every group to draw its natural form even where the plan would fold
// it: the measured renderers are checked on their own, away from the card's width planning.
#[cfg(test)]
thread_local! {
    static FORCE_FULL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Draw every group in its natural form this thread (tests).
#[cfg(test)]
pub(crate) fn set_force_full(on: bool) {
    FORCE_FULL.with(|f| f.set(on));
}

#[cfg(test)]
fn forced_full() -> bool {
    FORCE_FULL.with(|f| f.get())
}

#[cfg(not(test))]
fn forced_full() -> bool {
    false
}

/// What the measured widths depend on besides the groups themselves.
fn signature(ctx: &Context) -> String {
    format!("{}:{}", crate::i18n::current().code(), ctx.pixels_per_point())
}

/// Start drawing `tab` in a row `avail` points wide: plan the groups' levels.
pub fn begin(ctx: &Context, tab: &str, avail: f32) {
    let sig = signature(ctx);
    // The card width this frame's forms belong to. Read once here: inside a group's row
    // the content rect is the group's remainder, not the card.
    let card_w = ctx.content_rect().width();
    let wide_now = card_w >= WIDE_MIN;
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
                widths: [FULL, COMPACT].map(|l| m.widths.get(&(tab.to_string(), i, t.clone(), l, form_key(card_w, tab, t))).copied()),
            })
            .collect();
        (titles, groups)
    });
    let levels = if forced_full() {
        vec![FULL; groups.len()]
    } else if groups.iter().all(|g| g.widths[0].is_some()) {
        plan(&groups, avail - EDGE_ROOM)
    } else {
        Vec::new()
    };
    ctx.data_mut(|d| {
        d.insert_temp(frame_id(), Frame { tab: tab.to_string(), wide: wide_now, card_w, titles, levels, drawn: Vec::new() });
        d.insert_temp(active_id(), true);
    });
}

/// How many groups of this ribbon row are drawn so far: the next group's place, which tells apart
/// groups that share a title (a custom group named like a built-in one).
pub fn position(ctx: &Context) -> usize {
    ctx.data(|d| d.get_temp::<Frame>(frame_id()).map(|f| f.drawn.len())).unwrap_or(0)
}

/// The measured row counts from `_parity/uia-word.json` (Office 16.0.20527, 125 %), in logical
/// points (px / 1.25): `(tab, section, narrow, &[(first width, rows)])`. Below the first width a
/// section is `narrow` (one row in Word's matrix unless the entry says otherwise); the last entry
/// holds above its width.
const SECTION_ROWS: &[(&str, &str, u8, &[(f32, u8)])] = &[
    ("Home", "Font", 1, &[(819.2, 3), (1520.0, 2)]),
    ("Home", "Paragraph", 1, &[(960.0, 3), (1520.0, 2)]),
    ("Home", "Editing", 1, &[(1520.0, 3)]),
    ("Insert", "Pages", 1, &[(1280.0, 3)]),
    ("Insert", "Media", 1, &[(880.0, 3), (1520.0, 1)]),
    ("Insert", "Links", 1, &[(1280.0, 3)]),
    ("Insert", "Header & Footer", 1, &[(928.8, 3), (1520.0, 1)]),
    ("Insert", "Text", 1, &[(1040.0, 3)]),
    ("Insert", "Symbols", 1, &[(1120.0, 2)]),
    ("Design", "Document Formatting", 1, &[(520.0, 3)]),
    ("Design", "Page Background", 3, &[(600.0, 1)]),
    ("Layout", "Page Setup", 1, &[(440.0, 3)]),
    ("Layout", "Arrange", 1, &[(819.2, 3)]),
    ("References", "Table of Contents", 1, &[(600.0, 2)]),
    ("References", "Footnotes", 1, &[(600.0, 3)]),
    ("References", "Citations & Bibliography", 1, &[(760.0, 3)]),
    ("References", "Captions", 1, &[(819.2, 3)]),
    ("References", "Index", 1, &[(819.2, 2)]),
    ("References", "Table of Authorities", 1, &[(819.2, 2)]),
    ("Mailings", "Create", 1, &[(880.0, 2), (1120.0, 1)]),
    ("Mailings", "Start Mail Merge", 1, &[(520.0, 3), (960.0, 1)]),
    ("Mailings", "Write & Insert Fields", 1, &[(680.0, 3)]),
    ("Mailings", "Preview Results", 1, &[(880.0, 3)]),
    ("Review", "Proofing", 1, &[(928.8, 3)]),
    ("Review", "Comments", 1, &[(1280.0, 3)]),
    // Review › Markup (1 → 3 rows from 1400 px) is not applicable: WordCraft has no independent
    // Markup section; Display for Review lives in Tracking.
    ("Review", "Tracking", 1, &[(1520.0, 2)]),
    ("View", "Views", 1, &[(880.0, 2)]),
    ("View", "Zoom", 1, &[(1120.0, 3)]),
    ("View", "Window", 1, &[(1040.0, 3)]),
    ("Help", "Help", 3, &[(440.0, 1)]),
];

/// The rows `section` takes at a client width of `width`; a section not in the measured table
/// keeps its natural form `natural`.
pub fn rows_in(tab: &str, section: &str, width: f32, natural: u8) -> u8 {
    let Some((_, _, narrow, breaks)) = SECTION_ROWS.iter().find(|(t, n, ..)| *t == tab && *n == section) else {
        return natural;
    };
    let mut rows = *narrow;
    for (min, count) in *breaks {
        if width >= *min {
            rows = *count;
        }
    }
    rows
}

/// The rows this section takes at the card's current client width.
pub fn rows_for(ctx: &Context, tab: &str, section: &str, natural: u8) -> u8 {
    rows_in(tab, section, ctx.content_rect().width(), natural)
}

/// Whether this frame's card is wide enough for the two-row Home forms (Font, Paragraph).
pub fn wide(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<Frame>(frame_id()).is_some_and(|f| f.wide))
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
        let card_w = d.get_temp::<Frame>(frame_id()).map(|f| f.card_w).unwrap_or(0.0);
        let m = d.get_temp_mut_or_default::<Measured>(measured_id());
        m.widths.insert((tab.clone(), i, title.to_string(), level, form_key(card_w, &tab, title)), width);
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

    /// Every measured 1↔3 transition (and Table of Contents' 1↔2), at the first width it was
    /// seen: `(tab, section, first width in pt, rows below it, rows at and above it)`.
    ///
    /// Review › Markup measures 1 → 3 rows from 1400 px but is not applicable: WordCraft has no
    /// independent Markup section; Display for Review lives in Tracking.
    const ROW_TRANSITIONS: &[(&str, &str, f32, u8, u8)] = &[
        ("Home", "Font", 819.2, 1, 3),
        ("Home", "Paragraph", 960.0, 1, 3),
        ("Home", "Editing", 1520.0, 1, 3),
        ("Insert", "Pages", 1280.0, 1, 3),
        ("Insert", "Media", 880.0, 1, 3),
        ("Insert", "Media", 1520.0, 3, 1),
        ("Insert", "Links", 1280.0, 1, 3),
        ("Insert", "Header & Footer", 928.8, 1, 3),
        ("Insert", "Header & Footer", 1520.0, 3, 1),
        ("Insert", "Text", 1040.0, 1, 3),
        ("Design", "Document Formatting", 520.0, 1, 3),
        ("Design", "Page Background", 600.0, 3, 1),
        ("Layout", "Page Setup", 440.0, 1, 3),
        ("Layout", "Arrange", 819.2, 1, 3),
        ("References", "Table of Contents", 600.0, 1, 2),
        ("References", "Footnotes", 600.0, 1, 3),
        ("References", "Citations & Bibliography", 760.0, 1, 3),
        ("References", "Captions", 819.2, 1, 3),
        ("Mailings", "Start Mail Merge", 520.0, 1, 3),
        ("Mailings", "Start Mail Merge", 960.0, 3, 1),
        ("Mailings", "Write & Insert Fields", 680.0, 1, 3),
        ("Mailings", "Preview Results", 880.0, 1, 3),
        ("Review", "Proofing", 928.8, 1, 3),
        ("Review", "Comments", 1280.0, 1, 3),
        ("View", "Zoom", 1120.0, 1, 3),
        ("View", "Window", 1040.0, 1, 3),
        ("Help", "Help", 440.0, 3, 1),
    ];

    /// Every measured row transition switches just at its first measured width: the row count
    /// below it is the entry's narrow form (or the previous transition), and the count at and
    /// above later widths follows the table. Table of Contents (2) and Footnotes (3) are included.
    #[test]
    fn measured_section_rows_switch_at_their_first_widths() {
        for (tab, section, narrow, breaks) in SECTION_ROWS {
            let mut previous = *narrow;
            for (first, rows) in *breaks {
                assert_eq!(rows_in(tab, section, first - 0.1, 3), previous, "{tab} › {section} below {first}");
                assert_eq!(rows_in(tab, section, *first, 3), *rows, "{tab} › {section} at {first}");
                previous = *rows;
            }
            // The measured matrix ends at 1900 px (1520 pt); the last count holds there.
            assert_eq!(rows_in(tab, section, 1520.0, 3), previous, "{tab} › {section} at 1520 pt");
        }
        // Every 1↔3 transition (and Table of Contents' 2-row form) in the measured matrix, at
        // its first width and just below it.
        for (tab, section, first, below, at) in ROW_TRANSITIONS {
            assert_eq!(rows_in(tab, section, first - 0.1, 3), *below, "{tab} › {section} below {first} pt");
            assert_eq!(rows_in(tab, section, *first, 3), *at, "{tab} › {section} at {first} pt");
        }
        assert_eq!(rows_in("Mailings", "Create", 1120.0, 1), 1, "Create one row from 1120 pt");
        assert_eq!(rows_in("Home", "Font", 2000.0, 3), 2, "the Home wide form");
    }

    /// Stored FULL widths belong to a layout form: the form key changes when a section's
    /// row count changes at its measured breakpoint, and when the card crosses the wide-Home
    /// threshold, so a width measured on one side never plans the other side.
    #[test]
    fn measured_widths_are_keyed_by_layout_form() {
        // The form key is the section's measured row count at a width plus the wide-Home
        // flag: no egui context needed, only the measured table and WIDE_MIN.
        fn form_at(width: f32, tab: &str, section: &str) -> String {
            format!("{}:{}", rows_in(tab, section, width, 0), width >= WIDE_MIN)
        }
        // Design › Document Formatting is 1 row below 520 pt and 3 rows from there.
        assert_ne!(
            form_at(400.0, "Design", "Document Formatting"),
            form_at(600.0, "Design", "Document Formatting"),
            "the 520-pt breakpoint is a new form"
        );
        // Home's wide form starts at 1520 pt of card.
        assert_ne!(form_at(1400.0, "Home", "Font"), form_at(1600.0, "Home", "Font"), "the wide threshold is a new form");
        // A section with no breakpoint keeps its form across widths.
        assert_eq!(form_at(900.0, "Home", "Clipboard"), form_at(1200.0, "Home", "Clipboard"), "Clipboard has no breakpoint");
    }

    /// The ribbon sections and the parity doc name the same reference: Word for Windows
    /// 16.0.20527 at 125 % DPI. A version/platform relabel in either place must land in both.
    #[test]
    fn measured_rows_name_the_same_reference_as_the_parity_doc() {
        let doc =
            std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/ui-parity.md")).expect("docs/ui-parity.md");
        for needle in ["Word for Windows 16.0.20527", "125%"] {
            assert!(doc.contains(needle), "docs/ui-parity.md no longer names the ribbon reference ({needle})");
        }
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
    use egui_kittest::kittest::{By, NodeT, Queryable};
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

    /// Word's wide Home form starts at 1520 pt of card: Font and Paragraph are two rows there,
    /// three rows at 1228 pt, which this also checks so the threshold cannot drift.
    #[test]
    fn home_wide_form_starts_at_1520_pt() {
        let h = harness(1228.0, 768.0, 1.25);
        assert!(!wide(&h.ctx), "1228 pt keeps the three-row Home");
        let h = harness(1520.0, 768.0, 1.25);
        assert!(wide(&h.ctx), "1520 pt draws the two-row Home");
        assert_eq!(planned(&h.ctx, "Font"), Some(FULL), "Font keeps its rows in the wide form");
        assert_eq!(planned(&h.ctx, "Paragraph"), Some(FULL), "Paragraph keeps its rows in the wide form");
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

    /// The two-row Insert › Symbols section (1400 px at 125 % = 1120 pt) offers the
    /// ready-made equation gallery, as the split button does outside the two-row form.
    #[test]
    fn two_row_symbols_keep_the_equation_gallery() {
        use egui_kittest::kittest::Queryable;
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), crate::Services::default());
        app.run("ui.tab", json!({"tab": "Insert"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 614.0)).with_pixels_per_point(1.25).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(8);
        // The split button's arrow opens Built-In; the plain insert would show no menu. The
        // arrow's popup is pointer-positioned, which the harness cannot aim at; pin the
        // affordance instead: the arrow is present with its menu name beside the insert.
        // (1400 pt window: the ribbon row itself clears the 1120-pt two-row breakpoint.)
        let arrows: Vec<f32> = h
            .query_all(By::new().predicate(|n| n.label().is_some_and(|l| l == "Equation options")))
            .filter(|n| n.rect().min.y >= 0.0)
            .map(|n| n.rect().width())
            .collect();
        assert!(!arrows.is_empty(), "the two-row Equation split button has its gallery arrow");
        assert!(arrows.iter().all(|w| *w <= 24.0), "the gallery arrow is a small marker, not a full button: {arrows:?}");
        let bodies: Vec<f32> = h
            .query_all(By::new().predicate(|n| n.label().is_some_and(|l| l == "Equation")))
            .filter(|n| n.rect().min.y >= 0.0)
            .map(|n| n.rect().width())
            .collect();
        assert!(!bodies.is_empty(), "the two-row Equation insert is present");
        assert!(
            arrows.iter().max_by(|a, b| a.total_cmp(b)).unwrap() < bodies.iter().max_by(|a, b| a.total_cmp(b)).unwrap(),
            "the insert body is wider than its arrow: bodies={bodies:?} arrows={arrows:?}"
        );
    }

    /// The two-row Review › Tracking section shows whether Track Changes is on.
    #[test]
    fn two_row_tracking_shows_the_track_changes_state() {
        use egui_kittest::kittest::Queryable;
        for on in [false, true] {
            let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), crate::Services::default());
            app.run("ui.tab", json!({"tab": "Review"})).unwrap();
            app.session.doc.settings.track_changes = on;
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1520.0, 614.0)).with_pixels_per_point(1.0).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                app,
            );
            h.run_steps(8);
            let node = h.get_by_label("Track Changes");
            let dump = format!("{node:?}");
            assert_eq!(dump.contains("toggled: True"), on, "Track Changes reports {on} in the two-row form: {dump}");
        }
    }

    /// The one-row View › Window section keeps every control reachable at 420 pt: opening
    /// the compact drop-down shows Full Screen inside the window, and it still toggles.
    #[test]
    fn compact_window_keeps_full_screen_reachable() {
        // 900 pt: the card is below the 1040-pt three-row breakpoint (Window is one row)
        // but wide enough to show the Window group without scrolling to it.
        let mut h = harness(900.0, 700.0, 1.0);
        h.state_mut().run("ui.tab", json!({"tab": "View"})).unwrap();
        h.run_steps(8);
        h.get_by_role_and_label(Role::Button, "Window").click();
        h.run_steps(3);
        assert!(shown(&h, "Full Screen"), "Full Screen is reachable in the compact Window group");
        let full = h.state().session.view.full_screen;
        h.state_mut().run("ui.tab", json!({"tab": "View"})).unwrap();
        let disabled = h.state().session.why_disabled("view.fullScreen").is_some();
        click_shown(&h, "Full Screen");
        h.run_steps(3);
        if disabled {
            assert_eq!(h.state().session.view.full_screen, full, "a disabled Full Screen stays off");
        } else {
            assert_ne!(h.state().session.view.full_screen, full, "Full Screen still toggles from the compact group");
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

    /// Draw every group naturally for the test's lifetime, even when the plan would fold it.
    struct ForcedFull;

    impl ForcedFull {
        fn new() -> Self {
            set_force_full(true);
            Self
        }
    }

    impl Drop for ForcedFull {
        fn drop(&mut self) {
            set_force_full(false);
        }
    }

    /// Every measured section draws the row count the plan records, on both sides of each
    /// transition: the count comes from the buttons actually laid out (distinct control tops),
    /// not from `rows_in`. Groups are forced to their natural form so a narrow card's fallback
    /// to one-row drop-downs does not hide the measured section.
    #[test]
    fn measured_section_rows_are_drawn() {
        let sections = [
            "Editing",
            "Pages",
            "Media",
            "Links",
            "Header & Footer",
            "Text",
            "Document Formatting",
            "Page Background",
            "Page Setup",
            "Arrange",
            "Table of Contents",
            "Footnotes",
            "Citations & Bibliography",
            "Captions",
            "Write & Insert Fields",
            "Preview Results",
            "Proofing",
            "Comments",
            "Zoom",
            "Window",
            "Help",
            "Symbols",
            "Index",
            "Table of Authorities",
            "Create",
            "Start Mail Merge",
            "Tracking",
            "Views",
        ];
        let mut cases: Vec<(String, String, f32, u8)> = Vec::new();
        for (tab, section, narrow, breaks) in SECTION_ROWS {
            if !sections.contains(section) {
                continue;
            }
            let mut previous = *narrow;
            for (first, rows) in *breaks {
                // A point either side, not the exact fraction: the card rounds its rect to a
                // 1/32 pt grid, so a width just under the first measured one may still round at it.
                cases.push(((*tab).to_string(), (*section).to_string(), first - 1.0, previous));
                cases.push(((*tab).to_string(), (*section).to_string(), first + 1.0, *rows));
                previous = *rows;
            }
        }
        cases.sort_by(|a, b| a.2.total_cmp(&b.2));
        let _forced = ForcedFull::new();
        let mut failures = Vec::new();
        let mut i = 0;
        while i < cases.len() {
            let width = cases[i].2;
            let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), crate::Services::default());
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(width, 700.0)).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                app,
            );
            h.run_steps(2);
            while i < cases.len() && cases[i].2 == width {
                let (tab, section, _, expected) = &cases[i];
                h.state_mut().run("ui.tab", json!({"tab": tab})).unwrap();
                h.run_steps(2);
                crate::widgets::GROUP_ROWS.with(|g| g.borrow_mut().clear());
                h.run_steps(1);
                let rows = crate::widgets::GROUP_ROWS.with(|g| g.take());
                let drawn = rows.iter().rev().find(|(title, _)| title.as_str() == section.as_str()).map(|(_, rows)| *rows);
                if drawn != Some(usize::from(*expected)) {
                    let card = h.ctx.content_rect().width();
                    failures.push(format!("{tab} › {section} at {width} pt (card {card}): drew {drawn:?}, expected {expected}"));
                }
                i += 1;
            }
        }
        assert!(failures.is_empty(), "measured rows not drawn: {failures:#?}");
    }
}
