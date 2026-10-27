use super::*;

/// The Paragraph dialog as one paragraph's formatting fills it, on page `tab`.
pub(super) fn paragraph_dialog(rp: &wordcraft_doc::resolve::ResolvedPara, tab: u8) -> Dialog {
    let line = match rp.line_spacing {
        wordcraft_doc::props::LineSpacing::Multiple(m) => m,
        _ => 1.0,
    };
    let (vl, vr) = rp.visual_indents();
    let unit = crate::options::unit().pt_per_unit();
    Dialog::Paragraph {
        rtl: rp.bidi,
        // The serde name (`thaiDistribute`), the one `para.set` and the combo's choices use.
        align: serde_json::to_value(rp.align.visual(rp.bidi)).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default(),
        left: vl / unit,
        right: vr / unit,
        first: rp.indent_first / unit,
        before: rp.space_before,
        after: rp.space_after,
        line,
        keep_next: rp.keep_next,
        keep_lines: rp.keep_lines,
        page_break: rp.page_break_before,
        widow: rp.widow_control,
        tab,
        asian: [rp.kinsoku, rp.word_wrap, rp.overflow_punct, rp.top_line_punct, rp.auto_space_de, rp.auto_space_dn],
        opts: [rp.mirror_indents, rp.contextual_spacing, rp.suppress_line_numbers, rp.suppress_hyphens],
        start: serde_json::Map::new(),
        mixed: BTreeSet::new(),
    }
}

/// The Paragraph dialog's fields as `para.set` takes them: `align`, `left` and `right` as seen
/// on the page (its `onPage`), the rest as paragraph properties, line spacing as a multiple.
pub(super) fn para_fields(d: &Dialog) -> serde_json::Map<String, Value> {
    let Dialog::Paragraph { rtl, align, left, right, first, before, after, line, keep_next, keep_lines, page_break, widow, asian, opts, .. } = d
    else {
        return serde_json::Map::new();
    };
    let unit = crate::options::unit().pt_per_unit();
    let [kinsoku, word_wrap, overflow, top_line, de, dn] = asian;
    let [mirror, contextual, no_numbers, no_hyphens] = opts;
    let fields = json!({
        "bidi": rtl, "align": align, "left": left * unit, "right": right * unit, "indentFirst": first * unit,
        "spaceBefore": before, "spaceAfter": after, "lineSpacing": {"rule": "multiple", "value": line},
        "keepNext": keep_next, "keepLines": keep_lines, "pageBreakBefore": page_break, "widowControl": widow,
        "mirrorIndents": mirror, "contextualSpacing": contextual, "suppressLineNumbers": no_numbers, "suppressHyphens": no_hyphens,
        "kinsoku": kinsoku, "wordWrap": word_wrap, "overflowPunct": overflow, "topLinePunct": top_line, "autoSpaceDE": de, "autoSpaceDN": dn,
    });
    match fields {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    }
}

/// The Paragraph dialog's number boxes: a length in the measurement unit (indents), points
/// (spacing) or lines (line spacing).
#[derive(Clone, Copy)]
pub(super) enum ParaNumber {
    Length,
    Points,
    Lines,
}

/// A Paragraph dialog number box for the field `key`. While the selected paragraphs differ in
/// it, it shows nothing, not even the unit, until a value is typed or dragged in.
pub(super) fn para_number(ui: &mut Ui, value: &mut f32, kind: ParaNumber, key: &str, mixed: &mut BTreeSet<String>, label: &egui::Response) {
    let unit = crate::options::unit();
    let blank = mixed.contains(key);
    // A value from the document outside the range (line spacing 6) stays until edited.
    let field = match kind {
        ParaNumber::Length => egui::DragValue::new(value).speed(3.6 / unit.pt_per_unit()).max_decimals(2),
        ParaNumber::Points => egui::DragValue::new(value).speed(1.0).range(0.0..=1584.0).clamp_existing_to_range(false),
        ParaNumber::Lines => egui::DragValue::new(value).speed(0.05).range(0.5..=5.0).clamp_existing_to_range(false),
    };
    let field = match kind {
        ParaNumber::Length if blank => field.measure_blank(unit),
        ParaNumber::Length => field.measure(unit),
        _ if blank => field.custom_formatter(|_, _| String::new()),
        ParaNumber::Points => field.suffix(format!(" {}", tl!("pt"))),
        ParaNumber::Lines => field,
    };
    if ui.add(field).labelled_by(label.id).changed() {
        mixed.remove(key);
    }
}

/// A Paragraph dialog check box for the field `key`. While the selected paragraphs differ in
/// it, it shows as neither on nor off, and a click turns it on for all of them.
pub(super) fn para_check(ui: &mut Ui, on: &mut bool, text: &str, key: &str, mixed: &mut BTreeSet<String>) {
    if !mixed.contains(key) {
        ui.checkbox(on, text);
    } else if ui.add(egui::Checkbox::new(&mut false, text).indeterminate(true)).clicked() {
        *on = true;
        mixed.remove(key);
    }
}

/// The Paragraph dialog's OK, as one `para.set` and so one undo step (#1346): the fields that
/// differ from what every selected paragraph had when it opened, not the blank ones, which
/// stay as each paragraph has them. The alignment and indents are as seen on the page, so a
/// new direction keeps them where the dialog shows them.
pub(super) fn apply_paragraph(app: &mut WordApp, d: &Dialog) {
    let Dialog::Paragraph { start, mixed, .. } = d else { return };
    let fields = para_fields(d);
    let turned = !mixed.contains("bidi") && fields.get("bidi") != start.get("bidi");
    let (mut props, mut page) = (serde_json::Map::new(), serde_json::Map::new());
    for (k, v) in fields {
        let on_page = matches!(k.as_str(), "align" | "left" | "right");
        if mixed.contains(&k) || (start.get(&k) == Some(&v) && !(turned && on_page)) {
            continue;
        }
        if on_page {
            page.insert(k, v);
        } else {
            props.insert(k, v);
        }
    }
    if !props.is_empty() || !page.is_empty() {
        let _ = app.run("para.set", json!({"props": props, "onPage": page}));
    }
}
