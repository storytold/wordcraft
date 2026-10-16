//! The document's equations and the equaSHion editor's tree, through OMML.
//!
//! WordCraft keeps its own equation model ([`wordcraft_doc::math::Math`]), layout and DOCX
//! save; the equation editor widget and the MathML / LaTeX readers work on
//! [`equashion_math::Math`]. The two meet in OMML: the document side is written and read with
//! the DOCX equation writer and reader ([`wordcraft_docx::omml`]), the editor side with
//! equaSHion's own OMML export and import. So an equation crosses over exactly as it would
//! through a saved file, and nothing here duplicates either converter.

use wordcraft_doc::math::{Arg, LimLoc, MAX_DEPTH, MNode, Math, is_integral};
use wordcraft_docx::omml::{equation_from_omml, equation_to_omml};

pub use equashion_math as eq;

/// A document equation (its `linear` text, `display` flag and tree) as the editor's tree. A
/// display equation becomes an `m:oMathPara` (the editor's `para` flag).
pub fn to_editor(linear: &str, display: bool, math: &Math) -> eq::Math {
    eq::from_omml(&equation_to_omml(linear, display, math))
}

/// The editor's tree as a document equation and its display flag, or `None` when its OMML
/// does not read back as an equation (it always should).
pub fn from_editor(m: &eq::Math) -> Option<(Math, bool)> {
    omml_equation(&eq::to_omml(m))
}

/// An OMML fragment (from the editor or the clipboard) as a new document equation and its
/// display flag. The source OMML is not kept: the tree is the equation from here on.
pub fn omml_equation(xml: &str) -> Option<(Math, bool)> {
    let (mut math, display) = equation_from_omml(xml)?;
    math.omml.clear();
    default_lim_locs(&mut math.nodes, 0);
    Some((math, display))
}

/// equaSHion always writes an n-ary operator's limit location; where it is the usual one
/// (beside an integral, under and over the rest) the document model leaves it unset, as it
/// does for equations made here, so a save writes no `m:limLoc` Word would not.
fn default_lim_locs(a: &mut Arg, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let d = depth + 1;
    for n in a.iter_mut() {
        match n {
            MNode::Run(_) => {}
            MNode::Nary { chr, lim_loc, sub, sup, e, .. } => {
                let usual = if is_integral(*chr) { LimLoc::SubSup } else { LimLoc::UndOvr };
                if *lim_loc == Some(usual) {
                    *lim_loc = None;
                }
                for x in [sub, sup, e] {
                    default_lim_locs(x, d);
                }
            }
            MNode::Frac { num, den, .. } => {
                default_lim_locs(num, d);
                default_lim_locs(den, d);
            }
            MNode::Script { base, sub, sup, .. } => {
                for x in [base, sub, sup] {
                    default_lim_locs(x, d);
                }
            }
            MNode::Rad { deg, e, .. } => {
                default_lim_locs(deg, d);
                default_lim_locs(e, d);
            }
            MNode::Delim { elems, .. } => elems.iter_mut().for_each(|x| default_lim_locs(x, d)),
            MNode::Func { name, e } => {
                default_lim_locs(name, d);
                default_lim_locs(e, d);
            }
            MNode::Lim { e, lim, .. } => {
                default_lim_locs(e, d);
                default_lim_locs(lim, d);
            }
            MNode::Acc { e, .. }
            | MNode::Bar { e, .. }
            | MNode::BorderBox { e, .. }
            | MNode::Boxed { e }
            | MNode::GroupChr { e, .. }
            | MNode::Phant { e, .. } => default_lim_locs(e, d),
            MNode::EqArr { rows } => rows.iter_mut().for_each(|x| default_lim_locs(x, d)),
            MNode::Matrix { rows, .. } => rows.iter_mut().flatten().for_each(|x| default_lim_locs(x, d)),
        }
    }
}

/// The editor's tree as OMML: what equation commands take as their `omml` parameter.
pub fn editor_omml(m: &eq::Math) -> String {
    eq::to_omml(m)
}

/// Presentation MathML (one `<math>` element) as a document equation. `None` when it is not
/// MathML equaSHion reads, or holds nothing.
pub fn from_mathml(xml: &str) -> Option<(Math, bool)> {
    let m = eq::from_mathml(xml).ok()?;
    if m.is_empty() {
        return None;
    }
    from_editor(&m)
}

/// LaTeX (or equaSHion's typed linear form) as a document equation, read by equaSHion's
/// parser (`\frac{a}{b}`, `\sqrt{x}`, `\sum_{i=1}^{n}`, `x^2`). `None` when nothing is left.
pub fn from_latex(src: &str, display: bool) -> Option<(Math, bool)> {
    let src = src.trim();
    if src.is_empty() {
        return None;
    }
    let mut m = eq::from_linear(src);
    if m.is_empty() {
        return None;
    }
    m.para = display;
    from_editor(&m)
}

/// Largest OMML or MathML clipboard text read as an equation, bytes.
pub const MAX_CLIP_MARKUP: usize = 1 << 20;
/// Largest delimited LaTeX clipboard text read as an equation, bytes.
pub const MAX_CLIP_LATEX: usize = 1 << 15;

/// The format of an equation on the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipFormat {
    Omml,
    MathMl,
    Latex,
}

impl ClipFormat {
    pub fn name(self) -> &'static str {
        match self {
            ClipFormat::Omml => "OMML",
            ClipFormat::MathMl => "MathML",
            ClipFormat::Latex => "LaTeX",
        }
    }
}

/// Recognise an equation in pasted plain text. The whole text (trimmed) must be exactly one OMML
/// `m:oMath` / `m:oMathPara` element, one MathML `<math>` element, or LaTeX between explicit
/// delimiters: `$$..$$` or `\[..\]` (display), `$..$` or `\(..\)` (inline). `None` when it is not
/// such a payload (paste it as text); otherwise its format and the equation with its display
/// flag, or why it could not be read.
pub fn clipboard_equation(text: &str) -> Option<(ClipFormat, Result<(Math, bool), String>)> {
    let t = text.trim();
    if let Some((qname, local)) = root_element(t) {
        let format = match local {
            "oMath" | "oMathPara" => ClipFormat::Omml,
            "math" => ClipFormat::MathMl,
            _ => return None,
        };
        if !whole_element(t, qname) {
            return None;
        }
        if t.len() > MAX_CLIP_MARKUP {
            return Some((format, Err(format!("the {} is too large", format.name()))));
        }
        let read = match format {
            ClipFormat::Omml => omml_equation(t).filter(|(m, _)| !m.nodes.is_empty()),
            _ => from_mathml(t),
        };
        return Some((format, read.ok_or_else(|| format!("not a {} equation WordCraft can read", format.name()))));
    }
    let (inner, display) = latex_inner(t)?;
    if inner.len() > MAX_CLIP_LATEX {
        return Some((ClipFormat::Latex, Err("the LaTeX is too long".into())));
    }
    Some((ClipFormat::Latex, from_latex(inner, display).ok_or_else(|| "the LaTeX equation is empty".to_string())))
}

/// The qualified and local name of the first element, after an XML declaration, processing
/// instructions and comments.
fn root_element(t: &str) -> Option<(&str, &str)> {
    let mut rest = t;
    loop {
        let (open, close) = if rest.starts_with("<?") {
            ("<?", "?>")
        } else if rest.starts_with("<!--") {
            ("<!--", "-->")
        } else {
            break;
        };
        let body = rest.get(open.len()..)?;
        rest = body.get(body.find(close)? + close.len()..)?.trim_start();
    }
    let r = rest.strip_prefix('<')?;
    let len = r.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(r.len());
    let qname = r.get(..len).filter(|n| !n.is_empty())?;
    Some((qname, qname.rsplit(':').next().unwrap_or(qname)))
}

/// The text ends with the root element's end tag and holds no second element of that name at
/// the top: it is that one element.
fn whole_element(t: &str, qname: &str) -> bool {
    let end = format!("</{qname}>");
    match t.find(&end) {
        Some(i) => i + end.len() == t.len(),
        // An empty element (`<m:oMath/>`): its start tag ends the text.
        None => {
            let Some(start) = t.find(&format!("<{qname}")) else { return false };
            let tag_end = t.get(start..).and_then(|r| r.find('>')).map(|g| start + g + 1);
            t.ends_with("/>") && tag_end == Some(t.len())
        }
    }
}

/// The LaTeX between explicit delimiters spanning the whole text, and whether it is display.
/// Single `$` follows the usual Markdown rule (no space just inside either `$`), so prices and
/// other text with dollar signs stay text.
fn latex_inner(t: &str) -> Option<(&str, bool)> {
    let pick = |open: &str, close: &str, display: bool| -> Option<(&str, bool)> {
        let inner = t.strip_prefix(open)?.strip_suffix(close)?;
        if inner.contains(open) || inner.contains(close) || inner.trim().is_empty() {
            return None;
        }
        Some((inner, display))
    };
    if t.starts_with("$$") {
        return pick("$$", "$$", true).filter(|(i, _)| !i.contains('$'));
    }
    pick("\\[", "\\]", true)
        .or_else(|| pick("\\(", "\\)", false))
        .or_else(|| pick("$", "$", false).filter(|(i, _)| !i.starts_with(char::is_whitespace) && !i.ends_with(char::is_whitespace)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::math::{MNode, merge_runs, parse_linear, to_linear};

    /// The equations the round trips are pinned on: (name, linear format).
    const EQUATIONS: &[(&str, &str)] = &[
        ("fraction", "a/b"),
        ("sqrt", "√(x+1)"),
        ("root with degree", "√(3&x)"),
        ("sum with limits", "∑_(i=1)^n▒i^2"),
        ("matrix", "[■(1&0@0&1)]"),
        ("accent", "x̂+A⃗"),
        ("quadratic formula", "x=(-b±√(b^2-4ac))/2a"),
        ("scripts", "x_i^2+e^(iπ)"),
        ("function", "sin⁡θ"),
        ("integral", "∫_0^1▒〖x dx〗"),
    ];

    fn doc_math(linear: &str) -> Math {
        Math { nodes: parse_linear(linear), ..Default::default() }
    }

    #[test]
    fn document_equations_round_trip_through_the_editor_tree() {
        for (name, lin) in EQUATIONS {
            let m = doc_math(lin);
            for display in [false, true] {
                let e = to_editor(lin, display, &m);
                assert_eq!(e.para, display, "{name}: display flag");
                let (back, d) = from_editor(&e).unwrap_or_else(|| panic!("{name}: no equation back"));
                assert_eq!(d, display, "{name}");
                // Exactly what a DOCX save and reopen gives (that pass fills in the matrix
                // column alignments the writer spells out, nothing else).
                let (saved, _) = equation_from_omml(&equation_to_omml(lin, display, &m)).unwrap_or_default();
                assert_eq!(merge_runs(back.nodes.clone()), merge_runs(saved.nodes), "{name}: {lin} -> {}", to_linear(&back.nodes));
                assert_eq!(to_linear(&back.nodes), to_linear(&m.nodes), "{name}");
                // And the editor's tree survives the document model unchanged.
                let e2 = to_editor(&to_linear(&back.nodes), d, &Math { omml: String::new(), ..back });
                assert_eq!(eq::to_linear(&e2), eq::to_linear(&e), "{name}");
            }
        }
    }

    #[test]
    fn editor_trees_round_trip_through_the_document_model() {
        for src in [r"\frac{a}{b}", r"\sqrt{x}", r"\sum_{i=1}^{n} i", r"\hat{x}", r"x=\frac{-b\pm\sqrt{b^2-4ac}}{2a}"] {
            let e = eq::from_linear(src);
            let (m, _) = from_editor(&e).unwrap_or_else(|| panic!("{src}"));
            let back = to_editor(&to_linear(&m.nodes), false, &m);
            assert_eq!(eq::to_linear(&back), eq::to_linear(&e), "{src}");
        }
    }

    #[test]
    fn structures_land_in_the_document_model() {
        let structure = |lin: &str| from_editor(&to_editor(lin, false, &doc_math(lin))).map(|(m, _)| m.nodes).unwrap_or_default();
        assert!(matches!(structure("a/b").as_slice(), [MNode::Frac { .. }]));
        assert!(matches!(structure("√(x+1)").as_slice(), [MNode::Rad { .. }]));
        assert!(matches!(structure("∑_(i=1)^n▒i").as_slice(), [MNode::Nary { chr: '∑', .. }]));
        assert!(matches!(structure("x̂").as_slice(), [MNode::Acc { .. }]));
        assert!(
            structure("[■(1&0@0&1)]")
                .iter()
                .any(|n| matches!(n, MNode::Delim { elems, .. } if matches!(elems.first().and_then(|e| e.first()), Some(MNode::Matrix { .. }))))
        );
    }

    #[test]
    fn source_omml_is_what_the_editor_gets() {
        // An imported equation keeps Word's XML; the editor reads that, not a rewrite.
        let xml = "<m:oMath><m:sSup><m:e><m:r><m:t>E</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:oMath>";
        let (m, _) = equation_from_omml(xml).unwrap();
        assert!(!m.omml.is_empty());
        assert_eq!(eq::to_linear(&to_editor(&to_linear(&m.nodes), false, &m)), "E^2");
    }

    #[test]
    fn clipboard_detection_takes_only_whole_payloads() {
        for t in [
            "",
            "hello",
            "costs $5",
            "$5 and $6",
            "between $ 5 and 6 $",
            "a \\frac{1}{2} b",
            "\\alpha",
            "$$",
            "$ $",
            "$a$ and $b$",
            "<b>x</b>",
            "<mathx>y</mathx>",
            "<math><mi>x</mi></math> and more",
            "<m:oMath><m:r><m:t>a</m:t></m:r></m:oMath><m:oMath><m:r><m:t>b</m:t></m:r></m:oMath>",
            "<m:oMath><m:r><m:t>x</m:t></m:r>",
            "<!-- unterminated",
            "<?xml",
        ] {
            assert!(clipboard_equation(t).is_none(), "{t:?}");
        }
        let read = |t: &str| clipboard_equation(t).map(|(f, r)| (f, r.map(|(m, d)| (to_linear(&m.nodes), d))));
        assert_eq!(read("  $x^2$\n"), Some((ClipFormat::Latex, Ok(("x^2".into(), false)))));
        assert_eq!(read("$$\\frac{a}{b}$$"), Some((ClipFormat::Latex, Ok(("a/b".into(), true)))));
        assert_eq!(read("\\[x\\]"), Some((ClipFormat::Latex, Ok(("x".into(), true)))));
        assert_eq!(read("\\(y\\)"), Some((ClipFormat::Latex, Ok(("y".into(), false)))));
        let para = "<?xml version=\"1.0\"?>\n<m:oMathPara xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\"><m:oMath><m:r><m:t>y</m:t></m:r></m:oMath></m:oMathPara>";
        assert_eq!(read(para), Some((ClipFormat::Omml, Ok(("y".into(), true)))));
        assert_eq!(read("<m:oMath><m:r><m:t>z</m:t></m:r></m:oMath>"), Some((ClipFormat::Omml, Ok(("z".into(), false)))));
        assert_eq!(read("<math display=\"block\"><mi>x</mi></math>"), Some((ClipFormat::MathMl, Ok(("x".into(), true)))));
        // Looks like an equation but cannot be read: the caller pastes text and says why.
        assert!(matches!(read("<m:oMath/>"), Some((ClipFormat::Omml, Err(_)))), "empty");
        assert!(matches!(read("<x:oMath xmlns:x=\"urn:other\"><x:r><x:t>a</x:t></x:r></x:oMath>"), Some((ClipFormat::Omml, Err(_)))));
        let long = format!("${}$", "x".repeat(MAX_CLIP_LATEX + 1));
        assert!(matches!(read(&long), Some((ClipFormat::Latex, Err(_)))));
        // equaSHion's own MathML copy reads back.
        let copied = eq::to_mathml_display(&eq::from_linear(r"\sqrt{x}"), true);
        assert!(
            matches!(clipboard_equation(&copied), Some((ClipFormat::MathMl, Ok((m, true)))) if matches!(m.nodes.as_slice(), [MNode::Rad { .. }]))
        );
    }

    #[test]
    fn mathml_and_latex_become_document_equations() {
        let (m, display) = from_mathml(r#"<math display="block"><mfrac><mi>a</mi><mi>b</mi></mfrac></math>"#).unwrap();
        assert!(display);
        assert!(matches!(m.nodes.as_slice(), [MNode::Frac { .. }]), "{:?}", m.nodes);
        let (m, display) = from_latex(r"\sqrt{x^2+1}", false).unwrap();
        assert!(!display);
        assert!(matches!(m.nodes.as_slice(), [MNode::Rad { .. }]), "{:?}", m.nodes);
        assert!(from_mathml("<p>not math</p>").is_none());
        assert!(from_latex("   ", false).is_none());
    }
}
