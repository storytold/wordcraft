use serde::{Deserialize, Serialize};
use wordcraft_layout::{NumeralMode, ViewMode};

/// View state that commands can change (ribbon View tab, status bar).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewState {
    pub zoom: f32,
    pub mode: ViewMode,
    pub read_mode: bool,
    /// View › Focus (#1678): the document alone on screen, with a slim bar to drop the ribbon down.
    pub focus_mode: bool,
    /// Full Screen View (#1678): the pages alone on screen. Never on together with `focus_mode`.
    #[serde(default)]
    pub full_screen_view: bool,
    pub marks: bool,
    pub ruler: bool,
    /// View › Show › Gridlines: the drawing grid over the page's text area (on screen only).
    pub gridlines: bool,
    /// Table Layout › View Gridlines: outlines of table cells (on screen only); on by default, as in Word.
    #[serde(default = "on")]
    pub table_gridlines: bool,
    pub nav_pane: bool,
    pub styles_pane: bool,
    /// Where the Styles pane sits: docked right (the default) or left, or floating (#1756).
    /// Kept in the user's preferences, not the document.
    #[serde(default)]
    pub styles_pane_dock: StylesPaneDock,
    /// Style Inspector pane (Home › Styles).
    #[serde(default)]
    pub style_inspector: bool,
    pub comments_pane: bool,
    /// The Clipboard pane (Home › Clipboard): items collected by Copy and Cut.
    #[serde(default)]
    pub clipboard_pane: bool,
    /// SmartArt Design › Text Pane: the selected SmartArt graphic's items as an editable outline.
    #[serde(default)]
    pub smart_art_pane: bool,
    /// The Accessibility pane (Review › Check Accessibility): the checker's findings.
    #[serde(default)]
    pub accessibility_pane: bool,
    /// The Alt Text pane (Picture/Shape Format › Alt Text): the selected object's or table's alt text.
    #[serde(default)]
    pub alt_text_pane: bool,
    /// The Format Shape pane (Shape Fill › More Gradients…, a shape's Format Shape…): the
    /// selected shapes' fill.
    #[serde(default)]
    pub format_shape_pane: bool,
    pub multi_page: bool,
    /// One Page or Page Width was chosen last: keep one page per row when a typed or wheel zoom replaces the fit mode.
    #[serde(default)]
    pub single_column: bool,
    /// Zoom to fit: "pageWidth", "onePage", "multiplePages", or empty.
    pub fit: String,
    /// Web Layout's text width, which the UI keeps at the window's width (#1876); never saved.
    /// Draft and Outline don't use it: they keep a fixed 800 pt column.
    #[serde(skip, default = "default_web_width")]
    pub web_width: f32,
    /// Transient viewport metrics for reading pagination; never saved in a document or preferences.
    #[serde(skip, default = "default_read_width")]
    pub read_width: f32,
    #[serde(skip, default = "default_read_height")]
    pub read_height: f32,
    /// Reader-only layout controls; they are neither document formatting nor saved UI prefs.
    /// `None` chooses responsive column width and keeps the document's line spacing.
    #[serde(skip)]
    pub read_column_width: Option<f32>,
    #[serde(skip, default = "default_read_zoom")]
    pub read_zoom: f32,
    #[serde(skip)]
    pub read_line_spacing: Option<f32>,
    /// Read Mode's Page Color: the paper the reading columns are drawn on.
    #[serde(skip)]
    pub read_page_color: crate::cmd::view::ReadPageColor,
    pub dark_mode: bool,
    pub show_markup: bool,
    pub track_changes_pane: bool,
    /// Check spelling and grammar as you type.
    pub proofing: bool,
    /// How digits look on screen (Word › File › Options › Advanced › Numeral).
    #[serde(default)]
    pub numeral: NumeralMode,
    /// Review › Hide Ink: ink strokes aren't shown on screen (they stay in the document).
    #[serde(default)]
    pub hide_ink: bool,
    /// Draw tab: the tool dragging on the page uses, and each pen's colour and thickness.
    #[serde(default)]
    pub draw: crate::cmd::draw::DrawState,
    /// Show field codes instead of their results (`view.fieldCodes`, Alt+F9).
    #[serde(default)]
    pub field_codes: bool,
    /// Print Layout › Hide White Space: pages run together, the top and bottom margins (with
    /// headers and footers) between pages collapse to a thin line. On screen only; the layout
    /// and page count are unchanged.
    #[serde(default)]
    pub hide_white_space: bool,
    /// Outline view: collapsed headings, Show Level, first lines, text formatting.
    #[serde(default)]
    pub outline: crate::cmd::outline::OutlineState,
    /// Developer › Design Mode: every content control shows its frame and title.
    #[serde(default)]
    pub design_mode: bool,
    /// View › Window › Full Screen (F11): the window fills the screen (#980). The front end
    /// carries it to the window.
    #[serde(default)]
    pub full_screen: bool,
}

/// Where the Styles pane sits (`view.stylesPaneDock`, #1756).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StylesPaneDock {
    #[default]
    Right,
    Left,
    /// A window of its own over the document, which can be moved and resized.
    #[serde(rename = "float")]
    Floating,
}

impl StylesPaneDock {
    /// `left`, `right` or `float`; anything else is `None`.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "right" => Some(Self::Right),
            "left" => Some(Self::Left),
            "float" => Some(Self::Floating),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Left => "left",
            Self::Floating => "float",
        }
    }
}

fn on() -> bool {
    true
}

/// The text column of Draft and Outline, and of Web Layout in a headless session.
pub(super) const DRAFT_WIDTH: f32 = 800.0;

fn default_web_width() -> f32 {
    DRAFT_WIDTH
}

fn default_read_width() -> f32 {
    300.0
}

fn default_read_height() -> f32 {
    500.0
}

fn default_read_zoom() -> f32 {
    1.0
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            zoom: 1.0,
            mode: ViewMode::Print,
            read_mode: false,
            focus_mode: false,
            full_screen_view: false,
            marks: false,
            ruler: true,
            gridlines: false,
            table_gridlines: true,
            nav_pane: false,
            styles_pane: false,
            styles_pane_dock: StylesPaneDock::Right,
            style_inspector: false,
            comments_pane: false,
            clipboard_pane: false,
            smart_art_pane: false,
            accessibility_pane: false,
            alt_text_pane: false,
            format_shape_pane: false,
            multi_page: false,
            single_column: false,
            fit: String::new(),
            web_width: default_web_width(),
            read_width: default_read_width(),
            read_height: default_read_height(),
            read_column_width: None,
            read_zoom: default_read_zoom(),
            read_line_spacing: None,
            read_page_color: Default::default(),
            dark_mode: false,
            show_markup: true,
            track_changes_pane: false,
            proofing: true,
            numeral: NumeralMode::default(),
            hide_ink: false,
            draw: Default::default(),
            field_codes: false,
            hide_white_space: false,
            outline: Default::default(),
            design_mode: false,
            full_screen: false,
        }
    }
}
