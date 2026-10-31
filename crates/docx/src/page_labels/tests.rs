use super::*;
use crate::{Flavor, package::zip_entries, read, write};
use wordcraft_doc::page_label::PageLabelMode;
use wordcraft_doc::{Block, Document, InlineObject, Paragraph, PartKind, Path, StoryRef, props::CharProps, section::NumFormat};

fn without_extensions(doc: &Document) -> Vec<u8> {
    let mut entries = crate::write::write_parts(doc, Flavor::Document).unwrap();
    for (name, bytes) in &mut entries {
        if !name.starts_with("word/") || !name.ends_with(".xml") {
            continue;
        }
        let mut xml = String::from_utf8(bytes.clone()).unwrap();
        for attr in ["wc:pageLabel", "wc:pageField", "wc:locked"] {
            let prefix = format!(" {attr}=\"");
            while let Some(start) = xml.find(&prefix) {
                let end = start + prefix.len() + xml[start + prefix.len()..].find('"').unwrap() + 1;
                xml.replace_range(start..end, "");
            }
        }
        *bytes = xml.into_bytes();
    }
    zip_entries(&entries).unwrap()
}

fn footer(doc: &mut Document, locked: bool) -> u32 {
    let mut p = Paragraph::with_text("Keep footer text: ", CharProps::default());
    p.insert_object(
        p.len(),
        InlineObject::Field {
            instr: r"PAGE \* Upper".into(),
            result: "Saved\tlocked\nresult".into(),
            locked,
            dirty: false,
            code: false,
            form: String::new(),
        },
        &CharProps::default(),
    )
    .unwrap();
    doc.add_part(PartKind::Footer, vec![std::sync::Arc::new(Block::Para(p))])
}

#[test]
fn standard_properties_restore_labels_and_page_fields_without_ignorable_attributes() {
    for mode in [PageLabelMode::Hidden, PageLabelMode::Text, PageLabelMode::Number, PageLabelMode::TextNumber] {
        for locked in [false, true] {
            let mut doc = Document::from_text("One\u{000c}Two");
            let id = footer(&mut doc, locked);
            doc.last_section.footers.default = Some(id);
            doc.last_section.page_num_format = NumFormat::UpperRoman;
            doc.last_section.page_num_start = Some(0);
            // Exercise long Unicode values beyond Word's legacy property size.
            let label = PageLabel { mode, name: "😀".repeat(256), text: "ä".repeat(256), zero_label: "Opening".into(), ..Default::default() };
            doc.last_section.page_label = Some(label.clone());
            doc.set_custom_prop("User property", "Keep this too");
            let bytes = without_extensions(&doc);
            let imported = read(&bytes).unwrap();
            assert_eq!(imported.last_section.page_label, Some(label));
            assert_eq!(imported.last_section.page_num_format, NumFormat::UpperRoman);
            assert_eq!(imported.last_section.page_num_start, Some(0));
            assert_eq!(imported.custom_props, doc.custom_props, "implementation metadata is consumed, user properties survive");
            let id = imported.last_section.footers.default.unwrap();
            let p = imported.para(StoryRef::Part(id), &Path::top(0)).unwrap();
            assert!(p.text.contains("Keep footer text: "));
            assert!(p.objects.iter().any(|o| matches!(o, InlineObject::Field {instr,locked:l,result,..} if instr.trim() == r"PAGE \* Upper" && *l == locked && (!locked || result == "Saved\tlocked\nresult"))));
            let twice = read(&write(&imported).unwrap()).unwrap();
            assert_eq!(twice.last_section.page_label, imported.last_section.page_label);
        }
    }
}

#[test]
fn shared_footers_recover_their_own_labels_after_section_reordering() {
    let mut doc = Document::from_text("First\nSecond\nThird");
    let id = footer(&mut doc, false);
    for (block, text) in [(0, "Preface"), (1, "Notice")] {
        let mut sec = wordcraft_doc::SectionProps {
            page_label: Some(PageLabel { mode: PageLabelMode::Text, text: text.into(), ..Default::default() }),
            ..Default::default()
        };
        sec.footers.default = Some(id);
        doc.para_mut(StoryRef::Body, &Path::top(block)).unwrap().section = Some(Box::new(sec));
    }
    doc.body.swap(0, 1);
    doc.last_section.footers.default = Some(id);
    let imported = read(&without_extensions(&doc)).unwrap();
    assert_eq!(imported.sections()[0].1.page_label.as_ref().unwrap().text, "Notice");
    assert_eq!(imported.sections()[1].1.page_label.as_ref().unwrap().text, "Preface");
    assert!(imported.last_section.page_label.is_none());
}

#[test]
fn property_chunks_are_bounded_deduplicated_and_checked_before_use() {
    let label = PageLabel { text: "😀".repeat(256), ..Default::default() };
    let mut export = Export::new(&[]);
    let tag = export.insert(&label, "PAGE", None).unwrap();
    assert!(tag.len() <= 64);
    let props: Vec<_> = export.properties().cloned().collect();
    assert!(props.len() > 2);
    assert!(props.iter().all(|p| p.value.encode_utf16().count() <= CHUNK_UNITS));
    assert_eq!(export.insert(&label, "PAGE", None), Some(tag.clone()));
    assert_eq!(export.properties().count(), props.len());
    let imported = Import::read(&props);
    assert_eq!(imported.records[&tag].label, label);
    assert!(Import::read(&props[1..]).records.is_empty());
    let mut broken = props.clone();
    broken.last_mut().unwrap().value.push('x');
    assert!(Import::read(&broken).records.is_empty());
    let mut duplicate = props.clone();
    duplicate.push(props[0].clone());
    assert!(Import::read(&duplicate).records.is_empty());
    assert!(Export::new(&props).insert(&label, "PAGE", None).is_none(), "never overwrite a pre-existing property");
    assert!(export.insert(&label, "INCLUDETEXT anything", None).is_none());
}

#[test]
fn newly_inserted_sections_inherit_labels_but_explicit_plain_footers_do_not() {
    let mut doc = Document::from_text("First\nInserted\nPlain");
    let id = footer(&mut doc, false);
    let plain = footer(&mut doc, false);
    let label = PageLabel { mode: PageLabelMode::Text, text: "Vorwort".into(), ..Default::default() };
    let mut first = wordcraft_doc::SectionProps::default();
    first.footers.default = Some(id);
    doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(first));
    doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().section = Some(Box::default());
    doc.last_section.footers.default = Some(plain);
    restore_inherited_sections(&mut doc, &HashMap::from([(id, label.clone())]));
    assert_eq!(doc.sections()[0].1.page_label, Some(label.clone()));
    assert_eq!(doc.sections()[1].1.page_label, Some(label));
    assert!(doc.last_section.page_label.is_none());
}

fn edit_part(bytes: &[u8], part: &str, before: &str, after: &str) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut entries = Vec::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        let name = file.name().to_string();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut bytes).unwrap();
        if name == part {
            let xml = String::from_utf8(bytes).unwrap();
            let start = xml.find("<w:sdtContent>").unwrap();
            assert!(xml[start..].contains(before));
            bytes = format!("{}{}", &xml[..start], xml[start..].replace(before, after)).into_bytes();
        }
        entries.push((name, bytes));
    }
    zip_entries(&entries).unwrap()
}

#[test]
fn generated_label_cache_updates_and_split_runs_still_recover() {
    let mut doc = Document::from_text("Body");
    let id = footer(&mut doc, false);
    doc.last_section.footers.default = Some(id);
    doc.last_section.page_label = Some(PageLabel { mode: PageLabelMode::TextNumber, text: "Preface".into(), ..Default::default() });
    let bytes = edit_part(&without_extensions(&doc), "word/footer1.xml", ">1</w:t>", ">42</w:t>");
    let bytes = edit_part(&bytes, "word/footer1.xml", "Preface ", "Pre</w:t></w:r><w:r><w:t>face ");
    let imported = read(&bytes).unwrap();
    assert_eq!(imported.last_section.page_label, doc.last_section.page_label);
    assert_eq!(imported.plain_text(StoryRef::Part(imported.last_section.footers.default.unwrap())), "Keep footer text: ");
}

#[test]
fn body_label_edits_preserve_sibling_header_fallbacks() {
    let mut doc = Document::from_text("Body");
    let id = footer(&mut doc, false);
    doc.last_section.headers.default = Some(id);
    doc.parts.get_mut(&id).unwrap().kind = PartKind::Header;
    doc.last_section.page_label = Some(PageLabel { mode: PageLabelMode::TextNumber, text: "Preface".into(), ..Default::default() });
    let p = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.insert_object(
        p.len(),
        InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() },
        &CharProps::default(),
    )
    .unwrap();
    let bytes = edit_part(&write(&doc).unwrap(), "word/document.xml", "Preface", "Edited");
    let imported = read(&bytes).unwrap();
    assert!(imported.last_section.page_label.is_none());
    assert!(imported.plain_text(StoryRef::Body).contains("Edited"));
    assert!(imported.plain_text(StoryRef::Part(imported.last_section.headers.default.unwrap())).contains("Preface"));
    let again = read(&write(&imported).unwrap()).unwrap();
    assert!(again.plain_text(StoryRef::Body).contains("Edited"));
    assert!(again.plain_text(StoryRef::Part(again.last_section.headers.default.unwrap())).contains("Preface"));
}

#[test]
fn edited_generated_labels_and_codes_survive_recovery() {
    for strip in [false, true] {
        for (before, after) in [("Preface", "Edited preface"), ("PAGE \\* Upper", "NUMPAGES \\* Upper")] {
            let mut doc = Document::from_text("Body");
            let id = footer(&mut doc, false);
            doc.last_section.footers.default = Some(id);
            doc.last_section.page_label = Some(PageLabel { mode: PageLabelMode::TextNumber, text: "Preface".into(), ..Default::default() });
            let bytes = if strip { without_extensions(&doc) } else { write(&doc).unwrap() };
            let imported = read(&edit_part(&bytes, "word/footer1.xml", before, after)).unwrap();
            assert!(imported.last_section.page_label.is_none(), "edited fallback must not use stale segment rules");
            let id = imported.last_section.footers.default.unwrap();
            let para = imported.para(StoryRef::Part(id), &Path::top(0)).unwrap();
            if before == "Preface" {
                assert!(para.plain_text().contains(after));
            } else {
                assert!(para.objects.iter().any(|o| matches!(o, InlineObject::Field {instr,..} if instr.contains("NUMPAGES"))));
            }
            let again = read(&write(&imported).unwrap()).unwrap();
            assert!(again.last_section.page_label.is_none());
            assert_eq!(again.plain_text(StoryRef::Part(again.last_section.footers.default.unwrap())), imported.plain_text(StoryRef::Part(id)));
        }
    }
}

#[test]
fn malformed_or_excessively_nested_field_markers_do_not_get_a_fingerprint() {
    for body in ["<w:fldChar w:fldCharType=\"end\"/>".to_string(), "<w:fldChar w:fldCharType=\"begin\"/>".repeat(MAX_DEPTH + 1)] {
        let xml = format!("<w:sdtContent xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{body}</w:sdtContent>");
        assert!(content_hash(&crate::xml::parse(xml.as_bytes()).unwrap()).is_none());
    }
}
