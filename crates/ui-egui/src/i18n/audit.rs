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
    // Process-wide names (font families, the custom dictionary) seen at any point of the run:
    // tests running alongside add and remove them, so one may be drawn and gone by the check.
    static GLOBAL_NAMES: std::cell::RefCell<std::collections::HashSet<String>> = std::cell::RefCell::new(Default::default());
}

/// Remember the process-wide names drawn this frame could have come from.
fn note_global_names() {
    GLOBAL_NAMES.with_borrow_mut(|seen| {
        seen.extend(wordcraft_fonts::FontDb::global().families());
        seen.extend(wordcraft_engine::proof::user_dictionary());
    });
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
    // Wide enough that no ribbon tab overflows behind its scroll arrows.
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(2560.0, 900.0));
    let mut drawn = Vec::new();
    let n = steps.len() + 3;
    let mut steps = steps.into_iter();
    for _ in 0..n {
        TIME.set(TIME.get() + 0.1);
        let input =
            egui::RawInput { screen_rect: Some(screen), time: Some(TIME.get()), events: steps.next().unwrap_or_default(), ..Default::default() };
        note_global_names();
        let mut out = ctx.run_ui(input, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        note_global_names();
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
    // Read Aloud holds each sentence instead of starting the system's speech engine: no sound on
    // machines that have one, and no worker thread failing at its own pace on machines that don't.
    app.session.read_aloud.backend = wordcraft_engine::speech::Backend::Hold;
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
        // Installed fonts, and the custom dictionary's words (File › Options › Proofing): the user's own.
        note_global_names();
        let mut names: std::collections::HashSet<String> = GLOBAL_NAMES.with_borrow(Clone::clone);
        names.extend(app.session.doc.styles.styles.iter().map(|s| s.name.clone()));
        // Fonts a document asks for, installed or not (Aptos is drawn with a substitute).
        names.extend([wordcraft_doc::styles::BODY_FONT, wordcraft_doc::styles::HEADING_FONT].map(str::to_string));
        // The selected SmartArt graphic's items (the Text Pane): document content.
        if let Some(g) = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session) {
            names.extend(g.items.iter().map(|i| i.text.clone()));
        }
        // The accessibility checker's findings: like command messages, they answer scripts and
        // MCP too, so they stay English for now.
        for i in wordcraft_engine::cmd::accessibility::check(&app.session.doc) {
            names.extend(["issue", "fix"].iter().filter_map(|k| i[*k].as_str()).map(str::to_string));
        }
        // The selected chart's data (Edit Data's grid): document content.
        if let Some(c) = wordcraft_engine::cmd::chart::selected_chart(&app.session) {
            names.extend(c.categories.iter().cloned());
            names.extend(c.series.iter().map(|s| s.name.clone()));
        }
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
            || plain == "CONFIDENTIAL" // the watermark dialog's text: document content, kept as typed
            || plain == "Figure 1" // the caption dialog's sample: the label goes into the document as is
    }
}

/// Lists untranslated interface text per UI state. `cargo test -p wordcraft-ui-egui
/// untranslated_interface_text -- --nocapture` prints the report. It runs with the other tests:
/// input is synthetic and timed by `TIME`, Read Aloud never starts a speech thread, and
/// process-wide names other tests change are gathered every frame.
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
    for pane in wordcraft_engine::cmd::options::PANES {
        let _ = app.run("ui.backstage", json!({"value": true, "page": "options", "pane": pane}));
        let drawn = frame(&ctx, &mut app);
        record(&app, &format!("File › Options › {pane}"), drawn);
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
        "caption",
        "index",
        "markEntry",
        "toc",
        "sourceManager",
        "createSource",
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
    // Right-click on the page (centred in the window).
    let drawn = frames(&ctx, &mut app, click(egui::pos2(1280.0, 520.0), egui::PointerButton::Secondary));
    record(&app, "context menu", drawn);
    frames(&ctx, &mut app, escape());
    // Contextual tabs: a table, then an equation.
    let _ = app.run("insert.table", json!({"rows": 2, "cols": 2}));
    for tab in ["Table Design", "Table Layout"] {
        let _ = app.run("ui.tab", json!({"tab": tab}));
        let drawn = frame(&ctx, &mut app);
        record(&app, &format!("tab {tab}"), drawn);
    }
    let _ = app.run("insert.smartArt", json!({"layout": "hierarchy"}));
    let _ = app.run("smartArt.textPane", json!({"value": true}));
    let _ = app.run("ui.tab", json!({"tab": "SmartArt Design"}));
    let drawn = frame(&ctx, &mut app);
    record(&app, "tab SmartArt Design", drawn);
    let _ = app.run("smartArt.textPane", json!({"value": false}));
    let _ = app.run("ui.dialog", json!({"name": "insertSmartArt"}));
    let drawn = frame(&ctx, &mut app);
    record(&app, "dialog insertSmartArt", drawn);
    app.dialog = None;
    let _ = app.run("insert.chart", json!({"type": "column"}));
    let _ = app.run("ui.tab", json!({"tab": "Chart Design"}));
    let drawn = frame(&ctx, &mut app);
    record(&app, "tab Chart Design", drawn);
    for name in ["chartData", "changeChartType", "insertChart"] {
        let _ = app.run("ui.dialog", json!({"name": name}));
        let drawn = frame(&ctx, &mut app);
        record(&app, &format!("dialog {name}"), drawn);
        app.dialog = None;
    }
    let _ = app.run("insert.equation", json!({}));
    let _ = app.run("ui.tab", json!({"tab": "Equation"}));
    let drawn = frame(&ctx, &mut app);
    record(&app, "tab Equation", drawn);
    // Every ribbon control on every tab, and whatever menu or dialog it opens. A fresh app per
    // tab keeps one tab's commands from changing the next.
    // A fresh app per control, set up the same way, so no click changes what the next one hits
    // (Escape, for one, leaves an equation and takes the Equation tab with it).
    let contextual = ["Table Design", "Table Layout", "Equation", "Chart Design", "SmartArt Design"];
    let open = |tab: &str| {
        let ctx = egui::Context::default();
        let mut app = fresh_app(&ctx);
        match tab {
            "Table Design" | "Table Layout" => drop(app.run("insert.table", json!({"rows": 2, "cols": 2}))),
            "Equation" => drop(app.run("insert.equation", json!({}))),
            "SmartArt Design" => drop(app.run("insert.smartArt", json!({"layout": "process"}))),
            "Chart Design" => drop(app.run("insert.chart", json!({"type": "column"}))),
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
    complete_catalogs_have_every_string_the_interface_looked_up(pseudo::looked_up());
}

/// Russian, Hungarian, Turkish, Bulgarian, Italian, Swedish and Croatian cover everything the audit saw
/// translated, plus the equation gallery's labels (shown in menus the audit doesn't open one by
/// one). Numbered style names go through a template.
fn complete_catalogs_have_every_string_the_interface_looked_up(mut keys: std::collections::BTreeSet<String>) {
    use wordcraft_engine::math_gallery::{BUILT_INS, STRUCTURES, SYMBOL_SETS};
    keys.extend(BUILT_INS.iter().map(|(_, label, _)| label.to_string()));
    keys.extend(SYMBOL_SETS.iter().map(|(name, _)| name.to_string()));
    for g in STRUCTURES {
        keys.insert(g.label.to_string());
        for (section, templates) in g.sections {
            keys.insert(section.to_string());
            keys.extend(templates.iter().map(|t| t.label.to_string()));
        }
    }
    let numbered = |s: &str| ["Heading ", "TOC "].iter().any(|p| s.strip_prefix(p).is_some_and(|n| n.chars().all(|c| c.is_ascii_digit())));
    // Names that read the same in these languages (citation style names included).
    let same = |s: &str| {
        matches!(
            s,
            "LaTeX"
                | "LaTeX (*.tex)"
                | "Unicode"
                | "I. II. III."
                | "i. ii. iii."
                | "APA"
                | "Chicago"
                | "GB/T 7714"
                | "GOST (name sort)"
                | "GOST (title sort)"
                | "Harvard (Anglia)"
                | "IEEE"
                | "ISO 690 (author-date)"
                | "ISO 690 (numerical)"
                | "MLA"
                | "SIST02"
                | "Turabian"
        )
    };
    // A command called with parameters it can't use (`bad parameters: \`path\` (string) is
    // required`): the message names JSON parameters for scripts and MCP, so it stays English like
    // the rest of the engine's errors; the status bar translates only the ones a catalog knows.
    let script_error = |s: &str| s.starts_with("bad parameters: ");
    // A pattern of placeholders and figures only (`{paper}   {width} × {height}`) has nothing to
    // translate: its placeholder names aren't words the interface shows.
    let words = |s: &str| english_words(&s.split('{').map(|p| p.split_once('}').map_or(p, |(_, rest)| rest)).collect::<String>());
    let mut report = String::new();
    for code in ["ru", "hu", "tr", "bg", "it", "sv", "hr"] {
        let lang = super::Lang::from_code(code).unwrap_or(super::Lang::EN);
        let missing: Vec<&String> =
            keys.iter().filter(|s| words(s) && !super::has(lang, s) && !numbered(s) && !same(s) && !script_error(s)).collect();
        if !missing.is_empty() {
            report.push_str(&format!("{} strings have no {code} entry:\n{missing:#?}\n", missing.len()));
        }
    }
    assert!(report.is_empty(), "{report}");
}

/// The text from `open` (the first char of `src`) to its matching `close`, skipping string
/// literals (reasons hold brackets: `(Shift+click)`).
fn balanced(src: &str, open: char, close: char) -> &str {
    let (mut depth, mut in_str, mut escaped) = (0usize, false, false);
    for (i, c) in src.char_indices() {
        match (in_str, escaped, c) {
            (true, true, _) => escaped = false,
            (true, false, '\\') => escaped = true,
            (true, false, '"') | (false, _, '"') => in_str = !in_str,
            (false, _, c) if c == open => depth += 1,
            (false, _, c) if c == close => {
                depth -= 1;
                if depth == 0 {
                    return &src[..i + c.len_utf8()];
                }
            }
            _ => {}
        }
    }
    src
}

/// The string literals in `code`.
fn literals(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = code;
    while let Some(start) = rest.find('"') {
        let body = &rest[start + 1..];
        let mut escaped = false;
        let end = body.char_indices().find(|&(_, c)| {
            let ends = !escaped && c == '"';
            escaped = !escaped && c == '\\';
            ends
        });
        let Some((end, _)) = end else { break };
        out.push(body[..end].replace("\\'", "'").replace("\\\"", "\""));
        rest = &body[end + 1..];
    }
    out
}

/// Every reason a command gives for being off (its `enabled` check), which ribbon tooltips show
/// (#1086). Read from the engine's command sources so a new reason can't slip by: the literals in
/// each `.when(…)` closure and in the checks it names (and the checks those call), with `const`
/// reasons resolved.
fn disabled_reasons() -> std::collections::BTreeSet<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../engine/src/cmd");
    let mut files: Vec<_> =
        std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "rs")).collect();
    files.sort();
    let all: String = files.iter().map(|p| std::fs::read_to_string(p).unwrap()).collect::<Vec<_>>().join("\n");
    // A named check's bodies (any file's `fn name(…) -> Option<&'static str>`).
    let checks = |name: &str| -> Vec<&str> {
        all.match_indices(&format!("fn {name}("))
            .filter_map(|(i, _)| {
                let from = &all[i..];
                let brace = from.find('{')?;
                from[..brace].contains("-> Option<&'static str>").then(|| balanced(&from[brace..], '{', '}'))
            })
            .collect()
    };
    let is_ident = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':');
    let mut bodies: Vec<&str> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for (i, _) in all.match_indices(".when(") {
        let args = balanced(&all[i + ".when".len()..], '(', ')');
        let inner = args.trim_start_matches('(').trim_end_matches(')').trim();
        if is_ident(inner) {
            names.push(inner.rsplit("::").next().unwrap_or(inner).to_string());
        } else {
            bodies.push(inner);
        }
    }
    let mut seen = std::collections::HashSet::new();
    while let Some(name) = names.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        for body in checks(&name) {
            bodies.push(body);
            // The checks this one calls (`has_object(s)`).
            for (j, _) in body.match_indices("(s)") {
                let callee = body[..j].rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or("");
                if !callee.is_empty() {
                    names.push(callee.to_string());
                }
            }
        }
    }
    let mut reasons = std::collections::BTreeSet::new();
    for body in bodies {
        // `cfg!(target_arch = "wasm32")` names a target, not a reason.
        let mut code = body.to_string();
        while let Some(i) = code.find("cfg!(") {
            let len = balanced(&code[i + "cfg!".len()..], '(', ')').len();
            code.replace_range(i..i + "cfg!".len() + len, "");
        }
        reasons.extend(literals(&code));
        // `const FROZEN: &str = "…"` reasons.
        for word in body.split(|c: char| !(c.is_ascii_uppercase() || c == '_')).filter(|w| w.len() > 2) {
            if let Some(i) = all.find(&format!("const {word}: &str = ")) {
                reasons.extend(literals(all[i..].lines().next().unwrap_or("")));
            }
        }
    }
    reasons
}

/// Ribbon tooltips show why a control is off in the interface's language: every reason has an
/// entry in every catalog (#2678).
#[test]
fn every_disabled_reason_is_translated() {
    let reasons = disabled_reasons();
    // The source scan finds every reason the registry gives in a blank and the sample document.
    for doc in [wordcraft_doc::Document::new(), wordcraft_engine::sample::sample_document()] {
        let session = wordcraft_engine::Session::new(doc);
        for spec in session.registry.all() {
            if let Some(r) = (spec.enabled)(&session) {
                assert!(reasons.contains(r), "{}: the source scan missed its reason {r:?}", spec.id);
            }
        }
    }
    assert!(reasons.len() > 20, "the source scan found only {reasons:#?}");
    let mut missing = Vec::new();
    for l in super::Lang::all().filter(|l| *l != super::Lang::EN) {
        missing.extend(reasons.iter().filter(|r| !super::has(l, r)).map(|r| format!("{}: {r:?}", l.code())));
    }
    assert!(missing.is_empty(), "{} disabled reasons are untranslated:\n{missing:#?}", missing.len());
}
