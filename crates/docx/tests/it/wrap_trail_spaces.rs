//! `w:wrapTrailSpaces` (Wrap trailing spaces onto the next line, a legacy `w:compat` option) is
//! read into `Settings::wrap_trail_spaces` and written back, in CT_Compat's order: after
//! `w:doNotExpandShiftReturn`, before `w:doNotUseHTMLParagraphAutoSpacing`.

use wordcraft_doc::Document;

fn settings_xml(d: &Document) -> String {
    let bytes = wordcraft_docx::write(d).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/settings.xml").unwrap(), &mut xml).unwrap();
    xml
}

fn rt(d: &Document) -> Document {
    wordcraft_docx::read(&wordcraft_docx::write(d).unwrap()).unwrap()
}

#[test]
fn wrap_trail_spaces_round_trips() {
    let mut d = Document::new();
    assert!(!rt(&d).settings.wrap_trail_spaces);
    assert!(!settings_xml(&d).contains("wrapTrailSpaces"));
    d.settings.wrap_trail_spaces = true;
    assert!(rt(&d).settings.wrap_trail_spaces);
    assert_eq!(settings_xml(&d).matches("<w:wrapTrailSpaces").count(), 1);
}

#[test]
fn wrap_trail_spaces_is_written_in_ct_compat_order() {
    let mut d = Document::new();
    d.settings.wrap_trail_spaces = true;
    d.settings.do_not_expand_shift_return = true;
    d.settings.sum_paragraph_spacing = true;
    let xml = settings_xml(&d);
    let at = |e: &str| xml.find(e).unwrap_or_else(|| panic!("{e} missing in {xml}"));
    assert!(at("<w:doNotExpandShiftReturn") < at("<w:wrapTrailSpaces"));
    assert!(at("<w:wrapTrailSpaces") < at("<w:doNotUseHTMLParagraphAutoSpacing"));
    assert!(at("<w:wrapTrailSpaces") < at("<w:compatSetting"));
}

#[test]
fn a_docx_with_wrap_trail_spaces_off_or_on_reads_it() {
    let mut d = Document::new();
    d.settings.wrap_trail_spaces = true;
    let on = settings_xml(&d);
    let off = on.replace("<w:wrapTrailSpaces/>", "<w:wrapTrailSpaces w:val=\"0\"/>");
    assert_ne!(on, off, "the writer writes an empty <w:wrapTrailSpaces/>");
    let mut bytes = Vec::new();
    {
        let src = wordcraft_docx::write(&d).unwrap();
        let mut zin = zip::ZipArchive::new(std::io::Cursor::new(src)).unwrap();
        let mut zout = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        for i in 0..zin.len() {
            let mut f = zin.by_index(i).unwrap();
            let name = f.name().to_string();
            let mut data = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut data).unwrap();
            zout.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
            let data = if name == "word/settings.xml" { off.clone().into_bytes() } else { data };
            std::io::Write::write_all(&mut zout, &data).unwrap();
        }
        zout.finish().unwrap();
    }
    assert!(!wordcraft_docx::read(&bytes).unwrap().settings.wrap_trail_spaces, "w:val=\"0\" is off");
}
