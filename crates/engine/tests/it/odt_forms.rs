//! ODT form controls (ODF 1.3 §13) open as content controls: a drop-down list and a check box
//! stay what they are when the document is saved as DOCX (`w:sdt`) and as ODT again. The ODT is
//! original and synthetic, shaped like LibreOffice's.

use std::io::{Read, Write};

use wordcraft_doc::control::ControlKind;
use wordcraft_doc::para::InlineObject;
use wordcraft_engine::io;

fn odt() -> Vec<u8> {
    let content = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:form="urn:oasis:names:tc:opendocument:xmlns:form:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"><office:body><office:text><office:forms><form:form form:name="Form"><form:listbox form:name="Colour" form:id="c1" form:dropdown="true"><form:option form:label="Red"/><form:option form:label="Green" form:current-selected="true"/></form:listbox><form:checkbox form:name="Agree" form:id="c2" form:current-state="unchecked"/></form:form></office:forms><text:p>Colour: <draw:control draw:control="c1"/> Agree: <draw:control draw:control="c2"/></text:p></office:text></office:body></office:document-content>"#;
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content)] {
        z.start_file(name, o).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn kinds(doc: &wordcraft_doc::Document) -> Vec<&'static str> {
    doc.body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| if let InlineObject::ControlStart { control } = o { Some(control.kind.name()) } else { None })
        .collect()
}

#[test]
fn odt_form_controls_become_content_controls_everywhere() {
    let doc = io::open_bytes("form.odt", &odt()).unwrap();
    assert_eq!(kinds(&doc), ["dropDown", "checkBox"]);
    let text: String = doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.replace(wordcraft_doc::para::OBJ, "")).collect();
    assert_eq!(text, "Colour: Green Agree: \u{2610}");
    // DOCX: Word content controls.
    let docx = io::save_bytes("form.docx", &doc).unwrap();
    let mut body = String::new();
    zip::ZipArchive::new(std::io::Cursor::new(&docx)).unwrap().by_name("word/document.xml").unwrap().read_to_string(&mut body).unwrap();
    assert!(body.contains("<w:dropDownList") && body.contains("w14:checkbox"), "{body}");
    let again = io::open_bytes("form.docx", &docx).unwrap();
    assert_eq!(kinds(&again), ["dropDown", "checkBox"]);
    // ODT again: form controls.
    let odt2 = io::open_bytes("form.odt", &io::save_bytes("form.odt", &again).unwrap()).unwrap();
    assert_eq!(kinds(&odt2), ["dropDown", "checkBox"]);
    assert!(odt2.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).any(
        |o| matches!(o, InlineObject::ControlStart { control } if matches!(&control.kind, ControlKind::DropDown { items, .. } if items.len() == 2))
    ));
}
