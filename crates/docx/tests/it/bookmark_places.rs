//! Bookmark markers that sit between blocks or between a table's rows, and a picture's own
//! `a:blip` extensions, survive opening and saving where the file had them: a bookmark around a
//! table (Word ends it with a `w:bookmarkEnd` child of `w:tbl`) keeps both ends, a body-level
//! start or end stays between the blocks, and a picture's artistic effect keeps its image layer
//! part. Hand-written fixtures (from the spec), no files produced by Word.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};

use wordcraft_doc::{Block, Document, InlineObject};

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main""#;
const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const HDPHOTO_REL: &str = "http://schemas.microsoft.com/office/2007/relationships/hdphoto";

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        out.insert(f.name().to_string(), b);
    }
    out
}

fn document_xml(body: &str) -> String {
    format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W_NS}><w:body>{body}<w:sectPr/></w:body></w:document>"#)
}

fn docx(body: &str) -> Vec<u8> {
    let doc = document_xml(body);
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>"#;
    zip(&[("_rels/.rels", ROOT_RELS.as_bytes()), ("word/document.xml", doc.as_bytes()), ("word/_rels/document.xml.rels", rels.as_bytes())])
}

/// The body's structure: paragraphs, tables, rows, cells and bookmark markers (by name), in
/// order. Section properties and everything inside paragraphs but the markers are left out.
fn skeleton(xml: &str) -> Vec<String> {
    let body = &xml[xml.find("<w:body>").expect("body")..];
    let mut names: HashMap<String, String> = HashMap::new();
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let end = rest.find('>').unwrap();
        let tag = &rest[..end];
        rest = &rest[end + 1..];
        let (closing, empty) = (tag.starts_with('/'), tag.ends_with('/'));
        let tag = tag.trim_start_matches('/').trim_end_matches('/');
        let name = tag.split(|c: char| c.is_whitespace()).next().unwrap_or("");
        let attr = |a: &str| {
            let k = format!("{a}=\"");
            tag.find(&k).map(|i| tag[i + k.len()..].split('"').next().unwrap_or("").to_string())
        };
        match name {
            "w:p" | "w:tbl" | "w:tr" | "w:tc" => {
                let short = name.trim_start_matches("w:");
                if !closing {
                    out.push(short.to_string());
                }
                // An empty paragraph (`<w:p/>`) opens and closes.
                if closing || empty {
                    out.push(format!("/{short}"));
                }
            }
            "w:bookmarkStart" => {
                let (id, n) = (attr("w:id").unwrap(), attr("w:name").unwrap());
                names.insert(id, n.clone());
                let cols = match (attr("w:colFirst"), attr("w:colLast")) {
                    (Some(f), Some(l)) => format!("[{f}-{l}]"),
                    _ => String::new(),
                };
                out.push(format!("[{n}{cols}"));
            }
            "w:bookmarkEnd" => {
                let n = names.get(&attr("w:id").unwrap()).cloned().unwrap_or_else(|| "?".into());
                out.push(format!("{n}]"));
            }
            _ => {}
        }
    }
    out
}

fn written_skeleton(d: &Document) -> Vec<String> {
    let files = unzip(&wordcraft_docx::write(d).unwrap());
    skeleton(&String::from_utf8(files["word/document.xml"].clone()).unwrap())
}

/// Reading `body` and saving it gives the same structure, saving again too.
fn keeps_places(body: &str) {
    let want = skeleton(&document_xml(body));
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    assert_eq!(written_skeleton(&d), want, "first save");
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(written_skeleton(&again), want, "second save");
}

fn p(text: &str) -> String {
    format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>")
}

fn table(rows: &[&str]) -> String {
    let trs: String = rows.iter().map(|r| format!("<w:tr><w:tc><w:tcPr/>{}</w:tc><w:tc><w:tcPr/>{}</w:tc></w:tr>", p(r), p("x"))).collect();
    format!(r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="1440"/><w:gridCol w:w="1440"/></w:tblGrid>{trs}</w:tbl>"#)
}

/// A bookmark around a table the way Word writes one: the end is a child of `w:tbl` after the
/// last row. Before, the end was dropped and the bookmark lost its end.
#[test]
fn bookmark_end_child_of_table_is_kept_after_the_last_row() {
    let t = table(&["a", "b"]);
    let t = t.replacen("<w:r><w:t>a</w:t>", r#"<w:bookmarkStart w:id="3" w:name="Bookmark_BS"/><w:r><w:t>a</w:t>"#, 1);
    let t = t.replacen("</w:tbl>", r#"<w:bookmarkEnd w:id="3"/></w:tbl>"#, 1);
    keeps_places(&format!("{}{t}{}", p("before"), p("after")));
}

/// Body-level markers around a table: the start before it, the end between it and the next
/// paragraph. Before, the start went into the first cell and the end into the next paragraph.
#[test]
fn body_level_markers_around_a_table_stay_between_blocks() {
    let body =
        format!(r#"{}<w:bookmarkStart w:id="1" w:name="Around"/>{}<w:bookmarkEnd w:id="1"/>{}"#, p("before"), table(&["a", "b", "c"]), p("after"));
    keeps_places(&body);
}

/// Body-level markers between paragraphs, and a bookmark ending at the end of the story after
/// the last paragraph.
#[test]
fn body_level_markers_between_paragraphs_stay_there() {
    let body = format!(
        r#"{}<w:bookmarkStart w:id="1" w:name="Block"/>{}{}<w:bookmarkEnd w:id="1"/>{}<w:bookmarkStart w:id="2" w:name="Tail"/>{}<w:bookmarkEnd w:id="2"/>"#,
        p("one"),
        p("two"),
        p("three"),
        p("four"),
        p("five")
    );
    keeps_places(&body);
}

/// Markers that are children of a table between its rows: a start before a row, an end after
/// one (a bookmark covering the middle rows).
#[test]
fn markers_between_rows_stay_between_rows() {
    let t = table(&["r1", "r2", "r3", "r4"]);
    let mut it = t.split("<w:tr>");
    let head = it.next().unwrap();
    let rows: Vec<&str> = it.collect();
    let t = format!(
        r#"{head}<w:tr>{}<w:bookmarkStart w:id="9" w:name="Rows"/><w:tr>{}<w:tr>{}<w:bookmarkEnd w:id="9"/><w:tr>{}"#,
        rows[0], rows[1], rows[2], rows[3]
    );
    keeps_places(&format!("{}{t}{}", p("before"), p("after")));
}

/// A start before a table nested in a cell, and an end after it, inside the outer cell.
#[test]
fn markers_around_a_nested_table_stay_in_the_outer_cell() {
    let inner = table(&["i1", "i2"]);
    let cell = format!(r#"{}<w:bookmarkStart w:id="4" w:name="Inner"/>{inner}<w:bookmarkEnd w:id="4"/>{}"#, p("cell"), p("cell end"));
    let outer = format!(r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2880"/></w:tblGrid><w:tr><w:tc><w:tcPr/>{cell}</w:tc></w:tr></w:tbl>"#);
    keeps_places(&format!("{}{outer}{}", p("before"), p("after")));
}

/// A bookmark around a body-level table that starts the document and ends the story right after
/// it (#464, now written back between the blocks).
#[test]
fn bookmark_around_first_and_last_table() {
    let body = format!(r#"<w:bookmarkStart w:id="7" w:name="Whole"/>{}<w:bookmarkEnd w:id="7"/>"#, table(&["only"]));
    let want = skeleton(&document_xml(&body));
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    // A story ends with a paragraph: saving adds an empty one after the final table.
    let mut got = written_skeleton(&d);
    assert_eq!(got.split_off(got.len() - 2), ["p", "/p"]);
    assert_eq!(got, want);
}

/// A column bookmark keeps the table columns it covers.
#[test]
fn column_bookmark_keeps_its_columns() {
    let t = table(&["a", "b"]).replacen(
        "<w:r><w:t>a</w:t></w:r>",
        r#"<w:bookmarkStart w:id="5" w:name="Cols" w:colFirst="0" w:colLast="1"/><w:r><w:t>a</w:t></w:r><w:bookmarkEnd w:id="5"/>"#,
        1,
    );
    keeps_places(&format!("{}{t}{}", p("before"), p("after")));
}

/// After an edit moves a marker off its paragraph's edge, it is written inside the paragraph
/// (where it now is), once.
#[test]
fn edited_paragraph_keeps_its_marker_inside() {
    let body = format!(r#"{}<w:bookmarkStart w:id="1" w:name="Moved"/>{}<w:bookmarkEnd w:id="1"/>{}"#, p("one"), p("two"), p("three"));
    let mut d = wordcraft_docx::read(&docx(&body)).unwrap();
    let Block::Para(two) = std::sync::Arc::make_mut(&mut d.body[1]) else { panic!() };
    two.insert_text(0, "new ", &Default::default()).unwrap();
    let got = written_skeleton(&d);
    assert_eq!(got, ["p", "/p", "p", "[Moved", "/p", "Moved]", "p", "/p"]);
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(again.bookmarks().len(), 1);
}

/// The model is unchanged: the markers are in the paragraphs they border.
#[test]
fn markers_between_blocks_are_in_the_paragraph_they_border() {
    let t = table(&["a"]).replacen("</w:tbl>", r#"<w:bookmarkEnd w:id="3"/></w:tbl>"#, 1);
    let d = wordcraft_docx::read(&docx(&format!(r#"<w:bookmarkStart w:id="3" w:name="T"/>{}{t}{}"#, p("x"), p("y")))).unwrap();
    let Block::Para(first) = &*d.body[0] else { panic!() };
    assert_eq!(first.objects, [InlineObject::BookmarkStart { name: "T".into() }]);
    let Block::Table(t) = &*d.body[1] else { panic!() };
    let last = t.rows[0].cells[1].blocks[0].as_para().unwrap();
    assert_eq!(last.objects, [InlineObject::BookmarkEnd { name: "T".into() }]);
}

/// A picture with an artistic effect: its `a:blip` extension refers to an image layer part
/// (`.wdp`). Before, the extension and the part were dropped.
#[test]
fn picture_keeps_its_image_layer_part() {
    let png = {
        let img = image::RgbaImage::from_fn(2, 2, |x, y| image::Rgba([x as u8 * 100, y as u8 * 100, 50, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let wdp = b"II\xbc\x01 not a real HD Photo, carried as bytes".to_vec();
    let pic = |n: u32| {
        format!(
            r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="635000" cy="635000"/><wp:docPr id="{n}" name="Picture {n}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="p.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdImg"><a:extLst><a:ext uri="{{BEBA8EAE-BF5A-486C-A8C5-ECC9F3942E4B}}"><a14:imgProps><a14:imgLayer r:embed="rIdWdp"><a14:imgEffect><a14:saturation sat="0"/></a14:imgEffect></a14:imgLayer></a14:imgProps></a:ext><a:ext uri="{{28A0092B-C50C-407E-A947-70E740481C1C}}"><a14:useLocalDpi val="0"/></a:ext></a:extLst></a:blip><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="635000" cy="635000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
        )
    };
    let doc = document_xml(&format!("{}{}", pic(1), pic(2)));
    let rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdImg" Type="{REL}/image" Target="media/image1.png"/><Relationship Id="rIdWdp" Type="{HDPHOTO_REL}" Target="media/hdphoto1.wdp"/></Relationships>"#
    );
    let types = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="wdp" ContentType="image/vnd.ms-photo"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let bytes = zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/media/image1.png", &png),
        ("word/media/hdphoto1.wdp", &wdp),
    ]);
    let d = wordcraft_docx::read(&bytes).unwrap();
    for round in 0..2 {
        let out = if round == 0 {
            wordcraft_docx::write(&d).unwrap()
        } else {
            wordcraft_docx::write(&wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap()).unwrap()
        };
        let files = unzip(&out);
        let wdps: Vec<_> = files.iter().filter(|(k, _)| k.ends_with(".wdp")).collect();
        assert_eq!(wdps.len(), 1, "round {round}: one image layer part, shared by both pictures: {:?}", files.keys());
        assert_eq!(wdps[0].1, &wdp, "round {round}: the part's bytes as read");
        let wdp_name = wdps[0].0.trim_start_matches("word/").to_string();
        let doc = String::from_utf8(files["word/document.xml"].clone()).unwrap();
        let rels = String::from_utf8(files["word/_rels/document.xml.rels"].clone()).unwrap();
        assert_eq!(doc.matches("a14:imgLayer r:embed=").count(), 2, "round {round}: {doc}");
        assert_eq!(doc.matches("a14:useLocalDpi").count(), 2);
        let id = doc.split("a14:imgLayer r:embed=\"").nth(1).unwrap().split('"').next().unwrap();
        let rel = rels.split("<Relationship ").find(|r| r.contains(&format!("Id=\"{id}\""))).unwrap_or_else(|| panic!("no {id} in {rels}"));
        assert!(rel.contains(HDPHOTO_REL) && rel.contains(&wdp_name), "round {round}: {rel}");
        let types = String::from_utf8(files["[Content_Types].xml"].clone()).unwrap();
        assert!(types.contains("image/vnd.ms-photo"), "{types}");
    }
}
