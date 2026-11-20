//! Pictures, shapes, text boxes and groups -> DrawingML (inline or anchored).
//!
//! Objects that are "treated as characters" stay inline; the others are anchored at their
//! position with their text wrapping, so they no longer push the text around (WordCraft renders
//! anchored pictures, wps shapes, text boxes and custom geometry; measured).
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use crate::convert::{Converter, WPS_URI, ext_of, render_blocks, signed32};
use crate::docx::{attr, pixel_size};
use crate::header::{color, num, truthy};
use crate::units::{emu_signed, round_even, trunc};
use crate::xml::Element;

const PIC_URI: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
const WPG_URI: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup";
const OBJECTS: &[&str] =
    &["hp:pic", "hp:container", "hp:rect", "hp:ellipse", "hp:arc", "hp:line", "hp:polygon", "hp:curve", "hp:connectLine", "hp:ole", "hp:textart"];
/// Deepest group nesting followed (hostile files).
const MAX_GROUP_DEPTH: usize = 32;

fn dash(style: &str) -> Option<&'static str> {
    Some(match style {
        "DASH" => "dash",
        "DOT" | "CIRCLE" => "sysDot",
        "DASH_DOT" => "dashDot",
        "DASH_DOT_DOT" => "lgDashDotDot",
        "LONG_DASH" => "lgDash",
        _ => return None,
    })
}

fn arrow(style: &str) -> Option<&'static str> {
    Some(match style {
        "ARROW" => "arrow",
        "SPEAR" => "triangle",
        "CONCAVE_ARROW" => "stealth",
        "EMPTY_DIAMOND" | "FILLED_DIAMOND" | "EMPTY_BOX" | "FILLED_BOX" => "diamond",
        "EMPTY_CIRCLE" | "FILLED_CIRCLE" => "oval",
        _ => return None,
    })
}

fn h_from(v: &str) -> &'static str {
    match v {
        "PAPER" => "page",
        "PAGE" => "margin",
        _ => "column",
    }
}

fn v_from(v: &str) -> &'static str {
    match v {
        "PAPER" => "page",
        "PAGE" => "margin",
        _ => "paragraph",
    }
}

fn anchor_v(v: &str) -> &'static str {
    match v {
        "CENTER" => "ctr",
        "BOTTOM" => "b",
        _ => "t",
    }
}

/// HWPUNIT -> EMU keeping the sign (a fractional value is cut first).
fn emu(hu: f64) -> i64 {
    emu_signed(hu)
}

/// Degrees -> DrawingML angle (60000ths, within one turn).
fn rot60k(deg: i64) -> i64 {
    deg.saturating_mul(60000).rem_euclid(21_600_000)
}

/// hc:gradation -> a:gradFill. Types: 1/LINEAR, 2/RADIAL (circular: light centre, as the viewer
/// draws it), 3/CONICAL (drawn round too), 4/SQUARE.
pub(crate) fn gradient_fill(kind: &str, angle: i64, cols: &[String]) -> String {
    let n = cols.len().saturating_sub(1).max(1) as f64;
    let stops: String =
        cols.iter().enumerate().map(|(k, c)| format!("<a:gs pos=\"{}\"><a:srgbClr val=\"{c}\"/></a:gs>", trunc(100000.0 * k as f64 / n))).collect();
    let kind = match kind {
        "1" => "LINEAR",
        "2" => "RADIAL",
        "3" => "CONICAL",
        "4" => "SQUARE",
        k => k,
    };
    let shade = if matches!(kind, "RADIAL" | "CONICAL" | "SQUARE") {
        let path = if kind == "SQUARE" { "rect" } else { "circle" };
        format!("<a:path path=\"{path}\"><a:fillToRect l=\"50000\" t=\"50000\" r=\"50000\" b=\"50000\"/></a:path>")
    } else {
        format!("<a:lin ang=\"{}\" scaled=\"0\"/>", rot60k(angle))
    };
    format!("<a:gradFill rotWithShape=\"1\"><a:gsLst>{stops}</a:gsLst>{shade}</a:gradFill>")
}

/// Where and how an object sits.
#[derive(Clone, Debug)]
pub(crate) struct Placement {
    pub inline: bool,
    /// EMU: left, right, top, bottom
    pub dist: [i64; 4],
    pub z: i64,
    pub behind: bool,
    pub h_from: String,
    pub v_from: String,
    pub x: i64,
    pub y: i64,
    pub align_h: String,
    pub align_v: String,
    pub wrap: String,
}

/// The object's size: its current, frame or original size, whichever is given first. A
/// level or upright line has no height or no width (its original size is only the space its
/// points are given in).
pub(crate) fn size_of(el: &Element) -> (i64, i64) {
    let line = matches!(el.tag.as_str(), "hp:line" | "hp:connectLine");
    for tag in ["hp:curSz", "hp:sz", "hp:orgSz"] {
        if let Some(s) = el.find(tag) {
            let (w, h) = (num(s.get("width")), num(s.get("height")));
            if (w > 0 && h > 0) || (line && w.max(h) > 0 && w.min(h) >= 0) {
                return (w, h);
            }
        }
    }
    (0, 0)
}

fn org_size(el: &Element, w: f64, h: f64) -> (f64, f64) {
    match el.find("hp:orgSz") {
        Some(o) if num(o.get("width")) > 0 && num(o.get("height")) > 0 => (num(o.get("width")) as f64, num(o.get("height")) as f64),
        _ => (w, h),
    }
}

fn angle_of(el: &Element) -> i64 {
    el.find("hp:rotationInfo").map(|r| num(r.get("angle"))).unwrap_or(0)
}

/// A turned object's frame (hp:sz) is the box around the turned shape; DrawingML wants the
/// shape's own size (hp:curSz), turned about the frame's centre. Group members are already
/// placed unturned.
fn unrotated(el: &Element, x: f64, y: f64, w: f64, h: f64, grouped: bool) -> (f64, f64, f64, f64) {
    let Some(cur) = el.find("hp:curSz") else { return (x, y, w, h) };
    if grouped || angle_of(el) == 0 {
        return (x, y, w, h);
    }
    let (cw, ch) = (num(cur.get("width")) as f64, num(cur.get("height")) as f64);
    if cw <= 0.0 || ch <= 0.0 || (cw == w && ch == h) {
        return (x, y, w, h);
    }
    (x + (w - cw) / 2.0, y + (h - ch) / 2.0, cw, ch)
}

/// hp:imgClip -> a:srcRect. The clip is in the picture's own space: hp:imgDim when given, else
/// its pixels at 96 dpi (75 HWPUNIT each), widened to the clip if larger.
fn crop(el: &Element, data: &[u8]) -> String {
    let Some(clip) = el.find("hp:imgClip") else { return String::new() };
    let (l, t, r, b) = (num(clip.get("left")), num(clip.get("top")), num(clip.get("right")), num(clip.get("bottom")));
    let (mut fw, mut fh) = match el.find("hp:imgDim").filter(|d| num(d.get("dimwidth")) > 0) {
        Some(d) => (num(d.get("dimwidth")), num(d.get("dimheight"))),
        None => {
            let px = pixel_size(data).unwrap_or((0, 0));
            (i64::from(px.0) * 75, i64::from(px.1) * 75)
        }
    };
    fw = fw.max(r);
    fh = fh.max(b);
    if fw <= 0 || fh <= 0 || r <= l || b <= t || (l, t, r, b) == (0, 0, fw, fh) {
        return String::new();
    }
    let (fwf, fhf) = (fw as f64, fh as f64);
    let sides = [l as f64 / fwf, t as f64 / fhf, (fw - r) as f64 / fwf, (fh - b) as f64 / fhf];
    let [a, bb, c, d] = sides.map(|v| round_even(v * 100000.0).max(0));
    format!("<a:srcRect l=\"{a}\" t=\"{bb}\" r=\"{c}\" b=\"{d}\"/>")
}

/// Preset or custom geometry; turn = (degrees, shape w, h, frame w, h) turns the points of a
/// polygon or curve clockwise about the frame's centre.
fn geometry(el: &Element, turn: Option<(i64, f64, f64, f64, f64)>) -> String {
    let tag = el.tag.as_str();
    if tag == "hp:rect" && num(el.get("ratio")) > 0 {
        return format!(
            "<a:prstGeom prst=\"roundRect\"><a:avLst><a:gd name=\"adj\" fmla=\"val {}\"/></a:avLst></a:prstGeom>",
            num(el.get("ratio")).min(50) * 1000
        );
    }
    let preset = match tag {
        "hp:rect" => Some("rect"),
        "hp:ellipse" => Some("ellipse"),
        "hp:arc" => Some("arc"),
        "hp:line" | "hp:connectLine" => Some("line"),
        _ => None,
    };
    if let Some(p) = preset {
        return format!("<a:prstGeom prst=\"{p}\"><a:avLst/></a:prstGeom>");
    }
    let org = el.find("hp:orgSz");
    let mut pw = org.map(|o| num(o.get("width")).max(1)).unwrap_or(1);
    let mut ph = org.map(|o| num(o.get("height")).max(1)).unwrap_or(1);
    let mut pts: Vec<(i64, i64)> = if tag == "hp:polygon" {
        el.findall("hc:pt").map(|p| (num(p.get("x")), num(p.get("y")))).collect()
    } else {
        // hp:curve: segments, drawn through their end points
        let segs: Vec<&Element> = el.findall("hp:seg").collect();
        let mut v: Vec<(i64, i64)> = segs.first().map(|s| (num(s.get("x1")), num(s.get("y1")))).into_iter().collect();
        v.extend(segs.iter().map(|s| (num(s.get("x2")), num(s.get("y2")))));
        v
    };
    let (Some(&p0), Some(&pn)) = (pts.first(), pts.last()) else { return "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>".into() };
    if pts.len() < 2 {
        return "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>".into();
    }
    let closed = tag == "hp:polygon" && p0 == pn;
    if let Some((deg, sw, sh, fw, fh)) = turn {
        let rad = (deg as f64).to_radians();
        let (c, s) = (rad.cos(), rad.sin());
        let (pwf, phf) = (pw as f64, ph as f64);
        pts = pts
            .iter()
            .map(|&(px, py)| {
                let dx = px as f64 * sw / pwf - sw / 2.0;
                let dy = py as f64 * sh / phf - sh / 2.0;
                (round_even(fw / 2.0 + dx * c - dy * s), round_even(fh / 2.0 + dx * s + dy * c))
            })
            .collect();
        pw = round_even(fw).max(1);
        ph = round_even(fh).max(1);
    }
    let mut path = String::new();
    for (i, (x, y)) in pts.iter().enumerate() {
        if i == 0 {
            path.push_str(&format!("<a:moveTo><a:pt x=\"{x}\" y=\"{y}\"/></a:moveTo>"));
        } else {
            path.push_str(&format!("<a:lnTo><a:pt x=\"{x}\" y=\"{y}\"/></a:lnTo>"));
        }
    }
    if closed {
        path.push_str("<a:close/>");
    }
    format!(
        "<a:custGeom><a:avLst/><a:gdLst/><a:rect l=\"0\" t=\"0\" r=\"r\" b=\"b\"/><a:pathLst><a:path w=\"{pw}\" h=\"{ph}\">{path}</a:path></a:pathLst></a:custGeom>"
    )
}

fn fill(el: &Element) -> String {
    let Some(fb) = el.find("hc:fillBrush") else { return "<a:noFill/>".into() };
    if let Some(c) = fb.find("hc:winBrush").and_then(|wb| color(wb.get("faceColor"))) {
        return format!("<a:solidFill><a:srgbClr val=\"{c}\"/></a:solidFill>");
    }
    if let Some(gr) = fb.find("hc:gradation") {
        let cols: Vec<String> = gr.findall("hc:color").filter_map(|c| color(c.get("value"))).collect();
        if cols.len() >= 2 {
            return gradient_fill(gr.get("type").unwrap_or("LINEAR"), num(gr.get("angle")), &cols);
        }
        if let Some(c) = cols.first() {
            return format!("<a:solidFill><a:srgbClr val=\"{c}\"/></a:solidFill>");
        }
    }
    "<a:noFill/>".into()
}

fn outline(el: &Element) -> String {
    let Some(ls) = el.find("hp:lineShape") else {
        return "<a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"000000\"/></a:solidFill></a:ln>".into();
    };
    let style = ls.get("style").unwrap_or("SOLID");
    if style == "NONE" {
        return "<a:ln><a:noFill/></a:ln>".into();
    }
    let w = emu(num(ls.get("width")) as f64).max(3175);
    let col = color(ls.get("color")).unwrap_or_else(|| "000000".into());
    let d = dash(style).map(|d| format!("<a:prstDash val=\"{d}\"/>")).unwrap_or_default();
    let head = arrow(ls.get("headStyle").unwrap_or("")).map(|a| format!("<a:headEnd type=\"{a}\"/>")).unwrap_or_default();
    let tail = arrow(ls.get("tailStyle").unwrap_or("")).map(|a| format!("<a:tailEnd type=\"{a}\"/>")).unwrap_or_default();
    format!("<a:ln w=\"{w}\"><a:solidFill><a:srgbClr val=\"{col}\"/></a:solidFill>{d}{head}{tail}</a:ln>")
}

impl Converter<'_> {
    /// Runs for a top-level hp:pic / shape / hp:container.
    pub(crate) fn drawing(&mut self, el: &Element, rpr: &str) -> Vec<String> {
        let place = self.placement(el);
        let (mut w, mut h) = size_of(el);
        if let Some(sz) = el.find("hp:sz")
            && angle_of(el) != 0
            && num(sz.get("width")) > 0
        {
            (w, h) = (num(sz.get("width")), num(sz.get("height"))); // the frame around the turned shape
        }
        let (w, h) = (w as f64, h as f64);
        match self.graphic(el, w, h) {
            Some(g) => vec![self.frame(&g, rpr, &place, w, h)],
            None => vec![],
        }
    }

    pub(crate) fn placement(&self, el: &Element) -> Placement {
        let pos = el.find("hp:pos");
        let om = el.find("hp:outMargin");
        let d = |k: &str| om.map(|o| emu(num(o.get(k)) as f64)).unwrap_or(0);
        let mut p = Placement {
            inline: pos.is_none_or(|p| truthy(p.get("treatAsChar"))),
            dist: [d("left"), d("right"), d("top"), d("bottom")],
            z: num(el.get("zOrder")).max(0),
            behind: false,
            h_from: "column".into(),
            v_from: "paragraph".into(),
            x: 0,
            y: 0,
            align_h: "LEFT".into(),
            align_v: "TOP".into(),
            wrap: "<wp:wrapTopAndBottom/>".into(),
        };
        let Some(pos) = pos else { return p };
        p.h_from = h_from(pos.get("horzRelTo").unwrap_or("COLUMN")).into();
        p.v_from = v_from(pos.get("vertRelTo").unwrap_or("PARA")).into();
        // 32-bit values written unsigned (4294966596 is -700)
        p.x = signed32(pos.get("horzOffset"));
        p.y = signed32(pos.get("vertOffset"));
        p.align_h = pos.get("horzAlign").unwrap_or("LEFT").into();
        p.align_v = pos.get("vertAlign").unwrap_or("TOP").into();
        let side = match el.get("textFlow").unwrap_or("") {
            "LEFT_ONLY" => "left",
            "RIGHT_ONLY" => "right",
            "LARGEST_ONLY" => "largest",
            _ => "bothSides",
        };
        p.wrap = match el.get("textWrap").unwrap_or("TOP_AND_BOTTOM") {
            "SQUARE" | "TIGHT" => format!("<wp:wrapSquare wrapText=\"{side}\"/>"),
            // "Through": text may fill the object's empty inside (frames drawn as outlines).
            "THROUGH" | "IN_FRONT_OF_TEXT" => "<wp:wrapNone/>".into(),
            "BEHIND_TEXT" => {
                p.behind = true;
                "<wp:wrapNone/>".into()
            }
            _ => "<wp:wrapTopAndBottom/>".into(),
        };
        p
    }

    /// HWPUNIT offsets from the reference edge, resolving the object's alignment.
    pub(crate) fn offsets(&self, p: &Placement, w: f64, h: f64) -> (i64, i64) {
        let (s, m) = (&self.sec, &self.sec.margin);
        let (page_w, page_h) = if s.landscape { (s.width.max(s.height), s.width.min(s.height)) } else { (s.width, s.height) };
        let body_w = page_w - m.left - m.right - m.gutter;
        let ref_w = match p.h_from.as_str() {
            "page" => page_w,
            // the current column, whose left edge the offset is written from
            "column" if self.columns.0 > 1 => {
                let (n, gap) = (self.columns.0, self.columns.1.max(0));
                ((body_w - gap * (n - 1)) / n).max(0)
            }
            _ => body_w,
        } as f64;
        let ref_h = match p.v_from.as_str() {
            "page" => page_h,
            "margin" => self.body_height(),
            _ => 0,
        } as f64;
        let (mut x, mut y) = (p.x as f64, p.y as f64);
        match p.align_h.as_str() {
            "CENTER" => x += (ref_w - w) / 2.0,
            "RIGHT" | "OUTSIDE" => x = ref_w - w - x,
            _ => {}
        }
        if ref_h != 0.0 {
            match p.align_v.as_str() {
                "CENTER" => y += (ref_h - h) / 2.0,
                "BOTTOM" | "OUTSIDE" => y = ref_h - h - y,
                _ => {}
            }
        }
        (trunc(x), trunc(y))
    }

    pub(crate) fn frame(&mut self, graphic: &str, rpr: &str, p: &Placement, w: f64, h: f64) -> String {
        self.pic_n += 1;
        let n = self.pic_n;
        // one side may be 0 (a level or upright line)
        let (w, h) = if w > 0.0 || h > 0.0 { (w.max(0.0), h.max(0.0)) } else { (1.0, 1.0) };
        let (cx, cy) = (emu(w), emu(h));
        let d = p.dist;
        let dists = format!("distT=\"{}\" distB=\"{}\" distL=\"{}\" distR=\"{}\"", d[2], d[3], d[0], d[1]);
        let doc_pr = format!("<wp:docPr id=\"{n}\" name=\"Object {n}\"/>");
        let body = if p.inline {
            format!(
                "<wp:inline {dists}><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>{doc_pr}<wp:cNvGraphicFramePr/>{graphic}</wp:inline>"
            )
        } else {
            let (x, y) = self.offsets(p, w, h);
            format!(
                "<wp:anchor {dists} simplePos=\"0\" relativeHeight=\"{}\" behindDoc=\"{}\" locked=\"0\" layoutInCell=\"1\" allowOverlap=\"1\"><wp:simplePos x=\"0\" y=\"0\"/><wp:positionH relativeFrom=\"{}\"><wp:posOffset>{}</wp:posOffset></wp:positionH><wp:positionV relativeFrom=\"{}\"><wp:posOffset>{}</wp:posOffset></wp:positionV><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>{}{doc_pr}<wp:cNvGraphicFramePr/>{graphic}</wp:anchor>",
                251_658_240i64.saturating_add(p.z),
                u8::from(p.behind),
                p.h_from,
                emu(x as f64),
                p.v_from,
                emu(y as f64),
                p.wrap
            )
        };
        format!("<w:r>{rpr}<w:drawing>{body}</w:drawing></w:r>")
    }

    fn graphic(&mut self, el: &Element, w: f64, h: f64) -> Option<String> {
        if el.tag == "hp:container" {
            // A group stays one object (wpg), so its members keep their places relative to
            // each other whether the group is inline or floating (WordCraft renders it).
            let members = self.members(el, 0);
            if members.is_empty() {
                return None;
            }
            let (ow, oh) = org_size(el, w, h);
            let ext = format!(
                "<a:off x=\"0\" y=\"0\"/><a:ext cx=\"{}\" cy=\"{}\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"{}\" cy=\"{}\"/>",
                emu(w),
                emu(h),
                emu(ow),
                emu(oh)
            );
            return Some(format!(
                "<a:graphic><a:graphicData uri=\"{WPG_URI}\"><wpg:wgp><wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm>{ext}</a:xfrm></wpg:grpSpPr>{members}</wpg:wgp></a:graphicData></a:graphic>"
            ));
        }
        let body = self.body(el, 0.0, 0.0, w, h, false)?;
        let uri = if el.tag == "hp:pic" { PIC_URI } else { WPS_URI };
        Some(format!("<a:graphic><a:graphicData uri=\"{uri}\">{body}</a:graphicData></a:graphic>"))
    }

    /// Members of a group, placed in the group's own coordinates (its original size); the
    /// group's child extent maps them onto its current size. Members' offsets and sizes are
    /// stored in those coordinates (a member can be larger than the group).
    fn members(&mut self, el: &Element, depth: usize) -> String {
        if depth > MAX_GROUP_DEPTH {
            self.missing("nesting too deep");
            return String::new();
        }
        let mut out = String::new();
        for c in el.children.iter().filter(|c| OBJECTS.contains(&c.tag.as_str())) {
            let off = c.find("hp:offset");
            let (dx, dy) = off.map(|o| (num(o.get("x")) as f64, num(o.get("y")) as f64)).unwrap_or((0.0, 0.0));
            let (cw, ch) = size_of(c);
            let (cw, ch) = (cw as f64, ch as f64);
            if c.tag == "hp:container" {
                let inner = self.members(c, depth + 1);
                if !inner.is_empty() {
                    self.pic_n += 1;
                    let (ow, oh) = org_size(c, cw, ch);
                    let xf = format!(
                        "<a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"{}\" cy=\"{}\"/>",
                        emu(dx),
                        emu(dy),
                        emu(cw),
                        emu(ch),
                        emu(ow),
                        emu(oh)
                    );
                    out.push_str(&format!(
                        "<wpg:grpSp><wpg:cNvPr id=\"{n}\" name=\"Group {n}\"/><wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm>{xf}</a:xfrm></wpg:grpSpPr>{inner}</wpg:grpSp>",
                        n = self.pic_n
                    ));
                }
                continue;
            }
            if let Some(body) = self.body(c, dx, dy, cw, ch, true) {
                out.push_str(&body);
            }
        }
        out
    }

    fn body(&mut self, el: &Element, x: f64, y: f64, w: f64, h: f64, grouped: bool) -> Option<String> {
        match el.tag.as_str() {
            "hp:pic" => {
                let (x, y, w, h) = unrotated(el, x, y, w, h, grouped);
                self.picture_body(el, x, y, w, h)
            }
            "hp:rect" | "hp:ellipse" | "hp:arc" | "hp:line" | "hp:polygon" | "hp:curve" | "hp:connectLine" => {
                Some(self.shape_body(el, x, y, w, h, grouped))
            }
            tag => {
                self.missing(tag);
                None
            }
        }
    }

    fn picture_body(&mut self, el: &Element, x: f64, y: f64, w: f64, h: f64) -> Option<String> {
        let img = el.find("hc:img").or_else(|| el.iter("hc:img").next());
        let got = img.and_then(|i| self.pkg.binary(i.get("binaryItemIDRef").unwrap_or("")));
        let Some((name, data)) = got else {
            self.missing("picture (missing image data)");
            return None;
        };
        let ext = ext_of(name);
        let media = self.out.add_media(name, data, &ext);
        let rid = self.add_rel("image", &media);
        self.stat("pictures");
        self.pic_n += 1;
        let line = if el.find("hp:lineShape").is_some() { outline(el) } else { String::new() };
        let angle = angle_of(el);
        let rot_attr = if angle != 0 { format!(" rot=\"{}\"", rot60k(angle)) } else { String::new() };
        let base = name.rsplit('/').next().unwrap_or(name);
        Some(format!(
            "<pic:pic><pic:nvPicPr><pic:cNvPr id=\"{}\" name={}/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{rid}\"/>{}<a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm{rot_attr}><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>{line}</pic:spPr></pic:pic>",
            self.pic_n,
            attr(base),
            crop(el, data),
            emu(x),
            emu(y),
            emu(w.max(1.0)),
            emu(h.max(1.0))
        ))
    }

    fn shape_body(&mut self, el: &Element, x: f64, y: f64, w: f64, h: f64, grouped: bool) -> String {
        let mut angle = angle_of(el);
        let (mut x, mut y, mut w, mut h) = (x, y, w, h);
        let mut turn = None;
        if angle != 0 && !grouped && matches!(el.tag.as_str(), "hp:polygon" | "hp:curve") {
            // Turn the points themselves inside the frame: WordCraft (0.10) does not draw a
            // turned shape whose box sticks out of its frame.
            let (_, _, uw, uh) = unrotated(el, x, y, w, h, grouped);
            turn = Some((angle, uw, uh, w, h));
            angle = 0;
        } else {
            (x, y, w, h) = unrotated(el, x, y, w, h, grouped);
        }
        let (cx, cy) = (emu(w.max(0.0)), emu(h.max(0.0)));
        let (mut flip_h, mut flip_v) = (false, false);
        let geom = geometry(el, turn);
        let is_line = matches!(el.tag.as_str(), "hp:line" | "hp:connectLine");
        if is_line {
            let s = el.find("hc:startPt").or_else(|| el.find("hp:startPt"));
            let e = el.find("hc:endPt").or_else(|| el.find("hp:endPt"));
            if let (Some(s), Some(e)) = (s, e) {
                flip_h = num(e.get("x")) < num(s.get("x"));
                flip_v = num(e.get("y")) < num(s.get("y"));
            }
        }
        let mut flip = format!("{}{}", if flip_h { " flipH=\"1\"" } else { "" }, if flip_v { " flipV=\"1\"" } else { "" });
        if let Some(fl) = el.find("hp:flip") {
            flip = format!(
                "{}{}",
                if truthy(fl.get("horizontal")) != flip_h { " flipH=\"1\"" } else { "" },
                if truthy(fl.get("vertical")) != flip_v { " flipV=\"1\"" } else { "" }
            );
        }
        let rot_attr = if angle != 0 { format!(" rot=\"{}\"", rot60k(angle)) } else { String::new() };
        let fill = if is_line { "<a:noFill/>".to_string() } else { fill(el) };
        let (mut body, mut text) = ("<wps:bodyPr/>".to_string(), String::new());
        let dt = el.find("hp:drawText");
        if let Some(sub) = dt.and_then(|d| d.find("hp:subList")) {
            self.stat("text boxes");
            let mut blocks = self.blocks(sub.findall("hp:p"), true);
            text = format!("<wps:txbx><w:txbxContent>{}</w:txbxContent></wps:txbx>", render_blocks(&mut blocks, true));
            let tm = dt.and_then(|d| d.find("hp:textMargin"));
            let ins = |k: &str| tm.map(|t| emu(num(t.get(k)) as f64)).unwrap_or(36000);
            body = format!(
                "<wps:bodyPr lIns=\"{}\" tIns=\"{}\" rIns=\"{}\" bIns=\"{}\" anchor=\"{}\" wrap=\"square\"><a:noAutofit/></wps:bodyPr>",
                ins("left"),
                ins("top"),
                ins("right"),
                ins("bottom"),
                anchor_v(sub.get("vertAlign").unwrap_or("TOP"))
            );
        } else {
            self.stat("shapes");
        }
        self.pic_n += 1;
        let name = if grouped { format!("<wps:cNvPr id=\"{n}\" name=\"Shape {n}\"/>", n = self.pic_n) } else { String::new() };
        let tb = if text.is_empty() { "" } else { " txBox=\"1\"" };
        format!(
            "<wps:wsp>{name}<wps:cNvSpPr{tb}/><wps:spPr><a:xfrm{rot_attr}{flip}><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>{geom}{fill}{}</wps:spPr>{text}{body}</wps:wsp>",
            emu(x),
            emu(y),
            outline(el)
        )
    }
}
