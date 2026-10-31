//! Task panes: Navigation (headings, pages, search results), Clipboard, Styles, Style Inspector,
//! Accessibility and Alt Text (see `a11y_pane`), Format Shape (see `format_shape`), Restrict Editing
//! ([`crate::dialogs_review`]), Comments, and the SmartArt Text Pane. `craft_ui::panes` draws them
//! around the document, with their title bars, menus and resize edges; the session's view flags say
//! which are open.

use std::collections::BTreeMap;

use craft_ui::panes::{Edge, PaneContent, PaneHost, PaneLabels, PaneRequest, PaneState, Place, SidePolicy};
use egui::{Stroke, Ui, vec2};
use serde_json::{Value, json};
use wordcraft_doc::numbering::{Counters, LevelSuffix};
use wordcraft_doc::section::NumFormat;
use wordcraft_doc::{Document, Path, Pos, StoryRef};
use wordcraft_engine::StylesPaneDock;

use crate::WordApp;
use crate::theme::{Tokens, regular, semibold};

mod comments;
mod host;
mod inspector;
mod nav;
mod side;
mod state;

#[cfg(test)]
pub(crate) use comments::comment_draft;
use comments::comments;
pub(crate) use comments::{CommentDrafts, comment_editor, commit_comment, commit_reply, flush_comment_drafts, has_comment_drafts, reply_editor};
use host::Content;
pub use host::show;
pub(crate) use host::{dock_command, overlay_open};
pub(crate) use inspector::describe_props;
use inspector::inspector;
use nav::nav;
#[cfg(test)]
use nav::{nav_heading, nav_headings, visible_headings};
use side::{clipboard, styles};
pub(crate) use state::float_rect;
#[cfg(test)]
use state::pane_state;
pub use state::{SavedDock, SavedPane, open};
pub(crate) use state::{lenient_order, lenient_places};

/// The task panes, in their default order: each edge's first pane is outermost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pane {
    /// SmartArt Design › Text Pane, while a SmartArt graphic WordCraft can edit is selected.
    SmartArtText,
    Navigation,
    Clipboard,
    Styles,
    StyleInspector,
    Accessibility,
    AltText,
    FormatShape,
    RestrictEditing,
    Comments,
}

impl Pane {
    fn title(self) -> &'static str {
        match self {
            Self::SmartArtText => "Text Pane",
            Self::Navigation => "Navigation",
            Self::Clipboard => "Clipboard",
            Self::Styles => "Styles",
            Self::StyleInspector => "Style Inspector",
            Self::Accessibility => "Accessibility",
            Self::AltText => "Alt Text",
            Self::FormatShape => "Format Shape",
            Self::RestrictEditing => "Restrict Editing",
            Self::Comments => "Comments",
        }
    }

    fn default_state(self) -> PaneState {
        let (edge, width) = match self {
            Self::SmartArtText => (Edge::Start, 250.0),
            Self::Navigation => (Edge::Start, 260.0),
            Self::Clipboard => (Edge::Start, 250.0),
            Self::Styles => (Edge::End, 250.0),
            Self::StyleInspector => (Edge::End, 240.0),
            Self::Accessibility => (Edge::End, 280.0),
            Self::AltText => (Edge::End, 260.0),
            Self::FormatShape => (Edge::End, 280.0),
            Self::RestrictEditing => (Edge::End, 270.0),
            Self::Comments => (Edge::End, 290.0),
        };
        PaneState::docked(edge, width)
    }

    /// The pane's key in `ui.json`'s `panePlaces`.
    pub fn key(self) -> &'static str {
        match self {
            Self::SmartArtText => "smartArtText",
            Self::Navigation => "navigation",
            Self::Clipboard => "clipboard",
            Self::Styles => "styles",
            Self::StyleInspector => "styleInspector",
            Self::Accessibility => "accessibility",
            Self::AltText => "altText",
            Self::FormatShape => "formatShape",
            Self::RestrictEditing => "restrictEditing",
            Self::Comments => "comments",
        }
    }

    const ALL: [Self; 10] = [
        Self::SmartArtText,
        Self::Navigation,
        Self::Clipboard,
        Self::Styles,
        Self::StyleInspector,
        Self::Accessibility,
        Self::AltText,
        Self::FormatShape,
        Self::RestrictEditing,
        Self::Comments,
    ];

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|pane| pane.key() == key)
    }

    /// The command that closes the pane; Restrict Editing has none and closes in [`close`].
    fn close_command(self) -> Option<&'static str> {
        match self {
            Self::SmartArtText => Some("smartArt.textPane"),
            Self::Navigation => Some("view.navigationPane"),
            Self::Clipboard => Some("edit.clipboardPane"),
            Self::Styles => Some("view.stylesPane"),
            Self::StyleInspector => Some("styles.inspector"),
            Self::Accessibility => Some("view.accessibilityPane"),
            Self::AltText => Some("view.altTextPane"),
            Self::FormatShape => Some("view.formatShapePane"),
            Self::Comments => Some("view.commentsPane"),
            Self::RestrictEditing => None,
        }
    }
}

impl PaneContent<Pane> for Content<'_> {
    fn title(&self, pane: &Pane) -> String {
        tl!(pane.title()).into()
    }

    fn body(&mut self, ui: &mut Ui, pane: &Pane) {
        let app = &mut *self.app;
        match pane {
            Pane::SmartArtText => crate::dialogs_smart_art::text_pane(app, ui),
            Pane::Navigation => nav(app, ui),
            Pane::Clipboard => clipboard(app, ui),
            Pane::Styles => styles(app, ui),
            Pane::StyleInspector => inspector(app, ui),
            Pane::Accessibility => crate::a11y_pane::accessibility(app, ui),
            Pane::AltText => crate::a11y_pane::alt_text(app, ui),
            Pane::FormatShape => {
                egui::ScrollArea::vertical().show(ui, |ui| crate::format_shape::pane(app, ui));
            }
            Pane::RestrictEditing => crate::dialogs_review::restrict_pane(app, ui),
            Pane::Comments => comments(app, ui),
        }
    }

    fn default_state(&self, pane: &Pane) -> PaneState {
        pane.default_state()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod navigation_tests {
    use super::*;

    /// Numbered headings are listed with the number they show, and numbered body paragraphs in
    /// the same list still count (#3418).
    #[test]
    fn headings_show_their_list_numbers() {
        use wordcraft_doc::props::NumRef;
        use wordcraft_doc::{CharProps, ListKind, Paragraph, para_block};
        let mut doc = Document::new();
        let num = doc.numbering.add_list(ListKind::Legal);
        let para = |text: &str, style: &str, level: Option<u8>| {
            let mut p = Paragraph::with_text(text, CharProps::default()).styled(style);
            p.props.numbering = level.map(|level| NumRef { num, level });
            para_block(p)
        };
        doc.body = vec![
            para("Intro", "Heading1", Some(0)),
            para("Scope", "Heading2", Some(1)),
            para("A numbered body paragraph", "Normal", Some(1)),
            para("Terms", "Heading2", Some(1)),
            para("Method", "Heading1", Some(0)),
            para("Appendix", "Heading1", None),
        ];
        let texts: Vec<String> = nav_headings(&doc).into_iter().map(|(_, _, t)| t).collect();
        assert_eq!(texts, ["1. Intro", "1.1. Scope", "1.3. Terms", "2. Method", "Appendix"]);
    }

    #[test]
    fn long_headings_wrap_without_stretching_navigation() {
        // A heading that runs well past the width, and a single word wider than the pane.
        let headings = ["An extremely long heading with words that must stay inside the navigation pane ".repeat(20), "x".repeat(400)];
        // Draw one heading in a pane of `width`: the pane rect, the heading row's rect, the label's
        // line count and whether it was elided.
        let render = |width: f32, level: u8, text: &str| -> (egui::Rect, egui::Rect, usize, bool) {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 600.0));
            let mut panel_rect = egui::Rect::NOTHING;
            let mut heading_rect = egui::Rect::NOTHING;
            let output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                let result = egui::Panel::left("navigation_test")
                    .default_size(width)
                    .resizable(true)
                    .show(ui, |ui| egui::ScrollArea::vertical().show(ui, |ui| nav_heading(ui, level, text, true, None).1).inner);
                panel_rect = result.response.rect;
                heading_rect = result.inner.rect;
            });
            let (rows, elided) = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some((t.galley.rows.len(), t.galley.elided)),
                    _ => None,
                })
                .unwrap_or((0, false));
            output.drop_without_applying_deltas();
            (panel_rect, heading_rect, rows, elided)
        };
        for (width, level) in [(260.0, 0), (180.0, 5)] {
            let (_, one_line, _, _) = render(width, level, "Kilns");
            for text in &headings {
                let (panel_rect, heading_rect, rows, elided) = render(width, level, text);
                assert!(panel_rect.width() < width + 20.0, "long heading expanded the {width}px pane to {panel_rect:?}");
                assert!(heading_rect.max.x <= panel_rect.max.x, "heading escaped pane: {heading_rect:?} / {panel_rect:?}");
                // The heading wraps onto further lines instead of ending in an ellipsis (#3015).
                assert!(rows > 1, "long heading must wrap onto further lines at {width}px, got {rows}");
                assert!(!elided, "long heading must not end in an ellipsis at {width}px");
                // ... and its row grows with the text, where a short heading keeps one row.
                assert!(heading_rect.height() > one_line.height(), "a wrapped heading row is taller at {width}px: {heading_rect:?} vs {one_line:?}");
            }
        }
    }

    /// Word lists headings flush left, each level indented under its parent (#1199).
    #[test]
    fn headings_align_left_and_indent_by_level() {
        let text_left = |level: u8| {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 600.0));
            let mut panel_left = 0.0;
            let output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                let result = egui::Panel::left("navigation_test").default_size(260.0).show(ui, |ui| nav_heading(ui, level, "Kilns", false, None));
                panel_left = result.response.rect.min.x;
            });
            let x = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == "Kilns" => Some(t.pos.x + t.galley.rect.min.x),
                    _ => None,
                })
                .unwrap();
            output.drop_without_applying_deltas();
            x - panel_left
        };
        let (top, second) = (text_left(0), text_left(1));
        assert!(top < 40.0, "a top-level heading starts at the pane's left edge, not centred: {top}px in");
        assert!((second - top - 14.0).abs() < 0.5, "a level-2 heading is indented 14px under its parent: {top} / {second}");
    }

    /// Collapsing a heading hides everything nested under it, and only that (#1997).
    #[test]
    fn collapsed_heading_hides_its_descendants_only() {
        // 1 / 1.1 / 1.1.1 / 1.2 / 1.2.1 / 2 / 2.1
        let levels = [0, 1, 2, 1, 2, 0, 1];
        let none = [false; 7];
        assert_eq!(visible_headings(&levels, &none), vec![true; 7]);
        let mut c = none;
        c[3] = true; // collapse 1.2
        assert_eq!(visible_headings(&levels, &c), vec![true, true, true, true, false, true, true]);
        c[3] = false;
        c[0] = true; // collapse 1
        assert_eq!(visible_headings(&levels, &c), vec![true, false, false, false, false, true, true]);
        // A collapsed child inside a collapsed parent stays hidden and does not reopen siblings.
        c[1] = true;
        assert_eq!(visible_headings(&levels, &c), vec![true, false, false, false, false, true, true]);
    }
}
