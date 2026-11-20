//! A table of contents in an ODT file (ODF 1.3 §8.3 `text:table-of-content`) opens as one
//! WordCraft updates: References › Update Table regenerates its entries from the document's
//! headings, with or without a title, and it is saved back as an ODF table of contents. The ODT
//! is original and synthetic, shaped like LibreOffice's.

use serde_json::json;
use wordcraft_engine::{Session, io};

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0""#;

/// A document with a table of contents whose entries are stale (made before the headings were
/// renamed), titled or not.
fn odt(titled: bool) -> Vec<u8> {
    let title = if titled {
        r#"<text:index-title text:name="T_Head"><text:p text:style-name="Contents_20_Heading">Contents</text:p></text:index-title>"#
    } else {
        ""
    };
    let content = format!(
        r#"<office:document-content {NS}><office:body><office:text>
        <text:table-of-content text:name="T"><text:table-of-content-source text:outline-level="2"/><text:index-body>{title}<text:p text:style-name="Contents_20_1">Old chapter<text:tab/>1</text:p></text:index-body></text:table-of-content>
        <text:h text:outline-level="1">Planning</text:h><text:p>Text.</text:p>
        <text:h text:outline-level="2">Budget</text:h><text:p>More text.</text:p>
        <text:h text:outline-level="3">Too deep</text:h><text:p>Last.</text:p>
        </office:text></office:body></office:document-content>"#
    );
    let mut z = craft_zip::Writer::new();
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content.as_str())] {
        z.add(name, data.as_bytes(), craft_zip::Compression::Stored).unwrap();
    }
    z.finish().unwrap()
}

/// The text of the body's paragraphs in the TOC styles (without an untitled table's field
/// paragraph, which the engine keeps where the table started).
fn entries(s: &Session) -> Vec<String> {
    s.doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .filter(|p| p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading"))
        .map(|p| p.text.split('\t').next().unwrap_or("").trim_matches(wordcraft_doc::para::OBJ).to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

#[test]
fn an_odt_table_of_contents_updates_from_the_headings() {
    for titled in [true, false] {
        let doc = io::open_bytes("toc.odt", &odt(titled)).unwrap();
        let mut s = Session::new(doc);
        assert_eq!(entries(&s), ["Old chapter"], "titled: {titled}");
        s.run("references.updateToc", &json!({})).unwrap();
        // Two levels, as the file's source says.
        assert_eq!(entries(&s), ["Planning", "Budget"], "titled: {titled}");
        // Saved and opened again, it is still a table of contents.
        let again = io::open_bytes("toc.odt", &io::save_bytes("toc.odt", &s.doc).unwrap()).unwrap();
        let mut s2 = Session::new(again);
        assert_eq!(entries(&s2), ["Planning", "Budget"], "titled: {titled}");
        s2.run("references.updateToc", &json!({})).unwrap();
        assert_eq!(entries(&s2), ["Planning", "Budget"], "titled: {titled}");
    }
}

#[test]
fn an_odt_index_updates_from_its_marks() {
    // An alphabetical index whose entries are index marks in the text (`XE` fields once open):
    // Update Index lists them, rather than emptying the index.
    let content = format!(
        r#"<office:document-content {NS}><office:body><office:text>
        <text:alphabetical-index text:name="A"><text:alphabetical-index-source/><text:index-body><text:p>stale, 9</text:p></text:index-body></text:alphabetical-index>
        <text:p>Clay <text:alphabetical-index-mark text:string-value="kilns"/>is fired; <text:alphabetical-index-mark-start text:id="m"/>glazes<text:alphabetical-index-mark-end text:id="m"/> shine.</text:p>
        </office:text></office:body></office:document-content>"#
    );
    let mut z = craft_zip::Writer::new();
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content.as_str())] {
        z.add(name, data.as_bytes(), craft_zip::Compression::Stored).unwrap();
    }
    let mut s = Session::new(io::open_bytes("index.odt", &z.finish().unwrap()).unwrap());
    s.run("references.updateIndex", &json!({})).unwrap();
    let text: Vec<String> = s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.replace(wordcraft_doc::para::OBJ, "")).collect();
    assert!(text.iter().any(|t| t.starts_with("glazes")), "{text:?}");
    assert!(text.iter().any(|t| t.starts_with("kilns")), "{text:?}");
    assert!(!text.iter().any(|t| t.contains("stale")), "{text:?}");
}

#[test]
fn an_odt_bibliography_updates_from_its_citations_sources() {
    // A bibliography and a citation carrying its source: Update lists the source.
    let content = format!(
        r#"<office:document-content {NS}><office:body><office:text>
        <text:p>Clay <text:bibliography-mark text:identifier="Smi20" text:bibliography-type="book" text:author="Smith, Jane" text:title="Kilns and Fire" text:year="2020">[Smi20]</text:bibliography-mark>.</text:p>
        <text:bibliography text:name="B"><text:bibliography-source/><text:index-body><text:p>stale</text:p></text:index-body></text:bibliography>
        </office:text></office:body></office:document-content>"#
    );
    let mut z = craft_zip::Writer::new();
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content.as_str())] {
        z.add(name, data.as_bytes(), craft_zip::Compression::Stored).unwrap();
    }
    let mut s = Session::new(io::open_bytes("bib.odt", &z.finish().unwrap()).unwrap());
    s.run("references.updateIndex", &json!({})).unwrap();
    let text: Vec<String> = s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.replace(wordcraft_doc::para::OBJ, "")).collect();
    assert!(text.iter().any(|t| t.contains("Kilns and Fire")), "{text:?}");
    assert!(!text.iter().any(|t| t.contains("stale")), "{text:?}");
}

/// #6486: LibreOffice numbers headings through its outline style, the number followed by a tab,
/// and its Contents styles carry only the right-aligned dotted stop for the page number. After
/// Update Table, the tab after an entry's number stops near the number, not at the page-number
/// stop: the title stays at the left and each entry stays on one line.
#[test]
fn an_odt_toc_with_numbered_headings_keeps_its_entries_on_one_line() {
    let outline = |level: u8, cm: f32| {
        format!(
            r#"<text:outline-level-style text:level="{level}" style:num-format="1" style:num-suffix="." text:display-levels="{level}"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" text:list-tab-stop-position="{cm}cm" fo:text-indent="-{cm}cm" fo:margin-left="{cm}cm"/></style:list-level-properties></text:outline-level-style>"#
        )
    };
    let contents = |n: u8, indent: f32| {
        format!(
            r#"<style:style style:name="Contents_20_{n}" style:display-name="Contents {n}" style:family="paragraph"><style:paragraph-properties fo:margin-left="{indent}cm"><style:tab-stops><style:tab-stop style:position="16.99cm" style:type="right" style:leader-style="dotted" style:leader-text="."/></style:tab-stops></style:paragraph-properties></style:style>"#
        )
    };
    let styles = format!(
        r#"<office:document-styles {NS}><office:styles>{}{}<text:outline-style style:name="Outline">{}{}</text:outline-style></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21.001cm" fo:page-height="29.7cm" fo:margin-left="2cm" fo:margin-right="2cm" fo:margin-top="2cm" fo:margin-bottom="2cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#,
        contents(1, 0.0),
        contents(2, 0.499),
        outline(1, 0.762),
        outline(2, 1.016)
    );
    let content = format!(
        r#"<office:document-content {NS}><office:body><office:text>
        <text:table-of-content text:name="T"><text:table-of-content-source text:outline-level="2"/><text:index-body><text:p text:style-name="Contents_20_1">1. Planning<text:tab/>1</text:p><text:p text:style-name="Contents_20_2">1.1. Budget<text:tab/>1</text:p></text:index-body></text:table-of-content>
        <text:h text:outline-level="1">Planning</text:h><text:p>Text.</text:p>
        <text:h text:outline-level="2">Budget</text:h><text:p>More text.</text:p>
        </office:text></office:body></office:document-content>"#
    );
    let mut z = craft_zip::Writer::new();
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("styles.xml", styles.as_str()), ("content.xml", content.as_str())]
    {
        z.add(name, data.as_bytes(), craft_zip::Compression::Stored).unwrap();
    }
    let mut s = Session::new(io::open_bytes("toc.odt", &z.finish().unwrap()).unwrap());
    s.run("references.updateToc", &json!({})).unwrap();
    let layout = s.layout();
    let mut checked = 0;
    for (i, b) in s.doc.body.iter().enumerate() {
        let Some(p) = b.as_para().filter(|p| p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading")) else {
            continue;
        };
        let Some(tab) = p.text.find('\t') else { continue };
        let pos = |off| wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(i), off };
        let title = layout.caret(&pos(tab + 1)).unwrap();
        let end = layout.caret(&pos(p.len())).unwrap();
        assert!(p.plain_text().contains("Planning") || p.plain_text().contains("Budget"), "{:?}", p.text);
        assert_eq!((title.page, title.top), (end.page, end.top), "{:?} wraps", p.text);
        assert!(title.x < 200.0, "{:?}: the title starts at {}", p.text, title.x);
        checked += 1;
    }
    assert_eq!(checked, 2);
}
