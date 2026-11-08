//! The macOS menu bar: WordCraft, File, Edit, View, Go, Insert, Format, Tools, Table, Window and Help,
//! as native menus, the way Word for Mac lays them out.
//!
//! Every item runs a registered command through `WordApp::run`, the path the ribbon and the
//! keyboard take (so "Save changes?" prompts, input dialogs and clipboard handling all apply), or,
//! for Cut / Copy / Paste / Select All, feeds egui the input events the keyboard would have. Items
//! expose native key equivalents for common commands. Editing actions feed the focused egui
//! widget, so Undo and clipboard shortcuts also work in search fields and dialogs.
//!
//! Quit routes through `Action::Quit` -> `WordApp::quit_requested` so unsaved changes prompt the user
//! before exiting, protecting against accidental closure on Command-Q (#4295).
//!
//! The item list is plain data (tested on every platform against the registry); the AppKit part
//! is macOS only.

/// One item of a menu.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub(crate) enum Entry {
    /// A registered command, run with no parameters, under its registry label.
    Cmd(&'static str),
    /// A command with parameters (JSON) under its own label.
    With(&'static str, &'static str, &'static str),
    /// Keyboard-equivalent input for egui.
    Input(InputKind, &'static str),
    /// File ▸ Open Recent.
    Recent,
    Sep,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum InputKind {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

use Entry::{Cmd, Input, Recent, Sep, With};

/// A dialog the ribbon opens (`ui.dialog`), under its ribbon label.
macro_rules! dialog {
    ($name:literal, $label:literal) => {
        With("ui.dialog", concat!(r#"{"name":""#, $name, r#""}"#), $label)
    };
}

/// A File (Backstage) page, as the ribbon's File tab opens it.
macro_rules! backstage {
    ($page:literal, $label:literal) => {
        With("ui.backstage", concat!(r#"{"value":true,"page":""#, $page, r#""}"#), $label)
    };
}

// Every item runs the way the ribbon (or the keyboard) runs it: commands that need input go
// through the ribbon's own dialog or Backstage page, never with empty parameters.
pub(crate) const FILE: &[Entry] = &[
    Cmd("file.new"),
    backstage!("new", "New from Template…"),
    Cmd("file.open"),
    Recent,
    Sep,
    Cmd("file.close"),
    Sep,
    Cmd("file.save"),
    Cmd("file.saveAs"),
    Sep,
    backstage!("export", "Export…"),
    backstage!("info", "Info"),
    Sep,
    Cmd("file.print"),
];

pub(crate) const EDIT: &[Entry] = &[
    Input(InputKind::Undo, "Undo"),
    Input(InputKind::Redo, "Redo"),
    Cmd("edit.repeat"),
    Sep,
    Input(InputKind::Cut, "Cut"),
    Input(InputKind::Copy, "Copy"),
    Input(InputKind::Paste, "Paste"),
    Cmd("edit.pasteSpecial"),
    Cmd("edit.pasteText"),
    Cmd("edit.pasteMerge"),
    Input(InputKind::SelectAll, "Select All"),
    Sep,
    Cmd("edit.formatPainter"),
    Sep,
    Cmd("edit.find"),
    dialog!("replace", "Replace…"),
    Cmd("edit.advancedFind"),
    Cmd("edit.findNext"),
    Cmd("edit.findPrevious"),
];

pub(crate) const VIEW: &[Entry] = &[
    Cmd("view.draft"),
    Cmd("view.webLayout"),
    Cmd("view.printLayout"),
    Cmd("view.outline"),
    Cmd("view.readMode"),
    Cmd("view.focus"),
    Cmd("view.immersive"),
    Sep,
    Cmd("view.ruler"),
    Cmd("view.gridlines"),
    Cmd("view.navigationPane"),
    Cmd("view.marks"),
    Cmd("view.hideWhiteSpace"),
    Sep,
    Cmd("view.zoom100"),
    Cmd("view.zoomIn"),
    Cmd("view.zoomOut"),
    Cmd("view.onePage"),
    Cmd("view.multiplePages"),
    Cmd("view.pageWidth"),
    dialog!("zoom", "Zoom…"),
    Sep,
    Cmd("review.changes"),
    Sep,
    Cmd("view.darkMode"),
    Cmd("view.split"),
    With("ui.collapseRibbon", "{}", "Collapse Ribbon"),
];

pub(crate) const GO: &[Entry] = &[
    Cmd("caret.docStart"),
    Cmd("caret.docEnd"),
    Cmd("caret.pageUp"),
    Cmd("caret.pageDown"),
    Cmd("caret.paraUp"),
    Cmd("caret.paraDown"),
    Sep,
    dialog!("goto", "Go To…"),
    Cmd("edit.goBack"),
    Sep,
    Cmd("review.nextComment"),
    Cmd("review.previousComment"),
    Cmd("review.nextChange"),
    Cmd("review.previousChange"),
    Cmd("references.nextFootnote"),
];

pub(crate) const INSERT: &[Entry] = &[
    Cmd("insert.pageBreak"),
    Cmd("insert.blankPage"),
    Cmd("insert.coverPage"),
    Sep,
    dialog!("insertTable", "Insert Table…"),
    Cmd("insert.picture"),
    Cmd("insert.icon"),
    Cmd("insert.smartArt"),
    Cmd("insert.chart"),
    Sep,
    dialog!("link", "Link"),
    dialog!("bookmark", "Bookmark"),
    Cmd("insert.crossReference"),
    Cmd("review.newComment"),
    Sep,
    Cmd("insert.header"),
    Cmd("insert.footer"),
    Cmd("insert.pageNumber"),
    Sep,
    Cmd("insert.textBox"),
    Cmd("insert.dropCap"),
    Cmd("insert.dateTime"),
    dialog!("field", "Field…"),
    Cmd("insert.object"),
    Sep,
    Cmd("insert.equation"),
    dialog!("symbol", "Symbol"),
    Sep,
    Cmd("references.footnote"),
    Cmd("references.endnote"),
    Cmd("references.caption"),
    dialog!("toc", "Custom Table of Contents…"),
    Cmd("references.citation"),
    Cmd("references.bibliography"),
];

pub(crate) const FORMAT: &[Entry] = &[
    dialog!("font", "Font"),
    dialog!("paragraph", "Paragraph"),
    Sep,
    Cmd("format.bold"),
    Cmd("format.italic"),
    Cmd("format.underline"),
    Cmd("format.strikethrough"),
    Cmd("format.subscript"),
    Cmd("format.superscript"),
    Cmd("format.clear"),
    Sep,
    Cmd("para.alignLeft"),
    Cmd("para.alignCenter"),
    Cmd("para.alignRight"),
    Cmd("para.justify"),
    Sep,
    Cmd("para.bullets"),
    Cmd("para.numbering"),
    Cmd("para.indent"),
    Cmd("para.outdent"),
    Cmd("para.tabs"),
    Cmd("para.borders"),
    Sep,
    dialog!("newStyle", "Create a Style"),
    dialog!("manageStyles", "Manage Styles"),
    Sep,
    dialog!("watermark", "Custom Watermark…"),
    Cmd("design.pageBorders"),
    dialog!("pageSetup", "Page Setup"),
];

pub(crate) const TOOLS: &[Entry] = &[
    Cmd("review.spelling"),
    Cmd("review.thesaurus"),
    dialog!("wordCount", "Word Count"),
    Cmd("review.language"),
    Cmd("tools.autocorrect"),
    Sep,
    Cmd("review.trackChanges"),
    Cmd("review.acceptAll"),
    Cmd("review.rejectAll"),
    Cmd("review.compare"),
    Sep,
    Cmd("review.readAloud"),
    Cmd("file.accessibility"),
    Sep,
    Cmd("tools.macros"),
    Cmd("tools.templates"),
    dialog!("customizeKeyboard", "Customize Keyboard"),
    dialog!("customizeRibbon", "Customize Ribbon"),
];

pub(crate) const TABLE: &[Entry] = &[
    Cmd("table.draw"),
    Sep,
    dialog!("insertTable", "Insert Table…"),
    Cmd("table.insertRowAbove"),
    Cmd("table.insertRowBelow"),
    Cmd("table.insertColumnLeft"),
    Cmd("table.insertColumnRight"),
    Sep,
    Cmd("table.deleteTable"),
    Cmd("table.deleteRow"),
    Cmd("table.deleteColumn"),
    Cmd("table.deleteCells"),
    Sep,
    Cmd("table.selectTable"),
    Cmd("table.selectRow"),
    Cmd("table.selectColumn"),
    Cmd("table.selectCell"),
    Sep,
    Cmd("table.merge"),
    Cmd("table.split"),
    Cmd("table.splitTable"),
    Sep,
    Cmd("table.fromText"),
    Cmd("table.toText"),
    Sep,
    Cmd("table.properties"),
];

pub(crate) const HELP: &[Entry] = &[Cmd("tools.search")];

pub(crate) const ABOUT: Entry = With("ui.dialog", r#"{"name":"about"}"#, "About WordCraft");
pub(crate) const SETTINGS: Entry = With("ui.backstage", r#"{"value":true,"page":"options"}"#, "Settings…");

/// The menus after the application menu, with their (catalog) titles.
pub(crate) const MENUS: [(&str, &[Entry]); 10] = [
    ("File", FILE),
    ("Edit", EDIT),
    ("View", VIEW),
    ("Go", GO),
    ("Insert", INSERT),
    ("Format", FORMAT),
    ("Tools", TOOLS),
    ("Table", TABLE),
    ("Window", &[]),
    ("Help", HELP),
];

#[cfg(target_os = "macos")]
pub(crate) use bar::NativeMenu;

#[cfg(target_os = "macos")]
mod bar {
    use std::str::FromStr;
    use std::sync::mpsc::{Receiver, Sender, channel};

    use muda::accelerator::Accelerator;
    use muda::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use serde_json::{Value, json};
    use wordcraft_ui_egui::{WordApp, i18n, widgets};

    use super::{ABOUT, Entry, InputKind, MENUS, SETTINGS};

    #[derive(Clone, Debug)]
    enum Action {
        Run(&'static str, Value),
        Input(InputKind),
        Open(String),
        /// Standard macOS Hide (⌘H).
        Hide,
        /// Standard macOS Minimize (⌘M).
        Minimize,
        /// WordCraft ▸ Quit WordCraft, with save check before exit (#4295).
        Quit,
    }

    struct Tracked {
        item: MenuItem,
        action: Action,
        accelerator: Option<Accelerator>,
        enabled: bool,
    }

    pub(crate) struct NativeMenu {
        /// Kept alive: AppKit holds the menu only while muda's handles exist.
        _menu: Menu,
        _submenus: Vec<Submenu>,
        events: Receiver<MenuEvent>,
        actions: Vec<(muda::MenuId, Action)>,
        tracked: Vec<Tracked>,
        recent: Option<Submenu>,
        recent_paths: Option<Vec<String>>,
        seq: u64,
        /// The interface language (its code) the labels are in.
        lang: &'static str,
        /// Input events for the next frame's `raw_input_hook`.
        pub(crate) input: Vec<egui::Event>,
    }

    /// Labels only the menu bar uses, in Italian (the catalogs have the rest; other languages
    /// fall back to English for these few).
    const EXTRA_IT: &[(&str, &str)] = &[
        ("Services", "Servizi"),
        ("Hide WordCraft", "Nascondi WordCraft"),
        ("Hide Others", "Nascondi altre"),
        ("Show All", "Mostra tutte"),
        ("Quit WordCraft", "Esci da WordCraft"),
        ("Minimize", "Riduci a icona"),
        ("Zoom", "Ingrandisci"),
        ("Enter Full Screen", "Attiva modalità a tutto schermo"),
        ("Bring All to Front", "Porta tutto in primo piano"),
        ("Edit", "Modifica"),
        ("Go", "Vai"),
        ("Open Recent", "Apri recenti"),
        ("About WordCraft", "Informazioni su WordCraft"),
        ("New from Template…", "Nuovo da modello…"),
        ("Export…", "Esporta…"),
        ("Table", "Tabella"),
    ];

    /// A translated label.
    fn t(s: &str) -> &str {
        if i18n::current().code() == "it"
            && let Some((_, it)) = EXTRA_IT.iter().find(|(en, _)| *en == s)
        {
            return it;
        }
        i18n::t(s)
    }

    /// Append an item, reporting backend failures without closing the editor.
    fn append(menu: &Submenu, item: &dyn IsMenuItem) {
        if let Err(e) = menu.append(item) {
            log::warn!("native menu: {e}");
        }
    }

    /// Resolve a command's translated label, retaining its id as a fallback.
    fn label(id: &str, app: &WordApp) -> String {
        app.session.registry.get(id).map_or_else(|| id.to_owned(), |s| t(s.label).to_owned())
    }

    impl NativeMenu {
        /// Allocate a stable event identifier within this menu installation.
        fn next_id(&mut self) -> String {
            self.seq = self.seq.saturating_add(1);
            format!("wordcraft.{}", self.seq)
        }

        /// Add an action and track its enablement and configurable shortcut.
        fn add(&mut self, menu: &Submenu, text: &str, action: Action, app: &WordApp) {
            let accelerator = accelerator(&action, app);
            let enabled = action_enabled(&action, app, false);
            let item = MenuItem::with_id(self.next_id(), text, enabled, accelerator);
            self.actions.push((item.id().clone(), action.clone()));
            self.tracked.push(Tracked { item: item.clone(), action, enabled, accelerator });
            append(menu, &item);
        }

        /// Materialize a declarative menu entry using the current command registry.
        fn entry(&mut self, menu: &Submenu, e: Entry, app: &WordApp) {
            match e {
                Entry::Cmd(id) => {
                    // A command this version doesn't have is left out.
                    if app.session.registry.get(id).is_some() || id.starts_with("ui.") {
                        self.add(menu, &label(id, app), Action::Run(id, json!({})), app);
                    }
                }
                Entry::With(id, params, text) => {
                    let params = serde_json::from_str(params).unwrap_or_else(|_| json!({}));
                    self.add(menu, t(text), Action::Run(id, params), app);
                }
                Entry::Input(kind, text) => self.add(menu, t(text), Action::Input(kind), app),
                Entry::Recent => {
                    let sub = Submenu::new(t("Open Recent"), false);
                    append(menu, &sub);
                    self.recent = Some(sub);
                }
                Entry::Sep => append(menu, &PredefinedMenuItem::separator()),
            }
        }

        /// Build the menu bar and install it. `None` off the main thread. `events` is the previous
        /// bar's channel: muda takes the click handler only once per process.
        fn install(app: &WordApp, ctx: &egui::Context, events: Option<Receiver<MenuEvent>>) -> Option<Self> {
            objc2::MainThreadMarker::new()?;
            let events = events.unwrap_or_else(|| {
                let (tx, rx) = channel::<MenuEvent>();
                set_handler(tx, ctx.clone());
                rx
            });
            let menu = Menu::new();
            let mut me = Self {
                _menu: menu.clone(),
                _submenus: Vec::new(),
                events,
                actions: Vec::new(),
                tracked: Vec::new(),
                recent: None,
                recent_paths: None,
                seq: 0,
                lang: i18n::current().code(),
                input: Vec::new(),
            };

            let app_menu = Submenu::new("WordCraft", true);
            me.entry(&app_menu, ABOUT, app);
            append(&app_menu, &PredefinedMenuItem::separator());
            me.entry(&app_menu, SETTINGS, app);
            append(&app_menu, &PredefinedMenuItem::separator());
            append(&app_menu, &PredefinedMenuItem::services(Some(t("Services"))));
            append(&app_menu, &PredefinedMenuItem::separator());
            me.add(&app_menu, t("Hide WordCraft"), Action::Hide, app);
            append(&app_menu, &PredefinedMenuItem::hide_others(Some(t("Hide Others"))));
            append(&app_menu, &PredefinedMenuItem::show_all(Some(t("Show All"))));
            append(&app_menu, &PredefinedMenuItem::separator());

            // Never use AppKit's predefined Quit: it bypasses unsaved-document protection.
            me.add(&app_menu, t("Quit WordCraft"), Action::Quit, app);

            let mut subs = vec![app_menu];

            for (title, entries) in MENUS {
                let sub = Submenu::new(t(title), true);
                if title == "Window" {
                    me.add(&sub, t("Minimize"), Action::Minimize, app);
                    append(&sub, &PredefinedMenuItem::maximize(Some(t("Zoom"))));
                    append(&sub, &PredefinedMenuItem::separator());
                    append(&sub, &PredefinedMenuItem::fullscreen(Some(t("Enter Full Screen"))));
                    append(&sub, &PredefinedMenuItem::separator());
                    append(&sub, &PredefinedMenuItem::bring_all_to_front(Some(t("Bring All to Front"))));
                }
                for e in entries {
                    me.entry(&sub, *e, app);
                }
                subs.push(sub);
            }
            for s in &subs {
                if let Err(e) = menu.append(s) {
                    log::warn!("native menu: {e}");
                }
            }
            menu.init_for_nsapp();
            if let Some(w) = subs.iter().find(|s| s.text() == t("Window")) {
                w.set_as_windows_menu_for_nsapp();
            }
            if let Some(h) = subs.last() {
                h.set_as_help_menu_for_nsapp();
            }
            me._submenus = subs;
            Some(me)
        }

        /// Each frame: build (or, after a language change, rebuild) the bar, run what was chosen,
        /// and bring enablement and Open Recent up to date.
        pub(crate) fn frame(slot: &mut Option<Self>, app: &mut WordApp, ctx: &egui::Context) {
            if ctx.with_plugin(|_: &mut TextFieldFocus| ()).is_none() {
                ctx.add_plugin(TextFieldFocus::default());
            }
            if slot.as_ref().is_none_or(|m| m.lang != i18n::current().code()) {
                let events = slot.take().map(|m| m.events);
                *slot = Self::install(app, ctx, events);
            }
            let Some(me) = slot.as_mut() else { return };
            while let Ok(e) = me.events.try_recv() {
                let Some(action) = me.actions.iter().find(|(id, _)| id == e.id()).map(|(_, a)| a.clone()) else { continue };
                if !action_enabled(&action, app, text_field_focused(ctx, app.active_viewport())) {
                    continue;
                }
                match action {
                    Action::Run(id, params) => {
                        if let Err(err) = app.run(id, params) {
                            app.status(err);
                        }
                    }
                    Action::Open(path) => {
                        if let Err(err) = app.run("file.open", json!({"path": path})) {
                            app.status(err);
                        }
                    }
                    Action::Hide => {
                        if let Some(mtm) = objc2::MainThreadMarker::new() {
                            objc2_app_kit::NSApplication::sharedApplication(mtm).hide(None);
                        }
                    }
                    Action::Minimize => ctx.send_viewport_cmd_to(app.active_viewport(), egui::ViewportCommand::Minimized(true)),
                    Action::Quit => {
                        if app.close_requested() {
                            app.quit_requested = true;
                        }
                    }
                    Action::Input(kind) => {
                        // Document undo uses its registry; text fields keep egui's local undo.
                        if app.canvas.focused && !app.modal_open() && matches!(kind, InputKind::Undo | InputKind::Redo) {
                            let id = if kind == InputKind::Undo { "edit.undo" } else { "edit.redo" };
                            if let Err(err) = app.run(id, json!({})) {
                                app.status(err);
                            }
                            continue;
                        }
                        let events = input_events(kind);
                        if events.is_empty() {
                            // Paste with no text on the clipboard (a picture): the ribbon's Paste,
                            // which reads pictures, HTML and RTF itself (#45, #584, #687).
                            if app.canvas.focused
                                && !app.modal_open()
                                && let Err(err) = app.run("edit.paste", json!({}))
                            {
                                app.status(err);
                            }
                        } else {
                            me.input.extend(events);
                        }
                    }
                }
                ctx.request_repaint();
            }
            let field_focused = text_field_focused(ctx, app.active_viewport());
            for t in &mut me.tracked {
                let enabled = action_enabled(&t.action, app, field_focused);
                if enabled != t.enabled {
                    t.item.set_enabled(enabled);
                    t.enabled = enabled;
                }
                let accelerator = accelerator(&t.action, app);
                if accelerator != t.accelerator {
                    if let Err(err) = t.item.set_accelerator(accelerator) {
                        log::warn!("native menu shortcut: {err}");
                    }
                    t.accelerator = accelerator;
                }
            }
            if me.recent_paths.as_ref() != Some(&app.ui.recent) {
                me.rebuild_recent(app);
            }
            if let Some(recent) = &me.recent {
                recent.set_enabled(!app.modal_open() && !app.ui.recent.is_empty());
            }
        }

        /// Replace recent-file actions when the editor's recent paths change.
        fn rebuild_recent(&mut self, app: &WordApp) {
            let Some(sub) = self.recent.clone() else { return };
            // Bounded: each pass removes one item of a finite submenu.
            while sub.remove_at(0).is_some() {}
            self.actions.retain(|(_, a)| !matches!(a, Action::Open(_)));
            for p in &app.ui.recent {
                let name = std::path::Path::new(p).file_name().map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned());
                let item = MenuItem::with_id(self.next_id(), name, true, None);
                self.actions.push((item.id().clone(), Action::Open(p.clone())));
                append(&sub, &item);
            }
            sub.set_enabled(!app.ui.recent.is_empty());
            self.recent_paths = Some(app.ui.recent.clone());
        }
    }

    /// Map supported Command bindings to AppKit, reserving standard Mac shortcuts.
    fn accelerator(action: &Action, app: &WordApp) -> Option<Accelerator> {
        let fixed = match action {
            Action::Hide => Some("CMD+H"),
            Action::Minimize => Some("CMD+M"),
            Action::Quit => Some("CMD+Q"),
            Action::Input(InputKind::Undo) => Some("CMD+Z"),
            Action::Input(InputKind::Redo) => Some("CMD+SHIFT+Z"),
            Action::Input(InputKind::Cut) => Some("CMD+X"),
            Action::Input(InputKind::Copy) => Some("CMD+C"),
            Action::Input(InputKind::Paste) => Some("CMD+V"),
            Action::Input(InputKind::SelectAll) => Some("CMD+A"),
            Action::Run("ui.backstage", params) if params.get("page").and_then(Value::as_str) == Some("options") => Some("CMD+Comma"),
            _ => None,
        };
        if let Some(key) = fixed {
            return Accelerator::from_str(key).ok();
        }
        let Action::Run(id, _) = action else { return None };
        // Only global document shortcuts belong in AppKit. Registering navigation or plain
        // character keys here would steal typing from text fields and input methods.
        if !matches!(
            *id,
            "file.new"
                | "file.open"
                | "file.close"
                | "file.save"
                | "file.saveAs"
                | "file.print"
                | "edit.find"
                | "format.bold"
                | "format.italic"
                | "format.underline"
        ) {
            return None;
        }
        app.session
            .keymap
            .keys_for(&app.session.registry, id)
            .iter()
            .filter(|key| {
                key.starts_with("Mod+")
                    && !matches!(
                        key.as_str(),
                        "Mod+H" | "Mod+M" | "Mod+Q" | "Mod+," | "Mod+X" | "Mod+C" | "Mod+V" | "Mod+A" | "Mod+Z" | "Mod+Shift+Z"
                    )
            })
            .find_map(|key| Accelerator::from_str(&key.replace("Mod+", "CMD+")).ok())
    }

    /// Retain text-field focus for each window: the menu runs in the root viewport,
    /// while a focused comment or Backstage field may belong to another window.
    #[derive(Default)]
    struct TextFieldFocus(std::collections::HashMap<egui::ViewportId, bool>);

    impl egui::Plugin for TextFieldFocus {
        /// Identify this focus tracker in egui's plugin inspection.
        fn debug_name(&self) -> &'static str {
            "native_menu_text_focus"
        }

        /// Snapshot focus after drawing each viewport and forget closed viewports.
        fn on_end_pass(&mut self, ui: &mut egui::Ui) {
            let ctx = ui.ctx();
            ctx.input(|i| self.0.retain(|id, _| i.raw.viewports.contains_key(id)));
            self.0.insert(ctx.viewport_id(), ctx.text_edit_focused());
        }
    }

    /// Read focus in the target viewport without substituting the root window's field.
    fn text_field_focused(ctx: &egui::Context, target: egui::ViewportId) -> bool {
        if target == ctx.viewport_id() {
            ctx.text_edit_focused()
        } else {
            ctx.with_plugin(|p: &mut TextFieldFocus| p.0.get(&target).copied().unwrap_or(false)).unwrap_or(false)
        }
    }

    /// A field in one window must neither enable nor disable formatting in another.
    #[test]
    fn text_field_focus_is_tracked_per_viewport() {
        let ctx = egui::Context::default();
        ctx.add_plugin(TextFieldFocus::default());
        let child = egui::ViewportId::from_hash_of("menu-focus-child");
        let field = egui::Id::unique("child-field");
        let mut text = String::from("Comment");
        let mut raw = egui::RawInput { viewport_id: child, ..Default::default() };
        raw.viewports.entry(child).or_default();
        ctx.run_ui(raw, |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).id(field)).request_focus();
        })
        .drop_without_applying_deltas();
        let mut root = egui::RawInput::default();
        root.viewports.entry(child).or_default();
        ctx.run_ui(root, |ui| {
            assert!(!text_field_focused(ui.ctx(), egui::ViewportId::ROOT));
            assert!(text_field_focused(ui.ctx(), child));
        })
        .drop_without_applying_deltas();
    }

    /// Apply document/modal guards both to displayed items and queued native actions.
    fn action_enabled(action: &Action, app: &WordApp, field_focused: bool) -> bool {
        match action {
            Action::Hide | Action::Minimize => true,
            // Input must remain available inside modal dialogs' text fields.
            Action::Input(_) => true,
            Action::Quit => !app.modal_open(),
            Action::Open(_) => !app.modal_open(),
            Action::Run(id, params) => {
                if app.modal_open() {
                    return false;
                }
                // Plain-text fields own these shortcuts; formatting the document behind
                // a comment editor or Backstage field would change unrelated content.
                if field_focused && matches!(*id, "format.bold" | "format.italic" | "format.underline") {
                    return false;
                }
                if matches!(*id, "ui.dialog" | "ui.backstage") {
                    app.has_document()
                        || matches!(params.get("name").and_then(Value::as_str), Some("about"))
                        || matches!(params.get("page").and_then(Value::as_str), Some("options" | "new"))
                } else {
                    widgets::enabled(app, id)
                }
            }
        }
    }

    /// Synthesize a complete Command-key press/release for the focused editor.
    fn key_events(key: egui::Key, shift: bool) -> Vec<egui::Event> {
        [true, false]
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers: egui::Modifiers { shift, ..egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND },
            })
            .to_vec()
    }

    /// Clicks go to `tx`, and wake the event loop so the next frame sees them.
    fn set_handler(tx: Sender<MenuEvent>, ctx: egui::Context) {
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = tx.send(e);
            ctx.request_repaint();
        }));
    }

    /// The input events the keyboard would have produced (as egui-winit makes them).
    pub(super) fn input_events(kind: InputKind) -> Vec<egui::Event> {
        match kind {
            InputKind::Undo => key_events(egui::Key::Z, false),
            InputKind::Redo => key_events(egui::Key::Z, true),
            InputKind::Cut => vec![egui::Event::Cut],
            InputKind::Copy => vec![egui::Event::Copy],
            InputKind::Paste => {
                // As egui-winit does for ⌘V: a paste event only when there is text.
                let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default().replace("\r\n", "\n");
                if text.is_empty() { Vec::new() } else { vec![egui::Event::Paste(text)] }
            }
            InputKind::SelectAll => key_events(egui::Key::A, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command the menus name is registered (a renamed command would silently vanish).
    /// Reject menu entries that refer to neither an engine nor a UI command.
    #[test]
    fn every_menu_command_is_registered() {
        let reg = wordcraft_engine::cmd::registry();
        let mut missing = Vec::new();
        for (_, entries) in MENUS {
            for e in entries {
                if let Entry::Cmd(id) = e
                    && !id.starts_with("ui.")
                    && reg.get(id).is_none()
                {
                    missing.push(*id);
                }
            }
        }
        assert!(missing.is_empty(), "unregistered menu commands: {missing:?}");
    }

    /// Flatten the menu definitions for registry and parameter validation.
    fn all_entries() -> Vec<Entry> {
        let mut v = vec![ABOUT, SETTINGS];
        for (_, entries) in MENUS {
            v.extend_from_slice(entries);
        }
        v
    }

    /// Keep every declarative parameter payload valid JSON.
    #[test]
    fn json_params_parse() {
        for e in all_entries() {
            if let Entry::With(_, p, _) = e {
                assert!(serde_json::from_str::<serde_json::Value>(p).is_ok(), "{p}");
            }
        }
    }

    /// Every item does something on a sample document, as its ribbon button would: no command
    /// fails for want of parameters, and every dialog it names exists.
    #[test]
    fn every_item_runs_on_a_sample_document() {
        use wordcraft_ui_egui::{Services, WordApp};
        // These open the system's file, print or sound services, or end the session.
        const SKIP: &[&str] = &[
            "file.open",
            "file.save",
            "file.saveAs",
            "file.print",
            "file.close",
            "insert.picture",
            "insert.object",
            "review.compare",
            "review.readAloud",
            "table.fromText",
            "table.toText",
        ];
        let mut failures = Vec::new();
        for e in all_entries() {
            let (id, params) = match e {
                Entry::Cmd(id) => (id, serde_json::json!({})),
                Entry::With(id, p, _) => (id, serde_json::from_str(p).unwrap()),
                _ => continue,
            };
            if SKIP.contains(&id) {
                continue;
            }
            let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), Services::default());
            let r = app.run(id, params.clone());
            // Commands that need something first (a search, a copy, an edit) say so, as their
            // keys do; that is not a dead item.
            const NEEDS_STATE: &[&str] = &[
                "edit.repeat",
                "edit.pasteText",
                "edit.pasteMerge",
                "edit.findNext",
                "edit.findPrevious",
                "edit.goBack",
                "table.insertRowAbove",
                "table.insertRowBelow",
                "table.insertColumnLeft",
                "table.insertColumnRight",
                "table.deleteTable",
                "table.deleteRow",
                "table.deleteColumn",
                "table.deleteCells",
                "table.selectTable",
                "table.selectRow",
                "table.selectColumn",
                "table.selectCell",
                "table.merge",
                "table.split",
                "table.splitTable",
                "table.properties",
            ];
            if let Err(err) = &r
                && !NEEDS_STATE.contains(&id)
            {
                failures.push(format!("{id} {params}: {err}"));
            } else if id == "ui.dialog" && app.dialog.is_none() {
                failures.push(format!("{id} {params}: no such dialog"));
            }
        }
        assert!(failures.is_empty(), "menu items that do nothing:\n{}", failures.join("\n"));
    }
}
