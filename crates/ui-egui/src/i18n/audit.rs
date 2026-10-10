//! Finds interface text drawn without going through translation: the app is laid out in the
//! pseudo-language (see `pseudo.rs`) in many states, and any visible word outside `⟦…⟧` marks is
//! reported. The document itself is drawn as an image, so its text never shows up here.

use std::cell::Cell;
use std::collections::BTreeMap;

use serde_json::json;

use super::pseudo;
use crate::WordApp;

thread_local! {
    // Input time, advanced every frame so tooltips and animations move on.
    static TIME: Cell<f64> = const { Cell::new(0.0) };
}

fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
        egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
        _ => {}
    }
}

/// Run a frame per entry of `steps` (its events), then a few quiet ones, and return every text
/// drawn in the last.
fn frames(ctx: &egui::Context, app: &mut WordApp, steps: Vec<Vec<egui::Event>>) -> Vec<String> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
    let mut drawn = Vec::new();
    let n = steps.len() + 3;
    let mut steps = steps.into_iter();
    for _ in 0..n {
        TIME.set(TIME.get() + 0.1);
        let input =
            egui::RawInput { screen_rect: Some(screen), time: Some(TIME.get()), events: steps.next().unwrap_or_default(), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        drawn.clear();
        for s in &out.shapes {
            texts(&s.shape, &mut drawn);
        }
        out.textures_delta.clear();
    }
    drawn
}

fn frame(ctx: &egui::Context, app: &mut WordApp) -> Vec<String> {
    frames(ctx, app, Vec::new())
}

/// Move there, press, release: one frame each, as a real click arrives.
fn click(at: egui::Pos2, button: egui::PointerButton) -> Vec<Vec<egui::Event>> {
    let press = |pressed| egui::Event::PointerButton { pos: at, button, pressed, modifiers: Default::default() };
    vec![vec![egui::Event::PointerMoved(at)], vec![press(true)], vec![press(false)]]
}

fn escape() -> Vec<Vec<egui::Event>> {
    vec![vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]]
}

/// Centres of the enabled, clickable widgets drawn last frame in the ribbon: below the tab row
/// and above the document area (the largest clickable widget).
fn ribbon_controls(ctx: &egui::Context) -> Vec<egui::Pos2> {
    ctx.viewport(|v| {
        let widgets: Vec<&egui::WidgetRect> = v
            .prev_pass
            .widgets
            .layers()
            .filter(|(layer, _)| matches!(layer.order, egui::Order::Background | egui::Order::Middle))
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.enabled && w.sense.senses_click())
            .collect();
        let page_top = widgets.iter().max_by(|a, b| a.rect.area().total_cmp(&b.rect.area())).map_or(f32::INFINITY, |w| w.rect.min.y);
        // The tab row ends where the ribbon's first group label row starts; tabs sit near y = 53.
        let tabs_bottom = 64.0;
        widgets.iter().filter(|w| w.rect.width() < 300.0).map(|w| w.rect.center()).filter(|c| c.y > tabs_bottom && c.y < page_top).collect()
    })
}

fn fresh_app(ctx: &egui::Context) -> WordApp {
    ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), Default::default());
    // Fonts install over the first frames.
    for _ in 0..2 {
        frame(ctx, &mut app);
    }
    app
}

/// Words (two or more ASCII letters in a row) drawn outside translation marks.
fn english_words(text: &str) -> bool {
    let plain = pseudo::unmarked(text);
    plain.as_bytes().windows(2).any(|w| w[0].is_ascii_alphabetic() && w[1].is_ascii_alphabetic())
}

/// Text that is right to show untranslated: what the author or the machine supplied, not the
/// interface's own wording.
struct Exempt {
    /// The document's text (headings in the Navigation pane, the title in the title bar).
    body: String,
    /// Installed font families and the document's style names (pickers and editable fields).
    names: std::collections::HashSet<String>,
}

impl Exempt {
    fn new(app: &WordApp) -> Self {
        let mut names: std::collections::HashSet<String> = wordcraft_fonts::FontDb::global().families().into_iter().collect();
        names.extend(app.session.doc.styles.styles.iter().map(|s| s.name.clone()));
        // Fonts a document asks for, installed or not (Aptos is drawn with a substitute).
        names.extend([wordcraft_doc::styles::BODY_FONT, wordcraft_doc::styles::HEADING_FONT].map(str::to_string));
        for m in crate::credits::MODELS {
            names.extend([m.company, m.model, m.version].map(str::to_string));
        }
        Exempt { body: app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), names }
    }

    fn allows(&self, app: &WordApp, text: &str) -> bool {
        let plain = pseudo::unmarked(text);
        let plain = plain.trim().trim_end_matches(" —").trim();
        let is_shortcut =
            || plain.split('+').all(|k| matches!(k, "Ctrl" | "Shift" | "Alt" | "Cmd" | "Esc" | "Tab" | "Enter") || k.chars().count() <= 3);
        // A command's error message in the status bar: commands answer scripts and MCP too, so
        // their messages stay English for now.
        let status = app.status_msg.iter().map(|(m, _)| m).chain(&app.read_aloud_error).any(|m| text.contains(m.as_str()));
        // A theme's fonts after its (translated) name: `Studio: Georgia / Georgia`.
        let theme_fonts = plain.strip_prefix(": ").is_some_and(|f| f.split(" / ").count() == 2);
        let icon_lettering = matches!(plain, "Aa" | "ab" | "abc" | "ac" | "cd" | "ab-" | "fx" | "ABC");
        let initials = plain.chars().count() == 2 && plain.chars().all(|c| c.is_ascii_uppercase());
        icon_lettering // drawn inside icons
            || initials // the account button
            || status
            || theme_fonts
            || is_shortcut()
            || self.body.contains(plain)
            || self.names.contains(plain)
            || plain.starts_with('@') // contributor handles
            || plain.starts_with("Style") || plain.starts_with("Table Style ") // a new style's default name
            || plain.contains('┬') || plain.contains('\u{2061}') // equation input (UnicodeMath), drawn as math
            || matches!(plain, "WordCraft" | "WordCraft User" | "Discord" | "https://" | "auto" | "I. II. III." | "i. ii. iii.")
    }
}

/// Lists untranslated interface text per UI state. `cargo test -p wordcraft-ui-egui
/// untranslated_interface_text -- --nocapture` prints the report.
#[test]
fn untranslated_interface_text() {
    pseudo::set(true);
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut record = |app: &WordApp, state: &str, drawn: Vec<String>| {
        let exempt = Exempt::new(app);
        for t in drawn.into_iter().filter(|t| english_words(t) && !exempt.allows(app, t)) {
            found.entry(t).or_default().push(state.to_string());
        }
    };
    let ctx = egui::Context::default();
    let mut app = fresh_app(&ctx);
    let tabs: Vec<&str> = crate::ribbon::TABS.iter().copied().filter(|t| *t != "File").collect();
    for tab in &tabs {
        let _ = app.run("ui.tab", json!({"tab": tab}));
        let drawn = frame(&ctx, &mut app);
        record(&app, &format!("tab {tab}"), drawn);
    }
    let _ = app.run("ui.tab", json!({"tab": "Home"}));
    for page in ["home", "new", "open", "info", "export", "options"] {
        let _ = app.run("ui.backstage", json!({"value": true, "page": page}));
        let drawn = frame(&ctx, &mut app);
        record(&app, &format!("File › {page}"), drawn);
    }
    let _ = app.run("ui.backstage", json!({"value": false}));
    let dialogs = [
        "font",
        "paragraph",
        "goto",
        "insertTable",
        "pageSetup",
        "link",
        "bookmark",
        "wordCount",
        "zoom",
        "watermark",
        "newStyle",
        "newTableStyle",
        "modifyTableStyle",
        "commands",
        "tableProperties",
        "pasteSpecial",
        "about",
        "contributors",
        "models",
        "insertMergeField",
        "findRecipient",
    ];
    for name in dialogs {
        let _ = app.run("ui.dialog", json!({"name": name}));
        let drawn = frame(&ctx, &mut app);
        record(&app, &format!("dialog {name}"), drawn);
        app.dialog = None;
    }
    let panes: Vec<String> =
        app.session.registry.all().iter().map(|c| c.id.to_string()).filter(|id| id.starts_with("view.") && id.ends_with("Pane")).collect();
    for pane in &panes {
        let _ = app.run(pane, json!({"value": true}));
        let drawn = frame(&ctx, &mut app);
        record(&app, &format!("pane {pane}"), drawn);
        let _ = app.run(pane, json!({"value": false}));
    }
    // Right-click on the page.
    let drawn = frames(&ctx, &mut app, click(egui::pos2(720.0, 520.0), egui::PointerButton::Secondary));
    record(&app, "context menu", drawn);
    frames(&ctx, &mut app, escape());
    // Contextual tabs: a table, then an equation.
    let _ = app.run("insert.table", json!({"rows": 2, "cols": 2}));
    for tab in ["Table Design", "Table Layout"] {
        let _ = app.run("ui.tab", json!({"tab": tab}));
        let drawn = frame(&ctx, &mut app);
        record(&app, &format!("tab {tab}"), drawn);
    }
    let _ = app.run("insert.equation", json!({}));
    let _ = app.run("ui.tab", json!({"tab": "Equation"}));
    let drawn = frame(&ctx, &mut app);
    record(&app, "tab Equation", drawn);
    // Every ribbon control on every tab, and whatever menu or dialog it opens. A fresh app per
    // tab keeps one tab's commands from changing the next.
    // A fresh app per control, set up the same way, so no click changes what the next one hits
    // (Escape, for one, leaves an equation and takes the Equation tab with it).
    let contextual = ["Table Design", "Table Layout", "Equation"];
    let open = |tab: &str| {
        let ctx = egui::Context::default();
        let mut app = fresh_app(&ctx);
        match tab {
            "Table Design" | "Table Layout" => drop(app.run("insert.table", json!({"rows": 2, "cols": 2}))),
            "Equation" => drop(app.run("insert.equation", json!({}))),
            _ => {}
        }
        let _ = app.run("ui.tab", json!({"tab": tab}));
        frame(&ctx, &mut app);
        (ctx, app)
    };
    for tab in tabs.iter().chain(&contextual) {
        let (ctx, _) = open(tab);
        let controls = ribbon_controls(&ctx);
        for at in controls {
            let (ctx, mut app) = open(tab);
            // Hover first (tooltips), then click (menus, dialogs).
            let hovered = frames(&ctx, &mut app, vec![vec![egui::Event::PointerMoved(at)]; 3]);
            record(&app, &format!("{tab} › hover"), hovered);
            let drawn = frames(&ctx, &mut app, click(at, egui::PointerButton::Primary));
            record(&app, &format!("{tab} › click"), drawn);
        }
    }
    pseudo::set(false);
    let mut report = String::new();
    for (text, states) in &found {
        let mut states = states.clone();
        states.sort();
        states.dedup();
        report.push_str(&format!("{:?}  [{}]\n", pseudo::unmarked(text), states.join(", ")));
    }
    assert!(found.is_empty(), "{} interface texts skip translation (wrap them in tl!):\n{report}", found.len());
}
