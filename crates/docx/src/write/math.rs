//! [`wordcraft_doc::math`] tree → Office Math (OMML).

use wordcraft_doc::math::{Arg, ColJc, FracKind, LimLoc, MAX_DEPTH, MNode, MRun, MScr, MSty, Math, MathJc, ScriptKind, parse_linear};

use crate::xml::W;

/// Write an equation: `m:oMath`, inside `m:oMathPara` for a display equation. The source OMML
/// is written back unchanged when the equation came from a file.
pub fn write_equation(w: &mut W, linear: &str, display: bool, math: &Math) {
    if display {
        w.open("m:oMathPara", &[]);
        let jc = match math.jc {
            MathJc::CenterGroup => None,
            MathJc::Center => Some("center"),
            MathJc::Left => Some("left"),
            MathJc::Right => Some("right"),
        };
        if let Some(jc) = jc {
            w.open("m:oMathParaPr", &[]);
            w.empty("m:jc", &[("m:val", jc)]);
            w.close("m:oMathParaPr");
        }
    }
    if !math.omml.is_empty() && math.omml.starts_with("<m:oMath") && crate::xml::parse(wrap_ns(&math.omml).as_bytes()).is_ok() {
        w.raw(&math.omml);
    } else {
        let parsed;
        let nodes: &Arg = if math.nodes.is_empty() {
            parsed = parse_linear(linear);
            &parsed
        } else {
            &math.nodes
        };
        w.open("m:oMath", &[]);
        arg_inner(w, nodes, 0);
        w.close("m:oMath");
    }
    if display {
        w.close("m:oMathPara");
    }
}

/// Count the equation's numbered rows; when some use an automatic number (`#` alone), a copy
/// with the numbers written out as `(n)`.
pub fn resolve_numbers(math: &Math, display: bool, counter: &mut u32) -> Option<Math> {
    if !display {
        return None;
    }
    let [MNode::EqArr { rows }] = math.nodes.as_slice() else { return None };
    let mut out = rows.clone();
    let mut changed = false;
    for row in out.iter_mut() {
        if !wordcraft_doc::math::has_number_mark(row) {
            continue;
        }
        *counter = counter.saturating_add(1);
        // Nothing after the `#`: an automatic number.
        let mut after = false;
        let mut empty = true;
        for n in row.iter() {
            match n {
                MNode::Run(r) if !after => {
                    if let Some(k) = r.text.find('#') {
                        after = true;
                        if !r.text.get(k + 1..).unwrap_or("").is_empty() {
                            empty = false;
                        }
                    }
                }
                _ if after => empty = false,
                _ => {}
            }
        }
        if empty {
            let n = vec![MNode::Run(MRun::new(counter.to_string()))];
            row.push(MNode::Delim { beg: Some('('), end: Some(')'), sep: None, grow: true, shp_match: false, elems: vec![n] });
            changed = true;
        }
    }
    changed.then(|| Math { nodes: vec![MNode::EqArr { rows: out }], omml: String::new(), ..math.clone() })
}

/// The kept XML with namespace declarations, for validating it on its own.
fn wrap_ns(xml: &str) -> String {
    let decl: String = crate::xml::body_ns().iter().filter(|(k, _)| k.starts_with("xmlns:")).map(|(k, v)| format!(" {k}=\"{v}\"")).collect();
    format!("<w:x{decl}>{xml}</w:x>")
}

fn val(w: &mut W, name: &str, v: &str) {
    w.empty(name, &[("m:val", v)]);
}

fn arg(w: &mut W, name: &str, a: &[MNode], depth: usize) {
    if a.is_empty() {
        w.empty(name, &[]);
        return;
    }
    w.open(name, &[]);
    arg_inner(w, a, depth + 1);
    w.close(name);
}

fn arg_inner(w: &mut W, a: &[MNode], depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for n in a {
        node(w, n, depth);
    }
}

fn ctrl(w: &mut W) {
    w.open("m:ctrlPr", &[]);
    w.open("w:rPr", &[]);
    w.empty("w:rFonts", &[("w:ascii", "Cambria Math"), ("w:hAnsi", "Cambria Math")]);
    w.empty("w:i", &[]);
    w.close("w:rPr");
    w.close("m:ctrlPr");
}

fn char_prop(w: &mut W, name: &str, c: Option<char>) {
    let s = c.map(|c| c.to_string()).unwrap_or_default();
    val(w, name, &s);
}

fn node(w: &mut W, n: &MNode, depth: usize) {
    match n {
        MNode::Run(r) => run(w, r),
        MNode::Frac { kind, num, den } => {
            w.open("m:f", &[]);
            w.open("m:fPr", &[]);
            match kind {
                FracKind::Bar => {}
                FracKind::Skewed => val(w, "m:type", "skw"),
                FracKind::Linear => val(w, "m:type", "lin"),
                FracKind::NoBar => val(w, "m:type", "noBar"),
            }
            ctrl(w);
            w.close("m:fPr");
            arg(w, "m:num", num, depth);
            arg(w, "m:den", den, depth);
            w.close("m:f");
        }
        MNode::Script { kind, base, sub, sup } => {
            let (el, pr) = match kind {
                ScriptKind::Sup => ("m:sSup", "m:sSupPr"),
                ScriptKind::Sub => ("m:sSub", "m:sSubPr"),
                ScriptKind::SubSup => ("m:sSubSup", "m:sSubSupPr"),
                ScriptKind::Pre => ("m:sPre", "m:sPrePr"),
            };
            w.open(el, &[]);
            w.open(pr, &[]);
            ctrl(w);
            w.close(pr);
            if *kind == ScriptKind::Pre {
                arg(w, "m:sub", sub, depth);
                arg(w, "m:sup", sup, depth);
                arg(w, "m:e", base, depth);
            } else {
                arg(w, "m:e", base, depth);
                if matches!(kind, ScriptKind::Sub | ScriptKind::SubSup) {
                    arg(w, "m:sub", sub, depth);
                }
                if matches!(kind, ScriptKind::Sup | ScriptKind::SubSup) {
                    arg(w, "m:sup", sup, depth);
                }
            }
            w.close(el);
        }
        MNode::Rad { deg, deg_hide, e } => {
            w.open("m:rad", &[]);
            w.open("m:radPr", &[]);
            if *deg_hide {
                val(w, "m:degHide", "1");
            }
            ctrl(w);
            w.close("m:radPr");
            arg(w, "m:deg", deg, depth);
            arg(w, "m:e", e, depth);
            w.close("m:rad");
        }
        MNode::Nary { chr, lim_loc, grow, sub_hide, sup_hide, sub, sup, e } => {
            w.open("m:nary", &[]);
            w.open("m:naryPr", &[]);
            if *chr != '∫' {
                char_prop(w, "m:chr", Some(*chr));
            }
            match lim_loc {
                Some(LimLoc::UndOvr) => val(w, "m:limLoc", "undOvr"),
                Some(LimLoc::SubSup) => val(w, "m:limLoc", "subSup"),
                None => {}
            }
            if *grow {
                val(w, "m:grow", "1");
            }
            if *sub_hide {
                val(w, "m:subHide", "1");
            }
            if *sup_hide {
                val(w, "m:supHide", "1");
            }
            ctrl(w);
            w.close("m:naryPr");
            arg(w, "m:sub", sub, depth);
            arg(w, "m:sup", sup, depth);
            arg(w, "m:e", e, depth);
            w.close("m:nary");
        }
        MNode::Delim { beg, end, sep, grow, shp_match, elems } => {
            w.open("m:d", &[]);
            w.open("m:dPr", &[]);
            if *beg != Some('(') {
                char_prop(w, "m:begChr", *beg);
            }
            if *sep != Some('|') {
                char_prop(w, "m:sepChr", *sep);
            }
            if *end != Some(')') {
                char_prop(w, "m:endChr", *end);
            }
            if !grow {
                val(w, "m:grow", "0");
            }
            if *shp_match {
                val(w, "m:shp", "match");
            }
            ctrl(w);
            w.close("m:dPr");
            for e in elems {
                arg(w, "m:e", e, depth);
            }
            w.close("m:d");
        }
        MNode::Func { name, e } => {
            w.open("m:func", &[]);
            w.open("m:funcPr", &[]);
            ctrl(w);
            w.close("m:funcPr");
            arg(w, "m:fName", name, depth);
            arg(w, "m:e", e, depth);
            w.close("m:func");
        }
        MNode::Lim { upper, e, lim } => {
            let (el, pr) = if *upper { ("m:limUpp", "m:limUppPr") } else { ("m:limLow", "m:limLowPr") };
            w.open(el, &[]);
            w.open(pr, &[]);
            ctrl(w);
            w.close(pr);
            arg(w, "m:e", e, depth);
            arg(w, "m:lim", lim, depth);
            w.close(el);
        }
        MNode::Acc { chr, e } => {
            w.open("m:acc", &[]);
            w.open("m:accPr", &[]);
            char_prop(w, "m:chr", Some(*chr));
            ctrl(w);
            w.close("m:accPr");
            arg(w, "m:e", e, depth);
            w.close("m:acc");
        }
        MNode::Bar { top, e } => {
            w.open("m:bar", &[]);
            w.open("m:barPr", &[]);
            val(w, "m:pos", if *top { "top" } else { "bot" });
            ctrl(w);
            w.close("m:barPr");
            arg(w, "m:e", e, depth);
            w.close("m:bar");
        }
        MNode::BorderBox { hide, strike, e } => {
            w.open("m:borderBox", &[]);
            w.open("m:borderBoxPr", &[]);
            for (on, name) in hide.iter().zip(["m:hideTop", "m:hideBot", "m:hideLeft", "m:hideRight"]) {
                if *on {
                    val(w, name, "1");
                }
            }
            for (on, name) in strike.iter().zip(["m:strikeH", "m:strikeV", "m:strikeBLTR", "m:strikeTLBR"]) {
                if *on {
                    val(w, name, "1");
                }
            }
            ctrl(w);
            w.close("m:borderBoxPr");
            arg(w, "m:e", e, depth);
            w.close("m:borderBox");
        }
        MNode::Boxed { e } => {
            w.open("m:box", &[]);
            w.open("m:boxPr", &[]);
            ctrl(w);
            w.close("m:boxPr");
            arg(w, "m:e", e, depth);
            w.close("m:box");
        }
        MNode::GroupChr { chr, top, e } => {
            w.open("m:groupChr", &[]);
            w.open("m:groupChrPr", &[]);
            char_prop(w, "m:chr", Some(*chr));
            val(w, "m:pos", if *top { "top" } else { "bot" });
            val(w, "m:vertJc", if *top { "bot" } else { "top" });
            ctrl(w);
            w.close("m:groupChrPr");
            arg(w, "m:e", e, depth);
            w.close("m:groupChr");
        }
        MNode::EqArr { rows } => {
            w.open("m:eqArr", &[]);
            w.open("m:eqArrPr", &[]);
            // Numbered rows (`#`) stretch across the line so the number reaches the margin.
            if rows.iter().any(|r| wordcraft_doc::math::has_number_mark(r)) {
                val(w, "m:maxDist", "1");
            }
            ctrl(w);
            w.close("m:eqArrPr");
            for r in rows {
                arg(w, "m:e", r, depth);
            }
            w.close("m:eqArr");
        }
        MNode::Matrix { rows, col_jc } => {
            let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0).max(1);
            w.open("m:m", &[]);
            w.open("m:mPr", &[]);
            w.open("m:mcs", &[]);
            for c in 0..cols {
                let jc = match col_jc.get(c).copied().unwrap_or_default() {
                    ColJc::Center => "center",
                    ColJc::Left => "left",
                    ColJc::Right => "right",
                };
                w.open("m:mc", &[]);
                w.open("m:mcPr", &[]);
                val(w, "m:count", "1");
                val(w, "m:mcJc", jc);
                w.close("m:mcPr");
                w.close("m:mc");
            }
            w.close("m:mcs");
            ctrl(w);
            w.close("m:mPr");
            for r in rows {
                w.open("m:mr", &[]);
                for c in 0..cols {
                    arg(w, "m:e", r.get(c).map(|v| v.as_slice()).unwrap_or(&[]), depth);
                }
                w.close("m:mr");
            }
            w.close("m:m");
        }
        MNode::Phant { show, zero_wid, zero_asc, zero_desc, e } => {
            w.open("m:phant", &[]);
            w.open("m:phantPr", &[]);
            val(w, "m:show", if *show { "1" } else { "0" });
            for (on, name) in [(*zero_wid, "m:zeroWid"), (*zero_asc, "m:zeroAsc"), (*zero_desc, "m:zeroDesc")] {
                if on {
                    val(w, name, "1");
                }
            }
            ctrl(w);
            w.close("m:phantPr");
            arg(w, "m:e", e, depth);
            w.close("m:phant");
        }
    }
}

fn run(w: &mut W, r: &MRun) {
    w.open("m:r", &[]);
    let sty = r.sty.map(|s| match s {
        MSty::Plain => "p",
        MSty::Bold => "b",
        MSty::Italic => "i",
        MSty::BoldItalic => "bi",
    });
    let scr = match r.scr {
        MScr::Roman => None,
        MScr::Script => Some("script"),
        MScr::Fraktur => Some("fraktur"),
        MScr::DoubleStruck => Some("double-struck"),
        MScr::SansSerif => Some("sans-serif"),
        MScr::Monospace => Some("monospace"),
    };
    if sty.is_some() || scr.is_some() || r.nor || r.lit {
        w.open("m:rPr", &[]);
        if r.lit {
            w.empty("m:lit", &[]);
        }
        if r.nor {
            w.empty("m:nor", &[]);
        }
        if let Some(s) = scr {
            val(w, "m:scr", s);
        }
        if let Some(s) = sty {
            val(w, "m:sty", s);
        }
        w.close("m:rPr");
    }
    w.open("w:rPr", &[]);
    let font = r.font.as_deref().filter(|_| r.nor).unwrap_or("Cambria Math");
    w.empty("w:rFonts", &[("w:ascii", font), ("w:hAnsi", font)]);
    if let Some(c) = r.color {
        w.val("w:color", &c.hex());
    }
    if let Some(sz) = r.size {
        let hp = (sz * 2.0).round().clamp(2.0, 3276.0) as i64;
        w.val("w:sz", &hp.to_string());
        w.val("w:szCs", &hp.to_string());
    }
    w.close("w:rPr");
    w.leaf("m:t", &[("xml:space", "preserve")], &r.text);
    w.close("m:r");
}
