//! Picture crop, stroke, and DrawingML effect preservation around `craft-docx`.

use std::collections::{BTreeMap, HashSet};

use craft_xml::{El, Node};
use wordcraft_doc::effects::{Glow, Shadow, ShapeEffects};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::Rgb;
use wordcraft_doc::{Block, Blocks, Document, Part, PartKind};

const ZIP_LIMITS: craft_zip::Limits = craft_zip::Limits { max_entries: 10_000, max_total_bytes: 256 << 20 };
const INK_URI: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingInk";
const CHARTEX_URI: &str = "http://schemas.microsoft.com/office/drawing/2014/chartex";

#[derive(Clone, Debug, Default)]
struct PicExtra {
    crop: Option<[f32; 4]>,
    stroke: Option<Rgb>,
    stroke_width: f32,
    effects: ShapeEffects,
}

fn attr_i64(el: &El, key: &str) -> Option<i64> {
    el.attr_or_plain(key)?.trim().parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| v.round() as i64)
}

fn color_and_transparency(parent: &El, default_color: Rgb) -> (Rgb, f32) {
    let Some(clr) = parent.child("a:srgbClr") else {
        return (default_color, 0.0);
    };
    let color = clr.attr_or_plain("val").and_then(Rgb::parse).unwrap_or(default_color);
    let mut opacity = clr.child("a:alpha").and_then(|a| attr_i64(a, "val")).map_or(100.0, |v| v.clamp(0, 100_000) as f32 / 1000.0);
    if let Some(m) = clr.child("a:alphaMod").and_then(|a| attr_i64(a, "val")) {
        opacity *= m.max(0) as f32 / 100_000.0;
    }
    if let Some(o) = clr.child("a:alphaOff").and_then(|a| attr_i64(a, "val")) {
        opacity += o as f32 / 1000.0;
    }
    (color, 100.0 - opacity.clamp(0.0, 100.0))
}

fn parse_pic_extra_el(pic: &El) -> PicExtra {
    let mut extra = PicExtra::default();
    if let Some(sr) = pic.find("a:srcRect") {
        let mut crop = [0.0f32; 4];
        for (i, k) in ["l", "t", "r", "b"].into_iter().enumerate() {
            if let Some(v) = attr_i64(sr, k)
                && let Some(slot) = crop.get_mut(i)
            {
                *slot = (v.clamp(-100_000_000, 100_000_000) as f32 / 100_000.0).clamp(-1000.0, 1000.0);
            }
        }
        extra.crop = Some(crop);
    }
    if let Some(sppr) = pic.find("pic:spPr").or_else(|| pic.find("wps:spPr")) {
        if let Some(ln) = sppr.child("a:ln") {
            if let Some(w) = attr_i64(ln, "w") {
                extra.stroke_width = (w.max(0) as f32 / 12_700.0).clamp(0.0, 14_400.0);
            }
            extra.stroke = ln.find("a:srgbClr").and_then(|c| c.attr_or_plain("val")).and_then(Rgb::parse);
        }
        if let Some(shdw) = sppr.find("a:outerShdw") {
            let blur = attr_i64(shdw, "blurRad").map_or(0.0, |v| v.max(0) as f32 / 12_700.0);
            let distance = attr_i64(shdw, "dist").map_or(0.0, |v| v.max(0) as f32 / 12_700.0);
            let angle = attr_i64(shdw, "dir").map_or(0.0, |v| (v.rem_euclid(21_600_000) as f32) / 60_000.0);
            let rot_with_shape = !matches!(shdw.attr_or_plain("rotWithShape"), Some("0" | "false"));
            let (color, transparency) = color_and_transparency(shdw, Rgb::BLACK);
            extra.effects.shadow = Some(Shadow { color, transparency, blur, distance, angle, rot_with_shape });
        }
        if let Some(glow) = sppr.find("a:glow") {
            let size = attr_i64(glow, "rad").map_or(0.0, |v| v.max(0) as f32 / 12_700.0);
            let (color, transparency) = color_and_transparency(glow, Glow::default().color);
            extra.effects.glow = Some(Glow { color, size, transparency });
        }
        if let Some(se) = sppr.find("a:softEdge")
            && let Some(r) = attr_i64(se, "rad")
        {
            extra.effects.soft_edge = Some((r.max(0) as f32 / 12_700.0).clamp(0.0, 14_400.0));
        }
        extra.effects = extra.effects.sanitized();
    }
    extra
}

fn children_of_choice(ac: &El) -> Option<&El> {
    const KNOWN: &[&str] = &["wps", "wpg", "wpc", "wpi", "w14", "w15", "wp14", "a14", "w16se", "w16cid", "w16", "w16cex", "w16sdtdh", "v", "m"];
    let ok = |c: &El| c.attr("Requires").unwrap_or("").split_whitespace().all(|r| KNOWN.contains(&r));
    ac.children("mc:Choice").find(|c| ok(c)).or_else(|| ac.child("mc:Fallback")).or_else(|| ac.child("mc:Choice"))
}

fn resolve_word_target(target: &str) -> String {
    let t = target.trim();
    if let Some(stripped) = t.strip_prefix('/') {
        stripped.to_string()
    } else if let Some(rest) = t.strip_prefix("../") {
        rest.to_string()
    } else {
        format!("word/{}", t.trim_start_matches("./"))
    }
}

fn embedded_image_rids(archive: &mut craft_zip::Archive<'_>) -> HashSet<String> {
    let mut valid = HashSet::new();
    let Some(rels_bytes) = archive.read("word/_rels/document.xml.rels", craft_docx::MAX_XML_FILE as u64).ok().flatten() else {
        return valid;
    };
    let Ok(root) = craft_xml::parse(&rels_bytes) else {
        return valid;
    };
    for rel in root.elements() {
        if rel.local() != "Relationship" {
            continue;
        }
        if rel.attr_or_plain("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External")) {
            continue;
        }
        let Some(id) = rel.attr_or_plain("Id") else { continue };
        let Some(target) = rel.attr_or_plain("Target") else { continue };
        let part = resolve_word_target(target);
        if archive.is_stored(&part).is_some() {
            valid.insert(id.to_string());
        }
    }
    valid
}

fn collect_vml_shape_images(shape: &El, valid_rids: &HashSet<String>, out: &mut Vec<PicExtra>) {
    if shape.local() == "group" {
        for ch in shape.elements() {
            if ch.name.starts_with("v:") && ch.local() != "shapetype" {
                collect_vml_shape_images(ch, valid_rids, out);
            }
        }
        return;
    }
    if let Some(img) = shape.find("v:imagedata") {
        if let Some(id) = img.attr("r:id").or_else(|| img.attr("r:pict"))
            && valid_rids.contains(id)
        {
            out.push(PicExtra::default());
        }
        return;
    }
    if let Some(txbx) = shape.find("w:txbxContent") {
        collect_xml_pic_extras(txbx, valid_rids, out);
    }
}

fn collect_xml_pic_extras(el: &El, valid_rids: &HashSet<String>, out: &mut Vec<PicExtra>) {
    match el.name.as_str() {
        "mc:AlternateContent" => {
            if let Some(choice) = children_of_choice(el) {
                let is_ink_or_chartex = choice
                    .find("a:graphicData")
                    .and_then(|gd| gd.attr("uri"))
                    .is_some_and(|u| u == INK_URI || u == CHARTEX_URI);
                if !is_ink_or_chartex {
                    collect_xml_pic_extras(choice, valid_rids, out);
                }
            }
        }
        "a:graphicData" => {
            let uri = el.attr("uri").unwrap_or("");
            if uri == INK_URI || uri.ends_with("/chart") || uri.ends_with("/diagram") {
                return;
            }
            let direct_shape = el.child("wps:wsp").filter(|shape| {
                shape.child("wps:spPr").and_then(|props| props.child("a:blipFill")).is_none()
            });
            if el.child("wpc:wpc").is_none()
                && el.child("wpg:wgp").is_none()
                && direct_shape.is_none()
                && let Some(blip) = el.find("a:blip")
            {
                if let Some(embed) = blip.attr("r:embed")
                    && valid_rids.contains(embed)
                {
                    out.push(parse_pic_extra_el(el));
                }
                return;
            }
            for ch in el.elements() {
                collect_xml_pic_extras(ch, valid_rids, out);
            }
        }
        "pic:pic" => {
            if let Some(embed) = el.find("a:blip").and_then(|b| b.attr("r:embed"))
                && valid_rids.contains(embed)
            {
                out.push(parse_pic_extra_el(el));
            }
        }
        "w:pict" => {
            for ch in el.elements() {
                if ch.name.starts_with("v:") && ch.local() != "shapetype" {
                    collect_vml_shape_images(ch, valid_rids, out);
                }
            }
        }
        "w:object" => {
            if let Some(ch) = el.elements().find(|c| c.name.starts_with("v:") && c.local() != "shapetype") {
                collect_vml_shape_images(ch, valid_rids, out);
            }
        }
        _ => {
            for ch in el.elements() {
                collect_xml_pic_extras(ch, valid_rids, out);
            }
        }
    }
}

fn apply_extras_to_blocks(
    blocks: &mut Blocks,
    parts: &mut BTreeMap<u32, Part>,
    visited: &mut HashSet<u32>,
    iter: &mut std::slice::Iter<'_, PicExtra>,
) {
    for block in blocks.iter_mut() {
        match std::sync::Arc::make_mut(block) {
            Block::Para(p) => {
                for obj in &mut p.objects {
                    apply_extras_to_obj(obj, parts, visited, iter);
                }
            }
            Block::Table(t) => {
                for r in &mut t.rows {
                    for c in &mut r.cells {
                        apply_extras_to_blocks(&mut c.blocks, parts, visited, iter);
                    }
                }
            }
        }
    }
}

fn apply_extras_to_obj(
    obj: &mut InlineObject,
    parts: &mut BTreeMap<u32, Part>,
    visited: &mut HashSet<u32>,
    iter: &mut std::slice::Iter<'_, PicExtra>,
) {
    match obj {
        InlineObject::Image { crop, float, .. } => {
            if let Some(ex) = iter.next() {
                if let Some(c) = ex.crop {
                    *crop = c;
                }
                if ex.stroke.is_some() {
                    float.pic_stroke = ex.stroke;
                    float.pic_stroke_width = ex.stroke_width;
                }
                if !ex.effects.is_empty() {
                    float.pic_effects = ex.effects;
                }
            }
        }
        InlineObject::Shape { story: Some(id), .. } => {
            let sid = *id;
            if visited.insert(sid)
                && let Some(mut part) = parts.remove(&sid)
            {
                if part.kind == PartKind::TextBox {
                    apply_extras_to_blocks(&mut part.blocks, parts, visited, iter);
                }
                parts.insert(sid, part);
            }
        }
        InlineObject::Group { children, .. } => {
            for ch in children {
                apply_extras_to_obj(&mut ch.obj, parts, visited, iter);
            }
        }
        _ => {}
    }
}

pub(crate) fn apply_read_extras(bytes: &[u8], doc: &mut Document) {
    let Ok(mut archive) = craft_zip::Archive::open(bytes, ZIP_LIMITS) else {
        return;
    };
    let Some(doc_xml) = archive.read("word/document.xml", craft_docx::MAX_XML_FILE as u64).ok().flatten() else {
        return;
    };
    let Ok(s) = std::str::from_utf8(&doc_xml) else {
        return;
    };
    if !s.contains("pic:pic") && !s.contains("a:blipFill") {
        return;
    }
    let Ok(root) = craft_xml::parse(&doc_xml) else {
        return;
    };
    let valid_rids = embedded_image_rids(&mut archive);
    let mut extras = Vec::new();
    let body = root.child("w:body").unwrap_or(&root);
    collect_xml_pic_extras(body, &valid_rids, &mut extras);
    if extras.iter().any(|e| e.crop.is_some() || e.stroke.is_some() || !e.effects.is_empty()) {
        let mut iter = extras.iter();
        let mut visited = HashSet::new();
        apply_extras_to_blocks(&mut doc.body, &mut doc.parts, &mut visited, &mut iter);
    }
}

fn collect_doc_images(
    doc: &Document,
    blocks: &Blocks,
    visited: &mut HashSet<u32>,
    out: &mut Vec<PicExtra>,
) {
    for block in blocks.iter() {
        match block.as_ref() {
            Block::Para(p) => {
                for obj in &p.objects {
                    collect_obj_images(doc, obj, visited, out);
                }
            }
            Block::Table(t) => {
                for r in &t.rows {
                    for c in &r.cells {
                        collect_doc_images(doc, &c.blocks, visited, out);
                    }
                }
            }
        }
    }
}

fn collect_obj_images(
    doc: &Document,
    obj: &InlineObject,
    visited: &mut HashSet<u32>,
    out: &mut Vec<PicExtra>,
) {
    match obj {
        InlineObject::Image { media, crop, float, ole, .. } => {
            if ole.as_deref().is_some_and(|e| !e.is_odf_object()) {
                return;
            }
            if !doc.media.contains_key(media) {
                return;
            }
            out.push(PicExtra {
                crop: crop.iter().any(|&v| v != 0.0).then_some(*crop),
                stroke: float.pic_stroke,
                stroke_width: float.pic_stroke_width,
                effects: float.pic_effects,
            });
        }
        InlineObject::Shape { story: Some(id), .. } => {
            if visited.insert(*id)
                && let Some(part) = doc.parts.get(id)
                && part.kind == PartKind::TextBox
            {
                collect_doc_images(doc, &part.blocks, visited, out);
            }
        }
        InlineObject::Group { children, .. } => {
            for ch in children {
                collect_obj_images(doc, &ch.obj, visited, out);
            }
        }
        _ => {}
    }
}

fn build_sppr_extra_els(ex: &PicExtra) -> Vec<Node> {
    let mut nodes = Vec::new();
    if let Some(c) = ex.stroke {
        let w = wordcraft_geom::to_emu(ex.stroke_width.clamp(0.0, 100.0)).to_string();
        nodes.push(Node::El(El {
            name: "a:ln".into(),
            attrs: vec![("w".into(), w)],
            children: vec![Node::El(El {
                name: "a:solidFill".into(),
                attrs: Vec::new(),
                children: vec![Node::El(El {
                    name: "a:srgbClr".into(),
                    attrs: vec![("val".into(), c.hex())],
                    children: Vec::new(),
                })],
            })],
        }));
    }
    let fx = &ex.effects;
    if !fx.is_empty() {
        let mut fx_children = Vec::new();
        if let Some(sh) = &fx.shadow {
            let blur = wordcraft_geom::to_emu(sh.blur.max(0.0)).to_string();
            let dist = wordcraft_geom::to_emu(sh.distance.max(0.0)).to_string();
            let dir = ((sh.angle.rem_euclid(360.0) * 60_000.0).round() as i64).clamp(0, 21_599_999).to_string();
            let rot = if sh.rot_with_shape { "1" } else { "0" };
            let alpha = (((100.0 - sh.transparency.clamp(0.0, 100.0)) * 1000.0).round() as i64).clamp(0, 100_000).to_string();
            fx_children.push(Node::El(El {
                name: "a:outerShdw".into(),
                attrs: vec![
                    ("blurRad".into(), blur),
                    ("dist".into(), dist),
                    ("dir".into(), dir),
                    ("rotWithShape".into(), rot.into()),
                ],
                children: vec![Node::El(El {
                    name: "a:srgbClr".into(),
                    attrs: vec![("val".into(), sh.color.hex())],
                    children: vec![Node::El(El {
                        name: "a:alpha".into(),
                        attrs: vec![("val".into(), alpha)],
                        children: Vec::new(),
                    })],
                })],
            }));
        }
        if let Some(gl) = &fx.glow {
            let rad = wordcraft_geom::to_emu(gl.size.max(0.0)).to_string();
            let alpha = (((100.0 - gl.transparency.clamp(0.0, 100.0)) * 1000.0).round() as i64).clamp(0, 100_000).to_string();
            fx_children.push(Node::El(El {
                name: "a:glow".into(),
                attrs: vec![("rad".into(), rad)],
                children: vec![Node::El(El {
                    name: "a:srgbClr".into(),
                    attrs: vec![("val".into(), gl.color.hex())],
                    children: vec![Node::El(El {
                        name: "a:alpha".into(),
                        attrs: vec![("val".into(), alpha)],
                        children: Vec::new(),
                    })],
                })],
            }));
        }
        if let Some(se) = fx.soft_edge {
            let rad = wordcraft_geom::to_emu(se.max(0.0)).to_string();
            fx_children.push(Node::El(El {
                name: "a:softEdge".into(),
                attrs: vec![("rad".into(), rad)],
                children: Vec::new(),
            }));
        }
        nodes.push(Node::El(El {
            name: "a:effectLst".into(),
            attrs: Vec::new(),
            children: fx_children,
        }));
    }
    nodes
}

fn patch_pic_el(el: &mut El, extras: &[PicExtra], pic_idx: &mut usize) -> bool {
    if el.name == "mc:Fallback" {
        return false;
    }
    if el.name == "pic:pic" {
        let idx = *pic_idx;
        *pic_idx += 1;
        let Some(ex) = extras.get(idx) else {
            return false;
        };
        let mut changed = false;
        if let Some(crop) = ex.crop
            && let Some(bf) = el.child_mut("pic:blipFill")
        {
            let fmt = |v: f32| ((wordcraft_geom::finite(v).clamp(-1000.0, 1000.0) * 100_000.0).round() as i64).to_string();
            let attrs = vec![
                ("l".into(), fmt(crop[0])),
                ("t".into(), fmt(crop[1])),
                ("r".into(), fmt(crop[2])),
                ("b".into(), fmt(crop[3])),
            ];
            if let Some(sr) = bf.child_mut("a:srcRect") {
                if sr.attrs != attrs {
                    sr.attrs = attrs;
                    changed = true;
                }
            } else {
                let insert_pos = bf
                    .children
                    .iter()
                    .position(|n| matches!(n, Node::El(c) if c.name == "a:stretch"))
                    .unwrap_or(bf.children.len());
                bf.children.insert(
                    insert_pos,
                    Node::El(El { name: "a:srcRect".into(), attrs, children: Vec::new() }),
                );
                changed = true;
            }
        }
        let sppr_nodes = build_sppr_extra_els(ex);
        if !sppr_nodes.is_empty()
            && let Some(sppr) = el.child_mut("pic:spPr")
        {
            sppr.children.extend(sppr_nodes);
            changed = true;
        }
        return changed;
    }
    let mut changed = false;
    for ch in el.elements_mut() {
        if patch_pic_el(ch, extras, pic_idx) {
            changed = true;
        }
    }
    changed
}

fn rewrite_pic_xml(data: &[u8], extras: &[PicExtra]) -> Option<Vec<u8>> {
    if !extras.iter().any(|e| e.crop.is_some() || e.stroke.is_some() || !e.effects.is_empty()) {
        return None;
    }
    let mut root = craft_xml::parse(data).ok()?;
    let mut pic_idx = 0usize;
    if !patch_pic_el(&mut root, extras, &mut pic_idx) {
        return None;
    }
    let mut out = String::with_capacity(data.len() + 256);
    out.push_str(craft_xml::DECL_STANDALONE);
    root.write_xml(&mut out);
    Some(out.into_bytes())
}

pub(crate) fn apply_write_extras(doc: &Document, bytes: Vec<u8>) -> Vec<u8> {
    let mut extras = Vec::new();
    let mut visited = HashSet::new();
    collect_doc_images(doc, &doc.body, &mut visited, &mut extras);
    if !extras.iter().any(|e| e.crop.is_some() || e.stroke.is_some() || !e.effects.is_empty()) {
        return bytes;
    }
    let Ok(mut archive) = craft_zip::Archive::open(&bytes, ZIP_LIMITS) else {
        return bytes;
    };
    let mut entries = Vec::with_capacity(archive.len());
    let mut changed = false;
    for idx in 0..archive.len() {
        let Some(name) = archive.entry_name(idx).ok().flatten() else {
            return bytes;
        };
        if archive.entry_is_dir(idx).ok().flatten().unwrap_or(false) {
            continue;
        }
        let stored = archive.is_stored(&name).unwrap_or(false);
        let Some(mut data) = archive.read(&name, craft_docx::MAX_XML_FILE as u64).ok().flatten() else {
            return bytes;
        };
        if name == "word/document.xml"
            && let Some(rewritten) = rewrite_pic_xml(&data, &extras)
        {
            data = rewritten;
            changed = true;
        }
        let comp = if stored { craft_zip::Compression::Stored } else { craft_zip::Compression::Deflated };
        entries.push((name, data, comp));
    }
    if !changed {
        return bytes;
    }
    let mut out = craft_zip::Writer::new();
    for (name, data, comp) in entries {
        if out.add(&name, &data, comp).is_err() {
            return bytes;
        }
    }
    out.finish().unwrap_or(bytes)
}
