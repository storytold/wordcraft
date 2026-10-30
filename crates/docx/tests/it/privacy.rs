//! Privacy settings (#1062): `w:removePersonalInformation` and `w:removeDateAndTime`
//! (ECMA-376 Part 1 §17.15.1.69–70) are kept on save and honoured by the writer. Synthetic
//! fixtures written for these tests from the spec; the names are made up.
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const RT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";

fn zip(entries: &[(&str, String)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in entries {
        z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(text.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn unzip(bytes: &[u8]) -> BTreeMap<String, String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut parts = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut s = String::new();
        if f.read_to_string(&mut s).is_ok() {
            parts.insert(f.name().unwrap().to_string(), s);
        }
    }
    parts
}

/// A package with two named commenters, a tracked insertion and deletion, the document
/// properties' creator and last-modified-by, and `settings` inside `w:settings`.
fn fixture(settings: &str) -> Vec<u8> {
    let body = r#"<w:p><w:commentRangeStart w:id="1"/><w:r><w:t>Clause one.</w:t></w:r><w:commentRangeEnd w:id="1"/><w:r><w:commentReference w:id="1"/></w:r></w:p>
<w:p><w:commentRangeStart w:id="2"/><w:ins w:id="10" w:author="Quill Ramsey" w:date="2026-03-04T05:06:07Z"><w:r><w:t>Added</w:t></w:r></w:ins><w:del w:id="11" w:author="Ona Vettel" w:date="2026-03-05T05:06:07Z"><w:r><w:delText>Removed</w:delText></w:r></w:del><w:commentRangeEnd w:id="2"/><w:r><w:commentReference w:id="2"/></w:r></w:p>"#;
    let comments = format!(
        r#"<w:comments {W_NS}><w:comment w:id="1" w:author="Quill Ramsey" w:initials="QR" w:date="2026-03-01T10:00:00Z"><w:p><w:r><w:t>Check this.</w:t></w:r></w:p></w:comment><w:comment w:id="2" w:author="Ona Vettel" w:initials="OV" w:date="2026-03-02T10:00:00Z"><w:p><w:r><w:t>Agreed.</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Draft terms</dc:title><dc:creator>Quill Ramsey</dc:creator><cp:lastModifiedBy>Ona Vettel</cp:lastModifiedBy></cp:coreProperties>"#;
    let root_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RT}officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#
    );
    let doc_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RT}comments" Target="comments.xml"/><Relationship Id="rId2" Type="{RT}settings" Target="settings.xml"/></Relationships>"#
    );
    zip(&[
        ("_rels/.rels", root_rels),
        ("word/document.xml", format!(r#"<w:document {W_NS}><w:body>{body}</w:body></w:document>"#)),
        ("word/_rels/document.xml.rels", doc_rels),
        ("word/comments.xml", comments),
        ("word/settings.xml", format!(r#"<w:settings {W_NS}>{settings}</w:settings>"#)),
        ("docProps/core.xml", core.to_string()),
    ])
}

const NAMES: [&str; 4] = ["Quill Ramsey", "Ona Vettel", "\"QR\"", "\"OV\""];

/// Every part of the saved package, joined.
fn saved(doc: &wordcraft_doc::Document) -> (BTreeMap<String, String>, String) {
    let parts = unzip(&wordcraft_docx::write(doc).unwrap());
    let all = parts.values().cloned().collect::<Vec<_>>().join("\n");
    (parts, all)
}

#[test]
fn remove_personal_information_and_dates_on_save() {
    let doc = wordcraft_docx::read(&fixture("<w:removePersonalInformation/><w:removeDateAndTime/>")).unwrap();
    assert!(doc.settings.remove_personal_info && doc.settings.remove_date_time);
    let before = doc.clone();
    let (parts, all) = saved(&doc);
    // The open document keeps its real names and dates.
    assert_eq!(doc, before);
    assert_eq!(doc.core.creator, "Quill Ramsey");
    assert!(doc.comments.values().any(|c| c.author == "Ona Vettel" && !c.date.is_empty()));
    // No name, initials or annotation date anywhere in the file.
    for name in NAMES {
        assert!(!all.contains(name), "{name} saved");
    }
    assert!(!all.contains("w:date="), "annotation date saved");
    assert!(!all.contains("2026-03-0"), "annotation date saved");
    let comments = &parts["word/comments.xml"];
    assert_eq!(comments.matches(r#"w:author="Author""#).count(), 2, "{comments}");
    assert_eq!(comments.matches(r#"w:initials="A""#).count(), 2, "{comments}");
    let body = &parts["word/document.xml"];
    assert!(body.contains("<w:ins ") && body.contains("<w:del "));
    assert_eq!(body.matches(r#"w:author="Author""#).count(), 2, "{body}");
    let core = &parts["docProps/core.xml"];
    assert!(!core.contains("dc:creator") && !core.contains("cp:lastModifiedBy"), "{core}");
    assert!(core.contains("Draft terms"), "title kept: {core}");
    // Both settings are written back (in CT_Settings order, after zoom), so Word keeps honouring them.
    let settings = &parts["word/settings.xml"];
    let zoom = settings.find("w:zoom").unwrap();
    let personal = settings.find("<w:removePersonalInformation/>").unwrap();
    let dates = settings.find("<w:removeDateAndTime/>").unwrap();
    assert!(zoom < personal && personal < dates, "{settings}");
    let again = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    assert!(again.settings.remove_personal_info && again.settings.remove_date_time);
}

#[test]
fn each_privacy_setting_works_alone() {
    // Dates only: names stay.
    let doc = wordcraft_docx::read(&fixture("<w:removeDateAndTime/>")).unwrap();
    let (parts, all) = saved(&doc);
    assert!(!all.contains("w:date="));
    assert!(all.contains("Quill Ramsey") && all.contains("Ona Vettel"));
    assert!(parts["docProps/core.xml"].contains("<dc:creator>Quill Ramsey</dc:creator>"));
    assert!(!parts["word/settings.xml"].contains("removePersonalInformation"));
    // Names only: dates stay.
    let doc = wordcraft_docx::read(&fixture(r#"<w:removePersonalInformation w:val="1"/>"#)).unwrap();
    let (_, all) = saved(&doc);
    for name in NAMES {
        assert!(!all.contains(name), "{name} saved");
    }
    assert_eq!(all.matches("w:date=").count(), 4, "two comments and two changes keep their dates");
    // Off (explicitly or absent): everything is saved as it was.
    for settings in ["", r#"<w:removePersonalInformation w:val="0"/><w:removeDateAndTime w:val="false"/>"#] {
        let doc = wordcraft_docx::read(&fixture(settings)).unwrap();
        assert!(!doc.settings.remove_personal_info && !doc.settings.remove_date_time);
        let (parts, all) = saved(&doc);
        assert!(all.contains("Quill Ramsey") && all.contains(r#"w:initials="OV""#));
        assert_eq!(all.matches("w:date=").count(), 4);
        assert!(parts["docProps/core.xml"].contains("<cp:lastModifiedBy>Ona Vettel</cp:lastModifiedBy>"));
        assert!(!parts["word/settings.xml"].contains("removeDateAndTime"));
    }
}
