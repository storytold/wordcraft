use super::bx::{Bx, Cls};
use super::{Arc, C, FaceRef, MClass, MItem, MathTable, Rgb};

pub(super) struct Ctx {
    pub(super) face: FaceRef,
    pub(super) t: Option<Arc<MathTable>>,
    pub(super) upem: f32,
    pub(super) base: f32,
    pub(super) color: Rgb,
    pub(super) text_font: String,
    /// Nodes laid out so far (caps work on hostile input).
    pub(super) nodes: usize,
    /// The argument being laid out (for caret slots).
    pub(super) path: Vec<(usize, usize)>,
    /// Index of the node being laid out in its argument, innermost last.
    pub(super) node_stack: Vec<usize>,
}

pub(super) const MAX_NODES: usize = 200_000;

/// Constants as fractions of the em when there's no `MATH` table.
pub(super) fn fallback(c: C) -> f32 {
    match c {
        C::MathLeading => 0.15,
        C::AxisHeight => 0.25,
        C::AccentBaseHeight => 0.45,
        C::FlattenedAccentBaseHeight => 0.66,
        C::SubscriptShiftDown => 0.2,
        C::SubscriptTopMax => 0.36,
        C::SubscriptBaselineDropMin => 0.05,
        C::SuperscriptShiftUp => 0.36,
        C::SuperscriptShiftUpCramped => 0.29,
        C::SuperscriptBottomMin => 0.11,
        C::SuperscriptBaselineDropMax => 0.25,
        C::SubSuperscriptGapMin => 0.2,
        C::SuperscriptBottomMaxWithSubscript => 0.36,
        C::SpaceAfterScript => 0.05,
        C::UpperLimitGapMin => 0.1,
        C::UpperLimitBaselineRiseMin => 0.2,
        C::LowerLimitGapMin => 0.1,
        C::LowerLimitBaselineDropMin => 0.6,
        C::StackTopShiftUp => 0.44,
        C::StackTopDisplayStyleShiftUp => 0.68,
        C::StackBottomShiftDown => 0.34,
        C::StackBottomDisplayStyleShiftDown => 0.69,
        C::StackGapMin => 0.12,
        C::StackDisplayStyleGapMin => 0.28,
        C::StretchStackTopShiftUp => 0.2,
        C::StretchStackBottomShiftDown => 0.6,
        C::StretchStackGapAboveMin => 0.1,
        C::StretchStackGapBelowMin => 0.1,
        C::FractionNumeratorShiftUp => 0.39,
        C::FractionNumeratorDisplayStyleShiftUp => 0.68,
        C::FractionDenominatorShiftDown => 0.34,
        C::FractionDenominatorDisplayStyleShiftDown => 0.69,
        C::FractionNumeratorGapMin => 0.04,
        C::FractionNumDisplayStyleGapMin => 0.12,
        C::FractionRuleThickness => 0.05,
        C::FractionDenominatorGapMin => 0.04,
        C::FractionDenomDisplayStyleGapMin => 0.12,
        C::SkewedFractionHorizontalGap => 0.35,
        C::SkewedFractionVerticalGap => 0.1,
        C::OverbarVerticalGap => 0.15,
        C::OverbarRuleThickness => 0.05,
        C::OverbarExtraAscender => 0.05,
        C::UnderbarVerticalGap => 0.15,
        C::UnderbarRuleThickness => 0.05,
        C::UnderbarExtraDescender => 0.05,
        C::RadicalVerticalGap => 0.05,
        C::RadicalDisplayStyleVerticalGap => 0.15,
        C::RadicalRuleThickness => 0.05,
        C::RadicalExtraAscender => 0.05,
        C::RadicalKernBeforeDegree => 0.28,
        C::RadicalKernAfterDegree => -0.55,
    }
}

/// Characters drawn as operators with their own spacing class.
pub(super) fn class_of(c: char) -> Cls {
    match wordcraft_doc::math::math_class(c) {
        MClass::Ord => Cls::Ord,
        MClass::Op => Cls::Op,
        MClass::Bin => Cls::Bin,
        MClass::Rel => Cls::Rel,
        MClass::Open => Cls::Open,
        MClass::Close => Cls::Close,
        MClass::Punct => Cls::Punct,
        MClass::Inner => Cls::Inner,
    }
}

/// Accents drawn across the whole base (stretched horizontally).
pub(super) fn wide_accent(c: char) -> bool {
    matches!(
        c,
        '\u{20D6}'
            | '\u{20D7}'
            | '\u{20E1}'
            | '\u{20D0}'
            | '\u{20D1}'
            | '\u{2190}'
            | '\u{2192}'
            | '\u{2194}'
            | '\u{23DC}'
            | '\u{23DD}'
            | '\u{23DE}'
            | '\u{23DF}'
            | '\u{23B4}'
            | '\u{23B5}'
    )
}

/// The character an operator atom draws.
pub(super) fn op_char(b: &Bx) -> Option<char> {
    b.items.iter().find_map(|i| match i {
        MItem::Glyphs { text, .. } => {
            let mut cs = text.chars();
            let c = cs.next()?;
            cs.next().is_none().then_some(c)
        }
        _ => None,
    })
}
