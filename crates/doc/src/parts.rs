use super::block::Blocks;
use super::position::Pos;
use super::*;

/// A free id, `min` or more, for something new (a part, a list, a comment): one past `largest`, the
/// largest id in use, so ids keep growing; once that would pass `u32::MAX` (a file can use any id),
/// the lowest one `used` (every id in use) doesn't hold. `None` when every id from `min` up is taken.
pub fn free_id(largest: Option<u32>, used: impl Iterator<Item = u32>, min: u32) -> Option<u32> {
    let Some(largest) = largest else { return Some(min) };
    if let Some(next) = largest.checked_add(1) {
        return Some(next.max(min));
    }
    let mut ids: Vec<u32> = used.filter(|&i| i >= min).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut want = min;
    for i in ids {
        if i > want {
            break;
        }
        want = want.checked_add(1)?;
    }
    Some(want)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PartKind {
    #[default]
    Header,
    Footer,
    Footnote,
    Endnote,
    Comment,
    TextBox,
}

/// A document's own separators and continuation notice for its footnotes or its endnotes
/// (ECMA-376 §17.11: the `w:footnote`/`w:endnote` entries of type `separator`,
/// `continuationSeparator` and `continuationNotice`). `None` = the default: a short line above
/// the notes, a line across the column above notes continued from the page before, no notice.
/// The lines themselves are [`InlineObject::note_separator`] objects in the paragraphs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct NoteSeparators {
    /// Above the notes on a page.
    pub separator: Option<Blocks>,
    /// Above the rest of a note continued from the page before.
    pub continuation_separator: Option<Blocks>,
    /// Below the part of a note that continues on the next page.
    pub continuation_notice: Option<Blocks>,
}

impl NoteSeparators {
    /// No separator of the document's own: the defaults apply.
    pub fn is_empty(&self) -> bool {
        self.separator.is_none() && self.continuation_separator.is_none() && self.continuation_notice.is_none()
    }
}

/// A secondary story.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Part {
    pub kind: PartKind,
    pub blocks: Blocks,
    /// A text box's text direction, alignment and link (unused by other parts).
    #[serde(default, skip_serializing_if = "TextBody::is_default")]
    pub body: TextBody,
}

/// How a text box shows its text (Shape Format › Text): which way it runs, where it sits between
/// the box's top and bottom, and the box its overflow continues in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TextBody {
    pub vert: props::TextVert,
    pub anchor: props::VAlign,
    /// Create Link: the text box (another `TextBox` part, shown by its own shape) the text
    /// continues in when it doesn't fit this one. The linked box shows the rest of this box's
    /// story; its own blocks are unused. See [`Document::text_box_chains`].
    pub next: Option<u32>,
    /// Internal margins (left, top, right, bottom), EMU, when the file gives its own
    /// (`wps:bodyPr` `lIns`… ECMA-376 Part 1 §21.1.2.1.1, or VML `v:textbox` `inset`);
    /// `None` is Word's defaults.
    pub inset: Option<[u32; 4]>,
    /// Resize shape to fit text (`a:spAutoFit`, VML `mso-fit-shape-to-text`): Word grows the
    /// box to its text, so none of the text is cut.
    pub auto_fit: bool,
    /// The colour the shape's style gives its text (`wps:style/a:fontRef`): what automatic text
    /// colour means inside this box, instead of black. Text whose colour comes from its runs,
    /// styles or the document defaults keeps it (Word, observed with synthetic documents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_color: Option<props::Rgb>,
}

/// Word's default text box margins (left, top, right, bottom), EMU: 0.1" and 0.05".
pub const TEXT_BOX_INSET: [u32; 4] = [91_440, 45_720, 91_440, 45_720];

/// Widest text box margin kept, EMU (22", the largest page side).
pub const MAX_TEXT_BOX_INSET: u32 = 20_116_800;

impl TextBody {
    pub fn is_default(&self) -> bool {
        *self == TextBody::default()
    }

    /// The internal margins (left, top, right, bottom) in points.
    pub fn insets_pt(&self) -> [f32; 4] {
        self.inset.unwrap_or(TEXT_BOX_INSET).map(|v| v.min(MAX_TEXT_BOX_INSET) as f32 / 12_700.0)
    }
}

/// Most boxes one chain of linked text boxes runs through (hostile files).
pub const MAX_LINKED_BOXES: usize = 256;

/// A range that stays editable while the document is protected; see [`Document::perm_ranges`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermRange {
    pub id: u32,
    /// Editing group (`everyone`, `editors`…); empty when the range names an editor.
    pub group: String,
    /// The one editor the range is for; empty for a group.
    pub editor: String,
    pub start: Pos,
    pub end: Pos,
}

impl PermRange {
    /// Can everyone edit it (rather than one person or group)?
    pub fn everyone(&self) -> bool {
        self.group.eq_ignore_ascii_case("everyone")
    }
}
