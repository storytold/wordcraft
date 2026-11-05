//! A DrawingML locked canvas (`lc:lockedCanvas`, ECMA-376 Part 1 §20.3.2.1): the shapes,
//! pictures and text Word 2007 writes in a drawing in compatibility mode (#5060). It is a group
//! shape in the DrawingML namespace (`a:sp`, `a:pic`, `a:grpSp`, `a:cxnSp`, text in `a:txSp`), so
//! it is read as the `wpg:wgp` group the reader already knows: each member renamed into its
//! WordprocessingML drawing form, and its DrawingML text into `w:txbxContent` paragraphs.

use super::{El, Node, Rgb};

/// Groups nested deeper than this are left out, as `group_members` does.
const MAX_NEST: usize = 8;
/// DrawingML's text size when neither the run nor the list style gives one (18 pt, in
/// hundredths of a point).
const DEFAULT_SZ: i64 = 1800;

/// The locked canvas `lc` as a `wpg:wgp` group. `theme` resolves its text colours.
pub(super) fn as_group(lc: &El, theme: &[Rgb]) -> El {
    el("wpg:wgp", Vec::new(), members(lc, theme, 0))
}

/// The members of DrawingML group `g`, in their WordprocessingML drawing form.
fn members(g: &El, theme: &[Rgb], depth: usize) -> Vec<Node> {
    g.els()
        .filter_map(|e| match e.name.as_str() {
            "a:grpSpPr" => Some(renamed(e, "wpg:grpSpPr")),
            "a:grpSp" if depth < MAX_NEST => Some(el("wpg:grpSp", Vec::new(), members(e, theme, depth + 1))),
            "a:sp" => Some(shape(e, theme)),
            "a:cxnSp" => Some(connector(e)),
            "a:pic" => Some(picture(e)),
            "a:txSp" => Some(text_shape(e, theme)),
            _ => None,
        })
        .map(Node::El)
        .collect()
}

/// `a:sp` as `wps:wsp`: its name, properties and style, and its text as a text box's.
fn shape(sp: &El, theme: &[Rgb]) -> El {
    let nv = sp.child("a:nvSpPr");
    let mut kids: Vec<Node> = [
        nv.and_then(|n| n.child("a:cNvPr")).map(|e| renamed(e, "wps:cNvPr")),
        nv.and_then(|n| n.child("a:cNvSpPr")).map(|e| renamed(e, "wps:cNvSpPr")),
        sp.child("a:spPr").map(|e| renamed(e, "wps:spPr")),
        sp.child("a:style").map(|e| renamed(e, "wps:style")),
    ]
    .into_iter()
    .flatten()
    .map(Node::El)
    .collect();
    // The text sits in the shape's own rectangle (`a:useSpRect`); a text rectangle of its own
    // (`a:txSp/a:xfrm`) is not kept apart.
    if let Some(body) = sp.child("a:txSp").and_then(|t| t.child("a:txBody")) {
        kids.extend(text_box(body, theme));
    }
    el("wps:wsp", Vec::new(), kids)
}

/// A group's text shape (`a:txSp` as a member, text with no shape of its own) as a `wps:wsp` text
/// box without fill or outline at its own rectangle (`a:xfrm`).
fn text_shape(tx: &El, theme: &[Rgb]) -> El {
    let none = || Node::El(el("a:noFill", Vec::new(), Vec::new()));
    let geometry = el("a:prstGeom", attrs(&[("prst", "rect")]), vec![Node::El(el("a:avLst", Vec::new(), Vec::new()))]);
    let mut sp_pr: Vec<Node> = tx.child("a:xfrm").map(|x| Node::El(x.clone())).into_iter().collect();
    sp_pr.extend([Node::El(geometry), none(), Node::El(el("a:ln", Vec::new(), vec![none()]))]);
    let mut kids = vec![Node::El(el("wps:cNvSpPr", attrs(&[("txBox", "1")]), Vec::new())), Node::El(el("wps:spPr", Vec::new(), sp_pr))];
    if let Some(body) = tx.child("a:txBody") {
        kids.extend(text_box(body, theme));
    }
    el("wps:wsp", Vec::new(), kids)
}

/// DrawingML text body `body` as a text box's content and body properties.
fn text_box(body: &El, theme: &[Rgb]) -> Vec<Node> {
    let content = el("w:txbxContent", Vec::new(), paragraphs(body, theme));
    let mut kids = vec![Node::El(el("wps:txbx", Vec::new(), vec![Node::El(content)]))];
    if let Some(bp) = body.child("a:bodyPr") {
        kids.push(Node::El(renamed(bp, "wps:bodyPr")));
    }
    kids
}

/// `a:cxnSp` as a `wps:wsp` line. Its glued ends name shapes by the canvas's own ids, which
/// aren't the document's, so they are left out.
fn connector(cx: &El) -> El {
    let kids = [
        cx.child("a:nvCxnSpPr").and_then(|n| n.child("a:cNvPr")).map(|e| renamed(e, "wps:cNvPr")),
        cx.child("a:spPr").map(|e| renamed(e, "wps:spPr")),
        cx.child("a:style").map(|e| renamed(e, "wps:style")),
    ];
    el("wps:wsp", Vec::new(), kids.into_iter().flatten().map(Node::El).collect())
}

/// `a:pic` as `pic:pic`.
fn picture(pic: &El) -> El {
    let nv = pic.child("a:nvPicPr").and_then(|n| n.child("a:cNvPr")).map(|e| el("pic:nvPicPr", Vec::new(), vec![Node::El(renamed(e, "pic:cNvPr"))]));
    let kids = [nv, pic.child("a:blipFill").map(|e| renamed(e, "pic:blipFill")), pic.child("a:spPr").map(|e| renamed(e, "pic:spPr"))];
    el("pic:pic", Vec::new(), kids.into_iter().flatten().map(Node::El).collect())
}

/// The paragraphs of DrawingML text body `body` as `w:p`s. DrawingML text has no space before
/// or after a paragraph and single lines, so each says so rather than taking the document's
/// defaults.
fn paragraphs(body: &El, theme: &[Rgb]) -> Vec<Node> {
    body.children("a:p")
        .map(|p| {
            // A run's properties default to its paragraph's own (`a:pPr/a:defRPr`), then to the
            // list style's for the paragraph's level (`lvl`, 0–8), property by property.
            let ppr_el = p.child("a:pPr");
            let level = ppr_el.and_then(|pp| pp.attr("lvl")).and_then(|v| v.trim().parse::<u8>().ok()).filter(|l| *l <= 8).unwrap_or(0);
            let level_style = body.child("a:lstStyle").and_then(|l| l.child(&format!("a:lvl{}pPr", level + 1)));
            let defaults: Vec<&El> =
                [ppr_el.and_then(|pp| pp.child("a:defRPr")), level_style.and_then(|l| l.child("a:defRPr"))].into_iter().flatten().collect();
            let defaults = defaults.as_slice();
            let mut ppr =
                vec![Node::El(el("w:spacing", attrs(&[("w:before", "0"), ("w:after", "0"), ("w:line", "240"), ("w:lineRule", "auto")]), Vec::new()))];
            let jc = match ppr_el.and_then(|pp| pp.attr("algn")) {
                Some("ctr") => Some("center"),
                Some("r") => Some("right"),
                Some("just") => Some("both"),
                Some("dist") => Some("distribute"),
                Some("l") => Some("left"),
                _ => None,
            };
            if let Some(jc) = jc {
                ppr.push(Node::El(el("w:jc", attrs(&[("w:val", jc)]), Vec::new())));
            }
            // The paragraph mark's own size and font (`a:endParaRPr`), which an empty one's
            // height follows.
            ppr.push(Node::El(run_props(p.child("a:endParaRPr"), defaults, theme)));
            let mut kids = vec![Node::El(el("w:pPr", Vec::new(), ppr))];
            for r in p.els() {
                let text = match r.name.as_str() {
                    "a:r" | "a:fld" => Node::El(el(
                        "w:t",
                        attrs(&[("xml:space", "preserve")]),
                        vec![Node::Text(r.child("a:t").map(|t| t.text()).unwrap_or_default())],
                    )),
                    "a:br" => Node::El(el("w:br", Vec::new(), Vec::new())),
                    _ => continue,
                };
                let rpr = run_props(r.child("a:rPr"), defaults, theme);
                kids.push(Node::El(el("w:r", Vec::new(), vec![Node::El(rpr), text])));
            }
            Node::El(el("w:p", Vec::new(), kids))
        })
        .collect()
}

/// A DrawingML run's properties (`a:rPr`, each falling back on the first of `defaults`, the
/// `a:defRPr`s in order, that gives it) as `w:rPr`: font, size, bold, italic, underline,
/// strikethrough, caps, baseline and colour.
fn run_props(rpr: Option<&El>, defaults: &[&El], theme: &[Rgb]) -> El {
    let get = |n: &str| rpr.and_then(|r| r.attr(n)).or_else(|| defaults.iter().find_map(|d| d.attr(n)));
    let child = |n: &str| rpr.and_then(|r| r.child(n)).or_else(|| defaults.iter().find_map(|d| d.child(n)));
    let on = |n: &str| get(n).is_some_and(|v| v == "1" || v == "true");
    let mut kids = Vec::new();
    let mut push = |name: &str, a: &[(&str, &str)]| kids.push(Node::El(el(name, attrs(a), Vec::new())));
    // Theme fonts (`+mn-lt`) are left to the document's.
    let face = |n: &str| child(n).and_then(|f| f.attr("typeface")).filter(|t| !t.is_empty() && !t.starts_with('+'));
    let mut fonts = Vec::new();
    if let Some(f) = face("a:latin") {
        fonts.extend([("w:ascii", f), ("w:hAnsi", f)]);
    }
    if let Some(f) = face("a:ea") {
        fonts.push(("w:eastAsia", f));
    }
    if let Some(f) = face("a:cs") {
        fonts.push(("w:cs", f));
    }
    if !fonts.is_empty() {
        push("w:rFonts", &fonts);
    }
    if on("b") {
        push("w:b", &[]);
    }
    if on("i") {
        push("w:i", &[]);
    }
    match get("cap") {
        Some("all") => push("w:caps", &[]),
        Some("small") => push("w:smallCaps", &[]),
        _ => {}
    }
    match get("strike") {
        Some("sngStrike") => push("w:strike", &[]),
        Some("dblStrike") => push("w:dstrike", &[]),
        _ => {}
    }
    match get("u") {
        None | Some("none") => {}
        Some("dbl") => push("w:u", &[("w:val", "double")]),
        Some(_) => push("w:u", &[("w:val", "single")]),
    }
    match get("baseline").and_then(|v| v.trim().parse::<i64>().ok()) {
        Some(b) if b > 0 => push("w:vertAlign", &[("w:val", "superscript")]),
        Some(b) if b < 0 => push("w:vertAlign", &[("w:val", "subscript")]),
        _ => {}
    }
    if let Some(rgb) = child("a:solidFill").and_then(|f| crate::read::drawing_color::color(f, theme)) {
        push("w:color", &[("w:val", &rgb.hex())]);
    }
    // Hundredths of a point into half-points, within Word's 1–1638 pt.
    let sz = get("sz").and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(DEFAULT_SZ).clamp(100, 163_800) / 50;
    let sz = sz.to_string();
    push("w:sz", &[("w:val", &sz)]);
    push("w:szCs", &[("w:val", &sz)]);
    // WordArt's outline, shadow, glow and reflection, as Word 2010 writes them in `w:rPr`.
    kids.extend(text_effects(child("a:ln"), child("a:effectLst"), theme).into_iter().map(Node::El));
    el("w:rPr", Vec::new(), kids)
}

/// A DrawingML run's text outline (`a:ln`) and effects (`a:effectLst`: outer shadow, glow,
/// reflection) as the `w14:` run properties ([MS-DOCX] §2.6.1) the run reader takes for WordArt.
/// Colours are resolved here, theme and modifiers included, into a `w14:srgbClr` with its
/// transparency. Lengths (EMUs), angles and percentages carry over as they are.
fn text_effects(ln: Option<&El>, fx: Option<&El>, theme: &[Rgb]) -> Vec<El> {
    let color = |e: &El| -> Option<Node> {
        let (rgb, transparency) = crate::read::drawing_color::color_alpha(e, theme, None)?;
        let alpha = ((transparency.clamp(0.0, 100.0) * 1000.0).round() as i64).to_string();
        let kids = if transparency > 0.0 { vec![Node::El(el("w14:alpha", attrs(&[("w14:val", &alpha)]), Vec::new()))] } else { Vec::new() };
        Some(Node::El(el("w14:srgbClr", attrs(&[("w14:val", &rgb.hex())]), kids)))
    };
    // The attributes `names` of `e`, under the `w14:` prefix.
    let copied = |e: &El, names: &[&str]| -> Vec<(String, String)> {
        names.iter().filter_map(|n| e.attr(n).map(|v| (format!("w14:{n}"), v.to_string()))).collect()
    };
    let mut out = Vec::new();
    if let Some(ln) = ln
        && let Some(c) = ln.child("a:solidFill").and_then(color)
    {
        out.push(el("w14:textOutline", copied(ln, &["w"]), vec![Node::El(el("w14:solidFill", Vec::new(), vec![c]))]));
    }
    for e in fx.into_iter().flat_map(|f| f.els()) {
        match e.name.as_str() {
            "a:outerShdw" => out.push(el("w14:shadow", copied(e, &["blurRad", "dist", "dir"]), color(e).into_iter().collect())),
            "a:glow" => out.push(el("w14:glow", copied(e, &["rad"]), color(e).into_iter().collect())),
            "a:reflection" => out.push(el("w14:reflection", copied(e, &["blurRad", "stA", "endPos", "dist"]), Vec::new())),
            _ => {}
        }
    }
    out
}

fn el(name: &str, attrs: Vec<(String, String)>, kids: Vec<Node>) -> El {
    El { name: name.to_string(), attrs, kids }
}

fn attrs(a: &[(&str, &str)]) -> Vec<(String, String)> {
    a.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// `e` under another name, with its attributes and children.
fn renamed(e: &El, name: &str) -> El {
    El { name: name.to_string(), ..e.clone() }
}
