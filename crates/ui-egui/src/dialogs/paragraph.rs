//! The Paragraph dialog (and its Asian Typography page): the framework's `ParagraphDialog` over the
//! selected paragraphs, with its OK held until `para.set` has run, so a failing command keeps the
//! dialog open with the reason.

use craft::text::ui::DocUnit;
use craft::text::ui::character::{
    AsianTypography, LineBreakOptions, LineSpacing, ParagraphAlign, ParagraphDialog, ParagraphDirection, ParagraphFormat, ParagraphTab, SpacingKind,
    SpecialIndent,
};
use craft::ui::dialog::DialogId;
use craft::ui::runtime::env::Label;
use serde_json::{Map, Value, json};
use wordcraft_doc::props::{Align, LineSpacing as DocSpacing, ParaProps};
use wordcraft_doc::resolve::ResolvedPara;

use super::host::{DialogHooks, Keyed, Settle, open_with};
use crate::WordApp;
use crate::options::unit_kind;

/// The Paragraph dialog as the dialog host holds it, under its dialog name.
pub(crate) type ParagraphHost = Keyed<ParagraphDialog>;

/// The fields `para.set` reads as seen on the page rather than as stored: a new direction keeps them where the
/// dialog shows them.
const ON_PAGE: [&str; 3] = ["align", "left", "right"];

/// Opens the Paragraph dialog (`paragraph`) or its Asian Typography page (`asianTypography`) over the selected
/// paragraphs. `None` for another name, and while no paragraph holds the caret.
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let (key, tab) = match name {
        "paragraph" => ("paragraph", ParagraphTab::IndentsAndSpacing),
        "asianTypography" => ("asianTypography", ParagraphTab::AsianTypography),
        _ => return None,
    };
    let format = read(app)?;
    // Only pixel units read the resolution, and WordCraft never shows pixels.
    let unit = DocUnit::new(unit_kind(app.session.units()), 96.0);
    let dialog = ParagraphDialog::new(format).word(true).tab(tab).unit(unit).held_ok();
    Some(open_with(app, parent, Keyed::new(key, dialog), DialogHooks { prepare: None, settle }))
}

/// The settle hook: runs a filed OK as `para.set`. The dialog closes when the command ran and stays, showing the
/// reason, when it failed. Tabs… applies first and then opens the Tabs dialog. The hook always keeps itself: a
/// refused OK leaves the dialog open and files its next answer here, and the host drops the hook once the dialog closed.
fn settle(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    let Some(answer) = app.dialog_host.get_mut_by_id::<ParagraphHost>(id).and_then(|host| host.inner_mut().take_ok()) else {
        return Settle::Keep;
    };
    let start = read(app).unwrap_or_default();
    let result = apply_paragraph(app, &answer.format, &start);
    let applied = result.is_ok();
    if let Some(host) = app.dialog_host.get_mut_by_id::<ParagraphHost>(id) {
        host.inner_mut().answer_ok(result.map_err(Label::from));
    }
    if applied && answer.open_tabs {
        crate::dialogs::open(app, "tabs", &json!({}));
    }
    Settle::Keep
}

/// The selected paragraphs as the dialog shows them: a field every paragraph shares is `Some`, the rest `None`
/// (the field starts empty and OK leaves it as each paragraph has it).
pub(super) fn read(app: &WordApp) -> Option<ParagraphFormat> {
    let s = &app.session;
    let story = s.sel.focus.story;
    let of = |props: &ParaProps| format_of(&s.doc.styles.resolve_para(props));
    let mut each = wordcraft_engine::cmd::selected_paths(s).into_iter().filter_map(|path| s.doc.para(story, &path)).map(|p| of(&p.props));
    let first = each.next().or_else(|| s.doc.para_at(&s.sel.focus).map(|p| of(&p.props)))?;
    Some(each.fold(first, |shared, other| intersect(shared, &other)))
}

/// One paragraph's resolved formatting as the dialog's fields. Lengths are points, alignment and indents as seen on
/// the page.
fn format_of(rp: &ResolvedPara) -> ParagraphFormat {
    let mut f = ParagraphFormat::default();
    let (left, right) = rp.visual_indents();
    let first = rp.indent_first;
    f.align = Some(match rp.align.visual(rp.bidi) {
        Align::Left => ParagraphAlign::Left,
        Align::Center => ParagraphAlign::Center,
        Align::Right => ParagraphAlign::Right,
        Align::Justify => ParagraphAlign::Justify,
        Align::Distribute => ParagraphAlign::Distributed,
    });
    f.before_text = Some(left);
    f.after_text = Some(Some(right));
    f.special = Some(if first > 0.0 {
        SpecialIndent::FirstLine
    } else if first < 0.0 {
        SpecialIndent::Hanging
    } else {
        SpecialIndent::None
    });
    f.by = Some(first.abs());
    f.space_before = Some(rp.space_before);
    f.space_after = Some(rp.space_after);
    f.space_before_kind = SpacingKind::Points;
    f.space_after_kind = SpacingKind::Points;
    f.line_spacing = Some(match rp.line_spacing {
        DocSpacing::Multiple(m) => LineSpacing::Multiple(m),
        DocSpacing::AtLeast(p) => LineSpacing::AtLeast(p),
        DocSpacing::Exactly(p) => LineSpacing::Exactly(p),
    });
    f.direction = Some(Some(if rp.bidi { ParagraphDirection::RightToLeft } else { ParagraphDirection::LeftToRight }));
    f.mirror_indents = Some(Some(rp.mirror_indents));
    // PLAN: assumed ParagraphFormat::contextual_spacing (dialogs-w3, S-L289)
    f.contextual_spacing = Some(Some(rp.contextual_spacing));
    f.breaks = Some(LineBreakOptions {
        widow_control: Some(rp.widow_control),
        keep_with_next: Some(rp.keep_next),
        keep_lines_together: Some(rp.keep_lines),
        page_break_before: Some(rp.page_break_before),
        suppress_line_numbers: Some(rp.suppress_line_numbers),
        dont_hyphenate: Some(rp.suppress_hyphens),
    });
    f.asian = Some(AsianTypography {
        kinsoku: Some(rp.kinsoku),
        word_wrap: Some(rp.word_wrap),
        overflow_punct: Some(rp.overflow_punct),
        top_line_punct: Some(rp.top_line_punct),
        auto_space_de: Some(rp.auto_space_de),
        auto_space_dn: Some(rp.auto_space_dn),
    });
    f
}

/// `a` where both agree, `None` (mixed) where they differ.
fn same<T: PartialEq>(a: Option<T>, b: Option<T>) -> Option<T> {
    if a == b { a } else { None }
}

/// The format both paragraphs share: each field that differs becomes `None`.
fn intersect(mut a: ParagraphFormat, b: &ParagraphFormat) -> ParagraphFormat {
    a.align = same(a.align, b.align);
    a.before_text = same(a.before_text, b.before_text);
    a.after_text = Some(same(a.after_text.flatten(), b.after_text.flatten()));
    a.special = same(a.special, b.special);
    a.by = same(a.by, b.by);
    a.space_before = same(a.space_before, b.space_before);
    a.space_after = same(a.space_after, b.space_after);
    a.line_spacing = same(a.line_spacing, b.line_spacing);
    a.direction = Some(same(a.direction.flatten(), b.direction.flatten()));
    a.mirror_indents = Some(same(a.mirror_indents.flatten(), b.mirror_indents.flatten()));
    a.contextual_spacing = Some(same(a.contextual_spacing.flatten(), b.contextual_spacing.flatten()));
    if let (Some(x), Some(y)) = (a.breaks.as_mut(), b.breaks.as_ref()) {
        x.widow_control = same(x.widow_control, y.widow_control);
        x.keep_with_next = same(x.keep_with_next, y.keep_with_next);
        x.keep_lines_together = same(x.keep_lines_together, y.keep_lines_together);
        x.page_break_before = same(x.page_break_before, y.page_break_before);
        x.suppress_line_numbers = same(x.suppress_line_numbers, y.suppress_line_numbers);
        x.dont_hyphenate = same(x.dont_hyphenate, y.dont_hyphenate);
    }
    if let (Some(x), Some(y)) = (a.asian.as_mut(), b.asian.as_ref()) {
        x.kinsoku = same(x.kinsoku, y.kinsoku);
        x.word_wrap = same(x.word_wrap, y.word_wrap);
        x.overflow_punct = same(x.overflow_punct, y.overflow_punct);
        x.top_line_punct = same(x.top_line_punct, y.top_line_punct);
        x.auto_space_de = same(x.auto_space_de, y.auto_space_de);
        x.auto_space_dn = same(x.auto_space_dn, y.auto_space_dn);
    }
    a
}

/// The first-line indent a format stands for, points (negative: hanging); `None` while the paragraphs differ.
fn first_indent(f: &ParagraphFormat) -> Option<f32> {
    match f.special? {
        SpecialIndent::None => Some(0.0),
        SpecialIndent::FirstLine => f.by,
        SpecialIndent::Hanging => f.by.map(|by| -by),
        _ => None,
    }
}

/// The format's fields as `para.set` takes them. A field the paragraphs differ in is absent, so OK leaves it.
fn fields(f: &ParagraphFormat) -> Map<String, Value> {
    let mut m = Map::new();
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            m.insert(key.to_string(), value);
        }
    };
    // `Automatic` stands for no choice: the model stores a bool.
    put(
        "bidi",
        f.direction.flatten().and_then(|d| match d {
            ParagraphDirection::LeftToRight => Some(json!(false)),
            ParagraphDirection::RightToLeft => Some(json!(true)),
            _ => None,
        }),
    );
    put(
        "align",
        f.align.and_then(|a| {
            serde_json::to_value(match a {
                ParagraphAlign::Left => Align::Left,
                ParagraphAlign::Center => Align::Center,
                ParagraphAlign::Right => Align::Right,
                ParagraphAlign::Distributed => Align::Distribute,
                _ => Align::Justify,
            })
            .ok()
        }),
    );
    put("left", f.before_text.map(|v| json!(v)));
    put("right", f.after_text.flatten().map(|v| json!(v)));
    put("indentFirst", first_indent(f).map(|v| json!(v)));
    put("spaceBefore", f.space_before.filter(|_| f.space_before_kind == SpacingKind::Points).map(|v| json!(v)));
    put("spaceAfter", f.space_after.filter(|_| f.space_after_kind == SpacingKind::Points).map(|v| json!(v)));
    // The model's own encoding of the rule, so a fixed rule survives an unchanged OK.
    put(
        "lineSpacing",
        f.line_spacing.and_then(|l| match l {
            LineSpacing::Multiple(v) => serde_json::to_value(DocSpacing::Multiple(v)).ok(),
            LineSpacing::Exactly(v) => serde_json::to_value(DocSpacing::Exactly(v)).ok(),
            LineSpacing::AtLeast(v) => serde_json::to_value(DocSpacing::AtLeast(v)).ok(),
            _ => None,
        }),
    );
    put("mirrorIndents", f.mirror_indents.flatten().map(|v| json!(v)));
    put("contextualSpacing", f.contextual_spacing.flatten().map(|v| json!(v)));
    if let Some(b) = &f.breaks {
        put("widowControl", b.widow_control.map(|v| json!(v)));
        put("keepNext", b.keep_with_next.map(|v| json!(v)));
        put("keepLines", b.keep_lines_together.map(|v| json!(v)));
        put("pageBreakBefore", b.page_break_before.map(|v| json!(v)));
        put("suppressLineNumbers", b.suppress_line_numbers.map(|v| json!(v)));
        put("suppressHyphens", b.dont_hyphenate.map(|v| json!(v)));
    }
    if let Some(a) = &f.asian {
        put("kinsoku", a.kinsoku.map(|v| json!(v)));
        put("wordWrap", a.word_wrap.map(|v| json!(v)));
        put("overflowPunct", a.overflow_punct.map(|v| json!(v)));
        put("topLinePunct", a.top_line_punct.map(|v| json!(v)));
        put("autoSpaceDE", a.auto_space_de.map(|v| json!(v)));
        put("autoSpaceDN", a.auto_space_dn.map(|v| json!(v)));
    }
    m
}

<<<<<<< HEAD
/// OK as one `para.set`, so one undo step: the fields that differ from what every selected paragraph had when the
/// dialog opened. The alignment and indents are as seen on the page, so a new direction sends them again to keep them
/// where the dialog shows them. Nothing changed runs nothing.
fn apply_paragraph(app: &mut WordApp, new: &ParagraphFormat, start: &ParagraphFormat) -> Result<(), String> {
    let (fields, start) = (fields(new), fields(start));
    let turned = matches!((fields.get("bidi"), start.get("bidi")), (Some(a), Some(b)) if a != b);
    let (mut props, mut page) = (Map::new(), Map::new());
=======
/// The Paragraph dialog's Special box and its By value, as in Word: the first-line indent
/// `first` (in the measurement unit) is a First line indent when above 0 and a Hanging one when
/// below. Picking First line or Hanging from (none), or while the selected paragraphs differ,
/// starts it at half an inch as Word does; switching between the two keeps By.
pub(super) fn para_special(r: &mut crate::rtl::Cells, first: &mut f32, mixed: &mut BTreeSet<String>) {
    const SPECIAL: [&str; 3] = ["(none)", "First line", "Hanging"];
    let unit = crate::options::unit();
    let kind_of = |first: f32| {
        if first > 0.0 {
            1
        } else if first < 0.0 {
            2
        } else {
            0
        }
    };
    let blank = mixed.contains("indentFirst");
    let kind = kind_of(*first);
    let special_label = r.cell(|ui| ui.label(tl!("Special:")));
    let shown = if blank { "" } else { tl!(SPECIAL[kind]) };
    r.cell(|ui| {
        let combo = craft_ui::ComboBox::from_id_salt("para_special").selected_text(shown).show_ui(ui, |ui| {
            for (k, l) in SPECIAL.into_iter().enumerate() {
                if ui.selectable_label(!blank && k == kind, tl!(l)).clicked() && (blank || k != kind) {
                    let by = if blank || kind == 0 { 36.0 / unit.pt_per_unit() } else { first.abs() };
                    *first = [0.0, by, -by][k];
                    mixed.remove("indentFirst");
                }
            }
        });
        combo.response.labelled_by(special_label.id);
    });
    // A kind just picked counts from this frame on.
    let (blank, kind) = (mixed.contains("indentFirst"), kind_of(*first));
    let by_label = r.cell(|ui| ui.label(tl!("By:")));
    r.cell(|ui| {
        if blank || kind == 0 {
            // Nothing to set until Special says which way.
            let mut none = 0.0;
            ui.add_enabled(false, egui::DragValue::new(&mut none).measure_blank(unit)).labelled_by(by_label.id);
        } else {
            // Typed values count once committed: the 0 of "0.25" would otherwise turn Special
            // to (none) and disable the box mid-typing.
            let mut by = first.abs();
            // By stops at 0 when dragged down instead of turning negative for a frame (which the
            // `abs` below would flip back), and at Word's 22".
            let field = egui::DragValue::new(&mut by)
                .speed(3.6 / unit.pt_per_unit())
                .range(0.0..=1584.0 / unit.pt_per_unit())
                .clamp_existing_to_range(false)
                .max_decimals(2)
                .update_while_editing(false)
                .measure(unit);
            if ui.add(field).labelled_by(by_label.id).changed() {
                mixed.remove("indentFirst");
            }
            *first = if kind == 2 { -by.abs() } else { by.abs() };
        }
    });
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
>>>>>>> up/main
    for (k, v) in fields {
        let on_page = ON_PAGE.contains(&k.as_str());
        if start.get(&k) == Some(&v) && !(turned && on_page) {
            continue;
        }
        if on_page {
            page.insert(k, v);
        } else {
            props.insert(k, v);
        }
    }
    if props.is_empty() && page.is_empty() {
        return Ok(());
    }
    app.run("para.set", json!({"props": props, "onPage": page})).map(|_| ())
}
