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

/// Integral signs (their limits default to sub/sup positions).
pub fn is_integral(c: char) -> bool {
    matches!(c, '∫' | '∬' | '∭' | '∮' | '∯' | '∰' | '∱' | '∲' | '∳' | '⨌')
}

/// Characters that take n-ary operator form in the linear format.
pub fn is_nary_char(c: char) -> bool {
    is_integral(c) || matches!(c, '∑' | '∏' | '∐' | '⋃' | '⋂' | '⋁' | '⋀' | '⨁' | '⨂' | '⨀' | '⨄' | '⨆')
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
                s.push('(');
                s.push('_');
                lin_operand(sub, s, depth);
                s.push('^');
                lin_operand(sup, s, depth);
                s.push(')');
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
        MNode::Delim { beg, end, sep, elems, .. } => {
            s.push(beg.unwrap_or('〖'));
            for (i, e) in elems.iter().enumerate() {
                if i > 0 {
                    s.push(sep.unwrap_or('│'));
                }
                lin_arg(e, s, depth);
            }
            s.push(end.unwrap_or('〗'));
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

/// Parse linear-format text (a practical subset of UnicodeMath: `a/b`, `x^2`, `x_i`, `√x`,
/// `√(n&x)`, `∑_(i=1)^n▒…`, brackets, function names) into nodes. Never fails: what isn't
/// understood stays as text.
pub fn parse_linear(s: &str) -> Arg {
    let chars: Vec<char> = s.chars().take(10_000).collect();
    let mut p = LinParser { c: chars, i: 0, depth: 0 };
    let out = p.seq(&[]);
    merge_runs(out)
}

/// Function names set upright and spaced as operators.
pub const FUNCTION_NAMES: &[&str] = &[
    "arccos", "arcsin", "arctan", "arg", "cos", "cosh", "cot", "coth", "csc", "csch", "def", "deg", "det", "dim", "exp", "gcd", "hom", "inf", "ker",
    "lg", "lim", "liminf", "limsup", "ln", "log", "max", "min", "mod", "Pr", "sec", "sech", "sin", "sinh", "sup", "tan", "tanh",
];

struct LinParser {
    c: Vec<char>,
    i: usize,
    depth: usize,
}

fn opening(c: char) -> Option<char> {
    Some(match c {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '⟨' => '⟩',
        '⌈' => '⌉',
        '⌊' => '⌋',
        '〖' => '〗',
        _ => return None,
    })
}

/// Characters that end an operand (fraction numerator/denominator boundaries).
fn is_operator(c: char) -> bool {
    "+-−±∓=<>≤≥≠≈≡∼≅∝×÷·∙→←↔⇒⇐⇔∈∉⊂⊃⊆⊇∪∩∧∨,;:!".contains(c)
}

impl LinParser {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    /// A sequence until one of `stops` (not consumed) or the end.
    fn seq(&mut self, stops: &[char]) -> Arg {
        let mut out: Arg = Vec::new();
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            // Too deep: the rest is text.
            let rest: String = self.c.get(self.i..).unwrap_or(&[]).iter().collect();
            self.i = self.c.len();
            self.depth -= 1;
            return vec![MNode::Run(MRun::new(rest))];
        }
        while let Some(c) = self.peek() {
            if stops.contains(&c) {
                break;
            }
            match c {
                '/' | '⁄' | '∕' | '¦' => {
                    self.i += 1;
                    // Numerator: the operand at the end of `out`.
                    let start = operand_start(&out);
                    let num: Arg = out.drain(start..).collect();
                    let num = strip_parens(num);
                    let den = self.operand(stops);
                    let kind = match c {
                        '⁄' => FracKind::Skewed,
                        '∕' => FracKind::Linear,
                        '¦' => FracKind::NoBar,
                        _ => FracKind::Bar,
                    };
                    out.push(MNode::Frac { kind, num, den });
                }
                '^' | '_' => {
                    self.i += 1;
                    let script = self.script_operand(stops);
                    let base = out.pop();
                    out.push(attach_script(base, c == '^', script));
                }
                '√' | '∛' | '∜' => {
                    self.i += 1;
                    let mut deg = match c {
                        '∛' => vec![MNode::Run(MRun::new("3"))],
                        '∜' => vec![MNode::Run(MRun::new("4"))],
                        _ => Vec::new(),
                    };
                    let mut e = self.operand(stops);
                    // √(n&x): degree and base.
                    if let Some(k) = e.iter().position(|n| matches!(n, MNode::Run(r) if r.text == "&")) {
                        let rest = e.split_off(k + 1);
                        e.pop();
                        deg = e;
                        e = rest;
                    }
                    let deg_hide = deg.is_empty();
                    out.push(MNode::Rad { deg, deg_hide, e });
                }
                c if is_nary_char(c) => {
                    self.i += 1;
                    let (mut sub, mut sup) = (Vec::new(), Vec::new());
                    loop {
                        match self.peek() {
                            Some('_') => {
                                self.i += 1;
                                sub = self.script_operand(stops);
                            }
                            Some('^') => {
                                self.i += 1;
                                sup = self.script_operand(stops);
                            }
                            _ => break,
                        }
                    }
                    if self.peek() == Some('▒') {
                        self.i += 1;
                    }
                    let e = self.operand(stops);
                    out.push(MNode::Nary { chr: c, lim_loc: None, grow: false, sub_hide: sub.is_empty(), sup_hide: sup.is_empty(), sub, sup, e });
                }
                c if opening(c).is_some() => {
                    self.i += 1;
                    let close = opening(c).unwrap_or(')');
                    let inner = self.seq(&[close]);
                    let closed = self.peek() == Some(close);
                    if closed {
                        self.i += 1;
                    }
                    let beg = if c == '〖' { None } else { Some(c) };
                    let end = if close == '〗' || !closed { None } else { Some(close) };
                    out.push(MNode::Delim { beg, end, sep: None, grow: true, shp_match: false, elems: vec![inner] });
                }
                '█' | '■' => {
                    self.i += 1;
                    if self.peek() == Some('(') {
                        self.i += 1;
                        let inner = self.seq(&[')']);
                        if self.peek() == Some(')') {
                            self.i += 1;
                        }
                        out.push(grid(c == '■', inner));
                    } else {
                        out.push(MNode::Run(MRun::new(c.to_string())));
                    }
                }
                '▒' => self.i += 1,
                c if c.is_ascii_alphabetic() => {
                    // A known function name applies to the next operand.
                    let start = self.i;
                    while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                        self.i += 1;
                    }
                    let word: String = self.c.get(start..self.i).unwrap_or(&[]).iter().collect();
                    let func = FUNCTION_NAMES.iter().find(|f| word.starts_with(**f) && (word.len() == f.len() || self.peek().is_none()));
                    match func {
                        Some(f) if word.len() == f.len() && self.peek().is_some_and(|c| c != '/' && !stops.contains(&c)) => {
                            if self.peek() == Some('\u{2061}') || self.peek() == Some(' ') {
                                self.i += 1;
                            }
                            let name = vec![MNode::Run(MRun::plain(word.clone()))];
                            if word == "lim" && self.peek() == Some('_') {
                                self.i += 1;
                                let lim = self.script_operand(stops);
                                let base = MNode::Lim { upper: false, e: name, lim };
                                let e = self.operand(stops);
                                out.push(MNode::Func { name: vec![base], e });
                            } else {
                                let e = self.operand(stops);
                                out.push(MNode::Func { name, e });
                            }
                        }
                        Some(_) if word.len() <= 6 && FUNCTION_NAMES.contains(&word.as_str()) => {
                            out.push(MNode::Run(MRun::plain(word)));
                        }
                        _ => {
                            for ch in word.chars() {
                                out.push(MNode::Run(MRun::new(ch.to_string())));
                            }
                        }
                    }
                }
                ' ' => self.i += 1,
                c => {
                    self.i += 1;
                    out.push(MNode::Run(MRun::new(c.to_string())));
                }
            }
        }
        self.depth -= 1;
        out
    }

    /// An operand: a bracketed group (brackets dropped) or a run of ordinary atoms.
    fn operand(&mut self, stops: &[char]) -> Arg {
        match self.peek() {
            Some('(') => {
                self.i += 1;
                let inner = self.seq(&[')']);
                if self.peek() == Some(')') {
                    self.i += 1;
                }
                let mut v = inner;
                self.trailing_scripts(&mut v, stops);
                v
            }
            Some(c) if c == '√' || is_nary_char(c) || c == '█' || c == '■' || opening(c).is_some() => {
                let mut one = self.one(stops);
                self.trailing_scripts(&mut one, stops);
                one
            }
            _ => {
                let mut out = Vec::new();
                let start = self.i;
                while let Some(c) = self.peek() {
                    if stops.contains(&c) || is_operator(c) || c == '/' || c == ' ' || opening(c).is_some() || c == ')' || c == '▒' {
                        break;
                    }
                    if c == '^' || c == '_' {
                        self.i += 1;
                        let s = self.script_operand(stops);
                        let base = out.pop();
                        out.push(attach_script(base, c == '^', s));
                        continue;
                    }
                    if c == '√' || is_nary_char(c) {
                        break;
                    }
                    self.i += 1;
                    out.push(MNode::Run(MRun::new(c.to_string())));
                }
                if self.i == start
                    && let Some(c) = self.peek()
                    && !stops.contains(&c)
                    && c != ')'
                {
                    // A lone operator or space as operand.
                    self.i += 1;
                    if c != ' ' {
                        out.push(MNode::Run(MRun::new(c.to_string())));
                    }
                }
                merge_runs(out)
            }
        }
    }

    /// One structure starting at the cursor (radical, n-ary, bracket group).
    fn one(&mut self, stops: &[char]) -> Arg {
        let start = self.i;
        let end = self.structure_end(stops, 0);
        if self.depth > MAX_DEPTH {
            let t: String = self.c.get(start..end).unwrap_or(&[]).iter().collect();
            return vec![MNode::Run(MRun::new(t))];
        }
        let slice: Vec<char> = self.c.get(start..end).unwrap_or(&[]).to_vec();
        let mut p = LinParser { c: slice, i: 0, depth: self.depth };
        p.seq(&[])
    }

    /// Index just past the structure starting at the cursor (the cursor moves there).
    fn structure_end(&mut self, stops: &[char], lvl: usize) -> usize {
        let Some(c) = self.peek() else { return self.i };
        if lvl > MAX_DEPTH {
            self.i = self.c.len();
            return self.i;
        }
        self.i += 1;
        if let Some(close) = opening(c) {
            self.skip_group(c, close);
            return self.i;
        }
        if c == '█' || c == '■' {
            if self.peek() == Some('(') {
                self.i += 1;
                self.skip_group('(', ')');
            }
            return self.i;
        }
        if is_nary_char(c) {
            while let Some(k) = self.peek() {
                if k == '_' || k == '^' {
                    self.i += 1;
                    self.skip_operand(stops, lvl + 1);
                } else {
                    break;
                }
            }
            if self.peek() == Some('▒') {
                self.i += 1;
            }
        }
        // √ and n-ary take an operand.
        self.skip_operand(stops, lvl + 1);
        self.i
    }

    fn skip_group(&mut self, open: char, close: char) {
        let mut level = 1usize;
        while let Some(c) = self.peek() {
            self.i += 1;
            if c == open {
                level += 1;
            } else if c == close {
                level -= 1;
                if level == 0 {
                    return;
                }
            }
        }
    }

    fn skip_operand(&mut self, stops: &[char], lvl: usize) {
        match self.peek() {
            Some(c) if opening(c).is_some() => {
                self.i += 1;
                self.skip_group(c, opening(c).unwrap_or(')'));
            }
            Some(c) if c == '√' || is_nary_char(c) => {
                let _ = self.structure_end(stops, lvl + 1);
            }
            _ => {
                while let Some(c) = self.peek() {
                    if stops.contains(&c) || is_operator(c) || c == '/' || c == ' ' || opening(c).is_some() || c == ')' || c == '▒' {
                        break;
                    }
                    self.i += 1;
                }
            }
        }
    }

    fn trailing_scripts(&mut self, v: &mut Arg, stops: &[char]) {
        while let Some(c @ ('^' | '_')) = self.peek() {
            self.i += 1;
            let s = self.script_operand(stops);
            let base = if v.len() == 1 {
                v.pop()
            } else {
                Some(MNode::Delim { beg: None, end: None, sep: None, grow: true, shp_match: false, elems: vec![std::mem::take(v)] })
            };
            v.push(attach_script(base, c == '^', s));
        }
    }

    /// A script: a bracketed group or a single atom (letters and digits run together).
    fn script_operand(&mut self, stops: &[char]) -> Arg {
        match self.peek() {
            Some('(') => {
                self.i += 1;
                let inner = self.seq(&[')']);
                if self.peek() == Some(')') {
                    self.i += 1;
                }
                inner
            }
            Some(c) if c == '√' || is_nary_char(c) || opening(c).is_some() => self.one(stops),
            Some(c) if c.is_alphanumeric() => {
                let start = self.i;
                let digits = c.is_ascii_digit();
                while self.peek().is_some_and(|k| if digits { k.is_ascii_digit() || k == '.' } else { k.is_alphanumeric() && !k.is_ascii_digit() }) {
                    self.i += 1;
                    if !digits {
                        break;
                    }
                }
                let t: String = self.c.get(start..self.i).unwrap_or(&[]).iter().collect();
                vec![MNode::Run(MRun::new(t))]
            }
            Some(c) if !stops.contains(&c) => {
                self.i += 1;
                vec![MNode::Run(MRun::new(c.to_string()))]
            }
            _ => Vec::new(),
        }
    }
}

/// Where the trailing operand of `out` starts (after the last operator run).
fn operand_start(out: &[MNode]) -> usize {
    let mut k = out.len();
    while k > 0 {
        match out.get(k - 1) {
            Some(MNode::Run(r)) if r.text.chars().all(is_operator) || r.text == " " => break,
            _ => k -= 1,
        }
    }
    // An empty numerator takes the last atom anyway.
    if k == out.len() { out.len().saturating_sub(1) } else { k }
}

/// A single bracket group used as an operand loses its (grouping) brackets.
fn strip_parens(mut a: Arg) -> Arg {
    if a.len() == 1
        && let Some(MNode::Delim { beg: Some('('), end: Some(')'), elems, .. }) = a.first_mut()
        && elems.len() == 1
    {
        return elems.pop().unwrap_or_default();
    }
    a
}

fn attach_script(base: Option<MNode>, sup: bool, s: Arg) -> MNode {
    match base {
        Some(MNode::Script { kind: ScriptKind::Sub, base, sub, .. }) if sup => MNode::Script { kind: ScriptKind::SubSup, base, sub, sup: s },
        Some(MNode::Script { kind: ScriptKind::Sup, base, sup: sp, .. }) if !sup => MNode::Script { kind: ScriptKind::SubSup, base, sub: s, sup: sp },
        Some(b) => {
            if sup {
                MNode::Script { kind: ScriptKind::Sup, base: vec![b], sub: Vec::new(), sup: s }
            } else {
                MNode::Script { kind: ScriptKind::Sub, base: vec![b], sub: s, sup: Vec::new() }
            }
        }
        None => {
            if sup {
                MNode::Script { kind: ScriptKind::Sup, base: Vec::new(), sub: Vec::new(), sup: s }
            } else {
                MNode::Script { kind: ScriptKind::Sub, base: Vec::new(), sub: s, sup: Vec::new() }
            }
        }
    }
}

/// `█(a@b)` equation array / `■(a&b@c&d)` matrix from the parsed inside.
fn grid(matrix: bool, inner: Arg) -> MNode {
    let mut rows: Vec<Vec<Arg>> = vec![vec![Vec::new()]];
    for n in inner {
        match &n {
            MNode::Run(r) if r.text == "@" => rows.push(vec![Vec::new()]),
            MNode::Run(r) if r.text == "&" && matrix => {
                if let Some(row) = rows.last_mut() {
                    row.push(Vec::new());
                }
            }
            _ => {
                if let Some(cell) = rows.last_mut().and_then(|r| r.last_mut()) {
                    cell.push(n);
                }
            }
        }
    }
    if matrix {
        let rows = rows.into_iter().map(|r| r.into_iter().map(merge_runs).collect()).collect();
        MNode::Matrix { rows, col_jc: Vec::new() }
    } else {
        MNode::EqArr { rows: rows.into_iter().map(|r| merge_runs(r.into_iter().flatten().collect())).collect() }
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
        for s in ["x=(-b±√(b^2-4ac))/2a", "∑_(i=1)^n▒i", "a/b+c", "E=mc^2", "√(3&x)"] {
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
