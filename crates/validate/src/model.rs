//! ECMA-376 content-model tables for the grammar checks.
//!
//! GENERATED FILE: do not edit by hand. Regenerate from `crates/validate/model/` with
//!
//! ```text
//! python3 generate_model.py --model-dir . --xsd-dir <strict-xsd-dir> --out ../src/model.rs
//! ```
//!
//! The chain from the ECMA-376 schema archives to this file is documented in
//! `crates/validate/model/README.md`. Inputs (SHA-256):
//!
//! - `content_model_v2.json`: `f36a42e324bae4cf4d6ce03ebbfb53e2e5729d76ae5f4929332a272d02545978`
//! - `content_model_dml_v2.json`: `9a3e58c9bc70d3b2fa9718a02666b51d2f4822e66ff61e212cdea63e3e569549`
//! - `content_model_v2_transitional.json`: `92c433577c914785908f42553718cc41b4f12161125596383ab6836ac3c54fbf`
//! - `content_model_dml_v2_transitional.json`: `81290399fb8f89aa287d275172b7214378449a54fd3783e42a8f899958dae127`
//! - `wml.xsd (Strict)`: `44fc2233c5df2cfbe0b3b50e7024bd8412ec99dfe7176313ef58845fdec5498d`
//!
//! Counts: 747 Strict types (380 WML + 367 DML); 20 Transitional overrides (19 WML + 1 DML);
//! 1335 elements (794 WML + 541 DML), 61 with more than one type variant; 0 ambiguous (parent type, element)
//! pairs left to fallback selection; 63 `ST_NumberFormat` values.
//!
//! Group contexts that no type flattens contribute no parent types. Where such a group is a
//! type's whole content, the extractor records that type as empty: these tables do not carry
//! its content, so a consumer must not read its empty child list as "no children allowed".
//! - `a:EG_Effect`: `a:CT_EffectContainer`
//! - `a:EG_Media`: referenced by no in-scope complex type
//! - `a:EG_TextGeometry`: referenced by no in-scope complex type
//! - `w:EG_RubyContent`: `w:CT_RubyContent`

use Compositor::{All, Choice, Sequence};

/// How many times a child may occur in its slot: a finite bound or `unbounded`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaxOccurs {
    /// At most this many occurrences.
    Limited(u16),
    /// No upper bound (`maxOccurs="unbounded"`).
    Unbounded,
}

/// The XSD compositor a child slot sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Compositor {
    /// `xsd:sequence`: slots in a fixed order.
    Sequence,
    /// `xsd:choice`: one of the slots sharing a `choice_group`.
    Choice,
    /// `xsd:all`: each slot at most once, any order.
    All,
}

/// One child slot in a type's flattened content model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChildSpec {
    /// Prefixed element name, e.g. `w:tblPr`; `<any>` for an `xsd:any` wildcard.
    pub name: &'static str,
    /// Minimum occurrences, with the enclosing particles' minOccurs multiplied in.
    pub min: u16,
    /// Maximum occurrences, with the enclosing particles' maxOccurs multiplied in.
    pub max: MaxOccurs,
    /// The compositor this slot sits in.
    pub compositor: Compositor,
    /// The choice group id shared by the members of one `xsd:choice`, if any.
    pub choice_group: Option<&'static str>,
    /// Index of this slot in the type's flattened child list.
    pub position: u16,
}

/// The content model of one complex type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TypeModel {
    /// Namespace-qualified type key, e.g. `w:CT_Tc`, `a:CT_FontCollection`.
    pub key: &'static str,
    /// The type's child slots in flattened schema order.
    pub children: &'static [ChildSpec],
}

/// One type an element name is declared with, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Variant {
    /// The type key of this declaration (may name a simple or out-of-scope type
    /// that has no [`TypeModel`]; such an element is unmodelled).
    pub type_key: &'static str,
    /// Parent type keys this variant is selected under (group contexts expanded).
    pub contexts: &'static [&'static str],
}

/// An element name and every type it is declared with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ElementModel {
    /// Prefixed element name, e.g. `w:sectPr`.
    pub name: &'static str,
    /// The type key used in the most declaration contexts.
    pub canonical: &'static str,
    /// Every declared type, sorted by type key.
    pub variants: &'static [Variant],
}

/// Builds one child slot; a `max` of `None` means unbounded. Keeps table rows short.
const fn ch(name: &'static str, min: u16, max: Option<u16>, compositor: Compositor, group: Option<&'static str>, position: u16) -> ChildSpec {
    let max = match max {
        Some(n) => MaxOccurs::Limited(n),
        None => MaxOccurs::Unbounded,
    };
    ChildSpec { name, min, max, compositor, choice_group: group, position }
}

/// Builds one type row.
const fn ty(key: &'static str, children: &'static [ChildSpec]) -> TypeModel {
    TypeModel { key, children }
}

/// Builds one element variant.
const fn va(type_key: &'static str, contexts: &'static [&'static str]) -> Variant {
    Variant { type_key, contexts }
}

/// Builds one element row.
const fn el(name: &'static str, canonical: &'static str, variants: &'static [Variant]) -> ElementModel {
    ElementModel { name, canonical, variants }
}

/// The Strict model of a type, by qualified key (binary search over [`TYPES`]).
pub(crate) fn type_model(key: &str) -> Option<&'static TypeModel> {
    TYPES.binary_search_by(|t| t.key.cmp(key)).ok().and_then(|i| TYPES.get(i))
}

/// The Transitional model of a type, only where it differs from the Strict one.
pub(crate) fn transitional_type_model(key: &str) -> Option<&'static TypeModel> {
    TRANSITIONAL_TYPES.binary_search_by(|t| t.key.cmp(key)).ok().and_then(|i| TRANSITIONAL_TYPES.get(i))
}

/// The row for a prefixed element name (binary search over [`ELEMENTS`]).
pub(crate) fn element_model(prefixed_name: &str) -> Option<&'static ElementModel> {
    ELEMENTS.binary_search_by(|e| e.name.cmp(prefixed_name)).ok().and_then(|i| ELEMENTS.get(i))
}

/// Every Strict complex type of the WML and DML profiles, sorted by key.
#[rustfmt::skip]
pub(crate) static TYPES: &[TypeModel] = &[
    ty("a:CT_AdjPoint2D", &[]),
    ty("a:CT_AdjustHandleList", &[
        ch("a:ahXY", 0, None, Choice, Some("choice1"), 0),
        ch("a:ahPolar", 0, None, Choice, Some("choice1"), 1),
    ]),
    ty("a:CT_AlphaBiLevelEffect", &[]),
    ty("a:CT_AlphaCeilingEffect", &[]),
    ty("a:CT_AlphaFloorEffect", &[]),
    ty("a:CT_AlphaInverseEffect", &[
        ch("a:scrgbClr", 0, Some(1), Choice, Some("choice1"), 0),
        ch("a:srgbClr", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:hslClr", 0, Some(1), Choice, Some("choice1"), 2),
        ch("a:sysClr", 0, Some(1), Choice, Some("choice1"), 3),
        ch("a:schemeClr", 0, Some(1), Choice, Some("choice1"), 4),
        ch("a:prstClr", 0, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_AlphaModulateEffect", &[
        ch("a:cont", 1, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_AlphaModulateFixedEffect", &[]),
    ty("a:CT_AlphaOutsetEffect", &[]),
    ty("a:CT_AlphaReplaceEffect", &[]),
    ty("a:CT_Angle", &[]),
    ty("a:CT_AnimationChartBuildProperties", &[]),
    ty("a:CT_AnimationChartElement", &[]),
    ty("a:CT_AnimationDgmBuildProperties", &[]),
    ty("a:CT_AnimationDgmElement", &[]),
    ty("a:CT_AnimationElementChoice", &[
        ch("a:dgm", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:chart", 1, Some(1), Choice, Some("choice1"), 1),
    ]),
    ty("a:CT_AnimationGraphicalObjectBuildProperties", &[
        ch("a:bldDgm", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:bldChart", 1, Some(1), Choice, Some("choice1"), 1),
    ]),
    ty("a:CT_AudioCD", &[
        ch("a:st", 1, Some(1), Sequence, None, 0),
        ch("a:end", 1, Some(1), Sequence, None, 1),
        ch("a:extLst", 0, Some(1), Sequence, None, 2),
    ]),
    ty("a:CT_AudioCDTime", &[]),
    ty("a:CT_AudioFile", &[
        ch("a:extLst", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_Backdrop", &[
        ch("a:anchor", 1, Some(1), Sequence, None, 0),
        ch("a:norm", 1, Some(1), Sequence, None, 1),
        ch("a:up", 1, Some(1), Sequence, None, 2),
        ch("a:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("a:CT_BackgroundFillStyleList", &[
        ch("a:noFill", 3, None, Choice, Some("choice1"), 0),
        ch("a:solidFill", 3, None, Choice, Some("choice1"), 1),
        ch("a:gradFill", 3, None, Choice, Some("choice1"), 2),
        ch("a:blipFill", 3, None, Choice, Some("choice1"), 3),
        ch("a:pattFill", 3, None, Choice, Some("choice1"), 4),
        ch("a:grpFill", 3, None, Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_BackgroundFormatting", &[
        ch("a:noFill", 0, Some(1), Choice, Some("choice1"), 0),
        ch("a:solidFill", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:gradFill", 0, Some(1), Choice, Some("choice1"), 2),
        ch("a:blipFill", 0, Some(1), Choice, Some("choice1"), 3),
        ch("a:pattFill", 0, Some(1), Choice, Some("choice1"), 4),
        ch("a:grpFill", 0, Some(1), Choice, Some("choice1"), 5),
        ch("a:effectLst", 0, Some(1), Choice, Some("choice2"), 6),
        ch("a:effectDag", 0, Some(1), Choice, Some("choice2"), 7),
    ]),
    ty("a:CT_BaseStyles", &[
        ch("a:clrScheme", 1, Some(1), Sequence, None, 0),
        ch("a:fontScheme", 1, Some(1), Sequence, None, 1),
        ch("a:fmtScheme", 1, Some(1), Sequence, None, 2),
        ch("a:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("a:CT_BaseStylesOverride", &[
        ch("a:clrScheme", 0, Some(1), Sequence, None, 0),
        ch("a:fontScheme", 0, Some(1), Sequence, None, 1),
        ch("a:fmtScheme", 0, Some(1), Sequence, None, 2),
    ]),
    ty("a:CT_Bevel", &[]),
    ty("a:CT_BiLevelEffect", &[]),
    ty("a:CT_BlendEffect", &[
        ch("a:cont", 1, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_Blip", &[
        ch("a:alphaBiLevel", 0, None, Choice, Some("choice1"), 0),
        ch("a:alphaCeiling", 0, None, Choice, Some("choice1"), 1),
        ch("a:alphaFloor", 0, None, Choice, Some("choice1"), 2),
        ch("a:alphaInv", 0, None, Choice, Some("choice1"), 3),
        ch("a:alphaMod", 0, None, Choice, Some("choice1"), 4),
        ch("a:alphaModFix", 0, None, Choice, Some("choice1"), 5),
        ch("a:alphaRepl", 0, None, Choice, Some("choice1"), 6),
        ch("a:biLevel", 0, None, Choice, Some("choice1"), 7),
        ch("a:blur", 0, None, Choice, Some("choice1"), 8),
        ch("a:clrChange", 0, None, Choice, Some("choice1"), 9),
        ch("a:clrRepl", 0, None, Choice, Some("choice1"), 10),
        ch("a:duotone", 0, None, Choice, Some("choice1"), 11),
        ch("a:fillOverlay", 0, None, Choice, Some("choice1"), 12),
        ch("a:grayscl", 0, None, Choice, Some("choice1"), 13),
        ch("a:hsl", 0, None, Choice, Some("choice1"), 14),
        ch("a:lum", 0, None, Choice, Some("choice1"), 15),
        ch("a:tint", 0, None, Choice, Some("choice1"), 16),
        ch("a:extLst", 0, Some(1), Sequence, None, 17),
    ]),
    ty("a:CT_BlipFillProperties", &[
        ch("a:blip", 0, Some(1), Sequence, None, 0),
        ch("a:srcRect", 0, Some(1), Sequence, None, 1),
        ch("a:tile", 0, Some(1), Choice, Some("choice1"), 2),
        ch("a:stretch", 0, Some(1), Choice, Some("choice1"), 3),
    ]),
    ty("a:CT_BlurEffect", &[]),
    ty("a:CT_Boolean", &[]),
    ty("a:CT_Camera", &[
        ch("a:rot", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_Cell3D", &[
        ch("a:bevel", 1, Some(1), Sequence, None, 0),
        ch("a:lightRig", 0, Some(1), Sequence, None, 1),
        ch("a:extLst", 0, Some(1), Sequence, None, 2),
    ]),
    ty("a:CT_ClipboardStyleSheet", &[
        ch("a:themeElements", 1, Some(1), Sequence, None, 0),
        ch("a:clrMap", 1, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_Color", &[
        ch("a:scrgbClr", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:srgbClr", 1, Some(1), Choice, Some("choice1"), 1),
        ch("a:hslClr", 1, Some(1), Choice, Some("choice1"), 2),
        ch("a:sysClr", 1, Some(1), Choice, Some("choice1"), 3),
        ch("a:schemeClr", 1, Some(1), Choice, Some("choice1"), 4),
        ch("a:prstClr", 1, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_ColorChangeEffect", &[
        ch("a:clrFrom", 1, Some(1), Sequence, None, 0),
        ch("a:clrTo", 1, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_ColorMRU", &[
        ch("a:scrgbClr", 0, None, Choice, Some("choice1"), 0),
        ch("a:srgbClr", 0, None, Choice, Some("choice1"), 1),
        ch("a:hslClr", 0, None, Choice, Some("choice1"), 2),
        ch("a:sysClr", 0, None, Choice, Some("choice1"), 3),
        ch("a:schemeClr", 0, None, Choice, Some("choice1"), 4),
        ch("a:prstClr", 0, None, Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_ColorMapping", &[
        ch("a:extLst", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_ColorMappingOverride", &[
        ch("a:masterClrMapping", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:overrideClrMapping", 1, Some(1), Choice, Some("choice1"), 1),
    ]),
    ty("a:CT_ColorReplaceEffect", &[
        ch("a:scrgbClr", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:srgbClr", 1, Some(1), Choice, Some("choice1"), 1),
        ch("a:hslClr", 1, Some(1), Choice, Some("choice1"), 2),
        ch("a:sysClr", 1, Some(1), Choice, Some("choice1"), 3),
        ch("a:schemeClr", 1, Some(1), Choice, Some("choice1"), 4),
        ch("a:prstClr", 1, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_ColorScheme", &[
        ch("a:dk1", 1, Some(1), Sequence, None, 0),
        ch("a:lt1", 1, Some(1), Sequence, None, 1),
        ch("a:dk2", 1, Some(1), Sequence, None, 2),
        ch("a:lt2", 1, Some(1), Sequence, None, 3),
        ch("a:accent1", 1, Some(1), Sequence, None, 4),
        ch("a:accent2", 1, Some(1), Sequence, None, 5),
        ch("a:accent3", 1, Some(1), Sequence, None, 6),
        ch("a:accent4", 1, Some(1), Sequence, None, 7),
        ch("a:accent5", 1, Some(1), Sequence, None, 8),
        ch("a:accent6", 1, Some(1), Sequence, None, 9),
        ch("a:hlink", 1, Some(1), Sequence, None, 10),
        ch("a:folHlink", 1, Some(1), Sequence, None, 11),
        ch("a:extLst", 0, Some(1), Sequence, None, 12),
    ]),
    ty("a:CT_ColorSchemeAndMapping", &[
        ch("a:clrScheme", 1, Some(1), Sequence, None, 0),
        ch("a:clrMap", 0, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_ColorSchemeList", &[
        ch("a:extraClrScheme", 0, None, Sequence, None, 0),
    ]),
    ty("a:CT_ComplementTransform", &[]),
    ty("a:CT_Connection", &[]),
    ty("a:CT_ConnectionSite", &[
        ch("a:pos", 1, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_ConnectionSiteList", &[
        ch("a:cxn", 0, None, Sequence, None, 0),
    ]),
    ty("a:CT_ConnectorLocking", &[
        ch("a:extLst", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_ContentPartLocking", &[
        ch("a:extLst", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_CustomColor", &[
        ch("a:scrgbClr", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:srgbClr", 1, Some(1), Choice, Some("choice1"), 1),
        ch("a:hslClr", 1, Some(1), Choice, Some("choice1"), 2),
        ch("a:sysClr", 1, Some(1), Choice, Some("choice1"), 3),
        ch("a:schemeClr", 1, Some(1), Choice, Some("choice1"), 4),
        ch("a:prstClr", 1, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_CustomColorList", &[
        ch("a:custClr", 0, None, Sequence, None, 0),
    ]),
    ty("a:CT_CustomGeometry2D", &[
        ch("a:avLst", 0, Some(1), Sequence, None, 0),
        ch("a:gdLst", 0, Some(1), Sequence, None, 1),
        ch("a:ahLst", 0, Some(1), Sequence, None, 2),
        ch("a:cxnLst", 0, Some(1), Sequence, None, 3),
        ch("a:rect", 0, Some(1), Sequence, None, 4),
        ch("a:pathLst", 1, Some(1), Sequence, None, 5),
    ]),
    ty("a:CT_DashStop", &[]),
    ty("a:CT_DashStopList", &[
        ch("a:ds", 0, None, Sequence, None, 0),
    ]),
    ty("a:CT_DefaultShapeDefinition", &[
        ch("a:spPr", 1, Some(1), Sequence, None, 0),
        ch("a:bodyPr", 1, Some(1), Sequence, None, 1),
        ch("a:lstStyle", 1, Some(1), Sequence, None, 2),
        ch("a:style", 0, Some(1), Sequence, None, 3),
        ch("a:extLst", 0, Some(1), Sequence, None, 4),
    ]),
    ty("a:CT_DuotoneEffect", &[
        ch("a:scrgbClr", 2, Some(2), Choice, Some("choice1"), 0),
        ch("a:srgbClr", 2, Some(2), Choice, Some("choice1"), 1),
        ch("a:hslClr", 2, Some(2), Choice, Some("choice1"), 2),
        ch("a:sysClr", 2, Some(2), Choice, Some("choice1"), 3),
        ch("a:schemeClr", 2, Some(2), Choice, Some("choice1"), 4),
        ch("a:prstClr", 2, Some(2), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_EffectContainer", &[]),
    ty("a:CT_EffectList", &[
        ch("a:blur", 0, Some(1), Sequence, None, 0),
        ch("a:fillOverlay", 0, Some(1), Sequence, None, 1),
        ch("a:glow", 0, Some(1), Sequence, None, 2),
        ch("a:innerShdw", 0, Some(1), Sequence, None, 3),
        ch("a:outerShdw", 0, Some(1), Sequence, None, 4),
        ch("a:prstShdw", 0, Some(1), Sequence, None, 5),
        ch("a:reflection", 0, Some(1), Sequence, None, 6),
        ch("a:softEdge", 0, Some(1), Sequence, None, 7),
    ]),
    ty("a:CT_EffectProperties", &[
        ch("a:effectLst", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:effectDag", 1, Some(1), Choice, Some("choice1"), 1),
    ]),
    ty("a:CT_EffectReference", &[]),
    ty("a:CT_EffectStyleItem", &[
        ch("a:effectLst", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:effectDag", 1, Some(1), Choice, Some("choice1"), 1),
        ch("a:scene3d", 0, Some(1), Sequence, None, 2),
        ch("a:sp3d", 0, Some(1), Sequence, None, 3),
    ]),
    ty("a:CT_EffectStyleList", &[
        ch("a:effectStyle", 3, None, Sequence, None, 0),
    ]),
    ty("a:CT_EmbeddedWAVAudioFile", &[]),
    ty("a:CT_EmptyElement", &[]),
    ty("a:CT_FillEffect", &[
        ch("a:noFill", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:solidFill", 1, Some(1), Choice, Some("choice1"), 1),
        ch("a:gradFill", 1, Some(1), Choice, Some("choice1"), 2),
        ch("a:blipFill", 1, Some(1), Choice, Some("choice1"), 3),
        ch("a:pattFill", 1, Some(1), Choice, Some("choice1"), 4),
        ch("a:grpFill", 1, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_FillOverlayEffect", &[
        ch("a:noFill", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:solidFill", 1, Some(1), Choice, Some("choice1"), 1),
        ch("a:gradFill", 1, Some(1), Choice, Some("choice1"), 2),
        ch("a:blipFill", 1, Some(1), Choice, Some("choice1"), 3),
        ch("a:pattFill", 1, Some(1), Choice, Some("choice1"), 4),
        ch("a:grpFill", 1, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_FillProperties", &[
        ch("a:noFill", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:solidFill", 1, Some(1), Choice, Some("choice1"), 1),
        ch("a:gradFill", 1, Some(1), Choice, Some("choice1"), 2),
        ch("a:blipFill", 1, Some(1), Choice, Some("choice1"), 3),
        ch("a:pattFill", 1, Some(1), Choice, Some("choice1"), 4),
        ch("a:grpFill", 1, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_FillStyleList", &[
        ch("a:noFill", 3, None, Choice, Some("choice1"), 0),
        ch("a:solidFill", 3, None, Choice, Some("choice1"), 1),
        ch("a:gradFill", 3, None, Choice, Some("choice1"), 2),
        ch("a:blipFill", 3, None, Choice, Some("choice1"), 3),
        ch("a:pattFill", 3, None, Choice, Some("choice1"), 4),
        ch("a:grpFill", 3, None, Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_FixedPercentage", &[]),
    ty("a:CT_FlatText", &[]),
    ty("a:CT_FontCollection", &[
        ch("a:latin", 1, Some(1), Sequence, None, 0),
        ch("a:ea", 1, Some(1), Sequence, None, 1),
        ch("a:cs", 1, Some(1), Sequence, None, 2),
        ch("a:font", 0, None, Sequence, None, 3),
        ch("a:extLst", 0, Some(1), Sequence, None, 4),
    ]),
    ty("a:CT_FontReference", &[
        ch("a:scrgbClr", 0, Some(1), Choice, Some("choice1"), 0),
        ch("a:srgbClr", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:hslClr", 0, Some(1), Choice, Some("choice1"), 2),
        ch("a:sysClr", 0, Some(1), Choice, Some("choice1"), 3),
        ch("a:schemeClr", 0, Some(1), Choice, Some("choice1"), 4),
        ch("a:prstClr", 0, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_FontScheme", &[
        ch("a:majorFont", 1, Some(1), Sequence, None, 0),
        ch("a:minorFont", 1, Some(1), Sequence, None, 1),
        ch("a:extLst", 0, Some(1), Sequence, None, 2),
    ]),
    ty("a:CT_GammaTransform", &[]),
    ty("a:CT_GeomGuide", &[]),
    ty("a:CT_GeomGuideList", &[
        ch("a:gd", 0, None, Sequence, None, 0),
    ]),
    ty("a:CT_GeomRect", &[]),
    ty("a:CT_GlowEffect", &[
        ch("a:scrgbClr", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:srgbClr", 1, Some(1), Choice, Some("choice1"), 1),
        ch("a:hslClr", 1, Some(1), Choice, Some("choice1"), 2),
        ch("a:sysClr", 1, Some(1), Choice, Some("choice1"), 3),
        ch("a:schemeClr", 1, Some(1), Choice, Some("choice1"), 4),
        ch("a:prstClr", 1, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_GradientFillProperties", &[
        ch("a:gsLst", 0, Some(1), Sequence, None, 0),
        ch("a:lin", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:path", 0, Some(1), Choice, Some("choice1"), 2),
        ch("a:tileRect", 0, Some(1), Sequence, None, 3),
    ]),
    ty("a:CT_GradientStop", &[
        ch("a:scrgbClr", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:srgbClr", 1, Some(1), Choice, Some("choice1"), 1),
        ch("a:hslClr", 1, Some(1), Choice, Some("choice1"), 2),
        ch("a:sysClr", 1, Some(1), Choice, Some("choice1"), 3),
        ch("a:schemeClr", 1, Some(1), Choice, Some("choice1"), 4),
        ch("a:prstClr", 1, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_GradientStopList", &[
        ch("a:gs", 2, None, Sequence, None, 0),
    ]),
    ty("a:CT_GraphicalObject", &[
        ch("a:graphicData", 1, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_GraphicalObjectData", &[
        ch("<any>", 0, None, Sequence, None, 0),
    ]),
    ty("a:CT_GraphicalObjectFrameLocking", &[
        ch("a:extLst", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_GrayscaleEffect", &[]),
    ty("a:CT_GrayscaleTransform", &[]),
    ty("a:CT_GroupFillProperties", &[]),
    ty("a:CT_GroupLocking", &[
        ch("a:extLst", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_GroupShapeProperties", &[
        ch("a:xfrm", 0, Some(1), Sequence, None, 0),
        ch("a:noFill", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:solidFill", 0, Some(1), Choice, Some("choice1"), 2),
        ch("a:gradFill", 0, Some(1), Choice, Some("choice1"), 3),
        ch("a:blipFill", 0, Some(1), Choice, Some("choice1"), 4),
        ch("a:pattFill", 0, Some(1), Choice, Some("choice1"), 5),
        ch("a:grpFill", 0, Some(1), Choice, Some("choice1"), 6),
        ch("a:effectLst", 0, Some(1), Choice, Some("choice2"), 7),
        ch("a:effectDag", 0, Some(1), Choice, Some("choice2"), 8),
        ch("a:scene3d", 0, Some(1), Sequence, None, 9),
        ch("a:extLst", 0, Some(1), Sequence, None, 10),
    ]),
    ty("a:CT_GroupTransform2D", &[
        ch("a:off", 0, Some(1), Sequence, None, 0),
        ch("a:ext", 0, Some(1), Sequence, None, 1),
        ch("a:chOff", 0, Some(1), Sequence, None, 2),
        ch("a:chExt", 0, Some(1), Sequence, None, 3),
    ]),
    ty("a:CT_GvmlConnector", &[
        ch("a:nvCxnSpPr", 1, Some(1), Sequence, None, 0),
        ch("a:spPr", 1, Some(1), Sequence, None, 1),
        ch("a:style", 0, Some(1), Sequence, None, 2),
        ch("a:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("a:CT_GvmlConnectorNonVisual", &[
        ch("a:cNvPr", 1, Some(1), Sequence, None, 0),
        ch("a:cNvCxnSpPr", 1, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_GvmlGraphicFrameNonVisual", &[
        ch("a:cNvPr", 1, Some(1), Sequence, None, 0),
        ch("a:cNvGraphicFramePr", 1, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_GvmlGraphicalObjectFrame", &[
        ch("a:nvGraphicFramePr", 1, Some(1), Sequence, None, 0),
        ch("a:graphic", 1, Some(1), Sequence, None, 1),
        ch("a:xfrm", 1, Some(1), Sequence, None, 2),
        ch("a:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("a:CT_GvmlGroupShape", &[
        ch("a:nvGrpSpPr", 1, Some(1), Sequence, None, 0),
        ch("a:grpSpPr", 1, Some(1), Sequence, None, 1),
        ch("a:txSp", 0, None, Choice, Some("choice1"), 2),
        ch("a:sp", 0, None, Choice, Some("choice1"), 3),
        ch("a:cxnSp", 0, None, Choice, Some("choice1"), 4),
        ch("a:pic", 0, None, Choice, Some("choice1"), 5),
        ch("a:graphicFrame", 0, None, Choice, Some("choice1"), 6),
        ch("a:grpSp", 0, None, Choice, Some("choice1"), 7),
        ch("a:extLst", 0, Some(1), Sequence, None, 8),
    ]),
    ty("a:CT_GvmlGroupShapeNonVisual", &[
        ch("a:cNvPr", 1, Some(1), Sequence, None, 0),
        ch("a:cNvGrpSpPr", 1, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_GvmlPicture", &[
        ch("a:nvPicPr", 1, Some(1), Sequence, None, 0),
        ch("a:blipFill", 1, Some(1), Sequence, None, 1),
        ch("a:spPr", 1, Some(1), Sequence, None, 2),
        ch("a:style", 0, Some(1), Sequence, None, 3),
        ch("a:extLst", 0, Some(1), Sequence, None, 4),
    ]),
    ty("a:CT_GvmlPictureNonVisual", &[
        ch("a:cNvPr", 1, Some(1), Sequence, None, 0),
        ch("a:cNvPicPr", 1, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_GvmlShape", &[
        ch("a:nvSpPr", 1, Some(1), Sequence, None, 0),
        ch("a:spPr", 1, Some(1), Sequence, None, 1),
        ch("a:txSp", 0, Some(1), Sequence, None, 2),
        ch("a:style", 0, Some(1), Sequence, None, 3),
        ch("a:extLst", 0, Some(1), Sequence, None, 4),
    ]),
    ty("a:CT_GvmlShapeNonVisual", &[
        ch("a:cNvPr", 1, Some(1), Sequence, None, 0),
        ch("a:cNvSpPr", 1, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_GvmlTextShape", &[
        ch("a:txBody", 1, Some(1), Sequence, None, 0),
        ch("a:useSpRect", 1, Some(1), Choice, Some("choice1"), 1),
        ch("a:xfrm", 1, Some(1), Choice, Some("choice1"), 2),
        ch("a:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("a:CT_GvmlUseShapeRectangle", &[]),
    ty("a:CT_HSLEffect", &[]),
    ty("a:CT_Headers", &[
        ch("a:header", 0, None, Sequence, None, 0),
    ]),
    ty("a:CT_HslColor", &[
        ch("a:tint", 0, None, Choice, Some("choice1"), 0),
        ch("a:shade", 0, None, Choice, Some("choice1"), 1),
        ch("a:comp", 0, None, Choice, Some("choice1"), 2),
        ch("a:inv", 0, None, Choice, Some("choice1"), 3),
        ch("a:gray", 0, None, Choice, Some("choice1"), 4),
        ch("a:alpha", 0, None, Choice, Some("choice1"), 5),
        ch("a:alphaOff", 0, None, Choice, Some("choice1"), 6),
        ch("a:alphaMod", 0, None, Choice, Some("choice1"), 7),
        ch("a:hue", 0, None, Choice, Some("choice1"), 8),
        ch("a:hueOff", 0, None, Choice, Some("choice1"), 9),
        ch("a:hueMod", 0, None, Choice, Some("choice1"), 10),
        ch("a:sat", 0, None, Choice, Some("choice1"), 11),
        ch("a:satOff", 0, None, Choice, Some("choice1"), 12),
        ch("a:satMod", 0, None, Choice, Some("choice1"), 13),
        ch("a:lum", 0, None, Choice, Some("choice1"), 14),
        ch("a:lumOff", 0, None, Choice, Some("choice1"), 15),
        ch("a:lumMod", 0, None, Choice, Some("choice1"), 16),
        ch("a:red", 0, None, Choice, Some("choice1"), 17),
        ch("a:redOff", 0, None, Choice, Some("choice1"), 18),
        ch("a:redMod", 0, None, Choice, Some("choice1"), 19),
        ch("a:green", 0, None, Choice, Some("choice1"), 20),
        ch("a:greenOff", 0, None, Choice, Some("choice1"), 21),
        ch("a:greenMod", 0, None, Choice, Some("choice1"), 22),
        ch("a:blue", 0, None, Choice, Some("choice1"), 23),
        ch("a:blueOff", 0, None, Choice, Some("choice1"), 24),
        ch("a:blueMod", 0, None, Choice, Some("choice1"), 25),
        ch("a:gamma", 0, None, Choice, Some("choice1"), 26),
        ch("a:invGamma", 0, None, Choice, Some("choice1"), 27),
    ]),
    ty("a:CT_Hyperlink", &[
        ch("a:snd", 0, Some(1), Sequence, None, 0),
        ch("a:extLst", 0, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_InnerShadowEffect", &[
        ch("a:scrgbClr", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:srgbClr", 1, Some(1), Choice, Some("choice1"), 1),
        ch("a:hslClr", 1, Some(1), Choice, Some("choice1"), 2),
        ch("a:sysClr", 1, Some(1), Choice, Some("choice1"), 3),
        ch("a:schemeClr", 1, Some(1), Choice, Some("choice1"), 4),
        ch("a:prstClr", 1, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_InverseGammaTransform", &[]),
    ty("a:CT_InverseTransform", &[]),
    ty("a:CT_LightRig", &[
        ch("a:rot", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_LineEndProperties", &[]),
    ty("a:CT_LineJoinBevel", &[]),
    ty("a:CT_LineJoinMiterProperties", &[]),
    ty("a:CT_LineJoinRound", &[]),
    ty("a:CT_LineProperties", &[
        ch("a:noFill", 0, Some(1), Choice, Some("choice1"), 0),
        ch("a:solidFill", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:gradFill", 0, Some(1), Choice, Some("choice1"), 2),
        ch("a:pattFill", 0, Some(1), Choice, Some("choice1"), 3),
        ch("a:prstDash", 0, Some(1), Choice, Some("choice2"), 4),
        ch("a:custDash", 0, Some(1), Choice, Some("choice2"), 5),
        ch("a:round", 0, Some(1), Choice, Some("choice3"), 6),
        ch("a:bevel", 0, Some(1), Choice, Some("choice3"), 7),
        ch("a:miter", 0, Some(1), Choice, Some("choice3"), 8),
        ch("a:headEnd", 0, Some(1), Sequence, None, 9),
        ch("a:tailEnd", 0, Some(1), Sequence, None, 10),
        ch("a:extLst", 0, Some(1), Sequence, None, 11),
    ]),
    ty("a:CT_LineStyleList", &[
        ch("a:ln", 3, None, Sequence, None, 0),
    ]),
    ty("a:CT_LinearShadeProperties", &[]),
    ty("a:CT_LuminanceEffect", &[]),
    ty("a:CT_NoFillProperties", &[]),
    ty("a:CT_NonVisualConnectorProperties", &[
        ch("a:cxnSpLocks", 0, Some(1), Sequence, None, 0),
        ch("a:stCxn", 0, Some(1), Sequence, None, 1),
        ch("a:endCxn", 0, Some(1), Sequence, None, 2),
        ch("a:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("a:CT_NonVisualContentPartProperties", &[
        ch("a:cpLocks", 0, Some(1), Sequence, None, 0),
        ch("a:extLst", 0, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_NonVisualDrawingProps", &[
        ch("a:hlinkClick", 0, Some(1), Sequence, None, 0),
        ch("a:hlinkHover", 0, Some(1), Sequence, None, 1),
        ch("a:extLst", 0, Some(1), Sequence, None, 2),
    ]),
    ty("a:CT_NonVisualDrawingShapeProps", &[
        ch("a:spLocks", 0, Some(1), Sequence, None, 0),
        ch("a:extLst", 0, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_NonVisualGraphicFrameProperties", &[
        ch("a:graphicFrameLocks", 0, Some(1), Sequence, None, 0),
        ch("a:extLst", 0, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_NonVisualGroupDrawingShapeProps", &[
        ch("a:grpSpLocks", 0, Some(1), Sequence, None, 0),
        ch("a:extLst", 0, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_NonVisualPictureProperties", &[
        ch("a:picLocks", 0, Some(1), Sequence, None, 0),
        ch("a:extLst", 0, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_ObjectStyleDefaults", &[
        ch("a:spDef", 0, Some(1), Sequence, None, 0),
        ch("a:lnDef", 0, Some(1), Sequence, None, 1),
        ch("a:txDef", 0, Some(1), Sequence, None, 2),
        ch("a:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("a:CT_OfficeArtExtension", &[
        ch("<any>", 0, None, Sequence, None, 0),
    ]),
    ty("a:CT_OfficeArtExtensionList", &[
        ch("a:ext", 0, None, Sequence, None, 0),
    ]),
    ty("a:CT_OfficeStyleSheet", &[
        ch("a:themeElements", 1, Some(1), Sequence, None, 0),
        ch("a:objectDefaults", 0, Some(1), Sequence, None, 1),
        ch("a:extraClrSchemeLst", 0, Some(1), Sequence, None, 2),
        ch("a:custClrLst", 0, Some(1), Sequence, None, 3),
        ch("a:extLst", 0, Some(1), Sequence, None, 4),
    ]),
    ty("a:CT_OuterShadowEffect", &[
        ch("a:scrgbClr", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:srgbClr", 1, Some(1), Choice, Some("choice1"), 1),
        ch("a:hslClr", 1, Some(1), Choice, Some("choice1"), 2),
        ch("a:sysClr", 1, Some(1), Choice, Some("choice1"), 3),
        ch("a:schemeClr", 1, Some(1), Choice, Some("choice1"), 4),
        ch("a:prstClr", 1, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_Path2D", &[
        ch("a:close", 0, None, Choice, Some("choice1"), 0),
        ch("a:moveTo", 0, None, Choice, Some("choice1"), 1),
        ch("a:lnTo", 0, None, Choice, Some("choice1"), 2),
        ch("a:arcTo", 0, None, Choice, Some("choice1"), 3),
        ch("a:quadBezTo", 0, None, Choice, Some("choice1"), 4),
        ch("a:cubicBezTo", 0, None, Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_Path2DArcTo", &[]),
    ty("a:CT_Path2DClose", &[]),
    ty("a:CT_Path2DCubicBezierTo", &[
        ch("a:pt", 3, Some(3), Sequence, None, 0),
    ]),
    ty("a:CT_Path2DLineTo", &[
        ch("a:pt", 1, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_Path2DList", &[
        ch("a:path", 0, None, Sequence, None, 0),
    ]),
    ty("a:CT_Path2DMoveTo", &[
        ch("a:pt", 1, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_Path2DQuadBezierTo", &[
        ch("a:pt", 2, Some(2), Sequence, None, 0),
    ]),
    ty("a:CT_PathShadeProperties", &[
        ch("a:fillToRect", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_PatternFillProperties", &[
        ch("a:fgClr", 0, Some(1), Sequence, None, 0),
        ch("a:bgClr", 0, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_Percentage", &[]),
    ty("a:CT_PictureLocking", &[
        ch("a:extLst", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_Point2D", &[]),
    ty("a:CT_Point3D", &[]),
    ty("a:CT_PolarAdjustHandle", &[
        ch("a:pos", 1, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_PositiveFixedAngle", &[]),
    ty("a:CT_PositiveFixedPercentage", &[]),
    ty("a:CT_PositivePercentage", &[]),
    ty("a:CT_PositiveSize2D", &[]),
    ty("a:CT_PresetColor", &[
        ch("a:tint", 0, None, Choice, Some("choice1"), 0),
        ch("a:shade", 0, None, Choice, Some("choice1"), 1),
        ch("a:comp", 0, None, Choice, Some("choice1"), 2),
        ch("a:inv", 0, None, Choice, Some("choice1"), 3),
        ch("a:gray", 0, None, Choice, Some("choice1"), 4),
        ch("a:alpha", 0, None, Choice, Some("choice1"), 5),
        ch("a:alphaOff", 0, None, Choice, Some("choice1"), 6),
        ch("a:alphaMod", 0, None, Choice, Some("choice1"), 7),
        ch("a:hue", 0, None, Choice, Some("choice1"), 8),
        ch("a:hueOff", 0, None, Choice, Some("choice1"), 9),
        ch("a:hueMod", 0, None, Choice, Some("choice1"), 10),
        ch("a:sat", 0, None, Choice, Some("choice1"), 11),
        ch("a:satOff", 0, None, Choice, Some("choice1"), 12),
        ch("a:satMod", 0, None, Choice, Some("choice1"), 13),
        ch("a:lum", 0, None, Choice, Some("choice1"), 14),
        ch("a:lumOff", 0, None, Choice, Some("choice1"), 15),
        ch("a:lumMod", 0, None, Choice, Some("choice1"), 16),
        ch("a:red", 0, None, Choice, Some("choice1"), 17),
        ch("a:redOff", 0, None, Choice, Some("choice1"), 18),
        ch("a:redMod", 0, None, Choice, Some("choice1"), 19),
        ch("a:green", 0, None, Choice, Some("choice1"), 20),
        ch("a:greenOff", 0, None, Choice, Some("choice1"), 21),
        ch("a:greenMod", 0, None, Choice, Some("choice1"), 22),
        ch("a:blue", 0, None, Choice, Some("choice1"), 23),
        ch("a:blueOff", 0, None, Choice, Some("choice1"), 24),
        ch("a:blueMod", 0, None, Choice, Some("choice1"), 25),
        ch("a:gamma", 0, None, Choice, Some("choice1"), 26),
        ch("a:invGamma", 0, None, Choice, Some("choice1"), 27),
    ]),
    ty("a:CT_PresetGeometry2D", &[
        ch("a:avLst", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_PresetLineDashProperties", &[]),
    ty("a:CT_PresetShadowEffect", &[
        ch("a:scrgbClr", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:srgbClr", 1, Some(1), Choice, Some("choice1"), 1),
        ch("a:hslClr", 1, Some(1), Choice, Some("choice1"), 2),
        ch("a:sysClr", 1, Some(1), Choice, Some("choice1"), 3),
        ch("a:schemeClr", 1, Some(1), Choice, Some("choice1"), 4),
        ch("a:prstClr", 1, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_PresetTextShape", &[
        ch("a:avLst", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_QuickTimeFile", &[
        ch("a:extLst", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_Ratio", &[]),
    ty("a:CT_ReflectionEffect", &[]),
    ty("a:CT_RegularTextRun", &[
        ch("a:rPr", 0, Some(1), Sequence, None, 0),
        ch("a:t", 1, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_RelativeOffsetEffect", &[]),
    ty("a:CT_RelativeRect", &[]),
    ty("a:CT_SRgbColor", &[
        ch("a:tint", 0, None, Choice, Some("choice1"), 0),
        ch("a:shade", 0, None, Choice, Some("choice1"), 1),
        ch("a:comp", 0, None, Choice, Some("choice1"), 2),
        ch("a:inv", 0, None, Choice, Some("choice1"), 3),
        ch("a:gray", 0, None, Choice, Some("choice1"), 4),
        ch("a:alpha", 0, None, Choice, Some("choice1"), 5),
        ch("a:alphaOff", 0, None, Choice, Some("choice1"), 6),
        ch("a:alphaMod", 0, None, Choice, Some("choice1"), 7),
        ch("a:hue", 0, None, Choice, Some("choice1"), 8),
        ch("a:hueOff", 0, None, Choice, Some("choice1"), 9),
        ch("a:hueMod", 0, None, Choice, Some("choice1"), 10),
        ch("a:sat", 0, None, Choice, Some("choice1"), 11),
        ch("a:satOff", 0, None, Choice, Some("choice1"), 12),
        ch("a:satMod", 0, None, Choice, Some("choice1"), 13),
        ch("a:lum", 0, None, Choice, Some("choice1"), 14),
        ch("a:lumOff", 0, None, Choice, Some("choice1"), 15),
        ch("a:lumMod", 0, None, Choice, Some("choice1"), 16),
        ch("a:red", 0, None, Choice, Some("choice1"), 17),
        ch("a:redOff", 0, None, Choice, Some("choice1"), 18),
        ch("a:redMod", 0, None, Choice, Some("choice1"), 19),
        ch("a:green", 0, None, Choice, Some("choice1"), 20),
        ch("a:greenOff", 0, None, Choice, Some("choice1"), 21),
        ch("a:greenMod", 0, None, Choice, Some("choice1"), 22),
        ch("a:blue", 0, None, Choice, Some("choice1"), 23),
        ch("a:blueOff", 0, None, Choice, Some("choice1"), 24),
        ch("a:blueMod", 0, None, Choice, Some("choice1"), 25),
        ch("a:gamma", 0, None, Choice, Some("choice1"), 26),
        ch("a:invGamma", 0, None, Choice, Some("choice1"), 27),
    ]),
    ty("a:CT_ScRgbColor", &[
        ch("a:tint", 0, None, Choice, Some("choice1"), 0),
        ch("a:shade", 0, None, Choice, Some("choice1"), 1),
        ch("a:comp", 0, None, Choice, Some("choice1"), 2),
        ch("a:inv", 0, None, Choice, Some("choice1"), 3),
        ch("a:gray", 0, None, Choice, Some("choice1"), 4),
        ch("a:alpha", 0, None, Choice, Some("choice1"), 5),
        ch("a:alphaOff", 0, None, Choice, Some("choice1"), 6),
        ch("a:alphaMod", 0, None, Choice, Some("choice1"), 7),
        ch("a:hue", 0, None, Choice, Some("choice1"), 8),
        ch("a:hueOff", 0, None, Choice, Some("choice1"), 9),
        ch("a:hueMod", 0, None, Choice, Some("choice1"), 10),
        ch("a:sat", 0, None, Choice, Some("choice1"), 11),
        ch("a:satOff", 0, None, Choice, Some("choice1"), 12),
        ch("a:satMod", 0, None, Choice, Some("choice1"), 13),
        ch("a:lum", 0, None, Choice, Some("choice1"), 14),
        ch("a:lumOff", 0, None, Choice, Some("choice1"), 15),
        ch("a:lumMod", 0, None, Choice, Some("choice1"), 16),
        ch("a:red", 0, None, Choice, Some("choice1"), 17),
        ch("a:redOff", 0, None, Choice, Some("choice1"), 18),
        ch("a:redMod", 0, None, Choice, Some("choice1"), 19),
        ch("a:green", 0, None, Choice, Some("choice1"), 20),
        ch("a:greenOff", 0, None, Choice, Some("choice1"), 21),
        ch("a:greenMod", 0, None, Choice, Some("choice1"), 22),
        ch("a:blue", 0, None, Choice, Some("choice1"), 23),
        ch("a:blueOff", 0, None, Choice, Some("choice1"), 24),
        ch("a:blueMod", 0, None, Choice, Some("choice1"), 25),
        ch("a:gamma", 0, None, Choice, Some("choice1"), 26),
        ch("a:invGamma", 0, None, Choice, Some("choice1"), 27),
    ]),
    ty("a:CT_Scale2D", &[
        ch("a:sx", 1, Some(1), Sequence, None, 0),
        ch("a:sy", 1, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_Scene3D", &[
        ch("a:camera", 1, Some(1), Sequence, None, 0),
        ch("a:lightRig", 1, Some(1), Sequence, None, 1),
        ch("a:backdrop", 0, Some(1), Sequence, None, 2),
        ch("a:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("a:CT_SchemeColor", &[
        ch("a:tint", 0, None, Choice, Some("choice1"), 0),
        ch("a:shade", 0, None, Choice, Some("choice1"), 1),
        ch("a:comp", 0, None, Choice, Some("choice1"), 2),
        ch("a:inv", 0, None, Choice, Some("choice1"), 3),
        ch("a:gray", 0, None, Choice, Some("choice1"), 4),
        ch("a:alpha", 0, None, Choice, Some("choice1"), 5),
        ch("a:alphaOff", 0, None, Choice, Some("choice1"), 6),
        ch("a:alphaMod", 0, None, Choice, Some("choice1"), 7),
        ch("a:hue", 0, None, Choice, Some("choice1"), 8),
        ch("a:hueOff", 0, None, Choice, Some("choice1"), 9),
        ch("a:hueMod", 0, None, Choice, Some("choice1"), 10),
        ch("a:sat", 0, None, Choice, Some("choice1"), 11),
        ch("a:satOff", 0, None, Choice, Some("choice1"), 12),
        ch("a:satMod", 0, None, Choice, Some("choice1"), 13),
        ch("a:lum", 0, None, Choice, Some("choice1"), 14),
        ch("a:lumOff", 0, None, Choice, Some("choice1"), 15),
        ch("a:lumMod", 0, None, Choice, Some("choice1"), 16),
        ch("a:red", 0, None, Choice, Some("choice1"), 17),
        ch("a:redOff", 0, None, Choice, Some("choice1"), 18),
        ch("a:redMod", 0, None, Choice, Some("choice1"), 19),
        ch("a:green", 0, None, Choice, Some("choice1"), 20),
        ch("a:greenOff", 0, None, Choice, Some("choice1"), 21),
        ch("a:greenMod", 0, None, Choice, Some("choice1"), 22),
        ch("a:blue", 0, None, Choice, Some("choice1"), 23),
        ch("a:blueOff", 0, None, Choice, Some("choice1"), 24),
        ch("a:blueMod", 0, None, Choice, Some("choice1"), 25),
        ch("a:gamma", 0, None, Choice, Some("choice1"), 26),
        ch("a:invGamma", 0, None, Choice, Some("choice1"), 27),
    ]),
    ty("a:CT_Shape3D", &[
        ch("a:bevelT", 0, Some(1), Sequence, None, 0),
        ch("a:bevelB", 0, Some(1), Sequence, None, 1),
        ch("a:extrusionClr", 0, Some(1), Sequence, None, 2),
        ch("a:contourClr", 0, Some(1), Sequence, None, 3),
        ch("a:extLst", 0, Some(1), Sequence, None, 4),
    ]),
    ty("a:CT_ShapeLocking", &[
        ch("a:extLst", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_ShapeProperties", &[
        ch("a:xfrm", 0, Some(1), Sequence, None, 0),
        ch("a:custGeom", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:prstGeom", 0, Some(1), Choice, Some("choice1"), 2),
        ch("a:noFill", 0, Some(1), Choice, Some("choice2"), 3),
        ch("a:solidFill", 0, Some(1), Choice, Some("choice2"), 4),
        ch("a:gradFill", 0, Some(1), Choice, Some("choice2"), 5),
        ch("a:blipFill", 0, Some(1), Choice, Some("choice2"), 6),
        ch("a:pattFill", 0, Some(1), Choice, Some("choice2"), 7),
        ch("a:grpFill", 0, Some(1), Choice, Some("choice2"), 8),
        ch("a:ln", 0, Some(1), Sequence, None, 9),
        ch("a:effectLst", 0, Some(1), Choice, Some("choice3"), 10),
        ch("a:effectDag", 0, Some(1), Choice, Some("choice3"), 11),
        ch("a:scene3d", 0, Some(1), Sequence, None, 12),
        ch("a:sp3d", 0, Some(1), Sequence, None, 13),
        ch("a:extLst", 0, Some(1), Sequence, None, 14),
    ]),
    ty("a:CT_ShapeStyle", &[
        ch("a:lnRef", 1, Some(1), Sequence, None, 0),
        ch("a:fillRef", 1, Some(1), Sequence, None, 1),
        ch("a:effectRef", 1, Some(1), Sequence, None, 2),
        ch("a:fontRef", 1, Some(1), Sequence, None, 3),
    ]),
    ty("a:CT_SoftEdgesEffect", &[]),
    ty("a:CT_SolidColorFillProperties", &[
        ch("a:scrgbClr", 0, Some(1), Choice, Some("choice1"), 0),
        ch("a:srgbClr", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:hslClr", 0, Some(1), Choice, Some("choice1"), 2),
        ch("a:sysClr", 0, Some(1), Choice, Some("choice1"), 3),
        ch("a:schemeClr", 0, Some(1), Choice, Some("choice1"), 4),
        ch("a:prstClr", 0, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_SphereCoords", &[]),
    ty("a:CT_StretchInfoProperties", &[
        ch("a:fillRect", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_StyleMatrix", &[
        ch("a:fillStyleLst", 1, Some(1), Sequence, None, 0),
        ch("a:lnStyleLst", 1, Some(1), Sequence, None, 1),
        ch("a:effectStyleLst", 1, Some(1), Sequence, None, 2),
        ch("a:bgFillStyleLst", 1, Some(1), Sequence, None, 3),
    ]),
    ty("a:CT_StyleMatrixReference", &[
        ch("a:scrgbClr", 0, Some(1), Choice, Some("choice1"), 0),
        ch("a:srgbClr", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:hslClr", 0, Some(1), Choice, Some("choice1"), 2),
        ch("a:sysClr", 0, Some(1), Choice, Some("choice1"), 3),
        ch("a:schemeClr", 0, Some(1), Choice, Some("choice1"), 4),
        ch("a:prstClr", 0, Some(1), Choice, Some("choice1"), 5),
    ]),
    ty("a:CT_SupplementalFont", &[]),
    ty("a:CT_SystemColor", &[
        ch("a:tint", 0, None, Choice, Some("choice1"), 0),
        ch("a:shade", 0, None, Choice, Some("choice1"), 1),
        ch("a:comp", 0, None, Choice, Some("choice1"), 2),
        ch("a:inv", 0, None, Choice, Some("choice1"), 3),
        ch("a:gray", 0, None, Choice, Some("choice1"), 4),
        ch("a:alpha", 0, None, Choice, Some("choice1"), 5),
        ch("a:alphaOff", 0, None, Choice, Some("choice1"), 6),
        ch("a:alphaMod", 0, None, Choice, Some("choice1"), 7),
        ch("a:hue", 0, None, Choice, Some("choice1"), 8),
        ch("a:hueOff", 0, None, Choice, Some("choice1"), 9),
        ch("a:hueMod", 0, None, Choice, Some("choice1"), 10),
        ch("a:sat", 0, None, Choice, Some("choice1"), 11),
        ch("a:satOff", 0, None, Choice, Some("choice1"), 12),
        ch("a:satMod", 0, None, Choice, Some("choice1"), 13),
        ch("a:lum", 0, None, Choice, Some("choice1"), 14),
        ch("a:lumOff", 0, None, Choice, Some("choice1"), 15),
        ch("a:lumMod", 0, None, Choice, Some("choice1"), 16),
        ch("a:red", 0, None, Choice, Some("choice1"), 17),
        ch("a:redOff", 0, None, Choice, Some("choice1"), 18),
        ch("a:redMod", 0, None, Choice, Some("choice1"), 19),
        ch("a:green", 0, None, Choice, Some("choice1"), 20),
        ch("a:greenOff", 0, None, Choice, Some("choice1"), 21),
        ch("a:greenMod", 0, None, Choice, Some("choice1"), 22),
        ch("a:blue", 0, None, Choice, Some("choice1"), 23),
        ch("a:blueOff", 0, None, Choice, Some("choice1"), 24),
        ch("a:blueMod", 0, None, Choice, Some("choice1"), 25),
        ch("a:gamma", 0, None, Choice, Some("choice1"), 26),
        ch("a:invGamma", 0, None, Choice, Some("choice1"), 27),
    ]),
    ty("a:CT_Table", &[
        ch("a:tblPr", 0, Some(1), Sequence, None, 0),
        ch("a:tblGrid", 1, Some(1), Sequence, None, 1),
        ch("a:tr", 0, None, Sequence, None, 2),
    ]),
    ty("a:CT_TableBackgroundStyle", &[
        ch("a:fill", 0, Some(1), Choice, Some("choice1"), 0),
        ch("a:fillRef", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:effect", 0, Some(1), Choice, Some("choice2"), 2),
        ch("a:effectRef", 0, Some(1), Choice, Some("choice2"), 3),
    ]),
    ty("a:CT_TableCell", &[
        ch("a:txBody", 0, Some(1), Sequence, None, 0),
        ch("a:tcPr", 0, Some(1), Sequence, None, 1),
        ch("a:extLst", 0, Some(1), Sequence, None, 2),
    ]),
    ty("a:CT_TableCellBorderStyle", &[
        ch("a:left", 0, Some(1), Sequence, None, 0),
        ch("a:right", 0, Some(1), Sequence, None, 1),
        ch("a:top", 0, Some(1), Sequence, None, 2),
        ch("a:bottom", 0, Some(1), Sequence, None, 3),
        ch("a:insideH", 0, Some(1), Sequence, None, 4),
        ch("a:insideV", 0, Some(1), Sequence, None, 5),
        ch("a:tl2br", 0, Some(1), Sequence, None, 6),
        ch("a:tr2bl", 0, Some(1), Sequence, None, 7),
        ch("a:extLst", 0, Some(1), Sequence, None, 8),
    ]),
    ty("a:CT_TableCellProperties", &[
        ch("a:lnL", 0, Some(1), Sequence, None, 0),
        ch("a:lnR", 0, Some(1), Sequence, None, 1),
        ch("a:lnT", 0, Some(1), Sequence, None, 2),
        ch("a:lnB", 0, Some(1), Sequence, None, 3),
        ch("a:lnTlToBr", 0, Some(1), Sequence, None, 4),
        ch("a:lnBlToTr", 0, Some(1), Sequence, None, 5),
        ch("a:cell3D", 0, Some(1), Sequence, None, 6),
        ch("a:noFill", 0, Some(1), Choice, Some("choice1"), 7),
        ch("a:solidFill", 0, Some(1), Choice, Some("choice1"), 8),
        ch("a:gradFill", 0, Some(1), Choice, Some("choice1"), 9),
        ch("a:blipFill", 0, Some(1), Choice, Some("choice1"), 10),
        ch("a:pattFill", 0, Some(1), Choice, Some("choice1"), 11),
        ch("a:grpFill", 0, Some(1), Choice, Some("choice1"), 12),
        ch("a:headers", 0, Some(1), Sequence, None, 13),
        ch("a:extLst", 0, Some(1), Sequence, None, 14),
    ]),
    ty("a:CT_TableCol", &[
        ch("a:extLst", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_TableGrid", &[
        ch("a:gridCol", 0, None, Sequence, None, 0),
    ]),
    ty("a:CT_TablePartStyle", &[
        ch("a:tcTxStyle", 0, Some(1), Sequence, None, 0),
        ch("a:tcStyle", 0, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_TableProperties", &[
        ch("a:noFill", 0, Some(1), Choice, Some("choice1"), 0),
        ch("a:solidFill", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:gradFill", 0, Some(1), Choice, Some("choice1"), 2),
        ch("a:blipFill", 0, Some(1), Choice, Some("choice1"), 3),
        ch("a:pattFill", 0, Some(1), Choice, Some("choice1"), 4),
        ch("a:grpFill", 0, Some(1), Choice, Some("choice1"), 5),
        ch("a:effectLst", 0, Some(1), Choice, Some("choice2"), 6),
        ch("a:effectDag", 0, Some(1), Choice, Some("choice2"), 7),
        ch("a:tableStyle", 0, Some(1), Choice, Some("choice3"), 8),
        ch("a:tableStyleId", 0, Some(1), Choice, Some("choice3"), 9),
        ch("a:extLst", 0, Some(1), Sequence, None, 10),
    ]),
    ty("a:CT_TableRow", &[
        ch("a:tc", 0, None, Sequence, None, 0),
        ch("a:extLst", 0, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_TableStyle", &[
        ch("a:tblBg", 0, Some(1), Sequence, None, 0),
        ch("a:wholeTbl", 0, Some(1), Sequence, None, 1),
        ch("a:band1H", 0, Some(1), Sequence, None, 2),
        ch("a:band2H", 0, Some(1), Sequence, None, 3),
        ch("a:band1V", 0, Some(1), Sequence, None, 4),
        ch("a:band2V", 0, Some(1), Sequence, None, 5),
        ch("a:lastCol", 0, Some(1), Sequence, None, 6),
        ch("a:firstCol", 0, Some(1), Sequence, None, 7),
        ch("a:lastRow", 0, Some(1), Sequence, None, 8),
        ch("a:seCell", 0, Some(1), Sequence, None, 9),
        ch("a:swCell", 0, Some(1), Sequence, None, 10),
        ch("a:firstRow", 0, Some(1), Sequence, None, 11),
        ch("a:neCell", 0, Some(1), Sequence, None, 12),
        ch("a:nwCell", 0, Some(1), Sequence, None, 13),
        ch("a:extLst", 0, Some(1), Sequence, None, 14),
    ]),
    ty("a:CT_TableStyleCellStyle", &[
        ch("a:tcBdr", 0, Some(1), Sequence, None, 0),
        ch("a:fill", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:fillRef", 0, Some(1), Choice, Some("choice1"), 2),
        ch("a:cell3D", 0, Some(1), Sequence, None, 3),
    ]),
    ty("a:CT_TableStyleList", &[
        ch("a:tblStyle", 0, None, Sequence, None, 0),
    ]),
    ty("a:CT_TableStyleTextStyle", &[
        ch("a:font", 0, Some(1), Choice, Some("choice1"), 0),
        ch("a:fontRef", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:scrgbClr", 0, Some(1), Choice, Some("choice2"), 2),
        ch("a:srgbClr", 0, Some(1), Choice, Some("choice2"), 3),
        ch("a:hslClr", 0, Some(1), Choice, Some("choice2"), 4),
        ch("a:sysClr", 0, Some(1), Choice, Some("choice2"), 5),
        ch("a:schemeClr", 0, Some(1), Choice, Some("choice2"), 6),
        ch("a:prstClr", 0, Some(1), Choice, Some("choice2"), 7),
        ch("a:extLst", 0, Some(1), Sequence, None, 8),
    ]),
    ty("a:CT_TextAutonumberBullet", &[]),
    ty("a:CT_TextBlipBullet", &[
        ch("a:blip", 1, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_TextBody", &[
        ch("a:bodyPr", 1, Some(1), Sequence, None, 0),
        ch("a:lstStyle", 0, Some(1), Sequence, None, 1),
        ch("a:p", 1, None, Sequence, None, 2),
    ]),
    ty("a:CT_TextBodyProperties", &[
        ch("a:prstTxWarp", 0, Some(1), Sequence, None, 0),
        ch("a:noAutofit", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:normAutofit", 0, Some(1), Choice, Some("choice1"), 2),
        ch("a:spAutoFit", 0, Some(1), Choice, Some("choice1"), 3),
        ch("a:scene3d", 0, Some(1), Sequence, None, 4),
        ch("a:sp3d", 0, Some(1), Choice, Some("choice2"), 5),
        ch("a:flatTx", 0, Some(1), Choice, Some("choice2"), 6),
        ch("a:extLst", 0, Some(1), Sequence, None, 7),
    ]),
    ty("a:CT_TextBulletColorFollowText", &[]),
    ty("a:CT_TextBulletSizeFollowText", &[]),
    ty("a:CT_TextBulletSizePercent", &[]),
    ty("a:CT_TextBulletSizePoint", &[]),
    ty("a:CT_TextBulletTypefaceFollowText", &[]),
    ty("a:CT_TextCharBullet", &[]),
    ty("a:CT_TextCharacterProperties", &[
        ch("a:ln", 0, Some(1), Sequence, None, 0),
        ch("a:noFill", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:solidFill", 0, Some(1), Choice, Some("choice1"), 2),
        ch("a:gradFill", 0, Some(1), Choice, Some("choice1"), 3),
        ch("a:blipFill", 0, Some(1), Choice, Some("choice1"), 4),
        ch("a:pattFill", 0, Some(1), Choice, Some("choice1"), 5),
        ch("a:grpFill", 0, Some(1), Choice, Some("choice1"), 6),
        ch("a:effectLst", 0, Some(1), Choice, Some("choice2"), 7),
        ch("a:effectDag", 0, Some(1), Choice, Some("choice2"), 8),
        ch("a:highlight", 0, Some(1), Sequence, None, 9),
        ch("a:uLnTx", 0, Some(1), Choice, Some("choice3"), 10),
        ch("a:uLn", 0, Some(1), Choice, Some("choice3"), 11),
        ch("a:uFillTx", 0, Some(1), Choice, Some("choice4"), 12),
        ch("a:uFill", 0, Some(1), Choice, Some("choice4"), 13),
        ch("a:latin", 0, Some(1), Sequence, None, 14),
        ch("a:ea", 0, Some(1), Sequence, None, 15),
        ch("a:cs", 0, Some(1), Sequence, None, 16),
        ch("a:sym", 0, Some(1), Sequence, None, 17),
        ch("a:hlinkClick", 0, Some(1), Sequence, None, 18),
        ch("a:hlinkMouseOver", 0, Some(1), Sequence, None, 19),
        ch("a:rtl", 0, Some(1), Sequence, None, 20),
        ch("a:extLst", 0, Some(1), Sequence, None, 21),
    ]),
    ty("a:CT_TextField", &[
        ch("a:rPr", 0, Some(1), Sequence, None, 0),
        ch("a:pPr", 0, Some(1), Sequence, None, 1),
        ch("a:t", 0, Some(1), Sequence, None, 2),
    ]),
    ty("a:CT_TextFont", &[]),
    ty("a:CT_TextLineBreak", &[
        ch("a:rPr", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_TextListStyle", &[
        ch("a:defPPr", 0, Some(1), Sequence, None, 0),
        ch("a:lvl1pPr", 0, Some(1), Sequence, None, 1),
        ch("a:lvl2pPr", 0, Some(1), Sequence, None, 2),
        ch("a:lvl3pPr", 0, Some(1), Sequence, None, 3),
        ch("a:lvl4pPr", 0, Some(1), Sequence, None, 4),
        ch("a:lvl5pPr", 0, Some(1), Sequence, None, 5),
        ch("a:lvl6pPr", 0, Some(1), Sequence, None, 6),
        ch("a:lvl7pPr", 0, Some(1), Sequence, None, 7),
        ch("a:lvl8pPr", 0, Some(1), Sequence, None, 8),
        ch("a:lvl9pPr", 0, Some(1), Sequence, None, 9),
        ch("a:extLst", 0, Some(1), Sequence, None, 10),
    ]),
    ty("a:CT_TextNoAutofit", &[]),
    ty("a:CT_TextNoBullet", &[]),
    ty("a:CT_TextNormalAutofit", &[]),
    ty("a:CT_TextParagraph", &[
        ch("a:pPr", 0, Some(1), Sequence, None, 0),
        ch("a:r", 0, None, Choice, Some("choice1"), 1),
        ch("a:br", 0, None, Choice, Some("choice1"), 2),
        ch("a:fld", 0, None, Choice, Some("choice1"), 3),
        ch("a:endParaRPr", 0, Some(1), Sequence, None, 4),
    ]),
    ty("a:CT_TextParagraphProperties", &[
        ch("a:lnSpc", 0, Some(1), Sequence, None, 0),
        ch("a:spcBef", 0, Some(1), Sequence, None, 1),
        ch("a:spcAft", 0, Some(1), Sequence, None, 2),
        ch("a:buClrTx", 0, Some(1), Choice, Some("choice1"), 3),
        ch("a:buClr", 0, Some(1), Choice, Some("choice1"), 4),
        ch("a:buSzTx", 0, Some(1), Choice, Some("choice2"), 5),
        ch("a:buSzPct", 0, Some(1), Choice, Some("choice2"), 6),
        ch("a:buSzPts", 0, Some(1), Choice, Some("choice2"), 7),
        ch("a:buFontTx", 0, Some(1), Choice, Some("choice3"), 8),
        ch("a:buFont", 0, Some(1), Choice, Some("choice3"), 9),
        ch("a:buNone", 0, Some(1), Choice, Some("choice4"), 10),
        ch("a:buAutoNum", 0, Some(1), Choice, Some("choice4"), 11),
        ch("a:buChar", 0, Some(1), Choice, Some("choice4"), 12),
        ch("a:buBlip", 0, Some(1), Choice, Some("choice4"), 13),
        ch("a:tabLst", 0, Some(1), Sequence, None, 14),
        ch("a:defRPr", 0, Some(1), Sequence, None, 15),
        ch("a:extLst", 0, Some(1), Sequence, None, 16),
    ]),
    ty("a:CT_TextShapeAutofit", &[]),
    ty("a:CT_TextSpacing", &[
        ch("a:spcPct", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:spcPts", 1, Some(1), Choice, Some("choice1"), 1),
    ]),
    ty("a:CT_TextSpacingPercent", &[]),
    ty("a:CT_TextSpacingPoint", &[]),
    ty("a:CT_TextTabStop", &[]),
    ty("a:CT_TextTabStopList", &[
        ch("a:tab", 0, Some(32), Sequence, None, 0),
    ]),
    ty("a:CT_TextUnderlineFillFollowText", &[]),
    ty("a:CT_TextUnderlineFillGroupWrapper", &[]),
    ty("a:CT_TextUnderlineLineFollowText", &[]),
    ty("a:CT_ThemeableLineStyle", &[
        ch("a:ln", 1, Some(1), Choice, Some("choice1"), 0),
        ch("a:lnRef", 1, Some(1), Choice, Some("choice1"), 1),
    ]),
    ty("a:CT_TileInfoProperties", &[]),
    ty("a:CT_TintEffect", &[]),
    ty("a:CT_Transform2D", &[
        ch("a:off", 0, Some(1), Sequence, None, 0),
        ch("a:ext", 0, Some(1), Sequence, None, 1),
    ]),
    ty("a:CT_TransformEffect", &[]),
    ty("a:CT_Vector3D", &[]),
    ty("a:CT_VideoFile", &[
        ch("a:extLst", 0, Some(1), Sequence, None, 0),
    ]),
    ty("a:CT_WholeE2oFormatting", &[
        ch("a:ln", 0, Some(1), Sequence, None, 0),
        ch("a:effectLst", 0, Some(1), Choice, Some("choice1"), 1),
        ch("a:effectDag", 0, Some(1), Choice, Some("choice1"), 2),
    ]),
    ty("a:CT_XYAdjustHandle", &[
        ch("a:pos", 1, Some(1), Sequence, None, 0),
    ]),
    ty("b:CT_AuthorType", &[
        ch("b:Artist", 0, None, Choice, Some("choice1"), 0),
        ch("b:Author", 0, None, Choice, Some("choice1"), 1),
        ch("b:BookAuthor", 0, None, Choice, Some("choice1"), 2),
        ch("b:Compiler", 0, None, Choice, Some("choice1"), 3),
        ch("b:Composer", 0, None, Choice, Some("choice1"), 4),
        ch("b:Conductor", 0, None, Choice, Some("choice1"), 5),
        ch("b:Counsel", 0, None, Choice, Some("choice1"), 6),
        ch("b:Director", 0, None, Choice, Some("choice1"), 7),
        ch("b:Editor", 0, None, Choice, Some("choice1"), 8),
        ch("b:Interviewee", 0, None, Choice, Some("choice1"), 9),
        ch("b:Interviewer", 0, None, Choice, Some("choice1"), 10),
        ch("b:Inventor", 0, None, Choice, Some("choice1"), 11),
        ch("b:Performer", 0, None, Choice, Some("choice1"), 12),
        ch("b:ProducerName", 0, None, Choice, Some("choice1"), 13),
        ch("b:Translator", 0, None, Choice, Some("choice1"), 14),
        ch("b:Writer", 0, None, Choice, Some("choice1"), 15),
    ]),
    ty("b:CT_NameListType", &[
        ch("b:Person", 1, None, Sequence, None, 0),
    ]),
    ty("b:CT_NameOrCorporateType", &[
        ch("b:NameList", 0, Some(1), Choice, Some("choice1"), 0),
        ch("b:Corporate", 0, Some(1), Choice, Some("choice1"), 1),
    ]),
    ty("b:CT_NameType", &[
        ch("b:NameList", 1, Some(1), Sequence, None, 0),
    ]),
    ty("b:CT_PersonType", &[
        ch("b:Last", 0, None, Sequence, None, 0),
        ch("b:First", 0, None, Sequence, None, 1),
        ch("b:Middle", 0, None, Sequence, None, 2),
    ]),
    ty("b:CT_SourceType", &[
        ch("b:AbbreviatedCaseNumber", 0, None, Choice, Some("choice1"), 0),
        ch("b:AlbumTitle", 0, None, Choice, Some("choice1"), 1),
        ch("b:Author", 0, None, Choice, Some("choice1"), 2),
        ch("b:BookTitle", 0, None, Choice, Some("choice1"), 3),
        ch("b:Broadcaster", 0, None, Choice, Some("choice1"), 4),
        ch("b:BroadcastTitle", 0, None, Choice, Some("choice1"), 5),
        ch("b:CaseNumber", 0, None, Choice, Some("choice1"), 6),
        ch("b:ChapterNumber", 0, None, Choice, Some("choice1"), 7),
        ch("b:City", 0, None, Choice, Some("choice1"), 8),
        ch("b:Comments", 0, None, Choice, Some("choice1"), 9),
        ch("b:ConferenceName", 0, None, Choice, Some("choice1"), 10),
        ch("b:CountryRegion", 0, None, Choice, Some("choice1"), 11),
        ch("b:Court", 0, None, Choice, Some("choice1"), 12),
        ch("b:Day", 0, None, Choice, Some("choice1"), 13),
        ch("b:DayAccessed", 0, None, Choice, Some("choice1"), 14),
        ch("b:Department", 0, None, Choice, Some("choice1"), 15),
        ch("b:Distributor", 0, None, Choice, Some("choice1"), 16),
        ch("b:Edition", 0, None, Choice, Some("choice1"), 17),
        ch("b:Guid", 0, None, Choice, Some("choice1"), 18),
        ch("b:Institution", 0, None, Choice, Some("choice1"), 19),
        ch("b:InternetSiteTitle", 0, None, Choice, Some("choice1"), 20),
        ch("b:Issue", 0, None, Choice, Some("choice1"), 21),
        ch("b:JournalName", 0, None, Choice, Some("choice1"), 22),
        ch("b:LCID", 0, None, Choice, Some("choice1"), 23),
        ch("b:Medium", 0, None, Choice, Some("choice1"), 24),
        ch("b:Month", 0, None, Choice, Some("choice1"), 25),
        ch("b:MonthAccessed", 0, None, Choice, Some("choice1"), 26),
        ch("b:NumberVolumes", 0, None, Choice, Some("choice1"), 27),
        ch("b:Pages", 0, None, Choice, Some("choice1"), 28),
        ch("b:PatentNumber", 0, None, Choice, Some("choice1"), 29),
        ch("b:PeriodicalTitle", 0, None, Choice, Some("choice1"), 30),
        ch("b:ProductionCompany", 0, None, Choice, Some("choice1"), 31),
        ch("b:PublicationTitle", 0, None, Choice, Some("choice1"), 32),
        ch("b:Publisher", 0, None, Choice, Some("choice1"), 33),
        ch("b:RecordingNumber", 0, None, Choice, Some("choice1"), 34),
        ch("b:RefOrder", 0, None, Choice, Some("choice1"), 35),
        ch("b:Reporter", 0, None, Choice, Some("choice1"), 36),
        ch("b:SourceType", 0, None, Choice, Some("choice1"), 37),
        ch("b:ShortTitle", 0, None, Choice, Some("choice1"), 38),
        ch("b:StandardNumber", 0, None, Choice, Some("choice1"), 39),
        ch("b:StateProvince", 0, None, Choice, Some("choice1"), 40),
        ch("b:Station", 0, None, Choice, Some("choice1"), 41),
        ch("b:Tag", 0, None, Choice, Some("choice1"), 42),
        ch("b:Theater", 0, None, Choice, Some("choice1"), 43),
        ch("b:ThesisType", 0, None, Choice, Some("choice1"), 44),
        ch("b:Title", 0, None, Choice, Some("choice1"), 45),
        ch("b:Type", 0, None, Choice, Some("choice1"), 46),
        ch("b:URL", 0, None, Choice, Some("choice1"), 47),
        ch("b:Version", 0, None, Choice, Some("choice1"), 48),
        ch("b:Volume", 0, None, Choice, Some("choice1"), 49),
        ch("b:Year", 0, None, Choice, Some("choice1"), 50),
        ch("b:YearAccessed", 0, None, Choice, Some("choice1"), 51),
    ]),
    ty("b:CT_Sources", &[
        ch("b:Source", 0, None, Sequence, None, 0),
    ]),
    ty("c:CT_Area3DChart", &[
        ch("c:grouping", 0, Some(1), Sequence, None, 0),
        ch("c:varyColors", 0, Some(1), Sequence, None, 1),
        ch("c:ser", 0, None, Sequence, None, 2),
        ch("c:dLbls", 0, Some(1), Sequence, None, 3),
        ch("c:dropLines", 0, Some(1), Sequence, None, 4),
        ch("c:gapDepth", 0, Some(1), Sequence, None, 5),
        ch("c:axId", 2, Some(3), Sequence, None, 6),
        ch("c:extLst", 0, Some(1), Sequence, None, 7),
    ]),
    ty("c:CT_AreaChart", &[
        ch("c:grouping", 0, Some(1), Sequence, None, 0),
        ch("c:varyColors", 0, Some(1), Sequence, None, 1),
        ch("c:ser", 0, None, Sequence, None, 2),
        ch("c:dLbls", 0, Some(1), Sequence, None, 3),
        ch("c:dropLines", 0, Some(1), Sequence, None, 4),
        ch("c:axId", 2, Some(2), Sequence, None, 5),
        ch("c:extLst", 0, Some(1), Sequence, None, 6),
    ]),
    ty("c:CT_AreaSer", &[
        ch("c:idx", 1, Some(1), Sequence, None, 0),
        ch("c:order", 1, Some(1), Sequence, None, 1),
        ch("c:tx", 0, Some(1), Sequence, None, 2),
        ch("c:spPr", 0, Some(1), Sequence, None, 3),
        ch("c:pictureOptions", 0, Some(1), Sequence, None, 4),
        ch("c:dPt", 0, None, Sequence, None, 5),
        ch("c:dLbls", 0, Some(1), Sequence, None, 6),
        ch("c:trendline", 0, None, Sequence, None, 7),
        ch("c:errBars", 0, Some(2), Sequence, None, 8),
        ch("c:cat", 0, Some(1), Sequence, None, 9),
        ch("c:val", 0, Some(1), Sequence, None, 10),
        ch("c:extLst", 0, Some(1), Sequence, None, 11),
    ]),
    ty("c:CT_AxDataSource", &[
        ch("c:multiLvlStrRef", 1, Some(1), Choice, Some("choice1"), 0),
        ch("c:numRef", 1, Some(1), Choice, Some("choice1"), 1),
        ch("c:numLit", 1, Some(1), Choice, Some("choice1"), 2),
        ch("c:strRef", 1, Some(1), Choice, Some("choice1"), 3),
        ch("c:strLit", 1, Some(1), Choice, Some("choice1"), 4),
    ]),
    ty("c:CT_AxPos", &[]),
    ty("c:CT_AxisUnit", &[]),
    ty("c:CT_BandFmt", &[
        ch("c:idx", 1, Some(1), Sequence, None, 0),
        ch("c:spPr", 0, Some(1), Sequence, None, 1),
    ]),
    ty("c:CT_BandFmts", &[
        ch("c:bandFmt", 0, None, Sequence, None, 0),
    ]),
    ty("c:CT_Bar3DChart", &[
        ch("c:barDir", 1, Some(1), Sequence, None, 0),
        ch("c:grouping", 0, Some(1), Sequence, None, 1),
        ch("c:varyColors", 0, Some(1), Sequence, None, 2),
        ch("c:ser", 0, None, Sequence, None, 3),
        ch("c:dLbls", 0, Some(1), Sequence, None, 4),
        ch("c:gapWidth", 0, Some(1), Sequence, None, 5),
        ch("c:gapDepth", 0, Some(1), Sequence, None, 6),
        ch("c:shape", 0, Some(1), Sequence, None, 7),
        ch("c:axId", 2, Some(3), Sequence, None, 8),
        ch("c:extLst", 0, Some(1), Sequence, None, 9),
    ]),
    ty("c:CT_BarChart", &[
        ch("c:barDir", 1, Some(1), Sequence, None, 0),
        ch("c:grouping", 0, Some(1), Sequence, None, 1),
        ch("c:varyColors", 0, Some(1), Sequence, None, 2),
        ch("c:ser", 0, None, Sequence, None, 3),
        ch("c:dLbls", 0, Some(1), Sequence, None, 4),
        ch("c:gapWidth", 0, Some(1), Sequence, None, 5),
        ch("c:overlap", 0, Some(1), Sequence, None, 6),
        ch("c:serLines", 0, None, Sequence, None, 7),
        ch("c:axId", 2, Some(2), Sequence, None, 8),
        ch("c:extLst", 0, Some(1), Sequence, None, 9),
    ]),
    ty("c:CT_BarDir", &[]),
    ty("c:CT_BarGrouping", &[]),
    ty("c:CT_BarSer", &[
        ch("c:idx", 1, Some(1), Sequence, None, 0),
        ch("c:order", 1, Some(1), Sequence, None, 1),
        ch("c:tx", 0, Some(1), Sequence, None, 2),
        ch("c:spPr", 0, Some(1), Sequence, None, 3),
        ch("c:invertIfNegative", 0, Some(1), Sequence, None, 4),
        ch("c:pictureOptions", 0, Some(1), Sequence, None, 5),
        ch("c:dPt", 0, None, Sequence, None, 6),
        ch("c:dLbls", 0, Some(1), Sequence, None, 7),
        ch("c:trendline", 0, None, Sequence, None, 8),
        ch("c:errBars", 0, Some(1), Sequence, None, 9),
        ch("c:cat", 0, Some(1), Sequence, None, 10),
        ch("c:val", 0, Some(1), Sequence, None, 11),
        ch("c:shape", 0, Some(1), Sequence, None, 12),
        ch("c:extLst", 0, Some(1), Sequence, None, 13),
    ]),
    ty("c:CT_Boolean", &[]),
    ty("c:CT_BubbleChart", &[
        ch("c:varyColors", 0, Some(1), Sequence, None, 0),
        ch("c:ser", 0, None, Sequence, None, 1),
        ch("c:dLbls", 0, Some(1), Sequence, None, 2),
        ch("c:bubble3D", 0, Some(1), Sequence, None, 3),
        ch("c:bubbleScale", 0, Some(1), Sequence, None, 4),
        ch("c:showNegBubbles", 0, Some(1), Sequence, None, 5),
        ch("c:sizeRepresents", 0, Some(1), Sequence, None, 6),
        ch("c:axId", 2, Some(2), Sequence, None, 7),
        ch("c:extLst", 0, Some(1), Sequence, None, 8),
    ]),
    ty("c:CT_BubbleScale", &[]),
    ty("c:CT_BubbleSer", &[
        ch("c:idx", 1, Some(1), Sequence, None, 0),
        ch("c:order", 1, Some(1), Sequence, None, 1),
        ch("c:tx", 0, Some(1), Sequence, None, 2),
        ch("c:spPr", 0, Some(1), Sequence, None, 3),
        ch("c:invertIfNegative", 0, Some(1), Sequence, None, 4),
        ch("c:dPt", 0, None, Sequence, None, 5),
        ch("c:dLbls", 0, Some(1), Sequence, None, 6),
        ch("c:trendline", 0, None, Sequence, None, 7),
        ch("c:errBars", 0, Some(2), Sequence, None, 8),
        ch("c:xVal", 0, Some(1), Sequence, None, 9),
        ch("c:yVal", 0, Some(1), Sequence, None, 10),
        ch("c:bubbleSize", 0, Some(1), Sequence, None, 11),
        ch("c:bubble3D", 0, Some(1), Sequence, None, 12),
        ch("c:extLst", 0, Some(1), Sequence, None, 13),
    ]),
    ty("c:CT_BuiltInUnit", &[]),
    ty("c:CT_CatAx", &[
        ch("c:axId", 1, Some(1), Sequence, None, 0),
        ch("c:scaling", 1, Some(1), Sequence, None, 1),
        ch("c:delete", 0, Some(1), Sequence, None, 2),
        ch("c:axPos", 1, Some(1), Sequence, None, 3),
        ch("c:majorGridlines", 0, Some(1), Sequence, None, 4),
        ch("c:minorGridlines", 0, Some(1), Sequence, None, 5),
        ch("c:title", 0, Some(1), Sequence, None, 6),
        ch("c:numFmt", 0, Some(1), Sequence, None, 7),
        ch("c:majorTickMark", 0, Some(1), Sequence, None, 8),
        ch("c:minorTickMark", 0, Some(1), Sequence, None, 9),
        ch("c:tickLblPos", 0, Some(1), Sequence, None, 10),
        ch("c:spPr", 0, Some(1), Sequence, None, 11),
        ch("c:txPr", 0, Some(1), Sequence, None, 12),
        ch("c:crossAx", 1, Some(1), Sequence, None, 13),
        ch("c:crosses", 0, Some(1), Choice, Some("choice1"), 14),
        ch("c:crossesAt", 0, Some(1), Choice, Some("choice1"), 15),
        ch("c:auto", 0, Some(1), Sequence, None, 16),
        ch("c:lblAlgn", 0, Some(1), Sequence, None, 17),
        ch("c:lblOffset", 0, Some(1), Sequence, None, 18),
        ch("c:tickLblSkip", 0, Some(1), Sequence, None, 19),
        ch("c:tickMarkSkip", 0, Some(1), Sequence, None, 20),
        ch("c:noMultiLvlLbl", 0, Some(1), Sequence, None, 21),
        ch("c:extLst", 0, Some(1), Sequence, None, 22),
    ]),
    ty("c:CT_Chart", &[
        ch("c:title", 0, Some(1), Sequence, None, 0),
        ch("c:autoTitleDeleted", 0, Some(1), Sequence, None, 1),
        ch("c:pivotFmts", 0, Some(1), Sequence, None, 2),
        ch("c:view3D", 0, Some(1), Sequence, None, 3),
        ch("c:floor", 0, Some(1), Sequence, None, 4),
        ch("c:sideWall", 0, Some(1), Sequence, None, 5),
        ch("c:backWall", 0, Some(1), Sequence, None, 6),
        ch("c:plotArea", 1, Some(1), Sequence, None, 7),
        ch("c:legend", 0, Some(1), Sequence, None, 8),
        ch("c:plotVisOnly", 0, Some(1), Sequence, None, 9),
        ch("c:dispBlanksAs", 0, Some(1), Sequence, None, 10),
        ch("c:showDLblsOverMax", 0, Some(1), Sequence, None, 11),
        ch("c:extLst", 0, Some(1), Sequence, None, 12),
    ]),
    ty("c:CT_ChartLines", &[
        ch("c:spPr", 0, Some(1), Sequence, None, 0),
    ]),
    ty("c:CT_ChartSpace", &[
        ch("c:date1904", 0, Some(1), Sequence, None, 0),
        ch("c:lang", 0, Some(1), Sequence, None, 1),
        ch("c:roundedCorners", 0, Some(1), Sequence, None, 2),
        ch("c:style", 0, Some(1), Sequence, None, 3),
        ch("c:clrMapOvr", 0, Some(1), Sequence, None, 4),
        ch("c:pivotSource", 0, Some(1), Sequence, None, 5),
        ch("c:protection", 0, Some(1), Sequence, None, 6),
        ch("c:chart", 1, Some(1), Sequence, None, 7),
        ch("c:spPr", 0, Some(1), Sequence, None, 8),
        ch("c:txPr", 0, Some(1), Sequence, None, 9),
        ch("c:externalData", 0, Some(1), Sequence, None, 10),
        ch("c:printSettings", 0, Some(1), Sequence, None, 11),
        ch("c:userShapes", 0, Some(1), Sequence, None, 12),
        ch("c:extLst", 0, Some(1), Sequence, None, 13),
    ]),
    ty("c:CT_CrossBetween", &[]),
    ty("c:CT_Crosses", &[]),
    ty("c:CT_CustSplit", &[
        ch("c:secondPiePt", 0, None, Sequence, None, 0),
    ]),
    ty("c:CT_DLbl", &[
        ch("c:idx", 1, Some(1), Sequence, None, 0),
        ch("c:delete", 1, Some(1), Choice, Some("choice1"), 1),
        ch("c:layout", 0, Some(1), Sequence, Some("choice1"), 2),
        ch("c:tx", 0, Some(1), Sequence, Some("choice1"), 3),
        ch("c:numFmt", 0, Some(1), Sequence, Some("choice1"), 4),
        ch("c:spPr", 0, Some(1), Sequence, Some("choice1"), 5),
        ch("c:txPr", 0, Some(1), Sequence, Some("choice1"), 6),
        ch("c:dLblPos", 0, Some(1), Sequence, Some("choice1"), 7),
        ch("c:showLegendKey", 0, Some(1), Sequence, Some("choice1"), 8),
        ch("c:showVal", 0, Some(1), Sequence, Some("choice1"), 9),
        ch("c:showCatName", 0, Some(1), Sequence, Some("choice1"), 10),
        ch("c:showSerName", 0, Some(1), Sequence, Some("choice1"), 11),
        ch("c:showPercent", 0, Some(1), Sequence, Some("choice1"), 12),
        ch("c:showBubbleSize", 0, Some(1), Sequence, Some("choice1"), 13),
        ch("c:separator", 0, Some(1), Sequence, Some("choice1"), 14),
        ch("c:extLst", 0, Some(1), Sequence, None, 15),
    ]),
    ty("c:CT_DLblPos", &[]),
    ty("c:CT_DLbls", &[
        ch("c:dLbl", 0, None, Sequence, None, 0),
        ch("c:delete", 1, Some(1), Choice, Some("choice1"), 1),
        ch("c:numFmt", 0, Some(1), Sequence, Some("choice1"), 2),
        ch("c:spPr", 0, Some(1), Sequence, Some("choice1"), 3),
        ch("c:txPr", 0, Some(1), Sequence, Some("choice1"), 4),
        ch("c:dLblPos", 0, Some(1), Sequence, Some("choice1"), 5),
        ch("c:showLegendKey", 0, Some(1), Sequence, Some("choice1"), 6),
        ch("c:showVal", 0, Some(1), Sequence, Some("choice1"), 7),
        ch("c:showCatName", 0, Some(1), Sequence, Some("choice1"), 8),
        ch("c:showSerName", 0, Some(1), Sequence, Some("choice1"), 9),
        ch("c:showPercent", 0, Some(1), Sequence, Some("choice1"), 10),
        ch("c:showBubbleSize", 0, Some(1), Sequence, Some("choice1"), 11),
        ch("c:separator", 0, Some(1), Sequence, Some("choice1"), 12),
        ch("c:showLeaderLines", 0, Some(1), Sequence, Some("choice1"), 13),
        ch("c:leaderLines", 0, Some(1), Sequence, Some("choice1"), 14),
        ch("c:extLst", 0, Some(1), Sequence, None, 15),
    ]),
    ty("c:CT_DPt", &[
        ch("c:idx", 1, Some(1), Sequence, None, 0),
        ch("c:invertIfNegative", 0, Some(1), Sequence, None, 1),
        ch("c:marker", 0, Some(1), Sequence, None, 2),
        ch("c:bubble3D", 0, Some(1), Sequence, None, 3),
        ch("c:explosion", 0, Some(1), Sequence, None, 4),
        ch("c:spPr", 0, Some(1), Sequence, None, 5),
        ch("c:pictureOptions", 0, Some(1), Sequence, None, 6),
        ch("c:extLst", 0, Some(1), Sequence, None, 7),
    ]),
    ty("c:CT_DTable", &[
        ch("c:showHorzBorder", 0, Some(1), Sequence, None, 0),
        ch("c:showVertBorder", 0, Some(1), Sequence, None, 1),
        ch("c:showOutline", 0, Some(1), Sequence, None, 2),
        ch("c:showKeys", 0, Some(1), Sequence, None, 3),
        ch("c:spPr", 0, Some(1), Sequence, None, 4),
        ch("c:txPr", 0, Some(1), Sequence, None, 5),
        ch("c:extLst", 0, Some(1), Sequence, None, 6),
    ]),
    ty("c:CT_DateAx", &[
        ch("c:axId", 1, Some(1), Sequence, None, 0),
        ch("c:scaling", 1, Some(1), Sequence, None, 1),
        ch("c:delete", 0, Some(1), Sequence, None, 2),
        ch("c:axPos", 1, Some(1), Sequence, None, 3),
        ch("c:majorGridlines", 0, Some(1), Sequence, None, 4),
        ch("c:minorGridlines", 0, Some(1), Sequence, None, 5),
        ch("c:title", 0, Some(1), Sequence, None, 6),
        ch("c:numFmt", 0, Some(1), Sequence, None, 7),
        ch("c:majorTickMark", 0, Some(1), Sequence, None, 8),
        ch("c:minorTickMark", 0, Some(1), Sequence, None, 9),
        ch("c:tickLblPos", 0, Some(1), Sequence, None, 10),
        ch("c:spPr", 0, Some(1), Sequence, None, 11),
        ch("c:txPr", 0, Some(1), Sequence, None, 12),
        ch("c:crossAx", 1, Some(1), Sequence, None, 13),
        ch("c:crosses", 0, Some(1), Choice, Some("choice1"), 14),
        ch("c:crossesAt", 0, Some(1), Choice, Some("choice1"), 15),
        ch("c:auto", 0, Some(1), Sequence, None, 16),
        ch("c:lblOffset", 0, Some(1), Sequence, None, 17),
        ch("c:baseTimeUnit", 0, Some(1), Sequence, None, 18),
        ch("c:majorUnit", 0, Some(1), Sequence, None, 19),
        ch("c:majorTimeUnit", 0, Some(1), Sequence, None, 20),
        ch("c:minorUnit", 0, Some(1), Sequence, None, 21),
        ch("c:minorTimeUnit", 0, Some(1), Sequence, None, 22),
        ch("c:extLst", 0, Some(1), Sequence, None, 23),
    ]),
    ty("c:CT_DepthPercent", &[]),
    ty("c:CT_DispBlanksAs", &[]),
    ty("c:CT_DispUnits", &[
        ch("c:custUnit", 1, Some(1), Choice, Some("choice1"), 0),
        ch("c:builtInUnit", 1, Some(1), Choice, Some("choice1"), 1),
        ch("c:dispUnitsLbl", 0, Some(1), Sequence, None, 2),
        ch("c:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("c:CT_DispUnitsLbl", &[
        ch("c:layout", 0, Some(1), Sequence, None, 0),
        ch("c:tx", 0, Some(1), Sequence, None, 1),
        ch("c:spPr", 0, Some(1), Sequence, None, 2),
        ch("c:txPr", 0, Some(1), Sequence, None, 3),
    ]),
    ty("c:CT_Double", &[]),
    ty("c:CT_DoughnutChart", &[
        ch("c:varyColors", 0, Some(1), Sequence, None, 0),
        ch("c:ser", 0, None, Sequence, None, 1),
        ch("c:dLbls", 0, Some(1), Sequence, None, 2),
        ch("c:firstSliceAng", 0, Some(1), Sequence, None, 3),
        ch("c:holeSize", 0, Some(1), Sequence, None, 4),
        ch("c:extLst", 0, Some(1), Sequence, None, 5),
    ]),
    ty("c:CT_ErrBarType", &[]),
    ty("c:CT_ErrBars", &[
        ch("c:errDir", 0, Some(1), Sequence, None, 0),
        ch("c:errBarType", 1, Some(1), Sequence, None, 1),
        ch("c:errValType", 1, Some(1), Sequence, None, 2),
        ch("c:noEndCap", 0, Some(1), Sequence, None, 3),
        ch("c:plus", 0, Some(1), Sequence, None, 4),
        ch("c:minus", 0, Some(1), Sequence, None, 5),
        ch("c:val", 0, Some(1), Sequence, None, 6),
        ch("c:spPr", 0, Some(1), Sequence, None, 7),
        ch("c:extLst", 0, Some(1), Sequence, None, 8),
    ]),
    ty("c:CT_ErrDir", &[]),
    ty("c:CT_ErrValType", &[]),
    ty("c:CT_Extension", &[
        ch("<any>", 1, Some(1), Sequence, None, 0),
    ]),
    ty("c:CT_ExtensionList", &[
        ch("c:ext", 0, None, Sequence, None, 0),
    ]),
    ty("c:CT_ExternalData", &[
        ch("c:autoUpdate", 0, Some(1), Sequence, None, 0),
    ]),
    ty("c:CT_FirstSliceAng", &[]),
    ty("c:CT_GapAmount", &[]),
    ty("c:CT_Grouping", &[]),
    ty("c:CT_HPercent", &[]),
    ty("c:CT_HeaderFooter", &[
        ch("c:oddHeader", 0, Some(1), Sequence, None, 0),
        ch("c:oddFooter", 0, Some(1), Sequence, None, 1),
        ch("c:evenHeader", 0, Some(1), Sequence, None, 2),
        ch("c:evenFooter", 0, Some(1), Sequence, None, 3),
        ch("c:firstHeader", 0, Some(1), Sequence, None, 4),
        ch("c:firstFooter", 0, Some(1), Sequence, None, 5),
    ]),
    ty("c:CT_HoleSize", &[]),
    ty("c:CT_Layout", &[
        ch("c:manualLayout", 0, Some(1), Sequence, None, 0),
        ch("c:extLst", 0, Some(1), Sequence, None, 1),
    ]),
    ty("c:CT_LayoutMode", &[]),
    ty("c:CT_LayoutTarget", &[]),
    ty("c:CT_LblAlgn", &[]),
    ty("c:CT_LblOffset", &[]),
    ty("c:CT_Legend", &[
        ch("c:legendPos", 0, Some(1), Sequence, None, 0),
        ch("c:legendEntry", 0, None, Sequence, None, 1),
        ch("c:layout", 0, Some(1), Sequence, None, 2),
        ch("c:overlay", 0, Some(1), Sequence, None, 3),
        ch("c:spPr", 0, Some(1), Sequence, None, 4),
        ch("c:txPr", 0, Some(1), Sequence, None, 5),
        ch("c:extLst", 0, Some(1), Sequence, None, 6),
    ]),
    ty("c:CT_LegendEntry", &[
        ch("c:idx", 1, Some(1), Sequence, None, 0),
        ch("c:delete", 1, Some(1), Choice, Some("choice1"), 1),
        ch("c:txPr", 0, Some(1), Sequence, Some("choice1"), 2),
        ch("c:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("c:CT_LegendPos", &[]),
    ty("c:CT_Line3DChart", &[
        ch("c:grouping", 1, Some(1), Sequence, None, 0),
        ch("c:varyColors", 0, Some(1), Sequence, None, 1),
        ch("c:ser", 0, None, Sequence, None, 2),
        ch("c:dLbls", 0, Some(1), Sequence, None, 3),
        ch("c:dropLines", 0, Some(1), Sequence, None, 4),
        ch("c:gapDepth", 0, Some(1), Sequence, None, 5),
        ch("c:axId", 3, Some(3), Sequence, None, 6),
        ch("c:extLst", 0, Some(1), Sequence, None, 7),
    ]),
    ty("c:CT_LineChart", &[
        ch("c:grouping", 1, Some(1), Sequence, None, 0),
        ch("c:varyColors", 0, Some(1), Sequence, None, 1),
        ch("c:ser", 0, None, Sequence, None, 2),
        ch("c:dLbls", 0, Some(1), Sequence, None, 3),
        ch("c:dropLines", 0, Some(1), Sequence, None, 4),
        ch("c:hiLowLines", 0, Some(1), Sequence, None, 5),
        ch("c:upDownBars", 0, Some(1), Sequence, None, 6),
        ch("c:marker", 0, Some(1), Sequence, None, 7),
        ch("c:smooth", 0, Some(1), Sequence, None, 8),
        ch("c:axId", 2, Some(2), Sequence, None, 9),
        ch("c:extLst", 0, Some(1), Sequence, None, 10),
    ]),
    ty("c:CT_LineSer", &[
        ch("c:idx", 1, Some(1), Sequence, None, 0),
        ch("c:order", 1, Some(1), Sequence, None, 1),
        ch("c:tx", 0, Some(1), Sequence, None, 2),
        ch("c:spPr", 0, Some(1), Sequence, None, 3),
        ch("c:marker", 0, Some(1), Sequence, None, 4),
        ch("c:dPt", 0, None, Sequence, None, 5),
        ch("c:dLbls", 0, Some(1), Sequence, None, 6),
        ch("c:trendline", 0, None, Sequence, None, 7),
        ch("c:errBars", 0, Some(1), Sequence, None, 8),
        ch("c:cat", 0, Some(1), Sequence, None, 9),
        ch("c:val", 0, Some(1), Sequence, None, 10),
        ch("c:smooth", 0, Some(1), Sequence, None, 11),
        ch("c:extLst", 0, Some(1), Sequence, None, 12),
    ]),
    ty("c:CT_LogBase", &[]),
    ty("c:CT_Lvl", &[
        ch("c:pt", 0, None, Sequence, None, 0),
    ]),
    ty("c:CT_ManualLayout", &[
        ch("c:layoutTarget", 0, Some(1), Sequence, None, 0),
        ch("c:xMode", 0, Some(1), Sequence, None, 1),
        ch("c:yMode", 0, Some(1), Sequence, None, 2),
        ch("c:wMode", 0, Some(1), Sequence, None, 3),
        ch("c:hMode", 0, Some(1), Sequence, None, 4),
        ch("c:x", 0, Some(1), Sequence, None, 5),
        ch("c:y", 0, Some(1), Sequence, None, 6),
        ch("c:w", 0, Some(1), Sequence, None, 7),
        ch("c:h", 0, Some(1), Sequence, None, 8),
        ch("c:extLst", 0, Some(1), Sequence, None, 9),
    ]),
    ty("c:CT_Marker", &[
        ch("c:symbol", 0, Some(1), Sequence, None, 0),
        ch("c:size", 0, Some(1), Sequence, None, 1),
        ch("c:spPr", 0, Some(1), Sequence, None, 2),
        ch("c:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("c:CT_MarkerSize", &[]),
    ty("c:CT_MarkerStyle", &[]),
    ty("c:CT_MultiLvlStrData", &[
        ch("c:ptCount", 0, Some(1), Sequence, None, 0),
        ch("c:lvl", 0, None, Sequence, None, 1),
        ch("c:extLst", 0, Some(1), Sequence, None, 2),
    ]),
    ty("c:CT_MultiLvlStrRef", &[
        ch("c:f", 1, Some(1), Sequence, None, 0),
        ch("c:multiLvlStrCache", 0, Some(1), Sequence, None, 1),
        ch("c:extLst", 0, Some(1), Sequence, None, 2),
    ]),
    ty("c:CT_NumData", &[
        ch("c:formatCode", 0, Some(1), Sequence, None, 0),
        ch("c:ptCount", 0, Some(1), Sequence, None, 1),
        ch("c:pt", 0, None, Sequence, None, 2),
        ch("c:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("c:CT_NumDataSource", &[
        ch("c:numRef", 1, Some(1), Choice, Some("choice1"), 0),
        ch("c:numLit", 1, Some(1), Choice, Some("choice1"), 1),
    ]),
    ty("c:CT_NumFmt", &[]),
    ty("c:CT_NumRef", &[
        ch("c:f", 1, Some(1), Sequence, None, 0),
        ch("c:numCache", 0, Some(1), Sequence, None, 1),
        ch("c:extLst", 0, Some(1), Sequence, None, 2),
    ]),
    ty("c:CT_NumVal", &[
        ch("c:v", 1, Some(1), Sequence, None, 0),
    ]),
    ty("c:CT_OfPieChart", &[
        ch("c:ofPieType", 1, Some(1), Sequence, None, 0),
        ch("c:varyColors", 0, Some(1), Sequence, None, 1),
        ch("c:ser", 0, None, Sequence, None, 2),
        ch("c:dLbls", 0, Some(1), Sequence, None, 3),
        ch("c:gapWidth", 0, Some(1), Sequence, None, 4),
        ch("c:splitType", 0, Some(1), Sequence, None, 5),
        ch("c:splitPos", 0, Some(1), Sequence, None, 6),
        ch("c:custSplit", 0, Some(1), Sequence, None, 7),
        ch("c:secondPieSize", 0, Some(1), Sequence, None, 8),
        ch("c:serLines", 0, None, Sequence, None, 9),
        ch("c:extLst", 0, Some(1), Sequence, None, 10),
    ]),
    ty("c:CT_OfPieType", &[]),
    ty("c:CT_Order", &[]),
    ty("c:CT_Orientation", &[]),
    ty("c:CT_Overlap", &[]),
    ty("c:CT_PageMargins", &[]),
    ty("c:CT_PageSetup", &[]),
    ty("c:CT_Period", &[]),
    ty("c:CT_Perspective", &[]),
    ty("c:CT_PictureFormat", &[]),
    ty("c:CT_PictureOptions", &[
        ch("c:applyToFront", 0, Some(1), Sequence, None, 0),
        ch("c:applyToSides", 0, Some(1), Sequence, None, 1),
        ch("c:applyToEnd", 0, Some(1), Sequence, None, 2),
        ch("c:pictureFormat", 0, Some(1), Sequence, None, 3),
        ch("c:pictureStackUnit", 0, Some(1), Sequence, None, 4),
    ]),
    ty("c:CT_PictureStackUnit", &[]),
    ty("c:CT_Pie3DChart", &[
        ch("c:varyColors", 0, Some(1), Sequence, None, 0),
        ch("c:ser", 0, None, Sequence, None, 1),
        ch("c:dLbls", 0, Some(1), Sequence, None, 2),
        ch("c:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("c:CT_PieChart", &[
        ch("c:varyColors", 0, Some(1), Sequence, None, 0),
        ch("c:ser", 0, None, Sequence, None, 1),
        ch("c:dLbls", 0, Some(1), Sequence, None, 2),
        ch("c:firstSliceAng", 0, Some(1), Sequence, None, 3),
        ch("c:extLst", 0, Some(1), Sequence, None, 4),
    ]),
    ty("c:CT_PieSer", &[
        ch("c:idx", 1, Some(1), Sequence, None, 0),
        ch("c:order", 1, Some(1), Sequence, None, 1),
        ch("c:tx", 0, Some(1), Sequence, None, 2),
        ch("c:spPr", 0, Some(1), Sequence, None, 3),
        ch("c:explosion", 0, Some(1), Sequence, None, 4),
        ch("c:dPt", 0, None, Sequence, None, 5),
        ch("c:dLbls", 0, Some(1), Sequence, None, 6),
        ch("c:cat", 0, Some(1), Sequence, None, 7),
        ch("c:val", 0, Some(1), Sequence, None, 8),
        ch("c:extLst", 0, Some(1), Sequence, None, 9),
    ]),
    ty("c:CT_PivotFmt", &[
        ch("c:idx", 1, Some(1), Sequence, None, 0),
        ch("c:spPr", 0, Some(1), Sequence, None, 1),
        ch("c:txPr", 0, Some(1), Sequence, None, 2),
        ch("c:marker", 0, Some(1), Sequence, None, 3),
        ch("c:dLbl", 0, Some(1), Sequence, None, 4),
        ch("c:extLst", 0, Some(1), Sequence, None, 5),
    ]),
    ty("c:CT_PivotFmts", &[
        ch("c:pivotFmt", 0, None, Sequence, None, 0),
    ]),
    ty("c:CT_PivotSource", &[
        ch("c:name", 1, Some(1), Sequence, None, 0),
        ch("c:fmtId", 1, Some(1), Sequence, None, 1),
        ch("c:extLst", 0, None, Sequence, None, 2),
    ]),
    ty("c:CT_PlotArea", &[
        ch("c:layout", 0, Some(1), Sequence, None, 0),
        ch("c:areaChart", 1, None, Choice, Some("choice1"), 1),
        ch("c:area3DChart", 1, None, Choice, Some("choice1"), 2),
        ch("c:lineChart", 1, None, Choice, Some("choice1"), 3),
        ch("c:line3DChart", 1, None, Choice, Some("choice1"), 4),
        ch("c:stockChart", 1, None, Choice, Some("choice1"), 5),
        ch("c:radarChart", 1, None, Choice, Some("choice1"), 6),
        ch("c:scatterChart", 1, None, Choice, Some("choice1"), 7),
        ch("c:pieChart", 1, None, Choice, Some("choice1"), 8),
        ch("c:pie3DChart", 1, None, Choice, Some("choice1"), 9),
        ch("c:doughnutChart", 1, None, Choice, Some("choice1"), 10),
        ch("c:barChart", 1, None, Choice, Some("choice1"), 11),
        ch("c:bar3DChart", 1, None, Choice, Some("choice1"), 12),
        ch("c:ofPieChart", 1, None, Choice, Some("choice1"), 13),
        ch("c:surfaceChart", 1, None, Choice, Some("choice1"), 14),
        ch("c:surface3DChart", 1, None, Choice, Some("choice1"), 15),
        ch("c:bubbleChart", 1, None, Choice, Some("choice1"), 16),
        ch("c:valAx", 0, None, Choice, Some("choice2"), 17),
        ch("c:catAx", 0, None, Choice, Some("choice2"), 18),
        ch("c:dateAx", 0, None, Choice, Some("choice2"), 19),
        ch("c:serAx", 0, None, Choice, Some("choice2"), 20),
        ch("c:dTable", 0, Some(1), Sequence, None, 21),
        ch("c:spPr", 0, Some(1), Sequence, None, 22),
        ch("c:extLst", 0, Some(1), Sequence, None, 23),
    ]),
    ty("c:CT_PrintSettings", &[
        ch("c:headerFooter", 0, Some(1), Sequence, None, 0),
        ch("c:pageMargins", 0, Some(1), Sequence, None, 1),
        ch("c:pageSetup", 0, Some(1), Sequence, None, 2),
    ]),
    ty("c:CT_Protection", &[
        ch("c:chartObject", 0, Some(1), Sequence, None, 0),
        ch("c:data", 0, Some(1), Sequence, None, 1),
        ch("c:formatting", 0, Some(1), Sequence, None, 2),
        ch("c:selection", 0, Some(1), Sequence, None, 3),
        ch("c:userInterface", 0, Some(1), Sequence, None, 4),
    ]),
    ty("c:CT_RadarChart", &[
        ch("c:radarStyle", 1, Some(1), Sequence, None, 0),
        ch("c:varyColors", 0, Some(1), Sequence, None, 1),
        ch("c:ser", 0, None, Sequence, None, 2),
        ch("c:dLbls", 0, Some(1), Sequence, None, 3),
        ch("c:axId", 2, Some(2), Sequence, None, 4),
        ch("c:extLst", 0, Some(1), Sequence, None, 5),
    ]),
    ty("c:CT_RadarSer", &[
        ch("c:idx", 1, Some(1), Sequence, None, 0),
        ch("c:order", 1, Some(1), Sequence, None, 1),
        ch("c:tx", 0, Some(1), Sequence, None, 2),
        ch("c:spPr", 0, Some(1), Sequence, None, 3),
        ch("c:marker", 0, Some(1), Sequence, None, 4),
        ch("c:dPt", 0, None, Sequence, None, 5),
        ch("c:dLbls", 0, Some(1), Sequence, None, 6),
        ch("c:cat", 0, Some(1), Sequence, None, 7),
        ch("c:val", 0, Some(1), Sequence, None, 8),
        ch("c:extLst", 0, Some(1), Sequence, None, 9),
    ]),
    ty("c:CT_RadarStyle", &[]),
    ty("c:CT_RelId", &[]),
    ty("c:CT_RotX", &[]),
    ty("c:CT_RotY", &[]),
    ty("c:CT_Scaling", &[
        ch("c:logBase", 0, Some(1), Sequence, None, 0),
        ch("c:orientation", 0, Some(1), Sequence, None, 1),
        ch("c:max", 0, Some(1), Sequence, None, 2),
        ch("c:min", 0, Some(1), Sequence, None, 3),
        ch("c:extLst", 0, Some(1), Sequence, None, 4),
    ]),
    ty("c:CT_ScatterChart", &[
        ch("c:scatterStyle", 1, Some(1), Sequence, None, 0),
        ch("c:varyColors", 0, Some(1), Sequence, None, 1),
        ch("c:ser", 0, None, Sequence, None, 2),
        ch("c:dLbls", 0, Some(1), Sequence, None, 3),
        ch("c:axId", 2, Some(2), Sequence, None, 4),
        ch("c:extLst", 0, Some(1), Sequence, None, 5),
    ]),
    ty("c:CT_ScatterSer", &[
        ch("c:idx", 1, Some(1), Sequence, None, 0),
        ch("c:order", 1, Some(1), Sequence, None, 1),
        ch("c:tx", 0, Some(1), Sequence, None, 2),
        ch("c:spPr", 0, Some(1), Sequence, None, 3),
        ch("c:marker", 0, Some(1), Sequence, None, 4),
        ch("c:dPt", 0, None, Sequence, None, 5),
        ch("c:dLbls", 0, Some(1), Sequence, None, 6),
        ch("c:trendline", 0, None, Sequence, None, 7),
        ch("c:errBars", 0, Some(2), Sequence, None, 8),
        ch("c:xVal", 0, Some(1), Sequence, None, 9),
        ch("c:yVal", 0, Some(1), Sequence, None, 10),
        ch("c:smooth", 0, Some(1), Sequence, None, 11),
        ch("c:extLst", 0, Some(1), Sequence, None, 12),
    ]),
    ty("c:CT_ScatterStyle", &[]),
    ty("c:CT_SecondPieSize", &[]),
    ty("c:CT_SerAx", &[
        ch("c:axId", 1, Some(1), Sequence, None, 0),
        ch("c:scaling", 1, Some(1), Sequence, None, 1),
        ch("c:delete", 0, Some(1), Sequence, None, 2),
        ch("c:axPos", 1, Some(1), Sequence, None, 3),
        ch("c:majorGridlines", 0, Some(1), Sequence, None, 4),
        ch("c:minorGridlines", 0, Some(1), Sequence, None, 5),
        ch("c:title", 0, Some(1), Sequence, None, 6),
        ch("c:numFmt", 0, Some(1), Sequence, None, 7),
        ch("c:majorTickMark", 0, Some(1), Sequence, None, 8),
        ch("c:minorTickMark", 0, Some(1), Sequence, None, 9),
        ch("c:tickLblPos", 0, Some(1), Sequence, None, 10),
        ch("c:spPr", 0, Some(1), Sequence, None, 11),
        ch("c:txPr", 0, Some(1), Sequence, None, 12),
        ch("c:crossAx", 1, Some(1), Sequence, None, 13),
        ch("c:crosses", 0, Some(1), Choice, Some("choice1"), 14),
        ch("c:crossesAt", 0, Some(1), Choice, Some("choice1"), 15),
        ch("c:tickLblSkip", 0, Some(1), Sequence, None, 16),
        ch("c:tickMarkSkip", 0, Some(1), Sequence, None, 17),
        ch("c:extLst", 0, Some(1), Sequence, None, 18),
    ]),
    ty("c:CT_SerTx", &[
        ch("c:strRef", 1, Some(1), Choice, Some("choice1"), 0),
        ch("c:v", 1, Some(1), Choice, Some("choice1"), 1),
    ]),
    ty("c:CT_Shape", &[]),
    ty("c:CT_SizeRepresents", &[]),
    ty("c:CT_Skip", &[]),
    ty("c:CT_SplitType", &[]),
    ty("c:CT_StockChart", &[
        ch("c:ser", 3, Some(4), Sequence, None, 0),
        ch("c:dLbls", 0, Some(1), Sequence, None, 1),
        ch("c:dropLines", 0, Some(1), Sequence, None, 2),
        ch("c:hiLowLines", 0, Some(1), Sequence, None, 3),
        ch("c:upDownBars", 0, Some(1), Sequence, None, 4),
        ch("c:axId", 2, Some(2), Sequence, None, 5),
        ch("c:extLst", 0, Some(1), Sequence, None, 6),
    ]),
    ty("c:CT_StrData", &[
        ch("c:ptCount", 0, Some(1), Sequence, None, 0),
        ch("c:pt", 0, None, Sequence, None, 1),
        ch("c:extLst", 0, Some(1), Sequence, None, 2),
    ]),
    ty("c:CT_StrRef", &[
        ch("c:f", 1, Some(1), Sequence, None, 0),
        ch("c:strCache", 0, Some(1), Sequence, None, 1),
        ch("c:extLst", 0, Some(1), Sequence, None, 2),
    ]),
    ty("c:CT_StrVal", &[
        ch("c:v", 1, Some(1), Sequence, None, 0),
    ]),
    ty("c:CT_Style", &[]),
    ty("c:CT_Surface", &[
        ch("c:thickness", 0, Some(1), Sequence, None, 0),
        ch("c:spPr", 0, Some(1), Sequence, None, 1),
        ch("c:pictureOptions", 0, Some(1), Sequence, None, 2),
        ch("c:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("c:CT_Surface3DChart", &[
        ch("c:wireframe", 0, Some(1), Sequence, None, 0),
        ch("c:ser", 0, None, Sequence, None, 1),
        ch("c:bandFmts", 0, Some(1), Sequence, None, 2),
        ch("c:axId", 3, Some(3), Sequence, None, 3),
        ch("c:extLst", 0, Some(1), Sequence, None, 4),
    ]),
    ty("c:CT_SurfaceChart", &[
        ch("c:wireframe", 0, Some(1), Sequence, None, 0),
        ch("c:ser", 0, None, Sequence, None, 1),
        ch("c:bandFmts", 0, Some(1), Sequence, None, 2),
        ch("c:axId", 2, Some(3), Sequence, None, 3),
        ch("c:extLst", 0, Some(1), Sequence, None, 4),
    ]),
    ty("c:CT_SurfaceSer", &[
        ch("c:idx", 1, Some(1), Sequence, None, 0),
        ch("c:order", 1, Some(1), Sequence, None, 1),
        ch("c:tx", 0, Some(1), Sequence, None, 2),
        ch("c:spPr", 0, Some(1), Sequence, None, 3),
        ch("c:cat", 0, Some(1), Sequence, None, 4),
        ch("c:val", 0, Some(1), Sequence, None, 5),
        ch("c:extLst", 0, Some(1), Sequence, None, 6),
    ]),
    ty("c:CT_TextLanguageID", &[]),
    ty("c:CT_Thickness", &[]),
    ty("c:CT_TickLblPos", &[]),
    ty("c:CT_TickMark", &[]),
    ty("c:CT_TimeUnit", &[]),
    ty("c:CT_Title", &[
        ch("c:tx", 0, Some(1), Sequence, None, 0),
        ch("c:layout", 0, Some(1), Sequence, None, 1),
        ch("c:overlay", 0, Some(1), Sequence, None, 2),
        ch("c:spPr", 0, Some(1), Sequence, None, 3),
        ch("c:txPr", 0, Some(1), Sequence, None, 4),
        ch("c:extLst", 0, Some(1), Sequence, None, 5),
    ]),
    ty("c:CT_Trendline", &[
        ch("c:name", 0, Some(1), Sequence, None, 0),
        ch("c:spPr", 0, Some(1), Sequence, None, 1),
        ch("c:trendlineType", 1, Some(1), Sequence, None, 2),
        ch("c:order", 0, Some(1), Sequence, None, 3),
        ch("c:period", 0, Some(1), Sequence, None, 4),
        ch("c:forward", 0, Some(1), Sequence, None, 5),
        ch("c:backward", 0, Some(1), Sequence, None, 6),
        ch("c:intercept", 0, Some(1), Sequence, None, 7),
        ch("c:dispRSqr", 0, Some(1), Sequence, None, 8),
        ch("c:dispEq", 0, Some(1), Sequence, None, 9),
        ch("c:trendlineLbl", 0, Some(1), Sequence, None, 10),
        ch("c:extLst", 0, Some(1), Sequence, None, 11),
    ]),
    ty("c:CT_TrendlineLbl", &[
        ch("c:layout", 0, Some(1), Sequence, None, 0),
        ch("c:tx", 0, Some(1), Sequence, None, 1),
        ch("c:numFmt", 0, Some(1), Sequence, None, 2),
        ch("c:spPr", 0, Some(1), Sequence, None, 3),
        ch("c:txPr", 0, Some(1), Sequence, None, 4),
        ch("c:extLst", 0, Some(1), Sequence, None, 5),
    ]),
    ty("c:CT_TrendlineType", &[]),
    ty("c:CT_Tx", &[
        ch("c:strRef", 1, Some(1), Choice, Some("choice1"), 0),
        ch("c:rich", 1, Some(1), Choice, Some("choice1"), 1),
    ]),
    ty("c:CT_UnsignedInt", &[]),
    ty("c:CT_UpDownBar", &[
        ch("c:spPr", 0, Some(1), Sequence, None, 0),
    ]),
    ty("c:CT_UpDownBars", &[
        ch("c:gapWidth", 0, Some(1), Sequence, None, 0),
        ch("c:upBars", 0, Some(1), Sequence, None, 1),
        ch("c:downBars", 0, Some(1), Sequence, None, 2),
        ch("c:extLst", 0, Some(1), Sequence, None, 3),
    ]),
    ty("c:CT_ValAx", &[
        ch("c:axId", 1, Some(1), Sequence, None, 0),
        ch("c:scaling", 1, Some(1), Sequence, None, 1),
        ch("c:delete", 0, Some(1), Sequence, None, 2),
        ch("c:axPos", 1, Some(1), Sequence, None, 3),
        ch("c:majorGridlines", 0, Some(1), Sequence, None, 4),
        ch("c:minorGridlines", 0, Some(1), Sequence, None, 5),
        ch("c:title", 0, Some(1), Sequence, None, 6),
        ch("c:numFmt", 0, Some(1), Sequence, None, 7),
        ch("c:majorTickMark", 0, Some(1), Sequence, None, 8),
        ch("c:minorTickMark", 0, Some(1), Sequence, None, 9),
        ch("c:tickLblPos", 0, Some(1), Sequence, None, 10),
        ch("c:spPr", 0, Some(1), Sequence, None, 11),
        ch("c:txPr", 0, Some(1), Sequence, None, 12),
        ch("c:crossAx", 1, Some(1), Sequence, None, 13),
        ch("c:crosses", 0, Some(1), Choice, Some("choice1"), 14),
        ch("c:crossesAt", 0, Some(1), Choice, Some("choice1"), 15),
        ch("c:crossBetween", 0, Some(1), Sequence, None, 16),
        ch("c:majorUnit", 0, Some(1), Sequence, None, 17),
        ch("c:minorUnit", 0, Some(1), Sequence, None, 18),
        ch("c:dispUnits", 0, Some(1), Sequence, None, 19),
        ch("c:extLst", 0, Some(1), Sequence, None, 20),
    ]),
    ty("c:CT_View3D", &[
        ch("c:rotX", 0, Some(1), Sequence, None, 0),
        ch("c:hPercent", 0, Some(1), Sequence, None, 1),
        ch("c:rotY", 0, Some(1), Sequence, None, 2),
        ch("c:depthPercent", 0, Some(1), Sequence, None, 3),
        ch("c:rAngAx", 0, Some(1), Sequence, None, 4),
        ch("c:perspective", 0, Some(1), Sequence, None, 5),
        ch("c:extLst", 0, Some(1), Sequence, None, 6),
    ]),
    ty("m:CT_Acc", &[
        ch("m:accPr", 0, Some(1), Sequence, None, 0),
        ch("m:e", 1, Some(1), Sequence, None, 1),
    ]),
    ty("m:CT_AccPr", &[
        ch("m:chr", 0, Some(1), Sequence, None, 0),
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 1),
    ]),
    ty("m:CT_Bar", &[
        ch("m:barPr", 0, Some(1), Sequence, None, 0),
        ch("m:e", 1, Some(1), Sequence, None, 1),
    ]),
    ty("m:CT_BarPr", &[
        ch("m:pos", 0, Some(1), Sequence, None, 0),
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 1),
    ]),
    ty("m:CT_BorderBox", &[
        ch("m:borderBoxPr", 0, Some(1), Sequence, None, 0),
        ch("m:e", 1, Some(1), Sequence, None, 1),
    ]),
    ty("m:CT_BorderBoxPr", &[
        ch("m:hideTop", 0, Some(1), Sequence, None, 0),
        ch("m:hideBot", 0, Some(1), Sequence, None, 1),
        ch("m:hideLeft", 0, Some(1), Sequence, None, 2),
        ch("m:hideRight", 0, Some(1), Sequence, None, 3),
        ch("m:strikeH", 0, Some(1), Sequence, None, 4),
        ch("m:strikeV", 0, Some(1), Sequence, None, 5),
        ch("m:strikeBLTR", 0, Some(1), Sequence, None, 6),
        ch("m:strikeTLBR", 0, Some(1), Sequence, None, 7),
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 8),
    ]),
    ty("m:CT_Box", &[
        ch("m:boxPr", 0, Some(1), Sequence, None, 0),
        ch("m:e", 1, Some(1), Sequence, None, 1),
    ]),
    ty("m:CT_BoxPr", &[
        ch("m:opEmu", 0, Some(1), Sequence, None, 0),
        ch("m:noBreak", 0, Some(1), Sequence, None, 1),
        ch("m:diff", 0, Some(1), Sequence, None, 2),
        ch("m:brk", 0, Some(1), Sequence, None, 3),
        ch("m:aln", 0, Some(1), Sequence, None, 4),
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 5),
    ]),
    ty("m:CT_BreakBin", &[]),
    ty("m:CT_BreakBinSub", &[]),
    ty("m:CT_Char", &[]),
    ty("m:CT_CtrlPr", &[
        ch("w:rPr", 0, Some(1), Sequence, Some("choice1"), 0),
        ch("w:ins", 0, Some(1), Choice, Some("choice1"), 1),
        ch("w:del", 0, Some(1), Choice, Some("choice1"), 2),
    ]),
    ty("m:CT_D", &[
        ch("m:dPr", 0, Some(1), Sequence, None, 0),
        ch("m:e", 1, None, Sequence, None, 1),
    ]),
    ty("m:CT_DPr", &[
        ch("m:begChr", 0, Some(1), Sequence, None, 0),
        ch("m:sepChr", 0, Some(1), Sequence, None, 1),
        ch("m:endChr", 0, Some(1), Sequence, None, 2),
        ch("m:grow", 0, Some(1), Sequence, None, 3),
        ch("m:shp", 0, Some(1), Sequence, None, 4),
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 5),
    ]),
    ty("m:CT_EqArr", &[
        ch("m:eqArrPr", 0, Some(1), Sequence, None, 0),
        ch("m:e", 1, None, Sequence, None, 1),
    ]),
    ty("m:CT_EqArrPr", &[
        ch("m:baseJc", 0, Some(1), Sequence, None, 0),
        ch("m:maxDist", 0, Some(1), Sequence, None, 1),
        ch("m:objDist", 0, Some(1), Sequence, None, 2),
        ch("m:rSpRule", 0, Some(1), Sequence, None, 3),
        ch("m:rSp", 0, Some(1), Sequence, None, 4),
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 5),
    ]),
    ty("m:CT_F", &[
        ch("m:fPr", 0, Some(1), Sequence, None, 0),
        ch("m:num", 1, Some(1), Sequence, None, 1),
        ch("m:den", 1, Some(1), Sequence, None, 2),
    ]),
    ty("m:CT_FPr", &[
        ch("m:type", 0, Some(1), Sequence, None, 0),
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 1),
    ]),
    ty("m:CT_FType", &[]),
    ty("m:CT_Func", &[
        ch("m:funcPr", 0, Some(1), Sequence, None, 0),
        ch("m:fName", 1, Some(1), Sequence, None, 1),
        ch("m:e", 1, Some(1), Sequence, None, 2),
    ]),
    ty("m:CT_FuncPr", &[
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 0),
    ]),
    ty("m:CT_GroupChr", &[
        ch("m:groupChrPr", 0, Some(1), Sequence, None, 0),
        ch("m:e", 1, Some(1), Sequence, None, 1),
    ]),
    ty("m:CT_GroupChrPr", &[
        ch("m:chr", 0, Some(1), Sequence, None, 0),
        ch("m:pos", 0, Some(1), Sequence, None, 1),
        ch("m:vertJc", 0, Some(1), Sequence, None, 2),
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 3),
    ]),
    ty("m:CT_Integer2", &[]),
    ty("m:CT_Integer255", &[]),
    ty("m:CT_LimLoc", &[]),
    ty("m:CT_LimLow", &[
        ch("m:limLowPr", 0, Some(1), Sequence, None, 0),
        ch("m:e", 1, Some(1), Sequence, None, 1),
        ch("m:lim", 1, Some(1), Sequence, None, 2),
    ]),
    ty("m:CT_LimLowPr", &[
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 0),
    ]),
    ty("m:CT_LimUpp", &[
        ch("m:limUppPr", 0, Some(1), Sequence, None, 0),
        ch("m:e", 1, Some(1), Sequence, None, 1),
        ch("m:lim", 1, Some(1), Sequence, None, 2),
    ]),
    ty("m:CT_LimUppPr", &[
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 0),
    ]),
    ty("m:CT_M", &[
        ch("m:mPr", 0, Some(1), Sequence, None, 0),
        ch("m:mr", 1, None, Sequence, None, 1),
    ]),
    ty("m:CT_MC", &[
        ch("m:mcPr", 0, Some(1), Sequence, None, 0),
    ]),
    ty("m:CT_MCPr", &[
        ch("m:count", 0, Some(1), Sequence, None, 0),
        ch("m:mcJc", 0, Some(1), Sequence, None, 1),
    ]),
    ty("m:CT_MCS", &[
        ch("m:mc", 1, None, Sequence, None, 0),
    ]),
    ty("m:CT_MPr", &[
        ch("m:baseJc", 0, Some(1), Sequence, None, 0),
        ch("m:plcHide", 0, Some(1), Sequence, None, 1),
        ch("m:rSpRule", 0, Some(1), Sequence, None, 2),
        ch("m:cGpRule", 0, Some(1), Sequence, None, 3),
        ch("m:rSp", 0, Some(1), Sequence, None, 4),
        ch("m:cSp", 0, Some(1), Sequence, None, 5),
        ch("m:cGp", 0, Some(1), Sequence, None, 6),
        ch("m:mcs", 0, Some(1), Sequence, None, 7),
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 8),
    ]),
    ty("m:CT_MR", &[
        ch("m:e", 1, None, Sequence, None, 0),
    ]),
    ty("m:CT_ManualBreak", &[]),
    ty("m:CT_MathPr", &[
        ch("m:mathFont", 0, Some(1), Sequence, None, 0),
        ch("m:brkBin", 0, Some(1), Sequence, None, 1),
        ch("m:brkBinSub", 0, Some(1), Sequence, None, 2),
        ch("m:smallFrac", 0, Some(1), Sequence, None, 3),
        ch("m:dispDef", 0, Some(1), Sequence, None, 4),
        ch("m:lMargin", 0, Some(1), Sequence, None, 5),
        ch("m:rMargin", 0, Some(1), Sequence, None, 6),
        ch("m:defJc", 0, Some(1), Sequence, None, 7),
        ch("m:preSp", 0, Some(1), Sequence, None, 8),
        ch("m:postSp", 0, Some(1), Sequence, None, 9),
        ch("m:interSp", 0, Some(1), Sequence, None, 10),
        ch("m:intraSp", 0, Some(1), Sequence, None, 11),
        ch("m:wrapIndent", 0, Some(1), Choice, Some("choice1"), 12),
        ch("m:wrapRight", 0, Some(1), Choice, Some("choice1"), 13),
        ch("m:intLim", 0, Some(1), Sequence, None, 14),
        ch("m:naryLim", 0, Some(1), Sequence, None, 15),
    ]),
    ty("m:CT_Nary", &[
        ch("m:naryPr", 0, Some(1), Sequence, None, 0),
        ch("m:sub", 1, Some(1), Sequence, None, 1),
        ch("m:sup", 1, Some(1), Sequence, None, 2),
        ch("m:e", 1, Some(1), Sequence, None, 3),
    ]),
    ty("m:CT_NaryPr", &[
        ch("m:chr", 0, Some(1), Sequence, None, 0),
        ch("m:limLoc", 0, Some(1), Sequence, None, 1),
        ch("m:grow", 0, Some(1), Sequence, None, 2),
        ch("m:subHide", 0, Some(1), Sequence, None, 3),
        ch("m:supHide", 0, Some(1), Sequence, None, 4),
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 5),
    ]),
    ty("m:CT_OMath", &[
        ch("m:acc", 0, None, Choice, Some("choice2"), 0),
        ch("m:bar", 0, None, Choice, Some("choice2"), 1),
        ch("m:box", 0, None, Choice, Some("choice2"), 2),
        ch("m:borderBox", 0, None, Choice, Some("choice2"), 3),
        ch("m:d", 0, None, Choice, Some("choice2"), 4),
        ch("m:eqArr", 0, None, Choice, Some("choice2"), 5),
        ch("m:f", 0, None, Choice, Some("choice2"), 6),
        ch("m:func", 0, None, Choice, Some("choice2"), 7),
        ch("m:groupChr", 0, None, Choice, Some("choice2"), 8),
        ch("m:limLow", 0, None, Choice, Some("choice2"), 9),
        ch("m:limUpp", 0, None, Choice, Some("choice2"), 10),
        ch("m:m", 0, None, Choice, Some("choice2"), 11),
        ch("m:nary", 0, None, Choice, Some("choice2"), 12),
        ch("m:phant", 0, None, Choice, Some("choice2"), 13),
        ch("m:rad", 0, None, Choice, Some("choice2"), 14),
        ch("m:sPre", 0, None, Choice, Some("choice2"), 15),
        ch("m:sSub", 0, None, Choice, Some("choice2"), 16),
        ch("m:sSubSup", 0, None, Choice, Some("choice2"), 17),
        ch("m:sSup", 0, None, Choice, Some("choice2"), 18),
        ch("m:r", 0, None, Choice, Some("choice2"), 19),
        ch("w:customXml", 0, None, Choice, Some("choice4"), 20),
        ch("w:fldSimple", 0, None, Choice, Some("choice4"), 21),
        ch("w:hyperlink", 0, None, Choice, Some("choice4"), 22),
        ch("w:smartTag", 0, None, Choice, Some("choice5"), 23),
        ch("w:sdt", 0, None, Choice, Some("choice5"), 24),
        ch("w:proofErr", 0, None, Choice, Some("choice6"), 25),
        ch("w:permStart", 0, None, Choice, Some("choice6"), 26),
        ch("w:permEnd", 0, None, Choice, Some("choice6"), 27),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice7"), 28),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice7"), 29),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice7"), 30),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice7"), 31),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice7"), 32),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice7"), 33),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice7"), 34),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice7"), 35),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice7"), 36),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice7"), 37),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice7"), 38),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice7"), 39),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice7"), 40),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice7"), 41),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice7"), 42),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice7"), 43),
        ch("w:ins", 0, None, Choice, Some("choice6"), 44),
        ch("w:del", 0, None, Choice, Some("choice6"), 45),
        ch("w:moveFrom", 0, None, Choice, Some("choice6"), 46),
        ch("w:moveTo", 0, None, Choice, Some("choice6"), 47),
        ch("m:oMathPara", 0, None, Choice, Some("choice8"), 48),
        ch("m:oMath", 0, None, Choice, Some("choice8"), 49),
    ]),
    ty("m:CT_OMathArg", &[
        ch("m:argPr", 0, Some(1), Sequence, None, 0),
        ch("m:acc", 0, None, Choice, Some("choice2"), 1),
        ch("m:bar", 0, None, Choice, Some("choice2"), 2),
        ch("m:box", 0, None, Choice, Some("choice2"), 3),
        ch("m:borderBox", 0, None, Choice, Some("choice2"), 4),
        ch("m:d", 0, None, Choice, Some("choice2"), 5),
        ch("m:eqArr", 0, None, Choice, Some("choice2"), 6),
        ch("m:f", 0, None, Choice, Some("choice2"), 7),
        ch("m:func", 0, None, Choice, Some("choice2"), 8),
        ch("m:groupChr", 0, None, Choice, Some("choice2"), 9),
        ch("m:limLow", 0, None, Choice, Some("choice2"), 10),
        ch("m:limUpp", 0, None, Choice, Some("choice2"), 11),
        ch("m:m", 0, None, Choice, Some("choice2"), 12),
        ch("m:nary", 0, None, Choice, Some("choice2"), 13),
        ch("m:phant", 0, None, Choice, Some("choice2"), 14),
        ch("m:rad", 0, None, Choice, Some("choice2"), 15),
        ch("m:sPre", 0, None, Choice, Some("choice2"), 16),
        ch("m:sSub", 0, None, Choice, Some("choice2"), 17),
        ch("m:sSubSup", 0, None, Choice, Some("choice2"), 18),
        ch("m:sSup", 0, None, Choice, Some("choice2"), 19),
        ch("m:r", 0, None, Choice, Some("choice2"), 20),
        ch("w:customXml", 0, None, Choice, Some("choice4"), 21),
        ch("w:fldSimple", 0, None, Choice, Some("choice4"), 22),
        ch("w:hyperlink", 0, None, Choice, Some("choice4"), 23),
        ch("w:smartTag", 0, None, Choice, Some("choice5"), 24),
        ch("w:sdt", 0, None, Choice, Some("choice5"), 25),
        ch("w:proofErr", 0, None, Choice, Some("choice6"), 26),
        ch("w:permStart", 0, None, Choice, Some("choice6"), 27),
        ch("w:permEnd", 0, None, Choice, Some("choice6"), 28),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice7"), 29),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice7"), 30),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice7"), 31),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice7"), 32),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice7"), 33),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice7"), 34),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice7"), 35),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice7"), 36),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice7"), 37),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice7"), 38),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice7"), 39),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice7"), 40),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice7"), 41),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice7"), 42),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice7"), 43),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice7"), 44),
        ch("w:ins", 0, None, Choice, Some("choice6"), 45),
        ch("w:del", 0, None, Choice, Some("choice6"), 46),
        ch("w:moveFrom", 0, None, Choice, Some("choice6"), 47),
        ch("w:moveTo", 0, None, Choice, Some("choice6"), 48),
        ch("m:oMathPara", 0, None, Choice, Some("choice8"), 49),
        ch("m:oMath", 0, None, Choice, Some("choice8"), 50),
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 51),
    ]),
    ty("m:CT_OMathArgPr", &[
        ch("m:argSz", 0, Some(1), Sequence, None, 0),
    ]),
    ty("m:CT_OMathJc", &[]),
    ty("m:CT_OMathPara", &[
        ch("m:oMathParaPr", 0, Some(1), Sequence, None, 0),
        ch("m:oMath", 1, None, Sequence, None, 1),
    ]),
    ty("m:CT_OMathParaPr", &[
        ch("m:jc", 0, Some(1), Sequence, None, 0),
    ]),
    ty("m:CT_OnOff", &[]),
    ty("m:CT_Phant", &[
        ch("m:phantPr", 0, Some(1), Sequence, None, 0),
        ch("m:e", 1, Some(1), Sequence, None, 1),
    ]),
    ty("m:CT_PhantPr", &[
        ch("m:show", 0, Some(1), Sequence, None, 0),
        ch("m:zeroWid", 0, Some(1), Sequence, None, 1),
        ch("m:zeroAsc", 0, Some(1), Sequence, None, 2),
        ch("m:zeroDesc", 0, Some(1), Sequence, None, 3),
        ch("m:transp", 0, Some(1), Sequence, None, 4),
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 5),
    ]),
    ty("m:CT_R", &[
        ch("m:rPr", 0, Some(1), Sequence, None, 0),
        ch("w:rPr", 0, Some(1), Sequence, None, 1),
        ch("w:br", 0, None, Choice, Some("choice2"), 2),
        ch("w:t", 0, None, Choice, Some("choice2"), 3),
        ch("w:contentPart", 0, None, Choice, Some("choice2"), 4),
        ch("w:delText", 0, None, Choice, Some("choice2"), 5),
        ch("w:instrText", 0, None, Choice, Some("choice2"), 6),
        ch("w:delInstrText", 0, None, Choice, Some("choice2"), 7),
        ch("w:noBreakHyphen", 0, None, Choice, Some("choice2"), 8),
        ch("w:softHyphen", 0, None, Choice, Some("choice2"), 9),
        ch("w:dayShort", 0, None, Choice, Some("choice2"), 10),
        ch("w:monthShort", 0, None, Choice, Some("choice2"), 11),
        ch("w:yearShort", 0, None, Choice, Some("choice2"), 12),
        ch("w:dayLong", 0, None, Choice, Some("choice2"), 13),
        ch("w:monthLong", 0, None, Choice, Some("choice2"), 14),
        ch("w:yearLong", 0, None, Choice, Some("choice2"), 15),
        ch("w:annotationRef", 0, None, Choice, Some("choice2"), 16),
        ch("w:footnoteRef", 0, None, Choice, Some("choice2"), 17),
        ch("w:endnoteRef", 0, None, Choice, Some("choice2"), 18),
        ch("w:separator", 0, None, Choice, Some("choice2"), 19),
        ch("w:continuationSeparator", 0, None, Choice, Some("choice2"), 20),
        ch("w:sym", 0, None, Choice, Some("choice2"), 21),
        ch("w:pgNum", 0, None, Choice, Some("choice2"), 22),
        ch("w:cr", 0, None, Choice, Some("choice2"), 23),
        ch("w:tab", 0, None, Choice, Some("choice2"), 24),
        ch("w:object", 0, None, Choice, Some("choice2"), 25),
        ch("w:fldChar", 0, None, Choice, Some("choice2"), 26),
        ch("w:ruby", 0, None, Choice, Some("choice2"), 27),
        ch("w:footnoteReference", 0, None, Choice, Some("choice2"), 28),
        ch("w:endnoteReference", 0, None, Choice, Some("choice2"), 29),
        ch("w:commentReference", 0, None, Choice, Some("choice2"), 30),
        ch("w:drawing", 0, None, Choice, Some("choice2"), 31),
        ch("w:ptab", 0, None, Choice, Some("choice2"), 32),
        ch("w:lastRenderedPageBreak", 0, None, Choice, Some("choice2"), 33),
        ch("m:t", 0, None, Choice, Some("choice1"), 34),
    ]),
    ty("m:CT_RPR", &[
        ch("m:lit", 0, Some(1), Sequence, None, 0),
        ch("m:nor", 0, Some(1), Choice, Some("choice1"), 1),
        ch("m:scr", 0, Some(1), Sequence, Some("choice1"), 2),
        ch("m:sty", 0, Some(1), Sequence, Some("choice1"), 3),
        ch("m:brk", 0, Some(1), Sequence, None, 4),
        ch("m:aln", 0, Some(1), Sequence, None, 5),
    ]),
    ty("m:CT_Rad", &[
        ch("m:radPr", 0, Some(1), Sequence, None, 0),
        ch("m:deg", 1, Some(1), Sequence, None, 1),
        ch("m:e", 1, Some(1), Sequence, None, 2),
    ]),
    ty("m:CT_RadPr", &[
        ch("m:degHide", 0, Some(1), Sequence, None, 0),
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 1),
    ]),
    ty("m:CT_SPre", &[
        ch("m:sPrePr", 0, Some(1), Sequence, None, 0),
        ch("m:sub", 1, Some(1), Sequence, None, 1),
        ch("m:sup", 1, Some(1), Sequence, None, 2),
        ch("m:e", 1, Some(1), Sequence, None, 3),
    ]),
    ty("m:CT_SPrePr", &[
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 0),
    ]),
    ty("m:CT_SSub", &[
        ch("m:sSubPr", 0, Some(1), Sequence, None, 0),
        ch("m:e", 1, Some(1), Sequence, None, 1),
        ch("m:sub", 1, Some(1), Sequence, None, 2),
    ]),
    ty("m:CT_SSubPr", &[
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 0),
    ]),
    ty("m:CT_SSubSup", &[
        ch("m:sSubSupPr", 0, Some(1), Sequence, None, 0),
        ch("m:e", 1, Some(1), Sequence, None, 1),
        ch("m:sub", 1, Some(1), Sequence, None, 2),
        ch("m:sup", 1, Some(1), Sequence, None, 3),
    ]),
    ty("m:CT_SSubSupPr", &[
        ch("m:alnScr", 0, Some(1), Sequence, None, 0),
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 1),
    ]),
    ty("m:CT_SSup", &[
        ch("m:sSupPr", 0, Some(1), Sequence, None, 0),
        ch("m:e", 1, Some(1), Sequence, None, 1),
        ch("m:sup", 1, Some(1), Sequence, None, 2),
    ]),
    ty("m:CT_SSupPr", &[
        ch("m:ctrlPr", 0, Some(1), Sequence, None, 0),
    ]),
    ty("m:CT_Script", &[]),
    ty("m:CT_Shp", &[]),
    ty("m:CT_SpacingRule", &[]),
    ty("m:CT_String", &[]),
    ty("m:CT_Style", &[]),
    ty("m:CT_Text", &[]),
    ty("m:CT_TopBot", &[]),
    ty("m:CT_TwipsMeasure", &[]),
    ty("m:CT_UnSignedInteger", &[]),
    ty("m:CT_XAlign", &[]),
    ty("m:CT_YAlign", &[]),
    ty("pic:CT_Picture", &[
        ch("pic:nvPicPr", 1, Some(1), Sequence, None, 0),
        ch("pic:blipFill", 1, Some(1), Sequence, None, 1),
        ch("pic:spPr", 1, Some(1), Sequence, None, 2),
    ]),
    ty("pic:CT_PictureNonVisual", &[
        ch("pic:cNvPr", 1, Some(1), Sequence, None, 0),
        ch("pic:cNvPicPr", 1, Some(1), Sequence, None, 1),
    ]),
    ty("w:CT_AbstractNum", &[
        ch("w:nsid", 0, Some(1), Sequence, None, 0),
        ch("w:multiLevelType", 0, Some(1), Sequence, None, 1),
        ch("w:tmpl", 0, Some(1), Sequence, None, 2),
        ch("w:name", 0, Some(1), Sequence, None, 3),
        ch("w:styleLink", 0, Some(1), Sequence, None, 4),
        ch("w:numStyleLink", 0, Some(1), Sequence, None, 5),
        ch("w:lvl", 0, Some(9), Sequence, None, 6),
    ]),
    ty("w:CT_AltChunk", &[
        ch("w:altChunkPr", 0, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_AltChunkPr", &[
        ch("w:matchSrc", 0, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_Attr", &[]),
    ty("w:CT_AutoCaption", &[]),
    ty("w:CT_AutoCaptions", &[
        ch("w:autoCaption", 1, None, Sequence, None, 0),
    ]),
    ty("w:CT_Background", &[
        ch("w:drawing", 0, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_Base64Binary", &[]),
    ty("w:CT_BdoContentRun", &[]),
    ty("w:CT_Body", &[
        ch("w:customXml", 0, None, Choice, Some("choice3"), 0),
        ch("w:sdt", 0, None, Choice, Some("choice3"), 1),
        ch("w:p", 0, None, Choice, Some("choice3"), 2),
        ch("w:tbl", 0, None, Choice, Some("choice3"), 3),
        ch("w:proofErr", 0, None, Choice, Some("choice4"), 4),
        ch("w:permStart", 0, None, Choice, Some("choice4"), 5),
        ch("w:permEnd", 0, None, Choice, Some("choice4"), 6),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice5"), 7),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice5"), 8),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice5"), 9),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice5"), 10),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice5"), 11),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice5"), 12),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice5"), 13),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice5"), 14),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice5"), 15),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice5"), 16),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice5"), 17),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice5"), 18),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice5"), 19),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice5"), 20),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice5"), 21),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice5"), 22),
        ch("w:ins", 0, None, Choice, Some("choice4"), 23),
        ch("w:del", 0, None, Choice, Some("choice4"), 24),
        ch("w:moveFrom", 0, None, Choice, Some("choice4"), 25),
        ch("w:moveTo", 0, None, Choice, Some("choice4"), 26),
        ch("m:oMathPara", 0, None, Choice, Some("choice6"), 27),
        ch("m:oMath", 0, None, Choice, Some("choice6"), 28),
        ch("w:altChunk", 0, None, Choice, Some("choice1"), 29),
        ch("w:sectPr", 0, Some(1), Sequence, None, 30),
    ]),
    ty("w:CT_Bookmark", &[]),
    ty("w:CT_BookmarkRange", &[]),
    ty("w:CT_Border", &[]),
    ty("w:CT_BottomPageBorder", &[]),
    ty("w:CT_Br", &[]),
    ty("w:CT_CalendarType", &[]),
    ty("w:CT_Caption", &[]),
    ty("w:CT_Captions", &[
        ch("w:caption", 1, None, Sequence, None, 0),
        ch("w:autoCaptions", 0, Some(1), Sequence, None, 1),
    ]),
    ty("w:CT_CellMergeTrackChange", &[]),
    ty("w:CT_CharacterSpacing", &[]),
    ty("w:CT_Charset", &[]),
    ty("w:CT_Cnf", &[]),
    ty("w:CT_Color", &[]),
    ty("w:CT_ColorSchemeMapping", &[]),
    ty("w:CT_Column", &[]),
    ty("w:CT_Columns", &[
        ch("w:col", 0, Some(45), Sequence, None, 0),
    ]),
    ty("w:CT_Comment", &[
        ch("w:customXml", 0, None, Choice, Some("choice3"), 0),
        ch("w:sdt", 0, None, Choice, Some("choice3"), 1),
        ch("w:p", 0, None, Choice, Some("choice3"), 2),
        ch("w:tbl", 0, None, Choice, Some("choice3"), 3),
        ch("w:proofErr", 0, None, Choice, Some("choice4"), 4),
        ch("w:permStart", 0, None, Choice, Some("choice4"), 5),
        ch("w:permEnd", 0, None, Choice, Some("choice4"), 6),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice5"), 7),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice5"), 8),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice5"), 9),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice5"), 10),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice5"), 11),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice5"), 12),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice5"), 13),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice5"), 14),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice5"), 15),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice5"), 16),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice5"), 17),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice5"), 18),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice5"), 19),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice5"), 20),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice5"), 21),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice5"), 22),
        ch("w:ins", 0, None, Choice, Some("choice4"), 23),
        ch("w:del", 0, None, Choice, Some("choice4"), 24),
        ch("w:moveFrom", 0, None, Choice, Some("choice4"), 25),
        ch("w:moveTo", 0, None, Choice, Some("choice4"), 26),
        ch("m:oMathPara", 0, None, Choice, Some("choice6"), 27),
        ch("m:oMath", 0, None, Choice, Some("choice6"), 28),
        ch("w:altChunk", 0, None, Choice, Some("choice1"), 29),
    ]),
    ty("w:CT_Comments", &[
        ch("w:comment", 0, None, Sequence, None, 0),
    ]),
    ty("w:CT_Compat", &[
        ch("w:spaceForUL", 0, Some(1), Sequence, None, 0),
        ch("w:balanceSingleByteDoubleByteWidth", 0, Some(1), Sequence, None, 1),
        ch("w:doNotLeaveBackslashAlone", 0, Some(1), Sequence, None, 2),
        ch("w:ulTrailSpace", 0, Some(1), Sequence, None, 3),
        ch("w:doNotExpandShiftReturn", 0, Some(1), Sequence, None, 4),
        ch("w:adjustLineHeightInTable", 0, Some(1), Sequence, None, 5),
        ch("w:applyBreakingRules", 0, Some(1), Sequence, None, 6),
        ch("w:compatSetting", 0, None, Sequence, None, 7),
    ]),
    ty("w:CT_CompatSetting", &[]),
    ty("w:CT_Control", &[]),
    ty("w:CT_CustomXmlBlock", &[
        ch("w:customXmlPr", 0, Some(1), Sequence, None, 0),
        ch("w:customXml", 0, None, Choice, Some("choice1"), 1),
        ch("w:sdt", 0, None, Choice, Some("choice1"), 2),
        ch("w:p", 0, None, Choice, Some("choice1"), 3),
        ch("w:tbl", 0, None, Choice, Some("choice1"), 4),
        ch("w:proofErr", 0, None, Choice, Some("choice2"), 5),
        ch("w:permStart", 0, None, Choice, Some("choice2"), 6),
        ch("w:permEnd", 0, None, Choice, Some("choice2"), 7),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice3"), 8),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice3"), 9),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice3"), 10),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice3"), 11),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice3"), 12),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice3"), 13),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice3"), 14),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice3"), 15),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice3"), 16),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice3"), 17),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice3"), 18),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice3"), 19),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice3"), 20),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice3"), 21),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice3"), 22),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice3"), 23),
        ch("w:ins", 0, None, Choice, Some("choice2"), 24),
        ch("w:del", 0, None, Choice, Some("choice2"), 25),
        ch("w:moveFrom", 0, None, Choice, Some("choice2"), 26),
        ch("w:moveTo", 0, None, Choice, Some("choice2"), 27),
        ch("m:oMathPara", 0, None, Choice, Some("choice4"), 28),
        ch("m:oMath", 0, None, Choice, Some("choice4"), 29),
    ]),
    ty("w:CT_CustomXmlCell", &[
        ch("w:customXmlPr", 0, Some(1), Sequence, None, 0),
        ch("w:tc", 0, None, Choice, Some("choice1"), 1),
        ch("w:customXml", 0, None, Choice, Some("choice1"), 2),
        ch("w:sdt", 0, None, Choice, Some("choice1"), 3),
        ch("w:proofErr", 0, None, Choice, Some("choice2"), 4),
        ch("w:permStart", 0, None, Choice, Some("choice2"), 5),
        ch("w:permEnd", 0, None, Choice, Some("choice2"), 6),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice3"), 7),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice3"), 8),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice3"), 9),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice3"), 10),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice3"), 11),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice3"), 12),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice3"), 13),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice3"), 14),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice3"), 15),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice3"), 16),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice3"), 17),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice3"), 18),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice3"), 19),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice3"), 20),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice3"), 21),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice3"), 22),
        ch("w:ins", 0, None, Choice, Some("choice2"), 23),
        ch("w:del", 0, None, Choice, Some("choice2"), 24),
        ch("w:moveFrom", 0, None, Choice, Some("choice2"), 25),
        ch("w:moveTo", 0, None, Choice, Some("choice2"), 26),
        ch("m:oMathPara", 0, None, Choice, Some("choice4"), 27),
        ch("m:oMath", 0, None, Choice, Some("choice4"), 28),
    ]),
    ty("w:CT_CustomXmlPr", &[
        ch("w:placeholder", 0, Some(1), Sequence, None, 0),
        ch("w:attr", 0, None, Sequence, None, 1),
    ]),
    ty("w:CT_CustomXmlRow", &[
        ch("w:customXmlPr", 0, Some(1), Sequence, None, 0),
        ch("w:tr", 0, None, Choice, Some("choice1"), 1),
        ch("w:customXml", 0, None, Choice, Some("choice1"), 2),
        ch("w:sdt", 0, None, Choice, Some("choice1"), 3),
        ch("w:proofErr", 0, None, Choice, Some("choice2"), 4),
        ch("w:permStart", 0, None, Choice, Some("choice2"), 5),
        ch("w:permEnd", 0, None, Choice, Some("choice2"), 6),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice3"), 7),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice3"), 8),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice3"), 9),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice3"), 10),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice3"), 11),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice3"), 12),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice3"), 13),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice3"), 14),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice3"), 15),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice3"), 16),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice3"), 17),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice3"), 18),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice3"), 19),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice3"), 20),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice3"), 21),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice3"), 22),
        ch("w:ins", 0, None, Choice, Some("choice2"), 23),
        ch("w:del", 0, None, Choice, Some("choice2"), 24),
        ch("w:moveFrom", 0, None, Choice, Some("choice2"), 25),
        ch("w:moveTo", 0, None, Choice, Some("choice2"), 26),
        ch("m:oMathPara", 0, None, Choice, Some("choice4"), 27),
        ch("m:oMath", 0, None, Choice, Some("choice4"), 28),
    ]),
    ty("w:CT_CustomXmlRun", &[
        ch("w:customXmlPr", 0, Some(1), Sequence, None, 0),
        ch("w:customXml", 0, None, Choice, Some("choice2"), 1),
        ch("w:smartTag", 0, None, Choice, Some("choice2"), 2),
        ch("w:sdt", 0, None, Choice, Some("choice2"), 3),
        ch("w:dir", 0, None, Choice, Some("choice2"), 4),
        ch("w:bdo", 0, None, Choice, Some("choice2"), 5),
        ch("w:r", 0, None, Choice, Some("choice2"), 6),
        ch("w:proofErr", 0, None, Choice, Some("choice3"), 7),
        ch("w:permStart", 0, None, Choice, Some("choice3"), 8),
        ch("w:permEnd", 0, None, Choice, Some("choice3"), 9),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice4"), 10),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice4"), 11),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice4"), 12),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice4"), 13),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice4"), 14),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice4"), 15),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice4"), 16),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice4"), 17),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice4"), 18),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice4"), 19),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice4"), 20),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice4"), 21),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice4"), 22),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice4"), 23),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice4"), 24),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice4"), 25),
        ch("w:ins", 0, None, Choice, Some("choice3"), 26),
        ch("w:del", 0, None, Choice, Some("choice3"), 27),
        ch("w:moveFrom", 0, None, Choice, Some("choice3"), 28),
        ch("w:moveTo", 0, None, Choice, Some("choice3"), 29),
        ch("m:oMathPara", 0, None, Choice, Some("choice5"), 30),
        ch("m:oMath", 0, None, Choice, Some("choice5"), 31),
        ch("w:fldSimple", 0, None, Choice, Some("choice1"), 32),
        ch("w:hyperlink", 0, None, Choice, Some("choice1"), 33),
        ch("w:subDoc", 0, None, Choice, Some("choice1"), 34),
    ]),
    ty("w:CT_DataBinding", &[]),
    ty("w:CT_DecimalNumber", &[]),
    ty("w:CT_DecimalNumberOrPrecent", &[]),
    ty("w:CT_DirContentRun", &[]),
    ty("w:CT_Div", &[
        ch("w:blockQuote", 0, Some(1), Sequence, None, 0),
        ch("w:bodyDiv", 0, Some(1), Sequence, None, 1),
        ch("w:marLeft", 1, Some(1), Sequence, None, 2),
        ch("w:marRight", 1, Some(1), Sequence, None, 3),
        ch("w:marTop", 1, Some(1), Sequence, None, 4),
        ch("w:marBottom", 1, Some(1), Sequence, None, 5),
        ch("w:divBdr", 0, Some(1), Sequence, None, 6),
        ch("w:divsChild", 0, None, Sequence, None, 7),
    ]),
    ty("w:CT_DivBdr", &[
        ch("w:top", 0, Some(1), Sequence, None, 0),
        ch("w:left", 0, Some(1), Sequence, None, 1),
        ch("w:bottom", 0, Some(1), Sequence, None, 2),
        ch("w:right", 0, Some(1), Sequence, None, 3),
    ]),
    ty("w:CT_Divs", &[
        ch("w:div", 1, None, Sequence, None, 0),
    ]),
    ty("w:CT_DocDefaults", &[
        ch("w:rPrDefault", 0, Some(1), Sequence, None, 0),
        ch("w:pPrDefault", 0, Some(1), Sequence, None, 1),
    ]),
    ty("w:CT_DocGrid", &[]),
    ty("w:CT_DocPart", &[
        ch("w:docPartPr", 0, Some(1), Sequence, None, 0),
        ch("w:docPartBody", 0, Some(1), Sequence, None, 1),
    ]),
    ty("w:CT_DocPartBehavior", &[]),
    ty("w:CT_DocPartBehaviors", &[
        ch("w:behavior", 1, None, Choice, Some("choice1"), 0),
    ]),
    ty("w:CT_DocPartCategory", &[
        ch("w:name", 1, Some(1), Sequence, None, 0),
        ch("w:gallery", 1, Some(1), Sequence, None, 1),
    ]),
    ty("w:CT_DocPartGallery", &[]),
    ty("w:CT_DocPartName", &[]),
    ty("w:CT_DocPartPr", &[
        ch("w:name", 1, Some(1), All, None, 0),
        ch("w:style", 0, Some(1), All, None, 1),
        ch("w:category", 0, Some(1), All, None, 2),
        ch("w:types", 0, Some(1), All, None, 3),
        ch("w:behaviors", 0, Some(1), All, None, 4),
        ch("w:description", 0, Some(1), All, None, 5),
        ch("w:guid", 0, Some(1), All, None, 6),
    ]),
    ty("w:CT_DocPartType", &[]),
    ty("w:CT_DocPartTypes", &[
        ch("w:type", 1, None, Choice, Some("choice1"), 0),
    ]),
    ty("w:CT_DocParts", &[
        ch("w:docPart", 1, None, Choice, Some("choice1"), 0),
    ]),
    ty("w:CT_DocProtect", &[]),
    ty("w:CT_DocRsids", &[
        ch("w:rsidRoot", 0, Some(1), Sequence, None, 0),
        ch("w:rsid", 0, None, Sequence, None, 1),
    ]),
    ty("w:CT_DocType", &[]),
    ty("w:CT_DocVar", &[]),
    ty("w:CT_DocVars", &[
        ch("w:docVar", 0, None, Sequence, None, 0),
    ]),
    ty("w:CT_Document", &[
        ch("w:background", 0, Some(1), Sequence, None, 0),
        ch("w:body", 0, Some(1), Sequence, None, 1),
    ]),
    ty("w:CT_DocumentBase", &[
        ch("w:background", 0, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_Drawing", &[
        ch("wp:anchor", 0, None, Choice, Some("choice1"), 0),
        ch("wp:inline", 0, None, Choice, Some("choice1"), 1),
    ]),
    ty("w:CT_EastAsianLayout", &[]),
    ty("w:CT_EdnDocProps", &[
        ch("w:pos", 0, Some(1), Sequence, None, 0),
        ch("w:numFmt", 0, Some(1), Sequence, None, 1),
        ch("w:numStart", 0, Some(1), Sequence, None, 2),
        ch("w:numRestart", 0, Some(1), Sequence, None, 3),
        ch("w:endnote", 0, Some(3), Sequence, None, 4),
    ]),
    ty("w:CT_EdnPos", &[]),
    ty("w:CT_EdnProps", &[
        ch("w:pos", 0, Some(1), Sequence, None, 0),
        ch("w:numFmt", 0, Some(1), Sequence, None, 1),
        ch("w:numStart", 0, Some(1), Sequence, None, 2),
        ch("w:numRestart", 0, Some(1), Sequence, None, 3),
    ]),
    ty("w:CT_Em", &[]),
    ty("w:CT_Empty", &[]),
    ty("w:CT_Endnotes", &[
        ch("w:endnote", 0, None, Sequence, None, 0),
    ]),
    ty("w:CT_FFCheckBox", &[
        ch("w:size", 1, Some(1), Choice, Some("choice1"), 0),
        ch("w:sizeAuto", 1, Some(1), Choice, Some("choice1"), 1),
        ch("w:default", 0, Some(1), Sequence, None, 2),
        ch("w:checked", 0, Some(1), Sequence, None, 3),
    ]),
    ty("w:CT_FFDDList", &[
        ch("w:result", 0, Some(1), Sequence, None, 0),
        ch("w:default", 0, Some(1), Sequence, None, 1),
        ch("w:listEntry", 0, None, Sequence, None, 2),
    ]),
    ty("w:CT_FFData", &[
        ch("w:name", 1, None, Choice, Some("choice1"), 0),
        ch("w:label", 0, None, Choice, Some("choice1"), 1),
        ch("w:tabIndex", 0, None, Choice, Some("choice1"), 2),
        ch("w:enabled", 1, None, Choice, Some("choice1"), 3),
        ch("w:calcOnExit", 1, None, Choice, Some("choice1"), 4),
        ch("w:entryMacro", 0, None, Choice, Some("choice1"), 5),
        ch("w:exitMacro", 0, None, Choice, Some("choice1"), 6),
        ch("w:helpText", 0, None, Choice, Some("choice1"), 7),
        ch("w:statusText", 0, None, Choice, Some("choice1"), 8),
        ch("w:checkBox", 1, None, Choice, Some("choice2"), 9),
        ch("w:ddList", 1, None, Choice, Some("choice2"), 10),
        ch("w:textInput", 1, None, Choice, Some("choice2"), 11),
    ]),
    ty("w:CT_FFHelpText", &[]),
    ty("w:CT_FFName", &[]),
    ty("w:CT_FFStatusText", &[]),
    ty("w:CT_FFTextInput", &[
        ch("w:type", 0, Some(1), Sequence, None, 0),
        ch("w:default", 0, Some(1), Sequence, None, 1),
        ch("w:maxLength", 0, Some(1), Sequence, None, 2),
        ch("w:format", 0, Some(1), Sequence, None, 3),
    ]),
    ty("w:CT_FFTextType", &[]),
    ty("w:CT_FitText", &[]),
    ty("w:CT_FldChar", &[
        ch("w:ffData", 0, Some(1), Choice, Some("choice1"), 0),
    ]),
    ty("w:CT_Font", &[
        ch("w:altName", 0, Some(1), Sequence, None, 0),
        ch("w:panose1", 0, Some(1), Sequence, None, 1),
        ch("w:charset", 0, Some(1), Sequence, None, 2),
        ch("w:family", 0, Some(1), Sequence, None, 3),
        ch("w:notTrueType", 0, Some(1), Sequence, None, 4),
        ch("w:pitch", 0, Some(1), Sequence, None, 5),
        ch("w:sig", 0, Some(1), Sequence, None, 6),
        ch("w:embedRegular", 0, Some(1), Sequence, None, 7),
        ch("w:embedBold", 0, Some(1), Sequence, None, 8),
        ch("w:embedItalic", 0, Some(1), Sequence, None, 9),
        ch("w:embedBoldItalic", 0, Some(1), Sequence, None, 10),
    ]),
    ty("w:CT_FontFamily", &[]),
    ty("w:CT_FontRel", &[]),
    ty("w:CT_FontSig", &[]),
    ty("w:CT_Fonts", &[]),
    ty("w:CT_FontsList", &[
        ch("w:font", 0, None, Sequence, None, 0),
    ]),
    ty("w:CT_Footnotes", &[
        ch("w:footnote", 0, None, Sequence, None, 0),
    ]),
    ty("w:CT_Frame", &[
        ch("w:sz", 0, Some(1), Sequence, None, 0),
        ch("w:name", 0, Some(1), Sequence, None, 1),
        ch("w:title", 0, Some(1), Sequence, None, 2),
        ch("w:longDesc", 0, Some(1), Sequence, None, 3),
        ch("w:sourceFileName", 0, Some(1), Sequence, None, 4),
        ch("w:marW", 0, Some(1), Sequence, None, 5),
        ch("w:marH", 0, Some(1), Sequence, None, 6),
        ch("w:scrollbar", 0, Some(1), Sequence, None, 7),
        ch("w:noResizeAllowed", 0, Some(1), Sequence, None, 8),
        ch("w:linkedToFile", 0, Some(1), Sequence, None, 9),
    ]),
    ty("w:CT_FrameLayout", &[]),
    ty("w:CT_FramePr", &[]),
    ty("w:CT_FrameScrollbar", &[]),
    ty("w:CT_Frameset", &[
        ch("w:sz", 0, Some(1), Sequence, None, 0),
        ch("w:framesetSplitbar", 0, Some(1), Sequence, None, 1),
        ch("w:frameLayout", 0, Some(1), Sequence, None, 2),
        ch("w:title", 0, Some(1), Sequence, None, 3),
        ch("w:frameset", 0, None, Choice, Some("choice1"), 4),
        ch("w:frame", 0, None, Choice, Some("choice1"), 5),
    ]),
    ty("w:CT_FramesetSplitbar", &[
        ch("w:w", 0, Some(1), Sequence, None, 0),
        ch("w:color", 0, Some(1), Sequence, None, 1),
        ch("w:noBorder", 0, Some(1), Sequence, None, 2),
        ch("w:flatBorders", 0, Some(1), Sequence, None, 3),
    ]),
    ty("w:CT_FtnDocProps", &[
        ch("w:pos", 0, Some(1), Sequence, None, 0),
        ch("w:numFmt", 0, Some(1), Sequence, None, 1),
        ch("w:numStart", 0, Some(1), Sequence, None, 2),
        ch("w:numRestart", 0, Some(1), Sequence, None, 3),
        ch("w:footnote", 0, Some(3), Sequence, None, 4),
    ]),
    ty("w:CT_FtnEdn", &[
        ch("w:customXml", 0, None, Choice, Some("choice3"), 0),
        ch("w:sdt", 0, None, Choice, Some("choice3"), 1),
        ch("w:p", 0, None, Choice, Some("choice3"), 2),
        ch("w:tbl", 0, None, Choice, Some("choice3"), 3),
        ch("w:proofErr", 0, None, Choice, Some("choice4"), 4),
        ch("w:permStart", 0, None, Choice, Some("choice4"), 5),
        ch("w:permEnd", 0, None, Choice, Some("choice4"), 6),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice5"), 7),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice5"), 8),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice5"), 9),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice5"), 10),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice5"), 11),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice5"), 12),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice5"), 13),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice5"), 14),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice5"), 15),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice5"), 16),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice5"), 17),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice5"), 18),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice5"), 19),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice5"), 20),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice5"), 21),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice5"), 22),
        ch("w:ins", 0, None, Choice, Some("choice4"), 23),
        ch("w:del", 0, None, Choice, Some("choice4"), 24),
        ch("w:moveFrom", 0, None, Choice, Some("choice4"), 25),
        ch("w:moveTo", 0, None, Choice, Some("choice4"), 26),
        ch("m:oMathPara", 0, None, Choice, Some("choice6"), 27),
        ch("m:oMath", 0, None, Choice, Some("choice6"), 28),
        ch("w:altChunk", 0, None, Choice, Some("choice1"), 29),
    ]),
    ty("w:CT_FtnEdnRef", &[]),
    ty("w:CT_FtnEdnSepRef", &[]),
    ty("w:CT_FtnPos", &[]),
    ty("w:CT_FtnProps", &[
        ch("w:pos", 0, Some(1), Sequence, None, 0),
        ch("w:numFmt", 0, Some(1), Sequence, None, 1),
        ch("w:numStart", 0, Some(1), Sequence, None, 2),
        ch("w:numRestart", 0, Some(1), Sequence, None, 3),
    ]),
    ty("w:CT_GlossaryDocument", &[
        ch("w:background", 0, Some(1), Sequence, None, 0),
        ch("w:docParts", 0, Some(1), Sequence, None, 1),
    ]),
    ty("w:CT_Guid", &[]),
    ty("w:CT_HdrFtr", &[]),
    ty("w:CT_HdrFtrRef", &[]),
    ty("w:CT_Headers", &[
        ch("w:header", 0, None, Sequence, None, 0),
    ]),
    ty("w:CT_Height", &[]),
    ty("w:CT_Highlight", &[]),
    ty("w:CT_HpsMeasure", &[]),
    ty("w:CT_Hyperlink", &[]),
    ty("w:CT_Ind", &[]),
    ty("w:CT_Jc", &[]),
    ty("w:CT_JcTable", &[]),
    ty("w:CT_Kinsoku", &[]),
    ty("w:CT_Lang", &[]),
    ty("w:CT_Language", &[]),
    ty("w:CT_LatentStyles", &[
        ch("w:lsdException", 0, None, Sequence, None, 0),
    ]),
    ty("w:CT_LevelSuffix", &[]),
    ty("w:CT_LevelText", &[]),
    ty("w:CT_LineNumber", &[]),
    ty("w:CT_Lock", &[]),
    ty("w:CT_LongHexNumber", &[]),
    ty("w:CT_LsdException", &[]),
    ty("w:CT_Lvl", &[
        ch("w:start", 0, Some(1), Sequence, None, 0),
        ch("w:numFmt", 0, Some(1), Sequence, None, 1),
        ch("w:lvlRestart", 0, Some(1), Sequence, None, 2),
        ch("w:pStyle", 0, Some(1), Sequence, None, 3),
        ch("w:isLgl", 0, Some(1), Sequence, None, 4),
        ch("w:suff", 0, Some(1), Sequence, None, 5),
        ch("w:lvlText", 0, Some(1), Sequence, None, 6),
        ch("w:lvlPicBulletId", 0, Some(1), Sequence, None, 7),
        ch("w:lvlJc", 0, Some(1), Sequence, None, 8),
        ch("w:pPr", 0, Some(1), Sequence, None, 9),
        ch("w:rPr", 0, Some(1), Sequence, None, 10),
    ]),
    ty("w:CT_MacroName", &[]),
    ty("w:CT_MailMerge", &[
        ch("w:mainDocumentType", 1, Some(1), Sequence, None, 0),
        ch("w:linkToQuery", 0, Some(1), Sequence, None, 1),
        ch("w:dataType", 1, Some(1), Sequence, None, 2),
        ch("w:connectString", 0, Some(1), Sequence, None, 3),
        ch("w:query", 0, Some(1), Sequence, None, 4),
        ch("w:dataSource", 0, Some(1), Sequence, None, 5),
        ch("w:headerSource", 0, Some(1), Sequence, None, 6),
        ch("w:doNotSuppressBlankLines", 0, Some(1), Sequence, None, 7),
        ch("w:destination", 0, Some(1), Sequence, None, 8),
        ch("w:addressFieldName", 0, Some(1), Sequence, None, 9),
        ch("w:mailSubject", 0, Some(1), Sequence, None, 10),
        ch("w:mailAsAttachment", 0, Some(1), Sequence, None, 11),
        ch("w:viewMergedData", 0, Some(1), Sequence, None, 12),
        ch("w:activeRecord", 0, Some(1), Sequence, None, 13),
        ch("w:checkErrors", 0, Some(1), Sequence, None, 14),
        ch("w:odso", 0, Some(1), Sequence, None, 15),
    ]),
    ty("w:CT_MailMergeDataType", &[]),
    ty("w:CT_MailMergeDest", &[]),
    ty("w:CT_MailMergeDocType", &[]),
    ty("w:CT_MailMergeOdsoFMDFieldType", &[]),
    ty("w:CT_MailMergeSourceType", &[]),
    ty("w:CT_Markup", &[]),
    ty("w:CT_MarkupRange", &[]),
    ty("w:CT_MathCtrlDel", &[
        ch("w:rPr", 0, Some(1), Choice, Some("choice1"), 0),
    ]),
    ty("w:CT_MathCtrlIns", &[
        ch("w:del", 0, Some(1), Choice, Some("choice1"), 0),
        ch("w:rPr", 0, Some(1), Choice, Some("choice1"), 1),
    ]),
    ty("w:CT_MoveBookmark", &[]),
    ty("w:CT_MultiLevelType", &[]),
    ty("w:CT_Num", &[
        ch("w:abstractNumId", 1, Some(1), Sequence, None, 0),
        ch("w:lvlOverride", 0, Some(9), Sequence, None, 1),
    ]),
    ty("w:CT_NumFmt", &[]),
    ty("w:CT_NumLvl", &[
        ch("w:startOverride", 0, Some(1), Sequence, None, 0),
        ch("w:lvl", 0, Some(1), Sequence, None, 1),
    ]),
    ty("w:CT_NumPicBullet", &[
        ch("w:drawing", 1, Some(1), Choice, Some("choice1"), 0),
    ]),
    ty("w:CT_NumPr", &[
        ch("w:ilvl", 0, Some(1), Sequence, None, 0),
        ch("w:numId", 0, Some(1), Sequence, None, 1),
        ch("w:ins", 0, Some(1), Sequence, None, 2),
    ]),
    ty("w:CT_NumRestart", &[]),
    ty("w:CT_Numbering", &[
        ch("w:numPicBullet", 0, None, Sequence, None, 0),
        ch("w:abstractNum", 0, None, Sequence, None, 1),
        ch("w:num", 0, None, Sequence, None, 2),
        ch("w:numIdMacAtCleanup", 0, Some(1), Sequence, None, 3),
    ]),
    ty("w:CT_Object", &[
        ch("w:drawing", 0, Some(1), Sequence, None, 0),
        ch("w:control", 0, Some(1), Choice, Some("choice1"), 1),
        ch("w:objectLink", 0, Some(1), Choice, Some("choice1"), 2),
        ch("w:objectEmbed", 0, Some(1), Choice, Some("choice1"), 3),
        ch("w:movie", 0, Some(1), Choice, Some("choice1"), 4),
    ]),
    ty("w:CT_ObjectEmbed", &[]),
    ty("w:CT_ObjectLink", &[]),
    ty("w:CT_Odso", &[
        ch("w:udl", 0, Some(1), Sequence, None, 0),
        ch("w:table", 0, Some(1), Sequence, None, 1),
        ch("w:src", 0, Some(1), Sequence, None, 2),
        ch("w:colDelim", 0, Some(1), Sequence, None, 3),
        ch("w:type", 0, Some(1), Sequence, None, 4),
        ch("w:fHdr", 0, Some(1), Sequence, None, 5),
        ch("w:fieldMapData", 0, None, Sequence, None, 6),
        ch("w:recipientData", 0, None, Sequence, None, 7),
    ]),
    ty("w:CT_OdsoFieldMapData", &[
        ch("w:type", 0, Some(1), Sequence, None, 0),
        ch("w:name", 0, Some(1), Sequence, None, 1),
        ch("w:mappedName", 0, Some(1), Sequence, None, 2),
        ch("w:column", 0, Some(1), Sequence, None, 3),
        ch("w:lid", 0, Some(1), Sequence, None, 4),
        ch("w:dynamicAddress", 0, Some(1), Sequence, None, 5),
    ]),
    ty("w:CT_OnOff", &[]),
    ty("w:CT_OptimizeForBrowser", &[]),
    ty("w:CT_P", &[
        ch("w:pPr", 0, Some(1), Sequence, None, 0),
        ch("w:customXml", 0, None, Choice, Some("choice2"), 1),
        ch("w:smartTag", 0, None, Choice, Some("choice2"), 2),
        ch("w:sdt", 0, None, Choice, Some("choice2"), 3),
        ch("w:dir", 0, None, Choice, Some("choice2"), 4),
        ch("w:bdo", 0, None, Choice, Some("choice2"), 5),
        ch("w:r", 0, None, Choice, Some("choice2"), 6),
        ch("w:proofErr", 0, None, Choice, Some("choice3"), 7),
        ch("w:permStart", 0, None, Choice, Some("choice3"), 8),
        ch("w:permEnd", 0, None, Choice, Some("choice3"), 9),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice4"), 10),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice4"), 11),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice4"), 12),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice4"), 13),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice4"), 14),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice4"), 15),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice4"), 16),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice4"), 17),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice4"), 18),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice4"), 19),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice4"), 20),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice4"), 21),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice4"), 22),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice4"), 23),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice4"), 24),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice4"), 25),
        ch("w:ins", 0, None, Choice, Some("choice3"), 26),
        ch("w:del", 0, None, Choice, Some("choice3"), 27),
        ch("w:moveFrom", 0, None, Choice, Some("choice3"), 28),
        ch("w:moveTo", 0, None, Choice, Some("choice3"), 29),
        ch("m:oMathPara", 0, None, Choice, Some("choice5"), 30),
        ch("m:oMath", 0, None, Choice, Some("choice5"), 31),
        ch("w:fldSimple", 0, None, Choice, Some("choice1"), 32),
        ch("w:hyperlink", 0, None, Choice, Some("choice1"), 33),
        ch("w:subDoc", 0, None, Choice, Some("choice1"), 34),
    ]),
    ty("w:CT_PBdr", &[
        ch("w:top", 0, Some(1), Sequence, None, 0),
        ch("w:left", 0, Some(1), Sequence, None, 1),
        ch("w:bottom", 0, Some(1), Sequence, None, 2),
        ch("w:right", 0, Some(1), Sequence, None, 3),
        ch("w:between", 0, Some(1), Sequence, None, 4),
        ch("w:bar", 0, Some(1), Sequence, None, 5),
    ]),
    ty("w:CT_PPr", &[
        ch("w:pStyle", 0, Some(1), Sequence, None, 0),
        ch("w:keepNext", 0, Some(1), Sequence, None, 1),
        ch("w:keepLines", 0, Some(1), Sequence, None, 2),
        ch("w:pageBreakBefore", 0, Some(1), Sequence, None, 3),
        ch("w:framePr", 0, Some(1), Sequence, None, 4),
        ch("w:widowControl", 0, Some(1), Sequence, None, 5),
        ch("w:numPr", 0, Some(1), Sequence, None, 6),
        ch("w:suppressLineNumbers", 0, Some(1), Sequence, None, 7),
        ch("w:pBdr", 0, Some(1), Sequence, None, 8),
        ch("w:shd", 0, Some(1), Sequence, None, 9),
        ch("w:tabs", 0, Some(1), Sequence, None, 10),
        ch("w:suppressAutoHyphens", 0, Some(1), Sequence, None, 11),
        ch("w:kinsoku", 0, Some(1), Sequence, None, 12),
        ch("w:wordWrap", 0, Some(1), Sequence, None, 13),
        ch("w:overflowPunct", 0, Some(1), Sequence, None, 14),
        ch("w:topLinePunct", 0, Some(1), Sequence, None, 15),
        ch("w:autoSpaceDE", 0, Some(1), Sequence, None, 16),
        ch("w:autoSpaceDN", 0, Some(1), Sequence, None, 17),
        ch("w:bidi", 0, Some(1), Sequence, None, 18),
        ch("w:adjustRightInd", 0, Some(1), Sequence, None, 19),
        ch("w:snapToGrid", 0, Some(1), Sequence, None, 20),
        ch("w:spacing", 0, Some(1), Sequence, None, 21),
        ch("w:ind", 0, Some(1), Sequence, None, 22),
        ch("w:contextualSpacing", 0, Some(1), Sequence, None, 23),
        ch("w:mirrorIndents", 0, Some(1), Sequence, None, 24),
        ch("w:suppressOverlap", 0, Some(1), Sequence, None, 25),
        ch("w:jc", 0, Some(1), Sequence, None, 26),
        ch("w:textDirection", 0, Some(1), Sequence, None, 27),
        ch("w:textAlignment", 0, Some(1), Sequence, None, 28),
        ch("w:textboxTightWrap", 0, Some(1), Sequence, None, 29),
        ch("w:outlineLvl", 0, Some(1), Sequence, None, 30),
        ch("w:divId", 0, Some(1), Sequence, None, 31),
        ch("w:cnfStyle", 0, Some(1), Sequence, None, 32),
        ch("w:rPr", 0, Some(1), Sequence, None, 33),
        ch("w:sectPr", 0, Some(1), Sequence, None, 34),
        ch("w:pPrChange", 0, Some(1), Sequence, None, 35),
    ]),
    ty("w:CT_PPrBase", &[
        ch("w:pStyle", 0, Some(1), Sequence, None, 0),
        ch("w:keepNext", 0, Some(1), Sequence, None, 1),
        ch("w:keepLines", 0, Some(1), Sequence, None, 2),
        ch("w:pageBreakBefore", 0, Some(1), Sequence, None, 3),
        ch("w:framePr", 0, Some(1), Sequence, None, 4),
        ch("w:widowControl", 0, Some(1), Sequence, None, 5),
        ch("w:numPr", 0, Some(1), Sequence, None, 6),
        ch("w:suppressLineNumbers", 0, Some(1), Sequence, None, 7),
        ch("w:pBdr", 0, Some(1), Sequence, None, 8),
        ch("w:shd", 0, Some(1), Sequence, None, 9),
        ch("w:tabs", 0, Some(1), Sequence, None, 10),
        ch("w:suppressAutoHyphens", 0, Some(1), Sequence, None, 11),
        ch("w:kinsoku", 0, Some(1), Sequence, None, 12),
        ch("w:wordWrap", 0, Some(1), Sequence, None, 13),
        ch("w:overflowPunct", 0, Some(1), Sequence, None, 14),
        ch("w:topLinePunct", 0, Some(1), Sequence, None, 15),
        ch("w:autoSpaceDE", 0, Some(1), Sequence, None, 16),
        ch("w:autoSpaceDN", 0, Some(1), Sequence, None, 17),
        ch("w:bidi", 0, Some(1), Sequence, None, 18),
        ch("w:adjustRightInd", 0, Some(1), Sequence, None, 19),
        ch("w:snapToGrid", 0, Some(1), Sequence, None, 20),
        ch("w:spacing", 0, Some(1), Sequence, None, 21),
        ch("w:ind", 0, Some(1), Sequence, None, 22),
        ch("w:contextualSpacing", 0, Some(1), Sequence, None, 23),
        ch("w:mirrorIndents", 0, Some(1), Sequence, None, 24),
        ch("w:suppressOverlap", 0, Some(1), Sequence, None, 25),
        ch("w:jc", 0, Some(1), Sequence, None, 26),
        ch("w:textDirection", 0, Some(1), Sequence, None, 27),
        ch("w:textAlignment", 0, Some(1), Sequence, None, 28),
        ch("w:textboxTightWrap", 0, Some(1), Sequence, None, 29),
        ch("w:outlineLvl", 0, Some(1), Sequence, None, 30),
        ch("w:divId", 0, Some(1), Sequence, None, 31),
        ch("w:cnfStyle", 0, Some(1), Sequence, None, 32),
    ]),
    ty("w:CT_PPrChange", &[
        ch("w:pPr", 1, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_PPrDefault", &[
        ch("w:pPr", 0, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_PPrGeneral", &[
        ch("w:pStyle", 0, Some(1), Sequence, None, 0),
        ch("w:keepNext", 0, Some(1), Sequence, None, 1),
        ch("w:keepLines", 0, Some(1), Sequence, None, 2),
        ch("w:pageBreakBefore", 0, Some(1), Sequence, None, 3),
        ch("w:framePr", 0, Some(1), Sequence, None, 4),
        ch("w:widowControl", 0, Some(1), Sequence, None, 5),
        ch("w:numPr", 0, Some(1), Sequence, None, 6),
        ch("w:suppressLineNumbers", 0, Some(1), Sequence, None, 7),
        ch("w:pBdr", 0, Some(1), Sequence, None, 8),
        ch("w:shd", 0, Some(1), Sequence, None, 9),
        ch("w:tabs", 0, Some(1), Sequence, None, 10),
        ch("w:suppressAutoHyphens", 0, Some(1), Sequence, None, 11),
        ch("w:kinsoku", 0, Some(1), Sequence, None, 12),
        ch("w:wordWrap", 0, Some(1), Sequence, None, 13),
        ch("w:overflowPunct", 0, Some(1), Sequence, None, 14),
        ch("w:topLinePunct", 0, Some(1), Sequence, None, 15),
        ch("w:autoSpaceDE", 0, Some(1), Sequence, None, 16),
        ch("w:autoSpaceDN", 0, Some(1), Sequence, None, 17),
        ch("w:bidi", 0, Some(1), Sequence, None, 18),
        ch("w:adjustRightInd", 0, Some(1), Sequence, None, 19),
        ch("w:snapToGrid", 0, Some(1), Sequence, None, 20),
        ch("w:spacing", 0, Some(1), Sequence, None, 21),
        ch("w:ind", 0, Some(1), Sequence, None, 22),
        ch("w:contextualSpacing", 0, Some(1), Sequence, None, 23),
        ch("w:mirrorIndents", 0, Some(1), Sequence, None, 24),
        ch("w:suppressOverlap", 0, Some(1), Sequence, None, 25),
        ch("w:jc", 0, Some(1), Sequence, None, 26),
        ch("w:textDirection", 0, Some(1), Sequence, None, 27),
        ch("w:textAlignment", 0, Some(1), Sequence, None, 28),
        ch("w:textboxTightWrap", 0, Some(1), Sequence, None, 29),
        ch("w:outlineLvl", 0, Some(1), Sequence, None, 30),
        ch("w:divId", 0, Some(1), Sequence, None, 31),
        ch("w:cnfStyle", 0, Some(1), Sequence, None, 32),
        ch("w:pPrChange", 0, Some(1), Sequence, None, 33),
    ]),
    ty("w:CT_PTab", &[]),
    ty("w:CT_PageBorder", &[]),
    ty("w:CT_PageBorders", &[
        ch("w:top", 0, Some(1), Sequence, None, 0),
        ch("w:left", 0, Some(1), Sequence, None, 1),
        ch("w:bottom", 0, Some(1), Sequence, None, 2),
        ch("w:right", 0, Some(1), Sequence, None, 3),
    ]),
    ty("w:CT_PageMar", &[]),
    ty("w:CT_PageNumber", &[]),
    ty("w:CT_PageSz", &[]),
    ty("w:CT_Panose", &[]),
    ty("w:CT_PaperSource", &[]),
    ty("w:CT_ParaRPr", &[
        ch("w:ins", 0, Some(1), Sequence, None, 0),
        ch("w:del", 0, Some(1), Sequence, None, 1),
        ch("w:moveFrom", 0, Some(1), Sequence, None, 2),
        ch("w:moveTo", 0, Some(1), Sequence, None, 3),
        ch("w:rStyle", 0, None, Choice, Some("choice1"), 4),
        ch("w:rFonts", 0, None, Choice, Some("choice1"), 5),
        ch("w:b", 0, None, Choice, Some("choice1"), 6),
        ch("w:bCs", 0, None, Choice, Some("choice1"), 7),
        ch("w:i", 0, None, Choice, Some("choice1"), 8),
        ch("w:iCs", 0, None, Choice, Some("choice1"), 9),
        ch("w:caps", 0, None, Choice, Some("choice1"), 10),
        ch("w:smallCaps", 0, None, Choice, Some("choice1"), 11),
        ch("w:strike", 0, None, Choice, Some("choice1"), 12),
        ch("w:dstrike", 0, None, Choice, Some("choice1"), 13),
        ch("w:outline", 0, None, Choice, Some("choice1"), 14),
        ch("w:shadow", 0, None, Choice, Some("choice1"), 15),
        ch("w:emboss", 0, None, Choice, Some("choice1"), 16),
        ch("w:imprint", 0, None, Choice, Some("choice1"), 17),
        ch("w:noProof", 0, None, Choice, Some("choice1"), 18),
        ch("w:snapToGrid", 0, None, Choice, Some("choice1"), 19),
        ch("w:vanish", 0, None, Choice, Some("choice1"), 20),
        ch("w:webHidden", 0, None, Choice, Some("choice1"), 21),
        ch("w:color", 0, None, Choice, Some("choice1"), 22),
        ch("w:spacing", 0, None, Choice, Some("choice1"), 23),
        ch("w:w", 0, None, Choice, Some("choice1"), 24),
        ch("w:kern", 0, None, Choice, Some("choice1"), 25),
        ch("w:position", 0, None, Choice, Some("choice1"), 26),
        ch("w:sz", 0, None, Choice, Some("choice1"), 27),
        ch("w:szCs", 0, None, Choice, Some("choice1"), 28),
        ch("w:highlight", 0, None, Choice, Some("choice1"), 29),
        ch("w:u", 0, None, Choice, Some("choice1"), 30),
        ch("w:effect", 0, None, Choice, Some("choice1"), 31),
        ch("w:bdr", 0, None, Choice, Some("choice1"), 32),
        ch("w:shd", 0, None, Choice, Some("choice1"), 33),
        ch("w:fitText", 0, None, Choice, Some("choice1"), 34),
        ch("w:vertAlign", 0, None, Choice, Some("choice1"), 35),
        ch("w:rtl", 0, None, Choice, Some("choice1"), 36),
        ch("w:cs", 0, None, Choice, Some("choice1"), 37),
        ch("w:em", 0, None, Choice, Some("choice1"), 38),
        ch("w:lang", 0, None, Choice, Some("choice1"), 39),
        ch("w:eastAsianLayout", 0, None, Choice, Some("choice1"), 40),
        ch("w:specVanish", 0, None, Choice, Some("choice1"), 41),
        ch("w:oMath", 0, None, Choice, Some("choice1"), 42),
        ch("w:rPrChange", 0, Some(1), Sequence, None, 43),
    ]),
    ty("w:CT_ParaRPrChange", &[
        ch("w:rPr", 1, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_ParaRPrOriginal", &[
        ch("w:ins", 0, Some(1), Sequence, None, 0),
        ch("w:del", 0, Some(1), Sequence, None, 1),
        ch("w:moveFrom", 0, Some(1), Sequence, None, 2),
        ch("w:moveTo", 0, Some(1), Sequence, None, 3),
        ch("w:rStyle", 0, None, Choice, Some("choice1"), 4),
        ch("w:rFonts", 0, None, Choice, Some("choice1"), 5),
        ch("w:b", 0, None, Choice, Some("choice1"), 6),
        ch("w:bCs", 0, None, Choice, Some("choice1"), 7),
        ch("w:i", 0, None, Choice, Some("choice1"), 8),
        ch("w:iCs", 0, None, Choice, Some("choice1"), 9),
        ch("w:caps", 0, None, Choice, Some("choice1"), 10),
        ch("w:smallCaps", 0, None, Choice, Some("choice1"), 11),
        ch("w:strike", 0, None, Choice, Some("choice1"), 12),
        ch("w:dstrike", 0, None, Choice, Some("choice1"), 13),
        ch("w:outline", 0, None, Choice, Some("choice1"), 14),
        ch("w:shadow", 0, None, Choice, Some("choice1"), 15),
        ch("w:emboss", 0, None, Choice, Some("choice1"), 16),
        ch("w:imprint", 0, None, Choice, Some("choice1"), 17),
        ch("w:noProof", 0, None, Choice, Some("choice1"), 18),
        ch("w:snapToGrid", 0, None, Choice, Some("choice1"), 19),
        ch("w:vanish", 0, None, Choice, Some("choice1"), 20),
        ch("w:webHidden", 0, None, Choice, Some("choice1"), 21),
        ch("w:color", 0, None, Choice, Some("choice1"), 22),
        ch("w:spacing", 0, None, Choice, Some("choice1"), 23),
        ch("w:w", 0, None, Choice, Some("choice1"), 24),
        ch("w:kern", 0, None, Choice, Some("choice1"), 25),
        ch("w:position", 0, None, Choice, Some("choice1"), 26),
        ch("w:sz", 0, None, Choice, Some("choice1"), 27),
        ch("w:szCs", 0, None, Choice, Some("choice1"), 28),
        ch("w:highlight", 0, None, Choice, Some("choice1"), 29),
        ch("w:u", 0, None, Choice, Some("choice1"), 30),
        ch("w:effect", 0, None, Choice, Some("choice1"), 31),
        ch("w:bdr", 0, None, Choice, Some("choice1"), 32),
        ch("w:shd", 0, None, Choice, Some("choice1"), 33),
        ch("w:fitText", 0, None, Choice, Some("choice1"), 34),
        ch("w:vertAlign", 0, None, Choice, Some("choice1"), 35),
        ch("w:rtl", 0, None, Choice, Some("choice1"), 36),
        ch("w:cs", 0, None, Choice, Some("choice1"), 37),
        ch("w:em", 0, None, Choice, Some("choice1"), 38),
        ch("w:lang", 0, None, Choice, Some("choice1"), 39),
        ch("w:eastAsianLayout", 0, None, Choice, Some("choice1"), 40),
        ch("w:specVanish", 0, None, Choice, Some("choice1"), 41),
        ch("w:oMath", 0, None, Choice, Some("choice1"), 42),
    ]),
    ty("w:CT_Perm", &[]),
    ty("w:CT_PermStart", &[]),
    ty("w:CT_Pitch", &[]),
    ty("w:CT_PixelsMeasure", &[]),
    ty("w:CT_Placeholder", &[
        ch("w:docPart", 1, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_Proof", &[]),
    ty("w:CT_ProofErr", &[]),
    ty("w:CT_R", &[
        ch("w:rPr", 0, Some(1), Sequence, None, 0),
        ch("w:br", 0, None, Choice, Some("choice1"), 1),
        ch("w:t", 0, None, Choice, Some("choice1"), 2),
        ch("w:contentPart", 0, None, Choice, Some("choice1"), 3),
        ch("w:delText", 0, None, Choice, Some("choice1"), 4),
        ch("w:instrText", 0, None, Choice, Some("choice1"), 5),
        ch("w:delInstrText", 0, None, Choice, Some("choice1"), 6),
        ch("w:noBreakHyphen", 0, None, Choice, Some("choice1"), 7),
        ch("w:softHyphen", 0, None, Choice, Some("choice1"), 8),
        ch("w:dayShort", 0, None, Choice, Some("choice1"), 9),
        ch("w:monthShort", 0, None, Choice, Some("choice1"), 10),
        ch("w:yearShort", 0, None, Choice, Some("choice1"), 11),
        ch("w:dayLong", 0, None, Choice, Some("choice1"), 12),
        ch("w:monthLong", 0, None, Choice, Some("choice1"), 13),
        ch("w:yearLong", 0, None, Choice, Some("choice1"), 14),
        ch("w:annotationRef", 0, None, Choice, Some("choice1"), 15),
        ch("w:footnoteRef", 0, None, Choice, Some("choice1"), 16),
        ch("w:endnoteRef", 0, None, Choice, Some("choice1"), 17),
        ch("w:separator", 0, None, Choice, Some("choice1"), 18),
        ch("w:continuationSeparator", 0, None, Choice, Some("choice1"), 19),
        ch("w:sym", 0, None, Choice, Some("choice1"), 20),
        ch("w:pgNum", 0, None, Choice, Some("choice1"), 21),
        ch("w:cr", 0, None, Choice, Some("choice1"), 22),
        ch("w:tab", 0, None, Choice, Some("choice1"), 23),
        ch("w:object", 0, None, Choice, Some("choice1"), 24),
        ch("w:fldChar", 0, None, Choice, Some("choice1"), 25),
        ch("w:ruby", 0, None, Choice, Some("choice1"), 26),
        ch("w:footnoteReference", 0, None, Choice, Some("choice1"), 27),
        ch("w:endnoteReference", 0, None, Choice, Some("choice1"), 28),
        ch("w:commentReference", 0, None, Choice, Some("choice1"), 29),
        ch("w:drawing", 0, None, Choice, Some("choice1"), 30),
        ch("w:ptab", 0, None, Choice, Some("choice1"), 31),
        ch("w:lastRenderedPageBreak", 0, None, Choice, Some("choice1"), 32),
    ]),
    ty("w:CT_RPr", &[
        ch("w:rStyle", 0, None, Choice, Some("choice1"), 0),
        ch("w:rFonts", 0, None, Choice, Some("choice1"), 1),
        ch("w:b", 0, None, Choice, Some("choice1"), 2),
        ch("w:bCs", 0, None, Choice, Some("choice1"), 3),
        ch("w:i", 0, None, Choice, Some("choice1"), 4),
        ch("w:iCs", 0, None, Choice, Some("choice1"), 5),
        ch("w:caps", 0, None, Choice, Some("choice1"), 6),
        ch("w:smallCaps", 0, None, Choice, Some("choice1"), 7),
        ch("w:strike", 0, None, Choice, Some("choice1"), 8),
        ch("w:dstrike", 0, None, Choice, Some("choice1"), 9),
        ch("w:outline", 0, None, Choice, Some("choice1"), 10),
        ch("w:shadow", 0, None, Choice, Some("choice1"), 11),
        ch("w:emboss", 0, None, Choice, Some("choice1"), 12),
        ch("w:imprint", 0, None, Choice, Some("choice1"), 13),
        ch("w:noProof", 0, None, Choice, Some("choice1"), 14),
        ch("w:snapToGrid", 0, None, Choice, Some("choice1"), 15),
        ch("w:vanish", 0, None, Choice, Some("choice1"), 16),
        ch("w:webHidden", 0, None, Choice, Some("choice1"), 17),
        ch("w:color", 0, None, Choice, Some("choice1"), 18),
        ch("w:spacing", 0, None, Choice, Some("choice1"), 19),
        ch("w:w", 0, None, Choice, Some("choice1"), 20),
        ch("w:kern", 0, None, Choice, Some("choice1"), 21),
        ch("w:position", 0, None, Choice, Some("choice1"), 22),
        ch("w:sz", 0, None, Choice, Some("choice1"), 23),
        ch("w:szCs", 0, None, Choice, Some("choice1"), 24),
        ch("w:highlight", 0, None, Choice, Some("choice1"), 25),
        ch("w:u", 0, None, Choice, Some("choice1"), 26),
        ch("w:effect", 0, None, Choice, Some("choice1"), 27),
        ch("w:bdr", 0, None, Choice, Some("choice1"), 28),
        ch("w:shd", 0, None, Choice, Some("choice1"), 29),
        ch("w:fitText", 0, None, Choice, Some("choice1"), 30),
        ch("w:vertAlign", 0, None, Choice, Some("choice1"), 31),
        ch("w:rtl", 0, None, Choice, Some("choice1"), 32),
        ch("w:cs", 0, None, Choice, Some("choice1"), 33),
        ch("w:em", 0, None, Choice, Some("choice1"), 34),
        ch("w:lang", 0, None, Choice, Some("choice1"), 35),
        ch("w:eastAsianLayout", 0, None, Choice, Some("choice1"), 36),
        ch("w:specVanish", 0, None, Choice, Some("choice1"), 37),
        ch("w:oMath", 0, None, Choice, Some("choice1"), 38),
        ch("w:rPrChange", 0, Some(1), Sequence, None, 39),
    ]),
    ty("w:CT_RPrChange", &[
        ch("w:rPr", 1, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_RPrDefault", &[
        ch("w:rPr", 0, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_RPrOriginal", &[
        ch("w:rStyle", 0, None, Choice, Some("choice1"), 0),
        ch("w:rFonts", 0, None, Choice, Some("choice1"), 1),
        ch("w:b", 0, None, Choice, Some("choice1"), 2),
        ch("w:bCs", 0, None, Choice, Some("choice1"), 3),
        ch("w:i", 0, None, Choice, Some("choice1"), 4),
        ch("w:iCs", 0, None, Choice, Some("choice1"), 5),
        ch("w:caps", 0, None, Choice, Some("choice1"), 6),
        ch("w:smallCaps", 0, None, Choice, Some("choice1"), 7),
        ch("w:strike", 0, None, Choice, Some("choice1"), 8),
        ch("w:dstrike", 0, None, Choice, Some("choice1"), 9),
        ch("w:outline", 0, None, Choice, Some("choice1"), 10),
        ch("w:shadow", 0, None, Choice, Some("choice1"), 11),
        ch("w:emboss", 0, None, Choice, Some("choice1"), 12),
        ch("w:imprint", 0, None, Choice, Some("choice1"), 13),
        ch("w:noProof", 0, None, Choice, Some("choice1"), 14),
        ch("w:snapToGrid", 0, None, Choice, Some("choice1"), 15),
        ch("w:vanish", 0, None, Choice, Some("choice1"), 16),
        ch("w:webHidden", 0, None, Choice, Some("choice1"), 17),
        ch("w:color", 0, None, Choice, Some("choice1"), 18),
        ch("w:spacing", 0, None, Choice, Some("choice1"), 19),
        ch("w:w", 0, None, Choice, Some("choice1"), 20),
        ch("w:kern", 0, None, Choice, Some("choice1"), 21),
        ch("w:position", 0, None, Choice, Some("choice1"), 22),
        ch("w:sz", 0, None, Choice, Some("choice1"), 23),
        ch("w:szCs", 0, None, Choice, Some("choice1"), 24),
        ch("w:highlight", 0, None, Choice, Some("choice1"), 25),
        ch("w:u", 0, None, Choice, Some("choice1"), 26),
        ch("w:effect", 0, None, Choice, Some("choice1"), 27),
        ch("w:bdr", 0, None, Choice, Some("choice1"), 28),
        ch("w:shd", 0, None, Choice, Some("choice1"), 29),
        ch("w:fitText", 0, None, Choice, Some("choice1"), 30),
        ch("w:vertAlign", 0, None, Choice, Some("choice1"), 31),
        ch("w:rtl", 0, None, Choice, Some("choice1"), 32),
        ch("w:cs", 0, None, Choice, Some("choice1"), 33),
        ch("w:em", 0, None, Choice, Some("choice1"), 34),
        ch("w:lang", 0, None, Choice, Some("choice1"), 35),
        ch("w:eastAsianLayout", 0, None, Choice, Some("choice1"), 36),
        ch("w:specVanish", 0, None, Choice, Some("choice1"), 37),
        ch("w:oMath", 0, None, Choice, Some("choice1"), 38),
    ]),
    ty("w:CT_ReadingModeInkLockDown", &[]),
    ty("w:CT_RecipientData", &[
        ch("w:active", 0, Some(1), Sequence, None, 0),
        ch("w:column", 1, Some(1), Sequence, None, 1),
        ch("w:uniqueTag", 1, Some(1), Sequence, None, 2),
    ]),
    ty("w:CT_Recipients", &[
        ch("w:recipientData", 1, None, Sequence, None, 0),
    ]),
    ty("w:CT_Rel", &[]),
    ty("w:CT_Row", &[
        ch("w:tblPrEx", 0, Some(1), Sequence, None, 0),
        ch("w:trPr", 0, Some(1), Sequence, None, 1),
        ch("w:tc", 0, None, Choice, Some("choice1"), 2),
        ch("w:customXml", 0, None, Choice, Some("choice1"), 3),
        ch("w:sdt", 0, None, Choice, Some("choice1"), 4),
        ch("w:proofErr", 0, None, Choice, Some("choice2"), 5),
        ch("w:permStart", 0, None, Choice, Some("choice2"), 6),
        ch("w:permEnd", 0, None, Choice, Some("choice2"), 7),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice3"), 8),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice3"), 9),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice3"), 10),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice3"), 11),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice3"), 12),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice3"), 13),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice3"), 14),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice3"), 15),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice3"), 16),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice3"), 17),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice3"), 18),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice3"), 19),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice3"), 20),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice3"), 21),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice3"), 22),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice3"), 23),
        ch("w:ins", 0, None, Choice, Some("choice2"), 24),
        ch("w:del", 0, None, Choice, Some("choice2"), 25),
        ch("w:moveFrom", 0, None, Choice, Some("choice2"), 26),
        ch("w:moveTo", 0, None, Choice, Some("choice2"), 27),
        ch("m:oMathPara", 0, None, Choice, Some("choice4"), 28),
        ch("m:oMath", 0, None, Choice, Some("choice4"), 29),
    ]),
    ty("w:CT_Ruby", &[
        ch("w:rubyPr", 1, Some(1), Sequence, None, 0),
        ch("w:rt", 1, Some(1), Sequence, None, 1),
        ch("w:rubyBase", 1, Some(1), Sequence, None, 2),
    ]),
    ty("w:CT_RubyAlign", &[]),
    ty("w:CT_RubyContent", &[]),
    ty("w:CT_RubyPr", &[
        ch("w:rubyAlign", 1, Some(1), Sequence, None, 0),
        ch("w:hps", 1, Some(1), Sequence, None, 1),
        ch("w:hpsRaise", 1, Some(1), Sequence, None, 2),
        ch("w:hpsBaseText", 1, Some(1), Sequence, None, 3),
        ch("w:lid", 1, Some(1), Sequence, None, 4),
        ch("w:dirty", 0, Some(1), Sequence, None, 5),
    ]),
    ty("w:CT_RunTrackChange", &[
        ch("w:customXml", 0, None, Choice, Some("choice2"), 0),
        ch("w:smartTag", 0, None, Choice, Some("choice2"), 1),
        ch("w:sdt", 0, None, Choice, Some("choice2"), 2),
        ch("w:dir", 0, None, Choice, Some("choice2"), 3),
        ch("w:bdo", 0, None, Choice, Some("choice2"), 4),
        ch("w:r", 0, None, Choice, Some("choice2"), 5),
        ch("w:proofErr", 0, None, Choice, Some("choice3"), 6),
        ch("w:permStart", 0, None, Choice, Some("choice3"), 7),
        ch("w:permEnd", 0, None, Choice, Some("choice3"), 8),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice4"), 9),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice4"), 10),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice4"), 11),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice4"), 12),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice4"), 13),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice4"), 14),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice4"), 15),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice4"), 16),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice4"), 17),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice4"), 18),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice4"), 19),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice4"), 20),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice4"), 21),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice4"), 22),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice4"), 23),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice4"), 24),
        ch("w:ins", 0, None, Choice, Some("choice3"), 25),
        ch("w:del", 0, None, Choice, Some("choice3"), 26),
        ch("w:moveFrom", 0, None, Choice, Some("choice3"), 27),
        ch("w:moveTo", 0, None, Choice, Some("choice3"), 28),
        ch("m:oMathPara", 0, None, Choice, Some("choice5"), 29),
        ch("m:oMath", 0, None, Choice, Some("choice5"), 30),
        ch("m:acc", 0, None, Choice, Some("choice6"), 31),
        ch("m:bar", 0, None, Choice, Some("choice6"), 32),
        ch("m:box", 0, None, Choice, Some("choice6"), 33),
        ch("m:borderBox", 0, None, Choice, Some("choice6"), 34),
        ch("m:d", 0, None, Choice, Some("choice6"), 35),
        ch("m:eqArr", 0, None, Choice, Some("choice6"), 36),
        ch("m:f", 0, None, Choice, Some("choice6"), 37),
        ch("m:func", 0, None, Choice, Some("choice6"), 38),
        ch("m:groupChr", 0, None, Choice, Some("choice6"), 39),
        ch("m:limLow", 0, None, Choice, Some("choice6"), 40),
        ch("m:limUpp", 0, None, Choice, Some("choice6"), 41),
        ch("m:m", 0, None, Choice, Some("choice6"), 42),
        ch("m:nary", 0, None, Choice, Some("choice6"), 43),
        ch("m:phant", 0, None, Choice, Some("choice6"), 44),
        ch("m:rad", 0, None, Choice, Some("choice6"), 45),
        ch("m:sPre", 0, None, Choice, Some("choice6"), 46),
        ch("m:sSub", 0, None, Choice, Some("choice6"), 47),
        ch("m:sSubSup", 0, None, Choice, Some("choice6"), 48),
        ch("m:sSup", 0, None, Choice, Some("choice6"), 49),
        ch("m:r", 0, None, Choice, Some("choice6"), 50),
    ]),
    ty("w:CT_SaveThroughXslt", &[]),
    ty("w:CT_SdtBlock", &[
        ch("w:sdtPr", 0, Some(1), Sequence, None, 0),
        ch("w:sdtEndPr", 0, Some(1), Sequence, None, 1),
        ch("w:sdtContent", 0, Some(1), Sequence, None, 2),
    ]),
    ty("w:CT_SdtCell", &[
        ch("w:sdtPr", 0, Some(1), Sequence, None, 0),
        ch("w:sdtEndPr", 0, Some(1), Sequence, None, 1),
        ch("w:sdtContent", 0, Some(1), Sequence, None, 2),
    ]),
    ty("w:CT_SdtComboBox", &[
        ch("w:listItem", 0, None, Sequence, None, 0),
    ]),
    ty("w:CT_SdtContentBlock", &[]),
    ty("w:CT_SdtContentCell", &[]),
    ty("w:CT_SdtContentRow", &[]),
    ty("w:CT_SdtContentRun", &[]),
    ty("w:CT_SdtDate", &[
        ch("w:dateFormat", 0, Some(1), Sequence, None, 0),
        ch("w:lid", 0, Some(1), Sequence, None, 1),
        ch("w:storeMappedDataAs", 0, Some(1), Sequence, None, 2),
        ch("w:calendar", 0, Some(1), Sequence, None, 3),
    ]),
    ty("w:CT_SdtDateMappingType", &[]),
    ty("w:CT_SdtDocPart", &[
        ch("w:docPartGallery", 0, Some(1), Sequence, None, 0),
        ch("w:docPartCategory", 0, Some(1), Sequence, None, 1),
        ch("w:docPartUnique", 0, Some(1), Sequence, None, 2),
    ]),
    ty("w:CT_SdtDropDownList", &[
        ch("w:listItem", 0, None, Sequence, None, 0),
    ]),
    ty("w:CT_SdtEndPr", &[
        ch("w:rPr", 0, None, Choice, Some("choice1"), 0),
    ]),
    ty("w:CT_SdtListItem", &[]),
    ty("w:CT_SdtPr", &[
        ch("w:rPr", 0, Some(1), Sequence, None, 0),
        ch("w:alias", 0, Some(1), Sequence, None, 1),
        ch("w:tag", 0, Some(1), Sequence, None, 2),
        ch("w:id", 0, Some(1), Sequence, None, 3),
        ch("w:lock", 0, Some(1), Sequence, None, 4),
        ch("w:placeholder", 0, Some(1), Sequence, None, 5),
        ch("w:temporary", 0, Some(1), Sequence, None, 6),
        ch("w:showingPlcHdr", 0, Some(1), Sequence, None, 7),
        ch("w:dataBinding", 0, Some(1), Sequence, None, 8),
        ch("w:label", 0, Some(1), Sequence, None, 9),
        ch("w:tabIndex", 0, Some(1), Sequence, None, 10),
        ch("w:equation", 0, Some(1), Choice, Some("choice1"), 11),
        ch("w:comboBox", 0, Some(1), Choice, Some("choice1"), 12),
        ch("w:date", 0, Some(1), Choice, Some("choice1"), 13),
        ch("w:docPartObj", 0, Some(1), Choice, Some("choice1"), 14),
        ch("w:docPartList", 0, Some(1), Choice, Some("choice1"), 15),
        ch("w:dropDownList", 0, Some(1), Choice, Some("choice1"), 16),
        ch("w:picture", 0, Some(1), Choice, Some("choice1"), 17),
        ch("w:richText", 0, Some(1), Choice, Some("choice1"), 18),
        ch("w:text", 0, Some(1), Choice, Some("choice1"), 19),
        ch("w:citation", 0, Some(1), Choice, Some("choice1"), 20),
        ch("w:group", 0, Some(1), Choice, Some("choice1"), 21),
        ch("w:bibliography", 0, Some(1), Choice, Some("choice1"), 22),
    ]),
    ty("w:CT_SdtRow", &[
        ch("w:sdtPr", 0, Some(1), Sequence, None, 0),
        ch("w:sdtEndPr", 0, Some(1), Sequence, None, 1),
        ch("w:sdtContent", 0, Some(1), Sequence, None, 2),
    ]),
    ty("w:CT_SdtRun", &[
        ch("w:sdtPr", 0, Some(1), Sequence, None, 0),
        ch("w:sdtEndPr", 0, Some(1), Sequence, None, 1),
        ch("w:sdtContent", 0, Some(1), Sequence, None, 2),
    ]),
    ty("w:CT_SdtText", &[]),
    ty("w:CT_SectPr", &[
        ch("w:headerReference", 0, Some(6), Choice, Some("choice1"), 0),
        ch("w:footerReference", 0, Some(6), Choice, Some("choice1"), 1),
        ch("w:footnotePr", 0, Some(1), Sequence, None, 2),
        ch("w:endnotePr", 0, Some(1), Sequence, None, 3),
        ch("w:type", 0, Some(1), Sequence, None, 4),
        ch("w:pgSz", 0, Some(1), Sequence, None, 5),
        ch("w:pgMar", 0, Some(1), Sequence, None, 6),
        ch("w:paperSrc", 0, Some(1), Sequence, None, 7),
        ch("w:pgBorders", 0, Some(1), Sequence, None, 8),
        ch("w:lnNumType", 0, Some(1), Sequence, None, 9),
        ch("w:pgNumType", 0, Some(1), Sequence, None, 10),
        ch("w:cols", 0, Some(1), Sequence, None, 11),
        ch("w:formProt", 0, Some(1), Sequence, None, 12),
        ch("w:vAlign", 0, Some(1), Sequence, None, 13),
        ch("w:noEndnote", 0, Some(1), Sequence, None, 14),
        ch("w:titlePg", 0, Some(1), Sequence, None, 15),
        ch("w:textDirection", 0, Some(1), Sequence, None, 16),
        ch("w:bidi", 0, Some(1), Sequence, None, 17),
        ch("w:rtlGutter", 0, Some(1), Sequence, None, 18),
        ch("w:docGrid", 0, Some(1), Sequence, None, 19),
        ch("w:printerSettings", 0, Some(1), Sequence, None, 20),
        ch("w:sectPrChange", 0, Some(1), Sequence, None, 21),
    ]),
    ty("w:CT_SectPrBase", &[
        ch("w:footnotePr", 0, Some(1), Sequence, None, 0),
        ch("w:endnotePr", 0, Some(1), Sequence, None, 1),
        ch("w:type", 0, Some(1), Sequence, None, 2),
        ch("w:pgSz", 0, Some(1), Sequence, None, 3),
        ch("w:pgMar", 0, Some(1), Sequence, None, 4),
        ch("w:paperSrc", 0, Some(1), Sequence, None, 5),
        ch("w:pgBorders", 0, Some(1), Sequence, None, 6),
        ch("w:lnNumType", 0, Some(1), Sequence, None, 7),
        ch("w:pgNumType", 0, Some(1), Sequence, None, 8),
        ch("w:cols", 0, Some(1), Sequence, None, 9),
        ch("w:formProt", 0, Some(1), Sequence, None, 10),
        ch("w:vAlign", 0, Some(1), Sequence, None, 11),
        ch("w:noEndnote", 0, Some(1), Sequence, None, 12),
        ch("w:titlePg", 0, Some(1), Sequence, None, 13),
        ch("w:textDirection", 0, Some(1), Sequence, None, 14),
        ch("w:bidi", 0, Some(1), Sequence, None, 15),
        ch("w:rtlGutter", 0, Some(1), Sequence, None, 16),
        ch("w:docGrid", 0, Some(1), Sequence, None, 17),
        ch("w:printerSettings", 0, Some(1), Sequence, None, 18),
    ]),
    ty("w:CT_SectPrChange", &[
        ch("w:sectPr", 0, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_SectType", &[]),
    ty("w:CT_Settings", &[
        ch("w:writeProtection", 0, Some(1), Sequence, None, 0),
        ch("w:view", 0, Some(1), Sequence, None, 1),
        ch("w:zoom", 0, Some(1), Sequence, None, 2),
        ch("w:removePersonalInformation", 0, Some(1), Sequence, None, 3),
        ch("w:removeDateAndTime", 0, Some(1), Sequence, None, 4),
        ch("w:doNotDisplayPageBoundaries", 0, Some(1), Sequence, None, 5),
        ch("w:displayBackgroundShape", 0, Some(1), Sequence, None, 6),
        ch("w:printPostScriptOverText", 0, Some(1), Sequence, None, 7),
        ch("w:printFractionalCharacterWidth", 0, Some(1), Sequence, None, 8),
        ch("w:printFormsData", 0, Some(1), Sequence, None, 9),
        ch("w:embedTrueTypeFonts", 0, Some(1), Sequence, None, 10),
        ch("w:embedSystemFonts", 0, Some(1), Sequence, None, 11),
        ch("w:saveSubsetFonts", 0, Some(1), Sequence, None, 12),
        ch("w:saveFormsData", 0, Some(1), Sequence, None, 13),
        ch("w:mirrorMargins", 0, Some(1), Sequence, None, 14),
        ch("w:alignBordersAndEdges", 0, Some(1), Sequence, None, 15),
        ch("w:bordersDoNotSurroundHeader", 0, Some(1), Sequence, None, 16),
        ch("w:bordersDoNotSurroundFooter", 0, Some(1), Sequence, None, 17),
        ch("w:gutterAtTop", 0, Some(1), Sequence, None, 18),
        ch("w:hideSpellingErrors", 0, Some(1), Sequence, None, 19),
        ch("w:hideGrammaticalErrors", 0, Some(1), Sequence, None, 20),
        ch("w:activeWritingStyle", 0, None, Sequence, None, 21),
        ch("w:proofState", 0, Some(1), Sequence, None, 22),
        ch("w:formsDesign", 0, Some(1), Sequence, None, 23),
        ch("w:attachedTemplate", 0, Some(1), Sequence, None, 24),
        ch("w:linkStyles", 0, Some(1), Sequence, None, 25),
        ch("w:stylePaneFormatFilter", 0, Some(1), Sequence, None, 26),
        ch("w:stylePaneSortMethod", 0, Some(1), Sequence, None, 27),
        ch("w:documentType", 0, Some(1), Sequence, None, 28),
        ch("w:mailMerge", 0, Some(1), Sequence, None, 29),
        ch("w:revisionView", 0, Some(1), Sequence, None, 30),
        ch("w:trackRevisions", 0, Some(1), Sequence, None, 31),
        ch("w:doNotTrackMoves", 0, Some(1), Sequence, None, 32),
        ch("w:doNotTrackFormatting", 0, Some(1), Sequence, None, 33),
        ch("w:documentProtection", 0, Some(1), Sequence, None, 34),
        ch("w:autoFormatOverride", 0, Some(1), Sequence, None, 35),
        ch("w:styleLockTheme", 0, Some(1), Sequence, None, 36),
        ch("w:styleLockQFSet", 0, Some(1), Sequence, None, 37),
        ch("w:defaultTabStop", 0, Some(1), Sequence, None, 38),
        ch("w:autoHyphenation", 0, Some(1), Sequence, None, 39),
        ch("w:consecutiveHyphenLimit", 0, Some(1), Sequence, None, 40),
        ch("w:hyphenationZone", 0, Some(1), Sequence, None, 41),
        ch("w:doNotHyphenateCaps", 0, Some(1), Sequence, None, 42),
        ch("w:showEnvelope", 0, Some(1), Sequence, None, 43),
        ch("w:summaryLength", 0, Some(1), Sequence, None, 44),
        ch("w:clickAndTypeStyle", 0, Some(1), Sequence, None, 45),
        ch("w:defaultTableStyle", 0, Some(1), Sequence, None, 46),
        ch("w:evenAndOddHeaders", 0, Some(1), Sequence, None, 47),
        ch("w:bookFoldRevPrinting", 0, Some(1), Sequence, None, 48),
        ch("w:bookFoldPrinting", 0, Some(1), Sequence, None, 49),
        ch("w:bookFoldPrintingSheets", 0, Some(1), Sequence, None, 50),
        ch("w:drawingGridHorizontalSpacing", 0, Some(1), Sequence, None, 51),
        ch("w:drawingGridVerticalSpacing", 0, Some(1), Sequence, None, 52),
        ch("w:displayHorizontalDrawingGridEvery", 0, Some(1), Sequence, None, 53),
        ch("w:displayVerticalDrawingGridEvery", 0, Some(1), Sequence, None, 54),
        ch("w:doNotUseMarginsForDrawingGridOrigin", 0, Some(1), Sequence, None, 55),
        ch("w:drawingGridHorizontalOrigin", 0, Some(1), Sequence, None, 56),
        ch("w:drawingGridVerticalOrigin", 0, Some(1), Sequence, None, 57),
        ch("w:doNotShadeFormData", 0, Some(1), Sequence, None, 58),
        ch("w:noPunctuationKerning", 0, Some(1), Sequence, None, 59),
        ch("w:characterSpacingControl", 0, Some(1), Sequence, None, 60),
        ch("w:printTwoOnOne", 0, Some(1), Sequence, None, 61),
        ch("w:strictFirstAndLastChars", 0, Some(1), Sequence, None, 62),
        ch("w:noLineBreaksAfter", 0, Some(1), Sequence, None, 63),
        ch("w:noLineBreaksBefore", 0, Some(1), Sequence, None, 64),
        ch("w:savePreviewPicture", 0, Some(1), Sequence, None, 65),
        ch("w:doNotValidateAgainstSchema", 0, Some(1), Sequence, None, 66),
        ch("w:saveInvalidXml", 0, Some(1), Sequence, None, 67),
        ch("w:ignoreMixedContent", 0, Some(1), Sequence, None, 68),
        ch("w:alwaysShowPlaceholderText", 0, Some(1), Sequence, None, 69),
        ch("w:doNotDemarcateInvalidXml", 0, Some(1), Sequence, None, 70),
        ch("w:saveXmlDataOnly", 0, Some(1), Sequence, None, 71),
        ch("w:useXSLTWhenSaving", 0, Some(1), Sequence, None, 72),
        ch("w:saveThroughXslt", 0, Some(1), Sequence, None, 73),
        ch("w:showXMLTags", 0, Some(1), Sequence, None, 74),
        ch("w:alwaysMergeEmptyNamespace", 0, Some(1), Sequence, None, 75),
        ch("w:updateFields", 0, Some(1), Sequence, None, 76),
        ch("w:footnotePr", 0, Some(1), Sequence, None, 77),
        ch("w:endnotePr", 0, Some(1), Sequence, None, 78),
        ch("w:compat", 0, Some(1), Sequence, None, 79),
        ch("w:docVars", 0, Some(1), Sequence, None, 80),
        ch("w:rsids", 0, Some(1), Sequence, None, 81),
        ch("m:mathPr", 0, Some(1), Sequence, None, 82),
        ch("w:attachedSchema", 0, None, Sequence, None, 83),
        ch("w:themeFontLang", 0, Some(1), Sequence, None, 84),
        ch("w:clrSchemeMapping", 0, Some(1), Sequence, None, 85),
        ch("w:doNotIncludeSubdocsInStats", 0, Some(1), Sequence, None, 86),
        ch("w:doNotAutoCompressPictures", 0, Some(1), Sequence, None, 87),
        ch("w:forceUpgrade", 0, Some(1), Sequence, None, 88),
        ch("w:captions", 0, Some(1), Sequence, None, 89),
        ch("w:readModeInkLockDown", 0, Some(1), Sequence, None, 90),
        ch("w:smartTagType", 0, None, Sequence, None, 91),
        ch("sl:schemaLibrary", 0, Some(1), Sequence, None, 92),
        ch("w:doNotEmbedSmartTags", 0, Some(1), Sequence, None, 93),
        ch("w:decimalSymbol", 0, Some(1), Sequence, None, 94),
        ch("w:listSeparator", 0, Some(1), Sequence, None, 95),
    ]),
    ty("w:CT_Shd", &[]),
    ty("w:CT_SignedHpsMeasure", &[]),
    ty("w:CT_SignedTwipsMeasure", &[]),
    ty("w:CT_SimpleField", &[
        ch("w:customXml", 0, None, Choice, Some("choice2"), 0),
        ch("w:smartTag", 0, None, Choice, Some("choice2"), 1),
        ch("w:sdt", 0, None, Choice, Some("choice2"), 2),
        ch("w:dir", 0, None, Choice, Some("choice2"), 3),
        ch("w:bdo", 0, None, Choice, Some("choice2"), 4),
        ch("w:r", 0, None, Choice, Some("choice2"), 5),
        ch("w:proofErr", 0, None, Choice, Some("choice3"), 6),
        ch("w:permStart", 0, None, Choice, Some("choice3"), 7),
        ch("w:permEnd", 0, None, Choice, Some("choice3"), 8),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice4"), 9),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice4"), 10),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice4"), 11),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice4"), 12),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice4"), 13),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice4"), 14),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice4"), 15),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice4"), 16),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice4"), 17),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice4"), 18),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice4"), 19),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice4"), 20),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice4"), 21),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice4"), 22),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice4"), 23),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice4"), 24),
        ch("w:ins", 0, None, Choice, Some("choice3"), 25),
        ch("w:del", 0, None, Choice, Some("choice3"), 26),
        ch("w:moveFrom", 0, None, Choice, Some("choice3"), 27),
        ch("w:moveTo", 0, None, Choice, Some("choice3"), 28),
        ch("m:oMathPara", 0, None, Choice, Some("choice5"), 29),
        ch("m:oMath", 0, None, Choice, Some("choice5"), 30),
        ch("w:fldSimple", 0, None, Choice, Some("choice1"), 31),
        ch("w:hyperlink", 0, None, Choice, Some("choice1"), 32),
        ch("w:subDoc", 0, None, Choice, Some("choice1"), 33),
    ]),
    ty("w:CT_SmartTagPr", &[
        ch("w:attr", 0, None, Sequence, None, 0),
    ]),
    ty("w:CT_SmartTagRun", &[
        ch("w:smartTagPr", 0, Some(1), Sequence, None, 0),
        ch("w:customXml", 0, None, Choice, Some("choice2"), 1),
        ch("w:smartTag", 0, None, Choice, Some("choice2"), 2),
        ch("w:sdt", 0, None, Choice, Some("choice2"), 3),
        ch("w:dir", 0, None, Choice, Some("choice2"), 4),
        ch("w:bdo", 0, None, Choice, Some("choice2"), 5),
        ch("w:r", 0, None, Choice, Some("choice2"), 6),
        ch("w:proofErr", 0, None, Choice, Some("choice3"), 7),
        ch("w:permStart", 0, None, Choice, Some("choice3"), 8),
        ch("w:permEnd", 0, None, Choice, Some("choice3"), 9),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice4"), 10),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice4"), 11),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice4"), 12),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice4"), 13),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice4"), 14),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice4"), 15),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice4"), 16),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice4"), 17),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice4"), 18),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice4"), 19),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice4"), 20),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice4"), 21),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice4"), 22),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice4"), 23),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice4"), 24),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice4"), 25),
        ch("w:ins", 0, None, Choice, Some("choice3"), 26),
        ch("w:del", 0, None, Choice, Some("choice3"), 27),
        ch("w:moveFrom", 0, None, Choice, Some("choice3"), 28),
        ch("w:moveTo", 0, None, Choice, Some("choice3"), 29),
        ch("m:oMathPara", 0, None, Choice, Some("choice5"), 30),
        ch("m:oMath", 0, None, Choice, Some("choice5"), 31),
        ch("w:fldSimple", 0, None, Choice, Some("choice1"), 32),
        ch("w:hyperlink", 0, None, Choice, Some("choice1"), 33),
        ch("w:subDoc", 0, None, Choice, Some("choice1"), 34),
    ]),
    ty("w:CT_SmartTagType", &[]),
    ty("w:CT_Spacing", &[]),
    ty("w:CT_String", &[]),
    ty("w:CT_Style", &[
        ch("w:name", 0, Some(1), Sequence, None, 0),
        ch("w:aliases", 0, Some(1), Sequence, None, 1),
        ch("w:basedOn", 0, Some(1), Sequence, None, 2),
        ch("w:next", 0, Some(1), Sequence, None, 3),
        ch("w:link", 0, Some(1), Sequence, None, 4),
        ch("w:autoRedefine", 0, Some(1), Sequence, None, 5),
        ch("w:hidden", 0, Some(1), Sequence, None, 6),
        ch("w:uiPriority", 0, Some(1), Sequence, None, 7),
        ch("w:semiHidden", 0, Some(1), Sequence, None, 8),
        ch("w:unhideWhenUsed", 0, Some(1), Sequence, None, 9),
        ch("w:qFormat", 0, Some(1), Sequence, None, 10),
        ch("w:locked", 0, Some(1), Sequence, None, 11),
        ch("w:personal", 0, Some(1), Sequence, None, 12),
        ch("w:personalCompose", 0, Some(1), Sequence, None, 13),
        ch("w:personalReply", 0, Some(1), Sequence, None, 14),
        ch("w:rsid", 0, Some(1), Sequence, None, 15),
        ch("w:pPr", 0, Some(1), Sequence, None, 16),
        ch("w:rPr", 0, Some(1), Sequence, None, 17),
        ch("w:tblPr", 0, Some(1), Sequence, None, 18),
        ch("w:trPr", 0, Some(1), Sequence, None, 19),
        ch("w:tcPr", 0, Some(1), Sequence, None, 20),
        ch("w:tblStylePr", 0, None, Sequence, None, 21),
    ]),
    ty("w:CT_StylePaneFilter", &[]),
    ty("w:CT_StyleSort", &[]),
    ty("w:CT_Styles", &[
        ch("w:docDefaults", 0, Some(1), Sequence, None, 0),
        ch("w:latentStyles", 0, Some(1), Sequence, None, 1),
        ch("w:style", 0, None, Sequence, None, 2),
    ]),
    ty("w:CT_Sym", &[]),
    ty("w:CT_TabStop", &[]),
    ty("w:CT_Tabs", &[
        ch("w:tab", 1, None, Sequence, None, 0),
    ]),
    ty("w:CT_TargetScreenSz", &[]),
    ty("w:CT_Tbl", &[
        ch("w:bookmarkStart", 0, None, Choice, Some("choice1"), 0),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice1"), 1),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice1"), 2),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice1"), 3),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice1"), 4),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice1"), 5),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice1"), 6),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice1"), 7),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice1"), 8),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice1"), 9),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice1"), 10),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice1"), 11),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice1"), 12),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice1"), 13),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice1"), 14),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice1"), 15),
        ch("w:tblPr", 1, Some(1), Sequence, None, 16),
        ch("w:tblGrid", 1, Some(1), Sequence, None, 17),
        ch("w:tr", 0, None, Choice, Some("choice2"), 18),
        ch("w:customXml", 0, None, Choice, Some("choice2"), 19),
        ch("w:sdt", 0, None, Choice, Some("choice2"), 20),
        ch("w:proofErr", 0, None, Choice, Some("choice3"), 21),
        ch("w:permStart", 0, None, Choice, Some("choice3"), 22),
        ch("w:permEnd", 0, None, Choice, Some("choice3"), 23),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice4"), 24),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice4"), 25),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice4"), 26),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice4"), 27),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice4"), 28),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice4"), 29),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice4"), 30),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice4"), 31),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice4"), 32),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice4"), 33),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice4"), 34),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice4"), 35),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice4"), 36),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice4"), 37),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice4"), 38),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice4"), 39),
        ch("w:ins", 0, None, Choice, Some("choice3"), 40),
        ch("w:del", 0, None, Choice, Some("choice3"), 41),
        ch("w:moveFrom", 0, None, Choice, Some("choice3"), 42),
        ch("w:moveTo", 0, None, Choice, Some("choice3"), 43),
        ch("m:oMathPara", 0, None, Choice, Some("choice5"), 44),
        ch("m:oMath", 0, None, Choice, Some("choice5"), 45),
    ]),
    ty("w:CT_TblBorders", &[
        ch("w:top", 0, Some(1), Sequence, None, 0),
        ch("w:start", 0, Some(1), Sequence, None, 1),
        ch("w:bottom", 0, Some(1), Sequence, None, 2),
        ch("w:end", 0, Some(1), Sequence, None, 3),
        ch("w:insideH", 0, Some(1), Sequence, None, 4),
        ch("w:insideV", 0, Some(1), Sequence, None, 5),
    ]),
    ty("w:CT_TblCellMar", &[
        ch("w:top", 0, Some(1), Sequence, None, 0),
        ch("w:start", 0, Some(1), Sequence, None, 1),
        ch("w:bottom", 0, Some(1), Sequence, None, 2),
        ch("w:end", 0, Some(1), Sequence, None, 3),
    ]),
    ty("w:CT_TblGrid", &[
        ch("w:gridCol", 0, None, Sequence, None, 0),
        ch("w:tblGridChange", 0, Some(1), Sequence, None, 1),
    ]),
    ty("w:CT_TblGridBase", &[
        ch("w:gridCol", 0, None, Sequence, None, 0),
    ]),
    ty("w:CT_TblGridChange", &[
        ch("w:tblGrid", 1, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_TblGridCol", &[]),
    ty("w:CT_TblLayoutType", &[]),
    ty("w:CT_TblLook", &[]),
    ty("w:CT_TblOverlap", &[]),
    ty("w:CT_TblPPr", &[]),
    ty("w:CT_TblPr", &[
        ch("w:tblStyle", 0, Some(1), Sequence, None, 0),
        ch("w:tblpPr", 0, Some(1), Sequence, None, 1),
        ch("w:tblOverlap", 0, Some(1), Sequence, None, 2),
        ch("w:bidiVisual", 0, Some(1), Sequence, None, 3),
        ch("w:tblStyleRowBandSize", 0, Some(1), Sequence, None, 4),
        ch("w:tblStyleColBandSize", 0, Some(1), Sequence, None, 5),
        ch("w:tblW", 0, Some(1), Sequence, None, 6),
        ch("w:jc", 0, Some(1), Sequence, None, 7),
        ch("w:tblCellSpacing", 0, Some(1), Sequence, None, 8),
        ch("w:tblInd", 0, Some(1), Sequence, None, 9),
        ch("w:tblBorders", 0, Some(1), Sequence, None, 10),
        ch("w:shd", 0, Some(1), Sequence, None, 11),
        ch("w:tblLayout", 0, Some(1), Sequence, None, 12),
        ch("w:tblCellMar", 0, Some(1), Sequence, None, 13),
        ch("w:tblLook", 0, Some(1), Sequence, None, 14),
        ch("w:tblCaption", 0, Some(1), Sequence, None, 15),
        ch("w:tblDescription", 0, Some(1), Sequence, None, 16),
        ch("w:tblPrChange", 0, Some(1), Sequence, None, 17),
    ]),
    ty("w:CT_TblPrBase", &[
        ch("w:tblStyle", 0, Some(1), Sequence, None, 0),
        ch("w:tblpPr", 0, Some(1), Sequence, None, 1),
        ch("w:tblOverlap", 0, Some(1), Sequence, None, 2),
        ch("w:bidiVisual", 0, Some(1), Sequence, None, 3),
        ch("w:tblStyleRowBandSize", 0, Some(1), Sequence, None, 4),
        ch("w:tblStyleColBandSize", 0, Some(1), Sequence, None, 5),
        ch("w:tblW", 0, Some(1), Sequence, None, 6),
        ch("w:jc", 0, Some(1), Sequence, None, 7),
        ch("w:tblCellSpacing", 0, Some(1), Sequence, None, 8),
        ch("w:tblInd", 0, Some(1), Sequence, None, 9),
        ch("w:tblBorders", 0, Some(1), Sequence, None, 10),
        ch("w:shd", 0, Some(1), Sequence, None, 11),
        ch("w:tblLayout", 0, Some(1), Sequence, None, 12),
        ch("w:tblCellMar", 0, Some(1), Sequence, None, 13),
        ch("w:tblLook", 0, Some(1), Sequence, None, 14),
        ch("w:tblCaption", 0, Some(1), Sequence, None, 15),
        ch("w:tblDescription", 0, Some(1), Sequence, None, 16),
    ]),
    ty("w:CT_TblPrChange", &[
        ch("w:tblPr", 1, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_TblPrEx", &[
        ch("w:tblW", 0, Some(1), Sequence, None, 0),
        ch("w:jc", 0, Some(1), Sequence, None, 1),
        ch("w:tblCellSpacing", 0, Some(1), Sequence, None, 2),
        ch("w:tblInd", 0, Some(1), Sequence, None, 3),
        ch("w:tblBorders", 0, Some(1), Sequence, None, 4),
        ch("w:shd", 0, Some(1), Sequence, None, 5),
        ch("w:tblLayout", 0, Some(1), Sequence, None, 6),
        ch("w:tblCellMar", 0, Some(1), Sequence, None, 7),
        ch("w:tblLook", 0, Some(1), Sequence, None, 8),
        ch("w:tblPrExChange", 0, Some(1), Sequence, None, 9),
    ]),
    ty("w:CT_TblPrExBase", &[
        ch("w:tblW", 0, Some(1), Sequence, None, 0),
        ch("w:jc", 0, Some(1), Sequence, None, 1),
        ch("w:tblCellSpacing", 0, Some(1), Sequence, None, 2),
        ch("w:tblInd", 0, Some(1), Sequence, None, 3),
        ch("w:tblBorders", 0, Some(1), Sequence, None, 4),
        ch("w:shd", 0, Some(1), Sequence, None, 5),
        ch("w:tblLayout", 0, Some(1), Sequence, None, 6),
        ch("w:tblCellMar", 0, Some(1), Sequence, None, 7),
        ch("w:tblLook", 0, Some(1), Sequence, None, 8),
    ]),
    ty("w:CT_TblPrExChange", &[
        ch("w:tblPrEx", 1, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_TblStylePr", &[
        ch("w:pPr", 0, Some(1), Sequence, None, 0),
        ch("w:rPr", 0, Some(1), Sequence, None, 1),
        ch("w:tblPr", 0, Some(1), Sequence, None, 2),
        ch("w:trPr", 0, Some(1), Sequence, None, 3),
        ch("w:tcPr", 0, Some(1), Sequence, None, 4),
    ]),
    ty("w:CT_TblWidth", &[]),
    ty("w:CT_Tc", &[
        ch("w:tcPr", 0, Some(1), Sequence, None, 0),
        ch("w:customXml", 0, None, Choice, Some("choice3"), 1),
        ch("w:sdt", 0, None, Choice, Some("choice3"), 2),
        ch("w:p", 0, None, Choice, Some("choice3"), 3),
        ch("w:tbl", 0, None, Choice, Some("choice3"), 4),
        ch("w:proofErr", 0, None, Choice, Some("choice4"), 5),
        ch("w:permStart", 0, None, Choice, Some("choice4"), 6),
        ch("w:permEnd", 0, None, Choice, Some("choice4"), 7),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice5"), 8),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice5"), 9),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice5"), 10),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice5"), 11),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice5"), 12),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice5"), 13),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice5"), 14),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice5"), 15),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice5"), 16),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice5"), 17),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice5"), 18),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice5"), 19),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice5"), 20),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice5"), 21),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice5"), 22),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice5"), 23),
        ch("w:ins", 0, None, Choice, Some("choice4"), 24),
        ch("w:del", 0, None, Choice, Some("choice4"), 25),
        ch("w:moveFrom", 0, None, Choice, Some("choice4"), 26),
        ch("w:moveTo", 0, None, Choice, Some("choice4"), 27),
        ch("m:oMathPara", 0, None, Choice, Some("choice6"), 28),
        ch("m:oMath", 0, None, Choice, Some("choice6"), 29),
        ch("w:altChunk", 0, None, Choice, Some("choice1"), 30),
    ]),
    ty("w:CT_TcBorders", &[
        ch("w:top", 0, Some(1), Sequence, None, 0),
        ch("w:start", 0, Some(1), Sequence, None, 1),
        ch("w:bottom", 0, Some(1), Sequence, None, 2),
        ch("w:end", 0, Some(1), Sequence, None, 3),
        ch("w:insideH", 0, Some(1), Sequence, None, 4),
        ch("w:insideV", 0, Some(1), Sequence, None, 5),
        ch("w:tl2br", 0, Some(1), Sequence, None, 6),
        ch("w:tr2bl", 0, Some(1), Sequence, None, 7),
    ]),
    ty("w:CT_TcMar", &[
        ch("w:top", 0, Some(1), Sequence, None, 0),
        ch("w:start", 0, Some(1), Sequence, None, 1),
        ch("w:bottom", 0, Some(1), Sequence, None, 2),
        ch("w:end", 0, Some(1), Sequence, None, 3),
    ]),
    ty("w:CT_TcPr", &[
        ch("w:cnfStyle", 0, Some(1), Sequence, None, 0),
        ch("w:tcW", 0, Some(1), Sequence, None, 1),
        ch("w:gridSpan", 0, Some(1), Sequence, None, 2),
        ch("w:vMerge", 0, Some(1), Sequence, None, 3),
        ch("w:tcBorders", 0, Some(1), Sequence, None, 4),
        ch("w:shd", 0, Some(1), Sequence, None, 5),
        ch("w:noWrap", 0, Some(1), Sequence, None, 6),
        ch("w:tcMar", 0, Some(1), Sequence, None, 7),
        ch("w:textDirection", 0, Some(1), Sequence, None, 8),
        ch("w:tcFitText", 0, Some(1), Sequence, None, 9),
        ch("w:vAlign", 0, Some(1), Sequence, None, 10),
        ch("w:hideMark", 0, Some(1), Sequence, None, 11),
        ch("w:headers", 0, Some(1), Sequence, None, 12),
        ch("w:cellIns", 0, Some(1), Choice, Some("choice1"), 13),
        ch("w:cellDel", 0, Some(1), Choice, Some("choice1"), 14),
        ch("w:cellMerge", 0, Some(1), Choice, Some("choice1"), 15),
        ch("w:tcPrChange", 0, Some(1), Sequence, None, 16),
    ]),
    ty("w:CT_TcPrBase", &[
        ch("w:cnfStyle", 0, Some(1), Sequence, None, 0),
        ch("w:tcW", 0, Some(1), Sequence, None, 1),
        ch("w:gridSpan", 0, Some(1), Sequence, None, 2),
        ch("w:vMerge", 0, Some(1), Sequence, None, 3),
        ch("w:tcBorders", 0, Some(1), Sequence, None, 4),
        ch("w:shd", 0, Some(1), Sequence, None, 5),
        ch("w:noWrap", 0, Some(1), Sequence, None, 6),
        ch("w:tcMar", 0, Some(1), Sequence, None, 7),
        ch("w:textDirection", 0, Some(1), Sequence, None, 8),
        ch("w:tcFitText", 0, Some(1), Sequence, None, 9),
        ch("w:vAlign", 0, Some(1), Sequence, None, 10),
        ch("w:hideMark", 0, Some(1), Sequence, None, 11),
        ch("w:headers", 0, Some(1), Sequence, None, 12),
    ]),
    ty("w:CT_TcPrChange", &[
        ch("w:tcPr", 1, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_TcPrInner", &[
        ch("w:cnfStyle", 0, Some(1), Sequence, None, 0),
        ch("w:tcW", 0, Some(1), Sequence, None, 1),
        ch("w:gridSpan", 0, Some(1), Sequence, None, 2),
        ch("w:vMerge", 0, Some(1), Sequence, None, 3),
        ch("w:tcBorders", 0, Some(1), Sequence, None, 4),
        ch("w:shd", 0, Some(1), Sequence, None, 5),
        ch("w:noWrap", 0, Some(1), Sequence, None, 6),
        ch("w:tcMar", 0, Some(1), Sequence, None, 7),
        ch("w:textDirection", 0, Some(1), Sequence, None, 8),
        ch("w:tcFitText", 0, Some(1), Sequence, None, 9),
        ch("w:vAlign", 0, Some(1), Sequence, None, 10),
        ch("w:hideMark", 0, Some(1), Sequence, None, 11),
        ch("w:headers", 0, Some(1), Sequence, None, 12),
        ch("w:cellIns", 0, Some(1), Choice, Some("choice1"), 13),
        ch("w:cellDel", 0, Some(1), Choice, Some("choice1"), 14),
        ch("w:cellMerge", 0, Some(1), Choice, Some("choice1"), 15),
    ]),
    ty("w:CT_Text", &[]),
    ty("w:CT_TextAlignment", &[]),
    ty("w:CT_TextDirection", &[]),
    ty("w:CT_TextEffect", &[]),
    ty("w:CT_TextScale", &[]),
    ty("w:CT_TextboxTightWrap", &[]),
    ty("w:CT_TopPageBorder", &[]),
    ty("w:CT_TrPr", &[
        ch("w:cnfStyle", 0, None, Choice, Some("choice1"), 0),
        ch("w:divId", 0, None, Choice, Some("choice1"), 1),
        ch("w:gridBefore", 0, None, Choice, Some("choice1"), 2),
        ch("w:gridAfter", 0, None, Choice, Some("choice1"), 3),
        ch("w:wBefore", 0, None, Choice, Some("choice1"), 4),
        ch("w:wAfter", 0, None, Choice, Some("choice1"), 5),
        ch("w:cantSplit", 0, None, Choice, Some("choice1"), 6),
        ch("w:trHeight", 0, None, Choice, Some("choice1"), 7),
        ch("w:tblHeader", 0, None, Choice, Some("choice1"), 8),
        ch("w:tblCellSpacing", 0, None, Choice, Some("choice1"), 9),
        ch("w:jc", 0, None, Choice, Some("choice1"), 10),
        ch("w:hidden", 0, None, Choice, Some("choice1"), 11),
        ch("w:ins", 0, Some(1), Sequence, None, 12),
        ch("w:del", 0, Some(1), Sequence, None, 13),
        ch("w:trPrChange", 0, Some(1), Sequence, None, 14),
    ]),
    ty("w:CT_TrPrBase", &[
        ch("w:cnfStyle", 0, None, Choice, Some("choice1"), 0),
        ch("w:divId", 0, None, Choice, Some("choice1"), 1),
        ch("w:gridBefore", 0, None, Choice, Some("choice1"), 2),
        ch("w:gridAfter", 0, None, Choice, Some("choice1"), 3),
        ch("w:wBefore", 0, None, Choice, Some("choice1"), 4),
        ch("w:wAfter", 0, None, Choice, Some("choice1"), 5),
        ch("w:cantSplit", 0, None, Choice, Some("choice1"), 6),
        ch("w:trHeight", 0, None, Choice, Some("choice1"), 7),
        ch("w:tblHeader", 0, None, Choice, Some("choice1"), 8),
        ch("w:tblCellSpacing", 0, None, Choice, Some("choice1"), 9),
        ch("w:jc", 0, None, Choice, Some("choice1"), 10),
        ch("w:hidden", 0, None, Choice, Some("choice1"), 11),
    ]),
    ty("w:CT_TrPrChange", &[
        ch("w:trPr", 1, Some(1), Sequence, None, 0),
    ]),
    ty("w:CT_TrackChange", &[]),
    ty("w:CT_TrackChangeRange", &[]),
    ty("w:CT_TrackChangesView", &[]),
    ty("w:CT_TwipsMeasure", &[]),
    ty("w:CT_Underline", &[]),
    ty("w:CT_UnsignedDecimalNumber", &[]),
    ty("w:CT_VMerge", &[]),
    ty("w:CT_VerticalAlignRun", &[]),
    ty("w:CT_VerticalJc", &[]),
    ty("w:CT_View", &[]),
    ty("w:CT_WebSettings", &[
        ch("w:frameset", 0, Some(1), Sequence, None, 0),
        ch("w:divs", 0, Some(1), Sequence, None, 1),
        ch("w:encoding", 0, Some(1), Sequence, None, 2),
        ch("w:optimizeForBrowser", 0, Some(1), Sequence, None, 3),
        ch("w:allowPNG", 0, Some(1), Sequence, None, 4),
        ch("w:doNotRelyOnCSS", 0, Some(1), Sequence, None, 5),
        ch("w:doNotSaveAsSingleFile", 0, Some(1), Sequence, None, 6),
        ch("w:doNotOrganizeInFolder", 0, Some(1), Sequence, None, 7),
        ch("w:doNotUseLongFileNames", 0, Some(1), Sequence, None, 8),
        ch("w:pixelsPerInch", 0, Some(1), Sequence, None, 9),
        ch("w:targetScreenSz", 0, Some(1), Sequence, None, 10),
        ch("w:saveSmartTagsAsXml", 0, Some(1), Sequence, None, 11),
    ]),
    ty("w:CT_WriteProtection", &[]),
    ty("w:CT_WritingStyle", &[]),
    ty("w:CT_Zoom", &[]),
    ty("wp:CT_Anchor", &[
        ch("wp:simplePos", 1, Some(1), Sequence, None, 0),
        ch("wp:positionH", 1, Some(1), Sequence, None, 1),
        ch("wp:positionV", 1, Some(1), Sequence, None, 2),
        ch("wp:extent", 1, Some(1), Sequence, None, 3),
        ch("wp:effectExtent", 0, Some(1), Sequence, None, 4),
        ch("wp:wrapNone", 1, Some(1), Choice, Some("choice1"), 5),
        ch("wp:wrapSquare", 1, Some(1), Choice, Some("choice1"), 6),
        ch("wp:wrapTight", 1, Some(1), Choice, Some("choice1"), 7),
        ch("wp:wrapThrough", 1, Some(1), Choice, Some("choice1"), 8),
        ch("wp:wrapTopAndBottom", 1, Some(1), Choice, Some("choice1"), 9),
        ch("wp:docPr", 1, Some(1), Sequence, None, 10),
        ch("wp:cNvGraphicFramePr", 0, Some(1), Sequence, None, 11),
        ch("a:graphic", 1, Some(1), Sequence, None, 12),
    ]),
    ty("wp:CT_EffectExtent", &[]),
    ty("wp:CT_GraphicFrame", &[
        ch("wp:cNvPr", 1, Some(1), Sequence, None, 0),
        ch("wp:cNvFrPr", 1, Some(1), Sequence, None, 1),
        ch("wp:xfrm", 1, Some(1), Sequence, None, 2),
        ch("a:graphic", 1, Some(1), Sequence, None, 3),
        ch("wp:extLst", 0, Some(1), Sequence, None, 4),
    ]),
    ty("wp:CT_Inline", &[
        ch("wp:extent", 1, Some(1), Sequence, None, 0),
        ch("wp:effectExtent", 0, Some(1), Sequence, None, 1),
        ch("wp:docPr", 1, Some(1), Sequence, None, 2),
        ch("wp:cNvGraphicFramePr", 0, Some(1), Sequence, None, 3),
        ch("a:graphic", 1, Some(1), Sequence, None, 4),
    ]),
    ty("wp:CT_LinkedTextboxInformation", &[
        ch("wp:extLst", 0, Some(1), Sequence, None, 0),
    ]),
    ty("wp:CT_PosH", &[
        ch("wp:align", 1, Some(1), Choice, Some("choice1"), 0),
        ch("wp:posOffset", 1, Some(1), Choice, Some("choice1"), 1),
    ]),
    ty("wp:CT_PosV", &[
        ch("wp:align", 1, Some(1), Choice, Some("choice1"), 0),
        ch("wp:posOffset", 1, Some(1), Choice, Some("choice1"), 1),
    ]),
    ty("wp:CT_TextboxInfo", &[
        ch("wp:txbxContent", 1, Some(1), Sequence, None, 0),
        ch("wp:extLst", 0, Some(1), Sequence, None, 1),
    ]),
    ty("wp:CT_TxbxContent", &[]),
    ty("wp:CT_WordprocessingCanvas", &[
        ch("wp:bg", 0, Some(1), Sequence, None, 0),
        ch("wp:whole", 0, Some(1), Sequence, None, 1),
        ch("wp:wsp", 0, None, Choice, Some("choice1"), 2),
        ch("pic:pic", 0, None, Choice, Some("choice1"), 3),
        ch("wp:contentPart", 0, None, Choice, Some("choice1"), 4),
        ch("wp:wgp", 0, None, Choice, Some("choice1"), 5),
        ch("wp:graphicFrame", 0, None, Choice, Some("choice1"), 6),
        ch("wp:extLst", 0, Some(1), Sequence, None, 7),
    ]),
    ty("wp:CT_WordprocessingContentPart", &[
        ch("wp:nvContentPartPr", 0, Some(1), Sequence, None, 0),
        ch("wp:xfrm", 0, Some(1), Sequence, None, 1),
        ch("wp:extLst", 0, Some(1), Sequence, None, 2),
    ]),
    ty("wp:CT_WordprocessingContentPartNonVisual", &[
        ch("wp:cNvPr", 0, Some(1), Sequence, None, 0),
        ch("wp:cNvContentPartPr", 0, Some(1), Sequence, None, 1),
    ]),
    ty("wp:CT_WordprocessingGroup", &[
        ch("wp:cNvPr", 0, Some(1), Sequence, None, 0),
        ch("wp:cNvGrpSpPr", 1, Some(1), Sequence, None, 1),
        ch("wp:grpSpPr", 1, Some(1), Sequence, None, 2),
        ch("wp:wsp", 0, None, Choice, Some("choice1"), 3),
        ch("wp:grpSp", 0, None, Choice, Some("choice1"), 4),
        ch("wp:graphicFrame", 0, None, Choice, Some("choice1"), 5),
        ch("pic:pic", 0, None, Choice, Some("choice1"), 6),
        ch("wp:contentPart", 0, None, Choice, Some("choice1"), 7),
        ch("wp:extLst", 0, Some(1), Sequence, None, 8),
    ]),
    ty("wp:CT_WordprocessingShape", &[
        ch("wp:cNvPr", 0, Some(1), Sequence, None, 0),
        ch("wp:cNvSpPr", 1, Some(1), Choice, Some("choice1"), 1),
        ch("wp:cNvCnPr", 1, Some(1), Choice, Some("choice1"), 2),
        ch("wp:spPr", 1, Some(1), Sequence, None, 3),
        ch("wp:style", 0, Some(1), Sequence, None, 4),
        ch("wp:extLst", 0, Some(1), Sequence, None, 5),
        ch("wp:txbx", 0, Some(1), Choice, Some("choice2"), 6),
        ch("wp:linkedTxbx", 0, Some(1), Choice, Some("choice2"), 7),
        ch("wp:bodyPr", 1, Some(1), Sequence, None, 8),
    ]),
    ty("wp:CT_WrapNone", &[]),
    ty("wp:CT_WrapPath", &[
        ch("wp:start", 1, Some(1), Sequence, None, 0),
        ch("wp:lineTo", 2, None, Sequence, None, 1),
    ]),
    ty("wp:CT_WrapSquare", &[
        ch("wp:effectExtent", 0, Some(1), Sequence, None, 0),
    ]),
    ty("wp:CT_WrapThrough", &[
        ch("wp:wrapPolygon", 1, Some(1), Sequence, None, 0),
    ]),
    ty("wp:CT_WrapTight", &[
        ch("wp:wrapPolygon", 1, Some(1), Sequence, None, 0),
    ]),
    ty("wp:CT_WrapTopBottom", &[
        ch("wp:effectExtent", 0, Some(1), Sequence, None, 0),
    ]),
];

/// Transitional models of the types whose children differ from Strict, sorted by key.
#[rustfmt::skip]
pub(crate) static TRANSITIONAL_TYPES: &[TypeModel] = &[
    ty("c:CT_PrintSettings", &[
        ch("c:headerFooter", 0, Some(1), Sequence, None, 0),
        ch("c:pageMargins", 0, Some(1), Sequence, None, 1),
        ch("c:pageSetup", 0, Some(1), Sequence, None, 2),
        ch("c:legacyDrawingHF", 0, Some(1), Sequence, None, 3),
    ]),
    ty("m:CT_R", &[
        ch("m:rPr", 0, Some(1), Sequence, None, 0),
        ch("w:rPr", 0, Some(1), Sequence, None, 1),
        ch("w:br", 0, None, Choice, Some("choice2"), 2),
        ch("w:t", 0, None, Choice, Some("choice2"), 3),
        ch("w:contentPart", 0, None, Choice, Some("choice2"), 4),
        ch("w:delText", 0, None, Choice, Some("choice2"), 5),
        ch("w:instrText", 0, None, Choice, Some("choice2"), 6),
        ch("w:delInstrText", 0, None, Choice, Some("choice2"), 7),
        ch("w:noBreakHyphen", 0, None, Choice, Some("choice2"), 8),
        ch("w:softHyphen", 0, None, Choice, Some("choice2"), 9),
        ch("w:dayShort", 0, None, Choice, Some("choice2"), 10),
        ch("w:monthShort", 0, None, Choice, Some("choice2"), 11),
        ch("w:yearShort", 0, None, Choice, Some("choice2"), 12),
        ch("w:dayLong", 0, None, Choice, Some("choice2"), 13),
        ch("w:monthLong", 0, None, Choice, Some("choice2"), 14),
        ch("w:yearLong", 0, None, Choice, Some("choice2"), 15),
        ch("w:annotationRef", 0, None, Choice, Some("choice2"), 16),
        ch("w:footnoteRef", 0, None, Choice, Some("choice2"), 17),
        ch("w:endnoteRef", 0, None, Choice, Some("choice2"), 18),
        ch("w:separator", 0, None, Choice, Some("choice2"), 19),
        ch("w:continuationSeparator", 0, None, Choice, Some("choice2"), 20),
        ch("w:sym", 0, None, Choice, Some("choice2"), 21),
        ch("w:pgNum", 0, None, Choice, Some("choice2"), 22),
        ch("w:cr", 0, None, Choice, Some("choice2"), 23),
        ch("w:tab", 0, None, Choice, Some("choice2"), 24),
        ch("w:object", 0, None, Choice, Some("choice2"), 25),
        ch("w:pict", 0, None, Choice, Some("choice2"), 26),
        ch("w:fldChar", 0, None, Choice, Some("choice2"), 27),
        ch("w:ruby", 0, None, Choice, Some("choice2"), 28),
        ch("w:footnoteReference", 0, None, Choice, Some("choice2"), 29),
        ch("w:endnoteReference", 0, None, Choice, Some("choice2"), 30),
        ch("w:commentReference", 0, None, Choice, Some("choice2"), 31),
        ch("w:drawing", 0, None, Choice, Some("choice2"), 32),
        ch("w:ptab", 0, None, Choice, Some("choice2"), 33),
        ch("w:lastRenderedPageBreak", 0, None, Choice, Some("choice2"), 34),
        ch("m:t", 0, None, Choice, Some("choice1"), 35),
    ]),
    ty("w:CT_Background", &[
        ch("<any>", 0, None, Sequence, None, 0),
        ch("<any>", 0, None, Sequence, None, 1),
        ch("w:drawing", 0, Some(1), Sequence, None, 2),
    ]),
    ty("w:CT_Compat", &[
        ch("w:useSingleBorderforContiguousCells", 0, Some(1), Sequence, None, 0),
        ch("w:wpJustification", 0, Some(1), Sequence, None, 1),
        ch("w:noTabHangInd", 0, Some(1), Sequence, None, 2),
        ch("w:noLeading", 0, Some(1), Sequence, None, 3),
        ch("w:spaceForUL", 0, Some(1), Sequence, None, 4),
        ch("w:noColumnBalance", 0, Some(1), Sequence, None, 5),
        ch("w:balanceSingleByteDoubleByteWidth", 0, Some(1), Sequence, None, 6),
        ch("w:noExtraLineSpacing", 0, Some(1), Sequence, None, 7),
        ch("w:doNotLeaveBackslashAlone", 0, Some(1), Sequence, None, 8),
        ch("w:ulTrailSpace", 0, Some(1), Sequence, None, 9),
        ch("w:doNotExpandShiftReturn", 0, Some(1), Sequence, None, 10),
        ch("w:spacingInWholePoints", 0, Some(1), Sequence, None, 11),
        ch("w:lineWrapLikeWord6", 0, Some(1), Sequence, None, 12),
        ch("w:printBodyTextBeforeHeader", 0, Some(1), Sequence, None, 13),
        ch("w:printColBlack", 0, Some(1), Sequence, None, 14),
        ch("w:wpSpaceWidth", 0, Some(1), Sequence, None, 15),
        ch("w:showBreaksInFrames", 0, Some(1), Sequence, None, 16),
        ch("w:subFontBySize", 0, Some(1), Sequence, None, 17),
        ch("w:suppressBottomSpacing", 0, Some(1), Sequence, None, 18),
        ch("w:suppressTopSpacing", 0, Some(1), Sequence, None, 19),
        ch("w:suppressSpacingAtTopOfPage", 0, Some(1), Sequence, None, 20),
        ch("w:suppressTopSpacingWP", 0, Some(1), Sequence, None, 21),
        ch("w:suppressSpBfAfterPgBrk", 0, Some(1), Sequence, None, 22),
        ch("w:swapBordersFacingPages", 0, Some(1), Sequence, None, 23),
        ch("w:convMailMergeEsc", 0, Some(1), Sequence, None, 24),
        ch("w:truncateFontHeightsLikeWP6", 0, Some(1), Sequence, None, 25),
        ch("w:mwSmallCaps", 0, Some(1), Sequence, None, 26),
        ch("w:usePrinterMetrics", 0, Some(1), Sequence, None, 27),
        ch("w:doNotSuppressParagraphBorders", 0, Some(1), Sequence, None, 28),
        ch("w:wrapTrailSpaces", 0, Some(1), Sequence, None, 29),
        ch("w:footnoteLayoutLikeWW8", 0, Some(1), Sequence, None, 30),
        ch("w:shapeLayoutLikeWW8", 0, Some(1), Sequence, None, 31),
        ch("w:alignTablesRowByRow", 0, Some(1), Sequence, None, 32),
        ch("w:forgetLastTabAlignment", 0, Some(1), Sequence, None, 33),
        ch("w:adjustLineHeightInTable", 0, Some(1), Sequence, None, 34),
        ch("w:autoSpaceLikeWord95", 0, Some(1), Sequence, None, 35),
        ch("w:noSpaceRaiseLower", 0, Some(1), Sequence, None, 36),
        ch("w:doNotUseHTMLParagraphAutoSpacing", 0, Some(1), Sequence, None, 37),
        ch("w:layoutRawTableWidth", 0, Some(1), Sequence, None, 38),
        ch("w:layoutTableRowsApart", 0, Some(1), Sequence, None, 39),
        ch("w:useWord97LineBreakRules", 0, Some(1), Sequence, None, 40),
        ch("w:doNotBreakWrappedTables", 0, Some(1), Sequence, None, 41),
        ch("w:doNotSnapToGridInCell", 0, Some(1), Sequence, None, 42),
        ch("w:selectFldWithFirstOrLastChar", 0, Some(1), Sequence, None, 43),
        ch("w:applyBreakingRules", 0, Some(1), Sequence, None, 44),
        ch("w:doNotWrapTextWithPunct", 0, Some(1), Sequence, None, 45),
        ch("w:doNotUseEastAsianBreakRules", 0, Some(1), Sequence, None, 46),
        ch("w:useWord2002TableStyleRules", 0, Some(1), Sequence, None, 47),
        ch("w:growAutofit", 0, Some(1), Sequence, None, 48),
        ch("w:useFELayout", 0, Some(1), Sequence, None, 49),
        ch("w:useNormalStyleForList", 0, Some(1), Sequence, None, 50),
        ch("w:doNotUseIndentAsNumberingTabStop", 0, Some(1), Sequence, None, 51),
        ch("w:useAltKinsokuLineBreakRules", 0, Some(1), Sequence, None, 52),
        ch("w:allowSpaceOfSameStyleInTable", 0, Some(1), Sequence, None, 53),
        ch("w:doNotSuppressIndentation", 0, Some(1), Sequence, None, 54),
        ch("w:doNotAutofitConstrainedTables", 0, Some(1), Sequence, None, 55),
        ch("w:autofitToFirstFixedWidthCell", 0, Some(1), Sequence, None, 56),
        ch("w:underlineTabInNumList", 0, Some(1), Sequence, None, 57),
        ch("w:displayHangulFixedWidth", 0, Some(1), Sequence, None, 58),
        ch("w:splitPgBreakAndParaMark", 0, Some(1), Sequence, None, 59),
        ch("w:doNotVertAlignCellWithSp", 0, Some(1), Sequence, None, 60),
        ch("w:doNotBreakConstrainedForcedTable", 0, Some(1), Sequence, None, 61),
        ch("w:doNotVertAlignInTxbx", 0, Some(1), Sequence, None, 62),
        ch("w:useAnsiKerningPairs", 0, Some(1), Sequence, None, 63),
        ch("w:cachedColBalance", 0, Some(1), Sequence, None, 64),
        ch("w:compatSetting", 0, None, Sequence, None, 65),
    ]),
    ty("w:CT_FldChar", &[
        ch("w:fldData", 0, Some(1), Choice, Some("choice1"), 0),
        ch("w:ffData", 0, Some(1), Choice, Some("choice1"), 1),
        ch("w:numberingChange", 0, Some(1), Choice, Some("choice1"), 2),
    ]),
    ty("w:CT_Lvl", &[
        ch("w:start", 0, Some(1), Sequence, None, 0),
        ch("w:numFmt", 0, Some(1), Sequence, None, 1),
        ch("w:lvlRestart", 0, Some(1), Sequence, None, 2),
        ch("w:pStyle", 0, Some(1), Sequence, None, 3),
        ch("w:isLgl", 0, Some(1), Sequence, None, 4),
        ch("w:suff", 0, Some(1), Sequence, None, 5),
        ch("w:lvlText", 0, Some(1), Sequence, None, 6),
        ch("w:lvlPicBulletId", 0, Some(1), Sequence, None, 7),
        ch("w:legacy", 0, Some(1), Sequence, None, 8),
        ch("w:lvlJc", 0, Some(1), Sequence, None, 9),
        ch("w:pPr", 0, Some(1), Sequence, None, 10),
        ch("w:rPr", 0, Some(1), Sequence, None, 11),
    ]),
    ty("w:CT_NumPicBullet", &[
        ch("w:pict", 1, Some(1), Choice, Some("choice1"), 0),
        ch("w:drawing", 1, Some(1), Choice, Some("choice1"), 1),
    ]),
    ty("w:CT_NumPr", &[
        ch("w:ilvl", 0, Some(1), Sequence, None, 0),
        ch("w:numId", 0, Some(1), Sequence, None, 1),
        ch("w:numberingChange", 0, Some(1), Sequence, None, 2),
        ch("w:ins", 0, Some(1), Sequence, None, 3),
    ]),
    ty("w:CT_Object", &[
        ch("<any>", 0, None, Sequence, None, 0),
        ch("<any>", 0, None, Sequence, None, 1),
        ch("w:drawing", 0, Some(1), Sequence, None, 2),
        ch("w:control", 0, Some(1), Choice, Some("choice1"), 3),
        ch("w:objectLink", 0, Some(1), Choice, Some("choice1"), 4),
        ch("w:objectEmbed", 0, Some(1), Choice, Some("choice1"), 5),
        ch("w:movie", 0, Some(1), Choice, Some("choice1"), 6),
    ]),
    ty("w:CT_R", &[
        ch("w:rPr", 0, Some(1), Sequence, None, 0),
        ch("w:br", 0, None, Choice, Some("choice1"), 1),
        ch("w:t", 0, None, Choice, Some("choice1"), 2),
        ch("w:contentPart", 0, None, Choice, Some("choice1"), 3),
        ch("w:delText", 0, None, Choice, Some("choice1"), 4),
        ch("w:instrText", 0, None, Choice, Some("choice1"), 5),
        ch("w:delInstrText", 0, None, Choice, Some("choice1"), 6),
        ch("w:noBreakHyphen", 0, None, Choice, Some("choice1"), 7),
        ch("w:softHyphen", 0, None, Choice, Some("choice1"), 8),
        ch("w:dayShort", 0, None, Choice, Some("choice1"), 9),
        ch("w:monthShort", 0, None, Choice, Some("choice1"), 10),
        ch("w:yearShort", 0, None, Choice, Some("choice1"), 11),
        ch("w:dayLong", 0, None, Choice, Some("choice1"), 12),
        ch("w:monthLong", 0, None, Choice, Some("choice1"), 13),
        ch("w:yearLong", 0, None, Choice, Some("choice1"), 14),
        ch("w:annotationRef", 0, None, Choice, Some("choice1"), 15),
        ch("w:footnoteRef", 0, None, Choice, Some("choice1"), 16),
        ch("w:endnoteRef", 0, None, Choice, Some("choice1"), 17),
        ch("w:separator", 0, None, Choice, Some("choice1"), 18),
        ch("w:continuationSeparator", 0, None, Choice, Some("choice1"), 19),
        ch("w:sym", 0, None, Choice, Some("choice1"), 20),
        ch("w:pgNum", 0, None, Choice, Some("choice1"), 21),
        ch("w:cr", 0, None, Choice, Some("choice1"), 22),
        ch("w:tab", 0, None, Choice, Some("choice1"), 23),
        ch("w:object", 0, None, Choice, Some("choice1"), 24),
        ch("w:pict", 0, None, Choice, Some("choice1"), 25),
        ch("w:fldChar", 0, None, Choice, Some("choice1"), 26),
        ch("w:ruby", 0, None, Choice, Some("choice1"), 27),
        ch("w:footnoteReference", 0, None, Choice, Some("choice1"), 28),
        ch("w:endnoteReference", 0, None, Choice, Some("choice1"), 29),
        ch("w:commentReference", 0, None, Choice, Some("choice1"), 30),
        ch("w:drawing", 0, None, Choice, Some("choice1"), 31),
        ch("w:ptab", 0, None, Choice, Some("choice1"), 32),
        ch("w:lastRenderedPageBreak", 0, None, Choice, Some("choice1"), 33),
    ]),
    ty("w:CT_Settings", &[
        ch("w:writeProtection", 0, Some(1), Sequence, None, 0),
        ch("w:view", 0, Some(1), Sequence, None, 1),
        ch("w:zoom", 0, Some(1), Sequence, None, 2),
        ch("w:removePersonalInformation", 0, Some(1), Sequence, None, 3),
        ch("w:removeDateAndTime", 0, Some(1), Sequence, None, 4),
        ch("w:doNotDisplayPageBoundaries", 0, Some(1), Sequence, None, 5),
        ch("w:displayBackgroundShape", 0, Some(1), Sequence, None, 6),
        ch("w:printPostScriptOverText", 0, Some(1), Sequence, None, 7),
        ch("w:printFractionalCharacterWidth", 0, Some(1), Sequence, None, 8),
        ch("w:printFormsData", 0, Some(1), Sequence, None, 9),
        ch("w:embedTrueTypeFonts", 0, Some(1), Sequence, None, 10),
        ch("w:embedSystemFonts", 0, Some(1), Sequence, None, 11),
        ch("w:saveSubsetFonts", 0, Some(1), Sequence, None, 12),
        ch("w:saveFormsData", 0, Some(1), Sequence, None, 13),
        ch("w:mirrorMargins", 0, Some(1), Sequence, None, 14),
        ch("w:alignBordersAndEdges", 0, Some(1), Sequence, None, 15),
        ch("w:bordersDoNotSurroundHeader", 0, Some(1), Sequence, None, 16),
        ch("w:bordersDoNotSurroundFooter", 0, Some(1), Sequence, None, 17),
        ch("w:gutterAtTop", 0, Some(1), Sequence, None, 18),
        ch("w:hideSpellingErrors", 0, Some(1), Sequence, None, 19),
        ch("w:hideGrammaticalErrors", 0, Some(1), Sequence, None, 20),
        ch("w:activeWritingStyle", 0, None, Sequence, None, 21),
        ch("w:proofState", 0, Some(1), Sequence, None, 22),
        ch("w:formsDesign", 0, Some(1), Sequence, None, 23),
        ch("w:attachedTemplate", 0, Some(1), Sequence, None, 24),
        ch("w:linkStyles", 0, Some(1), Sequence, None, 25),
        ch("w:stylePaneFormatFilter", 0, Some(1), Sequence, None, 26),
        ch("w:stylePaneSortMethod", 0, Some(1), Sequence, None, 27),
        ch("w:documentType", 0, Some(1), Sequence, None, 28),
        ch("w:mailMerge", 0, Some(1), Sequence, None, 29),
        ch("w:revisionView", 0, Some(1), Sequence, None, 30),
        ch("w:trackRevisions", 0, Some(1), Sequence, None, 31),
        ch("w:doNotTrackMoves", 0, Some(1), Sequence, None, 32),
        ch("w:doNotTrackFormatting", 0, Some(1), Sequence, None, 33),
        ch("w:documentProtection", 0, Some(1), Sequence, None, 34),
        ch("w:autoFormatOverride", 0, Some(1), Sequence, None, 35),
        ch("w:styleLockTheme", 0, Some(1), Sequence, None, 36),
        ch("w:styleLockQFSet", 0, Some(1), Sequence, None, 37),
        ch("w:defaultTabStop", 0, Some(1), Sequence, None, 38),
        ch("w:autoHyphenation", 0, Some(1), Sequence, None, 39),
        ch("w:consecutiveHyphenLimit", 0, Some(1), Sequence, None, 40),
        ch("w:hyphenationZone", 0, Some(1), Sequence, None, 41),
        ch("w:doNotHyphenateCaps", 0, Some(1), Sequence, None, 42),
        ch("w:showEnvelope", 0, Some(1), Sequence, None, 43),
        ch("w:summaryLength", 0, Some(1), Sequence, None, 44),
        ch("w:clickAndTypeStyle", 0, Some(1), Sequence, None, 45),
        ch("w:defaultTableStyle", 0, Some(1), Sequence, None, 46),
        ch("w:evenAndOddHeaders", 0, Some(1), Sequence, None, 47),
        ch("w:bookFoldRevPrinting", 0, Some(1), Sequence, None, 48),
        ch("w:bookFoldPrinting", 0, Some(1), Sequence, None, 49),
        ch("w:bookFoldPrintingSheets", 0, Some(1), Sequence, None, 50),
        ch("w:drawingGridHorizontalSpacing", 0, Some(1), Sequence, None, 51),
        ch("w:drawingGridVerticalSpacing", 0, Some(1), Sequence, None, 52),
        ch("w:displayHorizontalDrawingGridEvery", 0, Some(1), Sequence, None, 53),
        ch("w:displayVerticalDrawingGridEvery", 0, Some(1), Sequence, None, 54),
        ch("w:doNotUseMarginsForDrawingGridOrigin", 0, Some(1), Sequence, None, 55),
        ch("w:drawingGridHorizontalOrigin", 0, Some(1), Sequence, None, 56),
        ch("w:drawingGridVerticalOrigin", 0, Some(1), Sequence, None, 57),
        ch("w:doNotShadeFormData", 0, Some(1), Sequence, None, 58),
        ch("w:noPunctuationKerning", 0, Some(1), Sequence, None, 59),
        ch("w:characterSpacingControl", 0, Some(1), Sequence, None, 60),
        ch("w:printTwoOnOne", 0, Some(1), Sequence, None, 61),
        ch("w:strictFirstAndLastChars", 0, Some(1), Sequence, None, 62),
        ch("w:noLineBreaksAfter", 0, Some(1), Sequence, None, 63),
        ch("w:noLineBreaksBefore", 0, Some(1), Sequence, None, 64),
        ch("w:savePreviewPicture", 0, Some(1), Sequence, None, 65),
        ch("w:doNotValidateAgainstSchema", 0, Some(1), Sequence, None, 66),
        ch("w:saveInvalidXml", 0, Some(1), Sequence, None, 67),
        ch("w:ignoreMixedContent", 0, Some(1), Sequence, None, 68),
        ch("w:alwaysShowPlaceholderText", 0, Some(1), Sequence, None, 69),
        ch("w:doNotDemarcateInvalidXml", 0, Some(1), Sequence, None, 70),
        ch("w:saveXmlDataOnly", 0, Some(1), Sequence, None, 71),
        ch("w:useXSLTWhenSaving", 0, Some(1), Sequence, None, 72),
        ch("w:saveThroughXslt", 0, Some(1), Sequence, None, 73),
        ch("w:showXMLTags", 0, Some(1), Sequence, None, 74),
        ch("w:alwaysMergeEmptyNamespace", 0, Some(1), Sequence, None, 75),
        ch("w:updateFields", 0, Some(1), Sequence, None, 76),
        ch("w:hdrShapeDefaults", 0, Some(1), Sequence, None, 77),
        ch("w:footnotePr", 0, Some(1), Sequence, None, 78),
        ch("w:endnotePr", 0, Some(1), Sequence, None, 79),
        ch("w:compat", 0, Some(1), Sequence, None, 80),
        ch("w:docVars", 0, Some(1), Sequence, None, 81),
        ch("w:rsids", 0, Some(1), Sequence, None, 82),
        ch("m:mathPr", 0, Some(1), Sequence, None, 83),
        ch("w:attachedSchema", 0, None, Sequence, None, 84),
        ch("w:themeFontLang", 0, Some(1), Sequence, None, 85),
        ch("w:clrSchemeMapping", 0, Some(1), Sequence, None, 86),
        ch("w:doNotIncludeSubdocsInStats", 0, Some(1), Sequence, None, 87),
        ch("w:doNotAutoCompressPictures", 0, Some(1), Sequence, None, 88),
        ch("w:forceUpgrade", 0, Some(1), Sequence, None, 89),
        ch("w:captions", 0, Some(1), Sequence, None, 90),
        ch("w:readModeInkLockDown", 0, Some(1), Sequence, None, 91),
        ch("w:smartTagType", 0, None, Sequence, None, 92),
        ch("sl:schemaLibrary", 0, Some(1), Sequence, None, 93),
        ch("w:shapeDefaults", 0, Some(1), Sequence, None, 94),
        ch("w:doNotEmbedSmartTags", 0, Some(1), Sequence, None, 95),
        ch("w:decimalSymbol", 0, Some(1), Sequence, None, 96),
        ch("w:listSeparator", 0, Some(1), Sequence, None, 97),
    ]),
    ty("w:CT_SimpleField", &[
        ch("w:fldData", 0, Some(1), Sequence, None, 0),
        ch("w:customXml", 0, None, Choice, Some("choice2"), 1),
        ch("w:smartTag", 0, None, Choice, Some("choice2"), 2),
        ch("w:sdt", 0, None, Choice, Some("choice2"), 3),
        ch("w:dir", 0, None, Choice, Some("choice2"), 4),
        ch("w:bdo", 0, None, Choice, Some("choice2"), 5),
        ch("w:r", 0, None, Choice, Some("choice2"), 6),
        ch("w:proofErr", 0, None, Choice, Some("choice3"), 7),
        ch("w:permStart", 0, None, Choice, Some("choice3"), 8),
        ch("w:permEnd", 0, None, Choice, Some("choice3"), 9),
        ch("w:bookmarkStart", 0, None, Choice, Some("choice4"), 10),
        ch("w:bookmarkEnd", 0, None, Choice, Some("choice4"), 11),
        ch("w:moveFromRangeStart", 0, None, Choice, Some("choice4"), 12),
        ch("w:moveFromRangeEnd", 0, None, Choice, Some("choice4"), 13),
        ch("w:moveToRangeStart", 0, None, Choice, Some("choice4"), 14),
        ch("w:moveToRangeEnd", 0, None, Choice, Some("choice4"), 15),
        ch("w:commentRangeStart", 0, None, Choice, Some("choice4"), 16),
        ch("w:commentRangeEnd", 0, None, Choice, Some("choice4"), 17),
        ch("w:customXmlInsRangeStart", 0, None, Choice, Some("choice4"), 18),
        ch("w:customXmlInsRangeEnd", 0, None, Choice, Some("choice4"), 19),
        ch("w:customXmlDelRangeStart", 0, None, Choice, Some("choice4"), 20),
        ch("w:customXmlDelRangeEnd", 0, None, Choice, Some("choice4"), 21),
        ch("w:customXmlMoveFromRangeStart", 0, None, Choice, Some("choice4"), 22),
        ch("w:customXmlMoveFromRangeEnd", 0, None, Choice, Some("choice4"), 23),
        ch("w:customXmlMoveToRangeStart", 0, None, Choice, Some("choice4"), 24),
        ch("w:customXmlMoveToRangeEnd", 0, None, Choice, Some("choice4"), 25),
        ch("w:ins", 0, None, Choice, Some("choice3"), 26),
        ch("w:del", 0, None, Choice, Some("choice3"), 27),
        ch("w:moveFrom", 0, None, Choice, Some("choice3"), 28),
        ch("w:moveTo", 0, None, Choice, Some("choice3"), 29),
        ch("m:oMathPara", 0, None, Choice, Some("choice5"), 30),
        ch("m:oMath", 0, None, Choice, Some("choice5"), 31),
        ch("w:fldSimple", 0, None, Choice, Some("choice1"), 32),
        ch("w:hyperlink", 0, None, Choice, Some("choice1"), 33),
        ch("w:subDoc", 0, None, Choice, Some("choice1"), 34),
    ]),
    ty("w:CT_TblBorders", &[
        ch("w:top", 0, Some(1), Sequence, None, 0),
        ch("w:start", 0, Some(1), Sequence, None, 1),
        ch("w:left", 0, Some(1), Sequence, None, 2),
        ch("w:bottom", 0, Some(1), Sequence, None, 3),
        ch("w:end", 0, Some(1), Sequence, None, 4),
        ch("w:right", 0, Some(1), Sequence, None, 5),
        ch("w:insideH", 0, Some(1), Sequence, None, 6),
        ch("w:insideV", 0, Some(1), Sequence, None, 7),
    ]),
    ty("w:CT_TblCellMar", &[
        ch("w:top", 0, Some(1), Sequence, None, 0),
        ch("w:start", 0, Some(1), Sequence, None, 1),
        ch("w:left", 0, Some(1), Sequence, None, 2),
        ch("w:bottom", 0, Some(1), Sequence, None, 3),
        ch("w:end", 0, Some(1), Sequence, None, 4),
        ch("w:right", 0, Some(1), Sequence, None, 5),
    ]),
    ty("w:CT_TcBorders", &[
        ch("w:top", 0, Some(1), Sequence, None, 0),
        ch("w:start", 0, Some(1), Sequence, None, 1),
        ch("w:left", 0, Some(1), Sequence, None, 2),
        ch("w:bottom", 0, Some(1), Sequence, None, 3),
        ch("w:end", 0, Some(1), Sequence, None, 4),
        ch("w:right", 0, Some(1), Sequence, None, 5),
        ch("w:insideH", 0, Some(1), Sequence, None, 6),
        ch("w:insideV", 0, Some(1), Sequence, None, 7),
        ch("w:tl2br", 0, Some(1), Sequence, None, 8),
        ch("w:tr2bl", 0, Some(1), Sequence, None, 9),
    ]),
    ty("w:CT_TcMar", &[
        ch("w:top", 0, Some(1), Sequence, None, 0),
        ch("w:start", 0, Some(1), Sequence, None, 1),
        ch("w:left", 0, Some(1), Sequence, None, 2),
        ch("w:bottom", 0, Some(1), Sequence, None, 3),
        ch("w:end", 0, Some(1), Sequence, None, 4),
        ch("w:right", 0, Some(1), Sequence, None, 5),
    ]),
    ty("w:CT_TcPr", &[
        ch("w:cnfStyle", 0, Some(1), Sequence, None, 0),
        ch("w:tcW", 0, Some(1), Sequence, None, 1),
        ch("w:gridSpan", 0, Some(1), Sequence, None, 2),
        ch("w:hMerge", 0, Some(1), Sequence, None, 3),
        ch("w:vMerge", 0, Some(1), Sequence, None, 4),
        ch("w:tcBorders", 0, Some(1), Sequence, None, 5),
        ch("w:shd", 0, Some(1), Sequence, None, 6),
        ch("w:noWrap", 0, Some(1), Sequence, None, 7),
        ch("w:tcMar", 0, Some(1), Sequence, None, 8),
        ch("w:textDirection", 0, Some(1), Sequence, None, 9),
        ch("w:tcFitText", 0, Some(1), Sequence, None, 10),
        ch("w:vAlign", 0, Some(1), Sequence, None, 11),
        ch("w:hideMark", 0, Some(1), Sequence, None, 12),
        ch("w:headers", 0, Some(1), Sequence, None, 13),
        ch("w:cellIns", 0, Some(1), Choice, Some("choice1"), 14),
        ch("w:cellDel", 0, Some(1), Choice, Some("choice1"), 15),
        ch("w:cellMerge", 0, Some(1), Choice, Some("choice1"), 16),
        ch("w:tcPrChange", 0, Some(1), Sequence, None, 17),
    ]),
    ty("w:CT_TcPrBase", &[
        ch("w:cnfStyle", 0, Some(1), Sequence, None, 0),
        ch("w:tcW", 0, Some(1), Sequence, None, 1),
        ch("w:gridSpan", 0, Some(1), Sequence, None, 2),
        ch("w:hMerge", 0, Some(1), Sequence, None, 3),
        ch("w:vMerge", 0, Some(1), Sequence, None, 4),
        ch("w:tcBorders", 0, Some(1), Sequence, None, 5),
        ch("w:shd", 0, Some(1), Sequence, None, 6),
        ch("w:noWrap", 0, Some(1), Sequence, None, 7),
        ch("w:tcMar", 0, Some(1), Sequence, None, 8),
        ch("w:textDirection", 0, Some(1), Sequence, None, 9),
        ch("w:tcFitText", 0, Some(1), Sequence, None, 10),
        ch("w:vAlign", 0, Some(1), Sequence, None, 11),
        ch("w:hideMark", 0, Some(1), Sequence, None, 12),
        ch("w:headers", 0, Some(1), Sequence, None, 13),
    ]),
    ty("w:CT_TcPrInner", &[
        ch("w:cnfStyle", 0, Some(1), Sequence, None, 0),
        ch("w:tcW", 0, Some(1), Sequence, None, 1),
        ch("w:gridSpan", 0, Some(1), Sequence, None, 2),
        ch("w:hMerge", 0, Some(1), Sequence, None, 3),
        ch("w:vMerge", 0, Some(1), Sequence, None, 4),
        ch("w:tcBorders", 0, Some(1), Sequence, None, 5),
        ch("w:shd", 0, Some(1), Sequence, None, 6),
        ch("w:noWrap", 0, Some(1), Sequence, None, 7),
        ch("w:tcMar", 0, Some(1), Sequence, None, 8),
        ch("w:textDirection", 0, Some(1), Sequence, None, 9),
        ch("w:tcFitText", 0, Some(1), Sequence, None, 10),
        ch("w:vAlign", 0, Some(1), Sequence, None, 11),
        ch("w:hideMark", 0, Some(1), Sequence, None, 12),
        ch("w:headers", 0, Some(1), Sequence, None, 13),
        ch("w:cellIns", 0, Some(1), Choice, Some("choice1"), 14),
        ch("w:cellDel", 0, Some(1), Choice, Some("choice1"), 15),
        ch("w:cellMerge", 0, Some(1), Choice, Some("choice1"), 16),
    ]),
    ty("w:CT_WebSettings", &[
        ch("w:frameset", 0, Some(1), Sequence, None, 0),
        ch("w:divs", 0, Some(1), Sequence, None, 1),
        ch("w:encoding", 0, Some(1), Sequence, None, 2),
        ch("w:optimizeForBrowser", 0, Some(1), Sequence, None, 3),
        ch("w:relyOnVML", 0, Some(1), Sequence, None, 4),
        ch("w:allowPNG", 0, Some(1), Sequence, None, 5),
        ch("w:doNotRelyOnCSS", 0, Some(1), Sequence, None, 6),
        ch("w:doNotSaveAsSingleFile", 0, Some(1), Sequence, None, 7),
        ch("w:doNotOrganizeInFolder", 0, Some(1), Sequence, None, 8),
        ch("w:doNotUseLongFileNames", 0, Some(1), Sequence, None, 9),
        ch("w:pixelsPerInch", 0, Some(1), Sequence, None, 10),
        ch("w:targetScreenSz", 0, Some(1), Sequence, None, 11),
        ch("w:saveSmartTagsAsXml", 0, Some(1), Sequence, None, 12),
    ]),
];

/// Every Strict element row of the WML and DML profiles, sorted by name.
#[rustfmt::skip]
pub(crate) static ELEMENTS: &[ElementModel] = &[
    el("a:accent1", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_ColorScheme"]),
    ]),
    el("a:accent2", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_ColorScheme"]),
    ]),
    el("a:accent3", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_ColorScheme"]),
    ]),
    el("a:accent4", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_ColorScheme"]),
    ]),
    el("a:accent5", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_ColorScheme"]),
    ]),
    el("a:accent6", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_ColorScheme"]),
    ]),
    el("a:ahLst", "a:CT_AdjustHandleList", &[
        va("a:CT_AdjustHandleList", &["a:CT_CustomGeometry2D"]),
    ]),
    el("a:ahPolar", "a:CT_PolarAdjustHandle", &[
        va("a:CT_PolarAdjustHandle", &["a:CT_AdjustHandleList"]),
    ]),
    el("a:ahXY", "a:CT_XYAdjustHandle", &[
        va("a:CT_XYAdjustHandle", &["a:CT_AdjustHandleList"]),
    ]),
    el("a:alpha", "a:CT_PositiveFixedPercentage", &[
        va("a:CT_PositiveFixedPercentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:alphaBiLevel", "a:CT_AlphaBiLevelEffect", &[
        va("a:CT_AlphaBiLevelEffect", &["a:CT_Blip"]),
    ]),
    el("a:alphaCeiling", "a:CT_AlphaCeilingEffect", &[
        va("a:CT_AlphaCeilingEffect", &["a:CT_Blip"]),
    ]),
    el("a:alphaFloor", "a:CT_AlphaFloorEffect", &[
        va("a:CT_AlphaFloorEffect", &["a:CT_Blip"]),
    ]),
    el("a:alphaInv", "a:CT_AlphaInverseEffect", &[
        va("a:CT_AlphaInverseEffect", &["a:CT_Blip"]),
    ]),
    el("a:alphaMod", "a:CT_AlphaModulateEffect", &[
        va("a:CT_AlphaModulateEffect", &["a:CT_Blip"]),
        va("a:CT_PositivePercentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:alphaModFix", "a:CT_AlphaModulateFixedEffect", &[
        va("a:CT_AlphaModulateFixedEffect", &["a:CT_Blip"]),
    ]),
    el("a:alphaOff", "a:CT_FixedPercentage", &[
        va("a:CT_FixedPercentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:alphaOutset", "a:CT_AlphaOutsetEffect", &[
        va("a:CT_AlphaOutsetEffect", &[]),
    ]),
    el("a:alphaRepl", "a:CT_AlphaReplaceEffect", &[
        va("a:CT_AlphaReplaceEffect", &["a:CT_Blip"]),
    ]),
    el("a:anchor", "a:CT_Point3D", &[
        va("a:CT_Point3D", &["a:CT_Backdrop"]),
    ]),
    el("a:arcTo", "a:CT_Path2DArcTo", &[
        va("a:CT_Path2DArcTo", &["a:CT_Path2D"]),
    ]),
    el("a:audioCd", "a:CT_AudioCD", &[
        va("a:CT_AudioCD", &[]),
    ]),
    el("a:audioFile", "a:CT_AudioFile", &[
        va("a:CT_AudioFile", &[]),
    ]),
    el("a:avLst", "a:CT_GeomGuideList", &[
        va("a:CT_GeomGuideList", &["a:CT_CustomGeometry2D", "a:CT_PresetGeometry2D", "a:CT_PresetTextShape"]),
    ]),
    el("a:backdrop", "a:CT_Backdrop", &[
        va("a:CT_Backdrop", &["a:CT_Scene3D"]),
    ]),
    el("a:band1H", "a:CT_TablePartStyle", &[
        va("a:CT_TablePartStyle", &["a:CT_TableStyle"]),
    ]),
    el("a:band1V", "a:CT_TablePartStyle", &[
        va("a:CT_TablePartStyle", &["a:CT_TableStyle"]),
    ]),
    el("a:band2H", "a:CT_TablePartStyle", &[
        va("a:CT_TablePartStyle", &["a:CT_TableStyle"]),
    ]),
    el("a:band2V", "a:CT_TablePartStyle", &[
        va("a:CT_TablePartStyle", &["a:CT_TableStyle"]),
    ]),
    el("a:bevel", "a:CT_LineJoinBevel", &[
        va("a:CT_Bevel", &["a:CT_Cell3D"]),
        va("a:CT_LineJoinBevel", &["a:CT_LineProperties"]),
    ]),
    el("a:bevelB", "a:CT_Bevel", &[
        va("a:CT_Bevel", &["a:CT_Shape3D"]),
    ]),
    el("a:bevelT", "a:CT_Bevel", &[
        va("a:CT_Bevel", &["a:CT_Shape3D"]),
    ]),
    el("a:bgClr", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_PatternFillProperties"]),
    ]),
    el("a:bgFillStyleLst", "a:CT_BackgroundFillStyleList", &[
        va("a:CT_BackgroundFillStyleList", &["a:CT_StyleMatrix"]),
    ]),
    el("a:biLevel", "a:CT_BiLevelEffect", &[
        va("a:CT_BiLevelEffect", &["a:CT_Blip"]),
    ]),
    el("a:bldChart", "a:CT_AnimationChartBuildProperties", &[
        va("a:CT_AnimationChartBuildProperties", &["a:CT_AnimationGraphicalObjectBuildProperties"]),
    ]),
    el("a:bldDgm", "a:CT_AnimationDgmBuildProperties", &[
        va("a:CT_AnimationDgmBuildProperties", &["a:CT_AnimationGraphicalObjectBuildProperties"]),
    ]),
    el("a:blend", "a:CT_BlendEffect", &[
        va("a:CT_BlendEffect", &[]),
    ]),
    el("a:blip", "a:CT_Blip", &[
        va("a:CT_Blip", &["a:CT_BlipFillProperties", "a:CT_TextBlipBullet"]),
    ]),
    el("a:blipFill", "a:CT_BlipFillProperties", &[
        va("a:CT_BlipFillProperties", &["a:CT_BackgroundFillStyleList", "a:CT_BackgroundFormatting", "a:CT_FillEffect", "a:CT_FillOverlayEffect", "a:CT_FillProperties", "a:CT_FillStyleList", "a:CT_GroupShapeProperties", "a:CT_GvmlPicture", "a:CT_ShapeProperties", "a:CT_TableCellProperties", "a:CT_TableProperties", "a:CT_TextCharacterProperties"]),
    ]),
    el("a:blue", "a:CT_Percentage", &[
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:blueMod", "a:CT_Percentage", &[
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:blueOff", "a:CT_Percentage", &[
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:blur", "a:CT_BlurEffect", &[
        va("a:CT_BlurEffect", &["a:CT_Blip", "a:CT_EffectList"]),
    ]),
    el("a:bodyPr", "a:CT_TextBodyProperties", &[
        va("a:CT_TextBodyProperties", &["a:CT_DefaultShapeDefinition", "a:CT_TextBody"]),
    ]),
    el("a:bottom", "a:CT_ThemeableLineStyle", &[
        va("a:CT_ThemeableLineStyle", &["a:CT_TableCellBorderStyle"]),
    ]),
    el("a:br", "a:CT_TextLineBreak", &[
        va("a:CT_TextLineBreak", &["a:CT_TextParagraph"]),
    ]),
    el("a:buAutoNum", "a:CT_TextAutonumberBullet", &[
        va("a:CT_TextAutonumberBullet", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:buBlip", "a:CT_TextBlipBullet", &[
        va("a:CT_TextBlipBullet", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:buChar", "a:CT_TextCharBullet", &[
        va("a:CT_TextCharBullet", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:buClr", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:buClrTx", "a:CT_TextBulletColorFollowText", &[
        va("a:CT_TextBulletColorFollowText", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:buFont", "a:CT_TextFont", &[
        va("a:CT_TextFont", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:buFontTx", "a:CT_TextBulletTypefaceFollowText", &[
        va("a:CT_TextBulletTypefaceFollowText", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:buNone", "a:CT_TextNoBullet", &[
        va("a:CT_TextNoBullet", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:buSzPct", "a:CT_TextBulletSizePercent", &[
        va("a:CT_TextBulletSizePercent", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:buSzPts", "a:CT_TextBulletSizePoint", &[
        va("a:CT_TextBulletSizePoint", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:buSzTx", "a:CT_TextBulletSizeFollowText", &[
        va("a:CT_TextBulletSizeFollowText", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:cNvCxnSpPr", "a:CT_NonVisualConnectorProperties", &[
        va("a:CT_NonVisualConnectorProperties", &["a:CT_GvmlConnectorNonVisual"]),
    ]),
    el("a:cNvGraphicFramePr", "a:CT_NonVisualGraphicFrameProperties", &[
        va("a:CT_NonVisualGraphicFrameProperties", &["a:CT_GvmlGraphicFrameNonVisual"]),
    ]),
    el("a:cNvGrpSpPr", "a:CT_NonVisualGroupDrawingShapeProps", &[
        va("a:CT_NonVisualGroupDrawingShapeProps", &["a:CT_GvmlGroupShapeNonVisual"]),
    ]),
    el("a:cNvPicPr", "a:CT_NonVisualPictureProperties", &[
        va("a:CT_NonVisualPictureProperties", &["a:CT_GvmlPictureNonVisual"]),
    ]),
    el("a:cNvPr", "a:CT_NonVisualDrawingProps", &[
        va("a:CT_NonVisualDrawingProps", &["a:CT_GvmlConnectorNonVisual", "a:CT_GvmlGraphicFrameNonVisual", "a:CT_GvmlGroupShapeNonVisual", "a:CT_GvmlPictureNonVisual", "a:CT_GvmlShapeNonVisual"]),
    ]),
    el("a:cNvSpPr", "a:CT_NonVisualDrawingShapeProps", &[
        va("a:CT_NonVisualDrawingShapeProps", &["a:CT_GvmlShapeNonVisual"]),
    ]),
    el("a:camera", "a:CT_Camera", &[
        va("a:CT_Camera", &["a:CT_Scene3D"]),
    ]),
    el("a:cell3D", "a:CT_Cell3D", &[
        va("a:CT_Cell3D", &["a:CT_TableCellProperties", "a:CT_TableStyleCellStyle"]),
    ]),
    el("a:chExt", "a:CT_PositiveSize2D", &[
        va("a:CT_PositiveSize2D", &["a:CT_GroupTransform2D"]),
    ]),
    el("a:chOff", "a:CT_Point2D", &[
        va("a:CT_Point2D", &["a:CT_GroupTransform2D"]),
    ]),
    el("a:chart", "a:CT_AnimationChartElement", &[
        va("a:CT_AnimationChartElement", &["a:CT_AnimationElementChoice"]),
    ]),
    el("a:close", "a:CT_Path2DClose", &[
        va("a:CT_Path2DClose", &["a:CT_Path2D"]),
    ]),
    el("a:clrChange", "a:CT_ColorChangeEffect", &[
        va("a:CT_ColorChangeEffect", &["a:CT_Blip"]),
    ]),
    el("a:clrFrom", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_ColorChangeEffect"]),
    ]),
    el("a:clrMap", "a:CT_ColorMapping", &[
        va("a:CT_ColorMapping", &["a:CT_ClipboardStyleSheet", "a:CT_ColorSchemeAndMapping"]),
    ]),
    el("a:clrRepl", "a:CT_ColorReplaceEffect", &[
        va("a:CT_ColorReplaceEffect", &["a:CT_Blip"]),
    ]),
    el("a:clrScheme", "a:CT_ColorScheme", &[
        va("a:CT_ColorScheme", &["a:CT_BaseStyles", "a:CT_BaseStylesOverride", "a:CT_ColorSchemeAndMapping"]),
    ]),
    el("a:clrTo", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_ColorChangeEffect"]),
    ]),
    el("a:comp", "a:CT_ComplementTransform", &[
        va("a:CT_ComplementTransform", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:cont", "a:CT_EffectContainer", &[
        va("a:CT_EffectContainer", &["a:CT_AlphaModulateEffect", "a:CT_BlendEffect"]),
    ]),
    el("a:contourClr", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_Shape3D"]),
    ]),
    el("a:cpLocks", "a:CT_ContentPartLocking", &[
        va("a:CT_ContentPartLocking", &["a:CT_NonVisualContentPartProperties"]),
    ]),
    el("a:cs", "a:CT_TextFont", &[
        va("a:CT_TextFont", &["a:CT_FontCollection", "a:CT_TextCharacterProperties"]),
    ]),
    el("a:cubicBezTo", "a:CT_Path2DCubicBezierTo", &[
        va("a:CT_Path2DCubicBezierTo", &["a:CT_Path2D"]),
    ]),
    el("a:custClr", "a:CT_CustomColor", &[
        va("a:CT_CustomColor", &["a:CT_CustomColorList"]),
    ]),
    el("a:custClrLst", "a:CT_CustomColorList", &[
        va("a:CT_CustomColorList", &["a:CT_OfficeStyleSheet"]),
    ]),
    el("a:custDash", "a:CT_DashStopList", &[
        va("a:CT_DashStopList", &["a:CT_LineProperties"]),
    ]),
    el("a:custGeom", "a:CT_CustomGeometry2D", &[
        va("a:CT_CustomGeometry2D", &["a:CT_ShapeProperties"]),
    ]),
    el("a:cxn", "a:CT_ConnectionSite", &[
        va("a:CT_ConnectionSite", &["a:CT_ConnectionSiteList"]),
    ]),
    el("a:cxnLst", "a:CT_ConnectionSiteList", &[
        va("a:CT_ConnectionSiteList", &["a:CT_CustomGeometry2D"]),
    ]),
    el("a:cxnSp", "a:CT_GvmlConnector", &[
        va("a:CT_GvmlConnector", &["a:CT_GvmlGroupShape"]),
    ]),
    el("a:cxnSpLocks", "a:CT_ConnectorLocking", &[
        va("a:CT_ConnectorLocking", &["a:CT_NonVisualConnectorProperties"]),
    ]),
    el("a:defPPr", "a:CT_TextParagraphProperties", &[
        va("a:CT_TextParagraphProperties", &["a:CT_TextListStyle"]),
    ]),
    el("a:defRPr", "a:CT_TextCharacterProperties", &[
        va("a:CT_TextCharacterProperties", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:dgm", "a:CT_AnimationDgmElement", &[
        va("a:CT_AnimationDgmElement", &["a:CT_AnimationElementChoice"]),
    ]),
    el("a:dk1", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_ColorScheme"]),
    ]),
    el("a:dk2", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_ColorScheme"]),
    ]),
    el("a:ds", "a:CT_DashStop", &[
        va("a:CT_DashStop", &["a:CT_DashStopList"]),
    ]),
    el("a:duotone", "a:CT_DuotoneEffect", &[
        va("a:CT_DuotoneEffect", &["a:CT_Blip"]),
    ]),
    el("a:ea", "a:CT_TextFont", &[
        va("a:CT_TextFont", &["a:CT_FontCollection", "a:CT_TextCharacterProperties"]),
    ]),
    el("a:effect", "a:CT_EffectReference", &[
        va("a:CT_EffectProperties", &["a:CT_TableBackgroundStyle"]),
        va("a:CT_EffectReference", &[]),
    ]),
    el("a:effectDag", "a:CT_EffectContainer", &[
        va("a:CT_EffectContainer", &["a:CT_BackgroundFormatting", "a:CT_EffectProperties", "a:CT_EffectStyleItem", "a:CT_GroupShapeProperties", "a:CT_ShapeProperties", "a:CT_TableProperties", "a:CT_TextCharacterProperties", "a:CT_WholeE2oFormatting"]),
    ]),
    el("a:effectLst", "a:CT_EffectList", &[
        va("a:CT_EffectList", &["a:CT_BackgroundFormatting", "a:CT_EffectProperties", "a:CT_EffectStyleItem", "a:CT_GroupShapeProperties", "a:CT_ShapeProperties", "a:CT_TableProperties", "a:CT_TextCharacterProperties", "a:CT_WholeE2oFormatting"]),
    ]),
    el("a:effectRef", "a:CT_StyleMatrixReference", &[
        va("a:CT_StyleMatrixReference", &["a:CT_ShapeStyle", "a:CT_TableBackgroundStyle"]),
    ]),
    el("a:effectStyle", "a:CT_EffectStyleItem", &[
        va("a:CT_EffectStyleItem", &["a:CT_EffectStyleList"]),
    ]),
    el("a:effectStyleLst", "a:CT_EffectStyleList", &[
        va("a:CT_EffectStyleList", &["a:CT_StyleMatrix"]),
    ]),
    el("a:end", "a:CT_AudioCDTime", &[
        va("a:CT_AudioCDTime", &["a:CT_AudioCD"]),
    ]),
    el("a:endCxn", "a:CT_Connection", &[
        va("a:CT_Connection", &["a:CT_NonVisualConnectorProperties"]),
    ]),
    el("a:endParaRPr", "a:CT_TextCharacterProperties", &[
        va("a:CT_TextCharacterProperties", &["a:CT_TextParagraph"]),
    ]),
    el("a:ext", "a:CT_PositiveSize2D", &[
        va("a:CT_OfficeArtExtension", &["a:CT_OfficeArtExtensionList"]),
        va("a:CT_PositiveSize2D", &["a:CT_GroupTransform2D", "a:CT_Transform2D"]),
    ]),
    el("a:extLst", "a:CT_OfficeArtExtensionList", &[
        va("a:CT_OfficeArtExtensionList", &["a:CT_AudioCD", "a:CT_AudioFile", "a:CT_Backdrop", "a:CT_BaseStyles", "a:CT_Blip", "a:CT_Cell3D", "a:CT_ColorMapping", "a:CT_ColorScheme", "a:CT_ConnectorLocking", "a:CT_ContentPartLocking", "a:CT_DefaultShapeDefinition", "a:CT_FontCollection", "a:CT_FontScheme", "a:CT_GraphicalObjectFrameLocking", "a:CT_GroupLocking", "a:CT_GroupShapeProperties", "a:CT_GvmlConnector", "a:CT_GvmlGraphicalObjectFrame", "a:CT_GvmlGroupShape", "a:CT_GvmlPicture", "a:CT_GvmlShape", "a:CT_GvmlTextShape", "a:CT_Hyperlink", "a:CT_LineProperties", "a:CT_NonVisualConnectorProperties", "a:CT_NonVisualContentPartProperties", "a:CT_NonVisualDrawingProps", "a:CT_NonVisualDrawingShapeProps", "a:CT_NonVisualGraphicFrameProperties", "a:CT_NonVisualGroupDrawingShapeProps", "a:CT_NonVisualPictureProperties", "a:CT_ObjectStyleDefaults", "a:CT_OfficeStyleSheet", "a:CT_PictureLocking", "a:CT_QuickTimeFile", "a:CT_Scene3D", "a:CT_Shape3D", "a:CT_ShapeLocking", "a:CT_ShapeProperties", "a:CT_TableCell", "a:CT_TableCellBorderStyle", "a:CT_TableCellProperties", "a:CT_TableCol", "a:CT_TableProperties", "a:CT_TableRow", "a:CT_TableStyle", "a:CT_TableStyleTextStyle", "a:CT_TextBodyProperties", "a:CT_TextCharacterProperties", "a:CT_TextListStyle", "a:CT_TextParagraphProperties", "a:CT_VideoFile"]),
    ]),
    el("a:extraClrScheme", "a:CT_ColorSchemeAndMapping", &[
        va("a:CT_ColorSchemeAndMapping", &["a:CT_ColorSchemeList"]),
    ]),
    el("a:extraClrSchemeLst", "a:CT_ColorSchemeList", &[
        va("a:CT_ColorSchemeList", &["a:CT_OfficeStyleSheet"]),
    ]),
    el("a:extrusionClr", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_Shape3D"]),
    ]),
    el("a:fgClr", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_PatternFillProperties"]),
    ]),
    el("a:fill", "a:CT_FillEffect", &[
        va("a:CT_FillEffect", &[]),
        va("a:CT_FillProperties", &["a:CT_TableBackgroundStyle", "a:CT_TableStyleCellStyle"]),
    ]),
    el("a:fillOverlay", "a:CT_FillOverlayEffect", &[
        va("a:CT_FillOverlayEffect", &["a:CT_Blip", "a:CT_EffectList"]),
    ]),
    el("a:fillRect", "a:CT_RelativeRect", &[
        va("a:CT_RelativeRect", &["a:CT_StretchInfoProperties"]),
    ]),
    el("a:fillRef", "a:CT_StyleMatrixReference", &[
        va("a:CT_StyleMatrixReference", &["a:CT_ShapeStyle", "a:CT_TableBackgroundStyle", "a:CT_TableStyleCellStyle"]),
    ]),
    el("a:fillStyleLst", "a:CT_FillStyleList", &[
        va("a:CT_FillStyleList", &["a:CT_StyleMatrix"]),
    ]),
    el("a:fillToRect", "a:CT_RelativeRect", &[
        va("a:CT_RelativeRect", &["a:CT_PathShadeProperties"]),
    ]),
    el("a:firstCol", "a:CT_TablePartStyle", &[
        va("a:CT_TablePartStyle", &["a:CT_TableStyle"]),
    ]),
    el("a:firstRow", "a:CT_TablePartStyle", &[
        va("a:CT_TablePartStyle", &["a:CT_TableStyle"]),
    ]),
    el("a:flatTx", "a:CT_FlatText", &[
        va("a:CT_FlatText", &["a:CT_TextBodyProperties"]),
    ]),
    el("a:fld", "a:CT_TextField", &[
        va("a:CT_TextField", &["a:CT_TextParagraph"]),
    ]),
    el("a:fmtScheme", "a:CT_StyleMatrix", &[
        va("a:CT_StyleMatrix", &["a:CT_BaseStyles", "a:CT_BaseStylesOverride"]),
    ]),
    el("a:folHlink", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_ColorScheme"]),
    ]),
    el("a:font", "a:CT_SupplementalFont", &[
        va("a:CT_FontCollection", &["a:CT_TableStyleTextStyle"]),
        va("a:CT_SupplementalFont", &["a:CT_FontCollection"]),
    ]),
    el("a:fontRef", "a:CT_FontReference", &[
        va("a:CT_FontReference", &["a:CT_ShapeStyle", "a:CT_TableStyleTextStyle"]),
    ]),
    el("a:fontScheme", "a:CT_FontScheme", &[
        va("a:CT_FontScheme", &["a:CT_BaseStyles", "a:CT_BaseStylesOverride"]),
    ]),
    el("a:gamma", "a:CT_GammaTransform", &[
        va("a:CT_GammaTransform", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:gd", "a:CT_GeomGuide", &[
        va("a:CT_GeomGuide", &["a:CT_GeomGuideList"]),
    ]),
    el("a:gdLst", "a:CT_GeomGuideList", &[
        va("a:CT_GeomGuideList", &["a:CT_CustomGeometry2D"]),
    ]),
    el("a:glow", "a:CT_GlowEffect", &[
        va("a:CT_GlowEffect", &["a:CT_EffectList"]),
    ]),
    el("a:gradFill", "a:CT_GradientFillProperties", &[
        va("a:CT_GradientFillProperties", &["a:CT_BackgroundFillStyleList", "a:CT_BackgroundFormatting", "a:CT_FillEffect", "a:CT_FillOverlayEffect", "a:CT_FillProperties", "a:CT_FillStyleList", "a:CT_GroupShapeProperties", "a:CT_LineProperties", "a:CT_ShapeProperties", "a:CT_TableCellProperties", "a:CT_TableProperties", "a:CT_TextCharacterProperties"]),
    ]),
    el("a:graphic", "a:CT_GraphicalObject", &[
        va("a:CT_GraphicalObject", &["a:CT_GvmlGraphicalObjectFrame", "wp:CT_Anchor", "wp:CT_GraphicFrame", "wp:CT_Inline"]),
    ]),
    el("a:graphicData", "a:CT_GraphicalObjectData", &[
        va("a:CT_GraphicalObjectData", &["a:CT_GraphicalObject"]),
    ]),
    el("a:graphicFrame", "a:CT_GvmlGraphicalObjectFrame", &[
        va("a:CT_GvmlGraphicalObjectFrame", &["a:CT_GvmlGroupShape"]),
    ]),
    el("a:graphicFrameLocks", "a:CT_GraphicalObjectFrameLocking", &[
        va("a:CT_GraphicalObjectFrameLocking", &["a:CT_NonVisualGraphicFrameProperties"]),
    ]),
    el("a:gray", "a:CT_GrayscaleTransform", &[
        va("a:CT_GrayscaleTransform", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:grayscl", "a:CT_GrayscaleEffect", &[
        va("a:CT_GrayscaleEffect", &["a:CT_Blip"]),
    ]),
    el("a:green", "a:CT_Percentage", &[
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:greenMod", "a:CT_Percentage", &[
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:greenOff", "a:CT_Percentage", &[
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:gridCol", "a:CT_TableCol", &[
        va("a:CT_TableCol", &["a:CT_TableGrid"]),
    ]),
    el("a:grpFill", "a:CT_GroupFillProperties", &[
        va("a:CT_GroupFillProperties", &["a:CT_BackgroundFillStyleList", "a:CT_BackgroundFormatting", "a:CT_FillEffect", "a:CT_FillOverlayEffect", "a:CT_FillProperties", "a:CT_FillStyleList", "a:CT_GroupShapeProperties", "a:CT_ShapeProperties", "a:CT_TableCellProperties", "a:CT_TableProperties", "a:CT_TextCharacterProperties"]),
    ]),
    el("a:grpSp", "a:CT_GvmlGroupShape", &[
        va("a:CT_GvmlGroupShape", &["a:CT_GvmlGroupShape"]),
    ]),
    el("a:grpSpLocks", "a:CT_GroupLocking", &[
        va("a:CT_GroupLocking", &["a:CT_NonVisualGroupDrawingShapeProps"]),
    ]),
    el("a:grpSpPr", "a:CT_GroupShapeProperties", &[
        va("a:CT_GroupShapeProperties", &["a:CT_GvmlGroupShape"]),
    ]),
    el("a:gs", "a:CT_GradientStop", &[
        va("a:CT_GradientStop", &["a:CT_GradientStopList"]),
    ]),
    el("a:gsLst", "a:CT_GradientStopList", &[
        va("a:CT_GradientStopList", &["a:CT_GradientFillProperties"]),
    ]),
    el("a:headEnd", "a:CT_LineEndProperties", &[
        va("a:CT_LineEndProperties", &["a:CT_LineProperties"]),
    ]),
    el("a:header", "xsd:string", &[
        va("xsd:string", &["a:CT_Headers"]),
    ]),
    el("a:headers", "a:CT_Headers", &[
        va("a:CT_Headers", &["a:CT_TableCellProperties"]),
    ]),
    el("a:highlight", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_TextCharacterProperties"]),
    ]),
    el("a:hlink", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_ColorScheme"]),
    ]),
    el("a:hlinkClick", "a:CT_Hyperlink", &[
        va("a:CT_Hyperlink", &["a:CT_NonVisualDrawingProps", "a:CT_TextCharacterProperties"]),
    ]),
    el("a:hlinkHover", "a:CT_Hyperlink", &[
        va("a:CT_Hyperlink", &["a:CT_NonVisualDrawingProps"]),
    ]),
    el("a:hlinkMouseOver", "a:CT_Hyperlink", &[
        va("a:CT_Hyperlink", &["a:CT_TextCharacterProperties"]),
    ]),
    el("a:hsl", "a:CT_HSLEffect", &[
        va("a:CT_HSLEffect", &["a:CT_Blip"]),
    ]),
    el("a:hslClr", "a:CT_HslColor", &[
        va("a:CT_HslColor", &["a:CT_AlphaInverseEffect", "a:CT_Color", "a:CT_ColorMRU", "a:CT_ColorReplaceEffect", "a:CT_CustomColor", "a:CT_DuotoneEffect", "a:CT_FontReference", "a:CT_GlowEffect", "a:CT_GradientStop", "a:CT_InnerShadowEffect", "a:CT_OuterShadowEffect", "a:CT_PresetShadowEffect", "a:CT_SolidColorFillProperties", "a:CT_StyleMatrixReference", "a:CT_TableStyleTextStyle"]),
    ]),
    el("a:hue", "a:CT_PositiveFixedAngle", &[
        va("a:CT_PositiveFixedAngle", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:hueMod", "a:CT_PositivePercentage", &[
        va("a:CT_PositivePercentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:hueOff", "a:CT_Angle", &[
        va("a:CT_Angle", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:innerShdw", "a:CT_InnerShadowEffect", &[
        va("a:CT_InnerShadowEffect", &["a:CT_EffectList"]),
    ]),
    el("a:insideH", "a:CT_ThemeableLineStyle", &[
        va("a:CT_ThemeableLineStyle", &["a:CT_TableCellBorderStyle"]),
    ]),
    el("a:insideV", "a:CT_ThemeableLineStyle", &[
        va("a:CT_ThemeableLineStyle", &["a:CT_TableCellBorderStyle"]),
    ]),
    el("a:inv", "a:CT_InverseTransform", &[
        va("a:CT_InverseTransform", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:invGamma", "a:CT_InverseGammaTransform", &[
        va("a:CT_InverseGammaTransform", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:lastCol", "a:CT_TablePartStyle", &[
        va("a:CT_TablePartStyle", &["a:CT_TableStyle"]),
    ]),
    el("a:lastRow", "a:CT_TablePartStyle", &[
        va("a:CT_TablePartStyle", &["a:CT_TableStyle"]),
    ]),
    el("a:latin", "a:CT_TextFont", &[
        va("a:CT_TextFont", &["a:CT_FontCollection", "a:CT_TextCharacterProperties"]),
    ]),
    el("a:left", "a:CT_ThemeableLineStyle", &[
        va("a:CT_ThemeableLineStyle", &["a:CT_TableCellBorderStyle"]),
    ]),
    el("a:lightRig", "a:CT_LightRig", &[
        va("a:CT_LightRig", &["a:CT_Cell3D", "a:CT_Scene3D"]),
    ]),
    el("a:lin", "a:CT_LinearShadeProperties", &[
        va("a:CT_LinearShadeProperties", &["a:CT_GradientFillProperties"]),
    ]),
    el("a:ln", "a:CT_LineProperties", &[
        va("a:CT_LineProperties", &["a:CT_LineStyleList", "a:CT_ShapeProperties", "a:CT_TextCharacterProperties", "a:CT_ThemeableLineStyle", "a:CT_WholeE2oFormatting"]),
    ]),
    el("a:lnB", "a:CT_LineProperties", &[
        va("a:CT_LineProperties", &["a:CT_TableCellProperties"]),
    ]),
    el("a:lnBlToTr", "a:CT_LineProperties", &[
        va("a:CT_LineProperties", &["a:CT_TableCellProperties"]),
    ]),
    el("a:lnDef", "a:CT_DefaultShapeDefinition", &[
        va("a:CT_DefaultShapeDefinition", &["a:CT_ObjectStyleDefaults"]),
    ]),
    el("a:lnL", "a:CT_LineProperties", &[
        va("a:CT_LineProperties", &["a:CT_TableCellProperties"]),
    ]),
    el("a:lnR", "a:CT_LineProperties", &[
        va("a:CT_LineProperties", &["a:CT_TableCellProperties"]),
    ]),
    el("a:lnRef", "a:CT_StyleMatrixReference", &[
        va("a:CT_StyleMatrixReference", &["a:CT_ShapeStyle", "a:CT_ThemeableLineStyle"]),
    ]),
    el("a:lnSpc", "a:CT_TextSpacing", &[
        va("a:CT_TextSpacing", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:lnStyleLst", "a:CT_LineStyleList", &[
        va("a:CT_LineStyleList", &["a:CT_StyleMatrix"]),
    ]),
    el("a:lnT", "a:CT_LineProperties", &[
        va("a:CT_LineProperties", &["a:CT_TableCellProperties"]),
    ]),
    el("a:lnTlToBr", "a:CT_LineProperties", &[
        va("a:CT_LineProperties", &["a:CT_TableCellProperties"]),
    ]),
    el("a:lnTo", "a:CT_Path2DLineTo", &[
        va("a:CT_Path2DLineTo", &["a:CT_Path2D"]),
    ]),
    el("a:lstStyle", "a:CT_TextListStyle", &[
        va("a:CT_TextListStyle", &["a:CT_DefaultShapeDefinition", "a:CT_TextBody"]),
    ]),
    el("a:lt1", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_ColorScheme"]),
    ]),
    el("a:lt2", "a:CT_Color", &[
        va("a:CT_Color", &["a:CT_ColorScheme"]),
    ]),
    el("a:lum", "a:CT_LuminanceEffect", &[
        va("a:CT_LuminanceEffect", &["a:CT_Blip"]),
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:lumMod", "a:CT_Percentage", &[
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:lumOff", "a:CT_Percentage", &[
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:lvl1pPr", "a:CT_TextParagraphProperties", &[
        va("a:CT_TextParagraphProperties", &["a:CT_TextListStyle"]),
    ]),
    el("a:lvl2pPr", "a:CT_TextParagraphProperties", &[
        va("a:CT_TextParagraphProperties", &["a:CT_TextListStyle"]),
    ]),
    el("a:lvl3pPr", "a:CT_TextParagraphProperties", &[
        va("a:CT_TextParagraphProperties", &["a:CT_TextListStyle"]),
    ]),
    el("a:lvl4pPr", "a:CT_TextParagraphProperties", &[
        va("a:CT_TextParagraphProperties", &["a:CT_TextListStyle"]),
    ]),
    el("a:lvl5pPr", "a:CT_TextParagraphProperties", &[
        va("a:CT_TextParagraphProperties", &["a:CT_TextListStyle"]),
    ]),
    el("a:lvl6pPr", "a:CT_TextParagraphProperties", &[
        va("a:CT_TextParagraphProperties", &["a:CT_TextListStyle"]),
    ]),
    el("a:lvl7pPr", "a:CT_TextParagraphProperties", &[
        va("a:CT_TextParagraphProperties", &["a:CT_TextListStyle"]),
    ]),
    el("a:lvl8pPr", "a:CT_TextParagraphProperties", &[
        va("a:CT_TextParagraphProperties", &["a:CT_TextListStyle"]),
    ]),
    el("a:lvl9pPr", "a:CT_TextParagraphProperties", &[
        va("a:CT_TextParagraphProperties", &["a:CT_TextListStyle"]),
    ]),
    el("a:majorFont", "a:CT_FontCollection", &[
        va("a:CT_FontCollection", &["a:CT_FontScheme"]),
    ]),
    el("a:masterClrMapping", "a:CT_EmptyElement", &[
        va("a:CT_EmptyElement", &["a:CT_ColorMappingOverride"]),
    ]),
    el("a:minorFont", "a:CT_FontCollection", &[
        va("a:CT_FontCollection", &["a:CT_FontScheme"]),
    ]),
    el("a:miter", "a:CT_LineJoinMiterProperties", &[
        va("a:CT_LineJoinMiterProperties", &["a:CT_LineProperties"]),
    ]),
    el("a:moveTo", "a:CT_Path2DMoveTo", &[
        va("a:CT_Path2DMoveTo", &["a:CT_Path2D"]),
    ]),
    el("a:neCell", "a:CT_TablePartStyle", &[
        va("a:CT_TablePartStyle", &["a:CT_TableStyle"]),
    ]),
    el("a:noAutofit", "a:CT_TextNoAutofit", &[
        va("a:CT_TextNoAutofit", &["a:CT_TextBodyProperties"]),
    ]),
    el("a:noFill", "a:CT_NoFillProperties", &[
        va("a:CT_NoFillProperties", &["a:CT_BackgroundFillStyleList", "a:CT_BackgroundFormatting", "a:CT_FillEffect", "a:CT_FillOverlayEffect", "a:CT_FillProperties", "a:CT_FillStyleList", "a:CT_GroupShapeProperties", "a:CT_LineProperties", "a:CT_ShapeProperties", "a:CT_TableCellProperties", "a:CT_TableProperties", "a:CT_TextCharacterProperties"]),
    ]),
    el("a:norm", "a:CT_Vector3D", &[
        va("a:CT_Vector3D", &["a:CT_Backdrop"]),
    ]),
    el("a:normAutofit", "a:CT_TextNormalAutofit", &[
        va("a:CT_TextNormalAutofit", &["a:CT_TextBodyProperties"]),
    ]),
    el("a:nvCxnSpPr", "a:CT_GvmlConnectorNonVisual", &[
        va("a:CT_GvmlConnectorNonVisual", &["a:CT_GvmlConnector"]),
    ]),
    el("a:nvGraphicFramePr", "a:CT_GvmlGraphicFrameNonVisual", &[
        va("a:CT_GvmlGraphicFrameNonVisual", &["a:CT_GvmlGraphicalObjectFrame"]),
    ]),
    el("a:nvGrpSpPr", "a:CT_GvmlGroupShapeNonVisual", &[
        va("a:CT_GvmlGroupShapeNonVisual", &["a:CT_GvmlGroupShape"]),
    ]),
    el("a:nvPicPr", "a:CT_GvmlPictureNonVisual", &[
        va("a:CT_GvmlPictureNonVisual", &["a:CT_GvmlPicture"]),
    ]),
    el("a:nvSpPr", "a:CT_GvmlShapeNonVisual", &[
        va("a:CT_GvmlShapeNonVisual", &["a:CT_GvmlShape"]),
    ]),
    el("a:nwCell", "a:CT_TablePartStyle", &[
        va("a:CT_TablePartStyle", &["a:CT_TableStyle"]),
    ]),
    el("a:objectDefaults", "a:CT_ObjectStyleDefaults", &[
        va("a:CT_ObjectStyleDefaults", &["a:CT_OfficeStyleSheet"]),
    ]),
    el("a:off", "a:CT_Point2D", &[
        va("a:CT_Point2D", &["a:CT_GroupTransform2D", "a:CT_Transform2D"]),
    ]),
    el("a:outerShdw", "a:CT_OuterShadowEffect", &[
        va("a:CT_OuterShadowEffect", &["a:CT_EffectList"]),
    ]),
    el("a:overrideClrMapping", "a:CT_ColorMapping", &[
        va("a:CT_ColorMapping", &["a:CT_ColorMappingOverride"]),
    ]),
    el("a:p", "a:CT_TextParagraph", &[
        va("a:CT_TextParagraph", &["a:CT_TextBody"]),
    ]),
    el("a:pPr", "a:CT_TextParagraphProperties", &[
        va("a:CT_TextParagraphProperties", &["a:CT_TextField", "a:CT_TextParagraph"]),
    ]),
    el("a:path", "a:CT_PathShadeProperties", &[
        va("a:CT_Path2D", &["a:CT_Path2DList"]),
        va("a:CT_PathShadeProperties", &["a:CT_GradientFillProperties"]),
    ]),
    el("a:pathLst", "a:CT_Path2DList", &[
        va("a:CT_Path2DList", &["a:CT_CustomGeometry2D"]),
    ]),
    el("a:pattFill", "a:CT_PatternFillProperties", &[
        va("a:CT_PatternFillProperties", &["a:CT_BackgroundFillStyleList", "a:CT_BackgroundFormatting", "a:CT_FillEffect", "a:CT_FillOverlayEffect", "a:CT_FillProperties", "a:CT_FillStyleList", "a:CT_GroupShapeProperties", "a:CT_LineProperties", "a:CT_ShapeProperties", "a:CT_TableCellProperties", "a:CT_TableProperties", "a:CT_TextCharacterProperties"]),
    ]),
    el("a:pic", "a:CT_GvmlPicture", &[
        va("a:CT_GvmlPicture", &["a:CT_GvmlGroupShape"]),
    ]),
    el("a:picLocks", "a:CT_PictureLocking", &[
        va("a:CT_PictureLocking", &["a:CT_NonVisualPictureProperties"]),
    ]),
    el("a:pos", "a:CT_AdjPoint2D", &[
        va("a:CT_AdjPoint2D", &["a:CT_ConnectionSite", "a:CT_PolarAdjustHandle", "a:CT_XYAdjustHandle"]),
    ]),
    el("a:prstClr", "a:CT_PresetColor", &[
        va("a:CT_PresetColor", &["a:CT_AlphaInverseEffect", "a:CT_Color", "a:CT_ColorMRU", "a:CT_ColorReplaceEffect", "a:CT_CustomColor", "a:CT_DuotoneEffect", "a:CT_FontReference", "a:CT_GlowEffect", "a:CT_GradientStop", "a:CT_InnerShadowEffect", "a:CT_OuterShadowEffect", "a:CT_PresetShadowEffect", "a:CT_SolidColorFillProperties", "a:CT_StyleMatrixReference", "a:CT_TableStyleTextStyle"]),
    ]),
    el("a:prstDash", "a:CT_PresetLineDashProperties", &[
        va("a:CT_PresetLineDashProperties", &["a:CT_LineProperties"]),
    ]),
    el("a:prstGeom", "a:CT_PresetGeometry2D", &[
        va("a:CT_PresetGeometry2D", &["a:CT_ShapeProperties"]),
    ]),
    el("a:prstShdw", "a:CT_PresetShadowEffect", &[
        va("a:CT_PresetShadowEffect", &["a:CT_EffectList"]),
    ]),
    el("a:prstTxWarp", "a:CT_PresetTextShape", &[
        va("a:CT_PresetTextShape", &["a:CT_TextBodyProperties"]),
    ]),
    el("a:pt", "a:CT_AdjPoint2D", &[
        va("a:CT_AdjPoint2D", &["a:CT_Path2DCubicBezierTo", "a:CT_Path2DLineTo", "a:CT_Path2DMoveTo", "a:CT_Path2DQuadBezierTo"]),
    ]),
    el("a:quadBezTo", "a:CT_Path2DQuadBezierTo", &[
        va("a:CT_Path2DQuadBezierTo", &["a:CT_Path2D"]),
    ]),
    el("a:quickTimeFile", "a:CT_QuickTimeFile", &[
        va("a:CT_QuickTimeFile", &[]),
    ]),
    el("a:r", "a:CT_RegularTextRun", &[
        va("a:CT_RegularTextRun", &["a:CT_TextParagraph"]),
    ]),
    el("a:rPr", "a:CT_TextCharacterProperties", &[
        va("a:CT_TextCharacterProperties", &["a:CT_RegularTextRun", "a:CT_TextField", "a:CT_TextLineBreak"]),
    ]),
    el("a:rect", "a:CT_GeomRect", &[
        va("a:CT_GeomRect", &["a:CT_CustomGeometry2D"]),
    ]),
    el("a:red", "a:CT_Percentage", &[
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:redMod", "a:CT_Percentage", &[
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:redOff", "a:CT_Percentage", &[
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:reflection", "a:CT_ReflectionEffect", &[
        va("a:CT_ReflectionEffect", &["a:CT_EffectList"]),
    ]),
    el("a:relOff", "a:CT_RelativeOffsetEffect", &[
        va("a:CT_RelativeOffsetEffect", &[]),
    ]),
    el("a:right", "a:CT_ThemeableLineStyle", &[
        va("a:CT_ThemeableLineStyle", &["a:CT_TableCellBorderStyle"]),
    ]),
    el("a:rot", "a:CT_SphereCoords", &[
        va("a:CT_SphereCoords", &["a:CT_Camera", "a:CT_LightRig"]),
    ]),
    el("a:round", "a:CT_LineJoinRound", &[
        va("a:CT_LineJoinRound", &["a:CT_LineProperties"]),
    ]),
    el("a:rtl", "a:CT_Boolean", &[
        va("a:CT_Boolean", &["a:CT_TextCharacterProperties"]),
    ]),
    el("a:sat", "a:CT_Percentage", &[
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:satMod", "a:CT_Percentage", &[
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:satOff", "a:CT_Percentage", &[
        va("a:CT_Percentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:scene3d", "a:CT_Scene3D", &[
        va("a:CT_Scene3D", &["a:CT_EffectStyleItem", "a:CT_GroupShapeProperties", "a:CT_ShapeProperties", "a:CT_TextBodyProperties"]),
    ]),
    el("a:schemeClr", "a:CT_SchemeColor", &[
        va("a:CT_SchemeColor", &["a:CT_AlphaInverseEffect", "a:CT_Color", "a:CT_ColorMRU", "a:CT_ColorReplaceEffect", "a:CT_CustomColor", "a:CT_DuotoneEffect", "a:CT_FontReference", "a:CT_GlowEffect", "a:CT_GradientStop", "a:CT_InnerShadowEffect", "a:CT_OuterShadowEffect", "a:CT_PresetShadowEffect", "a:CT_SolidColorFillProperties", "a:CT_StyleMatrixReference", "a:CT_TableStyleTextStyle"]),
    ]),
    el("a:scrgbClr", "a:CT_ScRgbColor", &[
        va("a:CT_ScRgbColor", &["a:CT_AlphaInverseEffect", "a:CT_Color", "a:CT_ColorMRU", "a:CT_ColorReplaceEffect", "a:CT_CustomColor", "a:CT_DuotoneEffect", "a:CT_FontReference", "a:CT_GlowEffect", "a:CT_GradientStop", "a:CT_InnerShadowEffect", "a:CT_OuterShadowEffect", "a:CT_PresetShadowEffect", "a:CT_SolidColorFillProperties", "a:CT_StyleMatrixReference", "a:CT_TableStyleTextStyle"]),
    ]),
    el("a:seCell", "a:CT_TablePartStyle", &[
        va("a:CT_TablePartStyle", &["a:CT_TableStyle"]),
    ]),
    el("a:shade", "a:CT_PositiveFixedPercentage", &[
        va("a:CT_PositiveFixedPercentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
    ]),
    el("a:snd", "a:CT_EmbeddedWAVAudioFile", &[
        va("a:CT_EmbeddedWAVAudioFile", &["a:CT_Hyperlink"]),
    ]),
    el("a:softEdge", "a:CT_SoftEdgesEffect", &[
        va("a:CT_SoftEdgesEffect", &["a:CT_EffectList"]),
    ]),
    el("a:solidFill", "a:CT_SolidColorFillProperties", &[
        va("a:CT_SolidColorFillProperties", &["a:CT_BackgroundFillStyleList", "a:CT_BackgroundFormatting", "a:CT_FillEffect", "a:CT_FillOverlayEffect", "a:CT_FillProperties", "a:CT_FillStyleList", "a:CT_GroupShapeProperties", "a:CT_LineProperties", "a:CT_ShapeProperties", "a:CT_TableCellProperties", "a:CT_TableProperties", "a:CT_TextCharacterProperties"]),
    ]),
    el("a:sp", "a:CT_GvmlShape", &[
        va("a:CT_GvmlShape", &["a:CT_GvmlGroupShape"]),
    ]),
    el("a:sp3d", "a:CT_Shape3D", &[
        va("a:CT_Shape3D", &["a:CT_EffectStyleItem", "a:CT_ShapeProperties", "a:CT_TextBodyProperties"]),
    ]),
    el("a:spAutoFit", "a:CT_TextShapeAutofit", &[
        va("a:CT_TextShapeAutofit", &["a:CT_TextBodyProperties"]),
    ]),
    el("a:spDef", "a:CT_DefaultShapeDefinition", &[
        va("a:CT_DefaultShapeDefinition", &["a:CT_ObjectStyleDefaults"]),
    ]),
    el("a:spLocks", "a:CT_ShapeLocking", &[
        va("a:CT_ShapeLocking", &["a:CT_NonVisualDrawingShapeProps"]),
    ]),
    el("a:spPr", "a:CT_ShapeProperties", &[
        va("a:CT_ShapeProperties", &["a:CT_DefaultShapeDefinition", "a:CT_GvmlConnector", "a:CT_GvmlPicture", "a:CT_GvmlShape"]),
    ]),
    el("a:spcAft", "a:CT_TextSpacing", &[
        va("a:CT_TextSpacing", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:spcBef", "a:CT_TextSpacing", &[
        va("a:CT_TextSpacing", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:spcPct", "a:CT_TextSpacingPercent", &[
        va("a:CT_TextSpacingPercent", &["a:CT_TextSpacing"]),
    ]),
    el("a:spcPts", "a:CT_TextSpacingPoint", &[
        va("a:CT_TextSpacingPoint", &["a:CT_TextSpacing"]),
    ]),
    el("a:srcRect", "a:CT_RelativeRect", &[
        va("a:CT_RelativeRect", &["a:CT_BlipFillProperties"]),
    ]),
    el("a:srgbClr", "a:CT_SRgbColor", &[
        va("a:CT_SRgbColor", &["a:CT_AlphaInverseEffect", "a:CT_Color", "a:CT_ColorMRU", "a:CT_ColorReplaceEffect", "a:CT_CustomColor", "a:CT_DuotoneEffect", "a:CT_FontReference", "a:CT_GlowEffect", "a:CT_GradientStop", "a:CT_InnerShadowEffect", "a:CT_OuterShadowEffect", "a:CT_PresetShadowEffect", "a:CT_SolidColorFillProperties", "a:CT_StyleMatrixReference", "a:CT_TableStyleTextStyle"]),
    ]),
    el("a:st", "a:CT_AudioCDTime", &[
        va("a:CT_AudioCDTime", &["a:CT_AudioCD"]),
    ]),
    el("a:stCxn", "a:CT_Connection", &[
        va("a:CT_Connection", &["a:CT_NonVisualConnectorProperties"]),
    ]),
    el("a:stretch", "a:CT_StretchInfoProperties", &[
        va("a:CT_StretchInfoProperties", &["a:CT_BlipFillProperties"]),
    ]),
    el("a:style", "a:CT_ShapeStyle", &[
        va("a:CT_ShapeStyle", &["a:CT_DefaultShapeDefinition", "a:CT_GvmlConnector", "a:CT_GvmlPicture", "a:CT_GvmlShape"]),
    ]),
    el("a:swCell", "a:CT_TablePartStyle", &[
        va("a:CT_TablePartStyle", &["a:CT_TableStyle"]),
    ]),
    el("a:sx", "a:CT_Ratio", &[
        va("a:CT_Ratio", &["a:CT_Scale2D"]),
    ]),
    el("a:sy", "a:CT_Ratio", &[
        va("a:CT_Ratio", &["a:CT_Scale2D"]),
    ]),
    el("a:sym", "a:CT_TextFont", &[
        va("a:CT_TextFont", &["a:CT_TextCharacterProperties"]),
    ]),
    el("a:sysClr", "a:CT_SystemColor", &[
        va("a:CT_SystemColor", &["a:CT_AlphaInverseEffect", "a:CT_Color", "a:CT_ColorMRU", "a:CT_ColorReplaceEffect", "a:CT_CustomColor", "a:CT_DuotoneEffect", "a:CT_FontReference", "a:CT_GlowEffect", "a:CT_GradientStop", "a:CT_InnerShadowEffect", "a:CT_OuterShadowEffect", "a:CT_PresetShadowEffect", "a:CT_SolidColorFillProperties", "a:CT_StyleMatrixReference", "a:CT_TableStyleTextStyle"]),
    ]),
    el("a:t", "xsd:string", &[
        va("xsd:string", &["a:CT_RegularTextRun", "a:CT_TextField"]),
    ]),
    el("a:tab", "a:CT_TextTabStop", &[
        va("a:CT_TextTabStop", &["a:CT_TextTabStopList"]),
    ]),
    el("a:tabLst", "a:CT_TextTabStopList", &[
        va("a:CT_TextTabStopList", &["a:CT_TextParagraphProperties"]),
    ]),
    el("a:tableStyle", "a:CT_TableStyle", &[
        va("a:CT_TableStyle", &["a:CT_TableProperties"]),
    ]),
    el("a:tableStyleId", "s:ST_Guid", &[
        va("s:ST_Guid", &["a:CT_TableProperties"]),
    ]),
    el("a:tailEnd", "a:CT_LineEndProperties", &[
        va("a:CT_LineEndProperties", &["a:CT_LineProperties"]),
    ]),
    el("a:tbl", "a:CT_Table", &[
        va("a:CT_Table", &[]),
    ]),
    el("a:tblBg", "a:CT_TableBackgroundStyle", &[
        va("a:CT_TableBackgroundStyle", &["a:CT_TableStyle"]),
    ]),
    el("a:tblGrid", "a:CT_TableGrid", &[
        va("a:CT_TableGrid", &["a:CT_Table"]),
    ]),
    el("a:tblPr", "a:CT_TableProperties", &[
        va("a:CT_TableProperties", &["a:CT_Table"]),
    ]),
    el("a:tblStyle", "a:CT_TableStyle", &[
        va("a:CT_TableStyle", &["a:CT_TableStyleList"]),
    ]),
    el("a:tblStyleLst", "a:CT_TableStyleList", &[
        va("a:CT_TableStyleList", &[]),
    ]),
    el("a:tc", "a:CT_TableCell", &[
        va("a:CT_TableCell", &["a:CT_TableRow"]),
    ]),
    el("a:tcBdr", "a:CT_TableCellBorderStyle", &[
        va("a:CT_TableCellBorderStyle", &["a:CT_TableStyleCellStyle"]),
    ]),
    el("a:tcPr", "a:CT_TableCellProperties", &[
        va("a:CT_TableCellProperties", &["a:CT_TableCell"]),
    ]),
    el("a:tcStyle", "a:CT_TableStyleCellStyle", &[
        va("a:CT_TableStyleCellStyle", &["a:CT_TablePartStyle"]),
    ]),
    el("a:tcTxStyle", "a:CT_TableStyleTextStyle", &[
        va("a:CT_TableStyleTextStyle", &["a:CT_TablePartStyle"]),
    ]),
    el("a:theme", "a:CT_OfficeStyleSheet", &[
        va("a:CT_OfficeStyleSheet", &[]),
    ]),
    el("a:themeElements", "a:CT_BaseStyles", &[
        va("a:CT_BaseStyles", &["a:CT_ClipboardStyleSheet", "a:CT_OfficeStyleSheet"]),
    ]),
    el("a:themeManager", "a:CT_EmptyElement", &[
        va("a:CT_EmptyElement", &[]),
    ]),
    el("a:themeOverride", "a:CT_BaseStylesOverride", &[
        va("a:CT_BaseStylesOverride", &[]),
    ]),
    el("a:tile", "a:CT_TileInfoProperties", &[
        va("a:CT_TileInfoProperties", &["a:CT_BlipFillProperties"]),
    ]),
    el("a:tileRect", "a:CT_RelativeRect", &[
        va("a:CT_RelativeRect", &["a:CT_GradientFillProperties"]),
    ]),
    el("a:tint", "a:CT_TintEffect", &[
        va("a:CT_PositiveFixedPercentage", &["a:CT_HslColor", "a:CT_PresetColor", "a:CT_SRgbColor", "a:CT_ScRgbColor", "a:CT_SchemeColor", "a:CT_SystemColor"]),
        va("a:CT_TintEffect", &["a:CT_Blip"]),
    ]),
    el("a:tl2br", "a:CT_ThemeableLineStyle", &[
        va("a:CT_ThemeableLineStyle", &["a:CT_TableCellBorderStyle"]),
    ]),
    el("a:top", "a:CT_ThemeableLineStyle", &[
        va("a:CT_ThemeableLineStyle", &["a:CT_TableCellBorderStyle"]),
    ]),
    el("a:tr", "a:CT_TableRow", &[
        va("a:CT_TableRow", &["a:CT_Table"]),
    ]),
    el("a:tr2bl", "a:CT_ThemeableLineStyle", &[
        va("a:CT_ThemeableLineStyle", &["a:CT_TableCellBorderStyle"]),
    ]),
    el("a:txBody", "a:CT_TextBody", &[
        va("a:CT_TextBody", &["a:CT_GvmlTextShape", "a:CT_TableCell"]),
    ]),
    el("a:txDef", "a:CT_DefaultShapeDefinition", &[
        va("a:CT_DefaultShapeDefinition", &["a:CT_ObjectStyleDefaults"]),
    ]),
    el("a:txSp", "a:CT_GvmlTextShape", &[
        va("a:CT_GvmlTextShape", &["a:CT_GvmlGroupShape", "a:CT_GvmlShape"]),
    ]),
    el("a:uFill", "a:CT_TextUnderlineFillGroupWrapper", &[
        va("a:CT_TextUnderlineFillGroupWrapper", &["a:CT_TextCharacterProperties"]),
    ]),
    el("a:uFillTx", "a:CT_TextUnderlineFillFollowText", &[
        va("a:CT_TextUnderlineFillFollowText", &["a:CT_TextCharacterProperties"]),
    ]),
    el("a:uLn", "a:CT_LineProperties", &[
        va("a:CT_LineProperties", &["a:CT_TextCharacterProperties"]),
    ]),
    el("a:uLnTx", "a:CT_TextUnderlineLineFollowText", &[
        va("a:CT_TextUnderlineLineFollowText", &["a:CT_TextCharacterProperties"]),
    ]),
    el("a:up", "a:CT_Vector3D", &[
        va("a:CT_Vector3D", &["a:CT_Backdrop"]),
    ]),
    el("a:useSpRect", "a:CT_GvmlUseShapeRectangle", &[
        va("a:CT_GvmlUseShapeRectangle", &["a:CT_GvmlTextShape"]),
    ]),
    el("a:videoFile", "a:CT_VideoFile", &[
        va("a:CT_VideoFile", &[]),
    ]),
    el("a:wavAudioFile", "a:CT_EmbeddedWAVAudioFile", &[
        va("a:CT_EmbeddedWAVAudioFile", &[]),
    ]),
    el("a:wholeTbl", "a:CT_TablePartStyle", &[
        va("a:CT_TablePartStyle", &["a:CT_TableStyle"]),
    ]),
    el("a:xfrm", "a:CT_Transform2D", &[
        va("a:CT_GroupTransform2D", &["a:CT_GroupShapeProperties"]),
        va("a:CT_Transform2D", &["a:CT_GvmlGraphicalObjectFrame", "a:CT_GvmlTextShape", "a:CT_ShapeProperties"]),
        va("a:CT_TransformEffect", &[]),
    ]),
    el("b:AbbreviatedCaseNumber", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:AlbumTitle", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Artist", "b:CT_NameType", &[
        va("b:CT_NameType", &["b:CT_AuthorType"]),
    ]),
    el("b:Author", "b:CT_NameOrCorporateType", &[
        va("b:CT_AuthorType", &["b:CT_SourceType"]),
        va("b:CT_NameOrCorporateType", &["b:CT_AuthorType"]),
    ]),
    el("b:BookAuthor", "b:CT_NameType", &[
        va("b:CT_NameType", &["b:CT_AuthorType"]),
    ]),
    el("b:BookTitle", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:BroadcastTitle", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Broadcaster", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:CaseNumber", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:ChapterNumber", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:City", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Comments", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Compiler", "b:CT_NameType", &[
        va("b:CT_NameType", &["b:CT_AuthorType"]),
    ]),
    el("b:Composer", "b:CT_NameType", &[
        va("b:CT_NameType", &["b:CT_AuthorType"]),
    ]),
    el("b:Conductor", "b:CT_NameType", &[
        va("b:CT_NameType", &["b:CT_AuthorType"]),
    ]),
    el("b:ConferenceName", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Corporate", "s:ST_String", &[
        va("s:ST_String", &["b:CT_NameOrCorporateType"]),
    ]),
    el("b:Counsel", "b:CT_NameType", &[
        va("b:CT_NameType", &["b:CT_AuthorType"]),
    ]),
    el("b:CountryRegion", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Court", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Day", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:DayAccessed", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Department", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Director", "b:CT_NameType", &[
        va("b:CT_NameType", &["b:CT_AuthorType"]),
    ]),
    el("b:Distributor", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Edition", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Editor", "b:CT_NameType", &[
        va("b:CT_NameType", &["b:CT_AuthorType"]),
    ]),
    el("b:First", "s:ST_String", &[
        va("s:ST_String", &["b:CT_PersonType"]),
    ]),
    el("b:Guid", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Institution", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:InternetSiteTitle", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Interviewee", "b:CT_NameType", &[
        va("b:CT_NameType", &["b:CT_AuthorType"]),
    ]),
    el("b:Interviewer", "b:CT_NameType", &[
        va("b:CT_NameType", &["b:CT_AuthorType"]),
    ]),
    el("b:Inventor", "b:CT_NameType", &[
        va("b:CT_NameType", &["b:CT_AuthorType"]),
    ]),
    el("b:Issue", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:JournalName", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:LCID", "s:ST_Lang", &[
        va("s:ST_Lang", &["b:CT_SourceType"]),
    ]),
    el("b:Last", "s:ST_String", &[
        va("s:ST_String", &["b:CT_PersonType"]),
    ]),
    el("b:Medium", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Middle", "s:ST_String", &[
        va("s:ST_String", &["b:CT_PersonType"]),
    ]),
    el("b:Month", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:MonthAccessed", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:NameList", "b:CT_NameListType", &[
        va("b:CT_NameListType", &["b:CT_NameOrCorporateType", "b:CT_NameType"]),
    ]),
    el("b:NumberVolumes", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Pages", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:PatentNumber", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Performer", "b:CT_NameOrCorporateType", &[
        va("b:CT_NameOrCorporateType", &["b:CT_AuthorType"]),
    ]),
    el("b:PeriodicalTitle", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Person", "b:CT_PersonType", &[
        va("b:CT_PersonType", &["b:CT_NameListType"]),
    ]),
    el("b:ProducerName", "b:CT_NameType", &[
        va("b:CT_NameType", &["b:CT_AuthorType"]),
    ]),
    el("b:ProductionCompany", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:PublicationTitle", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Publisher", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:RecordingNumber", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:RefOrder", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Reporter", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:ShortTitle", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Source", "b:CT_SourceType", &[
        va("b:CT_SourceType", &["b:CT_Sources"]),
    ]),
    el("b:SourceType", "b:ST_SourceType", &[
        va("b:ST_SourceType", &["b:CT_SourceType"]),
    ]),
    el("b:Sources", "b:CT_Sources", &[
        va("b:CT_Sources", &[]),
    ]),
    el("b:StandardNumber", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:StateProvince", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Station", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Tag", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Theater", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:ThesisType", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Title", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Translator", "b:CT_NameType", &[
        va("b:CT_NameType", &["b:CT_AuthorType"]),
    ]),
    el("b:Type", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:URL", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Version", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Volume", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:Writer", "b:CT_NameType", &[
        va("b:CT_NameType", &["b:CT_AuthorType"]),
    ]),
    el("b:Year", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("b:YearAccessed", "s:ST_String", &[
        va("s:ST_String", &["b:CT_SourceType"]),
    ]),
    el("c:applyToEnd", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_PictureOptions"]),
    ]),
    el("c:applyToFront", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_PictureOptions"]),
    ]),
    el("c:applyToSides", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_PictureOptions"]),
    ]),
    el("c:area3DChart", "c:CT_Area3DChart", &[
        va("c:CT_Area3DChart", &["c:CT_PlotArea"]),
    ]),
    el("c:areaChart", "c:CT_AreaChart", &[
        va("c:CT_AreaChart", &["c:CT_PlotArea"]),
    ]),
    el("c:auto", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_CatAx", "c:CT_DateAx"]),
    ]),
    el("c:autoTitleDeleted", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_Chart"]),
    ]),
    el("c:autoUpdate", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_ExternalData"]),
    ]),
    el("c:axId", "c:CT_UnsignedInt", &[
        va("c:CT_UnsignedInt", &["c:CT_Area3DChart", "c:CT_AreaChart", "c:CT_Bar3DChart", "c:CT_BarChart", "c:CT_BubbleChart", "c:CT_CatAx", "c:CT_DateAx", "c:CT_Line3DChart", "c:CT_LineChart", "c:CT_RadarChart", "c:CT_ScatterChart", "c:CT_SerAx", "c:CT_StockChart", "c:CT_Surface3DChart", "c:CT_SurfaceChart", "c:CT_ValAx"]),
    ]),
    el("c:axPos", "c:CT_AxPos", &[
        va("c:CT_AxPos", &["c:CT_CatAx", "c:CT_DateAx", "c:CT_SerAx", "c:CT_ValAx"]),
    ]),
    el("c:backWall", "c:CT_Surface", &[
        va("c:CT_Surface", &["c:CT_Chart"]),
    ]),
    el("c:backward", "c:CT_Double", &[
        va("c:CT_Double", &["c:CT_Trendline"]),
    ]),
    el("c:bandFmt", "c:CT_BandFmt", &[
        va("c:CT_BandFmt", &["c:CT_BandFmts"]),
    ]),
    el("c:bandFmts", "c:CT_BandFmts", &[
        va("c:CT_BandFmts", &["c:CT_Surface3DChart", "c:CT_SurfaceChart"]),
    ]),
    el("c:bar3DChart", "c:CT_Bar3DChart", &[
        va("c:CT_Bar3DChart", &["c:CT_PlotArea"]),
    ]),
    el("c:barChart", "c:CT_BarChart", &[
        va("c:CT_BarChart", &["c:CT_PlotArea"]),
    ]),
    el("c:barDir", "c:CT_BarDir", &[
        va("c:CT_BarDir", &["c:CT_Bar3DChart", "c:CT_BarChart"]),
    ]),
    el("c:baseTimeUnit", "c:CT_TimeUnit", &[
        va("c:CT_TimeUnit", &["c:CT_DateAx"]),
    ]),
    el("c:bubble3D", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_BubbleChart", "c:CT_BubbleSer", "c:CT_DPt"]),
    ]),
    el("c:bubbleChart", "c:CT_BubbleChart", &[
        va("c:CT_BubbleChart", &["c:CT_PlotArea"]),
    ]),
    el("c:bubbleScale", "c:CT_BubbleScale", &[
        va("c:CT_BubbleScale", &["c:CT_BubbleChart"]),
    ]),
    el("c:bubbleSize", "c:CT_NumDataSource", &[
        va("c:CT_NumDataSource", &["c:CT_BubbleSer"]),
    ]),
    el("c:builtInUnit", "c:CT_BuiltInUnit", &[
        va("c:CT_BuiltInUnit", &["c:CT_DispUnits"]),
    ]),
    el("c:cat", "c:CT_AxDataSource", &[
        va("c:CT_AxDataSource", &["c:CT_AreaSer", "c:CT_BarSer", "c:CT_LineSer", "c:CT_PieSer", "c:CT_RadarSer", "c:CT_SurfaceSer"]),
    ]),
    el("c:catAx", "c:CT_CatAx", &[
        va("c:CT_CatAx", &["c:CT_PlotArea"]),
    ]),
    el("c:chart", "c:CT_Chart", &[
        va("c:CT_Chart", &["c:CT_ChartSpace"]),
        va("c:CT_RelId", &[]),
    ]),
    el("c:chartObject", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_Protection"]),
    ]),
    el("c:chartSpace", "c:CT_ChartSpace", &[
        va("c:CT_ChartSpace", &[]),
    ]),
    el("c:clrMapOvr", "a:CT_ColorMapping", &[
        va("a:CT_ColorMapping", &["c:CT_ChartSpace"]),
    ]),
    el("c:crossAx", "c:CT_UnsignedInt", &[
        va("c:CT_UnsignedInt", &["c:CT_CatAx", "c:CT_DateAx", "c:CT_SerAx", "c:CT_ValAx"]),
    ]),
    el("c:crossBetween", "c:CT_CrossBetween", &[
        va("c:CT_CrossBetween", &["c:CT_ValAx"]),
    ]),
    el("c:crosses", "c:CT_Crosses", &[
        va("c:CT_Crosses", &["c:CT_CatAx", "c:CT_DateAx", "c:CT_SerAx", "c:CT_ValAx"]),
    ]),
    el("c:crossesAt", "c:CT_Double", &[
        va("c:CT_Double", &["c:CT_CatAx", "c:CT_DateAx", "c:CT_SerAx", "c:CT_ValAx"]),
    ]),
    el("c:custSplit", "c:CT_CustSplit", &[
        va("c:CT_CustSplit", &["c:CT_OfPieChart"]),
    ]),
    el("c:custUnit", "c:CT_Double", &[
        va("c:CT_Double", &["c:CT_DispUnits"]),
    ]),
    el("c:dLbl", "c:CT_DLbl", &[
        va("c:CT_DLbl", &["c:CT_DLbls", "c:CT_PivotFmt"]),
    ]),
    el("c:dLblPos", "c:CT_DLblPos", &[
        va("c:CT_DLblPos", &["c:CT_DLbl", "c:CT_DLbls"]),
    ]),
    el("c:dLbls", "c:CT_DLbls", &[
        va("c:CT_DLbls", &["c:CT_Area3DChart", "c:CT_AreaChart", "c:CT_AreaSer", "c:CT_Bar3DChart", "c:CT_BarChart", "c:CT_BarSer", "c:CT_BubbleChart", "c:CT_BubbleSer", "c:CT_DoughnutChart", "c:CT_Line3DChart", "c:CT_LineChart", "c:CT_LineSer", "c:CT_OfPieChart", "c:CT_Pie3DChart", "c:CT_PieChart", "c:CT_PieSer", "c:CT_RadarChart", "c:CT_RadarSer", "c:CT_ScatterChart", "c:CT_ScatterSer", "c:CT_StockChart"]),
    ]),
    el("c:dPt", "c:CT_DPt", &[
        va("c:CT_DPt", &["c:CT_AreaSer", "c:CT_BarSer", "c:CT_BubbleSer", "c:CT_LineSer", "c:CT_PieSer", "c:CT_RadarSer", "c:CT_ScatterSer"]),
    ]),
    el("c:dTable", "c:CT_DTable", &[
        va("c:CT_DTable", &["c:CT_PlotArea"]),
    ]),
    el("c:data", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_Protection"]),
    ]),
    el("c:date1904", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_ChartSpace"]),
    ]),
    el("c:dateAx", "c:CT_DateAx", &[
        va("c:CT_DateAx", &["c:CT_PlotArea"]),
    ]),
    el("c:delete", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_CatAx", "c:CT_DLbl", "c:CT_DLbls", "c:CT_DateAx", "c:CT_LegendEntry", "c:CT_SerAx", "c:CT_ValAx"]),
    ]),
    el("c:depthPercent", "c:CT_DepthPercent", &[
        va("c:CT_DepthPercent", &["c:CT_View3D"]),
    ]),
    el("c:dispBlanksAs", "c:CT_DispBlanksAs", &[
        va("c:CT_DispBlanksAs", &["c:CT_Chart"]),
    ]),
    el("c:dispEq", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_Trendline"]),
    ]),
    el("c:dispRSqr", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_Trendline"]),
    ]),
    el("c:dispUnits", "c:CT_DispUnits", &[
        va("c:CT_DispUnits", &["c:CT_ValAx"]),
    ]),
    el("c:dispUnitsLbl", "c:CT_DispUnitsLbl", &[
        va("c:CT_DispUnitsLbl", &["c:CT_DispUnits"]),
    ]),
    el("c:doughnutChart", "c:CT_DoughnutChart", &[
        va("c:CT_DoughnutChart", &["c:CT_PlotArea"]),
    ]),
    el("c:downBars", "c:CT_UpDownBar", &[
        va("c:CT_UpDownBar", &["c:CT_UpDownBars"]),
    ]),
    el("c:dropLines", "c:CT_ChartLines", &[
        va("c:CT_ChartLines", &["c:CT_Area3DChart", "c:CT_AreaChart", "c:CT_Line3DChart", "c:CT_LineChart", "c:CT_StockChart"]),
    ]),
    el("c:errBarType", "c:CT_ErrBarType", &[
        va("c:CT_ErrBarType", &["c:CT_ErrBars"]),
    ]),
    el("c:errBars", "c:CT_ErrBars", &[
        va("c:CT_ErrBars", &["c:CT_AreaSer", "c:CT_BarSer", "c:CT_BubbleSer", "c:CT_LineSer", "c:CT_ScatterSer"]),
    ]),
    el("c:errDir", "c:CT_ErrDir", &[
        va("c:CT_ErrDir", &["c:CT_ErrBars"]),
    ]),
    el("c:errValType", "c:CT_ErrValType", &[
        va("c:CT_ErrValType", &["c:CT_ErrBars"]),
    ]),
    el("c:evenFooter", "s:ST_Xstring", &[
        va("s:ST_Xstring", &["c:CT_HeaderFooter"]),
    ]),
    el("c:evenHeader", "s:ST_Xstring", &[
        va("s:ST_Xstring", &["c:CT_HeaderFooter"]),
    ]),
    el("c:explosion", "c:CT_UnsignedInt", &[
        va("c:CT_UnsignedInt", &["c:CT_DPt", "c:CT_PieSer"]),
    ]),
    el("c:ext", "c:CT_Extension", &[
        va("c:CT_Extension", &["c:CT_ExtensionList"]),
    ]),
    el("c:extLst", "c:CT_ExtensionList", &[
        va("c:CT_ExtensionList", &["c:CT_Area3DChart", "c:CT_AreaChart", "c:CT_AreaSer", "c:CT_Bar3DChart", "c:CT_BarChart", "c:CT_BarSer", "c:CT_BubbleChart", "c:CT_BubbleSer", "c:CT_CatAx", "c:CT_Chart", "c:CT_ChartSpace", "c:CT_DLbl", "c:CT_DLbls", "c:CT_DPt", "c:CT_DTable", "c:CT_DateAx", "c:CT_DispUnits", "c:CT_DoughnutChart", "c:CT_ErrBars", "c:CT_Layout", "c:CT_Legend", "c:CT_LegendEntry", "c:CT_Line3DChart", "c:CT_LineChart", "c:CT_LineSer", "c:CT_ManualLayout", "c:CT_Marker", "c:CT_MultiLvlStrData", "c:CT_MultiLvlStrRef", "c:CT_NumData", "c:CT_NumRef", "c:CT_OfPieChart", "c:CT_Pie3DChart", "c:CT_PieChart", "c:CT_PieSer", "c:CT_PivotFmt", "c:CT_PivotSource", "c:CT_PlotArea", "c:CT_RadarChart", "c:CT_RadarSer", "c:CT_Scaling", "c:CT_ScatterChart", "c:CT_ScatterSer", "c:CT_SerAx", "c:CT_StockChart", "c:CT_StrData", "c:CT_StrRef", "c:CT_Surface", "c:CT_Surface3DChart", "c:CT_SurfaceChart", "c:CT_SurfaceSer", "c:CT_Title", "c:CT_Trendline", "c:CT_TrendlineLbl", "c:CT_UpDownBars", "c:CT_ValAx", "c:CT_View3D"]),
    ]),
    el("c:externalData", "c:CT_ExternalData", &[
        va("c:CT_ExternalData", &["c:CT_ChartSpace"]),
    ]),
    el("c:f", "xsd:string", &[
        va("xsd:string", &["c:CT_MultiLvlStrRef", "c:CT_NumRef", "c:CT_StrRef"]),
    ]),
    el("c:firstFooter", "s:ST_Xstring", &[
        va("s:ST_Xstring", &["c:CT_HeaderFooter"]),
    ]),
    el("c:firstHeader", "s:ST_Xstring", &[
        va("s:ST_Xstring", &["c:CT_HeaderFooter"]),
    ]),
    el("c:firstSliceAng", "c:CT_FirstSliceAng", &[
        va("c:CT_FirstSliceAng", &["c:CT_DoughnutChart", "c:CT_PieChart"]),
    ]),
    el("c:floor", "c:CT_Surface", &[
        va("c:CT_Surface", &["c:CT_Chart"]),
    ]),
    el("c:fmtId", "c:CT_UnsignedInt", &[
        va("c:CT_UnsignedInt", &["c:CT_PivotSource"]),
    ]),
    el("c:formatCode", "s:ST_Xstring", &[
        va("s:ST_Xstring", &["c:CT_NumData"]),
    ]),
    el("c:formatting", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_Protection"]),
    ]),
    el("c:forward", "c:CT_Double", &[
        va("c:CT_Double", &["c:CT_Trendline"]),
    ]),
    el("c:gapDepth", "c:CT_GapAmount", &[
        va("c:CT_GapAmount", &["c:CT_Area3DChart", "c:CT_Bar3DChart", "c:CT_Line3DChart"]),
    ]),
    el("c:gapWidth", "c:CT_GapAmount", &[
        va("c:CT_GapAmount", &["c:CT_Bar3DChart", "c:CT_BarChart", "c:CT_OfPieChart", "c:CT_UpDownBars"]),
    ]),
    el("c:grouping", "c:CT_Grouping", &[
        va("c:CT_BarGrouping", &["c:CT_Bar3DChart", "c:CT_BarChart"]),
        va("c:CT_Grouping", &["c:CT_Area3DChart", "c:CT_AreaChart", "c:CT_Line3DChart", "c:CT_LineChart"]),
    ]),
    el("c:h", "c:CT_Double", &[
        va("c:CT_Double", &["c:CT_ManualLayout"]),
    ]),
    el("c:hMode", "c:CT_LayoutMode", &[
        va("c:CT_LayoutMode", &["c:CT_ManualLayout"]),
    ]),
    el("c:hPercent", "c:CT_HPercent", &[
        va("c:CT_HPercent", &["c:CT_View3D"]),
    ]),
    el("c:headerFooter", "c:CT_HeaderFooter", &[
        va("c:CT_HeaderFooter", &["c:CT_PrintSettings"]),
    ]),
    el("c:hiLowLines", "c:CT_ChartLines", &[
        va("c:CT_ChartLines", &["c:CT_LineChart", "c:CT_StockChart"]),
    ]),
    el("c:holeSize", "c:CT_HoleSize", &[
        va("c:CT_HoleSize", &["c:CT_DoughnutChart"]),
    ]),
    el("c:idx", "c:CT_UnsignedInt", &[
        va("c:CT_UnsignedInt", &["c:CT_AreaSer", "c:CT_BandFmt", "c:CT_BarSer", "c:CT_BubbleSer", "c:CT_DLbl", "c:CT_DPt", "c:CT_LegendEntry", "c:CT_LineSer", "c:CT_PieSer", "c:CT_PivotFmt", "c:CT_RadarSer", "c:CT_ScatterSer", "c:CT_SurfaceSer"]),
    ]),
    el("c:intercept", "c:CT_Double", &[
        va("c:CT_Double", &["c:CT_Trendline"]),
    ]),
    el("c:invertIfNegative", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_BarSer", "c:CT_BubbleSer", "c:CT_DPt"]),
    ]),
    el("c:lang", "c:CT_TextLanguageID", &[
        va("c:CT_TextLanguageID", &["c:CT_ChartSpace"]),
    ]),
    el("c:layout", "c:CT_Layout", &[
        va("c:CT_Layout", &["c:CT_DLbl", "c:CT_DispUnitsLbl", "c:CT_Legend", "c:CT_PlotArea", "c:CT_Title", "c:CT_TrendlineLbl"]),
    ]),
    el("c:layoutTarget", "c:CT_LayoutTarget", &[
        va("c:CT_LayoutTarget", &["c:CT_ManualLayout"]),
    ]),
    el("c:lblAlgn", "c:CT_LblAlgn", &[
        va("c:CT_LblAlgn", &["c:CT_CatAx"]),
    ]),
    el("c:lblOffset", "c:CT_LblOffset", &[
        va("c:CT_LblOffset", &["c:CT_CatAx", "c:CT_DateAx"]),
    ]),
    el("c:leaderLines", "c:CT_ChartLines", &[
        va("c:CT_ChartLines", &["c:CT_DLbls"]),
    ]),
    el("c:legend", "c:CT_Legend", &[
        va("c:CT_Legend", &["c:CT_Chart"]),
    ]),
    el("c:legendEntry", "c:CT_LegendEntry", &[
        va("c:CT_LegendEntry", &["c:CT_Legend"]),
    ]),
    el("c:legendPos", "c:CT_LegendPos", &[
        va("c:CT_LegendPos", &["c:CT_Legend"]),
    ]),
    el("c:line3DChart", "c:CT_Line3DChart", &[
        va("c:CT_Line3DChart", &["c:CT_PlotArea"]),
    ]),
    el("c:lineChart", "c:CT_LineChart", &[
        va("c:CT_LineChart", &["c:CT_PlotArea"]),
    ]),
    el("c:logBase", "c:CT_LogBase", &[
        va("c:CT_LogBase", &["c:CT_Scaling"]),
    ]),
    el("c:lvl", "c:CT_Lvl", &[
        va("c:CT_Lvl", &["c:CT_MultiLvlStrData"]),
    ]),
    el("c:majorGridlines", "c:CT_ChartLines", &[
        va("c:CT_ChartLines", &["c:CT_CatAx", "c:CT_DateAx", "c:CT_SerAx", "c:CT_ValAx"]),
    ]),
    el("c:majorTickMark", "c:CT_TickMark", &[
        va("c:CT_TickMark", &["c:CT_CatAx", "c:CT_DateAx", "c:CT_SerAx", "c:CT_ValAx"]),
    ]),
    el("c:majorTimeUnit", "c:CT_TimeUnit", &[
        va("c:CT_TimeUnit", &["c:CT_DateAx"]),
    ]),
    el("c:majorUnit", "c:CT_AxisUnit", &[
        va("c:CT_AxisUnit", &["c:CT_DateAx", "c:CT_ValAx"]),
    ]),
    el("c:manualLayout", "c:CT_ManualLayout", &[
        va("c:CT_ManualLayout", &["c:CT_Layout"]),
    ]),
    el("c:marker", "c:CT_Marker", &[
        va("c:CT_Boolean", &["c:CT_LineChart"]),
        va("c:CT_Marker", &["c:CT_DPt", "c:CT_LineSer", "c:CT_PivotFmt", "c:CT_RadarSer", "c:CT_ScatterSer"]),
    ]),
    el("c:max", "c:CT_Double", &[
        va("c:CT_Double", &["c:CT_Scaling"]),
    ]),
    el("c:min", "c:CT_Double", &[
        va("c:CT_Double", &["c:CT_Scaling"]),
    ]),
    el("c:minorGridlines", "c:CT_ChartLines", &[
        va("c:CT_ChartLines", &["c:CT_CatAx", "c:CT_DateAx", "c:CT_SerAx", "c:CT_ValAx"]),
    ]),
    el("c:minorTickMark", "c:CT_TickMark", &[
        va("c:CT_TickMark", &["c:CT_CatAx", "c:CT_DateAx", "c:CT_SerAx", "c:CT_ValAx"]),
    ]),
    el("c:minorTimeUnit", "c:CT_TimeUnit", &[
        va("c:CT_TimeUnit", &["c:CT_DateAx"]),
    ]),
    el("c:minorUnit", "c:CT_AxisUnit", &[
        va("c:CT_AxisUnit", &["c:CT_DateAx", "c:CT_ValAx"]),
    ]),
    el("c:minus", "c:CT_NumDataSource", &[
        va("c:CT_NumDataSource", &["c:CT_ErrBars"]),
    ]),
    el("c:multiLvlStrCache", "c:CT_MultiLvlStrData", &[
        va("c:CT_MultiLvlStrData", &["c:CT_MultiLvlStrRef"]),
    ]),
    el("c:multiLvlStrRef", "c:CT_MultiLvlStrRef", &[
        va("c:CT_MultiLvlStrRef", &["c:CT_AxDataSource"]),
    ]),
    el("c:name", "xsd:string", &[
        va("s:ST_Xstring", &["c:CT_PivotSource"]),
        va("xsd:string", &["c:CT_Trendline"]),
    ]),
    el("c:noEndCap", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_ErrBars"]),
    ]),
    el("c:noMultiLvlLbl", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_CatAx"]),
    ]),
    el("c:numCache", "c:CT_NumData", &[
        va("c:CT_NumData", &["c:CT_NumRef"]),
    ]),
    el("c:numFmt", "c:CT_NumFmt", &[
        va("c:CT_NumFmt", &["c:CT_CatAx", "c:CT_DLbl", "c:CT_DLbls", "c:CT_DateAx", "c:CT_SerAx", "c:CT_TrendlineLbl", "c:CT_ValAx"]),
    ]),
    el("c:numLit", "c:CT_NumData", &[
        va("c:CT_NumData", &["c:CT_AxDataSource", "c:CT_NumDataSource"]),
    ]),
    el("c:numRef", "c:CT_NumRef", &[
        va("c:CT_NumRef", &["c:CT_AxDataSource", "c:CT_NumDataSource"]),
    ]),
    el("c:oddFooter", "s:ST_Xstring", &[
        va("s:ST_Xstring", &["c:CT_HeaderFooter"]),
    ]),
    el("c:oddHeader", "s:ST_Xstring", &[
        va("s:ST_Xstring", &["c:CT_HeaderFooter"]),
    ]),
    el("c:ofPieChart", "c:CT_OfPieChart", &[
        va("c:CT_OfPieChart", &["c:CT_PlotArea"]),
    ]),
    el("c:ofPieType", "c:CT_OfPieType", &[
        va("c:CT_OfPieType", &["c:CT_OfPieChart"]),
    ]),
    el("c:order", "c:CT_Order", &[
        va("c:CT_Order", &["c:CT_Trendline"]),
        va("c:CT_UnsignedInt", &["c:CT_AreaSer", "c:CT_BarSer", "c:CT_BubbleSer", "c:CT_LineSer", "c:CT_PieSer", "c:CT_RadarSer", "c:CT_ScatterSer", "c:CT_SurfaceSer"]),
    ]),
    el("c:orientation", "c:CT_Orientation", &[
        va("c:CT_Orientation", &["c:CT_Scaling"]),
    ]),
    el("c:overlap", "c:CT_Overlap", &[
        va("c:CT_Overlap", &["c:CT_BarChart"]),
    ]),
    el("c:overlay", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_Legend", "c:CT_Title"]),
    ]),
    el("c:pageMargins", "c:CT_PageMargins", &[
        va("c:CT_PageMargins", &["c:CT_PrintSettings"]),
    ]),
    el("c:pageSetup", "c:CT_PageSetup", &[
        va("c:CT_PageSetup", &["c:CT_PrintSettings"]),
    ]),
    el("c:period", "c:CT_Period", &[
        va("c:CT_Period", &["c:CT_Trendline"]),
    ]),
    el("c:perspective", "c:CT_Perspective", &[
        va("c:CT_Perspective", &["c:CT_View3D"]),
    ]),
    el("c:pictureFormat", "c:CT_PictureFormat", &[
        va("c:CT_PictureFormat", &["c:CT_PictureOptions"]),
    ]),
    el("c:pictureOptions", "c:CT_PictureOptions", &[
        va("c:CT_PictureOptions", &["c:CT_AreaSer", "c:CT_BarSer", "c:CT_DPt", "c:CT_Surface"]),
    ]),
    el("c:pictureStackUnit", "c:CT_PictureStackUnit", &[
        va("c:CT_PictureStackUnit", &["c:CT_PictureOptions"]),
    ]),
    el("c:pie3DChart", "c:CT_Pie3DChart", &[
        va("c:CT_Pie3DChart", &["c:CT_PlotArea"]),
    ]),
    el("c:pieChart", "c:CT_PieChart", &[
        va("c:CT_PieChart", &["c:CT_PlotArea"]),
    ]),
    el("c:pivotFmt", "c:CT_PivotFmt", &[
        va("c:CT_PivotFmt", &["c:CT_PivotFmts"]),
    ]),
    el("c:pivotFmts", "c:CT_PivotFmts", &[
        va("c:CT_PivotFmts", &["c:CT_Chart"]),
    ]),
    el("c:pivotSource", "c:CT_PivotSource", &[
        va("c:CT_PivotSource", &["c:CT_ChartSpace"]),
    ]),
    el("c:plotArea", "c:CT_PlotArea", &[
        va("c:CT_PlotArea", &["c:CT_Chart"]),
    ]),
    el("c:plotVisOnly", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_Chart"]),
    ]),
    el("c:plus", "c:CT_NumDataSource", &[
        va("c:CT_NumDataSource", &["c:CT_ErrBars"]),
    ]),
    el("c:printSettings", "c:CT_PrintSettings", &[
        va("c:CT_PrintSettings", &["c:CT_ChartSpace"]),
    ]),
    el("c:protection", "c:CT_Protection", &[
        va("c:CT_Protection", &["c:CT_ChartSpace"]),
    ]),
    el("c:pt", "c:CT_StrVal", &[
        va("c:CT_NumVal", &["c:CT_NumData"]),
        va("c:CT_StrVal", &["c:CT_Lvl", "c:CT_StrData"]),
    ]),
    el("c:ptCount", "c:CT_UnsignedInt", &[
        va("c:CT_UnsignedInt", &["c:CT_MultiLvlStrData", "c:CT_NumData", "c:CT_StrData"]),
    ]),
    el("c:rAngAx", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_View3D"]),
    ]),
    el("c:radarChart", "c:CT_RadarChart", &[
        va("c:CT_RadarChart", &["c:CT_PlotArea"]),
    ]),
    el("c:radarStyle", "c:CT_RadarStyle", &[
        va("c:CT_RadarStyle", &["c:CT_RadarChart"]),
    ]),
    el("c:rich", "a:CT_TextBody", &[
        va("a:CT_TextBody", &["c:CT_Tx"]),
    ]),
    el("c:rotX", "c:CT_RotX", &[
        va("c:CT_RotX", &["c:CT_View3D"]),
    ]),
    el("c:rotY", "c:CT_RotY", &[
        va("c:CT_RotY", &["c:CT_View3D"]),
    ]),
    el("c:roundedCorners", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_ChartSpace"]),
    ]),
    el("c:scaling", "c:CT_Scaling", &[
        va("c:CT_Scaling", &["c:CT_CatAx", "c:CT_DateAx", "c:CT_SerAx", "c:CT_ValAx"]),
    ]),
    el("c:scatterChart", "c:CT_ScatterChart", &[
        va("c:CT_ScatterChart", &["c:CT_PlotArea"]),
    ]),
    el("c:scatterStyle", "c:CT_ScatterStyle", &[
        va("c:CT_ScatterStyle", &["c:CT_ScatterChart"]),
    ]),
    el("c:secondPiePt", "c:CT_UnsignedInt", &[
        va("c:CT_UnsignedInt", &["c:CT_CustSplit"]),
    ]),
    el("c:secondPieSize", "c:CT_SecondPieSize", &[
        va("c:CT_SecondPieSize", &["c:CT_OfPieChart"]),
    ]),
    el("c:selection", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_Protection"]),
    ]),
    el("c:separator", "xsd:string", &[
        va("xsd:string", &["c:CT_DLbl", "c:CT_DLbls"]),
    ]),
    el("c:ser", "c:CT_LineSer", &[
        va("c:CT_AreaSer", &["c:CT_Area3DChart", "c:CT_AreaChart"]),
        va("c:CT_BarSer", &["c:CT_Bar3DChart", "c:CT_BarChart"]),
        va("c:CT_BubbleSer", &["c:CT_BubbleChart"]),
        va("c:CT_LineSer", &["c:CT_Line3DChart", "c:CT_LineChart", "c:CT_StockChart"]),
        va("c:CT_PieSer", &["c:CT_DoughnutChart", "c:CT_OfPieChart", "c:CT_Pie3DChart", "c:CT_PieChart"]),
        va("c:CT_RadarSer", &["c:CT_RadarChart"]),
        va("c:CT_ScatterSer", &["c:CT_ScatterChart"]),
        va("c:CT_SurfaceSer", &["c:CT_Surface3DChart", "c:CT_SurfaceChart"]),
    ]),
    el("c:serAx", "c:CT_SerAx", &[
        va("c:CT_SerAx", &["c:CT_PlotArea"]),
    ]),
    el("c:serLines", "c:CT_ChartLines", &[
        va("c:CT_ChartLines", &["c:CT_BarChart", "c:CT_OfPieChart"]),
    ]),
    el("c:shape", "c:CT_Shape", &[
        va("c:CT_Shape", &["c:CT_Bar3DChart", "c:CT_BarSer"]),
    ]),
    el("c:showBubbleSize", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_DLbl", "c:CT_DLbls"]),
    ]),
    el("c:showCatName", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_DLbl", "c:CT_DLbls"]),
    ]),
    el("c:showDLblsOverMax", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_Chart"]),
    ]),
    el("c:showHorzBorder", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_DTable"]),
    ]),
    el("c:showKeys", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_DTable"]),
    ]),
    el("c:showLeaderLines", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_DLbls"]),
    ]),
    el("c:showLegendKey", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_DLbl", "c:CT_DLbls"]),
    ]),
    el("c:showNegBubbles", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_BubbleChart"]),
    ]),
    el("c:showOutline", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_DTable"]),
    ]),
    el("c:showPercent", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_DLbl", "c:CT_DLbls"]),
    ]),
    el("c:showSerName", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_DLbl", "c:CT_DLbls"]),
    ]),
    el("c:showVal", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_DLbl", "c:CT_DLbls"]),
    ]),
    el("c:showVertBorder", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_DTable"]),
    ]),
    el("c:sideWall", "c:CT_Surface", &[
        va("c:CT_Surface", &["c:CT_Chart"]),
    ]),
    el("c:size", "c:CT_MarkerSize", &[
        va("c:CT_MarkerSize", &["c:CT_Marker"]),
    ]),
    el("c:sizeRepresents", "c:CT_SizeRepresents", &[
        va("c:CT_SizeRepresents", &["c:CT_BubbleChart"]),
    ]),
    el("c:smooth", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_LineChart", "c:CT_LineSer", "c:CT_ScatterSer"]),
    ]),
    el("c:spPr", "a:CT_ShapeProperties", &[
        va("a:CT_ShapeProperties", &["c:CT_AreaSer", "c:CT_BandFmt", "c:CT_BarSer", "c:CT_BubbleSer", "c:CT_CatAx", "c:CT_ChartLines", "c:CT_ChartSpace", "c:CT_DLbl", "c:CT_DLbls", "c:CT_DPt", "c:CT_DTable", "c:CT_DateAx", "c:CT_DispUnitsLbl", "c:CT_ErrBars", "c:CT_Legend", "c:CT_LineSer", "c:CT_Marker", "c:CT_PieSer", "c:CT_PivotFmt", "c:CT_PlotArea", "c:CT_RadarSer", "c:CT_ScatterSer", "c:CT_SerAx", "c:CT_Surface", "c:CT_SurfaceSer", "c:CT_Title", "c:CT_Trendline", "c:CT_TrendlineLbl", "c:CT_UpDownBar", "c:CT_ValAx"]),
    ]),
    el("c:splitPos", "c:CT_Double", &[
        va("c:CT_Double", &["c:CT_OfPieChart"]),
    ]),
    el("c:splitType", "c:CT_SplitType", &[
        va("c:CT_SplitType", &["c:CT_OfPieChart"]),
    ]),
    el("c:stockChart", "c:CT_StockChart", &[
        va("c:CT_StockChart", &["c:CT_PlotArea"]),
    ]),
    el("c:strCache", "c:CT_StrData", &[
        va("c:CT_StrData", &["c:CT_StrRef"]),
    ]),
    el("c:strLit", "c:CT_StrData", &[
        va("c:CT_StrData", &["c:CT_AxDataSource"]),
    ]),
    el("c:strRef", "c:CT_StrRef", &[
        va("c:CT_StrRef", &["c:CT_AxDataSource", "c:CT_SerTx", "c:CT_Tx"]),
    ]),
    el("c:style", "c:CT_Style", &[
        va("c:CT_Style", &["c:CT_ChartSpace"]),
    ]),
    el("c:surface3DChart", "c:CT_Surface3DChart", &[
        va("c:CT_Surface3DChart", &["c:CT_PlotArea"]),
    ]),
    el("c:surfaceChart", "c:CT_SurfaceChart", &[
        va("c:CT_SurfaceChart", &["c:CT_PlotArea"]),
    ]),
    el("c:symbol", "c:CT_MarkerStyle", &[
        va("c:CT_MarkerStyle", &["c:CT_Marker"]),
    ]),
    el("c:thickness", "c:CT_Thickness", &[
        va("c:CT_Thickness", &["c:CT_Surface"]),
    ]),
    el("c:tickLblPos", "c:CT_TickLblPos", &[
        va("c:CT_TickLblPos", &["c:CT_CatAx", "c:CT_DateAx", "c:CT_SerAx", "c:CT_ValAx"]),
    ]),
    el("c:tickLblSkip", "c:CT_Skip", &[
        va("c:CT_Skip", &["c:CT_CatAx", "c:CT_SerAx"]),
    ]),
    el("c:tickMarkSkip", "c:CT_Skip", &[
        va("c:CT_Skip", &["c:CT_CatAx", "c:CT_SerAx"]),
    ]),
    el("c:title", "c:CT_Title", &[
        va("c:CT_Title", &["c:CT_CatAx", "c:CT_Chart", "c:CT_DateAx", "c:CT_SerAx", "c:CT_ValAx"]),
    ]),
    el("c:trendline", "c:CT_Trendline", &[
        va("c:CT_Trendline", &["c:CT_AreaSer", "c:CT_BarSer", "c:CT_BubbleSer", "c:CT_LineSer", "c:CT_ScatterSer"]),
    ]),
    el("c:trendlineLbl", "c:CT_TrendlineLbl", &[
        va("c:CT_TrendlineLbl", &["c:CT_Trendline"]),
    ]),
    el("c:trendlineType", "c:CT_TrendlineType", &[
        va("c:CT_TrendlineType", &["c:CT_Trendline"]),
    ]),
    el("c:tx", "c:CT_Tx", &[
        va("c:CT_SerTx", &["c:CT_AreaSer", "c:CT_BarSer", "c:CT_BubbleSer", "c:CT_LineSer", "c:CT_PieSer", "c:CT_RadarSer", "c:CT_ScatterSer", "c:CT_SurfaceSer"]),
        va("c:CT_Tx", &["c:CT_DLbl", "c:CT_DispUnitsLbl", "c:CT_Title", "c:CT_TrendlineLbl"]),
    ]),
    el("c:txPr", "a:CT_TextBody", &[
        va("a:CT_TextBody", &["c:CT_CatAx", "c:CT_ChartSpace", "c:CT_DLbl", "c:CT_DLbls", "c:CT_DTable", "c:CT_DateAx", "c:CT_DispUnitsLbl", "c:CT_Legend", "c:CT_LegendEntry", "c:CT_PivotFmt", "c:CT_SerAx", "c:CT_Title", "c:CT_TrendlineLbl", "c:CT_ValAx"]),
    ]),
    el("c:upBars", "c:CT_UpDownBar", &[
        va("c:CT_UpDownBar", &["c:CT_UpDownBars"]),
    ]),
    el("c:upDownBars", "c:CT_UpDownBars", &[
        va("c:CT_UpDownBars", &["c:CT_LineChart", "c:CT_StockChart"]),
    ]),
    el("c:userInterface", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_Protection"]),
    ]),
    el("c:userShapes", "c:CT_RelId", &[
        va("c:CT_RelId", &["c:CT_ChartSpace"]),
        va("cdr:CT_Drawing", &[]),
    ]),
    el("c:v", "s:ST_Xstring", &[
        va("s:ST_Xstring", &["c:CT_NumVal", "c:CT_SerTx", "c:CT_StrVal"]),
    ]),
    el("c:val", "c:CT_NumDataSource", &[
        va("c:CT_Double", &["c:CT_ErrBars"]),
        va("c:CT_NumDataSource", &["c:CT_AreaSer", "c:CT_BarSer", "c:CT_LineSer", "c:CT_PieSer", "c:CT_RadarSer", "c:CT_SurfaceSer"]),
    ]),
    el("c:valAx", "c:CT_ValAx", &[
        va("c:CT_ValAx", &["c:CT_PlotArea"]),
    ]),
    el("c:varyColors", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_Area3DChart", "c:CT_AreaChart", "c:CT_Bar3DChart", "c:CT_BarChart", "c:CT_BubbleChart", "c:CT_DoughnutChart", "c:CT_Line3DChart", "c:CT_LineChart", "c:CT_OfPieChart", "c:CT_Pie3DChart", "c:CT_PieChart", "c:CT_RadarChart", "c:CT_ScatterChart"]),
    ]),
    el("c:view3D", "c:CT_View3D", &[
        va("c:CT_View3D", &["c:CT_Chart"]),
    ]),
    el("c:w", "c:CT_Double", &[
        va("c:CT_Double", &["c:CT_ManualLayout"]),
    ]),
    el("c:wMode", "c:CT_LayoutMode", &[
        va("c:CT_LayoutMode", &["c:CT_ManualLayout"]),
    ]),
    el("c:wireframe", "c:CT_Boolean", &[
        va("c:CT_Boolean", &["c:CT_Surface3DChart", "c:CT_SurfaceChart"]),
    ]),
    el("c:x", "c:CT_Double", &[
        va("c:CT_Double", &["c:CT_ManualLayout"]),
    ]),
    el("c:xMode", "c:CT_LayoutMode", &[
        va("c:CT_LayoutMode", &["c:CT_ManualLayout"]),
    ]),
    el("c:xVal", "c:CT_AxDataSource", &[
        va("c:CT_AxDataSource", &["c:CT_BubbleSer", "c:CT_ScatterSer"]),
    ]),
    el("c:y", "c:CT_Double", &[
        va("c:CT_Double", &["c:CT_ManualLayout"]),
    ]),
    el("c:yMode", "c:CT_LayoutMode", &[
        va("c:CT_LayoutMode", &["c:CT_ManualLayout"]),
    ]),
    el("c:yVal", "c:CT_NumDataSource", &[
        va("c:CT_NumDataSource", &["c:CT_BubbleSer", "c:CT_ScatterSer"]),
    ]),
    el("m:acc", "m:CT_Acc", &[
        va("m:CT_Acc", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:accPr", "m:CT_AccPr", &[
        va("m:CT_AccPr", &["m:CT_Acc"]),
    ]),
    el("m:aln", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_BoxPr", "m:CT_RPR"]),
    ]),
    el("m:alnScr", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_SSubSupPr"]),
    ]),
    el("m:argPr", "m:CT_OMathArgPr", &[
        va("m:CT_OMathArgPr", &["m:CT_OMathArg"]),
    ]),
    el("m:argSz", "m:CT_Integer2", &[
        va("m:CT_Integer2", &["m:CT_OMathArgPr"]),
    ]),
    el("m:bar", "m:CT_Bar", &[
        va("m:CT_Bar", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:barPr", "m:CT_BarPr", &[
        va("m:CT_BarPr", &["m:CT_Bar"]),
    ]),
    el("m:baseJc", "m:CT_YAlign", &[
        va("m:CT_YAlign", &["m:CT_EqArrPr", "m:CT_MPr"]),
    ]),
    el("m:begChr", "m:CT_Char", &[
        va("m:CT_Char", &["m:CT_DPr"]),
    ]),
    el("m:borderBox", "m:CT_BorderBox", &[
        va("m:CT_BorderBox", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:borderBoxPr", "m:CT_BorderBoxPr", &[
        va("m:CT_BorderBoxPr", &["m:CT_BorderBox"]),
    ]),
    el("m:box", "m:CT_Box", &[
        va("m:CT_Box", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:boxPr", "m:CT_BoxPr", &[
        va("m:CT_BoxPr", &["m:CT_Box"]),
    ]),
    el("m:brk", "m:CT_ManualBreak", &[
        va("m:CT_ManualBreak", &["m:CT_BoxPr", "m:CT_RPR"]),
    ]),
    el("m:brkBin", "m:CT_BreakBin", &[
        va("m:CT_BreakBin", &["m:CT_MathPr"]),
    ]),
    el("m:brkBinSub", "m:CT_BreakBinSub", &[
        va("m:CT_BreakBinSub", &["m:CT_MathPr"]),
    ]),
    el("m:cGp", "m:CT_UnSignedInteger", &[
        va("m:CT_UnSignedInteger", &["m:CT_MPr"]),
    ]),
    el("m:cGpRule", "m:CT_SpacingRule", &[
        va("m:CT_SpacingRule", &["m:CT_MPr"]),
    ]),
    el("m:cSp", "m:CT_UnSignedInteger", &[
        va("m:CT_UnSignedInteger", &["m:CT_MPr"]),
    ]),
    el("m:chr", "m:CT_Char", &[
        va("m:CT_Char", &["m:CT_AccPr", "m:CT_GroupChrPr", "m:CT_NaryPr"]),
    ]),
    el("m:count", "m:CT_Integer255", &[
        va("m:CT_Integer255", &["m:CT_MCPr"]),
    ]),
    el("m:ctrlPr", "m:CT_CtrlPr", &[
        va("m:CT_CtrlPr", &["m:CT_AccPr", "m:CT_BarPr", "m:CT_BorderBoxPr", "m:CT_BoxPr", "m:CT_DPr", "m:CT_EqArrPr", "m:CT_FPr", "m:CT_FuncPr", "m:CT_GroupChrPr", "m:CT_LimLowPr", "m:CT_LimUppPr", "m:CT_MPr", "m:CT_NaryPr", "m:CT_OMathArg", "m:CT_PhantPr", "m:CT_RadPr", "m:CT_SPrePr", "m:CT_SSubPr", "m:CT_SSubSupPr", "m:CT_SSupPr"]),
    ]),
    el("m:d", "m:CT_D", &[
        va("m:CT_D", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:dPr", "m:CT_DPr", &[
        va("m:CT_DPr", &["m:CT_D"]),
    ]),
    el("m:defJc", "m:CT_OMathJc", &[
        va("m:CT_OMathJc", &["m:CT_MathPr"]),
    ]),
    el("m:deg", "m:CT_OMathArg", &[
        va("m:CT_OMathArg", &["m:CT_Rad"]),
    ]),
    el("m:degHide", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_RadPr"]),
    ]),
    el("m:den", "m:CT_OMathArg", &[
        va("m:CT_OMathArg", &["m:CT_F"]),
    ]),
    el("m:diff", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_BoxPr"]),
    ]),
    el("m:dispDef", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_MathPr"]),
    ]),
    el("m:e", "m:CT_OMathArg", &[
        va("m:CT_OMathArg", &["m:CT_Acc", "m:CT_Bar", "m:CT_BorderBox", "m:CT_Box", "m:CT_D", "m:CT_EqArr", "m:CT_Func", "m:CT_GroupChr", "m:CT_LimLow", "m:CT_LimUpp", "m:CT_MR", "m:CT_Nary", "m:CT_Phant", "m:CT_Rad", "m:CT_SPre", "m:CT_SSub", "m:CT_SSubSup", "m:CT_SSup"]),
    ]),
    el("m:endChr", "m:CT_Char", &[
        va("m:CT_Char", &["m:CT_DPr"]),
    ]),
    el("m:eqArr", "m:CT_EqArr", &[
        va("m:CT_EqArr", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:eqArrPr", "m:CT_EqArrPr", &[
        va("m:CT_EqArrPr", &["m:CT_EqArr"]),
    ]),
    el("m:f", "m:CT_F", &[
        va("m:CT_F", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:fName", "m:CT_OMathArg", &[
        va("m:CT_OMathArg", &["m:CT_Func"]),
    ]),
    el("m:fPr", "m:CT_FPr", &[
        va("m:CT_FPr", &["m:CT_F"]),
    ]),
    el("m:func", "m:CT_Func", &[
        va("m:CT_Func", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:funcPr", "m:CT_FuncPr", &[
        va("m:CT_FuncPr", &["m:CT_Func"]),
    ]),
    el("m:groupChr", "m:CT_GroupChr", &[
        va("m:CT_GroupChr", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:groupChrPr", "m:CT_GroupChrPr", &[
        va("m:CT_GroupChrPr", &["m:CT_GroupChr"]),
    ]),
    el("m:grow", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_DPr", "m:CT_NaryPr"]),
    ]),
    el("m:hideBot", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_BorderBoxPr"]),
    ]),
    el("m:hideLeft", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_BorderBoxPr"]),
    ]),
    el("m:hideRight", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_BorderBoxPr"]),
    ]),
    el("m:hideTop", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_BorderBoxPr"]),
    ]),
    el("m:intLim", "m:CT_LimLoc", &[
        va("m:CT_LimLoc", &["m:CT_MathPr"]),
    ]),
    el("m:interSp", "m:CT_TwipsMeasure", &[
        va("m:CT_TwipsMeasure", &["m:CT_MathPr"]),
    ]),
    el("m:intraSp", "m:CT_TwipsMeasure", &[
        va("m:CT_TwipsMeasure", &["m:CT_MathPr"]),
    ]),
    el("m:jc", "m:CT_OMathJc", &[
        va("m:CT_OMathJc", &["m:CT_OMathParaPr"]),
    ]),
    el("m:lMargin", "m:CT_TwipsMeasure", &[
        va("m:CT_TwipsMeasure", &["m:CT_MathPr"]),
    ]),
    el("m:lim", "m:CT_OMathArg", &[
        va("m:CT_OMathArg", &["m:CT_LimLow", "m:CT_LimUpp"]),
    ]),
    el("m:limLoc", "m:CT_LimLoc", &[
        va("m:CT_LimLoc", &["m:CT_NaryPr"]),
    ]),
    el("m:limLow", "m:CT_LimLow", &[
        va("m:CT_LimLow", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:limLowPr", "m:CT_LimLowPr", &[
        va("m:CT_LimLowPr", &["m:CT_LimLow"]),
    ]),
    el("m:limUpp", "m:CT_LimUpp", &[
        va("m:CT_LimUpp", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:limUppPr", "m:CT_LimUppPr", &[
        va("m:CT_LimUppPr", &["m:CT_LimUpp"]),
    ]),
    el("m:lit", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_RPR"]),
    ]),
    el("m:m", "m:CT_M", &[
        va("m:CT_M", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:mPr", "m:CT_MPr", &[
        va("m:CT_MPr", &["m:CT_M"]),
    ]),
    el("m:mathFont", "m:CT_String", &[
        va("m:CT_String", &["m:CT_MathPr"]),
    ]),
    el("m:mathPr", "m:CT_MathPr", &[
        va("m:CT_MathPr", &["w:CT_Settings"]),
    ]),
    el("m:maxDist", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_EqArrPr"]),
    ]),
    el("m:mc", "m:CT_MC", &[
        va("m:CT_MC", &["m:CT_MCS"]),
    ]),
    el("m:mcJc", "m:CT_XAlign", &[
        va("m:CT_XAlign", &["m:CT_MCPr"]),
    ]),
    el("m:mcPr", "m:CT_MCPr", &[
        va("m:CT_MCPr", &["m:CT_MC"]),
    ]),
    el("m:mcs", "m:CT_MCS", &[
        va("m:CT_MCS", &["m:CT_MPr"]),
    ]),
    el("m:mr", "m:CT_MR", &[
        va("m:CT_MR", &["m:CT_M"]),
    ]),
    el("m:nary", "m:CT_Nary", &[
        va("m:CT_Nary", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:naryLim", "m:CT_LimLoc", &[
        va("m:CT_LimLoc", &["m:CT_MathPr"]),
    ]),
    el("m:naryPr", "m:CT_NaryPr", &[
        va("m:CT_NaryPr", &["m:CT_Nary"]),
    ]),
    el("m:noBreak", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_BoxPr"]),
    ]),
    el("m:nor", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_RPR"]),
    ]),
    el("m:num", "m:CT_OMathArg", &[
        va("m:CT_OMathArg", &["m:CT_F"]),
    ]),
    el("m:oMath", "m:CT_OMath", &[
        va("m:CT_OMath", &["m:CT_OMath", "m:CT_OMathArg", "m:CT_OMathPara", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("m:oMathPara", "m:CT_OMathPara", &[
        va("m:CT_OMathPara", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("m:oMathParaPr", "m:CT_OMathParaPr", &[
        va("m:CT_OMathParaPr", &["m:CT_OMathPara"]),
    ]),
    el("m:objDist", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_EqArrPr"]),
    ]),
    el("m:opEmu", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_BoxPr"]),
    ]),
    el("m:phant", "m:CT_Phant", &[
        va("m:CT_Phant", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:phantPr", "m:CT_PhantPr", &[
        va("m:CT_PhantPr", &["m:CT_Phant"]),
    ]),
    el("m:plcHide", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_MPr"]),
    ]),
    el("m:pos", "m:CT_TopBot", &[
        va("m:CT_TopBot", &["m:CT_BarPr", "m:CT_GroupChrPr"]),
    ]),
    el("m:postSp", "m:CT_TwipsMeasure", &[
        va("m:CT_TwipsMeasure", &["m:CT_MathPr"]),
    ]),
    el("m:preSp", "m:CT_TwipsMeasure", &[
        va("m:CT_TwipsMeasure", &["m:CT_MathPr"]),
    ]),
    el("m:r", "m:CT_R", &[
        va("m:CT_R", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:rMargin", "m:CT_TwipsMeasure", &[
        va("m:CT_TwipsMeasure", &["m:CT_MathPr"]),
    ]),
    el("m:rPr", "m:CT_RPR", &[
        va("m:CT_RPR", &["m:CT_R"]),
    ]),
    el("m:rSp", "m:CT_UnSignedInteger", &[
        va("m:CT_UnSignedInteger", &["m:CT_EqArrPr", "m:CT_MPr"]),
    ]),
    el("m:rSpRule", "m:CT_SpacingRule", &[
        va("m:CT_SpacingRule", &["m:CT_EqArrPr", "m:CT_MPr"]),
    ]),
    el("m:rad", "m:CT_Rad", &[
        va("m:CT_Rad", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:radPr", "m:CT_RadPr", &[
        va("m:CT_RadPr", &["m:CT_Rad"]),
    ]),
    el("m:sPre", "m:CT_SPre", &[
        va("m:CT_SPre", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:sPrePr", "m:CT_SPrePr", &[
        va("m:CT_SPrePr", &["m:CT_SPre"]),
    ]),
    el("m:sSub", "m:CT_SSub", &[
        va("m:CT_SSub", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:sSubPr", "m:CT_SSubPr", &[
        va("m:CT_SSubPr", &["m:CT_SSub"]),
    ]),
    el("m:sSubSup", "m:CT_SSubSup", &[
        va("m:CT_SSubSup", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:sSubSupPr", "m:CT_SSubSupPr", &[
        va("m:CT_SSubSupPr", &["m:CT_SSubSup"]),
    ]),
    el("m:sSup", "m:CT_SSup", &[
        va("m:CT_SSup", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_RunTrackChange"]),
    ]),
    el("m:sSupPr", "m:CT_SSupPr", &[
        va("m:CT_SSupPr", &["m:CT_SSup"]),
    ]),
    el("m:scr", "m:CT_Script", &[
        va("m:CT_Script", &["m:CT_RPR"]),
    ]),
    el("m:sepChr", "m:CT_Char", &[
        va("m:CT_Char", &["m:CT_DPr"]),
    ]),
    el("m:show", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_PhantPr"]),
    ]),
    el("m:shp", "m:CT_Shp", &[
        va("m:CT_Shp", &["m:CT_DPr"]),
    ]),
    el("m:smallFrac", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_MathPr"]),
    ]),
    el("m:strikeBLTR", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_BorderBoxPr"]),
    ]),
    el("m:strikeH", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_BorderBoxPr"]),
    ]),
    el("m:strikeTLBR", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_BorderBoxPr"]),
    ]),
    el("m:strikeV", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_BorderBoxPr"]),
    ]),
    el("m:sty", "m:CT_Style", &[
        va("m:CT_Style", &["m:CT_RPR"]),
    ]),
    el("m:sub", "m:CT_OMathArg", &[
        va("m:CT_OMathArg", &["m:CT_Nary", "m:CT_SPre", "m:CT_SSub", "m:CT_SSubSup"]),
    ]),
    el("m:subHide", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_NaryPr"]),
    ]),
    el("m:sup", "m:CT_OMathArg", &[
        va("m:CT_OMathArg", &["m:CT_Nary", "m:CT_SPre", "m:CT_SSubSup", "m:CT_SSup"]),
    ]),
    el("m:supHide", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_NaryPr"]),
    ]),
    el("m:t", "m:CT_Text", &[
        va("m:CT_Text", &["m:CT_R"]),
    ]),
    el("m:transp", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_PhantPr"]),
    ]),
    el("m:type", "m:CT_FType", &[
        va("m:CT_FType", &["m:CT_FPr"]),
    ]),
    el("m:vertJc", "m:CT_TopBot", &[
        va("m:CT_TopBot", &["m:CT_GroupChrPr"]),
    ]),
    el("m:wrapIndent", "m:CT_TwipsMeasure", &[
        va("m:CT_TwipsMeasure", &["m:CT_MathPr"]),
    ]),
    el("m:wrapRight", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_MathPr"]),
    ]),
    el("m:zeroAsc", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_PhantPr"]),
    ]),
    el("m:zeroDesc", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_PhantPr"]),
    ]),
    el("m:zeroWid", "m:CT_OnOff", &[
        va("m:CT_OnOff", &["m:CT_PhantPr"]),
    ]),
    el("pic:blipFill", "a:CT_BlipFillProperties", &[
        va("a:CT_BlipFillProperties", &["pic:CT_Picture"]),
    ]),
    el("pic:cNvPicPr", "a:CT_NonVisualPictureProperties", &[
        va("a:CT_NonVisualPictureProperties", &["pic:CT_PictureNonVisual"]),
    ]),
    el("pic:cNvPr", "a:CT_NonVisualDrawingProps", &[
        va("a:CT_NonVisualDrawingProps", &["pic:CT_PictureNonVisual"]),
    ]),
    el("pic:nvPicPr", "pic:CT_PictureNonVisual", &[
        va("pic:CT_PictureNonVisual", &["pic:CT_Picture"]),
    ]),
    el("pic:pic", "pic:CT_Picture", &[
        va("pic:CT_Picture", &["wp:CT_WordprocessingCanvas", "wp:CT_WordprocessingGroup"]),
    ]),
    el("pic:spPr", "a:CT_ShapeProperties", &[
        va("a:CT_ShapeProperties", &["pic:CT_Picture"]),
    ]),
    el("w:abstractNum", "w:CT_AbstractNum", &[
        va("w:CT_AbstractNum", &["w:CT_Numbering"]),
    ]),
    el("w:abstractNumId", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_Num"]),
    ]),
    el("w:active", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_RecipientData"]),
    ]),
    el("w:activeRecord", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_MailMerge"]),
    ]),
    el("w:activeWritingStyle", "w:CT_WritingStyle", &[
        va("w:CT_WritingStyle", &["w:CT_Settings"]),
    ]),
    el("w:addressFieldName", "w:CT_String", &[
        va("w:CT_String", &["w:CT_MailMerge"]),
    ]),
    el("w:adjustLineHeightInTable", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Compat"]),
    ]),
    el("w:adjustRightInd", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:alias", "w:CT_String", &[
        va("w:CT_String", &["w:CT_SdtPr"]),
    ]),
    el("w:aliases", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Style"]),
    ]),
    el("w:alignBordersAndEdges", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:allowPNG", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_WebSettings"]),
    ]),
    el("w:altChunk", "w:CT_AltChunk", &[
        va("w:CT_AltChunk", &["w:CT_Body", "w:CT_Comment", "w:CT_FtnEdn", "w:CT_Tc"]),
    ]),
    el("w:altChunkPr", "w:CT_AltChunkPr", &[
        va("w:CT_AltChunkPr", &["w:CT_AltChunk"]),
    ]),
    el("w:altName", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Font"]),
    ]),
    el("w:alwaysMergeEmptyNamespace", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:alwaysShowPlaceholderText", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:annotationRef", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:applyBreakingRules", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Compat"]),
    ]),
    el("w:attachedSchema", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Settings"]),
    ]),
    el("w:attachedTemplate", "w:CT_Rel", &[
        va("w:CT_Rel", &["w:CT_Settings"]),
    ]),
    el("w:attr", "w:CT_Attr", &[
        va("w:CT_Attr", &["w:CT_CustomXmlPr", "w:CT_SmartTagPr"]),
    ]),
    el("w:autoCaption", "w:CT_AutoCaption", &[
        va("w:CT_AutoCaption", &["w:CT_AutoCaptions"]),
    ]),
    el("w:autoCaptions", "w:CT_AutoCaptions", &[
        va("w:CT_AutoCaptions", &["w:CT_Captions"]),
    ]),
    el("w:autoFormatOverride", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:autoHyphenation", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:autoRedefine", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Style"]),
    ]),
    el("w:autoSpaceDE", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:autoSpaceDN", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:b", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:bCs", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:background", "w:CT_Background", &[
        va("w:CT_Background", &["w:CT_DocumentBase"]),
    ]),
    el("w:balanceSingleByteDoubleByteWidth", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Compat"]),
    ]),
    el("w:bar", "w:CT_Border", &[
        va("w:CT_Border", &["w:CT_PBdr"]),
    ]),
    el("w:basedOn", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Style"]),
    ]),
    el("w:bdo", "w:CT_BdoContentRun", &[
        va("w:CT_BdoContentRun", &["w:CT_CustomXmlRun", "w:CT_P", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun"]),
    ]),
    el("w:bdr", "w:CT_Border", &[
        va("w:CT_Border", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:behavior", "w:CT_DocPartBehavior", &[
        va("w:CT_DocPartBehavior", &["w:CT_DocPartBehaviors"]),
    ]),
    el("w:behaviors", "w:CT_DocPartBehaviors", &[
        va("w:CT_DocPartBehaviors", &["w:CT_DocPartPr"]),
    ]),
    el("w:between", "w:CT_Border", &[
        va("w:CT_Border", &["w:CT_PBdr"]),
    ]),
    el("w:bibliography", "w:CT_Empty", &[
        va("w:CT_Empty", &["w:CT_SdtPr"]),
    ]),
    el("w:bidi", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase", "w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:bidiVisual", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_TblPrBase"]),
    ]),
    el("w:blockQuote", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Div"]),
    ]),
    el("w:body", "w:CT_Body", &[
        va("w:CT_Body", &["w:CT_Document"]),
    ]),
    el("w:bodyDiv", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Div"]),
    ]),
    el("w:bookFoldPrinting", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:bookFoldPrintingSheets", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_Settings"]),
    ]),
    el("w:bookFoldRevPrinting", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:bookmarkEnd", "w:CT_MarkupRange", &[
        va("w:CT_MarkupRange", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:bookmarkStart", "w:CT_Bookmark", &[
        va("w:CT_Bookmark", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:bordersDoNotSurroundFooter", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:bordersDoNotSurroundHeader", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:bottom", "w:CT_Border", &[
        va("w:CT_Border", &["w:CT_DivBdr", "w:CT_PBdr", "w:CT_TblBorders", "w:CT_TcBorders"]),
        va("w:CT_BottomPageBorder", &["w:CT_PageBorders"]),
        va("w:CT_TblWidth", &["w:CT_TblCellMar", "w:CT_TcMar"]),
    ]),
    el("w:br", "w:CT_Br", &[
        va("w:CT_Br", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:calcOnExit", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_FFData"]),
    ]),
    el("w:calendar", "w:CT_CalendarType", &[
        va("w:CT_CalendarType", &["w:CT_SdtDate"]),
    ]),
    el("w:cantSplit", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_TrPrBase"]),
    ]),
    el("w:caps", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:caption", "w:CT_Caption", &[
        va("w:CT_Caption", &["w:CT_Captions"]),
    ]),
    el("w:captions", "w:CT_Captions", &[
        va("w:CT_Captions", &["w:CT_Settings"]),
    ]),
    el("w:category", "w:CT_DocPartCategory", &[
        va("w:CT_DocPartCategory", &["w:CT_DocPartPr"]),
    ]),
    el("w:cellDel", "w:CT_TrackChange", &[
        va("w:CT_TrackChange", &["w:CT_TcPr", "w:CT_TcPrInner"]),
    ]),
    el("w:cellIns", "w:CT_TrackChange", &[
        va("w:CT_TrackChange", &["w:CT_TcPr", "w:CT_TcPrInner"]),
    ]),
    el("w:cellMerge", "w:CT_CellMergeTrackChange", &[
        va("w:CT_CellMergeTrackChange", &["w:CT_TcPr", "w:CT_TcPrInner"]),
    ]),
    el("w:characterSpacingControl", "w:CT_CharacterSpacing", &[
        va("w:CT_CharacterSpacing", &["w:CT_Settings"]),
    ]),
    el("w:charset", "w:CT_Charset", &[
        va("w:CT_Charset", &["w:CT_Font"]),
    ]),
    el("w:checkBox", "w:CT_FFCheckBox", &[
        va("w:CT_FFCheckBox", &["w:CT_FFData"]),
    ]),
    el("w:checkErrors", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_MailMerge"]),
    ]),
    el("w:checked", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_FFCheckBox"]),
    ]),
    el("w:citation", "w:CT_Empty", &[
        va("w:CT_Empty", &["w:CT_SdtPr"]),
    ]),
    el("w:clickAndTypeStyle", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Settings"]),
    ]),
    el("w:clrSchemeMapping", "w:CT_ColorSchemeMapping", &[
        va("w:CT_ColorSchemeMapping", &["w:CT_Settings"]),
    ]),
    el("w:cnfStyle", "w:CT_Cnf", &[
        va("w:CT_Cnf", &["w:CT_PPrBase", "w:CT_TcPrBase", "w:CT_TrPrBase"]),
    ]),
    el("w:col", "w:CT_Column", &[
        va("w:CT_Column", &["w:CT_Columns"]),
    ]),
    el("w:colDelim", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_Odso"]),
    ]),
    el("w:color", "w:CT_Color", &[
        va("w:CT_Color", &["w:CT_FramesetSplitbar", "w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:cols", "w:CT_Columns", &[
        va("w:CT_Columns", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:column", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_OdsoFieldMapData", "w:CT_RecipientData"]),
    ]),
    el("w:comboBox", "w:CT_SdtComboBox", &[
        va("w:CT_SdtComboBox", &["w:CT_SdtPr"]),
    ]),
    el("w:comment", "w:CT_Comment", &[
        va("w:CT_Comment", &["w:CT_Comments"]),
    ]),
    el("w:commentRangeEnd", "w:CT_MarkupRange", &[
        va("w:CT_MarkupRange", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:commentRangeStart", "w:CT_MarkupRange", &[
        va("w:CT_MarkupRange", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:commentReference", "w:CT_Markup", &[
        va("w:CT_Markup", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:comments", "w:CT_Comments", &[
        va("w:CT_Comments", &[]),
    ]),
    el("w:compat", "w:CT_Compat", &[
        va("w:CT_Compat", &["w:CT_Settings"]),
    ]),
    el("w:compatSetting", "w:CT_CompatSetting", &[
        va("w:CT_CompatSetting", &["w:CT_Compat"]),
    ]),
    el("w:connectString", "w:CT_String", &[
        va("w:CT_String", &["w:CT_MailMerge"]),
    ]),
    el("w:consecutiveHyphenLimit", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_Settings"]),
    ]),
    el("w:contentPart", "w:CT_Rel", &[
        va("w:CT_Rel", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:contextualSpacing", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:continuationSeparator", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:control", "w:CT_Control", &[
        va("w:CT_Control", &["w:CT_Object"]),
    ]),
    el("w:cr", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:cs", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:customXml", "w:CT_CustomXmlRun", &[
        va("w:CT_CustomXmlBlock", &["w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_FtnEdn", "w:CT_Tc"]),
        va("w:CT_CustomXmlCell", &["w:CT_CustomXmlCell", "w:CT_Row"]),
        va("w:CT_CustomXmlRow", &["w:CT_CustomXmlRow", "w:CT_Tbl"]),
        va("w:CT_CustomXmlRun", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_CustomXmlRun", "w:CT_P", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun"]),
    ]),
    el("w:customXmlDelRangeEnd", "w:CT_Markup", &[
        va("w:CT_Markup", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:customXmlDelRangeStart", "w:CT_TrackChange", &[
        va("w:CT_TrackChange", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:customXmlInsRangeEnd", "w:CT_Markup", &[
        va("w:CT_Markup", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:customXmlInsRangeStart", "w:CT_TrackChange", &[
        va("w:CT_TrackChange", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:customXmlMoveFromRangeEnd", "w:CT_Markup", &[
        va("w:CT_Markup", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:customXmlMoveFromRangeStart", "w:CT_TrackChange", &[
        va("w:CT_TrackChange", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:customXmlMoveToRangeEnd", "w:CT_Markup", &[
        va("w:CT_Markup", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:customXmlMoveToRangeStart", "w:CT_TrackChange", &[
        va("w:CT_TrackChange", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:customXmlPr", "w:CT_CustomXmlPr", &[
        va("w:CT_CustomXmlPr", &["w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun"]),
    ]),
    el("w:dataBinding", "w:CT_DataBinding", &[
        va("w:CT_DataBinding", &["w:CT_SdtPr"]),
    ]),
    el("w:dataSource", "w:CT_Rel", &[
        va("w:CT_Rel", &["w:CT_MailMerge"]),
    ]),
    el("w:dataType", "w:CT_MailMergeDataType", &[
        va("w:CT_MailMergeDataType", &["w:CT_MailMerge"]),
    ]),
    el("w:date", "w:CT_SdtDate", &[
        va("w:CT_SdtDate", &["w:CT_SdtPr"]),
    ]),
    el("w:dateFormat", "w:CT_String", &[
        va("w:CT_String", &["w:CT_SdtDate"]),
    ]),
    el("w:dayLong", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:dayShort", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:ddList", "w:CT_FFDDList", &[
        va("w:CT_FFDDList", &["w:CT_FFData"]),
    ]),
    el("w:decimalSymbol", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Settings"]),
    ]),
    el("w:default", "w:CT_OnOff", &[
        va("w:CT_DecimalNumber", &["w:CT_FFDDList"]),
        va("w:CT_OnOff", &["w:CT_FFCheckBox"]),
        va("w:CT_String", &["w:CT_FFTextInput"]),
    ]),
    el("w:defaultTabStop", "w:CT_TwipsMeasure", &[
        va("w:CT_TwipsMeasure", &["w:CT_Settings"]),
    ]),
    el("w:defaultTableStyle", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Settings"]),
    ]),
    el("w:del", "w:CT_TrackChange", &[
        va("w:CT_MathCtrlDel", &["m:CT_CtrlPr"]),
        va("w:CT_RPrChange", &["w:CT_MathCtrlIns"]),
        va("w:CT_RunTrackChange", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
        va("w:CT_TrackChange", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_TrPr"]),
    ]),
    el("w:delInstrText", "w:CT_Text", &[
        va("w:CT_Text", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:delText", "w:CT_Text", &[
        va("w:CT_Text", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:description", "w:CT_String", &[
        va("w:CT_String", &["w:CT_DocPartPr"]),
    ]),
    el("w:destination", "w:CT_MailMergeDest", &[
        va("w:CT_MailMergeDest", &["w:CT_MailMerge"]),
    ]),
    el("w:dir", "w:CT_DirContentRun", &[
        va("w:CT_DirContentRun", &["w:CT_CustomXmlRun", "w:CT_P", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun"]),
    ]),
    el("w:dirty", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_RubyPr"]),
    ]),
    el("w:displayBackgroundShape", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:displayHorizontalDrawingGridEvery", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_Settings"]),
    ]),
    el("w:displayVerticalDrawingGridEvery", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_Settings"]),
    ]),
    el("w:div", "w:CT_Div", &[
        va("w:CT_Div", &["w:CT_Divs"]),
    ]),
    el("w:divBdr", "w:CT_DivBdr", &[
        va("w:CT_DivBdr", &["w:CT_Div"]),
    ]),
    el("w:divId", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_PPrBase", "w:CT_TrPrBase"]),
    ]),
    el("w:divs", "w:CT_Divs", &[
        va("w:CT_Divs", &["w:CT_WebSettings"]),
    ]),
    el("w:divsChild", "w:CT_Divs", &[
        va("w:CT_Divs", &["w:CT_Div"]),
    ]),
    el("w:doNotAutoCompressPictures", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:doNotDemarcateInvalidXml", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:doNotDisplayPageBoundaries", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:doNotEmbedSmartTags", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:doNotExpandShiftReturn", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Compat"]),
    ]),
    el("w:doNotHyphenateCaps", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:doNotIncludeSubdocsInStats", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:doNotLeaveBackslashAlone", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Compat"]),
    ]),
    el("w:doNotOrganizeInFolder", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_WebSettings"]),
    ]),
    el("w:doNotRelyOnCSS", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_WebSettings"]),
    ]),
    el("w:doNotSaveAsSingleFile", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_WebSettings"]),
    ]),
    el("w:doNotShadeFormData", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:doNotSuppressBlankLines", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_MailMerge"]),
    ]),
    el("w:doNotTrackFormatting", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:doNotTrackMoves", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:doNotUseLongFileNames", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_WebSettings"]),
    ]),
    el("w:doNotUseMarginsForDrawingGridOrigin", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:doNotValidateAgainstSchema", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:docDefaults", "w:CT_DocDefaults", &[
        va("w:CT_DocDefaults", &["w:CT_Styles"]),
    ]),
    el("w:docGrid", "w:CT_DocGrid", &[
        va("w:CT_DocGrid", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:docPart", "w:CT_String", &[
        va("w:CT_DocPart", &["w:CT_DocParts"]),
        va("w:CT_String", &["w:CT_Placeholder"]),
    ]),
    el("w:docPartBody", "w:CT_Body", &[
        va("w:CT_Body", &["w:CT_DocPart"]),
    ]),
    el("w:docPartCategory", "w:CT_String", &[
        va("w:CT_String", &["w:CT_SdtDocPart"]),
    ]),
    el("w:docPartGallery", "w:CT_String", &[
        va("w:CT_String", &["w:CT_SdtDocPart"]),
    ]),
    el("w:docPartList", "w:CT_SdtDocPart", &[
        va("w:CT_SdtDocPart", &["w:CT_SdtPr"]),
    ]),
    el("w:docPartObj", "w:CT_SdtDocPart", &[
        va("w:CT_SdtDocPart", &["w:CT_SdtPr"]),
    ]),
    el("w:docPartPr", "w:CT_DocPartPr", &[
        va("w:CT_DocPartPr", &["w:CT_DocPart"]),
    ]),
    el("w:docPartUnique", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_SdtDocPart"]),
    ]),
    el("w:docParts", "w:CT_DocParts", &[
        va("w:CT_DocParts", &["w:CT_GlossaryDocument"]),
    ]),
    el("w:docVar", "w:CT_DocVar", &[
        va("w:CT_DocVar", &["w:CT_DocVars"]),
    ]),
    el("w:docVars", "w:CT_DocVars", &[
        va("w:CT_DocVars", &["w:CT_Settings"]),
    ]),
    el("w:document", "w:CT_Document", &[
        va("w:CT_Document", &[]),
    ]),
    el("w:documentProtection", "w:CT_DocProtect", &[
        va("w:CT_DocProtect", &["w:CT_Settings"]),
    ]),
    el("w:documentType", "w:CT_DocType", &[
        va("w:CT_DocType", &["w:CT_Settings"]),
    ]),
    el("w:drawing", "w:CT_Drawing", &[
        va("w:CT_Drawing", &["m:CT_R", "w:CT_Background", "w:CT_NumPicBullet", "w:CT_Object", "w:CT_R"]),
    ]),
    el("w:drawingGridHorizontalOrigin", "w:CT_TwipsMeasure", &[
        va("w:CT_TwipsMeasure", &["w:CT_Settings"]),
    ]),
    el("w:drawingGridHorizontalSpacing", "w:CT_TwipsMeasure", &[
        va("w:CT_TwipsMeasure", &["w:CT_Settings"]),
    ]),
    el("w:drawingGridVerticalOrigin", "w:CT_TwipsMeasure", &[
        va("w:CT_TwipsMeasure", &["w:CT_Settings"]),
    ]),
    el("w:drawingGridVerticalSpacing", "w:CT_TwipsMeasure", &[
        va("w:CT_TwipsMeasure", &["w:CT_Settings"]),
    ]),
    el("w:dropDownList", "w:CT_SdtDropDownList", &[
        va("w:CT_SdtDropDownList", &["w:CT_SdtPr"]),
    ]),
    el("w:dstrike", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:dynamicAddress", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_OdsoFieldMapData"]),
    ]),
    el("w:eastAsianLayout", "w:CT_EastAsianLayout", &[
        va("w:CT_EastAsianLayout", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:effect", "w:CT_TextEffect", &[
        va("w:CT_TextEffect", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:em", "w:CT_Em", &[
        va("w:CT_Em", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:embedBold", "w:CT_FontRel", &[
        va("w:CT_FontRel", &["w:CT_Font"]),
    ]),
    el("w:embedBoldItalic", "w:CT_FontRel", &[
        va("w:CT_FontRel", &["w:CT_Font"]),
    ]),
    el("w:embedItalic", "w:CT_FontRel", &[
        va("w:CT_FontRel", &["w:CT_Font"]),
    ]),
    el("w:embedRegular", "w:CT_FontRel", &[
        va("w:CT_FontRel", &["w:CT_Font"]),
    ]),
    el("w:embedSystemFonts", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:embedTrueTypeFonts", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:emboss", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:enabled", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_FFData"]),
    ]),
    el("w:encoding", "w:CT_String", &[
        va("w:CT_String", &["w:CT_WebSettings"]),
    ]),
    el("w:end", "w:CT_Border", &[
        va("w:CT_Border", &["w:CT_TblBorders", "w:CT_TcBorders"]),
        va("w:CT_TblWidth", &["w:CT_TblCellMar", "w:CT_TcMar"]),
    ]),
    el("w:endnote", "w:CT_FtnEdnSepRef", &[
        va("w:CT_FtnEdn", &["w:CT_Endnotes"]),
        va("w:CT_FtnEdnSepRef", &["w:CT_EdnDocProps"]),
    ]),
    el("w:endnotePr", "w:CT_EdnProps", &[
        va("w:CT_EdnDocProps", &["w:CT_Settings"]),
        va("w:CT_EdnProps", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:endnoteRef", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:endnoteReference", "w:CT_FtnEdnRef", &[
        va("w:CT_FtnEdnRef", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:endnotes", "w:CT_Endnotes", &[
        va("w:CT_Endnotes", &[]),
    ]),
    el("w:entryMacro", "w:CT_MacroName", &[
        va("w:CT_MacroName", &["w:CT_FFData"]),
    ]),
    el("w:equation", "w:CT_Empty", &[
        va("w:CT_Empty", &["w:CT_SdtPr"]),
    ]),
    el("w:evenAndOddHeaders", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:exitMacro", "w:CT_MacroName", &[
        va("w:CT_MacroName", &["w:CT_FFData"]),
    ]),
    el("w:fHdr", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Odso"]),
    ]),
    el("w:family", "w:CT_FontFamily", &[
        va("w:CT_FontFamily", &["w:CT_Font"]),
    ]),
    el("w:ffData", "w:CT_FFData", &[
        va("w:CT_FFData", &["w:CT_FldChar"]),
    ]),
    el("w:fieldMapData", "w:CT_OdsoFieldMapData", &[
        va("w:CT_OdsoFieldMapData", &["w:CT_Odso"]),
    ]),
    el("w:fitText", "w:CT_FitText", &[
        va("w:CT_FitText", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:flatBorders", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_FramesetSplitbar"]),
    ]),
    el("w:fldChar", "w:CT_FldChar", &[
        va("w:CT_FldChar", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:fldSimple", "w:CT_SimpleField", &[
        va("w:CT_SimpleField", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_CustomXmlRun", "w:CT_P", "w:CT_SimpleField", "w:CT_SmartTagRun"]),
    ]),
    el("w:font", "w:CT_Font", &[
        va("w:CT_Font", &["w:CT_FontsList"]),
    ]),
    el("w:fonts", "w:CT_FontsList", &[
        va("w:CT_FontsList", &[]),
    ]),
    el("w:footerReference", "w:CT_HdrFtrRef", &[
        va("w:CT_HdrFtrRef", &["w:CT_SectPr"]),
    ]),
    el("w:footnote", "w:CT_FtnEdnSepRef", &[
        va("w:CT_FtnEdn", &["w:CT_Footnotes"]),
        va("w:CT_FtnEdnSepRef", &["w:CT_FtnDocProps"]),
    ]),
    el("w:footnotePr", "w:CT_FtnProps", &[
        va("w:CT_FtnDocProps", &["w:CT_Settings"]),
        va("w:CT_FtnProps", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:footnoteRef", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:footnoteReference", "w:CT_FtnEdnRef", &[
        va("w:CT_FtnEdnRef", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:footnotes", "w:CT_Footnotes", &[
        va("w:CT_Footnotes", &[]),
    ]),
    el("w:forceUpgrade", "w:CT_Empty", &[
        va("w:CT_Empty", &["w:CT_Settings"]),
    ]),
    el("w:formProt", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:format", "w:CT_String", &[
        va("w:CT_String", &["w:CT_FFTextInput"]),
    ]),
    el("w:formsDesign", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:frame", "w:CT_Frame", &[
        va("w:CT_Frame", &["w:CT_Frameset"]),
    ]),
    el("w:frameLayout", "w:CT_FrameLayout", &[
        va("w:CT_FrameLayout", &["w:CT_Frameset"]),
    ]),
    el("w:framePr", "w:CT_FramePr", &[
        va("w:CT_FramePr", &["w:CT_PPrBase"]),
    ]),
    el("w:frameset", "w:CT_Frameset", &[
        va("w:CT_Frameset", &["w:CT_Frameset", "w:CT_WebSettings"]),
    ]),
    el("w:framesetSplitbar", "w:CT_FramesetSplitbar", &[
        va("w:CT_FramesetSplitbar", &["w:CT_Frameset"]),
    ]),
    el("w:ftr", "w:CT_HdrFtr", &[
        va("w:CT_HdrFtr", &[]),
    ]),
    el("w:gallery", "w:CT_DocPartGallery", &[
        va("w:CT_DocPartGallery", &["w:CT_DocPartCategory"]),
    ]),
    el("w:glossaryDocument", "w:CT_GlossaryDocument", &[
        va("w:CT_GlossaryDocument", &[]),
    ]),
    el("w:gridAfter", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_TrPrBase"]),
    ]),
    el("w:gridBefore", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_TrPrBase"]),
    ]),
    el("w:gridCol", "w:CT_TblGridCol", &[
        va("w:CT_TblGridCol", &["w:CT_TblGridBase"]),
    ]),
    el("w:gridSpan", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_TcPrBase"]),
    ]),
    el("w:group", "w:CT_Empty", &[
        va("w:CT_Empty", &["w:CT_SdtPr"]),
    ]),
    el("w:guid", "w:CT_Guid", &[
        va("w:CT_Guid", &["w:CT_DocPartPr"]),
    ]),
    el("w:gutterAtTop", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:hdr", "w:CT_HdrFtr", &[
        va("w:CT_HdrFtr", &[]),
    ]),
    el("w:header", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Headers"]),
    ]),
    el("w:headerReference", "w:CT_HdrFtrRef", &[
        va("w:CT_HdrFtrRef", &["w:CT_SectPr"]),
    ]),
    el("w:headerSource", "w:CT_Rel", &[
        va("w:CT_Rel", &["w:CT_MailMerge"]),
    ]),
    el("w:headers", "w:CT_Headers", &[
        va("w:CT_Headers", &["w:CT_TcPrBase"]),
    ]),
    el("w:helpText", "w:CT_FFHelpText", &[
        va("w:CT_FFHelpText", &["w:CT_FFData"]),
    ]),
    el("w:hidden", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Style", "w:CT_TrPrBase"]),
    ]),
    el("w:hideGrammaticalErrors", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:hideMark", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_TcPrBase"]),
    ]),
    el("w:hideSpellingErrors", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:highlight", "w:CT_Highlight", &[
        va("w:CT_Highlight", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:hps", "w:CT_HpsMeasure", &[
        va("w:CT_HpsMeasure", &["w:CT_RubyPr"]),
    ]),
    el("w:hpsBaseText", "w:CT_HpsMeasure", &[
        va("w:CT_HpsMeasure", &["w:CT_RubyPr"]),
    ]),
    el("w:hpsRaise", "w:CT_HpsMeasure", &[
        va("w:CT_HpsMeasure", &["w:CT_RubyPr"]),
    ]),
    el("w:hyperlink", "w:CT_Hyperlink", &[
        va("w:CT_Hyperlink", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_CustomXmlRun", "w:CT_P", "w:CT_SimpleField", "w:CT_SmartTagRun"]),
    ]),
    el("w:hyphenationZone", "w:CT_TwipsMeasure", &[
        va("w:CT_TwipsMeasure", &["w:CT_Settings"]),
    ]),
    el("w:i", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:iCs", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:id", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_SdtPr"]),
    ]),
    el("w:ignoreMixedContent", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:ilvl", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_NumPr"]),
    ]),
    el("w:imprint", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:ind", "w:CT_Ind", &[
        va("w:CT_Ind", &["w:CT_PPrBase"]),
    ]),
    el("w:ins", "w:CT_TrackChange", &[
        va("w:CT_MathCtrlIns", &["m:CT_CtrlPr"]),
        va("w:CT_RunTrackChange", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
        va("w:CT_TrackChange", &["w:CT_NumPr", "w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_TrPr"]),
    ]),
    el("w:insideH", "w:CT_Border", &[
        va("w:CT_Border", &["w:CT_TblBorders", "w:CT_TcBorders"]),
    ]),
    el("w:insideV", "w:CT_Border", &[
        va("w:CT_Border", &["w:CT_TblBorders", "w:CT_TcBorders"]),
    ]),
    el("w:instrText", "w:CT_Text", &[
        va("w:CT_Text", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:isLgl", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Lvl"]),
    ]),
    el("w:jc", "w:CT_JcTable", &[
        va("w:CT_Jc", &["w:CT_PPrBase"]),
        va("w:CT_JcTable", &["w:CT_TblPrBase", "w:CT_TblPrExBase", "w:CT_TrPrBase"]),
    ]),
    el("w:keepLines", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:keepNext", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:kern", "w:CT_HpsMeasure", &[
        va("w:CT_HpsMeasure", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:kinsoku", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:label", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_FFData", "w:CT_SdtPr"]),
    ]),
    el("w:lang", "w:CT_Language", &[
        va("w:CT_Language", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:lastRenderedPageBreak", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:latentStyles", "w:CT_LatentStyles", &[
        va("w:CT_LatentStyles", &["w:CT_Styles"]),
    ]),
    el("w:left", "w:CT_Border", &[
        va("w:CT_Border", &["w:CT_DivBdr", "w:CT_PBdr"]),
        va("w:CT_PageBorder", &["w:CT_PageBorders"]),
    ]),
    el("w:lid", "w:CT_Lang", &[
        va("w:CT_Lang", &["w:CT_OdsoFieldMapData", "w:CT_RubyPr", "w:CT_SdtDate"]),
    ]),
    el("w:link", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Style"]),
    ]),
    el("w:linkStyles", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:linkToQuery", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_MailMerge"]),
    ]),
    el("w:linkedToFile", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Frame"]),
    ]),
    el("w:listEntry", "w:CT_String", &[
        va("w:CT_String", &["w:CT_FFDDList"]),
    ]),
    el("w:listItem", "w:CT_SdtListItem", &[
        va("w:CT_SdtListItem", &["w:CT_SdtComboBox", "w:CT_SdtDropDownList"]),
    ]),
    el("w:listSeparator", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Settings"]),
    ]),
    el("w:lnNumType", "w:CT_LineNumber", &[
        va("w:CT_LineNumber", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:lock", "w:CT_Lock", &[
        va("w:CT_Lock", &["w:CT_SdtPr"]),
    ]),
    el("w:locked", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Style"]),
    ]),
    el("w:longDesc", "w:CT_Rel", &[
        va("w:CT_Rel", &["w:CT_Frame"]),
    ]),
    el("w:lsdException", "w:CT_LsdException", &[
        va("w:CT_LsdException", &["w:CT_LatentStyles"]),
    ]),
    el("w:lvl", "w:CT_Lvl", &[
        va("w:CT_Lvl", &["w:CT_AbstractNum", "w:CT_NumLvl"]),
    ]),
    el("w:lvlJc", "w:CT_Jc", &[
        va("w:CT_Jc", &["w:CT_Lvl"]),
    ]),
    el("w:lvlOverride", "w:CT_NumLvl", &[
        va("w:CT_NumLvl", &["w:CT_Num"]),
    ]),
    el("w:lvlPicBulletId", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_Lvl"]),
    ]),
    el("w:lvlRestart", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_Lvl"]),
    ]),
    el("w:lvlText", "w:CT_LevelText", &[
        va("w:CT_LevelText", &["w:CT_Lvl"]),
    ]),
    el("w:mailAsAttachment", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_MailMerge"]),
    ]),
    el("w:mailMerge", "w:CT_MailMerge", &[
        va("w:CT_MailMerge", &["w:CT_Settings"]),
    ]),
    el("w:mailSubject", "w:CT_String", &[
        va("w:CT_String", &["w:CT_MailMerge"]),
    ]),
    el("w:mainDocumentType", "w:CT_MailMergeDocType", &[
        va("w:CT_MailMergeDocType", &["w:CT_MailMerge"]),
    ]),
    el("w:mappedName", "w:CT_String", &[
        va("w:CT_String", &["w:CT_OdsoFieldMapData"]),
    ]),
    el("w:marBottom", "w:CT_SignedTwipsMeasure", &[
        va("w:CT_SignedTwipsMeasure", &["w:CT_Div"]),
    ]),
    el("w:marH", "w:CT_PixelsMeasure", &[
        va("w:CT_PixelsMeasure", &["w:CT_Frame"]),
    ]),
    el("w:marLeft", "w:CT_SignedTwipsMeasure", &[
        va("w:CT_SignedTwipsMeasure", &["w:CT_Div"]),
    ]),
    el("w:marRight", "w:CT_SignedTwipsMeasure", &[
        va("w:CT_SignedTwipsMeasure", &["w:CT_Div"]),
    ]),
    el("w:marTop", "w:CT_SignedTwipsMeasure", &[
        va("w:CT_SignedTwipsMeasure", &["w:CT_Div"]),
    ]),
    el("w:marW", "w:CT_PixelsMeasure", &[
        va("w:CT_PixelsMeasure", &["w:CT_Frame"]),
    ]),
    el("w:matchSrc", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_AltChunkPr"]),
    ]),
    el("w:maxLength", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_FFTextInput"]),
    ]),
    el("w:mirrorIndents", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:mirrorMargins", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:monthLong", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:monthShort", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:moveFrom", "w:CT_TrackChange", &[
        va("w:CT_RunTrackChange", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
        va("w:CT_TrackChange", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal"]),
    ]),
    el("w:moveFromRangeEnd", "w:CT_MarkupRange", &[
        va("w:CT_MarkupRange", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:moveFromRangeStart", "w:CT_MoveBookmark", &[
        va("w:CT_MoveBookmark", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:moveTo", "w:CT_TrackChange", &[
        va("w:CT_RunTrackChange", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
        va("w:CT_TrackChange", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal"]),
    ]),
    el("w:moveToRangeEnd", "w:CT_MarkupRange", &[
        va("w:CT_MarkupRange", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:moveToRangeStart", "w:CT_MoveBookmark", &[
        va("w:CT_MoveBookmark", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:movie", "w:CT_Rel", &[
        va("w:CT_Rel", &["w:CT_Object"]),
    ]),
    el("w:multiLevelType", "w:CT_MultiLevelType", &[
        va("w:CT_MultiLevelType", &["w:CT_AbstractNum"]),
    ]),
    el("w:name", "w:CT_String", &[
        va("w:CT_DocPartName", &["w:CT_DocPartPr"]),
        va("w:CT_FFName", &["w:CT_FFData"]),
        va("w:CT_String", &["w:CT_AbstractNum", "w:CT_DocPartCategory", "w:CT_Frame", "w:CT_OdsoFieldMapData", "w:CT_Style"]),
    ]),
    el("w:next", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Style"]),
    ]),
    el("w:noBorder", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_FramesetSplitbar"]),
    ]),
    el("w:noBreakHyphen", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:noEndnote", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:noLineBreaksAfter", "w:CT_Kinsoku", &[
        va("w:CT_Kinsoku", &["w:CT_Settings"]),
    ]),
    el("w:noLineBreaksBefore", "w:CT_Kinsoku", &[
        va("w:CT_Kinsoku", &["w:CT_Settings"]),
    ]),
    el("w:noProof", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:noPunctuationKerning", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:noResizeAllowed", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Frame"]),
    ]),
    el("w:noWrap", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_TcPrBase"]),
    ]),
    el("w:notTrueType", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Font"]),
    ]),
    el("w:nsid", "w:CT_LongHexNumber", &[
        va("w:CT_LongHexNumber", &["w:CT_AbstractNum"]),
    ]),
    el("w:num", "w:CT_Num", &[
        va("w:CT_Num", &["w:CT_Numbering"]),
    ]),
    el("w:numFmt", "w:CT_NumFmt", &[
        va("w:CT_NumFmt", &["w:CT_EdnProps", "w:CT_FtnProps", "w:CT_Lvl"]),
    ]),
    el("w:numId", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_NumPr"]),
    ]),
    el("w:numIdMacAtCleanup", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_Numbering"]),
    ]),
    el("w:numPicBullet", "w:CT_NumPicBullet", &[
        va("w:CT_NumPicBullet", &["w:CT_Numbering"]),
    ]),
    el("w:numPr", "w:CT_NumPr", &[
        va("w:CT_NumPr", &["w:CT_PPrBase"]),
    ]),
    el("w:numRestart", "w:CT_NumRestart", &[
        va("w:CT_NumRestart", &["w:CT_EdnDocProps", "w:CT_EdnProps", "w:CT_FtnDocProps", "w:CT_FtnProps"]),
    ]),
    el("w:numStart", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_EdnDocProps", "w:CT_EdnProps", "w:CT_FtnDocProps", "w:CT_FtnProps"]),
    ]),
    el("w:numStyleLink", "w:CT_String", &[
        va("w:CT_String", &["w:CT_AbstractNum"]),
    ]),
    el("w:numbering", "w:CT_Numbering", &[
        va("w:CT_Numbering", &[]),
    ]),
    el("w:oMath", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:object", "w:CT_Object", &[
        va("w:CT_Object", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:objectEmbed", "w:CT_ObjectEmbed", &[
        va("w:CT_ObjectEmbed", &["w:CT_Object"]),
    ]),
    el("w:objectLink", "w:CT_ObjectLink", &[
        va("w:CT_ObjectLink", &["w:CT_Object"]),
    ]),
    el("w:odso", "w:CT_Odso", &[
        va("w:CT_Odso", &["w:CT_MailMerge"]),
    ]),
    el("w:optimizeForBrowser", "w:CT_OptimizeForBrowser", &[
        va("w:CT_OptimizeForBrowser", &["w:CT_WebSettings"]),
    ]),
    el("w:outline", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:outlineLvl", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_PPrBase"]),
    ]),
    el("w:overflowPunct", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:p", "w:CT_P", &[
        va("w:CT_P", &["w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_FtnEdn", "w:CT_Tc"]),
    ]),
    el("w:pBdr", "w:CT_PBdr", &[
        va("w:CT_PBdr", &["w:CT_PPrBase"]),
    ]),
    el("w:pPr", "w:CT_PPrGeneral", &[
        va("w:CT_PPr", &["w:CT_P"]),
        va("w:CT_PPrBase", &["w:CT_PPrChange"]),
        va("w:CT_PPrGeneral", &["w:CT_Lvl", "w:CT_PPrDefault", "w:CT_Style", "w:CT_TblStylePr"]),
    ]),
    el("w:pPrChange", "w:CT_PPrChange", &[
        va("w:CT_PPrChange", &["w:CT_PPr", "w:CT_PPrGeneral"]),
    ]),
    el("w:pPrDefault", "w:CT_PPrDefault", &[
        va("w:CT_PPrDefault", &["w:CT_DocDefaults"]),
    ]),
    el("w:pStyle", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Lvl", "w:CT_PPrBase"]),
    ]),
    el("w:pageBreakBefore", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:panose1", "w:CT_Panose", &[
        va("w:CT_Panose", &["w:CT_Font"]),
    ]),
    el("w:paperSrc", "w:CT_PaperSource", &[
        va("w:CT_PaperSource", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:permEnd", "w:CT_Perm", &[
        va("w:CT_Perm", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:permStart", "w:CT_PermStart", &[
        va("w:CT_PermStart", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:personal", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Style"]),
    ]),
    el("w:personalCompose", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Style"]),
    ]),
    el("w:personalReply", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Style"]),
    ]),
    el("w:pgBorders", "w:CT_PageBorders", &[
        va("w:CT_PageBorders", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:pgMar", "w:CT_PageMar", &[
        va("w:CT_PageMar", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:pgNum", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:pgNumType", "w:CT_PageNumber", &[
        va("w:CT_PageNumber", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:pgSz", "w:CT_PageSz", &[
        va("w:CT_PageSz", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:picture", "w:CT_Empty", &[
        va("w:CT_Empty", &["w:CT_SdtPr"]),
    ]),
    el("w:pitch", "w:CT_Pitch", &[
        va("w:CT_Pitch", &["w:CT_Font"]),
    ]),
    el("w:pixelsPerInch", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_WebSettings"]),
    ]),
    el("w:placeholder", "w:CT_Placeholder", &[
        va("w:CT_Placeholder", &["w:CT_SdtPr"]),
        va("w:CT_String", &["w:CT_CustomXmlPr"]),
    ]),
    el("w:pos", "w:CT_FtnPos", &[
        va("w:CT_EdnPos", &["w:CT_EdnProps"]),
        va("w:CT_FtnPos", &["w:CT_FtnProps"]),
    ]),
    el("w:position", "w:CT_SignedHpsMeasure", &[
        va("w:CT_SignedHpsMeasure", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:printFormsData", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:printFractionalCharacterWidth", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:printPostScriptOverText", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:printTwoOnOne", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:printerSettings", "w:CT_Rel", &[
        va("w:CT_Rel", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:proofErr", "w:CT_ProofErr", &[
        va("w:CT_ProofErr", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_CustomXmlCell", "w:CT_CustomXmlRow", "w:CT_CustomXmlRun", "w:CT_FtnEdn", "w:CT_P", "w:CT_Row", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun", "w:CT_Tbl", "w:CT_Tc"]),
    ]),
    el("w:proofState", "w:CT_Proof", &[
        va("w:CT_Proof", &["w:CT_Settings"]),
    ]),
    el("w:ptab", "w:CT_PTab", &[
        va("w:CT_PTab", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:qFormat", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Style"]),
    ]),
    el("w:query", "w:CT_String", &[
        va("w:CT_String", &["w:CT_MailMerge"]),
    ]),
    el("w:r", "w:CT_R", &[
        va("w:CT_R", &["w:CT_CustomXmlRun", "w:CT_P", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun"]),
    ]),
    el("w:rFonts", "w:CT_Fonts", &[
        va("w:CT_Fonts", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:rPr", "w:CT_RPr", &[
        va("w:CT_ParaRPr", &["w:CT_PPr"]),
        va("w:CT_ParaRPrOriginal", &["w:CT_ParaRPrChange"]),
        va("w:CT_RPr", &["m:CT_CtrlPr", "m:CT_R", "w:CT_Lvl", "w:CT_MathCtrlDel", "w:CT_MathCtrlIns", "w:CT_R", "w:CT_RPrDefault", "w:CT_SdtEndPr", "w:CT_SdtPr", "w:CT_Style", "w:CT_TblStylePr"]),
        va("w:CT_RPrOriginal", &["w:CT_RPrChange"]),
    ]),
    el("w:rPrChange", "w:CT_RPrChange", &[
        va("w:CT_ParaRPrChange", &["w:CT_ParaRPr"]),
        va("w:CT_RPrChange", &["w:CT_RPr"]),
    ]),
    el("w:rPrDefault", "w:CT_RPrDefault", &[
        va("w:CT_RPrDefault", &["w:CT_DocDefaults"]),
    ]),
    el("w:rStyle", "w:CT_String", &[
        va("w:CT_String", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:readModeInkLockDown", "w:CT_ReadingModeInkLockDown", &[
        va("w:CT_ReadingModeInkLockDown", &["w:CT_Settings"]),
    ]),
    el("w:recipientData", "w:CT_RecipientData", &[
        va("w:CT_RecipientData", &["w:CT_Recipients"]),
        va("w:CT_Rel", &["w:CT_Odso"]),
    ]),
    el("w:recipients", "w:CT_Recipients", &[
        va("w:CT_Recipients", &[]),
    ]),
    el("w:removeDateAndTime", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:removePersonalInformation", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:result", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_FFDDList"]),
    ]),
    el("w:revisionView", "w:CT_TrackChangesView", &[
        va("w:CT_TrackChangesView", &["w:CT_Settings"]),
    ]),
    el("w:richText", "w:CT_Empty", &[
        va("w:CT_Empty", &["w:CT_SdtPr"]),
    ]),
    el("w:right", "w:CT_Border", &[
        va("w:CT_Border", &["w:CT_DivBdr", "w:CT_PBdr"]),
        va("w:CT_PageBorder", &["w:CT_PageBorders"]),
    ]),
    el("w:rsid", "w:CT_LongHexNumber", &[
        va("w:CT_LongHexNumber", &["w:CT_DocRsids", "w:CT_Style"]),
    ]),
    el("w:rsidRoot", "w:CT_LongHexNumber", &[
        va("w:CT_LongHexNumber", &["w:CT_DocRsids"]),
    ]),
    el("w:rsids", "w:CT_DocRsids", &[
        va("w:CT_DocRsids", &["w:CT_Settings"]),
    ]),
    el("w:rt", "w:CT_RubyContent", &[
        va("w:CT_RubyContent", &["w:CT_Ruby"]),
    ]),
    el("w:rtl", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:rtlGutter", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:ruby", "w:CT_Ruby", &[
        va("w:CT_Ruby", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:rubyAlign", "w:CT_RubyAlign", &[
        va("w:CT_RubyAlign", &["w:CT_RubyPr"]),
    ]),
    el("w:rubyBase", "w:CT_RubyContent", &[
        va("w:CT_RubyContent", &["w:CT_Ruby"]),
    ]),
    el("w:rubyPr", "w:CT_RubyPr", &[
        va("w:CT_RubyPr", &["w:CT_Ruby"]),
    ]),
    el("w:saveFormsData", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:saveInvalidXml", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:savePreviewPicture", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:saveSmartTagsAsXml", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_WebSettings"]),
    ]),
    el("w:saveSubsetFonts", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:saveThroughXslt", "w:CT_SaveThroughXslt", &[
        va("w:CT_SaveThroughXslt", &["w:CT_Settings"]),
    ]),
    el("w:saveXmlDataOnly", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:scrollbar", "w:CT_FrameScrollbar", &[
        va("w:CT_FrameScrollbar", &["w:CT_Frame"]),
    ]),
    el("w:sdt", "w:CT_SdtRun", &[
        va("w:CT_SdtBlock", &["w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_FtnEdn", "w:CT_Tc"]),
        va("w:CT_SdtCell", &["w:CT_CustomXmlCell", "w:CT_Row"]),
        va("w:CT_SdtRow", &["w:CT_CustomXmlRow", "w:CT_Tbl"]),
        va("w:CT_SdtRun", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_CustomXmlRun", "w:CT_P", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun"]),
    ]),
    el("w:sdtContent", "w:CT_SdtContentBlock", &[
        va("w:CT_SdtContentBlock", &["w:CT_SdtBlock"]),
        va("w:CT_SdtContentCell", &["w:CT_SdtCell"]),
        va("w:CT_SdtContentRow", &["w:CT_SdtRow"]),
        va("w:CT_SdtContentRun", &["w:CT_SdtRun"]),
    ]),
    el("w:sdtEndPr", "w:CT_SdtEndPr", &[
        va("w:CT_SdtEndPr", &["w:CT_SdtBlock", "w:CT_SdtCell", "w:CT_SdtRow", "w:CT_SdtRun"]),
    ]),
    el("w:sdtPr", "w:CT_SdtPr", &[
        va("w:CT_SdtPr", &["w:CT_SdtBlock", "w:CT_SdtCell", "w:CT_SdtRow", "w:CT_SdtRun"]),
    ]),
    el("w:sectPr", "w:CT_SectPr", &[
        va("w:CT_SectPr", &["w:CT_Body", "w:CT_PPr"]),
        va("w:CT_SectPrBase", &["w:CT_SectPrChange"]),
    ]),
    el("w:sectPrChange", "w:CT_SectPrChange", &[
        va("w:CT_SectPrChange", &["w:CT_SectPr"]),
    ]),
    el("w:semiHidden", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Style"]),
    ]),
    el("w:separator", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:settings", "w:CT_Settings", &[
        va("w:CT_Settings", &[]),
    ]),
    el("w:shadow", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:shd", "w:CT_Shd", &[
        va("w:CT_Shd", &["w:CT_PPrBase", "w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal", "w:CT_TblPrBase", "w:CT_TblPrExBase", "w:CT_TcPrBase"]),
    ]),
    el("w:showEnvelope", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:showXMLTags", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:showingPlcHdr", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_SdtPr"]),
    ]),
    el("w:sig", "w:CT_FontSig", &[
        va("w:CT_FontSig", &["w:CT_Font"]),
    ]),
    el("w:size", "w:CT_HpsMeasure", &[
        va("w:CT_HpsMeasure", &["w:CT_FFCheckBox"]),
    ]),
    el("w:sizeAuto", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_FFCheckBox"]),
    ]),
    el("w:smallCaps", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:smartTag", "w:CT_SmartTagRun", &[
        va("w:CT_SmartTagRun", &["m:CT_OMath", "m:CT_OMathArg", "w:CT_CustomXmlRun", "w:CT_P", "w:CT_RunTrackChange", "w:CT_SimpleField", "w:CT_SmartTagRun"]),
    ]),
    el("w:smartTagPr", "w:CT_SmartTagPr", &[
        va("w:CT_SmartTagPr", &["w:CT_SmartTagRun"]),
    ]),
    el("w:smartTagType", "w:CT_SmartTagType", &[
        va("w:CT_SmartTagType", &["w:CT_Settings"]),
    ]),
    el("w:snapToGrid", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase", "w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:softHyphen", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:sourceFileName", "w:CT_Rel", &[
        va("w:CT_Rel", &["w:CT_Frame"]),
    ]),
    el("w:spaceForUL", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Compat"]),
    ]),
    el("w:spacing", "w:CT_Spacing", &[
        va("w:CT_SignedTwipsMeasure", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
        va("w:CT_Spacing", &["w:CT_PPrBase"]),
    ]),
    el("w:specVanish", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:src", "w:CT_Rel", &[
        va("w:CT_Rel", &["w:CT_Odso"]),
    ]),
    el("w:start", "w:CT_Border", &[
        va("w:CT_Border", &["w:CT_TblBorders", "w:CT_TcBorders"]),
        va("w:CT_DecimalNumber", &["w:CT_Lvl"]),
        va("w:CT_TblWidth", &["w:CT_TblCellMar", "w:CT_TcMar"]),
    ]),
    el("w:startOverride", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_NumLvl"]),
    ]),
    el("w:statusText", "w:CT_FFStatusText", &[
        va("w:CT_FFStatusText", &["w:CT_FFData"]),
    ]),
    el("w:storeMappedDataAs", "w:CT_SdtDateMappingType", &[
        va("w:CT_SdtDateMappingType", &["w:CT_SdtDate"]),
    ]),
    el("w:strictFirstAndLastChars", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:strike", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:style", "w:CT_Style", &[
        va("w:CT_String", &["w:CT_DocPartPr"]),
        va("w:CT_Style", &["w:CT_Styles"]),
    ]),
    el("w:styleLink", "w:CT_String", &[
        va("w:CT_String", &["w:CT_AbstractNum"]),
    ]),
    el("w:styleLockQFSet", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:styleLockTheme", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:stylePaneFormatFilter", "w:CT_StylePaneFilter", &[
        va("w:CT_StylePaneFilter", &["w:CT_Settings"]),
    ]),
    el("w:stylePaneSortMethod", "w:CT_StyleSort", &[
        va("w:CT_StyleSort", &["w:CT_Settings"]),
    ]),
    el("w:styles", "w:CT_Styles", &[
        va("w:CT_Styles", &[]),
    ]),
    el("w:subDoc", "w:CT_Rel", &[
        va("w:CT_Rel", &["w:CT_CustomXmlRun", "w:CT_P", "w:CT_SimpleField", "w:CT_SmartTagRun"]),
    ]),
    el("w:suff", "w:CT_LevelSuffix", &[
        va("w:CT_LevelSuffix", &["w:CT_Lvl"]),
    ]),
    el("w:summaryLength", "w:CT_DecimalNumberOrPrecent", &[
        va("w:CT_DecimalNumberOrPrecent", &["w:CT_Settings"]),
    ]),
    el("w:suppressAutoHyphens", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:suppressLineNumbers", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:suppressOverlap", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:sym", "w:CT_Sym", &[
        va("w:CT_Sym", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:sz", "w:CT_String", &[
        va("w:CT_HpsMeasure", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
        va("w:CT_String", &["w:CT_Frame", "w:CT_Frameset"]),
    ]),
    el("w:szCs", "w:CT_HpsMeasure", &[
        va("w:CT_HpsMeasure", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:t", "w:CT_Text", &[
        va("w:CT_Text", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:tab", "w:CT_TabStop", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
        va("w:CT_TabStop", &["w:CT_Tabs"]),
    ]),
    el("w:tabIndex", "w:CT_UnsignedDecimalNumber", &[
        va("w:CT_UnsignedDecimalNumber", &["w:CT_FFData", "w:CT_SdtPr"]),
    ]),
    el("w:table", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Odso"]),
    ]),
    el("w:tabs", "w:CT_Tabs", &[
        va("w:CT_Tabs", &["w:CT_PPrBase"]),
    ]),
    el("w:tag", "w:CT_String", &[
        va("w:CT_String", &["w:CT_SdtPr"]),
    ]),
    el("w:targetScreenSz", "w:CT_TargetScreenSz", &[
        va("w:CT_TargetScreenSz", &["w:CT_WebSettings"]),
    ]),
    el("w:tbl", "w:CT_Tbl", &[
        va("w:CT_Tbl", &["w:CT_Body", "w:CT_Comment", "w:CT_CustomXmlBlock", "w:CT_FtnEdn", "w:CT_Tc"]),
    ]),
    el("w:tblBorders", "w:CT_TblBorders", &[
        va("w:CT_TblBorders", &["w:CT_TblPrBase", "w:CT_TblPrExBase"]),
    ]),
    el("w:tblCaption", "w:CT_String", &[
        va("w:CT_String", &["w:CT_TblPrBase"]),
    ]),
    el("w:tblCellMar", "w:CT_TblCellMar", &[
        va("w:CT_TblCellMar", &["w:CT_TblPrBase", "w:CT_TblPrExBase"]),
    ]),
    el("w:tblCellSpacing", "w:CT_TblWidth", &[
        va("w:CT_TblWidth", &["w:CT_TblPrBase", "w:CT_TblPrExBase", "w:CT_TrPrBase"]),
    ]),
    el("w:tblDescription", "w:CT_String", &[
        va("w:CT_String", &["w:CT_TblPrBase"]),
    ]),
    el("w:tblGrid", "w:CT_TblGridBase", &[
        va("w:CT_TblGrid", &["w:CT_Tbl"]),
        va("w:CT_TblGridBase", &["w:CT_TblGridChange"]),
    ]),
    el("w:tblGridChange", "w:CT_TblGridChange", &[
        va("w:CT_TblGridChange", &["w:CT_TblGrid"]),
    ]),
    el("w:tblHeader", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_TrPrBase"]),
    ]),
    el("w:tblInd", "w:CT_TblWidth", &[
        va("w:CT_TblWidth", &["w:CT_TblPrBase", "w:CT_TblPrExBase"]),
    ]),
    el("w:tblLayout", "w:CT_TblLayoutType", &[
        va("w:CT_TblLayoutType", &["w:CT_TblPrBase", "w:CT_TblPrExBase"]),
    ]),
    el("w:tblLook", "w:CT_TblLook", &[
        va("w:CT_TblLook", &["w:CT_TblPrBase", "w:CT_TblPrExBase"]),
    ]),
    el("w:tblOverlap", "w:CT_TblOverlap", &[
        va("w:CT_TblOverlap", &["w:CT_TblPrBase"]),
    ]),
    el("w:tblPr", "w:CT_TblPrBase", &[
        va("w:CT_TblPr", &["w:CT_Tbl"]),
        va("w:CT_TblPrBase", &["w:CT_Style", "w:CT_TblPrChange", "w:CT_TblStylePr"]),
    ]),
    el("w:tblPrChange", "w:CT_TblPrChange", &[
        va("w:CT_TblPrChange", &["w:CT_TblPr"]),
    ]),
    el("w:tblPrEx", "w:CT_TblPrExBase", &[
        va("w:CT_TblPrEx", &["w:CT_Row"]),
        va("w:CT_TblPrExBase", &["w:CT_TblPrExChange"]),
    ]),
    el("w:tblPrExChange", "w:CT_TblPrExChange", &[
        va("w:CT_TblPrExChange", &["w:CT_TblPrEx"]),
    ]),
    el("w:tblStyle", "w:CT_String", &[
        va("w:CT_String", &["w:CT_TblPrBase"]),
    ]),
    el("w:tblStyleColBandSize", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_TblPrBase"]),
    ]),
    el("w:tblStylePr", "w:CT_TblStylePr", &[
        va("w:CT_TblStylePr", &["w:CT_Style"]),
    ]),
    el("w:tblStyleRowBandSize", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_TblPrBase"]),
    ]),
    el("w:tblW", "w:CT_TblWidth", &[
        va("w:CT_TblWidth", &["w:CT_TblPrBase", "w:CT_TblPrExBase"]),
    ]),
    el("w:tblpPr", "w:CT_TblPPr", &[
        va("w:CT_TblPPr", &["w:CT_TblPrBase"]),
    ]),
    el("w:tc", "w:CT_Tc", &[
        va("w:CT_Tc", &["w:CT_CustomXmlCell", "w:CT_Row"]),
    ]),
    el("w:tcBorders", "w:CT_TcBorders", &[
        va("w:CT_TcBorders", &["w:CT_TcPrBase"]),
    ]),
    el("w:tcFitText", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_TcPrBase"]),
    ]),
    el("w:tcMar", "w:CT_TcMar", &[
        va("w:CT_TcMar", &["w:CT_TcPrBase"]),
    ]),
    el("w:tcPr", "w:CT_TcPr", &[
        va("w:CT_TcPr", &["w:CT_Style", "w:CT_TblStylePr", "w:CT_Tc"]),
        va("w:CT_TcPrInner", &["w:CT_TcPrChange"]),
    ]),
    el("w:tcPrChange", "w:CT_TcPrChange", &[
        va("w:CT_TcPrChange", &["w:CT_TcPr"]),
    ]),
    el("w:tcW", "w:CT_TblWidth", &[
        va("w:CT_TblWidth", &["w:CT_TcPrBase"]),
    ]),
    el("w:temporary", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_SdtPr"]),
    ]),
    el("w:text", "w:CT_SdtText", &[
        va("w:CT_SdtText", &["w:CT_SdtPr"]),
    ]),
    el("w:textAlignment", "w:CT_TextAlignment", &[
        va("w:CT_TextAlignment", &["w:CT_PPrBase"]),
    ]),
    el("w:textDirection", "w:CT_TextDirection", &[
        va("w:CT_TextDirection", &["w:CT_PPrBase", "w:CT_SectPr", "w:CT_SectPrBase", "w:CT_TcPrBase"]),
    ]),
    el("w:textInput", "w:CT_FFTextInput", &[
        va("w:CT_FFTextInput", &["w:CT_FFData"]),
    ]),
    el("w:textboxTightWrap", "w:CT_TextboxTightWrap", &[
        va("w:CT_TextboxTightWrap", &["w:CT_PPrBase"]),
    ]),
    el("w:themeFontLang", "w:CT_Language", &[
        va("w:CT_Language", &["w:CT_Settings"]),
    ]),
    el("w:title", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Frame", "w:CT_Frameset"]),
    ]),
    el("w:titlePg", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:tl2br", "w:CT_Border", &[
        va("w:CT_Border", &["w:CT_TcBorders"]),
    ]),
    el("w:tmpl", "w:CT_LongHexNumber", &[
        va("w:CT_LongHexNumber", &["w:CT_AbstractNum"]),
    ]),
    el("w:top", "w:CT_Border", &[
        va("w:CT_Border", &["w:CT_DivBdr", "w:CT_PBdr", "w:CT_TblBorders", "w:CT_TcBorders"]),
        va("w:CT_TblWidth", &["w:CT_TblCellMar", "w:CT_TcMar"]),
        va("w:CT_TopPageBorder", &["w:CT_PageBorders"]),
    ]),
    el("w:topLinePunct", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:tr", "w:CT_Row", &[
        va("w:CT_Row", &["w:CT_CustomXmlRow", "w:CT_Tbl"]),
    ]),
    el("w:tr2bl", "w:CT_Border", &[
        va("w:CT_Border", &["w:CT_TcBorders"]),
    ]),
    el("w:trHeight", "w:CT_Height", &[
        va("w:CT_Height", &["w:CT_TrPrBase"]),
    ]),
    el("w:trPr", "w:CT_TrPr", &[
        va("w:CT_TrPr", &["w:CT_Row", "w:CT_Style", "w:CT_TblStylePr"]),
        va("w:CT_TrPrBase", &["w:CT_TrPrChange"]),
    ]),
    el("w:trPrChange", "w:CT_TrPrChange", &[
        va("w:CT_TrPrChange", &["w:CT_TrPr"]),
    ]),
    el("w:trackRevisions", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:type", "w:CT_FFTextType", &[
        va("w:CT_DocPartType", &["w:CT_DocPartTypes"]),
        va("w:CT_FFTextType", &["w:CT_FFTextInput"]),
        va("w:CT_MailMergeOdsoFMDFieldType", &["w:CT_OdsoFieldMapData"]),
        va("w:CT_MailMergeSourceType", &["w:CT_Odso"]),
        va("w:CT_SectType", &["w:CT_SectPr", "w:CT_SectPrBase"]),
    ]),
    el("w:types", "w:CT_DocPartTypes", &[
        va("w:CT_DocPartTypes", &["w:CT_DocPartPr"]),
    ]),
    el("w:u", "w:CT_Underline", &[
        va("w:CT_Underline", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:udl", "w:CT_String", &[
        va("w:CT_String", &["w:CT_Odso"]),
    ]),
    el("w:uiPriority", "w:CT_DecimalNumber", &[
        va("w:CT_DecimalNumber", &["w:CT_Style"]),
    ]),
    el("w:ulTrailSpace", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Compat"]),
    ]),
    el("w:unhideWhenUsed", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Style"]),
    ]),
    el("w:uniqueTag", "w:CT_Base64Binary", &[
        va("w:CT_Base64Binary", &["w:CT_RecipientData"]),
    ]),
    el("w:updateFields", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:useXSLTWhenSaving", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_Settings"]),
    ]),
    el("w:vAlign", "w:CT_VerticalJc", &[
        va("w:CT_VerticalJc", &["w:CT_SectPr", "w:CT_SectPrBase", "w:CT_TcPrBase"]),
    ]),
    el("w:vMerge", "w:CT_VMerge", &[
        va("w:CT_VMerge", &["w:CT_TcPrBase"]),
    ]),
    el("w:vanish", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:vertAlign", "w:CT_VerticalAlignRun", &[
        va("w:CT_VerticalAlignRun", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:view", "w:CT_View", &[
        va("w:CT_View", &["w:CT_Settings"]),
    ]),
    el("w:viewMergedData", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_MailMerge"]),
    ]),
    el("w:w", "w:CT_TextScale", &[
        va("w:CT_TextScale", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
        va("w:CT_TwipsMeasure", &["w:CT_FramesetSplitbar"]),
    ]),
    el("w:wAfter", "w:CT_TblWidth", &[
        va("w:CT_TblWidth", &["w:CT_TrPrBase"]),
    ]),
    el("w:wBefore", "w:CT_TblWidth", &[
        va("w:CT_TblWidth", &["w:CT_TrPrBase"]),
    ]),
    el("w:webHidden", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_ParaRPr", "w:CT_ParaRPrOriginal", "w:CT_RPr", "w:CT_RPrOriginal"]),
    ]),
    el("w:webSettings", "w:CT_WebSettings", &[
        va("w:CT_WebSettings", &[]),
    ]),
    el("w:widowControl", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:wordWrap", "w:CT_OnOff", &[
        va("w:CT_OnOff", &["w:CT_PPrBase"]),
    ]),
    el("w:writeProtection", "w:CT_WriteProtection", &[
        va("w:CT_WriteProtection", &["w:CT_Settings"]),
    ]),
    el("w:yearLong", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:yearShort", "w:CT_Empty", &[
        va("w:CT_Empty", &["m:CT_R", "w:CT_R"]),
    ]),
    el("w:zoom", "w:CT_Zoom", &[
        va("w:CT_Zoom", &["w:CT_Settings"]),
    ]),
    el("wp:align", "wp:ST_AlignH", &[
        va("wp:ST_AlignH", &["wp:CT_PosH"]),
        va("wp:ST_AlignV", &["wp:CT_PosV"]),
    ]),
    el("wp:anchor", "wp:CT_Anchor", &[
        va("wp:CT_Anchor", &["w:CT_Drawing"]),
    ]),
    el("wp:bg", "a:CT_BackgroundFormatting", &[
        va("a:CT_BackgroundFormatting", &["wp:CT_WordprocessingCanvas"]),
    ]),
    el("wp:bodyPr", "a:CT_TextBodyProperties", &[
        va("a:CT_TextBodyProperties", &["wp:CT_WordprocessingShape"]),
    ]),
    el("wp:cNvCnPr", "a:CT_NonVisualConnectorProperties", &[
        va("a:CT_NonVisualConnectorProperties", &["wp:CT_WordprocessingShape"]),
    ]),
    el("wp:cNvContentPartPr", "a:CT_NonVisualContentPartProperties", &[
        va("a:CT_NonVisualContentPartProperties", &["wp:CT_WordprocessingContentPartNonVisual"]),
    ]),
    el("wp:cNvFrPr", "a:CT_NonVisualGraphicFrameProperties", &[
        va("a:CT_NonVisualGraphicFrameProperties", &["wp:CT_GraphicFrame"]),
    ]),
    el("wp:cNvGraphicFramePr", "a:CT_NonVisualGraphicFrameProperties", &[
        va("a:CT_NonVisualGraphicFrameProperties", &["wp:CT_Anchor", "wp:CT_Inline"]),
    ]),
    el("wp:cNvGrpSpPr", "a:CT_NonVisualGroupDrawingShapeProps", &[
        va("a:CT_NonVisualGroupDrawingShapeProps", &["wp:CT_WordprocessingGroup"]),
    ]),
    el("wp:cNvPr", "a:CT_NonVisualDrawingProps", &[
        va("a:CT_NonVisualDrawingProps", &["wp:CT_GraphicFrame", "wp:CT_WordprocessingContentPartNonVisual", "wp:CT_WordprocessingGroup", "wp:CT_WordprocessingShape"]),
    ]),
    el("wp:cNvSpPr", "a:CT_NonVisualDrawingShapeProps", &[
        va("a:CT_NonVisualDrawingShapeProps", &["wp:CT_WordprocessingShape"]),
    ]),
    el("wp:contentPart", "wp:CT_WordprocessingContentPart", &[
        va("wp:CT_WordprocessingContentPart", &["wp:CT_WordprocessingCanvas", "wp:CT_WordprocessingGroup"]),
    ]),
    el("wp:docPr", "a:CT_NonVisualDrawingProps", &[
        va("a:CT_NonVisualDrawingProps", &["wp:CT_Anchor", "wp:CT_Inline"]),
    ]),
    el("wp:effectExtent", "wp:CT_EffectExtent", &[
        va("wp:CT_EffectExtent", &["wp:CT_Anchor", "wp:CT_Inline", "wp:CT_WrapSquare", "wp:CT_WrapTopBottom"]),
    ]),
    el("wp:extLst", "a:CT_OfficeArtExtensionList", &[
        va("a:CT_OfficeArtExtensionList", &["wp:CT_GraphicFrame", "wp:CT_LinkedTextboxInformation", "wp:CT_TextboxInfo", "wp:CT_WordprocessingCanvas", "wp:CT_WordprocessingContentPart", "wp:CT_WordprocessingGroup", "wp:CT_WordprocessingShape"]),
    ]),
    el("wp:extent", "a:CT_PositiveSize2D", &[
        va("a:CT_PositiveSize2D", &["wp:CT_Anchor", "wp:CT_Inline"]),
    ]),
    el("wp:graphicFrame", "wp:CT_GraphicFrame", &[
        va("wp:CT_GraphicFrame", &["wp:CT_WordprocessingCanvas", "wp:CT_WordprocessingGroup"]),
    ]),
    el("wp:grpSp", "wp:CT_WordprocessingGroup", &[
        va("wp:CT_WordprocessingGroup", &["wp:CT_WordprocessingGroup"]),
    ]),
    el("wp:grpSpPr", "a:CT_GroupShapeProperties", &[
        va("a:CT_GroupShapeProperties", &["wp:CT_WordprocessingGroup"]),
    ]),
    el("wp:inline", "wp:CT_Inline", &[
        va("wp:CT_Inline", &["w:CT_Drawing"]),
    ]),
    el("wp:lineTo", "a:CT_Point2D", &[
        va("a:CT_Point2D", &["wp:CT_WrapPath"]),
    ]),
    el("wp:linkedTxbx", "wp:CT_LinkedTextboxInformation", &[
        va("wp:CT_LinkedTextboxInformation", &["wp:CT_WordprocessingShape"]),
    ]),
    el("wp:nvContentPartPr", "wp:CT_WordprocessingContentPartNonVisual", &[
        va("wp:CT_WordprocessingContentPartNonVisual", &["wp:CT_WordprocessingContentPart"]),
    ]),
    el("wp:posOffset", "wp:ST_PositionOffset", &[
        va("wp:ST_PositionOffset", &["wp:CT_PosH", "wp:CT_PosV"]),
    ]),
    el("wp:positionH", "wp:CT_PosH", &[
        va("wp:CT_PosH", &["wp:CT_Anchor"]),
    ]),
    el("wp:positionV", "wp:CT_PosV", &[
        va("wp:CT_PosV", &["wp:CT_Anchor"]),
    ]),
    el("wp:simplePos", "a:CT_Point2D", &[
        va("a:CT_Point2D", &["wp:CT_Anchor"]),
    ]),
    el("wp:spPr", "a:CT_ShapeProperties", &[
        va("a:CT_ShapeProperties", &["wp:CT_WordprocessingShape"]),
    ]),
    el("wp:start", "a:CT_Point2D", &[
        va("a:CT_Point2D", &["wp:CT_WrapPath"]),
    ]),
    el("wp:style", "a:CT_ShapeStyle", &[
        va("a:CT_ShapeStyle", &["wp:CT_WordprocessingShape"]),
    ]),
    el("wp:txbx", "wp:CT_TextboxInfo", &[
        va("wp:CT_TextboxInfo", &["wp:CT_WordprocessingShape"]),
    ]),
    el("wp:txbxContent", "wp:CT_TxbxContent", &[
        va("wp:CT_TxbxContent", &["wp:CT_TextboxInfo"]),
    ]),
    el("wp:wgp", "wp:CT_WordprocessingGroup", &[
        va("wp:CT_WordprocessingGroup", &["wp:CT_WordprocessingCanvas"]),
    ]),
    el("wp:whole", "a:CT_WholeE2oFormatting", &[
        va("a:CT_WholeE2oFormatting", &["wp:CT_WordprocessingCanvas"]),
    ]),
    el("wp:wpc", "wp:CT_WordprocessingCanvas", &[
        va("wp:CT_WordprocessingCanvas", &[]),
    ]),
    el("wp:wrapNone", "wp:CT_WrapNone", &[
        va("wp:CT_WrapNone", &["wp:CT_Anchor"]),
    ]),
    el("wp:wrapPolygon", "wp:CT_WrapPath", &[
        va("wp:CT_WrapPath", &["wp:CT_WrapThrough", "wp:CT_WrapTight"]),
    ]),
    el("wp:wrapSquare", "wp:CT_WrapSquare", &[
        va("wp:CT_WrapSquare", &["wp:CT_Anchor"]),
    ]),
    el("wp:wrapThrough", "wp:CT_WrapThrough", &[
        va("wp:CT_WrapThrough", &["wp:CT_Anchor"]),
    ]),
    el("wp:wrapTight", "wp:CT_WrapTight", &[
        va("wp:CT_WrapTight", &["wp:CT_Anchor"]),
    ]),
    el("wp:wrapTopAndBottom", "wp:CT_WrapTopBottom", &[
        va("wp:CT_WrapTopBottom", &["wp:CT_Anchor"]),
    ]),
    el("wp:wsp", "wp:CT_WordprocessingShape", &[
        va("wp:CT_WordprocessingShape", &["wp:CT_WordprocessingCanvas", "wp:CT_WordprocessingGroup"]),
    ]),
    el("wp:xfrm", "a:CT_Transform2D", &[
        va("a:CT_Transform2D", &["wp:CT_GraphicFrame", "wp:CT_WordprocessingContentPart"]),
    ]),
];

/// The `ST_NumberFormat` enumeration of `wml.xsd` (Strict), all 63 values in XSD order.
#[rustfmt::skip]
pub(crate) static NUM_FMT_VALUES: &[&str] = &[
    "decimal",
    "upperRoman",
    "lowerRoman",
    "upperLetter",
    "lowerLetter",
    "ordinal",
    "cardinalText",
    "ordinalText",
    "hex",
    "chicago",
    "ideographDigital",
    "japaneseCounting",
    "aiueo",
    "iroha",
    "decimalFullWidth",
    "decimalHalfWidth",
    "japaneseLegal",
    "japaneseDigitalTenThousand",
    "decimalEnclosedCircle",
    "decimalFullWidth2",
    "aiueoFullWidth",
    "irohaFullWidth",
    "decimalZero",
    "bullet",
    "ganada",
    "chosung",
    "decimalEnclosedFullstop",
    "decimalEnclosedParen",
    "decimalEnclosedCircleChinese",
    "ideographEnclosedCircle",
    "ideographTraditional",
    "ideographZodiac",
    "ideographZodiacTraditional",
    "taiwaneseCounting",
    "ideographLegalTraditional",
    "taiwaneseCountingThousand",
    "taiwaneseDigital",
    "chineseCounting",
    "chineseLegalSimplified",
    "chineseCountingThousand",
    "koreanDigital",
    "koreanCounting",
    "koreanLegal",
    "koreanDigital2",
    "vietnameseCounting",
    "russianLower",
    "russianUpper",
    "none",
    "numberInDash",
    "hebrew1",
    "hebrew2",
    "arabicAlpha",
    "arabicAbjad",
    "hindiVowels",
    "hindiConsonants",
    "hindiNumbers",
    "hindiCounting",
    "thaiLetters",
    "thaiNumbers",
    "thaiCounting",
    "bahtText",
    "dollarText",
    "custom",
];
