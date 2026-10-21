use wordcraft_doc::para::WrapText;
use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::{CharProps, Document, Paragraph};
use wordcraft_layout::fields::FieldCtx;
use wordcraft_layout::para::{Exclusion, Grid, ParaEnv, layout_para};

#[test]
fn old_public_shapes_stay_valid() {
    let doc = Document::new();
    let fields = FieldCtx::default();
    let exclusions = [Exclusion { top: 0.0, bottom: 60.0, left: 0.0, right: 180.0, top_bottom: false, contour: None, side: WrapText::BothSides }];
    let env = ParaEnv {
        doc: &doc,
        width: 200.0,
        label: None,
        fields: &fields,
        show_hidden: false,
        field_codes: false,
        hide_deleted: Default::default(),
        table: None,
        proofing: None,
        typing_word: None,
        numeral: Default::default(),
        exclusions: &exclusions,
        eq_number: 0,
        read_line_spacing: None,
        grid: Grid::default(),
        fit_objects: false,
        fitted: &[],
    };
    let mut p = Paragraph::with_text("Independent", CharProps { font: Some("Source Sans 3".into()), size: Some(12.0), ..Default::default() });
    p.props.line_spacing = Some(LineSpacing::Exactly(12.0));
    let laid = layout_para(&p, &env);
    assert!(laid.lines[0].top >= 60.0, "the unchanged public entry keeps default Wait");
    assert_eq!(laid.lines.last().unwrap().stop, p.text.len());
}
