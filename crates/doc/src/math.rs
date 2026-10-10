//! Equations: the Office Math object model (OMML, ECMA-376 Part 1 §22.1) as a tree, its linear
//! format (UnicodeMath, the plain text Word shows for an equation) and a parser for that format.
//!
//! An equation is a list of [`MNode`]s (an *argument*); structures hold arguments of their own
//! (a fraction's numerator and denominator, a radical's degree and base…). Layout reads the tree;
//! DOCX import builds it from OMML and keeps the source XML so a saved file carries the equation
//! exactly as Word wrote it.

use serde::{Deserialize, Serialize};

use crate::props::Rgb;

/// A sequence of math nodes (OMML's `m:e`, `m:num`, `m:sub`…).
pub type Arg = Vec<MNode>;

/// Deepest structure nesting kept (parsers stop here; layout draws nothing deeper).
pub const MAX_DEPTH: usize = 64;

/// An equation's structure, plus what's needed to write it back.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Math {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Arg,
    /// The source `m:oMath` element (DOCX import), written back unchanged on save. Empty for
    /// equations made here.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub omml: String,
    /// Justification of a display equation.
    #[serde(default, skip_serializing_if = "is_default")]
    pub jc: MathJc,
    /// Shown as its linear-format text (Equation › Convert › Linear) instead of built up.
    #[serde(default, skip_serializing_if = "is_default")]
    pub linear: bool,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

impl Math {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Display equation justification (`m:oMathParaPr/m:jc`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MathJc {
    #[default]
    CenterGroup,
    Center,
    Left,
    Right,
}

/// Math run style (`m:sty`): plain, bold, italic, bold italic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MSty {
    Plain,
    Bold,
    Italic,
    BoldItalic,
}

/// Math alphabet (`m:scr`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MScr {
    #[default]
    Roman,
    Script,
    Fraktur,
    DoubleStruck,
    SansSerif,
    Monospace,
}

/// A run of math text (`m:r`).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct MRun {
    pub text: String,
    /// `None`: math default (letters italic, the rest upright).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sty: Option<MSty>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub scr: MScr,
    /// Normal (non-math) text.
    #[serde(default, skip_serializing_if = "is_default")]
    pub nor: bool,
    /// Literal: no automatic operator handling.
    #[serde(default, skip_serializing_if = "is_default")]
    pub lit: bool,
    /// Font size, points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Rgb>,
    /// Font for normal text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
}

impl MRun {
    pub fn new(text: impl Into<String>) -> MRun {
        MRun { text: text.into(), ..Default::default() }
    }
    pub fn plain(text: impl Into<String>) -> MRun {
        MRun { text: text.into(), sty: Some(MSty::Plain), ..Default::default() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FracKind {
    /// Stacked with a rule.
    #[default]
    Bar,
    /// Skewed (num ⁄ den).
    Skewed,
    /// Linear (num/den).
    Linear,
    /// Stacked without a rule.
    NoBar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptKind {
    Sup,
    Sub,
    SubSup,
    /// Pre-scripts (`m:sPre`).
    Pre,
}

/// Where an n-ary operator's limits go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LimLoc {
    /// Under and over the operator.
    UndOvr,
    /// As sub- and superscripts.
    SubSup,
}

/// Column justification in a matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColJc {
    #[default]
    Center,
    Left,
    Right,
}

/// A math node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum MNode {
    Run(MRun),
    /// `m:f`.
    Frac {
        #[serde(default)]
        kind: FracKind,
        num: Arg,
        den: Arg,
    },
    /// `m:sSup`, `m:sSub`, `m:sSubSup`, `m:sPre`.
    Script {
        kind: ScriptKind,
        base: Arg,
        #[serde(default)]
        sub: Arg,
        #[serde(default)]
        sup: Arg,
    },
    /// `m:rad`.
    Rad {
        #[serde(default)]
        deg: Arg,
        #[serde(default)]
        deg_hide: bool,
        e: Arg,
    },
    /// `m:nary`: ∑ ∫ ∏ … with limits and an operand.
    Nary {
        chr: char,
        /// `None`: Word's default (integrals take sub/sup limits, the rest under/over).
        #[serde(default)]
        lim_loc: Option<LimLoc>,
        #[serde(default)]
        grow: bool,
        #[serde(default)]
        sub_hide: bool,
        #[serde(default)]
        sup_hide: bool,
        #[serde(default)]
        sub: Arg,
        #[serde(default)]
        sup: Arg,
        e: Arg,
    },
    /// `m:d`: delimiters around elements separated by `sep`.
    Delim {
        /// `None` = no delimiter.
        beg: Option<char>,
        end: Option<char>,
        #[serde(default)]
        sep: Option<char>,
        /// Delimiters grow to the content (Word's default).
        #[serde(default = "yes")]
        grow: bool,
        /// Grow to match the content exactly rather than centred on the math axis.
        #[serde(default)]
        shp_match: bool,
        elems: Vec<Arg>,
    },
    /// `m:func`: function name applied to an argument (sin x).
    Func {
        name: Arg,
        e: Arg,
    },
    /// `m:limLow` / `m:limUpp`.
    Lim {
        upper: bool,
        e: Arg,
        lim: Arg,
    },
    /// `m:acc`: accent over the base.
    Acc {
        chr: char,
        e: Arg,
    },
    /// `m:bar`: overbar or underbar.
    Bar {
        top: bool,
        e: Arg,
    },
    /// `m:borderBox`. `hide`: top, bottom, left, right. `strike`: horizontal, vertical,
    /// bottom-left→top-right, top-left→bottom-right.
    BorderBox {
        #[serde(default)]
        hide: [bool; 4],
        #[serde(default)]
        strike: [bool; 4],
        e: Arg,
    },
    /// `m:box`.
    Boxed {
        e: Arg,
    },
    /// `m:groupChr`: a stretched character (brace, arrow) under or over the base.
    GroupChr {
        chr: char,
        top: bool,
        e: Arg,
    },
    /// `m:eqArr`: rows aligned at `&`.
    EqArr {
        rows: Vec<Arg>,
    },
    /// `m:m`.
    Matrix {
        rows: Vec<Vec<Arg>>,
        #[serde(default)]
        col_jc: Vec<ColJc>,
    },
    /// `m:phant`.
    Phant {
        #[serde(default = "yes")]
        show: bool,
        #[serde(default)]
        zero_wid: bool,
        #[serde(default)]
        zero_asc: bool,
        #[serde(default)]
        zero_desc: bool,
        e: Arg,
    },
}

fn yes() -> bool {
    true
}

/// A row holds an equation-number mark (`#`) in a top-level run.
pub fn has_number_mark(row: &[MNode]) -> bool {
    row.iter().any(|n| matches!(n, MNode::Run(r) if !r.lit && r.text.contains('#')))
}

/// Integral signs (their limits default to sub/sup positions).
pub fn is_integral(c: char) -> bool {
    matches!(c, '∫' | '∬' | '∭' | '∮' | '∯' | '∰' | '∱' | '∲' | '∳' | '⨌')
}

/// Characters that take n-ary operator form in the linear format.
pub fn is_nary_char(c: char) -> bool {
    is_integral(c) || matches!(c, '∑' | '∏' | '∐' | '⋃' | '⋂' | '⋁' | '⋀' | '⨁' | '⨂' | '⨀' | '⨄' | '⨆')
}

/// Mathematical alphanumeric for `c` in a style and alphabet (Unicode block U+1D400).
pub fn math_alnum(c: char, bold: bool, italic: bool, scr: MScr) -> char {
    let off = |base: u32, k: u32| char::from_u32(base + k).unwrap_or(c);
    let upper = c.is_ascii_uppercase();
    let lower = c.is_ascii_lowercase();
    if upper || lower {
        let k = if upper { c as u32 - 'A' as u32 } else { c as u32 - 'a' as u32 };
        // Letters that live in the Letterlike Symbols block.
        let hole = |table: &[(char, char)]| table.iter().find(|(f, _)| *f == c).map(|(_, t)| *t);
        let (base_u, base_l) = match (scr, bold, italic) {
            (MScr::Roman, false, false) => return c,
            (MScr::Roman, true, false) => (0x1D400, 0x1D41A),
            (MScr::Roman, false, true) => {
                if c == 'h' {
                    return '\u{210E}';
                }
                (0x1D434, 0x1D44E)
            }
            (MScr::Roman, true, true) => (0x1D468, 0x1D482),
            (MScr::Script, false, _) => {
                if let Some(t) = hole(&[
                    ('B', 'ℬ'),
                    ('E', 'ℰ'),
                    ('F', 'ℱ'),
                    ('H', 'ℋ'),
                    ('I', 'ℐ'),
                    ('L', 'ℒ'),
                    ('M', 'ℳ'),
                    ('R', 'ℛ'),
                    ('e', 'ℯ'),
                    ('g', 'ℊ'),
                    ('o', 'ℴ'),
                ]) {
                    return t;
                }
                (0x1D49C, 0x1D4B6)
            }
            (MScr::Script, true, _) => (0x1D4D0, 0x1D4EA),
            (MScr::Fraktur, false, _) => {
                if let Some(t) = hole(&[('C', 'ℭ'), ('H', 'ℌ'), ('I', 'ℑ'), ('R', 'ℜ'), ('Z', 'ℨ')]) {
                    return t;
                }
                (0x1D504, 0x1D51E)
            }
            (MScr::Fraktur, true, _) => (0x1D56C, 0x1D586),
            (MScr::DoubleStruck, _, _) => {
                if let Some(t) = hole(&[('C', 'ℂ'), ('H', 'ℍ'), ('N', 'ℕ'), ('P', 'ℙ'), ('Q', 'ℚ'), ('R', 'ℝ'), ('Z', 'ℤ')]) {
                    return t;
                }
                (0x1D538, 0x1D552)
            }
            (MScr::SansSerif, false, false) => (0x1D5A0, 0x1D5BA),
            (MScr::SansSerif, true, false) => (0x1D5D4, 0x1D5EE),
            (MScr::SansSerif, false, true) => (0x1D608, 0x1D622),
            (MScr::SansSerif, true, true) => (0x1D63C, 0x1D656),
            (MScr::Monospace, _, _) => (0x1D670, 0x1D68A),
        };
        return off(if upper { base_u } else { base_l }, k);
    }
    if c.is_ascii_digit() {
        let k = c as u32 - '0' as u32;
        return match (scr, bold) {
            (MScr::Roman, true) => off(0x1D7CE, k),
            (MScr::DoubleStruck, _) => off(0x1D7D8, k),
            (MScr::SansSerif, false) => off(0x1D7E2, k),
            (MScr::SansSerif, true) => off(0x1D7EC, k),
            (MScr::Monospace, _) => off(0x1D7F6, k),
            _ => c,
        };
    }
    // Greek (roman alphabet only).
    if scr == MScr::Roman && (bold || italic) {
        let (cap, small) = match (bold, italic) {
            (true, false) => (0x1D6A8, 0x1D6C2),
            (false, true) => (0x1D6E2, 0x1D6FC),
            _ => (0x1D71C, 0x1D736),
        };
        let u = c as u32;
        if (0x391..=0x3A9).contains(&u) && u != 0x3A2 {
            return off(cap, u - 0x391);
        }
        if (0x3B1..=0x3C9).contains(&u) {
            return off(small, u - 0x3B1);
        }
        let extra = match c {
            '∂' => Some(25),
            'ϵ' => Some(26),
            'ϑ' => Some(27),
            'ϰ' => Some(28),
            'ϕ' => Some(29),
            'ϱ' => Some(30),
            'ϖ' => Some(31),
            _ => None,
        };
        if let Some(e) = extra
            && italic
        {
            return off(small, e);
        }
    }
    c
}

// ---------------------------------------------------------------------------------------------
// Linear format out.

/// The equation as linear-format text (`x=(-b±√(b^2-4ac))/2a`).
pub fn to_linear(nodes: &[MNode]) -> String {
    let mut s = String::new();
    lin_arg(nodes, &mut s, 0);
    s
}

fn lin_arg(a: &[MNode], s: &mut String, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for n in a {
        lin_node(n, s, depth + 1);
    }
}

/// An argument as an operand: bare when it's a single simple atom, parenthesised otherwise.
fn lin_operand(a: &[MNode], s: &mut String, depth: usize) {
    let mut inner = String::new();
    lin_arg(a, &mut inner, depth);
    let simple = match a {
        [MNode::Run(r)] => {
            let mut cs = r.text.chars();
            match (cs.next(), cs.next()) {
                (Some(c), None) => !"+-−=/^_ ".contains(c),
                _ => r.text.chars().all(|c| c.is_alphanumeric() || c == '.'),
            }
        }
        [MNode::Delim { .. }] => true,
        _ => false,
    };
    if simple && !inner.is_empty() {
        s.push_str(&inner);
    } else {
        s.push('(');
        s.push_str(&inner);
        s.push(')');
    }
}

fn lin_node(n: &MNode, s: &mut String, depth: usize) {
    match n {
        MNode::Run(r) => s.push_str(&r.text),
        MNode::Frac { kind, num, den } => {
            lin_operand(num, s, depth);
            s.push(match kind {
                FracKind::Skewed => '⁄',
                FracKind::Linear => '∕',
                FracKind::NoBar => '¦',
                FracKind::Bar => '/',
            });
            lin_operand(den, s, depth);
        }
        MNode::Script { kind, base, sub, sup } => {
            if *kind == ScriptKind::Pre {
                s.push('_');
                lin_operand(sub, s, depth);
                s.push('^');
                lin_operand(sup, s, depth);
                s.push('▒');
                lin_operand(base, s, depth);
                return;
            }
            lin_operand(base, s, depth);
            if matches!(kind, ScriptKind::Sub | ScriptKind::SubSup) {
                s.push('_');
                lin_operand(sub, s, depth);
            }
            if matches!(kind, ScriptKind::Sup | ScriptKind::SubSup) {
                s.push('^');
                lin_operand(sup, s, depth);
            }
        }
        MNode::Rad { deg, deg_hide, e } => {
            if *deg_hide || deg.is_empty() {
                s.push('√');
                lin_operand(e, s, depth);
            } else {
                s.push_str("√(");
                lin_arg(deg, s, depth);
                s.push('&');
                lin_arg(e, s, depth);
                s.push(')');
            }
        }
        MNode::Nary { chr, sub_hide, sup_hide, sub, sup, e, .. } => {
            s.push(*chr);
            if !sub_hide && !sub.is_empty() {
                s.push('_');
                lin_operand(sub, s, depth);
            }
            if !sup_hide && !sup.is_empty() {
                s.push('^');
                lin_operand(sup, s, depth);
            }
            s.push('▒');
            lin_operand(e, s, depth);
        }
        MNode::Delim { beg, end, elems, .. } => {
            // A standard pair writes as itself; anything else uses explicit `├X … Y┤`.
            let pair = matches!((beg, end), (Some(b), Some(e)) if crate::math_linear::closer(*b) == Some(*e));
            let invisible = beg.is_none() && end.is_none();
            if pair {
                s.extend(*beg);
            } else if invisible {
                s.push('〖');
            } else {
                s.push('├');
                s.extend(*beg);
            }
            for (i, e) in elems.iter().enumerate() {
                if i > 0 {
                    s.push('│');
                }
                lin_arg(e, s, depth);
            }
            if pair {
                s.extend(*end);
            } else if invisible {
                s.push('〗');
            } else {
                s.extend(*end);
                s.push('┤');
            }
        }
        MNode::Func { name, e } => {
            lin_arg(name, s, depth);
            s.push('\u{2061}');
            lin_operand(e, s, depth);
        }
        MNode::Lim { upper, e, lim } => {
            lin_operand(e, s, depth);
            s.push(if *upper { '┴' } else { '┬' });
            lin_operand(lim, s, depth);
        }
        MNode::Acc { chr, e } => {
            lin_operand(e, s, depth);
            s.push(*chr);
        }
        MNode::Bar { top, e } => {
            s.push(if *top { '¯' } else { '▁' });
            lin_operand(e, s, depth);
        }
        MNode::BorderBox { e, .. } => {
            s.push('▭');
            lin_operand(e, s, depth);
        }
        MNode::Boxed { e } => {
            s.push('□');
            lin_operand(e, s, depth);
        }
        MNode::GroupChr { chr, e, .. } => {
            s.push(*chr);
            lin_operand(e, s, depth);
        }
        // A numbered equation (`E=mc^2#(1)`) is written as its one row.
        MNode::EqArr { rows } if rows.len() == 1 && rows.first().is_some_and(|r| has_number_mark(r)) => {
            if let Some(r) = rows.first() {
                lin_arg(r, s, depth);
            }
        }
        MNode::EqArr { rows } => {
            s.push_str("█(");
            for (i, r) in rows.iter().enumerate() {
                if i > 0 {
                    s.push('@');
                }
                lin_arg(r, s, depth);
            }
            s.push(')');
        }
        MNode::Matrix { rows, .. } => {
            s.push_str("■(");
            for (i, r) in rows.iter().enumerate() {
                if i > 0 {
                    s.push('@');
                }
                for (j, c) in r.iter().enumerate() {
                    if j > 0 {
                        s.push('&');
                    }
                    lin_arg(c, s, depth);
                }
            }
            s.push(')');
        }
        MNode::Phant { e, .. } => {
            s.push('⟡');
            lin_operand(e, s, depth);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Linear format in.

/// Parse linear-format text into nodes ([`crate::math_linear`]). Never fails: what isn't
/// understood stays as text.
pub fn parse_linear(s: &str) -> Arg {
    crate::math_linear::parse(s)
}

pub use crate::math_linear::FUNCTION_NAMES;

/// Spacing class of a math character (TeX's atom classes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MClass {
    #[default]
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
}

/// The spacing class of a character in an equation.
pub fn math_class(c: char) -> MClass {
    match c {
        '+' | '−' | '-' | '±' | '∓' | '×' | '÷' | '·' | '∙' | '∗' | '*' | '∘' | '⊕' | '⊖' | '⊗' | '⊘' | '⊙' | '⊚' | '⊛' | '∪' | '∩' | '∧' | '∨'
        | '∖' | '⋅' | '⋆' | '⊎' | '⊓' | '⊔' | '⋄' | '⊞' | '⊟' | '⊠' | '⊡' | '⋉' | '⋊' | '⋋' | '⋌' | '⋎' | '⋏' | '⋒' | '⋓' | '∔' | '∸' | '≀' | '⨿'
        | '⊼' | '⊻' | '⊽' => MClass::Bin,
        '=' | '<' | '>' | '≤' | '≥' | '≠' | '≈' | '≡' | '≢' | '∼' | '≃' | '≅' | '≐' | '∝' | '≪' | '≫' | '≺' | '≻' | '≼' | '≽' | '⊂' | '⊃' | '⊆'
        | '⊇' | '⊄' | '⊅' | '⊈' | '⊉' | '⊊' | '⊋' | '∈' | '∉' | '∋' | '∌' | '⊥' | '∥' | '∦' | '∣' | '∤' | '≔' | '≕' | '≝' | '≜' | '≞' | '≟' | '→'
        | '←' | '↔' | '⇒' | '⇐' | '⇔' | '↦' | '⟼' | '⟶' | '⟵' | '⟷' | '⟹' | '⟸' | '⟺' | '↑' | '↓' | '↕' | '⇑' | '⇓' | '⇕' | '↗' | '↘' | '↙' | '↖'
        | '⊢' | '⊣' | '⊨' | '⊩' | '⊪' | '⊬' | '⊭' | '⊮' | '⊯' | '≲' | '≳' | '≍' | '⊏' | '⊐' | '⊑' | '⊒' | '⋢' | '⋣' | '≶' | '≷' | '≮' | '≯' | '≰'
        | '≱' | '≦' | '≧' | '⩽' | '⩾' | ':' | '∶' | '∷' | '∺' | '∻' | '≗' | '≙' | '≚' | '≑' | '≒' | '≓' | '≖' | '⇌' | '⇋' | '⇄' | '⇆' | '⇇' | '⇉'
        | '⇈' | '⇊' | '↪' | '↩' | '↼' | '↽' | '⇀' | '⇁' | '↿' | '↾' | '⇃' | '⇂' | '↶' | '↷' | '↺' | '↻' | '⊸' | '↜' | '↝' | '↞' | '↠' | '↢' | '↣'
        | '↫' | '↬' | '↭' | '⇚' | '⇛' | '↰' | '↱' | '⇝' | '≁' | '≄' | '≉' | '≇' | '≭' | '≨' | '≩' | '⊀' | '⊁' | '⋠' | '⋡' | '⋦' | '⋧' | '⋨' | '⋩'
        | '⋪' | '⋫' | '⋬' | '⋭' | '⋈' | '∴' | '∵' | '≏' | '≎' | '⋘' | '⋙' | '⊲' | '⊳' | '⊴' | '⊵' | '⋐' | '⋑' => {
            MClass::Rel
        }
        '(' | '[' | '{' | '⟨' | '⌈' | '⌊' | '⟦' | '〈' => MClass::Open,
        ')' | ']' | '}' | '⟩' | '⌉' | '⌋' | '⟧' | '〉' | '!' => MClass::Close,
        ',' | ';' => MClass::Punct,
        c if is_nary_char(c) => MClass::Op,
        _ => MClass::Ord,
    }
}

/// Merge adjacent runs with the same formatting; recurse into structures.
pub fn merge_runs(a: Arg) -> Arg {
    let mut out: Arg = Vec::with_capacity(a.len());
    for n in a {
        let n = match n {
            MNode::Run(r) => {
                if let Some(MNode::Run(prev)) = out.last_mut()
                    && prev.sty == r.sty
                    && prev.scr == r.scr
                    && prev.nor == r.nor
                    && prev.lit == r.lit
                    && prev.size == r.size
                    && prev.color == r.color
                    && prev.font == r.font
                {
                    prev.text.push_str(&r.text);
                    continue;
                }
                MNode::Run(r)
            }
            MNode::Frac { kind, num, den } => MNode::Frac { kind, num: merge_runs(num), den: merge_runs(den) },
            MNode::Script { kind, base, sub, sup } => MNode::Script { kind, base: merge_runs(base), sub: merge_runs(sub), sup: merge_runs(sup) },
            MNode::Rad { deg, deg_hide, e } => MNode::Rad { deg: merge_runs(deg), deg_hide, e: merge_runs(e) },
            MNode::Nary { chr, lim_loc, grow, sub_hide, sup_hide, sub, sup, e } => {
                MNode::Nary { chr, lim_loc, grow, sub_hide, sup_hide, sub: merge_runs(sub), sup: merge_runs(sup), e: merge_runs(e) }
            }
            MNode::Delim { beg, end, sep, grow, shp_match, elems } => {
                MNode::Delim { beg, end, sep, grow, shp_match, elems: elems.into_iter().map(merge_runs).collect() }
            }
            MNode::Func { name, e } => MNode::Func { name: merge_runs(name), e: merge_runs(e) },
            other => other,
        };
        out.push(n);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(t: &str) -> MNode {
        MNode::Run(MRun::new(t))
    }

    #[test]
    fn quadratic_formula_parses_into_structures() {
        let n = parse_linear("x=(-b±√(b^2-4ac))/2a");
        assert_eq!(n.len(), 2, "{n:#?}");
        assert_eq!(n[0], run("x="));
        let MNode::Frac { num, den, .. } = &n[1] else { panic!("{n:#?}") };
        assert_eq!(den, &vec![run("2a")]);
        assert!(num.iter().any(|m| matches!(m, MNode::Rad { .. })), "{num:#?}");
        let MNode::Rad { e, .. } = num.iter().find(|m| matches!(m, MNode::Rad { .. })).unwrap() else { panic!() };
        assert!(matches!(e.first(), Some(MNode::Script { kind: ScriptKind::Sup, .. })), "{e:#?}");
    }

    #[test]
    fn scripts_and_nary() {
        let n = parse_linear("∑_(i=1)^n▒i^2");
        let MNode::Nary { chr, sub, sup, e, .. } = &n[0] else { panic!("{n:#?}") };
        assert_eq!(*chr, '∑');
        assert_eq!(sub, &vec![run("i=1")]);
        assert_eq!(sup, &vec![run("n")]);
        assert!(matches!(e.first(), Some(MNode::Script { .. })));
        let n = parse_linear("x_i^2");
        assert!(matches!(&n[0], MNode::Script { kind: ScriptKind::SubSup, .. }), "{n:#?}");
    }

    #[test]
    fn functions_and_matrices() {
        let n = parse_linear("sin x");
        assert!(matches!(&n[0], MNode::Func { .. }), "{n:#?}");
        let n = parse_linear("■(a&b@c&d)");
        let MNode::Matrix { rows, .. } = &n[0] else { panic!("{n:#?}") };
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].len(), 2);
    }

    #[test]
    fn linear_round_trip_keeps_structure() {
        for s in [
            "x=(-b±√(b^2-4ac))/2a",
            "∑_(i=1)^n▒i",
            "a/b+c",
            "E=mc^2",
            "√(3&x)",
            "├]a[┤",
            "{█(x@y)┤",
            "|x|+‖v‖",
            "⏞(a+b)┴k",
            "_1^n▒Y",
            "sin^(−1)⁡x",
            "lim┬(n→∞)⁡a_n",
            "A⃗+x̂",
            "▭(a)+¯(b)+▁(c)",
            "(a│b)",
            "E=mc^2#(1)",
        ] {
            let n = parse_linear(s);
            let back = parse_linear(&to_linear(&n));
            assert_eq!(n, back, "{s} → {}", to_linear(&n));
        }
    }

    #[test]
    fn hostile_linear_never_panics() {
        for s in ["", "/", "^", "_", "√", "∑", "((((", "))))", "■(", "█(@@&&", "a/", "/b", "^^^__", "√(&)", "∫_^▒", "lim_", "sin"] {
            let _ = to_linear(&parse_linear(s));
        }
        let deep = "(".repeat(5000) + &")".repeat(5000);
        let _ = parse_linear(&deep);
        let deep = "√".repeat(3000);
        let _ = parse_linear(&deep);
    }
}
