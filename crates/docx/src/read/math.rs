//! Office Math (OMML, ECMA-376 Part 1 §22.1) → [`wordcraft_doc::math`] tree.

use wordcraft_doc::math::{Arg, ColJc, FracKind, LimLoc, MAX_DEPTH, MNode, MRun, MScr, MSty, Math, MathJc, ScriptKind, merge_runs};
use wordcraft_doc::props::Rgb;

use crate::xml::El;

/// An `m:oMath` element as an equation (structure plus its source XML).
pub fn read_omath(e: &El, jc: MathJc) -> Math {
    Math { nodes: merge_runs(arg(e, 0)), omml: e.to_xml(), jc }
}

/// `m:oMathParaPr/m:jc`.
pub fn para_jc(para: &El) -> MathJc {
    match para.child("m:oMathParaPr").and_then(|p| p.child("m:jc")).and_then(|j| j.attr("m:val")) {
        Some("left") => MathJc::Left,
        Some("right") => MathJc::Right,
        Some("center") => MathJc::Center,
        _ => MathJc::CenterGroup,
    }
}

/// OMML on/off property: absent `m:val` = on.
fn on(e: &El) -> bool {
    match e.attr("m:val") {
        None => true,
        Some(v) => !matches!(v.trim(), "0" | "false" | "off" | "False" | "FALSE"),
    }
}

/// An on/off property `name` in the properties element `pr` (absent = `default`).
fn prop(pr: Option<&El>, name: &str, default: bool) -> bool {
    pr.and_then(|p| p.child(name)).map(on).unwrap_or(default)
}

/// A character property (`m:chr`, `m:begChr`…): `None` when absent, `Some(None)` when empty.
fn chr(pr: Option<&El>, name: &str) -> Option<Option<char>> {
    let c = pr?.child(name)?;
    match c.attr("m:val") {
        Some(v) => Some(v.chars().next()),
        None => Some(None),
    }
}

fn val<'a>(pr: Option<&'a El>, name: &str) -> Option<&'a str> {
    pr.and_then(|p| p.child(name)).and_then(|c| c.attr("m:val"))
}

/// The argument held by child `name` of `e`.
fn sub_arg(e: &El, name: &str, depth: usize) -> Arg {
    e.child(name).map(|c| arg(c, depth + 1)).unwrap_or_default()
}

/// Read the math content of an element (`m:oMath`, `m:e`, `m:num`…).
fn arg(e: &El, depth: usize) -> Arg {
    let mut out = Vec::new();
    if depth > MAX_DEPTH {
        return out;
    }
    for k in e.els() {
        node(k, depth, &mut out);
    }
    out
}

fn node(k: &El, depth: usize, out: &mut Arg) {
    let d = depth + 1;
    match k.name.as_str() {
        "m:r" => {
            if let Some(r) = run(k) {
                out.push(MNode::Run(r));
            }
        }
        // Word puts ordinary runs in math zones for normal text in some files.
        "w:r" => {
            let text = run_text(k);
            if !text.is_empty() {
                let mut r = MRun { text, nor: true, sty: Some(MSty::Plain), ..Default::default() };
                wrpr(k.child("w:rPr"), &mut r);
                out.push(MNode::Run(r));
            }
        }
        "m:f" => {
            let pr = k.child("m:fPr");
            let kind = match val(pr, "m:type") {
                Some("skw") => FracKind::Skewed,
                Some("lin") => FracKind::Linear,
                Some("noBar") => FracKind::NoBar,
                _ => FracKind::Bar,
            };
            out.push(MNode::Frac { kind, num: sub_arg(k, "m:num", d), den: sub_arg(k, "m:den", d) });
        }
        "m:sSup" => out.push(MNode::Script { kind: ScriptKind::Sup, base: sub_arg(k, "m:e", d), sub: Vec::new(), sup: sub_arg(k, "m:sup", d) }),
        "m:sSub" => out.push(MNode::Script { kind: ScriptKind::Sub, base: sub_arg(k, "m:e", d), sub: sub_arg(k, "m:sub", d), sup: Vec::new() }),
        "m:sSubSup" => {
            out.push(MNode::Script { kind: ScriptKind::SubSup, base: sub_arg(k, "m:e", d), sub: sub_arg(k, "m:sub", d), sup: sub_arg(k, "m:sup", d) })
        }
        "m:sPre" => {
            out.push(MNode::Script { kind: ScriptKind::Pre, base: sub_arg(k, "m:e", d), sub: sub_arg(k, "m:sub", d), sup: sub_arg(k, "m:sup", d) })
        }
        "m:rad" => {
            let pr = k.child("m:radPr");
            out.push(MNode::Rad { deg: sub_arg(k, "m:deg", d), deg_hide: prop(pr, "m:degHide", false), e: sub_arg(k, "m:e", d) });
        }
        "m:nary" => {
            let pr = k.child("m:naryPr");
            let chr = chr(pr, "m:chr").map(|c| c.unwrap_or('∫')).unwrap_or('∫');
            let lim_loc = match val(pr, "m:limLoc") {
                Some("undOvr") => Some(LimLoc::UndOvr),
                Some("subSup") => Some(LimLoc::SubSup),
                _ => None,
            };
            out.push(MNode::Nary {
                chr,
                lim_loc,
                grow: prop(pr, "m:grow", false),
                sub_hide: prop(pr, "m:subHide", false),
                sup_hide: prop(pr, "m:supHide", false),
                sub: sub_arg(k, "m:sub", d),
                sup: sub_arg(k, "m:sup", d),
                e: sub_arg(k, "m:e", d),
            });
        }
        "m:d" => {
            let pr = k.child("m:dPr");
            let beg = chr(pr, "m:begChr").unwrap_or(Some('('));
            let end = chr(pr, "m:endChr").unwrap_or(Some(')'));
            let sep = chr(pr, "m:sepChr").unwrap_or(Some('|'));
            let elems: Vec<Arg> = k.children("m:e").map(|e| arg(e, d)).collect();
            out.push(MNode::Delim {
                beg,
                end,
                sep,
                grow: prop(pr, "m:grow", true),
                shp_match: val(pr, "m:shp") == Some("match"),
                elems: if elems.is_empty() { vec![Vec::new()] } else { elems },
            });
        }
        "m:func" => out.push(MNode::Func { name: sub_arg(k, "m:fName", d), e: sub_arg(k, "m:e", d) }),
        "m:limLow" => out.push(MNode::Lim { upper: false, e: sub_arg(k, "m:e", d), lim: sub_arg(k, "m:lim", d) }),
        "m:limUpp" => out.push(MNode::Lim { upper: true, e: sub_arg(k, "m:e", d), lim: sub_arg(k, "m:lim", d) }),
        "m:acc" => {
            let c = chr(k.child("m:accPr"), "m:chr").map(|c| c.unwrap_or('\u{302}')).unwrap_or('\u{302}');
            out.push(MNode::Acc { chr: c, e: sub_arg(k, "m:e", d) });
        }
        "m:bar" => out.push(MNode::Bar { top: val(k.child("m:barPr"), "m:pos") == Some("top"), e: sub_arg(k, "m:e", d) }),
        "m:borderBox" => {
            let pr = k.child("m:borderBoxPr");
            out.push(MNode::BorderBox {
                hide: [prop(pr, "m:hideTop", false), prop(pr, "m:hideBot", false), prop(pr, "m:hideLeft", false), prop(pr, "m:hideRight", false)],
                strike: [
                    prop(pr, "m:strikeH", false),
                    prop(pr, "m:strikeV", false),
                    prop(pr, "m:strikeBLTR", false),
                    prop(pr, "m:strikeTLBR", false),
                ],
                e: sub_arg(k, "m:e", d),
            });
        }
        "m:box" => out.push(MNode::Boxed { e: sub_arg(k, "m:e", d) }),
        "m:groupChr" => {
            let pr = k.child("m:groupChrPr");
            let c = chr(pr, "m:chr").map(|c| c.unwrap_or('\u{23DF}')).unwrap_or('\u{23DF}');
            out.push(MNode::GroupChr { chr: c, top: val(pr, "m:pos") == Some("top"), e: sub_arg(k, "m:e", d) });
        }
        "m:eqArr" => out.push(MNode::EqArr { rows: k.children("m:e").map(|e| arg(e, d)).collect() }),
        "m:m" => {
            let mut col_jc = Vec::new();
            if let Some(mcs) = k.child("m:mPr").and_then(|p| p.child("m:mcs")) {
                for mc in mcs.children("m:mc") {
                    let pr = mc.child("m:mcPr");
                    let jc = match val(pr, "m:mcJc") {
                        Some("left") => ColJc::Left,
                        Some("right") => ColJc::Right,
                        _ => ColJc::Center,
                    };
                    let count = val(pr, "m:count").and_then(|c| c.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, 64);
                    for _ in 0..count {
                        if col_jc.len() < 64 {
                            col_jc.push(jc);
                        }
                    }
                }
            }
            let rows = k.children("m:mr").take(1000).map(|r| r.children("m:e").take(64).map(|e| arg(e, d)).collect()).collect();
            out.push(MNode::Matrix { rows, col_jc });
        }
        "m:phant" => {
            let pr = k.child("m:phantPr");
            out.push(MNode::Phant {
                show: prop(pr, "m:show", true),
                zero_wid: prop(pr, "m:zeroWid", false),
                zero_asc: prop(pr, "m:zeroAsc", false),
                zero_desc: prop(pr, "m:zeroDesc", false),
                e: sub_arg(k, "m:e", d),
            });
        }
        // Tracked insertions show; deletions don't.
        "w:ins" | "w:moveTo" | "w:smartTag" | "w:customXml" | "m:oMath" => {
            if depth <= MAX_DEPTH {
                for c in k.els() {
                    node(c, d, out);
                }
            }
        }
        "w:sdt" => {
            if let Some(c) = k.child("w:sdtContent") {
                for c in c.els() {
                    node(c, d, out);
                }
            }
        }
        _ => {}
    }
}

/// `m:t` (and `w:t`) text of a run.
fn run_text(r: &El) -> String {
    let mut s = String::new();
    for c in r.els() {
        match c.name.as_str() {
            "m:t" | "w:t" => s.push_str(&c.text()),
            "w:tab" => s.push('\t'),
            _ => {}
        }
    }
    s
}

fn run(r: &El) -> Option<MRun> {
    let text = run_text(r);
    if text.is_empty() {
        return None;
    }
    let mut out = MRun { text, ..Default::default() };
    if let Some(pr) = r.child("m:rPr") {
        out.sty = match pr.child("m:sty").and_then(|s| s.attr("m:val")) {
            Some("p") => Some(MSty::Plain),
            Some("b") => Some(MSty::Bold),
            Some("i") => Some(MSty::Italic),
            Some("bi") => Some(MSty::BoldItalic),
            _ => None,
        };
        out.scr = match pr.child("m:scr").and_then(|s| s.attr("m:val")) {
            Some("script") => MScr::Script,
            Some("fraktur") => MScr::Fraktur,
            Some("double-struck") => MScr::DoubleStruck,
            Some("sans-serif") => MScr::SansSerif,
            Some("monospace") => MScr::Monospace,
            _ => MScr::Roman,
        };
        out.nor = pr.child("m:nor").is_some_and(on);
        out.lit = pr.child("m:lit").is_some_and(on);
    }
    wrpr(r.child("w:rPr"), &mut out);
    Some(out)
}

/// The run formatting a math run takes from `w:rPr`: size, colour, normal-text font and weight.
fn wrpr(pr: Option<&El>, out: &mut MRun) {
    let Some(pr) = pr else { return };
    if let Some(sz) = pr.child("w:sz").and_then(|s| s.attr("w:val")).and_then(|v| v.trim().parse::<f32>().ok()) {
        let pt = sz / 2.0;
        if pt.is_finite() && (1.0..=1638.0).contains(&pt) {
            out.size = Some(pt);
        }
    }
    if let Some(c) = pr.child("w:color").and_then(|c| c.attr("w:val")).and_then(Rgb::parse) {
        out.color = Some(c);
    }
    if out.nor {
        if let Some(f) = pr.child("w:rFonts").and_then(|f| f.attr("w:ascii").or_else(|| f.attr("w:hAnsi"))) {
            out.font = Some(f.to_string());
        }
        let bold = pr.child("w:b").is_some_and(crate::units::on_off);
        let italic = pr.child("w:i").is_some_and(crate::units::on_off);
        if bold || italic {
            out.sty = Some(match (bold, italic) {
                (true, true) => MSty::BoldItalic,
                (true, false) => MSty::Bold,
                _ => MSty::Italic,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn omath(inner: &str) -> El {
        let x = format!(
            r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{inner}</m:oMath>"#
        );
        crate::xml::parse(x.as_bytes()).unwrap()
    }

    #[test]
    fn reads_fraction_radical_and_scripts() {
        let e = omath(
            r#"<m:r><m:t>x=</m:t></m:r><m:f><m:num><m:r><m:t>-b±</m:t></m:r><m:rad><m:radPr><m:degHide m:val="1"/></m:radPr><m:deg/><m:e><m:sSup><m:e><m:r><m:t>b</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup><m:r><m:t>-4ac</m:t></m:r></m:e></m:rad></m:num><m:den><m:r><m:t>2a</m:t></m:r></m:den></m:f>"#,
        );
        let m = read_omath(&e, MathJc::CenterGroup);
        assert_eq!(m.nodes.len(), 2);
        let MNode::Frac { num, den, kind } = &m.nodes[1] else { panic!("{:#?}", m.nodes) };
        assert_eq!(*kind, FracKind::Bar);
        assert_eq!(den, &vec![MNode::Run(MRun::new("2a"))]);
        let MNode::Rad { deg_hide, e, .. } = &num[1] else { panic!() };
        assert!(*deg_hide);
        assert!(matches!(e[0], MNode::Script { kind: ScriptKind::Sup, .. }));
        assert_eq!(wordcraft_doc::math::to_linear(&m.nodes), "x=(-b±√(b^2-4ac))/2a");
        assert!(m.omml.starts_with("<m:oMath>"));
    }

    #[test]
    fn reads_nary_delims_matrix_and_defaults() {
        let e = omath(
            r#"<m:nary><m:naryPr><m:chr m:val="∑"/><m:limLoc m:val="undOvr"/></m:naryPr><m:sub><m:r><m:t>i</m:t></m:r></m:sub><m:sup/><m:e><m:r><m:t>i</m:t></m:r></m:e></m:nary><m:d><m:dPr><m:begChr m:val="["/><m:endChr m:val=""/></m:dPr><m:e><m:r><m:t>a</m:t></m:r></m:e><m:e><m:r><m:t>b</m:t></m:r></m:e></m:d><m:nary><m:e/></m:nary><m:m><m:mr><m:e/><m:e/></m:mr></m:m>"#,
        );
        let m = read_omath(&e, MathJc::CenterGroup);
        let MNode::Nary { chr, lim_loc, .. } = &m.nodes[0] else { panic!() };
        assert_eq!((*chr, *lim_loc), ('∑', Some(LimLoc::UndOvr)));
        let MNode::Delim { beg, end, sep, elems, .. } = &m.nodes[1] else { panic!() };
        assert_eq!((*beg, *end, *sep, elems.len()), (Some('['), None, Some('|'), 2));
        assert!(matches!(m.nodes[2], MNode::Nary { chr: '∫', .. }));
        assert!(matches!(&m.nodes[3], MNode::Matrix { rows, .. } if rows.len() == 1 && rows[0].len() == 2));
    }
}
