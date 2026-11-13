//! Ruby (ODF 1.3 §6.4 `text:ruby`: a base text with ruby text over it, furigana or pinyin) read as
//! WordCraft's Phonetic Guide and written back as ODF ruby.

use crate::odt::tests::{NS_T, content_of, package};
use crate::odt::{export, import};
use wordcraft_doc::para::InlineObject;

fn open(p: &str) -> wordcraft_doc::Document {
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="Ru1" style:family="ruby"><style:ruby-properties style:ruby-position="above" style:ruby-align="center"/></style:style></office:automatic-styles><office:body><office:text>{p}</office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

/// Each ruby: (base, ruby text), and the paragraph's text.
fn rubies(d: &wordcraft_doc::Document) -> (Vec<(String, String)>, String) {
    let p = d.body.iter().find_map(|b| b.as_para()).unwrap();
    let r = p
        .objects
        .iter()
        .filter_map(|o| if let InlineObject::Ruby { base, ruby, .. } = o { Some((base.clone(), ruby.clone())) } else { None })
        .collect();
    (r, p.text.replace(wordcraft_doc::para::OBJ, "[R]"))
}

#[test]
fn ruby_is_read_as_a_phonetic_guide_and_written_back() {
    let d = open(
        r#"<text:p>Ruby: <text:ruby text:style-name="Ru1"><text:ruby-base>漢字</text:ruby-base><text:ruby-text>かんじ</text:ruby-text></text:ruby> after.</text:p>"#,
    );
    assert_eq!(rubies(&d), (vec![("漢字".to_string(), "かんじ".to_string())], "Ruby: [R] after.".to_string()));
    let c = content_of(&export(&d).unwrap());
    assert!(c.contains("<text:ruby-base>漢字</text:ruby-base><text:ruby-text>かんじ</text:ruby-text></text:ruby>"), "{c}");
    assert!(c.contains(r#"style:family="ruby"><style:ruby-properties style:ruby-position="above""#), "{c}");
    assert_eq!(rubies(&import(&export(&d).unwrap()).unwrap()).0, vec![("漢字".to_string(), "かんじ".to_string())]);
}

#[test]
fn ruby_without_a_reading_or_a_base_is_its_text() {
    let d = open(r#"<text:p><text:ruby><text:ruby-base>漢字</text:ruby-base><text:ruby-text/></text:ruby>!</text:p>"#);
    assert_eq!(rubies(&d), (Vec::new(), "漢字!".to_string()));
    let d = open(
        r#"<text:p><text:ruby><text:ruby-base><text:span>基</text:span>本</text:ruby-base><text:ruby-text>きほん</text:ruby-text></text:ruby></text:p>"#,
    );
    assert_eq!(rubies(&d).0, vec![("基本".to_string(), "きほん".to_string())]);
}
