//! Office Math (OMML, ECMA-376 Part 1 §22.1) → [`wordcraft_doc::math`] tree.

use wordcraft_doc::math::{
    Arg, BrkBin, BrkBinSub, ColJc, FracKind, LimLoc, MAX_DEPTH, MNode, MRun, MScr, MSty, Math, MathJc, MathProps, ScriptKind, merge_runs,
};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::symbol_fonts;

use crate::xml::El;

/// An `m:oMath` element as an equation (structure plus its source XML).
pub fn read_omath(e: &El, jc: MathJc) -> Math {
    Math { nodes: merge_runs(arg(e, 0)), omml: e.to_xml(), jc, linear: false }
}

/// A colour every math run shares, taken off the runs: it is the equation's own colour, as
/// [`write_equation`](crate::write::math::write_equation) spreads it (#1467). The source OMML
/// is kept: the writer replaces it only when the colour changes.
pub fn lift_color(math: &mut Math) -> Option<Rgb> {
    let (mut shared, mut uniform) = (None, true);
    wordcraft_doc::math_edit::each_run_mut(&mut math.nodes, &mut |r| match shared {
        None => shared = Some(r.color),
        Some(c) => uniform &= c == r.color,
    });
    let c = shared.flatten().filter(|_| uniform)?;
    wordcraft_doc::math_edit::each_run_mut(&mut math.nodes, &mut |r| r.color = None);
    Some(c)
}

/// An explicit size shared by every run and structural control, without inherited arguments.
pub(super) fn shared_size(e: &El) -> Option<f32> {
    fn scan(e: &El, depth: usize, common: &mut Option<f32>, runs: &mut usize) -> Option<()> {
        if depth > MAX_DEPTH || e.name == "m:argSz" {
            return None;
        }
        // The reader drops deleted revisions, including their nested size properties.
        if matches!(e.name.as_str(), "w:del" | "w:moveFrom" | "m:del") {
            return Some(());
        }
        if matches!(e.name.as_str(), "m:r" | "w:r" | "m:ctrlPr") {
            let mut r = MRun::default();
            wrpr(e.child("w:rPr"), &mut r);
            let size = r.size?;
            if common.is_some_and(|previous| previous != size) {
                return None;
            }
            *common = Some(size);
            if e.name != "m:ctrlPr" {
                *runs = runs.saturating_add(1);
            }
        }
        if matches!(
            e.name.as_str(),
            "m:f"
                | "m:sSup"
                | "m:sSub"
                | "m:sSubSup"
                | "m:sPre"
                | "m:rad"
                | "m:nary"
                | "m:d"
                | "m:func"
                | "m:limLow"
                | "m:limUpp"
                | "m:acc"
                | "m:bar"
                | "m:borderBox"
                | "m:box"
                | "m:groupChr"
                | "m:eqArr"
                | "m:m"
                | "m:phant"
        ) {
            e.child(&format!("{}Pr", e.name))?.child("m:ctrlPr")?;
        }
        for child in e.els() {
            scan(child, depth + 1, common, runs)?;
        }
        Some(())
    }
    let (mut common, mut runs) = (None, 0usize);
    scan(e, 0, &mut common, &mut runs)?;
    common.filter(|_| runs > 0)
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

/// The settings part's `m:mathPr`: how display equations wrap.
pub fn read_math_pr(e: &El) -> MathProps {
    let mut m = MathProps::default();
    let pr = Some(e);
    m.brk_bin = match val(pr, "m:brkBin") {
        Some("after") => BrkBin::After,
        Some("repeat") => BrkBin::Repeat,
        _ => BrkBin::Before,
    };
    m.brk_bin_sub = match val(pr, "m:brkBinSub") {
        Some("-+") => BrkBinSub::MinusPlus,
        Some("+-") => BrkBinSub::PlusMinus,
        _ => BrkBinSub::MinusMinus,
    };
    if let Some(v) = e.child("m:wrapIndent").and_then(|c| crate::units::tw(c, "m:val")).filter(|v| v.is_finite() && *v >= 0.0) {
        m.wrap_indent = v.min(1584.0);
    }
    m.wrap_right = prop(pr, "m:wrapRight", false);
    m
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
        // Tracked insertions show; deletions don't. Hyperlinks, simple fields and direction
        // wrappers (EG_PContentMath, §22.1.2.77) keep their runs (#1273).
        "w:ins" | "w:moveTo" | "w:smartTag" | "w:customXml" | "w:hyperlink" | "w:fldSimple" | "w:dir" | "w:bdo" | "m:oMath" if depth <= MAX_DEPTH => {
            for c in k.els() {
                node(c, d, out);
            }
        }
        // Markup compatibility: the math choice (we read math), else the fallback.
        "mc:AlternateContent" if depth <= MAX_DEPTH => {
            if let Some(c) = k.child("mc:Choice").or_else(|| k.child("mc:Fallback")) {
                for c in c.els() {
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

/// `m:t` (and `w:t`) text of a run, with symbol characters (`w:sym`) and special hyphens.
/// Text in a symbol font (Symbol, Wingdings…) becomes its Unicode equivalent: the math font has
/// no glyphs at those private-use codes (#1273).
fn run_text(r: &El) -> String {
    let font = r.child("w:rPr").and_then(|p| p.child("w:rFonts")).and_then(|f| f.attr("w:ascii").or_else(|| f.attr("w:hAnsi"))).unwrap_or("");
    let mut s = String::new();
    for c in r.els() {
        match c.name.as_str() {
            "m:t" | "w:t" => s.push_str(&symbol_fonts::map_text(font, &c.text())),
            "w:tab" => s.push('\t'),
            "w:noBreakHyphen" => s.push('\u{2011}'),
            "w:sym" => {
                let ch = c.attr("w:char").and_then(|h| u32::from_str_radix(h.trim(), 16).ok()).and_then(char::from_u32).filter(|c| *c as u32 >= 0x20);
                if let Some(ch) = ch {
                    let f = c.attr("w:font").unwrap_or(font);
                    s.push(symbol_fonts::symbol_char(f, ch).unwrap_or(ch));
                }
            }
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
        if let Some(b) = pr.child("m:brk") {
            let aln = b.attr("m:alnAt").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
            out.brk = Some(aln.min(255) as u8);
        }
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
        // Symbol-font text was mapped to Unicode: it takes the text font.
        if let Some(f) =
            pr.child("w:rFonts").and_then(|f| f.attr("w:ascii").or_else(|| f.attr("w:hAnsi"))).filter(|f| !symbol_fonts::is_symbol_font(f))
        {
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
    fn shared_math_size() {
        let run = |size: Option<u16>| {
            format!("<m:r><w:rPr>{}</w:rPr><m:t>x</m:t></m:r>", size.map(|v| format!("<w:sz w:val=\"{v}\"/>")).unwrap_or_default())
        };
        for size in [18, 28, 40] {
            let control = format!("<m:ctrlPr><w:rPr><w:sz w:val=\"{size}\"/></w:rPr></m:ctrlPr>");
            let uniform = format!("<m:sSup><m:sSupPr>{control}</m:sSupPr><m:e>{}</m:e><m:sup>{}</m:sup></m:sSup>", run(Some(size)), run(Some(size)));
            assert_eq!(shared_size(&omath(&uniform)), Some(f32::from(size) / 2.0));
            for inner in [
                uniform.replace(&run(Some(size)), &run(None)),
                uniform.replace(&control, ""),
                uniform.replace(&control, &control.replace(&format!("val=\"{size}\""), "val=\"22\"")),
                format!("{}{uniform}", run(Some(22))),
                uniform.replace("<m:e>", "<m:e><m:argPr><m:argSz m:val=\"-1\"/></m:argPr>"),
            ] {
                assert_eq!(shared_size(&omath(&inner)), None, "preserve operand/control boundaries: {inner}");
            }
        }
    }

    fn sized_run(size: Option<u16>) -> String {
        let prop = size.map(|v| format!("<w:sz w:val=\"{v}\"/>")).unwrap_or_default();
        format!("<m:r><w:rPr>{prop}</w:rPr><m:t>x</m:t></m:r>")
    }

    #[test]
    fn shared_size_deletions() {
        for tag in ["w:del", "w:moveFrom", "m:del"] {
            for deleted in [sized_run(Some(22)), sized_run(None), "<m:argPr><m:argSz m:val=\"-1\"/></m:argPr>".into()] {
                let e = omath(&format!("{}<{tag}>{deleted}</{tag}>", sized_run(Some(28))));
                assert_eq!(texts(&mut read_omath(&e, MathJc::CenterGroup)), ["x"]);
                assert_eq!(shared_size(&e), Some(14.0), "{tag}: {deleted}");
                assert_eq!(shared_size(&omath(&format!("<{tag}>{deleted}</{tag}>"))), None);
            }
        }
    }

    #[test]
    fn shared_size_insertions() {
        for tag in ["w:ins", "w:moveTo"] {
            for (size, expected) in [(Some(28), Some(14.0)), (Some(22), None), (None, None)] {
                let e = omath(&format!("{}<{tag}>{}</{tag}>", sized_run(Some(28)), sized_run(size)));
                assert_eq!(texts(&mut read_omath(&e, MathJc::CenterGroup)).concat(), "xx");
                assert_eq!(shared_size(&e), expected, "{tag}: {size:?}");
            }
        }
    }

    #[test]
    fn shared_size_nested_controls() {
        let script = |size| {
            format!(
                "<m:sSup><m:sSupPr><m:ctrlPr><w:rPr><w:sz w:val=\"{size}\"/></w:rPr></m:ctrlPr></m:sSupPr><m:e>{}</m:e><m:sup>{}</m:sup></m:sSup>",
                sized_run(Some(28)),
                sized_run(Some(28))
            )
        };
        let visible = script(28);
        let inner = format!("<w:ins><w:moveTo>{visible}<w:del>{}</w:del></w:moveTo></w:ins>", script(22));
        let e = omath(&inner);
        assert_eq!(texts(&mut read_omath(&e, MathJc::CenterGroup)), ["x", "x"]);
        assert_eq!(shared_size(&e), Some(14.0));
        for visible in [script(22), visible.replace("<m:e>", "<m:e><m:argPr><m:argSz m:val=\"-1\"/></m:argPr>")] {
            let e = omath(&format!("<w:ins>{visible}</w:ins>"));
            assert_eq!(shared_size(&e), None, "visible size guards: {visible}");
        }
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

    /// Every run's text in the tree, in reading order.
    fn texts(m: &mut Math) -> Vec<String> {
        let mut out = Vec::new();
        wordcraft_doc::math_edit::each_run_mut(&mut m.nodes, &mut |r| out.push(r.text.clone()));
        out
    }

    fn t(s: &str) -> String {
        format!("<m:r><m:t>{s}</m:t></m:r>")
    }

    /// #1273: one equation using every Office Math element of ECMA-376 §22.1 (each argument
    /// holding a distinct marker) keeps every argument's text: nothing is silently dropped.
    #[test]
    fn every_omml_element_keeps_its_content() {
        let parts = [
            format!("<m:acc><m:accPr><m:chr m:val=\"̃\"/></m:accPr><m:e>{}</m:e></m:acc>", t("A1")),
            format!("<m:bar><m:barPr><m:pos m:val=\"top\"/></m:barPr><m:e>{}</m:e></m:bar>", t("B1")),
            format!("<m:box><m:boxPr><m:opEmu m:val=\"1\"/></m:boxPr><m:e>{}</m:e></m:box>", t("C1")),
            format!("<m:borderBox><m:borderBoxPr><m:strikeH/></m:borderBoxPr><m:e>{}</m:e></m:borderBox>", t("D1")),
            format!("<m:d><m:dPr><m:begChr m:val=\"[\"/></m:dPr><m:e>{}</m:e><m:e>{}</m:e></m:d>", t("E1"), t("E2")),
            format!("<m:eqArr><m:e>{}</m:e><m:e>{}</m:e></m:eqArr>", t("F1"), t("F2")),
            format!("<m:f><m:fPr><m:type m:val=\"skw\"/></m:fPr><m:num>{}</m:num><m:den>{}</m:den></m:f>", t("G1"), t("G2")),
            format!("<m:func><m:fName>{}</m:fName><m:e>{}</m:e></m:func>", t("H1"), t("H2")),
            format!("<m:groupChr><m:groupChrPr><m:chr m:val=\"⏞\"/><m:pos m:val=\"top\"/></m:groupChrPr><m:e>{}</m:e></m:groupChr>", t("I1")),
            format!("<m:limLow><m:e>{}</m:e><m:lim>{}</m:lim></m:limLow>", t("J1"), t("J2")),
            format!("<m:limUpp><m:e>{}</m:e><m:lim>{}</m:lim></m:limUpp>", t("K1"), t("K2")),
            format!("<m:m><m:mr><m:e>{}</m:e><m:e>{}</m:e></m:mr><m:mr><m:e>{}</m:e><m:e/></m:mr></m:m>", t("L1"), t("L2"), t("L3")),
            format!(
                "<m:nary><m:naryPr><m:chr m:val=\"∑\"/></m:naryPr><m:sub>{}</m:sub><m:sup>{}</m:sup><m:e>{}</m:e></m:nary>",
                t("M1"),
                t("M2"),
                t("M3")
            ),
            format!("<m:phant><m:phantPr><m:show m:val=\"0\"/></m:phantPr><m:e>{}</m:e></m:phant>", t("N1")),
            format!("<m:rad><m:deg>{}</m:deg><m:e>{}</m:e></m:rad>", t("O1"), t("O2")),
            format!("<m:sPre><m:sub>{}</m:sub><m:sup>{}</m:sup><m:e>{}</m:e></m:sPre>", t("P1"), t("P2"), t("P3")),
            format!("<m:sSub><m:e>{}</m:e><m:sub>{}</m:sub></m:sSub>", t("Q1"), t("Q2")),
            format!("<m:sSubSup><m:e>{}</m:e><m:sub>{}</m:sub><m:sup>{}</m:sup></m:sSubSup>", t("R1"), t("R2"), t("R3")),
            format!("<m:sSup><m:e>{}</m:e><m:sup>{}</m:sup></m:sSup>", t("S1"), t("S2")),
            // Run-level wrappers allowed in math (EG_PContentMath): hyperlinks, simple fields,
            // tracked insertions, content controls, markup compatibility.
            format!("<w:hyperlink w:anchor=\"x\">{}</w:hyperlink>", t("T1")),
            format!("<w:fldSimple w:instr=\"QUOTE\">{}</w:fldSimple>", t("T2")),
            format!("<w:ins w:id=\"1\" w:author=\"a\">{}</w:ins>", t("T3")),
            format!("<w:sdt><w:sdtContent>{}</w:sdtContent></w:sdt>", t("T4")),
            format!(
                "<mc:AlternateContent><mc:Choice Requires=\"m\">{}</mc:Choice><mc:Fallback><w:r><w:t>picture</w:t></w:r></mc:Fallback></mc:AlternateContent>",
                t("T5")
            ),
            "<w:r><w:t>T6</w:t></w:r>".to_string(),
        ];
        let x = format!(
            r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">{}</m:oMath>"#,
            parts.concat()
        );
        let e = crate::xml::parse(x.as_bytes()).unwrap();
        let mut m = read_omath(&e, MathJc::CenterGroup);
        let got = texts(&mut m).concat();
        let markers = [
            "A1", "B1", "C1", "D1", "E1", "E2", "F1", "F2", "G1", "G2", "H1", "H2", "I1", "J1", "J2", "K1", "K2", "L1", "L2", "L3", "M1", "M2", "M3",
            "N1", "O1", "O2", "P1", "P2", "P3", "Q1", "Q2", "R1", "R2", "R3", "S1", "S2", "T1", "T2", "T3", "T4", "T5", "T6",
        ];
        for k in markers {
            assert!(got.contains(k), "{k} dropped from {got}");
        }
        assert!(!got.contains("picture"), "the math choice wins over the fallback: {got}");
    }

    /// #1273: deleted text stays out; symbol characters and symbol-font text become Unicode.
    #[test]
    fn symbols_and_deletions_in_math() {
        let e = omath(
            r#"<m:r><m:t>x</m:t></m:r><w:del w:id="1" w:author="a"><m:r><m:t>gone</m:t></m:r></w:del><m:r><w:sym w:font="Symbol" w:char="F061"/></m:r><m:r><w:rPr><w:rFonts w:ascii="Symbol" w:hAnsi="Symbol"/></w:rPr><m:t>&#xF062;</m:t><w:noBreakHyphen/></m:r><w:r><w:rPr><w:rFonts w:ascii="Symbol"/></w:rPr><w:t>&#xF067;</w:t></w:r>"#,
        );
        let mut m = read_omath(&e, MathJc::CenterGroup);
        let got = texts(&mut m).concat();
        assert_eq!(got, "xαβ\u{2011}γ");
        let mut font = None;
        wordcraft_doc::math_edit::each_run_mut(&mut m.nodes, &mut |r| {
            if r.nor {
                font = Some(r.font.clone());
            }
        });
        assert_eq!(font, Some(None), "mapped symbol text takes the text font");
    }

    /// Hostile nesting through the wrappers is bounded (no stack overflow, no panic).
    #[test]
    fn deeply_nested_wrappers_are_bounded() {
        // Deeper than the math limit, within the XML parser's own depth limit.
        let n = 120;
        let inner = format!("{}{}{}", "<w:hyperlink>".repeat(n), t("deep"), "</w:hyperlink>".repeat(n));
        let e = omath(&inner);
        let _ = read_omath(&e, MathJc::CenterGroup);
        let inner = format!(
            "{}{}{}",
            "<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice Requires=\"m\">".repeat(n),
            t("deep"),
            "</mc:Choice></mc:AlternateContent>".repeat(n)
        );
        let e = omath(&inner);
        let _ = read_omath(&e, MathJc::CenterGroup);
    }
}
