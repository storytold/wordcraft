use super::{Stroke, Tokens, Ui, Value, WordApp, json, regular, semibold};

/// Longest change text a card shows; the rest is elided.
const PREVIEW_CHARS: usize = 300;

/// Draws the Reviewing Pane (Review › Tracking): a summary of the tracked changes, a reviewer
/// filter with Accept All / Reject All for the changes shown, and a card per change (who, what,
/// when, the text) with Go to, Accept and Reject. Accept and Reject on a card resolve that change
/// alone (`review.accept {"change": n}`), even where it shares its text with another author's.
pub(super) fn reviewing(app: &mut WordApp, ui: &mut Ui) {
    let all = app.session.run("review.changes", &json!({})).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let comments = app
        .session
        .run("review.comments", &json!({}))
        .ok()
        .and_then(|v| v.as_array().map(|a| a.iter().filter(|c| c.get("parent").is_none_or(Value::is_null)).count()))
        .unwrap_or(0);
    let t = Tokens::get(ui.ctx());
    let author = |c: &Value| c.get("author").and_then(Value::as_str).unwrap_or("").to_string();
    let kind = |c: &Value| c.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
    // Reviewers in the order they first appear, each with a colour of the balloon palette.
    let mut authors: Vec<String> = Vec::new();
    for c in &all {
        let a = author(c);
        if !authors.contains(&a) {
            authors.push(a);
        }
    }
    let colour = |a: &str| {
        let i = authors.iter().position(|x| x == a).unwrap_or(0);
        t.author_colors.get(i % t.author_colors.len()).copied().unwrap_or(t.text)
    };

    let count = |kinds: &[&str]| all.iter().filter(|c| kinds.contains(&kind(c).as_str())).count();
    let summary = if all.len() == 1 { tl!("1 revision").to_string() } else { format!("{} {}", all.len(), tl!("revisions")) };
    ui.label(egui::RichText::new(summary).font(semibold(13.0)));
    let detail = [
        (tl!("Insertions"), count(&["insert"])),
        (tl!("Deletions"), count(&["delete"])),
        (tl!("Moves"), count(&["moveFrom", "moveTo"])),
        (tl!("Formatting"), count(&["format"])),
        (tl!("Comments"), comments),
    ]
    .iter()
    .map(|(name, n)| format!("{name}: {n}"))
    .collect::<Vec<_>>()
    .join("  ·  ");
    ui.label(egui::RichText::new(detail).small().weak());

    // Which reviewer's changes are shown; a reviewer whose changes are all gone falls back to all.
    let document = app.session.document_id();
    let filter_key = egui::Id::unique(("reviewing_pane_filter", document));
    let mut filter: Option<String> = ui.data(|d| d.get_temp(filter_key)).flatten();
    if filter.as_ref().is_some_and(|f| !authors.contains(f)) {
        filter = None;
    }
    let name = |a: &str| if a.is_empty() { tl!("Unknown author").to_string() } else { a.to_string() };
    let mut action: Option<(&'static str, Value)> = None;
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let shown = filter.as_deref().map_or_else(|| tl!("All reviewers").to_string(), name);
        let show = ui.label(tl!("Show:"));
        let combo = craft_ui::ComboBox::from_id_salt("reviewing_pane_filter").selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(&mut filter, None, tl!("All reviewers"));
            for a in &authors {
                ui.selectable_value(&mut filter, Some(a.clone()), name(a));
            }
        });
        combo.response.labelled_by(show.id);
        let params = filter.as_ref().map_or_else(|| json!({}), |a| json!({"author": a}));
        let (accept, reject) =
            if filter.is_some() { (tl!("Accept All Shown"), tl!("Reject All Shown")) } else { (tl!("Accept All"), tl!("Reject All")) };
        if ui.add_enabled(!all.is_empty(), egui::Button::new(accept).small()).clicked() {
            action = Some(("review.acceptAll", params.clone()));
        }
        if ui.add_enabled(!all.is_empty(), egui::Button::new(reject).small()).clicked() {
            action = Some(("review.rejectAll", params));
        }
    });
    ui.data_mut(|d| d.insert_temp(filter_key, filter.clone()));
    ui.separator();

    if all.is_empty() {
        ui.label(egui::RichText::new(tl!("No tracked changes.")).small().weak());
    }
    egui::ScrollArea::vertical().id_salt("reviewing_pane_list").show(ui, |ui| {
        for (i, c) in all.iter().enumerate() {
            let who = author(c);
            if filter.as_ref().is_some_and(|f| *f != who) {
                continue;
            }
            let k = kind(c);
            let what = match k.as_str() {
                "insert" => tl!("Inserted"),
                "delete" => tl!("Deleted"),
                "moveFrom" => tl!("Moved from"),
                "moveTo" => tl!("Moved to"),
                _ => tl!("Formatted"),
            };
            let when = c.get("date").and_then(Value::as_str).map(crate::local_time::revision_time).unwrap_or_default();
            egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(name(&who)).font(semibold(12.0)).color(colour(&who)));
                    ui.label(egui::RichText::new(what).font(regular(12.0)));
                    ui.label(egui::RichText::new(when).small().weak());
                });
                let text = if k == "format" {
                    let props = c.get("props").and_then(Value::as_array).cloned().unwrap_or_default();
                    crate::panes::describe_props(&props).join(", ")
                } else {
                    let raw = c.get("text").and_then(Value::as_str).unwrap_or("").replace('\n', " ¶ ");
                    let mut s: String = raw.chars().take(PREVIEW_CHARS).collect();
                    if raw.chars().count() > PREVIEW_CHARS {
                        s.push('…');
                    }
                    s
                };
                if !text.trim().is_empty() {
                    let mut rich = egui::RichText::new(text).font(regular(12.5));
                    rich = match k.as_str() {
                        "delete" | "moveFrom" => rich.strikethrough(),
                        "insert" | "moveTo" => rich.underline(),
                        _ => rich,
                    };
                    ui.label(rich);
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button(tl!("Go to")).clicked()
                        && let (Some(a), Some(b)) = (c.get("start"), c.get("end"))
                    {
                        action = Some(("select.range", json!({"anchor": a, "focus": b})));
                    }
                    if ui.small_button(tl!("Accept")).clicked() {
                        action = Some(("review.accept", json!({"change": i})));
                    }
                    if ui.small_button(tl!("Reject")).clicked() {
                        action = Some(("review.reject", json!({"change": i})));
                    }
                });
            });
            ui.add_space(6.0);
        }
    });
    // One action a frame: a resolved change shifts the indices of the ones after it.
    if let Some((id, params)) = action {
        let _ = app.run(id, params);
        app.canvas.want_focus = id == "select.range";
    }
}
