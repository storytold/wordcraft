use super::page::MOVE_COLOR;
use super::*;

/// Formatting marks shown even while Show/Hide ¶ is off (File › Options › Display, "Always show
/// these formatting marks"). Show/Hide ¶ on shows them all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FormattingMarks {
    /// Tab characters (→).
    pub tabs: bool,
    /// Spaces (·).
    pub spaces: bool,
    /// Paragraph marks (¶), line breaks, page, column and section breaks.
    pub paragraphs: bool,
    /// Hidden text (shown with its dotted underline instead of left out).
    pub hidden: bool,
}

impl FormattingMarks {
    pub const ALL: FormattingMarks = FormattingMarks { tabs: true, spaces: true, paragraphs: true, hidden: true };

    /// The marks on screen: all of them with Show/Hide ¶ on, else the chosen ones.
    pub fn shown(self, show_all: bool) -> FormattingMarks {
        if show_all { FormattingMarks::ALL } else { self }
    }

    pub fn any(self) -> bool {
        self.tabs || self.spaces || self.paragraphs || self.hidden
    }
}

#[derive(Clone, Debug)]
pub struct DisplayOptions {
    /// Show formatting marks (¶): Show/Hide ¶, every mark.
    pub marks: bool,
    /// Marks shown while `marks` is off (File › Options › Display).
    pub always: FormattingMarks,
    /// Dim headers/footers (editing the body) or the body (editing a header/footer).
    pub dim_header: bool,
    pub dim_body: bool,
    /// Show tracked changes as markup (coloured, underlined/struck).
    pub markup: bool,
    /// Show on-screen-only marks: equation placeholders and prompts (never in print or PDF).
    pub placeholders: bool,
    /// Review › Hide Ink: leave ink strokes out (they stay in the document).
    pub hide_ink: bool,
    /// How tracked changes and comments are marked when `markup` is on (Track Changes Options).
    pub revisions: MarkupOptions,
    /// Leave this object out of the page: while it is dragged, the preview under the pointer
    /// stands in for it, so the page must not also show it where it was.
    pub hide_object: Option<wordcraft_doc::Pos>,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        DisplayOptions {
            marks: false,
            always: FormattingMarks::default(),
            dim_header: true,
            dim_body: false,
            markup: true,
            placeholders: false,
            hide_ink: false,
            revisions: MarkupOptions::default(),
            hide_object: None,
        }
    }
}

/// An object left out of a page's drawing ([`DisplayOptions::hide_object`]).
pub(super) struct Hidden {
    object: wordcraft_doc::Pos,
    /// The text box stories it shows (its own and its group members', resolved to the story each
    /// one shows): a box's text is drawn as lines of its story, so it goes with the box.
    boxes: Vec<u32>,
}

impl Hidden {
    /// The object at `object` and the text box stories whose lines belong to it.
    pub(super) fn new(doc: &Document, object: &wordcraft_doc::Pos) -> Hidden {
        let boxes = doc
            .para(object.story, &object.path)
            .and_then(|p| p.object_at(object.off))
            .map(|o| o.text_boxes().into_iter().map(|id| doc.text_box_chain(id).and_then(|c| c.first().copied()).unwrap_or(id)).collect())
            .unwrap_or_default();
        Hidden { object: object.clone(), boxes }
    }

    pub(super) fn hides(&self, it: &Placed) -> bool {
        let me = &self.object;
        match it {
            Placed::Object { story, path, off, .. } | Placed::Image { story, path, off, .. } | Placed::Graphic { story, path, off, .. } => {
                *story == me.story && *path == me.path && *off == me.off
            }
            Placed::Shape { object: Some(pos), .. } => pos == me,
            Placed::Lines { story: StoryRef::Part(id), .. } => self.boxes.contains(id),
            _ => false,
        }
    }
}

/// An option list with stable names for commands and saved preferences.
macro_rules! named {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub enum $name { $($(#[$vm])* $v),+ }
        impl $name {
            /// Every choice with its name, in menu order.
            pub const ALL: &'static [($name, &'static str)] = &[$(($name::$v, $s)),+];
            pub fn name(self) -> &'static str {
                Self::ALL.iter().find(|(v, _)| *v == self).map(|(_, s)| *s).unwrap_or("")
            }
            pub fn parse(s: &str) -> Option<Self> {
                Self::ALL.iter().find(|(_, n)| *n == s).map(|(v, _)| *v)
            }
        }
    };
}

named! {
    /// How inserted text is marked.
    InsertMark {
        #[default]
        Underline = "underline",
        DoubleUnderline = "doubleUnderline",
        Bold = "bold",
        Italic = "italic",
        Strikethrough = "strikethrough",
        ColorOnly = "colorOnly",
        None = "none",
    }
}

named! {
    /// How deleted text is marked.
    DeleteMark {
        #[default]
        Strikethrough = "strikethrough",
        DoubleStrikethrough = "doubleStrikethrough",
        Hidden = "hidden",
        Caret = "caret",
        Hash = "hash",
        Underline = "underline",
        ColorOnly = "colorOnly",
        /// No mark (an ordinary deletion keeps its colour; moved-away text is plain, as in Word).
        None = "none",
        Bold = "bold",
        Italic = "italic",
        DoubleUnderline = "doubleUnderline",
    }
}

named! {
    /// Where the bar beside changed lines goes.
    ChangeBar {
        #[default]
        Outside = "outside",
        Left = "left",
        Right = "right",
        None = "none",
    }
}

named! {
    /// What the markup area beside the page shows.
    BalloonMode {
        /// Deletions, formatting and comments in balloons; insertions stay inline. Where the
        /// editor shows balloons, deleted body text leaves the layout (`deletions_in_balloons`)
        /// and the editor draws a "Deleted: …" balloon at its place; elsewhere (no markup area,
        /// PDF, and headers, footers, notes and text boxes, which get no balloons) it is drawn
        /// inline as in `CommentsAndFormatting`.
        Revisions = "revisions",
        /// Everything inline, no markup area.
        Inline = "inline",
        #[default]
        CommentsAndFormatting = "commentsAndFormatting",
    }
}

named! {
    /// How comments are shown beside the page.
    CommentDisplay {
        /// Every comment in the margin, as in Word.
        #[default]
        Margin = "margin",
        /// Only the comment of the text under the caret (or the clicked one) floats beside the page.
        Selected = "selected",
    }
}

/// Track Changes Options: what markup shows and how revisions are drawn. A per-user preference,
/// as in Word (saved with the interface settings, not the document).
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarkupOptions {
    pub comments: bool,
    pub ink: bool,
    pub insertions_deletions: bool,
    pub formatting: bool,
    pub balloons: BalloonMode,
    pub comment_display: CommentDisplay,
    pub insert_mark: InsertMark,
    /// `None` = by author.
    pub insert_color: Option<Rgb>,
    pub delete_mark: DeleteMark,
    /// `None` = by author.
    pub delete_color: Option<Rgb>,
    pub changed_lines: ChangeBar,
    /// `None` = automatic (dark grey).
    pub changed_lines_color: Option<Rgb>,
    /// Record formatting changes while tracking.
    pub track_formatting: bool,
    /// How the place moved text was taken from is marked (Moves › Moved from).
    pub move_from_mark: DeleteMark,
    /// `None` = by author.
    pub move_from_color: Option<Rgb>,
    /// How moved text is marked where it went (Moves › Moved to).
    pub move_to_mark: InsertMark,
    /// `None` = by author.
    pub move_to_color: Option<Rgb>,
}

impl Default for MarkupOptions {
    fn default() -> Self {
        MarkupOptions {
            comments: true,
            ink: true,
            insertions_deletions: true,
            formatting: true,
            balloons: BalloonMode::default(),
            comment_display: CommentDisplay::default(),
            insert_mark: InsertMark::default(),
            insert_color: None,
            delete_mark: DeleteMark::default(),
            delete_color: None,
            changed_lines: ChangeBar::default(),
            changed_lines_color: None,
            track_formatting: true,
            move_from_mark: DeleteMark::DoubleStrikethrough,
            move_from_color: Some(MOVE_COLOR),
            move_to_mark: InsertMark::DoubleUnderline,
            move_to_color: Some(MOVE_COLOR),
        }
    }
}

impl MarkupOptions {
    /// Whether a tracked deletion takes no room with markup shown: insertions and deletions
    /// aren't shown, or its mark is Hidden. Text a move took away (`moved_from`) follows Moved
    /// from, other deletions follow Deletions, so Deletions = Hidden leaves moved-away text shown
    /// (Word for Mac's behaviour for that combination is unverified: this is the one place to
    /// change it).
    pub fn hides_deletion(&self, moved_from: bool) -> bool {
        let mark = if moved_from { self.move_from_mark } else { self.delete_mark };
        !self.insertions_deletions || mark == DeleteMark::Hidden
    }

    /// The deletions the layout leaves out with markup shown ([`crate::LayoutOptions::hide_marked`]).
    pub fn hidden_deletions(&self) -> crate::HideDeleted {
        crate::HideDeleted { plain: self.hides_deletion(false), moved_from: self.hides_deletion(true) }
    }
}
