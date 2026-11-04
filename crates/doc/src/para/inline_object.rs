use super::float::{Float, Wrap, cap_alt};
use super::inline_kinds::{NoteKind, PTabAlign, PTabBase, RubyAlign, ShapeKind};
use super::{CharProps, Deserialize, Serialize};

/// A picture's Crop to Shape outline: the shape WordCraft draws, and the file's own preset
/// (DrawingML `prst` and its `a:avLst` adjust values) when that isn't exactly `kind`'s, so a
/// preset WordCraft draws as an approximation is saved back as it was.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PictureShape {
    /// The outline drawn.
    pub kind: ShapeKind,
    /// The file's preset name; empty: `kind`'s own.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prst: String,
    /// The file's adjust values (`a:gd` `name`, `fmla`), in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub av: Vec<(String, String)>,
}

impl PictureShape {
    /// WordCraft's own `kind` outline, nothing kept from a file.
    pub fn new(kind: ShapeKind) -> PictureShape {
        PictureShape { kind, prst: String::new(), av: Vec::new() }
    }
}

/// The most a picture's side can be padded: crop fractions go no lower (Fit's negative crop).
pub const MIN_CROP: f32 = -100.0;
/// The most a command crops away from one side of a picture (Fill, Aspect Ratio).
pub const MAX_CROP: f32 = 0.95;

/// An inline object anchored at a U+FFFC in the paragraph text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InlineObject {
    Image {
        /// Key in `Document::media`.
        media: String,
        /// Display size, points.
        w: f32,
        h: f32,
        #[serde(default)]
        alt: String,
        #[serde(default)]
        float: Float,
        /// Crop (left, top, right, bottom) fractions of the source: positive trims that side,
        /// negative pads it (transparent, DrawingML's negative `a:srcRect`; Fit).
        #[serde(default)]
        crop: [f32; 4],
        /// The OLE object (embedded or linked file) or Word signature line this picture shows, as
        /// read from a file: saving writes the object back, not only its picture. See
        /// [`crate::graphic::Embedded`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ole: Option<std::sync::Arc<crate::graphic::Embedded>>,
        /// Crop to Shape (Picture Format › Crop › Crop to Shape): the outline the picture is
        /// clipped to, DrawingML's `a:prstGeom` in `pic:spPr`. None: a rectangle.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shape: Option<PictureShape>,
    },
    /// A chart or SmartArt diagram (see [`crate::graphic`]).
    Graphic {
        w: f32,
        h: f32,
        #[serde(default)]
        alt: String,
        #[serde(default)]
        float: Float,
        graphic: std::sync::Arc<crate::graphic::Graphic>,
    },
    Shape {
        kind: ShapeKind,
        w: f32,
        h: f32,
        fill: Option<crate::fill::Fill>,
        stroke: Option<crate::props::Rgb>,
        #[serde(default)]
        stroke_width: f32,
        #[serde(default)]
        float: Float,
        /// Text box content: `Document::parts` id.
        #[serde(default)]
        story: Option<u32>,
        /// A freeform's geometry (`ShapeKind::Freeform`): its paths, or an ink stroke.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        freeform: Option<std::sync::Arc<crate::freeform::Freeform>>,
        /// Shadow, glow and soft edges.
        #[serde(default, skip_serializing_if = "crate::effects::ShapeEffects::is_empty")]
        effects: crate::effects::ShapeEffects,
        /// Drawing id, text warp, connector ends and arrowheads.
        #[serde(default, skip_serializing_if = "crate::connector::ShapeExtra::is_empty")]
        extra: crate::connector::ShapeExtra,
    },
    /// Pictures, shapes and text boxes grouped into one object (Layout › Arrange › Group): it
    /// moves, wraps and resizes as one. Its members are laid out in the group's own coordinate
    /// space, `ch_w` × `ch_h`, which is stretched over the group's `w` × `h` (DrawingML's
    /// `a:chExt` and `a:ext`), so resizing the group scales them.
    Group {
        w: f32,
        h: f32,
        #[serde(default)]
        float: Float,
        /// Size of the members' coordinate space (their offsets and sizes are in it).
        ch_w: f32,
        ch_h: f32,
        children: Vec<GroupChild>,
        /// A Drawing Canvas (Insert › Shapes › New Drawing Canvas, DrawingML's `wpc:wpc`) rather
        /// than a group: it has its own background and outline, and its frame is independent of
        /// its members: resizing it doesn't scale them (`ch_w` × `ch_h` follow `w` × `h`), and
        /// what lies outside the frame is clipped.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        canvas: Option<CanvasStyle>,
    },
    /// A field: `instr` is the field code (`PAGE`, `NUMPAGES`, `DATE \@ "M/d/yyyy"`, `TOC \o "1-3"`…);
    /// `result` the cached display text. A `locked` field keeps its result when fields update.
    Field {
        instr: String,
        #[serde(default)]
        result: String,
        #[serde(default)]
        locked: bool,
        /// `w:dirty` (ECMA-376 §17.16.5.35): the cached result is stale — another program that
        /// couldn't compute it set the flag so Word refreshes the field on open. Kept through
        /// saves; cleared when WordCraft computes a result, like Word's F9.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        dirty: bool,
        /// Shows its code (`{ PAGE }`) instead of its result, the other way round from the
        /// document-wide View › Field Codes setting (`fields.toggleCode`, Shift+F9). A view
        /// toggle: never written to files.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        code: bool,
        /// A legacy form field's definition (`FORMCHECKBOX`, `FORMTEXT`, `FORMDROPDOWN`): the
        /// source's `w:ffData`, raw OOXML kept for round trip. Empty for other fields.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        form: String,
    },
    /// The start of a field whose result is ordinary content — formatted, possibly spanning
    /// paragraphs — up to the matching [`InlineObject::FieldEnd`]. Imported TOCs and citation
    /// managers' `ADDIN` fields are kept this way; `instr` is the field code. See
    /// [`crate::fields`].
    FieldStart {
        instr: String,
        #[serde(default)]
        locked: bool,
        /// `w:dirty` on the begin marker, as [`InlineObject::Field::dirty`].
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        dirty: bool,
    },
    /// The end of the innermost open [`InlineObject::FieldStart`].
    FieldEnd,
    /// The start of a content control whose content is what follows, up to the matching
    /// [`InlineObject::ControlEnd`]. See [`crate::control`].
    ControlStart {
        control: Box<crate::control::ContentControl>,
    },
    /// The end of the innermost open [`InlineObject::ControlStart`].
    ControlEnd,
    NoteRef {
        kind: NoteKind,
        /// `Document::parts` id of the note's story.
        id: u32,
        /// Custom mark (empty = automatic number).
        #[serde(default)]
        custom: String,
    },
    BookmarkStart {
        name: String,
    },
    BookmarkEnd {
        name: String,
    },
    CommentStart {
        id: u32,
    },
    CommentEnd {
        id: u32,
    },
    /// The start of a range that stays editable while the document is protected (Restrict
    /// Editing exceptions, `w:permStart`): for everyone (`group` = `everyone`), another editing
    /// group, or one editor (`editor`, a user name or address).
    PermStart {
        id: u32,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        group: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        editor: String,
    },
    /// The end of the editable range [`InlineObject::PermStart`] with the same `id` opened.
    PermEnd {
        id: u32,
    },
    /// An equation: its linear format (`x=(-b±√(b^2-4ac))/2a`, for plain text) and structure.
    /// A display equation sits on a line of its own.
    Equation {
        linear: String,
        #[serde(default)]
        display: bool,
        /// The structure; empty = parse `linear`.
        #[serde(default, skip_serializing_if = "crate::math::Math::is_empty")]
        math: crate::math::Math,
    },
    /// Ruby (Phonetic Guide): small `ruby` text (furigana, pinyin…) set over `base` text. The base
    /// text takes the formatting of the object's run; it reads as the paragraph's text.
    Ruby {
        base: String,
        ruby: String,
        #[serde(default)]
        align: RubyAlign,
        /// Ruby text size, points (`w:hps`).
        size: f32,
        /// From the base text's baseline up to the ruby text's baseline, points (`w:hpsRaise`).
        raise: f32,
        /// The base text size the ruby was set for, points (`w:hpsBaseText`).
        #[serde(default)]
        base_size: f32,
        /// Language of the ruby (`w:lid`, e.g. `ja-JP`); empty = unspecified.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        lang: String,
        /// Formatting of the ruby text over the run's (font, colour…); its size is `size`.
        #[serde(default, skip_serializing_if = "CharProps::is_empty")]
        props: Box<CharProps>,
    },
    /// A positional tab (Word's alignment tab, `w:ptab`): it goes to the left edge, centre or
    /// right edge of the margins or of the indents, whatever the paragraph's tab stops, and the
    /// text after it lines up there. Text already past that place sends it to the next line.
    PositionalTab {
        #[serde(default)]
        align: PTabAlign,
        #[serde(default)]
        relative_to: PTabBase,
        #[serde(default)]
        leader: crate::props::TabLeader,
    },
    /// Something we don't model, kept for round-trip (raw XML of the source format).
    Opaque {
        format: String,
        xml: String,
        #[serde(default)]
        text: String,
    },
}

/// One member of an [`InlineObject::Group`]: a picture or shape (its own `w`, `h` and story), at
/// `x`, `y` in the group's coordinate space. Its `float` is unused.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupChild {
    pub x: f32,
    pub y: f32,
    pub obj: InlineObject,
}

/// Most members a group keeps (hostile files).
pub const MAX_GROUP_CHILDREN: usize = 1000;

/// The background fill and outline of a Drawing Canvas (an [`InlineObject::Group`] with `canvas`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CanvasStyle {
    pub fill: Option<crate::fill::Fill>,
    pub stroke: Option<crate::props::Rgb>,
    /// Outline width, points.
    pub stroke_width: f32,
}

impl InlineObject {
    /// A picture, chart or diagram, shape, text box or group: something drawn with a frame, sized and wrapped.
    pub fn is_drawing(&self) -> bool {
        matches!(self, InlineObject::Image { .. } | InlineObject::Graphic { .. } | InlineObject::Shape { .. } | InlineObject::Group { .. })
    }
    /// A drawing's size and placement.
    pub fn frame(&self) -> Option<(f32, f32, &Float)> {
        match self {
            InlineObject::Image { w, h, float, .. }
            | InlineObject::Graphic { w, h, float, .. }
            | InlineObject::Shape { w, h, float, .. }
            | InlineObject::Group { w, h, float, .. } => Some((*w, *h, float)),
            _ => None,
        }
    }
    /// A drawing's placement, to change.
    pub fn float_mut(&mut self) -> Option<&mut Float> {
        match self {
            InlineObject::Image { float, .. }
            | InlineObject::Graphic { float, .. }
            | InlineObject::Shape { float, .. }
            | InlineObject::Group { float, .. } => Some(float),
            _ => None,
        }
    }
    /// Resize a drawing (a group's members scale with it; a canvas's keep their size and place).
    pub fn set_size(&mut self, nw: f32, nh: f32) {
        if let InlineObject::Image { w, h, .. }
        | InlineObject::Graphic { w, h, .. }
        | InlineObject::Shape { w, h, .. }
        | InlineObject::Group { w, h, .. } = self
        {
            *w = nw;
            *h = nh;
        }
        if let InlineObject::Group { ch_w, ch_h, canvas: Some(_), .. } = self {
            *ch_w = nw;
            *ch_h = nh;
        }
    }
    /// A Drawing Canvas's background and outline (`None` for anything else).
    pub fn canvas_style(&self) -> Option<&CanvasStyle> {
        match self {
            InlineObject::Group { canvas, .. } => canvas.as_ref(),
            _ => None,
        }
    }
    /// Whether this is a Drawing Canvas.
    pub fn is_canvas(&self) -> bool {
        self.canvas_style().is_some()
    }
    /// The text box story this object shows, if it is a text box.
    pub fn text_box(&self) -> Option<u32> {
        match self {
            InlineObject::Shape { story, .. } => *story,
            _ => None,
        }
    }
    /// The text box stories this object shows: a text box's, or its group members'.
    pub fn text_boxes(&self) -> Vec<u32> {
        match self {
            InlineObject::Shape { story: Some(id), .. } => vec![*id],
            InlineObject::Group { children, .. } => children.iter().filter_map(|c| c.obj.text_box()).collect(),
            _ => Vec::new(),
        }
    }
    /// The text box story slots of this object and its group members, to repoint.
    pub fn text_box_slots(&mut self) -> Vec<&mut Option<u32>> {
        match self {
            InlineObject::Shape { story, .. } => vec![story],
            InlineObject::Group { children, .. } => children
                .iter_mut()
                .filter_map(|c| match &mut c.obj {
                    InlineObject::Shape { story, .. } => Some(story),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
    /// A group's members placed in a `w` × `h` box at (`x`, `y`): each member's rectangle
    /// (x, y, w, h) and the member. Empty for anything else.
    pub fn group_rects(&self, x: f32, y: f32, w: f32, h: f32) -> Vec<([f32; 4], &InlineObject)> {
        let InlineObject::Group { ch_w, ch_h, children, .. } = self else { return Vec::new() };
        let fin = |v: f32| if v.is_finite() { v } else { 0.0 };
        let sx = if fin(*ch_w) > 0.0 { fin(w) / ch_w } else { 1.0 };
        let sy = if fin(*ch_h) > 0.0 { fin(h) / ch_h } else { 1.0 };
        children
            .iter()
            .take(MAX_GROUP_CHILDREN)
            .filter_map(|c| {
                let (cw, chh, _) = c.obj.frame()?;
                let r = [x + fin(c.x) * sx, y + fin(c.y) * sy, (fin(cw) * sx).clamp(0.0, 4000.0), (fin(chh) * sy).clamp(0.0, 4000.0)];
                (!matches!(c.obj, InlineObject::Group { .. })).then_some((r, &c.obj))
            })
            .collect()
    }
    /// A legacy form field's definition (`FORMCHECKBOX`, `FORMTEXT`, `FORMDROPDOWN`), read from
    /// the `w:ffData` it keeps; `None` for anything else.
    pub fn legacy_form(&self) -> Option<crate::legacy_form::LegacyForm> {
        match self {
            InlineObject::Field { form, .. } if !form.is_empty() => crate::legacy_form::LegacyForm::parse(form),
            _ => None,
        }
    }
    /// Zero-width markers don't take part in layout.
    pub fn is_marker(&self) -> bool {
        matches!(
            self,
            InlineObject::BookmarkStart { .. }
                | InlineObject::BookmarkEnd { .. }
                | InlineObject::CommentStart { .. }
                | InlineObject::CommentEnd { .. }
                | InlineObject::PermStart { .. }
                | InlineObject::PermEnd { .. }
                | InlineObject::FieldStart { .. }
                | InlineObject::FieldEnd
                | InlineObject::ControlStart { .. }
                | InlineObject::ControlEnd
        )
    }
    /// A drawing's description (alternative text), wherever it is kept.
    pub fn alt_text(&self) -> &str {
        match self {
            InlineObject::Image { alt, .. } | InlineObject::Graphic { alt, .. } => alt,
            InlineObject::Shape { float, .. } | InlineObject::Group { float, .. } => &float.alt.text,
            _ => "",
        }
    }
    /// Set a drawing's description (capped at [`MAX_ALT_TEXT`] characters); `false` if this is no drawing.
    pub fn set_alt_text(&mut self, text: &str) -> bool {
        let text = cap_alt(text);
        match self {
            InlineObject::Image { alt, .. } | InlineObject::Graphic { alt, .. } => *alt = text,
            InlineObject::Shape { float, .. } | InlineObject::Group { float, .. } => float.alt.text = text,
            _ => return false,
        }
        true
    }
    /// A drawing marked decorative.
    pub fn is_decorative(&self) -> bool {
        self.frame().is_some_and(|(_, _, f)| f.alt.decorative)
    }
    pub fn is_floating(&self) -> bool {
        match self {
            InlineObject::Image { float, .. }
            | InlineObject::Graphic { float, .. }
            | InlineObject::Shape { float, .. }
            | InlineObject::Group { float, .. } => float.wrap != Wrap::Inline,
            _ => false,
        }
    }
    /// The geometry of an ink stroke (a freeform shape drawn with a pen), if this is one.
    pub fn ink(&self) -> Option<&crate::freeform::Freeform> {
        match self {
            InlineObject::Shape { freeform: Some(f), .. } if f.is_ink() => Some(f),
            _ => None,
        }
    }
    /// The line a footnote or endnote separator draws (ECMA-376 §17.11: the `w:separator` and
    /// `w:continuationSeparator` run elements): a short line, or with `continuation` one across
    /// the column. Kept as the run element itself.
    pub fn note_separator(continuation: bool) -> InlineObject {
        let el = if continuation { "w:continuationSeparator" } else { "w:separator" };
        InlineObject::Opaque { format: "docx".into(), xml: format!("<w:r><{el}/></w:r>"), text: String::new() }
    }
    /// For a [`InlineObject::note_separator`]: whether it is the continuation separator.
    pub fn as_note_separator(&self) -> Option<bool> {
        match self {
            InlineObject::Opaque { format, xml, .. } if format == "docx" => match xml.as_str() {
                "<w:r><w:separator/></w:r>" => Some(false),
                "<w:r><w:continuationSeparator/></w:r>" => Some(true),
                _ => None,
            },
            _ => None,
        }
    }
    /// The text this object contributes to plain-text extraction.
    pub fn plain_text(&self) -> &str {
        match self {
            InlineObject::Field { result, .. } => result,
            InlineObject::Equation { linear, .. } => linear,
            InlineObject::Ruby { base, .. } => base,
            InlineObject::PositionalTab { .. } => "\t",
            InlineObject::Opaque { text, .. } => text,
            _ => "",
        }
    }
}
