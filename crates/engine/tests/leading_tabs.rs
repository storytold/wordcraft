//! Original DOCX fixtures for ordinary tabs that wrap with their following word (#2531).

use std::io::{Cursor, Write};

use wordcraft_doc::{Document, Pos};
use wordcraft_layout::{LayoutCache, LayoutOptions, layout};

fn input(text: &str, stops: &str) -> Vec<u8> {
    let props = r#"<w:rPr><w:rFonts w:ascii="Source Sans 3" w:hAnsi="Source Sans 3"/><w:sz w:val="28"/></w:rPr>"#;
    let xml = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
        <w:p><w:pPr><w:tabs>{stops}</w:tabs><w:ind w:firstLine="539"/><w:spacing w:before="0" w:after="0"/>{props}</w:pPr>
        <w:r>{props}<w:tab/></w:r><w:r>{props}<w:tab/></w:r><w:r>{props}<w:t>{text}</w:t></w:r></w:p>
        <w:p><w:pPr><w:spacing w:before="0" w:after="0"/>{props}</w:pPr><w:r>{props}<w:t>Following paragraph</w:t></w:r></w:p>
        <w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440"/></w:sectPr>
        </w:body></w:document>"#,
    );
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, value) in [("_rels/.rels", rels), ("word/document.xml", &xml)] {
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(value.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

const NEAR_EDGE: &str = r#"<w:tab w:val="center" w:pos="5018"/><w:tab w:val="left" w:pos="8625"/>"#;

fn positions(doc: &Document, text: &str) -> [wordcraft_geom::Point; 3] {
    let l = layout(doc, &mut LayoutCache::new(), &LayoutOptions::default());
    assert_eq!(l.pages.len(), 1);
    [Pos::body(0, 2), Pos::body(0, text.len() + 2), Pos::body(1, 0)].map(|pos| {
        let caret = l.caret(&pos).unwrap();
        wordcraft_geom::Point { x: caret.x, y: caret.top }
    })
}

#[test]
fn leading_left_tab_keeps_its_word_positioned_after_wrap_and_reopen() {
    // Word for Mac 16.114, Times New Roman 14 pt: the second tab and word wrap to a
    // line of their own, where the first (centre) stop centres the word. Test with
    // the bundled face instead: the centre is independent of glyph widths.
    let text = "Synthetic greeting";
    let mut doc = wordcraft_docx::read(&input(text, NEAR_EDGE)).unwrap();
    for _ in 0..3 {
        let para = doc.body[0].as_para().unwrap();
        assert_eq!(para.text, format!("\t\t{text}"));
        assert_eq!(para.props.tabs.as_ref().unwrap().len(), 2);
        assert!((para.props.indent_first.unwrap() - 26.95).abs() < 0.01);
        let [start, end, following] = positions(&doc, text);
        let center = (start.x + end.x) / 2.0;
        assert!((center - 322.9).abs() < 0.1, "greeting center {center}, start {start:?}, end {end:?}");
        assert!((following.x - 72.0).abs() < 0.01);
        assert!(following.y > start.y);
        doc = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    }
}

#[test]
fn leading_left_tabs_that_fit_keep_their_custom_stops() {
    let safe = r#"<w:tab w:val="left" w:pos="2160"/><w:tab w:val="left" w:pos="4320"/>"#;
    for (text, stops, expected) in [("Synthetic greeting", safe, 288.0), ("Hi", NEAR_EDGE, 503.25)] {
        let doc = wordcraft_docx::read(&input(text, stops)).unwrap();
        let [start, _, _] = positions(&doc, text);
        assert!((start.x - expected).abs() < 0.1, "{text:?}: {start:?}");
    }
}
