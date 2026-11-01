//! Every story of a `.doc` header subdocument ends with a guard paragraph mark Word does not show
//! ([MS-DOC] §2.8.25 Plcfhdd). Synthetic stories, shaped like the ones measured in real files
//! (`Confidential…\r\r`, `FOOTER…\r\r\r`).

use super::{StructSpec, plc, read, struct_doc, structured_spec};
use wordcraft_doc::Document;

/// One section whose odd header is `header` and odd footer is `footer` (either may be empty).
fn doc_with(header: &'static str, footer: &'static str) -> Document {
    let text = "Body\r";
    let ccp = text.chars().count() as u32;
    let h = header.chars().count() as u32;
    let f = footer.chars().count() as u32;
    // Six separator stories, then even header, odd header, even footer, odd footer, first
    // header, first footer.
    let hdd = plc(&[0, 0, 0, 0, 0, 0, 0, 0, h, h, h + f, h + f, h + f], &[]);
    let stories: Vec<&'static str> = [header, footer].into_iter().filter(|s| !s.is_empty()).collect();
    let spec = StructSpec { text, header_stories: stories, extra_pairs: vec![(11, hdd)], ..structured_spec(text, ccp) };
    read(&struct_doc(&spec)).expect("opens")
}

fn part_paras(doc: &Document, id: Option<u32>) -> Vec<String> {
    let Some(part) = id.and_then(|id| doc.parts.get(&id)) else { return Vec::new() };
    part.blocks.iter().filter_map(|b| b.as_para().map(|p| p.text.clone())).collect()
}

fn header_paras(doc: &Document) -> Vec<String> {
    part_paras(doc, doc.last_section.headers.default)
}

fn footer_paras(doc: &Document) -> Vec<String> {
    part_paras(doc, doc.last_section.footers.default)
}

#[test]
fn guard_mark_after_one_paragraph_is_dropped() {
    let doc = doc_with("Confidential\r\r", "Footer\r\r\r");
    assert_eq!(header_paras(&doc), ["Confidential"]);
    assert_eq!(footer_paras(&doc), ["Footer", ""]);
}

#[test]
fn single_mark_story_is_unchanged() {
    let doc = doc_with("Head\r", "");
    assert_eq!(header_paras(&doc), ["Head"]);
}

#[test]
fn guard_after_several_paragraphs() {
    let doc = doc_with("A\rB\r\r", "Only\r\r");
    assert_eq!(header_paras(&doc), ["A", "B"]);
    assert_eq!(footer_paras(&doc), ["Only"]);
}

#[test]
fn body_is_untouched() {
    let doc = doc_with("Confidential\r\r", "Footer\r\r\r");
    let body: Vec<String> = doc.body.iter().filter_map(|b| b.as_para().map(|p| p.text.clone())).collect();
    assert_eq!(body, ["Body"]);
}
