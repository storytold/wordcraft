//! Normal text in an equation (`m:nor`) is saved without `m:scr`/`m:sty` (#6073). In the OMML
//! schema `CT_RPR` holds `m:nor` or `m:scr`/`m:sty`, not both, and Word keeps only the `m:nor` of
//! such a run, so the bold or italic of normal text goes in `w:rPr` (`w:b`/`w:i`), which the
//! reader takes back for normal text.

use wordcraft_doc::math::{MNode, MRun, MScr, MSty, Math};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, Paragraph, para_block};

/// Save an equation holding one normal-text run; the run's `m:r` XML and the reopened run.
fn save(run: MRun) -> (String, MRun) {
    let math = Math { nodes: vec![MNode::Run(run)], ..Default::default() };
    let mut p = Paragraph::new();
    p.insert_object(0, InlineObject::Equation { linear: String::new(), display: false, math }, &CharProps::default()).unwrap();
    let mut doc = Document::new();
    doc.body = vec![para_block(p)];
    let bytes = wordcraft_docx::write(&doc).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    let start = xml.find("<m:r>").unwrap();
    let end = start + xml[start..].find("</m:r>").unwrap() + "</m:r>".len();
    let back = wordcraft_docx::read(&bytes).unwrap();
    let math = back.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).find_map(|o| match o {
        InlineObject::Equation { math, .. } => Some(math.clone()),
        _ => None,
    });
    let Some(MNode::Run(r)) = math.unwrap().nodes.into_iter().next() else { panic!("no run") };
    (xml[start..end].to_string(), r)
}

fn normal(sty: Option<MSty>) -> MRun {
    MRun { nor: true, sty, font: Some("Arial".into()), ..MRun::new("normal text") }
}

#[test]
fn normal_text_has_no_math_style() {
    for sty in [None, Some(MSty::Plain), Some(MSty::Bold), Some(MSty::Italic), Some(MSty::BoldItalic)] {
        let (run, _) = save(normal(sty));
        assert!(run.contains("<m:nor/>"), "{sty:?}: {run}");
        assert!(!run.contains("m:sty") && !run.contains("m:scr"), "{sty:?}: {run}");
    }
    // A script style isn't written for normal text either.
    let (run, _) = save(MRun { scr: MScr::Fraktur, ..normal(None) });
    assert!(!run.contains("m:scr"), "{run}");
}

#[test]
fn bold_and_italic_normal_text_survive_a_save() {
    for (sty, b, i) in [(MSty::Bold, true, false), (MSty::Italic, false, true), (MSty::BoldItalic, true, true)] {
        let (run, back) = save(normal(Some(sty)));
        assert_eq!((run.contains("<w:b/>"), run.contains("<w:i/>")), (b, i), "{sty:?}: {run}");
        assert_eq!(back, normal(Some(sty)), "{sty:?}");
    }
}

#[test]
fn math_runs_keep_their_style() {
    for sty in [MSty::Plain, MSty::Bold, MSty::Italic, MSty::BoldItalic] {
        let math = MRun { sty: Some(sty), ..MRun::new("x") };
        let (run, back) = save(math.clone());
        assert!(run.contains("m:sty") && !run.contains("<w:b/>"), "{sty:?}: {run}");
        assert_eq!(back, math);
    }
}
