//! An equation's kept source OMML is written back verbatim only when it is exactly one well-formed
//! `m:oMath` (#6072). `Math::omml` also comes from `.json` documents, so anything else must not
//! reach `document.xml`: it is written from the equation's nodes instead.

use wordcraft_doc::math::{MNode, MRun, Math};
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, Paragraph, para_block};

/// Save a one-equation document whose equation keeps `omml` and has the nodes `NODES`.
fn save(omml: &str, display: bool) -> (String, Document) {
    let math = Math { nodes: vec![MNode::Run(MRun::new("NODES"))], omml: omml.into(), ..Default::default() };
    let mut p = Paragraph::new();
    p.insert_object(0, InlineObject::Equation { linear: "NODES".into(), display, math }, &CharProps::default()).unwrap();
    let mut doc = Document::new();
    doc.body = vec![para_block(p)];
    let bytes = wordcraft_docx::write(&doc).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    // The saved file must open again (the main part is read strictly).
    let back = wordcraft_docx::read(&bytes).unwrap_or_else(|e| panic!("{omml:?} doesn't reopen: {e}"));
    (xml, back)
}

/// The text of the reopened document's equations and paragraphs.
fn texts(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    for b in &doc.body {
        let Some(p) = b.as_para() else { continue };
        for o in &p.objects {
            if let InlineObject::Equation { math, .. } = o {
                out.push(wordcraft_doc::math::to_linear(&math.nodes));
            }
        }
        let text: String = p.text.chars().filter(|c| *c != OBJ).collect();
        if !text.trim().is_empty() {
            out.push(text);
        }
    }
    out
}

#[test]
fn well_formed_kept_omml_is_written_back_unchanged() {
    let kept = "<m:oMath><m:r><m:t>KEPT</m:t></m:r></m:oMath>";
    let (xml, back) = save(kept, false);
    assert!(xml.contains(kept), "{xml}");
    assert_eq!(texts(&back), vec!["KEPT".to_string()]);
}

#[test]
fn malformed_kept_omml_is_written_from_the_nodes() {
    for kept in [
        // Unclosed, mismatched, an undefined entity.
        "<m:oMath><m:r>",
        "<m:oMath></m:r>",
        "<m:oMath><m:r><m:t>&bad;</m:t></m:r></m:oMath>",
        // Markup after the element, text after it, a second element.
        "<m:oMath></m:oMath></w:p><w:p><w:r><w:t>INJECTED</w:t></w:r></w:p><w:p>",
        "<m:oMath></m:oMath>INJECTED",
        "<m:oMath></m:oMath><w:r><w:t>INJECTED</w:t></w:r>",
        // Not an m:oMath.
        "<m:r><m:t>INJECTED</m:t></m:r>",
    ] {
        let (xml, back) = save(kept, false);
        assert!(!xml.contains("INJECTED") && !xml.contains("&bad;"), "{kept:?} was written: {xml}");
        assert_eq!(texts(&back), vec!["NODES".to_string()], "{kept:?}");
    }
}

#[test]
fn a_kept_omath_para_is_not_nested_in_another() {
    let kept = "<m:oMathPara><m:oMath><m:r><m:t>P</m:t></m:r></m:oMath></m:oMathPara>";
    let (xml, back) = save(kept, true);
    assert!(!xml.contains("<m:oMathPara><m:oMathPara>"), "{xml}");
    assert_eq!(texts(&back), vec!["NODES".to_string()]);
}
