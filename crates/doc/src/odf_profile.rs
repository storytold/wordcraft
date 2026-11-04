//! How WordCraft fills in what the OpenDocument standard leaves to the application.
//!
//! Where ODF 1.3 defines a value (directly, or by referring to XSL for an `fo:` property), that
//! is the rule and nothing here changes it. Where it doesn't (ODF 1.3 Part 3 §16.2: "in all
//! remaining cases an implementation-dependent value is used"), each application chooses, and
//! documents from one application can look different in another. An [`OdfProfile`] is that
//! choice for every such case: Microsoft Word's style ([`OdfPreset::Word`], the default: as
//! WordCraft lays out Word documents), LibreOffice's style ([`OdfPreset::LibreOffice`]), or the
//! user's own ([`OdfPreset::Custom`]). `docs/odf-interpretation.md` lists every case.

use serde::{Deserialize, Serialize};

use crate::props::Rgb;

/// Whose choices fill in what the standard leaves open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OdfPreset {
    /// Microsoft Word's style: the behaviour WordCraft has for its own and Word documents.
    /// (`wordcraft`, its earlier id, is still taken.)
    #[default]
    #[serde(alias = "wordcraft")]
    Word,
    /// LibreOffice's (and Collabora's, which share its engine).
    LibreOffice,
    /// The user's own choice for each case ([`OdfProfile`]'s values).
    Custom,
}

impl OdfPreset {
    pub const ALL: [OdfPreset; 3] = [OdfPreset::Word, OdfPreset::LibreOffice, OdfPreset::Custom];

    pub fn id(self) -> &'static str {
        match self {
            OdfPreset::Word => "word",
            OdfPreset::LibreOffice => "libreoffice",
            OdfPreset::Custom => "custom",
        }
    }

    /// The preset named `id` (any case): its id, or `wordcraft` / `msword` for Word's.
    pub fn from_id(id: &str) -> Option<OdfPreset> {
        let id = id.trim();
        if ["wordcraft", "msword", "ms-word"].iter().any(|a| a.eq_ignore_ascii_case(id)) {
            return Some(OdfPreset::Word);
        }
        OdfPreset::ALL.into_iter().find(|p| p.id().eq_ignore_ascii_case(id))
    }

    /// The two styles a document can be shown in, as the user chooses between them (Custom
    /// aside).
    pub const STYLES: [OdfPreset; 2] = [OdfPreset::Word, OdfPreset::LibreOffice];

    /// Its name as the user sees it (File › Options, English; the UI translates it).
    pub fn label(self) -> &'static str {
        match self {
            OdfPreset::Word => "Microsoft Word style",
            OdfPreset::LibreOffice => "LibreOffice style",
            OdfPreset::Custom => "Custom",
        }
    }
}

/// The most a length in a profile may be, points (22 inches).
pub const MAX_LENGTH: f32 = 1584.0;

/// The choice for each case the standard leaves open (see the module docs). With a preset other
/// than [`OdfPreset::Custom`], the preset's values apply whatever these hold
/// ([`OdfProfile::effective`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OdfProfile {
    pub preset: OdfPreset,
    /// A page margin a page layout doesn't state, points. §20.206–20.209 refer to XSL's margins
    /// (§7.10), whose initial value is 0; whether that initial value comes with the reference
    /// the standard doesn't say (LibreOffice uses 2 cm).
    pub page_margin: f32,
    /// The height of a header or footer whose style states none (§20.212.2 `fo:min-height`
    /// has no default), points.
    pub header_min_height: f32,
    /// A header's or footer's minimum height takes in its spacing from the body (`fo:margin-*`
    /// of `style:header-footer-properties`; the standard doesn't say), rather than the spacing
    /// coming after it.
    pub header_height_includes_spacing: bool,
    /// What a header or footer whose style doesn't say does with its spacing when its content
    /// is taller than its height (§20.264 `style:dynamic-spacing` has no default): grows into it
    /// (`true`), or keeps it (`false`, both presets: with no height stated, growing into it would
    /// drop the spacing the file states).
    pub dynamic_spacing: bool,
    /// Paragraph tab stops are measured from the paragraph's indent rather than the margin when
    /// the document doesn't say (§19.512.3 `style:position` doesn't say from where; §19.861 `text:relative-tab-stop-position` says so only for indexes; LibreOffice's own
    /// `TabsRelativeToIndent` setting, in `settings.xml`, says so when present).
    pub tabs_from_indent: bool,
    /// The gap between columns whose `style:columns` states neither one nor each column
    /// (§19.233 `fo:column-gap` has no default), points.
    pub column_gap: f32,
    /// A drawing shape's fill colour when no style gives one (§20.119 `draw:fill-color` has no
    /// default).
    pub shape_fill: Rgb,
    /// A drawing shape's line colour when no style gives one (§20.408 `svg:stroke-color` has no default).
    pub shape_line: Rgb,
    /// A drawing shape's line width when no style gives one, and what a width of 0 draws at
    /// (§20.410 `svg:stroke-width` gives neither), points.
    pub hairline: f32,
    /// A page-anchored frame placed against a paragraph (a combination §20.398 and §20.299 don't
    /// define) is placed against the page (`true`), else against the page's text area.
    pub page_frame_paragraph_is_page: bool,
    /// An in-line frame placed against something an in-line frame can't be (a paragraph or a
    /// page, §20.398 Table 23) is placed against the baseline (`true`), else stands on it.
    pub inline_invalid_relation_is_baseline: bool,
    /// A line holding only in-line pictures is as tall as they are (`true`), else as tall as
    /// its text would be too (the standard defines no line-height algorithm).
    pub picture_lines_fit_pictures: bool,
    /// A columned section that doesn't say whether its columns are balanced (§20.428
    /// `text:dont-balance-text-columns` has no default, and its value descriptions contradict
    /// its name) ends with balanced columns at the end of the document too (`true`), else only
    /// when text follows on the same page, as in Word.
    pub balance_section_columns: bool,
    /// The space between two paragraphs is the first's space after plus the second's space
    /// before (`true`), else the larger of the two, when the document doesn't say (the
    /// standard doesn't; LibreOffice's own `AddParaTableSpacing` setting does when present).
    pub sum_paragraph_spacing: bool,
    /// Widow and orphan control when no style states `fo:widows` or `fo:orphans` (§20.312,
    /// §20.244 refer to XSL's, whose initial value is 2 lines; whether that initial value comes
    /// with the reference the standard doesn't say, and LibreOffice reads none stated as none).
    pub widow_control: bool,
}

impl Default for OdfProfile {
    fn default() -> Self {
        OdfProfile::word()
    }
}

impl OdfProfile {
    /// Microsoft Word's style: as WordCraft lays out its own and Word documents.
    pub fn word() -> OdfProfile {
        OdfProfile {
            preset: OdfPreset::Word,
            page_margin: 0.0,
            header_min_height: 0.0,
            header_height_includes_spacing: false,
            dynamic_spacing: false,
            tabs_from_indent: false,
            column_gap: 36.0,
            shape_fill: Rgb(0x15, 0x60, 0x82),
            shape_line: Rgb(0x0E, 0x40, 0x5A),
            hairline: 0.25,
            page_frame_paragraph_is_page: false,
            inline_invalid_relation_is_baseline: false,
            picture_lines_fit_pictures: false,
            balance_section_columns: false,
            sum_paragraph_spacing: false,
            widow_control: true,
        }
    }

    /// LibreOffice's choices, measured in LibreOffice 24 where it doesn't document them.
    pub fn libreoffice() -> OdfProfile {
        OdfProfile {
            preset: OdfPreset::LibreOffice,
            page_margin: 2.0 * 72.0 / 2.54,
            header_min_height: 0.5 * 72.0 / 2.54,
            header_height_includes_spacing: true,
            dynamic_spacing: false,
            tabs_from_indent: true,
            column_gap: 0.0,
            shape_fill: Rgb(0x72, 0x9F, 0xCF),
            shape_line: Rgb(0x34, 0x65, 0xA4),
            hairline: 0.5,
            page_frame_paragraph_is_page: true,
            inline_invalid_relation_is_baseline: true,
            picture_lines_fit_pictures: true,
            balance_section_columns: true,
            sum_paragraph_spacing: true,
            widow_control: false,
        }
    }

    /// A preset's values (a custom profile starts from Word's style).
    pub fn of(preset: OdfPreset) -> OdfProfile {
        match preset {
            OdfPreset::Word | OdfPreset::Custom => OdfProfile { preset, ..OdfProfile::word() },
            OdfPreset::LibreOffice => OdfProfile::libreoffice(),
        }
    }

    /// The values that apply: a preset's own, or, for a custom profile, these (lengths made
    /// finite and bounded).
    pub fn effective(&self) -> OdfProfile {
        let length = |v: f32, least: f32| if v.is_finite() { v.clamp(least, MAX_LENGTH) } else { least };
        match self.preset {
            OdfPreset::Custom => OdfProfile {
                page_margin: length(self.page_margin, 0.0),
                header_min_height: length(self.header_min_height, 0.0),
                column_gap: length(self.column_gap, 0.0),
                hairline: length(self.hairline, 0.05),
                ..self.clone()
            },
            p => OdfProfile::of(p),
        }
    }

    /// What the layout needs to know of it for a document opened from ODF.
    pub fn layout(&self) -> OdfLayout {
        let e = self.effective();
        OdfLayout { picture_lines_fit_pictures: e.picture_lines_fit_pictures, tabs_from_indent: e.tabs_from_indent }
    }
}

/// A document opened from ODF, for the layout: rules of the standard that differ from Word's
/// (a paragraph's padding takes room on a side without a border, XSL's box model) apply, and
/// the profile's choices that are made as the document is laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OdfLayout {
    /// See [`OdfProfile::picture_lines_fit_pictures`].
    pub picture_lines_fit_pictures: bool,
    /// Tab stops are measured from the paragraph's left indent, default ones too (LibreOffice's
    /// `TabsRelativeToIndent`, else [`OdfProfile::tabs_from_indent`]). The document's explicit
    /// stops are held from the margin either way; the default ones are counted from the indent.
    pub tabs_from_indent: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_custom_values() {
        assert_eq!(OdfProfile::default().preset, OdfPreset::Word);
        assert_eq!(OdfPreset::from_id(" LibreOffice "), Some(OdfPreset::LibreOffice));
        assert_eq!(OdfPreset::from_id("google"), None);
        // Word's style by its id, and by the id it had before.
        for id in ["word", "Word", "wordcraft", "MSWord", "ms-word"] {
            assert_eq!(OdfPreset::from_id(id), Some(OdfPreset::Word), "{id}");
        }
        assert_eq!(serde_json::from_value::<OdfPreset>(serde_json::json!("wordcraft")).unwrap(), OdfPreset::Word);
        assert_eq!(serde_json::from_value::<OdfPreset>(serde_json::json!("word")).unwrap(), OdfPreset::Word);
        assert_eq!(OdfPreset::STYLES.map(OdfPreset::label), ["Microsoft Word style", "LibreOffice style"]);
        // Saved and taken by `file.options` as its id.
        for p in OdfPreset::ALL {
            assert_eq!(serde_json::to_value(p).unwrap(), serde_json::json!(p.id()));
        }
        // A preset's values apply whatever the profile holds.
        let lo = OdfProfile { column_gap: 99.0, ..OdfProfile::libreoffice() };
        assert_eq!(lo.effective(), OdfProfile::libreoffice());
        // A custom profile's own, bounded.
        let custom = OdfProfile { preset: OdfPreset::Custom, column_gap: f32::NAN, header_min_height: 1e9, ..OdfProfile::word() };
        let e = custom.effective();
        assert_eq!((e.column_gap, e.header_min_height), (0.0, MAX_LENGTH));
        assert!(OdfProfile::libreoffice().layout().picture_lines_fit_pictures);
        assert!(!OdfProfile::word().layout().picture_lines_fit_pictures);
    }
}
