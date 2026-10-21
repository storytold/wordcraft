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

/// Where a display equation too wide for its line breaks at a binary operator (`m:brkBin`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrkBin {
    /// The operator starts the next line.
    #[default]
    Before,
    /// The operator ends the line.
    After,
    /// The operator ends the line and starts the next one again.
    Repeat,
}

/// How a minus repeats across a break when [`BrkBin::Repeat`] (`m:brkBinSub`): the operator
/// ending the line, then the one starting the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrkBinSub {
    /// `--`
    #[default]
    MinusMinus,
    /// `-+`
    MinusPlus,
    /// `+-`
    PlusMinus,
}

/// Document-wide equation sizing and wrapping options (`m:mathPr` in the settings part).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MathProps {
    /// Reduce nested display fraction arguments (`m:smallFrac`).
    pub small_frac: bool,
    pub brk_bin: BrkBin,
    pub brk_bin_sub: BrkBinSub,
    /// Indent of a wrapped equation's continuation lines, points (`m:wrapIndent`).
    pub wrap_indent: f32,
    /// Continuation lines align right instead (`m:wrapRight`).
    pub wrap_right: bool,
}

/// `m:wrapIndent`'s default: 1440 twips, one inch.
pub const DEFAULT_WRAP_INDENT: f32 = 72.0;

impl Default for MathProps {
    fn default() -> Self {
        MathProps {
            small_frac: false,
            brk_bin: BrkBin::Before,
            brk_bin_sub: BrkBinSub::MinusMinus,
            wrap_indent: DEFAULT_WRAP_INDENT,
            wrap_right: false,
        }
    }
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
    /// A manual line break before this run in a display equation (`m:brk`): `Some(0)` wraps as
    /// usual, `Some(n)` aligns the new line at the `n`th operator of the first line (`m:alnAt`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brk: Option<u8>,
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
    row.iter().any(|n| matches!(n, MNode::Run(r) if !r.lit && !r.nor && r.text.contains('#')))
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
    for (i, n) in a.iter().enumerate() {
        lin_node(n, s, depth + 1);
        // A bare script or limit ends at a space: a letter, digit or period after it is its own item.
        if matches!(n, MNode::Script { kind, .. } if *kind != ScriptKind::Pre) || matches!(n, MNode::Lim { .. }) {
            let ends_bare = s.chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '.');
            if ends_bare && a.get(i + 1).and_then(first_lin_char).is_some_and(|c| c.is_alphanumeric() || c == '.') {
                s.push(' ');
            }
        }
    }
}

/// The first character `n` is written with, when that is cheap to tell (letters and digits).
fn first_lin_char(n: &MNode) -> Option<char> {
    match n {
        MNode::Run(r) => r.text.chars().next(),
        MNode::Func { name, .. } => name.first().and_then(first_lin_char),
        MNode::Script { kind, base, .. } if *kind != ScriptKind::Pre => base.first().and_then(first_lin_char),
        MNode::Lim { e, .. } => e.first().and_then(first_lin_char),
        MNode::Frac { num, .. } => num.first().and_then(first_lin_char),
        _ => None,
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

/// A script or limit operand. The parser reads a run of letters and digits up to the next space or
/// operator, so `V_dd` is written bare as Word's Linear format shows it (`V_(dd)` reads back the same).
fn lin_script(a: &[MNode], s: &mut String, depth: usize) {
    // Bare only when the parser reads the whole run back, in the same style.
    if let [MNode::Run(r)] = a
        && r.text.chars().nth(1).is_some()
    {
        let upright_name = r.sty == Some(MSty::Plain) && crate::math_linear::FUNCTION_NAMES.contains(&r.text.as_str());
        let number = r.text.starts_with(|c: char| c.is_ascii_digit()) && r.text.chars().all(|c| c.is_alphanumeric() || c == '.');
        let word = r.text.starts_with(char::is_alphabetic) && r.text.chars().all(char::is_alphanumeric);
        let upright_other = r.sty == Some(MSty::Plain) && !upright_name;
        if upright_other || (!number && !word) {
            s.push('(');
            s.push_str(&r.text);
            s.push(')');
            return;
        }
    }
    lin_operand(a, s, depth);
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
                lin_script(sub, s, depth);
                s.push('^');
                lin_script(sup, s, depth);
                s.push('▒');
                lin_operand(base, s, depth);
                return;
            }
            lin_operand(base, s, depth);
            if matches!(kind, ScriptKind::Sub | ScriptKind::SubSup) {
                s.push('_');
                lin_script(sub, s, depth);
            }
            if matches!(kind, ScriptKind::Sup | ScriptKind::SubSup) {
                s.push('^');
                lin_script(sup, s, depth);
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
                lin_script(sub, s, depth);
            }
            if !sup_hide && !sup.is_empty() {
                s.push('^');
                lin_script(sup, s, depth);
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
            lin_script(lim, s, depth);
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

/// Merge adjacent runs with the same formatting; recurse into every structure.
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
                    && r.brk.is_none()
                {
                    prev.text.push_str(&r.text);
                    continue;
                }
                MNode::Run(r)
            }
            other => crate::math_linear::map_args(other, &merge_runs),
        };
        out.push(n);
    }
    out
}

/// Deepest node (a leaf counts 1) the parsers wrap further. Half of [`MAX_DEPTH`]: serde_json
/// stops reading at 128 nested containers and a node costs two (object and array).
pub(crate) const WRAP_DEPTH: usize = MAX_DEPTH / 2;

/// Calls `f` with every argument of `n`.
fn each_arg<'a>(n: &'a MNode, f: &mut impl FnMut(&'a [MNode])) {
    match n {
        MNode::Run(_) => {}
        MNode::Frac { num, den, .. } => {
            f(num);
            f(den);
        }
        MNode::Script { base, sub, sup, .. } => {
            f(base);
            f(sub);
            f(sup);
        }
        MNode::Rad { deg, e, .. } => {
            f(deg);
            f(e);
        }
        MNode::Nary { sub, sup, e, .. } => {
            f(sub);
            f(sup);
            f(e);
        }
        MNode::Delim { elems, .. } => elems.iter().for_each(|a| f(a)),
        MNode::Func { name, e } => {
            f(name);
            f(e);
        }
        MNode::Lim { e, lim, .. } => {
            f(e);
            f(lim);
        }
        MNode::Acc { e, .. }
        | MNode::Bar { e, .. }
        | MNode::BorderBox { e, .. }
        | MNode::Boxed { e }
        | MNode::GroupChr { e, .. }
        | MNode::Phant { e, .. } => f(e),
        MNode::EqArr { rows } => rows.iter().for_each(|a| f(a)),
        MNode::Matrix { rows, .. } => rows.iter().flatten().for_each(|a| f(a)),
    }
}

/// `n` is already nested too deeply to be wrapped in another structure. The parsers use this so
/// `x^1^1^1…` stays shallow (a left-nested tree 10 000 deep overflows the stack in `Clone`,
/// `Drop`, `PartialEq` and serde). Iterative, stops at the first too-deep path; costs at most the
/// size of `n`, which the parsers' input caps bound.
pub(crate) fn too_deep(n: &MNode) -> bool {
    let mut stack: Vec<(&MNode, usize)> = vec![(n, 1)];
    while let Some((n, d)) = stack.pop() {
        if d >= WRAP_DEPTH {
            return true;
        }
        each_arg(n, &mut |a| stack.extend(a.iter().map(|c| (c, d + 1))));
    }
    false
}

/// Any of `a` is too deep to be wrapped ([`too_deep`]).
pub(crate) fn any_too_deep(a: &[MNode]) -> bool {
    a.iter().any(too_deep)
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

    //T1
    /// Deepest nesting of `a` (a leaf counts 1), without recursion.
    fn depth_of(a: &[MNode]) -> usize {
        let mut best = 0;
        let mut stack: Vec<(&MNode, usize)> = a.iter().map(|n| (n, 1)).collect();
        while let Some((n, d)) = stack.pop() {
            best = best.max(d);
            each_arg(n, &mut |a| stack.extend(a.iter().map(|c| (c, d + 1))));
        }
        best
    }

    /// Run `f` on a thread with a stack far below the 8 MB of a main thread (#1456).
    fn on_small_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new().stack_size(512 * 1024).spawn(f).unwrap().join().unwrap();
    }

    #[test]
    fn deeply_chained_input_stays_shallow() {
        on_small_stack(|| {
            let cases: Vec<(&str, Arg)> = vec![
                ("x^1^1…", parse_linear(&format!("x{}", "^1".repeat(9999)))),
                ("a/a/a…", parse_linear(&format!("a{}", "/a".repeat(9999)))),
                ("accents", parse_linear(&format!("x{}", "\u{302}".repeat(9999)))),
                ("limits", parse_linear(&format!("x{}", "┬a".repeat(9000)))),
                ("(a b)^1^1…", parse_linear(&format!("(a b){}", "^1".repeat(9000)))),
                ("latex x^1^1…", crate::math_latex::parse_latex(&format!("x{}", "^1".repeat(9999)))),
                ("latex \\sum…", crate::math_latex::parse_latex(&"\\sum".repeat(5000))),
                ("latex \\sin x^1…", crate::math_latex::parse_latex(&format!("\\sin x{}", "^1".repeat(9000)))),
            ];
            for (name, n) in cases {
                assert!(!n.is_empty(), "{name}");
                assert!(depth_of(&n) <= 70, "{name}: depth {}", depth_of(&n));
                let copy = n.clone();
                assert_eq!(copy, n, "{name}");
                let _ = to_linear(&n);
                let _ = crate::math_latex::to_latex(&n);
                let json = serde_json::to_string(&n).unwrap();
                let back: Arg = serde_json::from_str(&json).unwrap();
                assert_eq!(back, n, "{name}");
            }
        });
    }

    #[test]
    fn ordinary_nesting_is_still_built() {
        // Well below the cap: scripts, fractions and operators still wrap.
        let n = parse_linear(&format!("x{}", "^1".repeat(10)));
        assert_eq!(depth_of(&n), 11, "{n:#?}");
        let n = crate::math_latex::parse_latex(r"\sum_{i}\sum_{j}a_{ij}");
        assert!(matches!(&n[0], MNode::Nary { e, .. } if matches!(e.first(), Some(MNode::Nary { .. }))), "{n:#?}");
    }

    //T2
    #[test]
    fn latex_text_escapes_what_latex_reads_as_markup() {
        for text in ["a}b\\c", "{x}", "50% of #1", "a_b&c$d~e^f", "\\", "\\alpha", "\\{"] {
            let n = vec![MNode::Run(MRun { text: text.into(), nor: true, sty: Some(MSty::Plain), ..Default::default() })];
            let tex = crate::math_latex::to_latex(&n);
            assert_eq!(crate::math_latex::parse_latex(&tex), n, "{text:?} → {tex}");
        }
        let n = vec![MNode::Run(MRun::new("a\\b"))];
        assert_eq!(crate::math_latex::parse_latex(&crate::math_latex::to_latex(&n)), n);
    }

    //T3
    #[test]
    fn latex_runs_merge_inside_every_structure() {
        use crate::math_latex::parse_latex;
        for (tex, lin) in [(r"\overline{ab}", "¯(ab)"), (r"\underline{ab}", "▁(ab)"), (r"\boxed{ab}", "▭(ab)"), (r"\phantom{ab}", "⟡(ab)")] {
            assert_eq!(parse_latex(tex), parse_linear(lin), "{tex}");
        }
        let n = parse_latex(r"\begin{matrix}a b&c d\\e f&g\end{matrix}");
        let MNode::Matrix { rows, .. } = &n[0] else { panic!("{n:#?}") };
        assert!(rows.iter().flatten().all(|c| c.len() == 1), "{rows:#?}");
        let n = parse_latex(r"\hat{ab}\underbrace{ab}\underset{ab}{cd}");
        let runs = |a: &Arg| a.iter().filter(|n| matches!(n, MNode::Run(_))).count();
        assert!(matches!(&n[0], MNode::Acc { e, .. } if runs(e) == 1), "{n:#?}");
        assert!(matches!(&n[1], MNode::GroupChr { e, .. } if runs(e) == 1), "{n:#?}");
        assert!(matches!(&n[2], MNode::Lim { e, lim, .. } if runs(e) == 1 && runs(lim) == 1), "{n:#?}");
    }

    #[test]
    fn multi_letter_scripts_round_trip() {
        // Word for Mac 16.113.4 (#1609): a script is the whole run up to a space or operator, and Word's
        // Linear format shows it bare (`V_dd`); the parenthesised form reads back identically.
        let script = |kind, sub: &str, sup: &str| MNode::Script { kind, base: vec![run("V")], sub: parse_linear(sub), sup: parse_linear(sup) };
        for n in [
            vec![script(ScriptKind::Sub, "dd", "")],
            vec![script(ScriptKind::Sup, "", "ab")],
            vec![script(ScriptKind::SubSup, "in", "2")],
            vec![MNode::Nary {
                chr: '∑',
                lim_loc: None,
                grow: false,
                sub_hide: false,
                sup_hide: false,
                sub: vec![run("ij")],
                sup: vec![run("n")],
                e: vec![run("x")],
            }],
            vec![MNode::Lim { upper: false, e: vec![run("⟶")], lim: vec![run("yields")] }],
        ] {
            assert_eq!(parse_linear(&to_linear(&n)), n, "{}", to_linear(&n));
        }
        // Numbers and single letters stay bare.
        assert_eq!(to_linear(&[script(ScriptKind::SubSup, "10", "2.5")]), "V_10^2.5");
        assert_eq!(to_linear(&[script(ScriptKind::Sub, "d", "")]), "V_d");
        assert_eq!(to_linear(&[script(ScriptKind::Sub, "dd", "")]), "V_dd");
        assert_eq!(to_linear(&[script(ScriptKind::Sup, "", "ab")]), "V^ab");
    }

    #[test]
    fn multi_letter_script_operands_follow_word() {
        // #1609, Word for Mac 16.113.4: `V_dd` and `V_(dd)` build up the same; `dd` is all subscript.
        let sub_dd = MNode::Script { kind: ScriptKind::Sub, base: vec![run("V")], sub: vec![run("dd")], sup: vec![] };
        assert_eq!(parse_linear("V_dd"), vec![sub_dd.clone()]);
        assert_eq!(parse_linear("V_(dd)"), vec![sub_dd]);
        let sup_ab = MNode::Script { kind: ScriptKind::Sup, base: vec![run("x")], sub: vec![], sup: vec![run("ab")] };
        assert_eq!(parse_linear("x^ab"), vec![sup_ab.clone()]);
        assert_eq!(parse_linear("x^(ab)"), vec![sup_ab]);
        // Delimiters still end the operand.
        let both = MNode::Script { kind: ScriptKind::SubSup, base: vec![run("x")], sub: vec![run("i")], sup: vec![run("2")] };
        assert_eq!(parse_linear("x_i^2"), vec![both]);
        let a_i = MNode::Script { kind: ScriptKind::Sub, base: vec![run("a")], sub: vec![run("i")], sup: vec![] };
        assert_eq!(parse_linear("a_i b"), vec![a_i, run("b")]);
        let both2 = MNode::Script { kind: ScriptKind::SubSup, base: vec![run("x")], sub: vec![run("ij")], sup: vec![run("kl")] };
        assert_eq!(parse_linear("x_ij^kl"), vec![both2]);
        // Round trip is Word's bare form.
        assert_eq!(to_linear(&parse_linear("V_(dd)")), "V_dd");
    }

    #[test]
    fn a_bare_script_does_not_absorb_what_follows() {
        // #1609 review: `x_i` then `y` must not read back as x with subscript `iy`.
        let sc =
            |kind, base: &str, sub: &str, sup: &str| MNode::Script { kind, base: vec![run(base)], sub: parse_linear(sub), sup: parse_linear(sup) };
        let sin_x = MNode::Func { name: vec![MNode::Run(MRun::plain("sin"))], e: vec![run("x")] };
        let lim = MNode::Lim { upper: false, e: vec![run("a")], lim: vec![run("n")] };
        for n in [
            vec![sc(ScriptKind::Sub, "x", "i", ""), run("y")],
            vec![sc(ScriptKind::Sub, "a", "i", ""), sc(ScriptKind::Sub, "b", "j", "")],
            vec![sc(ScriptKind::Sup, "x", "", "2"), run("y")],
            vec![sc(ScriptKind::Sup, "x", "", "2"), run("3")],
            vec![sc(ScriptKind::Sub, "x", "i", ""), sin_x],
            vec![lim.clone(), run("y")],
            vec![sc(ScriptKind::Sub, "x", "ab", ""), run("y")],
            vec![sc(ScriptKind::Sub, "x", "i", ""), run(".5")],
        ] {
            let lin = to_linear(&n);
            assert_eq!(parse_linear(&lin), n, "{lin}");
        }
        assert_eq!(to_linear(&[sc(ScriptKind::Sub, "x", "i", ""), run("y")]), "x_i y");
        // Nothing extra when the script already ends in a bracket or a symbol follows.
        assert_eq!(to_linear(&[sc(ScriptKind::Sub, "x", "i", ""), run("+")]), "x_i+");
    }

    #[test]
    fn a_function_name_in_a_script_stays_upright() {
        // Word for Mac (#1609): `x_max` and `x_(max)` both give an upright function name.
        let up = vec![MNode::Script { kind: ScriptKind::Sub, base: vec![run("x")], sub: vec![MNode::Run(MRun::plain("max"))], sup: vec![] }];
        assert_eq!(parse_linear("x_max"), up);
        assert_eq!(parse_linear("x_(max)"), up);
        assert_eq!(parse_linear(&to_linear(&up)), up, "{}", to_linear(&up));
        // Other upright text is not a function name: it keeps its parentheses.
        let other = vec![MNode::Script { kind: ScriptKind::Sub, base: vec![run("x")], sub: vec![MNode::Run(MRun::plain("ab"))], sup: vec![] }];
        assert_eq!(to_linear(&other), "x_(ab)");
    }

    #[test]
    fn digits_then_letters_continue_a_script() {
        // Word for Mac (#1609): a script runs to a space or operator, so `x_2a` is subscript "2a".
        let sub = |t: &str| vec![MNode::Script { kind: ScriptKind::Sub, base: vec![run("x")], sub: vec![run(t)], sup: vec![] }];
        assert_eq!(parse_linear("x_2a"), sub("2a"));
        assert_eq!(parse_linear("x_a2"), sub("a2"));
        assert_eq!(parse_linear("x_αβ"), sub("αβ"));
        assert_eq!(parse_linear("x_10"), sub("10"));
        assert_eq!(parse_linear("x_2.5"), sub("2.5"));
        assert_eq!(to_linear(&sub("2a")), "x_2a");
        assert_eq!(parse_linear(&to_linear(&sub("2a"))), sub("2a"));
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
