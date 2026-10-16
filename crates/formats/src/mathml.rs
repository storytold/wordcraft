//! Equations as MathML (W3C MathML Core / Presentation MathML): written into HTML export and the
//! HTML a copy puts on the system clipboard, so programs that read MathML can get a formatted
//! equation instead of its linear text (#816; pasting into Word and LibreOffice not yet verified;
//! OMML for Word is #1297); and read back from pasted or opened HTML.
//!
//! The writer walks the equation tree ([`MNode`]) and wraps it in `<semantics>` with the linear
//! format as an `application/x-unicodemath` annotation, which renderers ignore and WordCraft reads
//! back exactly. The reader takes that annotation when present, else a TeX annotation, else
//! builds the tree from the presentation elements (fractions, scripts, radicals, under/over
//! limits, tables, fences); both bound their nesting and size.

use wordcraft_doc::math::{
    Arg, ColJc, FracKind, LimLoc, MAX_DEPTH, MClass, MNode, MRun, MScr, MSty, ScriptKind, is_integral, is_nary_char, math_alnum, math_class,
    to_linear,
};
use wordcraft_doc::math_linear::FUNCTION_NAMES;

use crate::html::esc;

/// The MathML namespace.
pub const NS: &str = "http://www.w3.org/1998/Math/MathML";
/// The annotation encoding carrying the linear format (UnicodeMath).
pub const UNICODEMATH: &str = "application/x-unicodemath";

// ---------------------------------------------------------------------------------------------
// Writer

/// An equation as a `<math>` element. `linear` (the linear format) goes in an annotation for an
/// exact round trip; empty = taken from the tree.
pub fn to_mathml(nodes: &[MNode], display: bool, linear: &str) -> String {
    let mut o = String::new();
    o.push_str(&format!("<math xmlns=\"{NS}\""));
    if display {
        o.push_str(" display=\"block\"");
    }
    o.push_str("><semantics><mrow>");
    arg(nodes, &mut o, 0, false);
    o.push_str("</mrow>");
    let linear = if linear.is_empty() { to_linear(nodes) } else { linear.to_string() };
    if !linear.is_empty() {
        o.push_str(&format!("<annotation encoding=\"{UNICODEMATH}\">{}</annotation>", esc(&linear)));
    }
    o.push_str("</semantics></math>");
    o
}

/// An equation known by its linear format, as a `<math>` element.
pub fn linear_to_mathml(linear: &str, display: bool) -> String {
    to_mathml(&wordcraft_doc::math::parse_linear(linear), display, linear)
}

/// An argument as a row of elements. `eq_arr`: a row of an equation array, whose `&` marks are
/// alignment points, not text.
fn arg(a: &[MNode], o: &mut String, depth: usize, eq_arr: bool) {
    if depth > MAX_DEPTH {
        return;
    }
    for n in a {
        node(n, o, depth + 1, eq_arr);
    }
}

/// An argument as one element (`<mrow>` around several).
fn group(a: &[MNode], o: &mut String, depth: usize) {
    o.push_str("<mrow>");
    arg(a, o, depth, false);
    o.push_str("</mrow>");
}

fn mo(c: char, attrs: &str, o: &mut String) {
    o.push_str(&format!("<mo{attrs}>{}</mo>", esc(&c.to_string())));
}

fn run(r: &MRun, o: &mut String, eq_arr: bool) {
    let color = r.color.map(|c| format!(" mathcolor=\"#{}\"", c.hex())).unwrap_or_default();
    if r.nor {
        if !r.text.is_empty() {
            o.push_str(&format!("<mtext{color}>{}</mtext>", esc(&r.text)));
        }
        return;
    }
    let bold = matches!(r.sty, Some(MSty::Bold | MSty::BoldItalic));
    let italic = match r.sty {
        None => r.scr == MScr::Roman,
        Some(s) => matches!(s, MSty::Italic | MSty::BoldItalic),
    };
    let cs: Vec<char> = r.text.chars().collect();
    let mut i = 0;
    while let Some(&c) = cs.get(i) {
        if c.is_ascii_digit() || (c == '.' && cs.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let mut num = String::new();
            while let Some(&d) =
                cs.get(i).filter(|d| d.is_ascii_digit() || **d == '.' || **d == ',' && cs.get(i + 1).is_some_and(|n| n.is_ascii_digit()))
            {
                num.push(d);
                i += 1;
            }
            let num: String = if bold { num.chars().map(|d| math_alnum(d, true, false, MScr::Roman)).collect() } else { num };
            o.push_str(&format!("<mn{color}>{}</mn>", esc(&num)));
            continue;
        }
        i += 1;
        if c.is_whitespace() || (eq_arr && c == '&') {
            continue;
        }
        if c.is_alphabetic() {
            if bold || r.scr != MScr::Roman {
                let m = math_alnum(c, bold, italic, r.scr);
                let plain = if m == c && !italic { " mathvariant=\"normal\"" } else { "" };
                o.push_str(&format!("<mi{plain}{color}>{}</mi>", esc(&m.to_string())));
            } else if italic {
                o.push_str(&format!("<mi{color}>{}</mi>", esc(&c.to_string())));
            } else {
                o.push_str(&format!("<mi mathvariant=\"normal\"{color}>{}</mi>", esc(&c.to_string())));
            }
        } else if math_class(c) == MClass::Ord && !c.is_ascii_punctuation() && c != '\u{2061}' {
            // ∞, ∂, ∅, ℏ…: identifiers, not operators.
            o.push_str(&format!("<mi{color}>{}</mi>", esc(&c.to_string())));
        } else {
            mo(c, &color, o);
        }
    }
}

/// A function name (`sin`, `lim`): one upright identifier when it's plain text.
fn func_name(name: &[MNode], o: &mut String, depth: usize) {
    match name {
        [MNode::Run(r)] if !r.nor && r.text.chars().all(char::is_alphabetic) && !r.text.is_empty() => {
            o.push_str(&format!("<mi>{}</mi>", esc(&r.text)));
        }
        _ => group(name, o, depth),
    }
}

fn node(n: &MNode, o: &mut String, depth: usize, eq_arr: bool) {
    match n {
        MNode::Run(r) => run(r, o, eq_arr),
        MNode::Frac { kind, num, den } => match kind {
            FracKind::Bar | FracKind::NoBar => {
                o.push_str(if *kind == FracKind::NoBar { "<mfrac linethickness=\"0\">" } else { "<mfrac>" });
                group(num, o, depth);
                group(den, o, depth);
                o.push_str("</mfrac>");
            }
            FracKind::Skewed | FracKind::Linear => {
                o.push_str("<mrow>");
                group(num, o, depth);
                mo(if *kind == FracKind::Skewed { '⁄' } else { '/' }, "", o);
                group(den, o, depth);
                o.push_str("</mrow>");
            }
        },
        MNode::Script { kind, base, sub, sup } => match kind {
            ScriptKind::Sup => {
                o.push_str("<msup>");
                group(base, o, depth);
                group(sup, o, depth);
                o.push_str("</msup>");
            }
            ScriptKind::Sub => {
                o.push_str("<msub>");
                group(base, o, depth);
                group(sub, o, depth);
                o.push_str("</msub>");
            }
            ScriptKind::SubSup => {
                o.push_str("<msubsup>");
                group(base, o, depth);
                group(sub, o, depth);
                group(sup, o, depth);
                o.push_str("</msubsup>");
            }
            ScriptKind::Pre => {
                o.push_str("<mmultiscripts>");
                group(base, o, depth);
                o.push_str("<mprescripts/>");
                group(sub, o, depth);
                group(sup, o, depth);
                o.push_str("</mmultiscripts>");
            }
        },
        MNode::Rad { deg, deg_hide, e } => {
            if *deg_hide || deg.is_empty() {
                o.push_str("<msqrt>");
                arg(e, o, depth, false);
                o.push_str("</msqrt>");
            } else {
                o.push_str("<mroot>");
                group(e, o, depth);
                group(deg, o, depth);
                o.push_str("</mroot>");
            }
        }
        MNode::Nary { chr, lim_loc, sub_hide, sup_hide, sub, sup, e, .. } => {
            let under_over = match lim_loc {
                Some(l) => *l == LimLoc::UndOvr,
                None => !is_integral(*chr),
            };
            let has_sub = !*sub_hide && !sub.is_empty();
            let has_sup = !*sup_hide && !sup.is_empty();
            let tag = match (under_over, has_sub, has_sup) {
                (_, false, false) => "",
                (true, true, true) => "munderover",
                (true, true, false) => "munder",
                (true, false, true) => "mover",
                (false, true, true) => "msubsup",
                (false, true, false) => "msub",
                (false, false, true) => "msup",
            };
            o.push_str("<mrow>");
            if !tag.is_empty() {
                o.push_str(&format!("<{tag}>"));
            }
            mo(*chr, " largeop=\"true\" movablelimits=\"false\"", o);
            if has_sub {
                group(sub, o, depth);
            }
            if has_sup {
                group(sup, o, depth);
            }
            if !tag.is_empty() {
                o.push_str(&format!("</{tag}>"));
            }
            group(e, o, depth);
            o.push_str("</mrow>");
        }
        MNode::Delim { beg, end, sep, grow, elems, .. } => {
            let stretchy = if *grow { "" } else { " stretchy=\"false\"" };
            o.push_str("<mrow>");
            if let Some(b) = beg {
                mo(*b, &format!(" fence=\"true\" form=\"prefix\"{stretchy}"), o);
            }
            for (i, e) in elems.iter().enumerate() {
                if i > 0 {
                    mo(sep.unwrap_or('|'), &format!(" separator=\"true\"{stretchy}"), o);
                }
                group(e, o, depth);
            }
            if let Some(c) = end {
                mo(*c, &format!(" fence=\"true\" form=\"postfix\"{stretchy}"), o);
            }
            o.push_str("</mrow>");
        }
        MNode::Func { name, e } => {
            o.push_str("<mrow>");
            func_name(name, o, depth);
            mo('\u{2061}', "", o);
            group(e, o, depth);
            o.push_str("</mrow>");
        }
        MNode::Lim { upper, e, lim } => {
            let tag = if *upper { "mover" } else { "munder" };
            o.push_str(&format!("<{tag}>"));
            func_name(e, o, depth);
            group(lim, o, depth);
            o.push_str(&format!("</{tag}>"));
        }
        MNode::Acc { chr, e } => {
            o.push_str("<mover accent=\"true\">");
            group(e, o, depth);
            mo(spacing_accent(*chr), "", o);
            o.push_str("</mover>");
        }
        MNode::Bar { top, e } => {
            if *top {
                o.push_str("<mover accent=\"true\">");
                group(e, o, depth);
                mo('\u{203E}', "", o);
                o.push_str("</mover>");
            } else {
                o.push_str("<munder accentunder=\"true\">");
                group(e, o, depth);
                mo('_', "", o);
                o.push_str("</munder>");
            }
        }
        MNode::BorderBox { hide, strike, e } => {
            let mut notation: Vec<&str> = Vec::new();
            for (h, name) in hide.iter().zip(["top", "bottom", "left", "right"]) {
                if !h {
                    notation.push(name);
                }
            }
            if notation.len() == 4 {
                notation = vec!["box"];
            }
            for (s, name) in strike.iter().zip(["horizontalstrike", "verticalstrike", "updiagonalstrike", "downdiagonalstrike"]) {
                if *s {
                    notation.push(name);
                }
            }
            o.push_str(&format!("<menclose notation=\"{}\">", notation.join(" ")));
            arg(e, o, depth, false);
            o.push_str("</menclose>");
        }
        MNode::Boxed { e } => group(e, o, depth),
        MNode::GroupChr { chr, top, e } => {
            let tag = if *top { "mover" } else { "munder" };
            o.push_str(&format!("<{tag}>"));
            group(e, o, depth);
            mo(*chr, " stretchy=\"true\"", o);
            o.push_str(&format!("</{tag}>"));
        }
        MNode::EqArr { rows } => {
            // `displaystyle` marks an equation array; a bare one-column table reads as a matrix.
            o.push_str("<mtable displaystyle=\"true\">");
            for r in rows {
                o.push_str("<mtr><mtd>");
                arg(r, o, depth, true);
                o.push_str("</mtd></mtr>");
            }
            o.push_str("</mtable>");
        }
        MNode::Matrix { rows, col_jc } => {
            o.push_str("<mtable>");
            for r in rows {
                o.push_str("<mtr>");
                for (k, cell) in r.iter().enumerate() {
                    match col_jc.get(k) {
                        Some(ColJc::Left) => o.push_str("<mtd columnalign=\"left\">"),
                        Some(ColJc::Right) => o.push_str("<mtd columnalign=\"right\">"),
                        _ => o.push_str("<mtd>"),
                    }
                    arg(cell, o, depth, false);
                    o.push_str("</mtd>");
                }
                o.push_str("</mtr>");
            }
            o.push_str("</mtable>");
        }
        MNode::Phant { show, zero_wid, zero_asc, zero_desc, e } => {
            let mut pad = String::new();
            for (z, a) in [(zero_wid, "width"), (zero_asc, "height"), (zero_desc, "depth")] {
                if *z {
                    pad.push_str(&format!(" {a}=\"0\""));
                }
            }
            if !pad.is_empty() {
                o.push_str(&format!("<mpadded{pad}>"));
            }
            o.push_str(if *show { "<mrow>" } else { "<mphantom>" });
            arg(e, o, depth, false);
            o.push_str(if *show { "</mrow>" } else { "</mphantom>" });
            if !pad.is_empty() {
                o.push_str("</mpadded>");
            }
        }
    }
}

/// Combining accents (how equations keep them) and the spacing characters MathML puts over the
/// base, both ways.
const ACCENTS: &[(char, char)] = &[
    ('\u{0302}', '^'),
    ('\u{0303}', '~'),
    ('\u{0304}', '¯'),
    ('\u{0305}', '\u{203E}'),
    ('\u{0306}', '˘'),
    ('\u{0307}', '˙'),
    ('\u{0308}', '¨'),
    ('\u{030C}', 'ˇ'),
    ('\u{0301}', '´'),
    ('\u{0300}', '`'),
    ('\u{030A}', '˚'),
    ('\u{20D7}', '→'),
    ('\u{20D6}', '←'),
    ('\u{20E1}', '↔'),
    ('\u{20D1}', '⇀'),
    ('\u{20D0}', '↼'),
];

fn spacing_accent(c: char) -> char {
    ACCENTS.iter().find(|(comb, _)| *comb == c).map(|(_, s)| *s).unwrap_or(c)
}

fn combining_accent(c: char) -> Option<char> {
    let c = match c {
        'ˆ' => '^',
        '˜' => '~',
        '‾' => '\u{203E}',
        '\u{20D7}' | '\u{0302}' | '\u{0303}' | '\u{0307}' | '\u{0308}' => return Some(c),
        c => c,
    };
    ACCENTS.iter().find(|(_, s)| *s == c).map(|(comb, _)| *comb)
}

// ---------------------------------------------------------------------------------------------
// Reader

/// Most elements read from one `<math>` element.
pub const MAX_NODES: usize = 20_000;

/// A parsed MathML element (namespace prefixes dropped, names lowercase).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct El {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    El(El),
    Text(String),
}

impl El {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    fn els(&self) -> impl Iterator<Item = &El> {
        self.children.iter().filter_map(|c| match c {
            Item::El(e) => Some(e),
            Item::Text(_) => None,
        })
    }
    /// The text inside, all descendants (depth-limited).
    pub fn text(&self) -> String {
        let mut s = String::new();
        text_into(self, &mut s, 0);
        s
    }
}

fn text_into(e: &El, s: &mut String, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for c in &e.children {
        match c {
            Item::Text(t) => s.push_str(t),
            Item::El(c) => text_into(c, s, depth + 1),
        }
    }
}

/// The element name without a namespace prefix (`m:math`, `mml:mfrac`).
pub fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// The linear format of a `<math>` element, and whether it's a display equation. `None` when it
/// holds nothing.
pub fn read(math: &El) -> Option<(String, bool)> {
    let display = math.attr("display").is_some_and(|d| d.eq_ignore_ascii_case("block")) || math.attr("mode").is_some_and(|d| d == "display");
    let linear = linear_of(math);
    let linear: String = linear.chars().filter(|c| !c.is_control()).collect();
    let linear = linear.trim().to_string();
    (!linear.is_empty()).then_some((linear, display))
}

fn linear_of(math: &El) -> String {
    // Our own copy carries the linear format exactly.
    if let Some(sem) = math.els().find(|e| local(&e.name) == "semantics") {
        let ann = |enc: &dyn Fn(&str) -> bool| {
            sem.els()
                .filter(|e| local(&e.name) == "annotation")
                .find(|e| e.attr("encoding").is_some_and(|v| enc(&v.to_ascii_lowercase())))
                .map(El::text)
                .filter(|t| !t.trim().is_empty())
        };
        if let Some(t) = ann(&|v| v == UNICODEMATH) {
            return t;
        }
        let tree = to_tree(math);
        if !tree.is_empty() {
            return to_linear(&tree);
        }
        if let Some(t) = ann(&|v| v.contains("tex")) {
            return to_linear(&wordcraft_doc::math_latex::parse_latex(&t));
        }
        return String::new();
    }
    to_linear(&to_tree(math))
}

/// A MathML element's children as an equation tree.
pub fn to_tree(math: &El) -> Arg {
    let mut budget = MAX_NODES;
    wordcraft_doc::math::merge_runs(children(math, 0, &mut budget))
}

fn children(e: &El, depth: usize, budget: &mut usize) -> Arg {
    let els: Vec<&El> = e.els().collect();
    seq(&els, depth, budget)
}

/// A sequence of elements; an n-ary operator without an operand takes the next element.
fn seq(els: &[&El], depth: usize, budget: &mut usize) -> Arg {
    let mut out: Arg = Vec::new();
    let mut i = 0;
    while let Some(el) = els.get(i) {
        i += 1;
        // A function name (`<mi>sin</mi>`) applies to the next element even without U+2061,
        // so it doesn't merge with it into `sinx`. Before an operator (`(`, U+2061) the runs
        // merge and the linear format finds the function itself.
        if local(&el.name) == "mi"
            && let Some(next) = els.get(i).filter(|n| local(&n.name) != "mo")
        {
            let t = el.text();
            let t = t.trim();
            if FUNCTION_NAMES.contains(&t) && *budget > 0 {
                *budget -= 1;
                i += 1;
                out.push(MNode::Func { name: vec![MNode::Run(MRun::plain(t))], e: elem(next, depth + 1, budget) });
                continue;
            }
        }
        let mut a = elem(el, depth, budget);
        if let [MNode::Nary { e, .. }] = a.as_mut_slice()
            && e.is_empty()
            && let Some(next) = els.get(i)
        {
            i += 1;
            *e = elem(next, depth, budget);
        }
        out.extend(a);
    }
    out
}

/// The `k`th child element as an argument.
fn nth(e: &El, k: usize, depth: usize, budget: &mut usize) -> Arg {
    e.els().nth(k).map(|c| elem(c, depth, budget)).unwrap_or_default()
}

/// A lone operator character (`<mo>∑</mo>`).
fn lone_char(a: &[MNode]) -> Option<char> {
    match a {
        [MNode::Run(r)] => {
            let mut cs = r.text.trim().chars();
            let c = cs.next()?;
            cs.next().is_none().then_some(c)
        }
        _ => None,
    }
}

/// A plain word (`lim`, `max`).
fn word(a: &[MNode]) -> bool {
    matches!(a, [MNode::Run(r)] if r.text.chars().count() > 1 && r.text.chars().all(char::is_alphabetic))
}

fn elem(e: &El, depth: usize, budget: &mut usize) -> Arg {
    if depth > MAX_DEPTH || *budget == 0 {
        return Vec::new();
    }
    *budget -= 1;
    let d = depth + 1;
    match local(&e.name) {
        "mi" | "mn" | "mo" | "ms" => {
            let t = e.text();
            let t = t.trim();
            if t.is_empty() {
                return Vec::new();
            }
            let mut r = MRun::new(t);
            if local(&e.name) == "mi" && e.attr("mathvariant") == Some("normal") && t.chars().count() == 1 {
                r.sty = Some(MSty::Plain);
            }
            vec![MNode::Run(r)]
        }
        "mtext" => {
            let t = e.text();
            if t.trim().is_empty() {
                return Vec::new();
            }
            vec![MNode::Run(MRun { text: t, nor: true, ..Default::default() })]
        }
        "mfrac" => {
            let thin = e.attr("linethickness").is_some_and(|t| {
                let s = t.trim();
                s.eq_ignore_ascii_case("none") || s.trim_end_matches(|c: char| c.is_alphabetic() || c == '%').trim() == "0"
            });
            let kind = if e.attr("bevelled").is_some_and(|b| b.eq_ignore_ascii_case("true")) {
                FracKind::Skewed
            } else if thin {
                FracKind::NoBar
            } else {
                FracKind::Bar
            };
            vec![MNode::Frac { kind, num: nth(e, 0, d, budget), den: nth(e, 1, d, budget) }]
        }
        "msqrt" => vec![MNode::Rad { deg: Vec::new(), deg_hide: true, e: children(e, d, budget) }],
        "mroot" => vec![MNode::Rad { deg: nth(e, 1, d, budget), deg_hide: false, e: nth(e, 0, d, budget) }],
        "msup" | "msub" | "msubsup" => {
            let name = local(&e.name);
            let base = nth(e, 0, d, budget);
            let (sub, sup) = match name {
                "msup" => (Vec::new(), nth(e, 1, d, budget)),
                "msub" => (nth(e, 1, d, budget), Vec::new()),
                _ => (nth(e, 1, d, budget), nth(e, 2, d, budget)),
            };
            if let Some(c) = lone_char(&base).filter(|c| is_nary_char(*c)) {
                return vec![nary(c, LimLoc::SubSup, sub, sup)];
            }
            let kind = match name {
                "msup" => ScriptKind::Sup,
                "msub" => ScriptKind::Sub,
                _ => ScriptKind::SubSup,
            };
            vec![MNode::Script { kind, base, sub, sup }]
        }
        "munder" | "mover" | "munderover" => {
            let name = local(&e.name);
            let base = nth(e, 0, d, budget);
            let (under, over) = match name {
                "munder" => (nth(e, 1, d, budget), Vec::new()),
                "mover" => (Vec::new(), nth(e, 1, d, budget)),
                _ => (nth(e, 1, d, budget), nth(e, 2, d, budget)),
            };
            if let Some(c) = lone_char(&base).filter(|c| is_nary_char(*c)) {
                return vec![nary(c, LimLoc::UndOvr, under, over)];
            }
            if name == "munderover" {
                let inner = MNode::Lim { upper: false, e: base, lim: under };
                return vec![MNode::Lim { upper: true, e: vec![inner], lim: over }];
            }
            let upper = name == "mover";
            let mark = if upper { lone_char(&over) } else { lone_char(&under) };
            match mark {
                Some('\u{203E}' | '¯' | '‾') if upper => vec![MNode::Bar { top: true, e: base }],
                Some('_' | '\u{203E}' | '¯') if !upper => vec![MNode::Bar { top: false, e: base }],
                Some(c @ ('⏞' | '⏟' | '⏜' | '⏝' | '⎴' | '⎵' | '←' | '→' | '↔' | '⇐' | '⇒' | '⇔')) if !word(&base) => {
                    vec![MNode::GroupChr { chr: c, top: upper, e: base }]
                }
                Some(c) if upper && combining_accent(c).is_some() => vec![MNode::Acc { chr: combining_accent(c).unwrap_or(c), e: base }],
                _ => vec![MNode::Lim { upper, e: base, lim: if upper { over } else { under } }],
            }
        }
        "mmultiscripts" => {
            let els: Vec<&El> = e.els().collect();
            let base = els.first().map(|b| elem(b, d, budget)).unwrap_or_default();
            let pre_at = els.iter().position(|c| local(&c.name) == "mprescripts");
            let script =
                |k: usize, budget: &mut usize| els.get(k).filter(|c| local(&c.name) != "none").map(|c| elem(c, d, budget)).unwrap_or_default();
            match pre_at {
                Some(p) => {
                    let sub = script(p + 1, budget);
                    let sup = script(p + 2, budget);
                    vec![MNode::Script { kind: ScriptKind::Pre, base, sub, sup }]
                }
                None => {
                    let sub = script(1, budget);
                    let sup = script(2, budget);
                    let kind = match (sub.is_empty(), sup.is_empty()) {
                        (true, true) => return base,
                        (false, true) => ScriptKind::Sub,
                        (true, false) => ScriptKind::Sup,
                        (false, false) => ScriptKind::SubSup,
                    };
                    vec![MNode::Script { kind, base, sub, sup }]
                }
            }
        }
        "mfenced" => {
            let first = |a: &str, dflt: char| e.attr(a).map(|v| v.trim().chars().next()).unwrap_or(Some(dflt));
            let beg = first("open", '(');
            let end = first("close", ')');
            let sep = first("separators", ',');
            let elems: Vec<Arg> = e.els().map(|c| elem(c, d, budget)).collect();
            vec![MNode::Delim { beg, end, sep, grow: true, shp_match: false, elems }]
        }
        "mtable" => {
            let rows: Vec<Vec<Arg>> = e
                .els()
                .filter(|r| matches!(local(&r.name), "mtr" | "mlabeledtr"))
                .take(256)
                .map(|r| r.els().filter(|c| local(&c.name) == "mtd").take(256).map(|c| children(c, d, budget)).collect())
                .collect();
            if rows.is_empty() {
                return Vec::new();
            }
            // One column is an equation array only when marked as one (`displaystyle`, alignment
            // marks, labelled rows, as TeX's align and our own writer give); else a column vector.
            let marked =
                e.attr("displaystyle").is_some_and(|v| v.trim() == "true") || e.els().any(|r| local(&r.name) == "mlabeledtr") || has_align_mark(e, 0);
            if marked && rows.iter().all(|r| r.len() <= 1) {
                return vec![MNode::EqArr { rows: rows.into_iter().map(|mut r| r.pop().unwrap_or_default()).collect() }];
            }
            vec![MNode::Matrix { rows, col_jc: Vec::new() }]
        }
        "mphantom" => vec![MNode::Phant { show: false, zero_wid: false, zero_asc: false, zero_desc: false, e: children(e, d, budget) }],
        "menclose" => {
            let n = e.attr("notation").unwrap_or("longdiv");
            let has = |k: &str| n.split_whitespace().any(|w| w.eq_ignore_ascii_case(k));
            if has("radical") {
                return vec![MNode::Rad { deg: Vec::new(), deg_hide: true, e: children(e, d, budget) }];
            }
            if has("box") || has("roundedbox") || has("circle") {
                return vec![MNode::BorderBox { hide: [false; 4], strike: [false; 4], e: children(e, d, budget) }];
            }
            if has("actuarial") {
                return vec![MNode::BorderBox { hide: [false, true, true, false], strike: [false; 4], e: children(e, d, budget) }];
            }
            let has_top = has("top");
            let has_bottom = has("bottom");
            let has_left = has("left");
            let has_right = has("right");
            let has_hstrike = has("horizontalstrike");
            let has_vstrike = has("verticalstrike");
            let has_upstrike = has("updiagonalstrike") || has("cross");
            let has_downstrike = has("downdiagonalstrike") || has("cross");

            if has_top || has_bottom || has_left || has_right || has_hstrike || has_vstrike || has_upstrike || has_downstrike {
                let hide = [!has_top, !has_bottom, !has_left, !has_right];
                let strike = [has_hstrike, has_vstrike, has_upstrike, has_downstrike];
                return vec![MNode::BorderBox { hide, strike, e: children(e, d, budget) }];
            }
            children(e, d, budget)
        }
        // The first child of `semantics` is the presentation; annotations aren't content.
        "semantics" => e.els().next().map(|c| elem(c, d, budget)).unwrap_or_default(),
        "annotation" | "annotation-xml" | "mspace" | "none" | "mprescripts" | "maligngroup" | "malignmark" => Vec::new(),
        _ => children(e, d, budget),
    }
}

/// Whether `e` holds a `maligngroup` or `malignmark` near the top (in a cell or a row inside it).
fn has_align_mark(e: &El, depth: usize) -> bool {
    depth <= 4 && e.els().any(|c| matches!(local(&c.name), "maligngroup" | "malignmark") || has_align_mark(c, depth + 1))
}

fn nary(chr: char, lim_loc: LimLoc, sub: Arg, sup: Arg) -> MNode {
    let lim_loc = Some(lim_loc).filter(|l| *l != if is_integral(chr) { LimLoc::SubSup } else { LimLoc::UndOvr });
    MNode::Nary { chr, lim_loc, grow: false, sub_hide: sub.is_empty(), sup_hide: sup.is_empty(), sub, sup, e: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::math::parse_linear;

    fn ml(linear: &str) -> String {
        to_mathml(&parse_linear(linear), false, "")
    }

    #[test]
    fn runs_split_into_identifiers_numbers_and_operators() {
        let m = ml("y=2x");
        assert!(m.starts_with("<math xmlns=\"http://www.w3.org/1998/Math/MathML\">"), "{m}");
        assert!(m.contains("<mi>y</mi><mo>=</mo><mn>2</mn><mi>x</mi>"), "{m}");
        assert!(m.contains("<annotation encoding=\"application/x-unicodemath\">y=2x</annotation>"), "{m}");
        assert!(to_mathml(&parse_linear("x"), true, "").contains("display=\"block\""));
    }

    #[test]
    fn structures_become_their_elements() {
        for (linear, want) in [
            ("a/b", "<mfrac><mrow><mi>a</mi></mrow><mrow><mi>b</mi></mrow></mfrac>"),
            ("x^2", "<msup><mrow><mi>x</mi></mrow><mrow><mn>2</mn></mrow></msup>"),
            ("x_s", "<msub><mrow><mi>x</mi></mrow><mrow><mi>s</mi></mrow></msub>"),
            ("x_i^2", "<msubsup>"),
            ("√x", "<msqrt><mi>x</mi></msqrt>"),
            ("√(3&x)", "<mroot><mrow><mi>x</mi></mrow><mrow><mn>3</mn></mrow></mroot>"),
            ("∑_(i=1)^n▒i", "<munderover><mo largeop=\"true\" movablelimits=\"false\">∑</mo>"),
            ("∫_0^1▒x", "<msubsup><mo largeop=\"true\" movablelimits=\"false\">∫</mo>"),
            ("(a+b)", "<mo fence=\"true\" form=\"prefix\">(</mo>"),
            ("sin⁡x", "<mi>sin</mi><mo>\u{2061}</mo>"),
            ("■(a&b@c&d)", "<mtable><mtr><mtd><mi>a</mi></mtd><mtd><mi>b</mi></mtd></mtr>"),
        ] {
            let m = ml(linear);
            assert!(m.contains(want), "{linear}: {m}");
        }
        let acc = to_mathml(&[MNode::Acc { chr: '\u{0302}', e: parse_linear("a") }], false, "");
        assert!(acc.contains("<mover accent=\"true\"><mrow><mi>a</mi></mrow><mo>^</mo></mover>"), "{acc}");
        let bar = to_mathml(&[MNode::Bar { top: true, e: parse_linear("a") }], false, "");
        assert!(bar.contains("<mo>\u{203E}</mo></mover>"), "{bar}");
        let lim = to_mathml(&[MNode::Lim { upper: false, e: parse_linear("lim"), lim: parse_linear("n→∞") }], false, "");
        assert!(lim.contains("<munder><mi>lim</mi><mrow><mi>n</mi><mo>→</mo><mi>∞</mi></mrow></munder>"), "{lim}");
        let bb = to_mathml(&[MNode::BorderBox { hide: [false; 4], strike: [false; 4], e: parse_linear("a") }], false, "");
        assert!(bb.contains("<menclose notation=\"box\">"), "{bb}");
        let gc = to_mathml(&[MNode::GroupChr { chr: '⏟', top: false, e: parse_linear("a") }], false, "");
        assert!(gc.contains("<munder><mrow><mi>a</mi></mrow><mo stretchy=\"true\">⏟</mo></munder>"), "{gc}");
        let pre = to_mathml(&[MNode::Script { kind: ScriptKind::Pre, base: parse_linear("C"), sub: parse_linear("n"), sup: vec![] }], false, "");
        assert!(pre.contains("<mprescripts/>"), "{pre}");
        let eq = to_mathml(&[MNode::EqArr { rows: vec![parse_linear("x&=1"), parse_linear("y&=2")] }], false, "");
        assert!(eq.contains("<mtd><mi>x</mi><mo>=</mo><mn>1</mn></mtd>") && !eq.contains("<mo>&amp;</mo>"), "{eq}");
        let ph = to_mathml(&[MNode::Phant { show: false, zero_wid: true, zero_asc: false, zero_desc: false, e: parse_linear("a") }], false, "");
        assert!(ph.contains("<mpadded width=\"0\"><mphantom>"), "{ph}");
        let styled = to_mathml(
            &[
                MNode::Run(MRun { text: "R".into(), scr: MScr::DoubleStruck, ..Default::default() }),
                MNode::Run(MRun::plain("d")),
                MNode::Run(MRun { text: "speed".into(), nor: true, ..Default::default() }),
            ],
            false,
            "",
        );
        assert!(
            styled.contains("<mi>ℝ</mi>") && styled.contains("<mi mathvariant=\"normal\">d</mi>") && styled.contains("<mtext>speed</mtext>"),
            "{styled}"
        );
        // Text is escaped.
        assert!(ml("a<b").contains("<mo>&lt;</mo>"));
    }

    fn pasted(html: &str) -> Vec<(String, bool)> {
        let flow = crate::html::parse(html);
        let mut out = Vec::new();
        for b in &flow.blocks {
            if let crate::model::FBlock::Para(p) = b {
                for i in &p.inlines {
                    if let crate::model::Inline::Equation { linear, display } = i {
                        out.push((linear.clone(), *display));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn html_export_round_trips_equations() {
        for (linear, display) in [("y=2x_s", false), ("x=(-b±√(b^2-4ac))/2a", true), ("∑_(k=1)^n▒k", false), ("■(a&b@c&d)", false)] {
            let html = crate::html::inlines_html(&[crate::model::Inline::Equation { linear: linear.into(), display }]);
            assert!(html.starts_with("<math"), "{html}");
            assert_eq!(pasted(&format!("<p>Before {html} after</p>")), vec![(linear.to_string(), display)], "{html}");
        }
    }

    #[test]
    fn foreign_mathml_is_read_from_its_elements() {
        let cases = [
            ("<math><mi>y</mi><mo>=</mo><mn>2</mn><msub><mi>x</mi><mi>s</mi></msub></math>", "y=2x_s"),
            ("<math display='block'><mfrac><mi>a</mi><mrow><mi>b</mi><mo>+</mo><mn>1</mn></mrow></mfrac></math>", "a/(b+1)"),
            ("<math><msqrt><mi>x</mi></msqrt></math>", "√x"),
            ("<math><munderover><mo>∑</mo><mrow><mi>k</mi><mo>=</mo><mn>1</mn></mrow><mi>n</mi></munderover><mi>k</mi></math>", "∑_(k=1)^n▒k"),
            ("<mml:math><mml:msup><mml:mi>e</mml:mi><mml:mi>x</mml:mi></mml:msup></mml:math>", "e^x"),
            // A TeX annotation is used when the presentation is empty.
            ("<math><semantics><mrow></mrow><annotation encoding='application/x-tex'>\\frac{1}{2}</annotation></semantics></math>", "1/2"),
        ];
        for (html, want) in cases {
            let got = pasted(html);
            assert_eq!(got.first().map(|g| g.0.as_str()), Some(want), "{html}");
        }
        assert!(pasted("<math display=\"block\"><mi>x</mi></math>").first().is_some_and(|g| g.1));
        // Text after the equation still arrives; an unclosed element ends with the input.
        let f = crate::html::parse("<p><math><mi>x</mi></math> tail</p>");
        assert!(crate::model::to_doc(&f).plain_text(wordcraft_doc::StoryRef::Body).contains("tail"));
        assert!(!pasted("<p><math><mfrac><mi>x").is_empty());
        // Hostile nesting is bounded.
        let deep = format!("<math>{}<mi>x</mi>{}</math>", "<mrow>".repeat(5000), "</mrow>".repeat(5000));
        assert!(pasted(&deep).len() <= 1);
    }

    #[test]
    fn math_over_the_token_cap_is_skipped_to_its_end() {
        // Past the cap, the rest of the element is dropped, not read as body text.
        let html = format!("<p><math>{}<mrow><mi>z</mi></mrow></math> tail</p>", "<mi>y</mi>".repeat(MAX_NODES * 2));
        let flow = crate::html::parse(&html);
        let mut text = String::new();
        for b in &flow.blocks {
            if let crate::model::FBlock::Para(p) = b {
                for i in &p.inlines {
                    if let crate::model::Inline::Text(t, _) = i {
                        text.push_str(t);
                    }
                }
            }
        }
        assert_eq!(text.trim(), "tail");
    }

    #[test]
    fn function_names_apply_without_the_invisible_operator() {
        let cases = [
            ("<math><mi>sin</mi><mi>x</mi></math>", "sin\u{2061}x"),
            ("<math><mi>log</mi><mrow><mi>a</mi><mi>b</mi></mrow></math>", "log\u{2061}ab"),
            ("<math><mi>cos</mi><mo>\u{2061}</mo><mi>x</mi></math>", "cos\u{2061}x"),
            ("<math><mi>ab</mi><mi>x</mi></math>", "abx"),
        ];
        for (html, want) in cases {
            assert_eq!(pasted(html).first().map(|g| g.0.as_str()), Some(want), "{html}");
        }
        let tree = to_tree(&El {
            name: "math".into(),
            attrs: vec![],
            children: vec![
                Item::El(El { name: "mi".into(), attrs: vec![], children: vec![Item::Text("sin".into())] }),
                Item::El(El { name: "mi".into(), attrs: vec![], children: vec![Item::Text("x".into())] }),
            ],
        });
        assert!(matches!(tree.as_slice(), [MNode::Func { .. }]), "{tree:?}");
    }

    #[test]
    fn one_column_tables_are_column_vectors_unless_marked() {
        let tree = |html: &str| {
            let linear = pasted(html).first().map(|g| g.0.clone()).unwrap_or_default();
            wordcraft_doc::math::parse_linear(&linear)
        };
        let vector = tree("<math><mtable><mtr><mtd><mi>a</mi></mtd></mtr><mtr><mtd><mi>b</mi></mtd></mtr></mtable></math>");
        assert!(matches!(vector.as_slice(), [MNode::Matrix { rows, .. }] if rows.len() == 2), "{vector:?}");
        for marked in [
            "<math><mtable displaystyle='true'><mtr><mtd><mi>a</mi></mtd></mtr><mtr><mtd><mi>b</mi></mtd></mtr></mtable></math>",
            "<math><mtable><mtr><mtd><mi>a</mi><maligngroup/><mo>=</mo><mn>1</mn></mtd></mtr><mtr><mtd><mi>b</mi></mtd></mtr></mtable></math>",
        ] {
            let t = tree(marked);
            assert!(matches!(t.as_slice(), [MNode::EqArr { rows }] if rows.len() == 2), "{marked}: {t:?}");
        }
        // Our own equation arrays read back as equation arrays from their elements alone.
        let ml = to_mathml(&[MNode::EqArr { rows: vec![parse_linear("x=1"), parse_linear("y=2")] }], false, "");
        let ml = ml.replace("application/x-unicodemath", "x-ignored");
        let t = tree(&ml);
        assert!(matches!(t.as_slice(), [MNode::EqArr { .. }]), "{ml}: {t:?}");
    }

    #[test]
    fn deep_nesting_is_bounded() {
        let mut n = parse_linear("x");
        for _ in 0..500 {
            n = vec![MNode::Frac { kind: FracKind::Bar, num: n, den: parse_linear("y") }];
        }
        let m = to_mathml(&n, false, "z");
        assert!(m.ends_with("</math>"));
    }

    #[test]
    fn test_mfrac_linethickness_none_and_menclose_notations() {
        let el_frac = El {
            name: "mfrac".into(),
            attrs: vec![("linethickness".into(), "none".into())],
            children: vec![
                Item::El(El { name: "mi".into(), attrs: vec![], children: vec![Item::Text("n".into())] }),
                Item::El(El { name: "mi".into(), attrs: vec![], children: vec![Item::Text("k".into())] }),
            ],
        };
        let mut budget = MAX_NODES;
        let tree_frac = elem(&el_frac, 0, &mut budget);
        assert!(matches!(tree_frac.as_slice(), [MNode::Frac { kind: FracKind::NoBar, .. }]), "{tree_frac:?}");

        let el_strike = El {
            name: "menclose".into(),
            attrs: vec![("notation".into(), "horizontalstrike".into())],
            children: vec![Item::El(El { name: "mi".into(), attrs: vec![], children: vec![Item::Text("x".into())] })],
        };
        let mut budget = MAX_NODES;
        let tree_strike = elem(&el_strike, 0, &mut budget);
        assert!(matches!(tree_strike.as_slice(), [MNode::BorderBox { strike: [true, false, false, false], .. }]), "{tree_strike:?}");

        let el_cross = El {
            name: "menclose".into(),
            attrs: vec![("notation".into(), "cross".into())],
            children: vec![Item::El(El { name: "mi".into(), attrs: vec![], children: vec![Item::Text("x".into())] })],
        };
        let mut budget = MAX_NODES;
        let tree_cross = elem(&el_cross, 0, &mut budget);
        assert!(matches!(tree_cross.as_slice(), [MNode::BorderBox { strike: [false, false, true, true], .. }]), "{tree_cross:?}");

        let el_rad = El {
            name: "menclose".into(),
            attrs: vec![("notation".into(), "radical".into())],
            children: vec![Item::El(El { name: "mi".into(), attrs: vec![], children: vec![Item::Text("x".into())] })],
        };
        let mut budget = MAX_NODES;
        let tree_rad = elem(&el_rad, 0, &mut budget);
        assert!(matches!(tree_rad.as_slice(), [MNode::Rad { deg_hide: true, .. }]), "{tree_rad:?}");
    }
}
