//! Shapes, text boxes, groups and canvases in a Strict document (#3548). ISO/IEC 29500 Strict
//! puts them in the one WordprocessingDrawing namespace, where Transitional documents use Word
//! 2010's `wps`, `wpg` and `wpc` namespaces. Fixtures are original and hand-written from
//! ECMA-376 Part 1, 5th edition (`dml-wordprocessingDrawing.xsd`, Strict and Transitional): each
//! drawing is written once and filled in with either set of namespaces.

use std::io::{Read, Write};

use wordcraft_doc::{Document, InlineObject, PartKind};

const STRICT_NS: &str = r#"xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main" xmlns:r="http://purl.oclc.org/ooxml/officeDocument/relationships" xmlns:wp="http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing" xmlns:a="http://purl.oclc.org/ooxml/drawingml/main" xmlns:wne="http://schemas.microsoft.com/office/word/2006/wordml" w:conformance="strict""#;
const TRANSITIONAL_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup" xmlns:wpc="http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas""#;
const STRICT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const TRANSITIONAL_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

/// How a fixture names its drawing elements: the shape, group and canvas prefixes, and the
/// text box content element.
struct Names {
    ns: &'static str,
    rels: &'static str,
    shape: &'static str,
    group: &'static str,
    canvas: &'static str,
    content: &'static str,
}

/// Word's Strict output: everything in `wp:`, text box content in `wne:txbxContent`.
const STRICT_WORD: Names = Names { ns: STRICT_NS, rels: STRICT_RELS, shape: "wp", group: "wp", canvas: "wp", content: "wne:txbxContent" };
/// The Strict schema's own names: the text box content in `wp:` too.
const STRICT_SCHEMA: Names = Names { ns: STRICT_NS, rels: STRICT_RELS, shape: "wp", group: "wp", canvas: "wp", content: "wp:txbxContent" };
/// The same drawings in a Transitional document.
const TRANSITIONAL: Names =
    Names { ns: TRANSITIONAL_NS, rels: TRANSITIONAL_RELS, shape: "wps", group: "wpg", canvas: "wpc", content: "w:txbxContent" };

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

fn solid(rgb: &str) -> String {
    format!(r#"<a:solidFill><a:srgbClr val="{rgb}"/></a:solidFill>"#)
}

/// A `wsp` at (`x`, `y`) EMUs in its group, `cx` × `cy`. `text` gives it a text box.
fn shape(n: &Names, prst: &str, (x, y, cx, cy): (u32, u32, u32, u32), fill: &str, text: Option<&str>) -> String {
    let s = n.shape;
    let connector = prst.contains("Connector");
    let nv = if connector {
        format!("<{s}:cNvCnPr/>")
    } else if text.is_some() {
        format!(r#"<{s}:cNvSpPr txBox="1"/>"#)
    } else {
        format!("<{s}:cNvSpPr/>")
    };
    let fill = if connector { String::new() } else { solid(fill) };
    let line =
        if connector { r#"<a:ln w="25400"><a:solidFill><a:srgbClr val="117A65"/></a:solidFill><a:tailEnd type="triangle"/></a:ln>"# } else { "" };
    let txbx = match text {
        Some(t) => {
            let c = n.content;
            format!(r#"<{s}:txbx><{c}><w:p><w:r><w:t>{t}</w:t></w:r></w:p></{c}></{s}:txbx>"#)
        }
        None => String::new(),
    };
    format!(
        r#"<{s}:wsp><{s}:cNvPr id="{cx}" name="{prst}"/>{nv}<{s}:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="{prst}"><a:avLst/></a:prstGeom>{fill}{line}</{s}:spPr>{txbx}<{s}:bodyPr anchor="ctr"/></{s}:wsp>"#
    )
}

/// Drawing `id`, anchored `x` EMUs from the page's left edge, 1 in × ½ in.
fn anchor(id: u32, x: u32, uri: &str, payload: &str) -> String {
    format!(
        r#"<w:r><w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="{id}" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="page"><wp:posOffset>{x}</wp:posOffset></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>914400</wp:posOffset></wp:positionV><wp:extent cx="914400" cy="457200"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/><wp:docPr id="{id}" name="Drawing {id}"/><wp:cNvGraphicFramePr/><a:graphic><a:graphicData uri="{uri}">{payload}</a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#
    )
}

/// A rectangle, a text box, an arrow, a group holding a nested group, and a canvas holding a
/// shape and a group, each 1 in × ½ in.
fn package(n: &Names) -> Vec<u8> {
    const WPS: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingShape";
    const WPG: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup";
    const WPC: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas";
    let full = (0, 0, 914_400, 457_200);
    let half = (0, 0, 457_200, 457_200);
    let g = n.group;
    let group_xfrm = |(x, y, cx, cy): (u32, u32, u32, u32)| {
        format!(
            r#"<{g}:cNvGrpSpPr/><{g}:grpSpPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/><a:chOff x="0" y="0"/><a:chExt cx="{cx}" cy="{cy}"/></a:xfrm>{}</{g}:grpSpPr>"#,
            solid("F39C12")
        )
    };
    let group = format!(
        "<{g}:wgp>{}{}<{g}:grpSp>{}{}</{g}:grpSp></{g}:wgp>",
        group_xfrm(full),
        shape(n, "ellipse", half, "28B463", None),
        group_xfrm((457_200, 0, 457_200, 457_200)),
        shape(n, "triangle", half, "8E44AD", None),
    );
    let c = n.canvas;
    let canvas = format!(
        r#"<{c}:wpc><{c}:bg>{}</{c}:bg><{c}:whole><a:ln w="12700">{}</a:ln></{c}:whole>{}<{g}:wgp>{}{}</{g}:wgp></{c}:wpc>"#,
        solid("FDF2E9"),
        solid("A04000"),
        shape(n, "roundRect", (45_720, 45_720, 365_760, 365_760), "16A085", None),
        group_xfrm((457_200, 45_720, 365_760, 365_760)),
        shape(n, "diamond", (0, 0, 365_760, 365_760), "2E86C1", None),
    );
    let drawings = [
        anchor(1, 914_400, WPS, &shape(n, "rect", full, "C0392B", None)),
        anchor(2, 2_286_000, WPS, &shape(n, "rect", full, "EAF2F8", Some("Strict text box"))),
        anchor(3, 3_657_600, WPS, &shape(n, "straightConnector1", full, "", None)),
        anchor(4, 5_029_200, WPG, &group),
        anchor(5, 6_400_800, WPC, &canvas),
    ];
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {}><w:body><w:p><w:r><w:t>Shapes</w:t></w:r>{}</w:p><w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr></w:body></w:document>"#,
        n.ns,
        drawings.concat()
    );
    zip(&[("_rels/.rels", n.rels.as_bytes()), ("word/document.xml", doc.as_bytes())])
}

fn objects(d: &Document) -> Vec<InlineObject> {
    d.body[0].as_para().unwrap().objects.clone()
}

/// The text of each text box story, in reading order.
fn box_texts(d: &Document) -> Vec<String> {
    let mut ids = Vec::new();
    d.objects_in_reading_order(&mut |o| ids.extend(o.text_boxes()));
    ids.iter()
        .filter_map(|id| d.parts.get(id).filter(|p| p.kind == PartKind::TextBox))
        .map(|p| p.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect::<Vec<_>>().join("\n"))
        .collect()
}

#[test]
fn strict_shapes_read_like_their_transitional_twins() {
    let transitional = wordcraft_docx::read(&package(&TRANSITIONAL)).unwrap();
    let want = objects(&transitional);
    // The twin itself reads every drawing: a rectangle, a text box, an arrow, a group whose
    // nested group is flattened into it, and a canvas with its background.
    assert_eq!(want.len(), 5, "{want:#?}");
    assert!(matches!(&want[0], InlineObject::Shape { story: None, freeform: None, .. }), "{:?}", want[0]);
    assert!(matches!(&want[1], InlineObject::Shape { story: Some(_), .. }), "{:?}", want[1]);
    assert!(matches!(&want[3], InlineObject::Group { children, canvas: None, .. } if children.len() == 2), "{:?}", want[3]);
    assert!(
        matches!(&want[4], InlineObject::Group { children, canvas: Some(c), .. } if children.len() == 2 && c.fill.is_some() && c.stroke.is_some()),
        "{:?}",
        want[4]
    );
    assert_eq!(box_texts(&transitional), ["Strict text box"]);
    for (name, names) in [("Word's Strict names", &STRICT_WORD), ("the Strict schema's names", &STRICT_SCHEMA)] {
        let strict = wordcraft_docx::read(&package(names)).unwrap();
        assert_eq!(objects(&strict), want, "{name}");
        assert_eq!(box_texts(&strict), ["Strict text box"], "{name}");
    }
}

#[test]
fn strict_shapes_are_saved_and_reopened() {
    let strict = wordcraft_docx::read(&package(&STRICT_WORD)).unwrap();
    let saved = wordcraft_docx::write(&strict).unwrap();
    let xml = document_xml(&saved);
    for name in ["<wps:wsp>", "<wpg:wgp>", "<wpc:wpc>", "<wpc:bg>", "<w:txbxContent>", "<wps:cNvCnPr"] {
        assert!(xml.contains(name), "{name} missing from the saved document");
    }
    let reopened = wordcraft_docx::read(&saved).unwrap();
    assert_eq!(objects(&reopened), objects(&strict));
    assert_eq!(box_texts(&reopened), ["Strict text box"]);
}
