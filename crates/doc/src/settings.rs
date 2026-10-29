use super::{Deserialize, Rgb, Serialize, math, section, styles};

/// A bibliography source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Source {
    /// Unique tag cited by CITATION fields.
    pub tag: String,
    /// book, bookSection, article (journal), periodical, conference, report, website, film, other
    pub kind: String,
    /// "Last, First; Last, First"
    pub author: String,
    pub title: String,
    pub year: String,
    pub publisher: String,
    pub city: String,
    pub journal: String,
    pub volume: String,
    pub pages: String,
    pub url: String,
    /// More bibliography fields (Create Source › Show All Bibliography Fields).
    pub editor: String,
    pub edition: String,
    pub issue: String,
    pub month: String,
    pub day: String,
    /// Date accessed (websites).
    pub accessed: String,
    pub doi: String,
}

/// A custom document property (File › Info › Properties › Custom). Citation managers keep
/// their per-document preferences here (Zotero: `ZOTERO_PREF_1`, `ZOTERO_PREF_2`…).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CustomProp {
    pub name: String,
    /// OOXML variant type: `lpwstr`, `i4`, `r8`, `bool`, `filetime`…, or `raw` when `value` is
    /// the property's XML content kept verbatim (vectors, blobs).
    pub kind: String,
    pub value: String,
}

/// Document properties (File › Info).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CoreProps {
    pub title: String,
    pub subject: String,
    pub creator: String,
    pub keywords: String,
    pub description: String,
    pub category: String,
    pub last_modified_by: String,
    pub created: String,
    pub modified: String,
    pub revision: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub track_changes: bool,
    /// Record a tracked cut and paste of a sentence or paragraph as a move (Track moves); off
    /// when the file has `w:doNotTrackMoves` (ECMA-376 §17.15.1.29), and then it is an
    /// ordinary deletion and insertion.
    pub track_moves: bool,
    /// Default tab stop interval, points.
    pub default_tab: f32,
    pub even_odd_headers: bool,
    pub mirror_margins: bool,
    pub auto_hyphenation: bool,
    /// Hyphenation zone (`w:hyphenationZone`), points: a line is hyphenated only when breaking
    /// it at a word boundary would leave more than this empty at its end.
    pub hyphenation_zone: f32,
    /// Words in capitals may be hyphenated (`w:doNotHyphenateCaps` turns it off).
    pub hyphenate_caps: bool,
    /// Most consecutive lines that may end with a hyphen (`w:consecutiveHyphenLimit`); 0 = no limit.
    pub consecutive_hyphen_limit: u32,
    pub page_color: Option<Rgb>,
    /// Picture or texture page background (Design › Page Color › Fill Effects), drawn over the colour.
    pub page_picture: Option<PagePicture>,
    pub watermark: Option<Watermark>,
    /// Theme fonts (major = headings, minor = body).
    pub major_font: String,
    pub minor_font: String,
    /// Theme colours: dk1 lt1 dk2 lt2 accent1..6 hlink folHlink.
    pub theme_colors: Vec<Rgb>,
    pub theme_name: String,
    pub footnote_format: section::NumFormat,
    pub endnote_format: section::NumFormat,
    /// The document's footnote and endnote placement, start and restart (`w:footnotePr` /
    /// `w:endnotePr` in the settings); sections may override them.
    #[serde(skip_serializing_if = "section::NoteProps::is_empty")]
    pub footnote_pr: section::NoteProps,
    #[serde(skip_serializing_if = "section::NoteProps::is_empty")]
    pub endnote_pr: section::NoteProps,
    /// Restrict Editing, when enforced: `readOnly`, `comments`, `trackedChanges`, `forms`, or
    /// `none` when only formatting is restricted (`w:documentProtection w:edit`).
    pub protection: Option<String>,
    /// Restrict Editing limits formatting to the styles that aren't locked, and forbids direct
    /// formatting (`w:formatting="1"`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub protect_formatting: bool,
    /// The protection password's hash, as the file's `w:documentProtection` attributes
    /// (`w:algorithmName`, `w:hashValue`, `w:saltValue`, `w:spinCount`, or the older
    /// `w:cryptAlgorithmSid`, `w:hash`, `w:salt`…). Empty: no password.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub protect_hash: Vec<(String, String)>,
    /// Word compatibility mode the document is laid out in (`compatibilityMode`): 15 for Word
    /// 2013 and later, which places a table's border at the margin rather than its text and lets
    /// justified lines shrink their spaces to fit more text.
    pub compat_mode: u32,
    /// Fonts the document's font table classes as roman (serif, `w:family w:val="roman"`): Word
    /// draws one that isn't installed in a serif face.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roman_fonts: Vec<String>,
    /// `w:doNotUseHTMLParagraphAutoSpacing`: the space between two paragraphs is the first's space
    /// after plus the second's space before, not the larger of the two.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sum_paragraph_spacing: bool,
    /// `w:doNotExpandShiftReturn`: a justified line that ends in a manual line break keeps its
    /// natural spacing; by default Word stretches it to the margin like a wrapped line.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub do_not_expand_shift_return: bool,
    /// Keep contextual paragraph spacing inside legacy table cells (`allowSpaceOfSameStyleInTable`).
    /// Word ignores the layout override in compatibility mode 15, but the flag still round-trips.
    pub allow_space_of_same_style_in_table: bool,
    /// Drawing grid spacing in points, across and down (View › Gridlines); ECMA-376's default is 1/8 inch.
    pub grid_h: f32,
    pub grid_v: f32,
    /// Equation options (`m:mathPr`), when the document has them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub math: Option<math::MathProps>,
    /// Citation and bibliography style (References › Style) by name, when the document
    /// records one (OOXML's `b:Sources/@StyleName`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bib_style: Option<String>,
    /// The template the document is attached to (Tools › Templates and Add-ins): the target of
    /// `w:attachedTemplate`'s relationship, a path or `file:` URL as the file gives it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attached_template: Option<String>,
    /// Update the document's styles from the attached template when it is opened (`w:linkStyles`).
    pub link_styles: bool,
    /// The zoom the document was saved at (`w:zoom`), restored when it is opened; `None` for 100%
    /// without a fit mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom: Option<Zoom>,
    /// Remove personal information when saving (`w:removePersonalInformation`, ECMA-376
    /// §17.15.1.70): the saved file names comment and revision authors "Author" and leaves out
    /// the document's creator and last-modified-by. The open document keeps its real values.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub remove_personal_info: bool,
    /// Remove date and time from annotations when saving (`w:removeDateAndTime`, §17.15.1.69):
    /// comments and tracked changes are saved without `w:date`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub remove_date_time: bool,
}

/// A saved zoom: a percentage, or a fit mode (the percentage it last came to).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Zoom {
    /// 10–500.
    pub percent: u32,
    pub fit: Option<ZoomFit>,
}

/// View › Zoom's fit modes a document can be saved in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ZoomFit {
    OnePage,
    PageWidth,
    TextWidth,
}

/// Word's compatibility mode for documents that don't state one.
pub const LEGACY_COMPAT_MODE: u32 = 12;

/// The compatibility mode of documents created by Word 2013 and later, and by WordCraft.
pub const COMPAT_MODE_CURRENT: u32 = 15;

/// Word's hyphenation zone when a document doesn't set one (0.25"), points.
pub const DEFAULT_HYPHENATION_ZONE: f32 = 18.0;

/// ECMA-376 §17.15.1.44/45: 180 twentieths of a point when a document doesn't say.
pub const DEFAULT_GRID: f32 = 9.0;

/// A picture fill for the page background (`v:background/v:fill` with an image).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PagePicture {
    /// Key into [`Document::media`].
    pub media: String,
    /// `v:fill/@type` (`tile`, `frame`, `tile`...); tiled when `tile`, stretched to the page otherwise.
    pub fill_type: String,
    /// `o:title` of the fill (texture name), kept for round trip.
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Watermark {
    pub text: String,
    pub font: String,
    pub color: Rgb,
    pub diagonal: bool,
    pub semitransparent: bool,
    /// The shape's box (its `style` width and height, points), which the text is stretched to
    /// fill (`v:textpath fitshape`); `None`: the box Word's Design › Watermark gives
    /// ([`Watermark::frame`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<[f32; 2]>,
}

impl Default for Watermark {
    fn default() -> Self {
        Watermark {
            text: "DRAFT".into(),
            font: styles::BODY_FONT.into(),
            color: Rgb(0xC0, 0xC0, 0xC0),
            diagonal: true,
            semitransparent: true,
            size: None,
        }
    }
}

impl Watermark {
    /// The box the text fills on a page `page_w` × `page_h` (points): its own size, else half the
    /// page diagonal across (half the page width when not diagonal), as high as the text's length
    /// makes it. Centred on the margins and, when diagonal, turned 45° counterclockwise (Word's
    /// `rotation:315`).
    pub fn frame(&self, page_w: f32, page_h: f32) -> [f32; 2] {
        if let Some([w, h]) = self.size.filter(|[w, h]| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0) {
            return [w.min(4000.0), h.min(4000.0)];
        }
        let (pw, ph) = (wordcraft_geom::finite(page_w).clamp(72.0, 4000.0), wordcraft_geom::finite(page_h).clamp(72.0, 4000.0));
        let across = if self.diagonal { (pw * pw + ph * ph).sqrt() } else { pw };
        let width = (across * 0.5).clamp(36.0, 2000.0);
        let chars = self.text.chars().take(200).count().max(1) as f32;
        [width, (width / (0.55 * chars)).clamp(12.0, 400.0)]
    }
}

/// Our default theme colours.
pub const THEME_COLORS: [Rgb; 12] = [
    Rgb(0x00, 0x00, 0x00),
    Rgb(0xFF, 0xFF, 0xFF),
    Rgb(0x0E, 0x28, 0x41),
    Rgb(0xE8, 0xE8, 0xE8),
    Rgb(0x15, 0x60, 0x82),
    Rgb(0xE9, 0x71, 0x32),
    Rgb(0x19, 0x6B, 0x24),
    Rgb(0x0F, 0x9E, 0xD5),
    Rgb(0xA0, 0x2B, 0x93),
    Rgb(0x4E, 0xA7, 0x2E),
    Rgb(0x46, 0x78, 0x86),
    Rgb(0x96, 0x60, 0x7D),
];

impl Default for Settings {
    fn default() -> Self {
        Settings {
            track_changes: false,
            track_moves: true,
            default_tab: 36.0,
            even_odd_headers: false,
            mirror_margins: false,
            auto_hyphenation: false,
            hyphenation_zone: DEFAULT_HYPHENATION_ZONE,
            hyphenate_caps: true,
            consecutive_hyphen_limit: 0,
            page_color: None,
            page_picture: None,
            watermark: None,
            major_font: styles::HEADING_FONT.into(),
            minor_font: styles::BODY_FONT.into(),
            theme_colors: THEME_COLORS.to_vec(),
            theme_name: "Craft".into(),
            footnote_format: section::NumFormat::Decimal,
            endnote_format: section::NumFormat::LowerRoman,
            footnote_pr: section::NoteProps::default(),
            endnote_pr: section::NoteProps::default(),
            protection: None,
            protect_formatting: false,
            protect_hash: Vec::new(),
            compat_mode: COMPAT_MODE_CURRENT,
            roman_fonts: Vec::new(),
            sum_paragraph_spacing: false,
            do_not_expand_shift_return: false,
            allow_space_of_same_style_in_table: false,
            grid_h: DEFAULT_GRID,
            grid_v: DEFAULT_GRID,
            math: None,
            bib_style: None,
            attached_template: None,
            link_styles: false,
            zoom: None,
            remove_personal_info: false,
            remove_date_time: false,
        }
    }
}
