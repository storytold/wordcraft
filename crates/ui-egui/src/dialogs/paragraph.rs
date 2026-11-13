use super::{BTreeSet, Dialog, MeasureExt, Ui, Value, WordApp, json};

/// The Paragraph dialog as one paragraph's formatting fills it, on page `tab`.
pub(super) fn paragraph_dialog(rp: &wordcraft_doc::resolve::ResolvedPara, tab: u8) -> Dialog {
    let line = match rp.line_spacing {
        wordcraft_doc::props::LineSpacing::Multiple(m) => m,
        _ => 1.0,
    };
    let (vl, vr) = rp.visual_indents();
    let unit = crate::options::unit().pt_per_unit();
    // Indents in characters show as characters (`2 ch`), the rest in the measurement unit.
    let [start, end, first] = rp.indent_chars;
    let (cl, cr) = if rp.bidi { (end, start) } else { (start, end) };
    let shown = |chars: f32, pt: f32| if chars != 0.0 { chars } else { pt / unit };
    Dialog::Paragraph {
        rtl: rp.bidi,
        align: format!("{:?}", rp.align.visual(rp.bidi)).to_lowercase(),
        left: shown(cl, vl),
        right: shown(cr, vr),
        first: shown(first, rp.indent_first),
        chars: [cl, cr, first].map(|c| c != 0.0),
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

/// The indent fields as `para.set` names them, left, right and first line: the length (points)
/// and the characters.
const INDENTS: [(&str, &str); 3] = [("left", "leftChars"), ("right", "rightChars"), ("indentFirst", "indentFirstChars")];

/// The Paragraph dialog's fields as `para.set` takes them: `align`, `left` and `right` as seen
/// on the page (its `onPage`), the rest as paragraph properties, line spacing as a multiple. An
/// indent in characters is just those (`leftChars`); one in a length also clears the characters
/// (`leftChars: 0`), as they would win over it.
pub(super) fn para_fields(d: &Dialog) -> serde_json::Map<String, Value> {
    let Dialog::Paragraph {
        rtl, align, left, right, first, chars, before, after, line, keep_next, keep_lines, page_break, widow, asian, opts, ..
    } = d
    else {
        return serde_json::Map::new();
    };
    let unit = crate::options::unit().pt_per_unit();
    let [kinsoku, word_wrap, overflow, top_line, de, dn] = asian;
    let [mirror, contextual, no_numbers, no_hyphens] = opts;
    let fields = json!({
        "bidi": rtl, "align": align,
        "spaceBefore": before, "spaceAfter": after, "lineSpacing": {"rule": "multiple", "value": line},
        "keepNext": keep_next, "keepLines": keep_lines, "pageBreakBefore": page_break, "widowControl": widow,
        "mirrorIndents": mirror, "contextualSpacing": contextual, "suppressLineNumbers": no_numbers, "suppressHyphens": no_hyphens,
        "kinsoku": kinsoku, "wordWrap": word_wrap, "overflowPunct": overflow, "topLinePunct": top_line, "autoSpaceDE": de, "autoSpaceDN": dn,
    });
    let mut fields = match fields {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    for (((length, in_chars), value), is_chars) in INDENTS.iter().zip([left, right, first]).zip(chars) {
        if *is_chars {
            fields.insert(in_chars.to_string(), json!(*value));
        } else {
            fields.insert(length.to_string(), json!(*value * unit));
            fields.insert(in_chars.to_string(), json!(0.0));
        }
    }
    fields
}

/// The Paragraph dialog's number boxes: points (spacing) or lines (line spacing).
#[derive(Clone, Copy)]
pub(super) enum ParaNumber {
    Points,
    Lines,
}

/// The right-to-left mark.
const RLM: char = '\u{200F}';

/// A Paragraph dialog number box for the field `key`. While the selected paragraphs differ in
/// it, it shows nothing, not even the unit, until a value is typed or dragged in.
pub(super) fn para_number(ui: &mut Ui, value: &mut f32, kind: ParaNumber, key: &str, mixed: &mut BTreeSet<String>, label: &egui::Response) {
    let blank = mixed.contains(key);
    // A value from the document outside the range (line spacing 6) stays until edited.
    let field = match kind {
        ParaNumber::Points => egui::DragValue::new(value).speed(1.0).range(0.0..=1584.0).clamp_existing_to_range(false),
        ParaNumber::Lines => egui::DragValue::new(value).speed(0.05).range(0.5..=5.0).clamp_existing_to_range(false),
    };
    let field = match kind {
        _ if blank => field.custom_formatter(|_, _| String::new()),
        // Right to left, the unit's space goes after the word and a right-to-left mark closes it: a
        // plain leading space between the digits and an Arabic unit is drawn with no width.
        ParaNumber::Points if crate::rtl::on() => field.suffix(format!("{} {RLM}", tl!("pt"))),
        ParaNumber::Points => field.suffix(format!(" {}", tl!("pt"))),
        ParaNumber::Lines => field,
    };
    if ui.add(field).labelled_by(label.id).changed() {
        mixed.remove(key);
    }
}

/// A Paragraph dialog indent box, indent `i` of [`INDENTS`]: a length in the measurement unit,
/// or characters when `in_chars` (`2 ch`, Word's unit for East Asian indents, one character
/// being the font size). A value typed in the other unit (`2 ch`, `1 cm`) switches the box; a
/// number alone stays in its unit. While the selected paragraphs differ in it, it shows nothing,
/// not even the unit, until a value is typed or dragged in.
pub(super) fn para_indent(ui: &mut Ui, value: &mut f32, in_chars: &mut bool, i: usize, mixed: &mut BTreeSet<String>, label: &egui::Response) {
    let Some(&(length, chars)) = INDENTS.get(i) else { return };
    let unit = crate::options::unit();
    let blank = mixed.contains(length) || mixed.contains(chars);
    let was_chars = *in_chars;
    // The unit of the text the box read this frame, if it read any.
    let typed = std::cell::Cell::new(None);
    let parse = |text: &str| {
        if let Some(n) = crate::options::parse_chars(text, was_chars) {
            typed.set(Some(true));
            return Some(n);
        }
        let n = crate::options::parse_length(text, unit)?;
        typed.set(Some(false));
        Some(n)
    };
    let field = egui::DragValue::new(value).max_decimals(2);
    let field = match was_chars {
        _ if blank => field.measure_blank(unit),
        true => field.speed(0.1).measure_chars(),
        false => field.speed(3.6 / unit.pt_per_unit()).measure(unit),
    };
    let changed = ui.add(field.custom_parser(parse)).labelled_by(label.id).changed();
    if let Some(now) = typed.get() {
        *in_chars = now;
    }
    if changed || typed.get().is_some() {
        mixed.remove(length);
        mixed.remove(chars);
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
        let on_page = matches!(k.as_str(), "align" | "left" | "right" | "leftChars" | "rightChars");
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
