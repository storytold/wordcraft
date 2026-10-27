use super::common::{BUILT_IN_LABELS, LEADERS, choice, heading, leader_char, sample};
use super::index::body_field;
use super::shell::{Next, go};
use super::*;

// ---------------------------------------------------------------------------------------------
// Table of Contents

/// The Custom Table of Contents dialog's fields (with its Options).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TocForm {
    pub page_numbers: bool,
    pub right_align: bool,
    pub leader: String,
    pub hyperlinks: bool,
    pub levels: u8,
    /// Options: heading styles (`\o`), outline levels (`\u`), TC fields (`\f`).
    pub heading_styles: bool,
    pub outline_levels: bool,
    pub tc_fields: bool,
    /// Options: other paragraph styles with a TOC level (0 = not in the table), by style name.
    pub style_levels: Vec<(String, u8)>,
}

impl TocForm {
    /// The table in the document, or the defaults for a new one.
    pub fn read(app: &WordApp) -> TocForm {
        let f = body_field(app, "TOC").unwrap_or_else(|| FieldCode::parse(r#"TOC \o "1-3" \h \z \u"#));
        let levels = f.arg('o').and_then(wordcraft_engine::cmd::references::level_range).map_or(3, |r| r.1);
        let listed: Vec<(String, u8)> = f
            .arg('t')
            .map(|t| {
                let parts: Vec<&str> = t.split(',').map(str::trim).collect();
                parts.chunks(2).filter_map(|c| Some((c.first()?.to_string(), c.get(1)?.parse::<u8>().ok()?.clamp(1, 9)))).collect()
            })
            .unwrap_or_default();
        let style_levels = app
            .session
            .doc
            .styles
            .styles
            .iter()
            .filter(|s| s.kind == StyleKind::Paragraph && !s.hidden && !s.id.starts_with("Heading") && !s.id.starts_with("TOC"))
            .map(|s| (s.name.clone(), listed.iter().find(|(n, _)| n.eq_ignore_ascii_case(&s.name)).map_or(0, |(_, l)| *l)))
            .collect();
        // Right-aligned page numbers: no `\p` separator, or a tab one.
        let leader = app
            .session
            .doc
            .body
            .iter()
            .filter_map(|b| b.as_para())
            .find(|p| p.props.style.as_deref().is_some_and(|s| s.starts_with("TOC") && s != "TOCHeading"))
            .and_then(|p| p.props.tabs.as_ref()?.last().map(|t| t.leader));
        TocForm {
            page_numbers: !f.has('n'),
            right_align: f.arg('p').is_none_or(|p| p.contains('\t')),
            leader: match leader {
                Some(wordcraft_doc::props::TabLeader::None) => "none",
                Some(wordcraft_doc::props::TabLeader::Hyphen) => "hyphen",
                Some(wordcraft_doc::props::TabLeader::Underscore) => "underscore",
                _ => "dot",
            }
            .into(),
            hyperlinks: f.has('h'),
            levels: levels.clamp(1, 9),
            heading_styles: f.has('o'),
            outline_levels: f.has('u'),
            tc_fields: f.has('f'),
            style_levels,
        }
    }

    /// `references.toc` parameters.
    pub fn params(&self) -> Value {
        let styles: serde_json::Map<String, Value> = self.style_levels.iter().filter(|(_, l)| *l > 0).map(|(n, l)| (n.clone(), json!(l))).collect();
        json!({
            "levels": self.levels,
            "pageNumbers": self.page_numbers,
            "rightAlign": self.right_align,
            "tabLeader": self.leader,
            "hyperlinks": self.hyperlinks,
            "headingStyles": self.heading_styles,
            "outlineLevels": self.outline_levels,
            "tcFields": self.tc_fields,
            "styleLevels": styles,
        })
    }
}

/// Draws the TOC preview and page-number, right-alignment, tab-leader, hyperlink and level
/// controls, with Options and Modify jumps and `references.toc` on OK.
pub(super) fn toc_ui(app: &mut WordApp, ui: &mut Ui, f: &mut TocForm) -> Next {
    let lines: Vec<(usize, String, String)> = (1..=f.levels.min(3))
        .map(|l| {
            let name = crate::i18n::fmt(tl!("Heading {n}"), &[("n", &l.to_string())]);
            (l as usize - 1, name, if f.page_numbers { (l * 2 - 1).to_string() } else { String::new() })
        })
        .collect();
    sample(ui, &lines, f.right_align, leader_char(&f.leader));
    ui.add_space(6.0);
    ui.checkbox(&mut f.page_numbers, tl!("Include page numbers"));
    ui.add_enabled_ui(f.page_numbers, |ui| {
        ui.checkbox(&mut f.right_align, tl!("Page numbers at the right margin"));
        ui.horizontal(|ui| {
            let leader_label = ui.label(tl!("Tab leader:"));
            ui.add_enabled_ui(f.right_align, |ui| {
                let opts: Vec<(String, &str)> = LEADERS.iter().map(|(v, l)| (v.to_string(), *l)).collect();
                choice(ui, "toc_leader", &mut f.leader, &opts, &leader_label);
            });
        });
    });
    ui.checkbox(&mut f.hyperlinks, tl!("Link entries to their headings"));
    ui.horizontal(|ui| {
        let levels_label = ui.label(tl!("Levels:"));
        ui.add(egui::DragValue::new(&mut f.levels).range(1..=9)).labelled_by(levels_label.id);
    });
    ui.add_space(4.0);
    let mut then = None;
    ui.horizontal(|ui| {
        if ui.button(tl!("Options…")).clicked() {
            then = Some(RefsDialog::TocOptions { was: Box::new(f.clone()), form: f.clone() });
        }
        if ui.button(tl!("Modify…")).clicked() {
            then = Some(RefsDialog::TocStyles { form: f.clone(), level: 1 });
        }
    });
    if let Some(g) = then {
        return go(g);
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok && let Err(e) = app.run("references.toc", f.params()) {
        app.status(e);
    }
    if ok || cancel { Next::Close } else { Next::Stay }
}

/// Draws the TOC sources: heading styles, per-style TOC levels, outline levels and TC entry
/// fields, with Reset. OK returns the edited form and Cancel the original.
pub(super) fn toc_options_ui(app: &mut WordApp, ui: &mut Ui, f: &mut TocForm, was: &TocForm) -> Next {
    heading(ui, "Collect entries from:");
    ui.checkbox(&mut f.heading_styles, crate::i18n::fmt(tl!("Heading styles (levels 1–{n})"), &[("n", &f.levels.to_string())]));
    ui.label(tl!("Other styles and their TOC level:"));
    // A solid scroll bar takes its own room beside the list instead of floating over the level boxes.
    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
        egui::Grid::new("toc_styles").num_columns(2).spacing(vec2(10.0, 3.0)).show(ui, |ui| {
            for (name, level) in f.style_levels.iter_mut() {
                // Built-in styles read in the interface language; the document keeps the English name.
                let shown = app.session.doc.styles.styles.iter().find(|st| st.name == *name).map_or_else(|| name.clone(), crate::i18n::style_name);
                let style_label = ui.label(shown);
                ui.add(egui::DragValue::new(level).range(0..=9).custom_formatter(|v, _| if v < 0.5 { String::new() } else { format!("{v:.0}") }))
                    .labelled_by(style_label.id);
                ui.end_row();
            }
        });
    });
    ui.checkbox(&mut f.outline_levels, tl!("Outline levels"));
    ui.checkbox(&mut f.tc_fields, tl!("TC entry fields"));
    ui.add_space(4.0);
    if ui.button(tl!("Reset")).clicked() {
        f.heading_styles = true;
        f.outline_levels = true;
        f.tc_fields = false;
        for (_, l) in f.style_levels.iter_mut() {
            *l = 0;
        }
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        return go(RefsDialog::Toc(f.clone()));
    }
    if cancel {
        return go(RefsDialog::Toc(was.clone()));
    }
    Next::Stay
}

/// Modify… in Table of Contents: pick a TOC level style and edit it in Modify Style, which
/// comes back here when it closes.
pub(super) fn toc_styles_ui(app: &mut WordApp, ui: &mut Ui, f: &mut TocForm, level: &mut u8) -> Next {
    ui.label(tl!("Styles:"));
    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
        for l in 1..=9u8 {
            let name = app.session.doc.styles.get(&format!("TOC{l}")).map(|s| s.name.clone()).unwrap_or_else(|| format!("TOC {l}"));
            ui.selectable_value(level, l, name);
        }
    });
    ui.add_space(6.0);
    if ui.button(tl!("Modify…")).clicked()
        && let Some(mut d) = Dialog::modify_style(app, &format!("TOC{level}"))
    {
        if let Dialog::ModifyStyle { resume, .. } = &mut d {
            *resume = Some(Box::new(Dialog::Refs(Box::new(RefsDialog::TocStyles { form: f.clone(), level: *level }))));
        }
        app.dialog = Some(d);
        return Next::Close;
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok || cancel { go(RefsDialog::Toc(f.clone())) } else { Next::Stay }
}

// ---------------------------------------------------------------------------------------------
// Table of Figures

/// The Table of Figures dialog's fields (#3013).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FiguresForm {
    /// The caption label whose captions are listed.
    pub label: String,
    /// Built-in labels, the user's own and any the document's captions use.
    pub labels: Vec<String>,
    pub include_label: bool,
    pub page_numbers: bool,
    pub right_align: bool,
    pub leader: String,
}

impl FiguresForm {
    pub fn read(app: &WordApp) -> FiguresForm {
        let mut labels: Vec<String> = BUILT_IN_LABELS.iter().map(ToString::to_string).collect();
        for l in app.ui.caption_labels.iter().cloned().chain(wordcraft_engine::cmd::citations::caption_labels(&app.session)) {
            if !labels.contains(&l) {
                labels.push(l);
            }
        }
        FiguresForm { label: "Figure".into(), labels, include_label: true, page_numbers: true, right_align: true, leader: "dot".into() }
    }

    /// `references.tableOfFigures` parameters.
    pub fn params(&self) -> Value {
        json!({
            "label": self.label,
            "includeLabel": self.include_label,
            "pageNumbers": self.page_numbers,
            "rightAlign": self.right_align,
            "tabLeader": self.leader,
        })
    }
}

/// Draws the table-of-figures preview and label, include-label, page-number, right-alignment and
/// tab-leader controls, running `references.tableOfFigures` on OK.
pub(super) fn figures_ui(app: &mut WordApp, ui: &mut Ui, f: &mut FiguresForm) -> Next {
    let lines: Vec<(usize, String, String)> = [(1, "1"), (2, "3")]
        .iter()
        .map(|(n, page)| {
            let text = if f.include_label { format!("{} {n}: {}", tl!(&f.label), tl!("Text")) } else { tl!("Text").to_string() };
            (0, text, if f.page_numbers { page.to_string() } else { String::new() })
        })
        .collect();
    sample(ui, &lines, f.right_align, leader_char(&f.leader));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let label_label = ui.label(tl!("Label:"));
        let labels: Vec<(String, &str)> = f.labels.iter().map(|l| (l.clone(), l.as_str())).collect();
        let combo = craft_ui::ComboBox::from_id_salt("tof_label").selected_text(tl!(&f.label)).width(150.0).show_ui(ui, |ui| {
            for (v, l) in &labels {
                ui.selectable_value(&mut f.label, v.clone(), tl!(l));
            }
        });
        combo.response.labelled_by(label_label.id);
    });
    ui.checkbox(&mut f.include_label, tl!("Include label and number"));
    ui.checkbox(&mut f.page_numbers, tl!("Include page numbers"));
    ui.add_enabled_ui(f.page_numbers, |ui| {
        ui.checkbox(&mut f.right_align, tl!("Page numbers at the right margin"));
        ui.horizontal(|ui| {
            let leader_label = ui.label(tl!("Tab leader:"));
            ui.add_enabled_ui(f.right_align, |ui| {
                let opts: Vec<(String, &str)> = LEADERS.iter().map(|(v, l)| (v.to_string(), *l)).collect();
                choice(ui, "tof_leader", &mut f.leader, &opts, &leader_label);
            });
        });
    });
    ui.add_space(4.0);
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok && let Err(e) = app.run("references.tableOfFigures", f.params()) {
        app.status(e);
    }
    if ok || cancel { Next::Close } else { Next::Stay }
}
