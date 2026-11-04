//! Office Math (OMML, ECMA-376 Part 1 §22.1) read from HTML: the `m:oMath` markup Word (and
//! tools that copy its HTML, such as Better Notes for Zotero) writes inside an
//! `<!--[if gte msEquation 12]>…<![endif]-->` conditional comment (#5582).
//!
//! The markup arrives as the HTML reader's [`El`] tree, so names are lowercase and namespace
//! prefixes are whatever the file used; only the structure is read (the linear format the flow
//! model carries has no run formatting beyond plain text). Elements this reader doesn't know
//! (`span`, `i`, `w:rPr`…) are transparent: their content is read, their properties aren't.

use wordcraft_doc::math::{Arg, ColJc, FracKind, LimLoc, MAX_DEPTH, MNode, MRun, MSty, ScriptKind, merge_runs, to_linear};

use crate::mathml::{El, Item, MAX_NODES, local};

/// The linear format of an `m:oMath` or `m:oMathPara` element, and whether it is a display
/// equation (`m:oMathPara`). `None` when it holds nothing.
pub fn read(e: &El) -> Option<(String, bool)> {
    let display = name(e) == "omathpara";
    // Structure without a character in it (a cut-off `m:f`) is no equation.
    if e.text().trim().is_empty() {
        return None;
    }
    let mut budget = MAX_NODES;
    let nodes = merge_runs(arg(e, 0, &mut budget));
    let linear: String = to_linear(&nodes).chars().filter(|c| !c.is_control()).collect();
    let linear = linear.trim().to_string();
    (!linear.is_empty()).then_some((linear, display))
}

/// The element's local name, lowercase.
fn name(e: &El) -> String {
    local(&e.name).to_ascii_lowercase()
}

fn els(e: &El) -> impl Iterator<Item = &El> {
    e.children.iter().filter_map(|c| match c {
        Item::El(e) => Some(e),
        Item::Text(_) => None,
    })
}

/// The first child element called `n` (local name, lowercase).
fn child<'a>(e: &'a El, n: &str) -> Option<&'a El> {
    els(e).find(|c| name(c) == n)
}

fn children<'a>(e: &'a El, n: &'a str) -> impl Iterator<Item = &'a El> {
    els(e).filter(move |c| name(c) == n)
}

/// An attribute by local name (`m:val`, `val`).
fn attr<'a>(e: &'a El, n: &str) -> Option<&'a str> {
    e.attrs.iter().find(|(k, _)| local(k).eq_ignore_ascii_case(n)).map(|(_, v)| v.as_str())
}

/// The `val` of property `n` in the properties element `pr`.
fn val<'a>(pr: Option<&'a El>, n: &str) -> Option<&'a str> {
    pr.and_then(|p| child(p, n)).and_then(|c| attr(c, "val"))
}

/// An on/off property `n` in `pr` (absent = `default`; present without a value = on).
fn prop(pr: Option<&El>, n: &str, default: bool) -> bool {
    match pr.and_then(|p| child(p, n)) {
        None => default,
        Some(c) => attr(c, "val").is_none_or(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off")),
    }
}

/// A character property (`chr`, `begChr`…): `None` when absent, `Some(None)` when empty.
fn chr(pr: Option<&El>, n: &str) -> Option<Option<char>> {
    let c = pr.and_then(|p| child(p, n))?;
    Some(attr(c, "val").and_then(|v| v.chars().next()))
}

/// The argument held by child `n` of `e`.
fn sub_arg(e: &El, n: &str, depth: usize, budget: &mut usize) -> Arg {
    child(e, n).map(|c| arg(c, depth + 1, budget)).unwrap_or_default()
}

/// The math content of an element (`m:oMath`, `m:e`, `m:num`…).
fn arg(e: &El, depth: usize, budget: &mut usize) -> Arg {
    let mut out = Vec::new();
    if depth > MAX_DEPTH {
        return out;
    }
    for k in els(e) {
        node(k, depth, budget, &mut out);
    }
    out
}

/// Text inside a run, its properties left out.
fn run_text(e: &El, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for c in &e.children {
        match c {
            Item::Text(t) => out.push_str(t),
            Item::El(c) if !name(c).ends_with("pr") => run_text(c, out, depth + 1),
            Item::El(_) => {}
        }
    }
}

fn run(r: &El) -> Option<MRun> {
    let mut text = String::new();
    run_text(r, &mut text, 0);
    if text.is_empty() {
        return None;
    }
    let mut out = MRun::new(text);
    if let Some(pr) = child(r, "rpr") {
        out.sty = match val(Some(pr), "sty") {
            Some("p") => Some(MSty::Plain),
            Some("b") => Some(MSty::Bold),
            Some("i") => Some(MSty::Italic),
            Some("bi") => Some(MSty::BoldItalic),
            _ => None,
        };
        out.nor = prop(Some(pr), "nor", false);
    }
    Some(out)
}

fn node(k: &El, depth: usize, budget: &mut usize, out: &mut Arg) {
    if depth > MAX_DEPTH || *budget == 0 {
        return;
    }
    *budget -= 1;
    let d = depth + 1;
    let n = name(k);
    let pr = child(k, &format!("{n}pr"));
    match n.as_str() {
        "r" => out.extend(run(k).map(MNode::Run)),
        "f" => {
            let kind = match val(pr, "type") {
                Some("skw") => FracKind::Skewed,
                Some("lin") => FracKind::Linear,
                Some("nobar") => FracKind::NoBar,
                _ => FracKind::Bar,
            };
            out.push(MNode::Frac { kind, num: sub_arg(k, "num", d, budget), den: sub_arg(k, "den", d, budget) });
        }
        "ssup" | "ssub" | "ssubsup" | "spre" => {
            let kind = match n.as_str() {
                "ssup" => ScriptKind::Sup,
                "ssub" => ScriptKind::Sub,
                "ssubsup" => ScriptKind::SubSup,
                _ => ScriptKind::Pre,
            };
            out.push(MNode::Script { kind, base: sub_arg(k, "e", d, budget), sub: sub_arg(k, "sub", d, budget), sup: sub_arg(k, "sup", d, budget) });
        }
        "rad" => {
            out.push(MNode::Rad { deg: sub_arg(k, "deg", d, budget), deg_hide: prop(pr, "deghide", false), e: sub_arg(k, "e", d, budget) });
        }
        "nary" => {
            let lim_loc = match val(pr, "limloc") {
                Some("undOvr" | "undovr") => Some(LimLoc::UndOvr),
                Some("subSup" | "subsup") => Some(LimLoc::SubSup),
                _ => None,
            };
            out.push(MNode::Nary {
                chr: chr(pr, "chr").flatten().unwrap_or('∫'),
                lim_loc,
                grow: prop(pr, "grow", false),
                sub_hide: prop(pr, "subhide", false),
                sup_hide: prop(pr, "suphide", false),
                sub: sub_arg(k, "sub", d, budget),
                sup: sub_arg(k, "sup", d, budget),
                e: sub_arg(k, "e", d, budget),
            });
        }
        "d" => {
            let elems: Vec<Arg> = children(k, "e").take(256).map(|e| arg(e, d, budget)).collect();
            out.push(MNode::Delim {
                beg: chr(pr, "begchr").unwrap_or(Some('(')),
                end: chr(pr, "endchr").unwrap_or(Some(')')),
                sep: chr(pr, "sepchr").unwrap_or(Some('|')),
                grow: prop(pr, "grow", true),
                shp_match: val(pr, "shp") == Some("match"),
                elems: if elems.is_empty() { vec![Vec::new()] } else { elems },
            });
        }
        "func" => out.push(MNode::Func { name: sub_arg(k, "fname", d, budget), e: sub_arg(k, "e", d, budget) }),
        "limlow" | "limupp" => {
            out.push(MNode::Lim { upper: n == "limupp", e: sub_arg(k, "e", d, budget), lim: sub_arg(k, "lim", d, budget) });
        }
        "acc" => out.push(MNode::Acc { chr: chr(pr, "chr").flatten().unwrap_or('\u{302}'), e: sub_arg(k, "e", d, budget) }),
        "bar" => out.push(MNode::Bar { top: val(pr, "pos") == Some("top"), e: sub_arg(k, "e", d, budget) }),
        "borderbox" => out.push(MNode::BorderBox {
            hide: [prop(pr, "hidetop", false), prop(pr, "hidebot", false), prop(pr, "hideleft", false), prop(pr, "hideright", false)],
            strike: [prop(pr, "strikeh", false), prop(pr, "strikev", false), prop(pr, "strikebltr", false), prop(pr, "striketlbr", false)],
            e: sub_arg(k, "e", d, budget),
        }),
        "box" => out.push(MNode::Boxed { e: sub_arg(k, "e", d, budget) }),
        "groupchr" => out.push(MNode::GroupChr {
            chr: chr(pr, "chr").flatten().unwrap_or('\u{23DF}'),
            top: val(pr, "pos") == Some("top"),
            e: sub_arg(k, "e", d, budget),
        }),
        "eqarr" => out.push(MNode::EqArr { rows: children(k, "e").take(1000).map(|e| arg(e, d, budget)).collect() }),
        "m" => {
            let rows: Vec<Vec<Arg>> = children(k, "mr").take(1000).map(|r| children(r, "e").take(64).map(|e| arg(e, d, budget)).collect()).collect();
            let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
            out.push(MNode::Matrix { rows, col_jc: vec![ColJc::Center; cols] });
        }
        "phant" => out.push(MNode::Phant {
            show: prop(pr, "show", true),
            zero_wid: prop(pr, "zerowid", false),
            zero_asc: prop(pr, "zeroasc", false),
            zero_desc: prop(pr, "zerodesc", false),
            e: sub_arg(k, "e", d, budget),
        }),
        // Deleted text doesn't show; properties aren't content.
        "del" | "movefrom" => {}
        _ if n.ends_with("pr") => {}
        // Anything else (`m:oMath` in `m:oMathPara`, HTML wrappers such as `span` or `i`, tracked
        // insertions) keeps its content where it stands.
        _ => {
            for c in els(k) {
                node(c, d, budget, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The OMML in `xml` read through the HTML reader's tree, as the importer does.
    fn lin(xml: &str) -> Option<(String, bool)> {
        let doc = crate::html::parse_office_math(xml);
        doc.first().and_then(read)
    }

    #[test]
    fn structures_read_as_linear_format() {
        let m = |b: &str| format!("<m:oMath>{b}</m:oMath>");
        let r = |t: &str| format!("<m:r><m:t>{t}</m:t></m:r>");
        assert_eq!(lin(&m(&r("x=1"))), Some(("x=1".into(), false)));
        let frac = format!("<m:f><m:num>{}</m:num><m:den>{}</m:den></m:f>", r("a"), r("b"));
        assert_eq!(lin(&m(&frac)).map(|l| l.0), Some("a/b".into()));
        let sup = format!("<m:sSup><m:e>{}</m:e><m:sup>{}</m:sup></m:sSup>", r("x"), r("2"));
        assert_eq!(lin(&m(&sup)).map(|l| l.0), Some("x^2".into()));
        let rad = format!("<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e>{}</m:e></m:rad>", r("y"));
        assert_eq!(lin(&m(&rad)).map(|l| l.0), Some("√y".into()));
        let delim = format!("<m:d><m:e>{}</m:e></m:d>", r("a+b"));
        assert_eq!(lin(&m(&delim)).map(|l| l.0), Some("(a+b)".into()));
        let nary = format!(
            "<m:nary><m:naryPr><m:chr m:val=\"∑\"/></m:naryPr><m:sub>{}</m:sub><m:sup>{}</m:sup><m:e>{}</m:e></m:nary>",
            r("i=1"),
            r("n"),
            r("i")
        );
        let got = lin(&m(&nary)).map(|l| l.0).unwrap_or_default();
        assert!(got.starts_with('∑') && got.contains("i=1") && got.contains('n'), "{got}");
    }

    #[test]
    fn a_math_paragraph_is_a_display_equation() {
        let got = lin("<m:oMathPara><m:oMath><m:r><m:t>E=mc</m:t></m:r></m:oMath></m:oMathPara>");
        assert_eq!(got, Some(("E=mc".into(), true)));
    }

    #[test]
    fn wrappers_and_properties_are_transparent() {
        let xml =
            "<m:oMath><m:r><m:rPr><m:sty m:val=\"p\"/></m:rPr><span style='font-family:\"Cambria Math\"'><i><m:t>z</m:t></i></span></m:r></m:oMath>";
        assert_eq!(lin(xml).map(|l| l.0), Some("z".into()));
    }

    #[test]
    fn empty_and_hostile_markup_reads_as_nothing_or_bounded() {
        assert_eq!(lin("<m:oMath></m:oMath>"), None);
        assert_eq!(lin("<m:oMath><m:f><m:num>"), None);
        let deep = format!("<m:oMath>{}<m:r><m:t>q</m:t></m:r></m:oMath>", "<m:d><m:e>".repeat(5000));
        let _ = lin(&deep);
    }
}
