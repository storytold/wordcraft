//! Task panes: Navigation (headings, pages, search results), Clipboard, Styles, Style Inspector,
//! Accessibility and Alt Text (see `a11y_pane`), Format Shape (see `format_shape`), Restrict Editing
//! ([`crate::restrict_pane`]), Comments, and the SmartArt Text Pane. The framework's pane area draws
//! them around the document, with their title bars, menus and resize edges; the session's view flags
//! say which are open.

use craft::theme::SizeTokens;
use craft::ui::contract::SizeLimits;
use craft::ui::patterns::dock::{PaneContent, PaneEdges, PaneSpec, PaneState};
use craft::ui::runtime::env::PaneEdge;
use egui::{Ui, vec2};

use crate::WordApp;

mod comments;
mod host;
mod inspector;
mod nav;
mod side;
mod state;

use comments::comments;
pub(crate) use comments::{commit_comment, commit_reply, flush_comment_drafts, has_comment_drafts, state_has_drafts};
use host::Content;
#[cfg(test)]
pub(crate) use host::apply;
pub(crate) use host::dock_command;
pub use host::show;
pub(crate) use inspector::describe_props;
use inspector::inspector;
use nav::nav;
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
    Proofing,
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
            Self::Proofing => "Proofing",
            Self::Comments => "Comments",
        }
    }

    /// The pane's default edge (logical: `Start` is the reading start, so it mirrors in a right-to-left
    /// interface by itself) and docked width.
    fn default_edge(self) -> (PaneEdge, f32) {
        match self {
            Self::SmartArtText => (PaneEdge::Start, 250.0),
            Self::Navigation => (PaneEdge::Start, 260.0),
            Self::Clipboard => (PaneEdge::Start, 250.0),
            Self::Styles => (PaneEdge::End, 250.0),
            Self::StyleInspector => (PaneEdge::End, 240.0),
            Self::Accessibility => (PaneEdge::End, 280.0),
            Self::AltText => (PaneEdge::End, 260.0),
            Self::FormatShape => (PaneEdge::End, 280.0),
            Self::RestrictEditing => (PaneEdge::End, 270.0),
            Self::Proofing => (PaneEdge::End, 400.0),
            Self::Comments => (PaneEdge::End, 290.0),
        }
    }

    /// What the pane area needs to know about the pane: title, default place, narrowest body, and that it docks
    /// beside the document only. The least body width and the first float size are the theme's dock sizes.
    fn spec(self, size: &SizeTokens) -> PaneSpec {
        let float = vec2(size.float.width, size.float.height);
        let mut spec = PaneSpec::new(tl!(self.title()).to_owned(), self.default_state(), float);
        spec.limits = SizeLimits::at_least(vec2(size.pane.least, 0.0));
        spec.edges = PaneEdges::SIDES;
        spec
    }

    /// The pane's place and docked width when the user has kept none.
    fn default_state(self) -> PaneState {
        let (edge, width) = self.default_edge();
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
            Self::Proofing => "proofing",
            Self::Comments => "comments",
        }
    }

    const ALL: [Self; 11] = [
        Self::SmartArtText,
        Self::Navigation,
        Self::Clipboard,
        Self::Styles,
        Self::StyleInspector,
        Self::Accessibility,
        Self::AltText,
        Self::FormatShape,
        Self::RestrictEditing,
        Self::Proofing,
        Self::Comments,
    ];

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|pane| pane.key() == key)
    }

    /// The command that closes the pane; Restrict Editing has none and closes in [`host::close`].
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
            Self::RestrictEditing | Self::Proofing => None,
        }
    }
}

/// Draws the body of `pane`. The pane area scrolls it, so no body brings a scroll area of its own.
fn pane_body(app: &mut WordApp, ui: &mut Ui, pane: Pane) {
    match pane {
        Pane::SmartArtText => crate::dialogs_smart_art::text_pane(app, ui),
        Pane::Navigation => nav(app, ui),
        Pane::Clipboard => clipboard(app, ui),
        Pane::Styles => styles(app, ui),
        Pane::StyleInspector => inspector(app, ui),
        Pane::Accessibility => crate::a11y_pane::accessibility(app, ui),
        Pane::AltText => crate::a11y_pane::alt_text(app, ui),
        Pane::FormatShape => crate::format_shape::pane(app, ui),
        Pane::RestrictEditing => crate::restrict_pane::show(app, ui),
        Pane::Proofing => crate::proofing_pane::show(app, ui),
        Pane::Comments => comments(app, ui),
    }
}

impl PaneContent<Pane> for Content<'_, '_> {
    fn spec(&self, pane: &Pane) -> PaneSpec {
        pane.spec(&self.size)
    }

    fn state(&self, pane: &Pane) -> Option<PaneState> {
        Some(state::pane_state(&self.app.borrow(), *pane, self.rtl))
    }

    fn body(&mut self, pane: &Pane, ui: &mut Ui, _cx: &mut craft::ui::contract::BodyCx<'_>) {
        let mut app = self.app.borrow_mut();
        pane_body(&mut app, ui, *pane);
    }
}

#[cfg(test)]
mod tests;
