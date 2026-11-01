//! #1378: an RTF file with several sections converts to DOCX with one `w:sectPr` per section.
//! The RTF is original and synthetic, laid out the way word processors write sections: document
//! page defaults, then `\sectd` + section words + the section's header/footer groups, a `\sect`
//! after the last `\par` (or after a table), and a `\sectd` inside a footer's field result.

use std::io::Read;

use wordcraft_engine::io;

const RTF: &str = r"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0 Times New Roman;}}
\paperw11906\paperh16838\margl1134\margr1134\margt1417\margb1134
\sectd\headery708\footery708\colsx708
{\headerr \pard\plain Part one - upright\par}
{\footerr \pard\plain Pg. {\field{\*\fldinst PAGE}{\fldrslt 1}}\sectd\colsx708 {\par}}
\pard\plain Annual summary\par
\pard\plain Opening text of the first part.\par \sect }
\sectd\lndscpsxn\pgwsxn16838\pghsxn11906\pgnrestart\headery708\footery708\colsx708
{\headerr \pard\plain Part two - wide\par}
{\footerr \pard\plain Pg. {\field{\*\fldinst PAGE}{\fldrslt 1}}\sectd\lndscpsxn\pgwsxn16838\pghsxn11906 {\par}}
\pard\plain Volumes table\par
\trowd\cellx3000\cellx6000\pard\intbl A\cell B\cell\row
\pard {\sect }
\sectd\lndscpsxn\pgwsxn16838\pghsxn11906\sbknone\pgnrestart\headery708\footery708\cols2\colsx708\linebetcol
\pard\plain Column text one.\par
\pard\plain Column text two.\par \sect }
\sectd\pgnrestart\headery708\footery708\colsx708
\pard\plain Closing words\par
\pard\plain Last line.\par}";

fn document_xml(docx: &[u8]) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
    let mut xml = String::new();
    zip.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    xml
}

#[test]
fn rtf_sections_convert_to_one_docx_section_each() {
    let doc = io::open_bytes("sections.rtf", RTF.as_bytes()).unwrap();
    assert_eq!(doc.sections().len(), 4);
    let xml = document_xml(&io::save_bytes("sections.docx", &doc).unwrap());
    // One sectPr per section, in order.
    let secs: Vec<&str> = xml.split("<w:sectPr").skip(1).map(|s| s.split("</w:sectPr>").next().unwrap_or("")).collect();
    assert_eq!(secs.len(), 4, "{xml}");
    let orient: Vec<bool> = secs.iter().map(|s| s.contains(r#"w:orient="landscape""#)).collect();
    assert_eq!(orient, [false, true, true, false]);
    // A4 both ways.
    assert!(secs[0].contains(r#"w:w="11906""#) && secs[0].contains(r#"w:h="16838""#), "{}", secs[0]);
    assert!(secs[1].contains(r#"w:w="16838""#) && secs[1].contains(r#"w:h="11906""#), "{}", secs[1]);
    // Section 3: continuous, two columns with a line between.
    assert!(secs[2].contains(r#"<w:type w:val="continuous""#), "{}", secs[2]);
    assert!(secs[2].contains(r#"w:num="2""#), "{}", secs[2]);
    assert!(!secs[0].contains(r#"w:num="2""#) && !secs[3].contains(r#"w:num="2""#));
    // Numbering restarts at 1 in sections 2-4, not in 1.
    assert!(!secs[0].contains(r#"w:start="#), "{}", secs[0]);
    for s in &secs[1..] {
        assert!(s.contains("<w:pgNumType") && s.contains(r#"w:start="1""#), "{s}");
    }
    // Sections 1 and 2 have their own header and footer; 3 and 4 link to the previous.
    for s in &secs[..2] {
        assert!(s.contains("<w:headerReference") && s.contains("<w:footerReference"), "{s}");
    }
    for s in &secs[2..] {
        assert!(!s.contains("<w:headerReference") && !s.contains("<w:footerReference"), "{s}");
    }
}
