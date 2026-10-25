//! The macOS menu bar: WordCraft, File, Edit, View, Go, Insert, Format, Tools, Table, Window and Help,
//! as native menus, the way Word for Mac lays them out.
//!
//! Every item runs a registered command through `WordApp::run`, the path the ribbon and the
//! keyboard take (so "Save changes?" prompts, input dialogs and clipboard handling all apply), or,
//! for Cut / Copy / Paste / Select All, feeds egui the input events the keyboard would have. Items
//! carry no key equivalents: the keyboard keeps going through egui and the keymap exactly as before,
//! so a menu never takes a key away from a text field or the document.
//!
//! Quit routes through `Action::Quit` -> `ViewportCommand::Close` so unsaved changes prompt the user
//! before exiting, protecting against accidental closure on Command-Q (#4295).
//!
//! The item list is plain data (tested on every platform against the registry); the AppKit part
//! is macOS only.

/// One item of a menu.
#[derive(Clone, Copy, Debug)]
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
    Cmd("edit.undo"),
    Cmd("edit.redo"),
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
    Cmd("ui.collapseRibbon"),
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
pub(crate) const SETTINGS: Entry = With("ui.backstage", r#"{"value":true,"page":"options"}"#, "Options");

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
        /// Window ▸ Minimize, without AppKit's ⌘M (Increase Indent in WordCraft's keymap).
        Minimize,
        /// WordCraft ▸ Quit WordCraft, with save check before exit (#4295).
        Quit,
    }

    struct Tracked {
        item: MenuItem,
        /// The command whose enablement the item follows.
        id: &'static str,
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

    fn append(menu: &Submenu, item: &dyn IsMenuItem) {
        if let Err(e) = menu.append(item) {
            log::warn!("native menu: {e}");
        }
    }

    fn label(id: &str, app: &WordApp) -> String {
        app.session.registry.get(id).map_or_else(|| id.to_owned(), |s| t(s.label).to_owned())
    }

    impl NativeMenu {
        fn next_id(&mut self) -> String {
            self.seq = self.seq.saturating_add(1);
            format!("wordcraft.{}", self.seq)
        }

        fn add(&mut self, menu: &Submenu, text: &str, action: Action, follow: Option<&'static str>) {
            let item = MenuItem::with_id(self.next_id(), text, true, None);
            self.actions.push((item.id().clone(), action));
            if let Some(id) = follow {
                self.tracked.push(Tracked { item: item.clone(), id, enabled: true });
            }
            append(menu, &item);
        }

        fn entry(&mut self, menu: &Submenu, e: Entry, app: &WordApp) {
            match e {
                Entry::Cmd(id) => {
                    // A command this version doesn't have is left out.
                    if app.session.registry.get(id).is_some() || id.starts_with("ui.") {
                        self.add(menu, &label(id, app), Action::Run(id, json!({})), Some(id));
                    }
                }
                Entry::With(id, params, text) => {
                    let params = serde_json::from_str(params).unwrap_or_else(|_| json!({}));
                    self.add(menu, t(text), Action::Run(id, params), None);
                }
                Entry::Input(kind, text) => self.add(menu, t(text), Action::Input(kind), None),
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
            append(&app_menu, &PredefinedMenuItem::hide(Some(t("Hide WordCraft"))));
            append(&app_menu, &PredefinedMenuItem::hide_others(Some(t("Hide Others"))));
            append(&app_menu, &PredefinedMenuItem::show_all(Some(t("Show All"))));
            append(&app_menu, &PredefinedMenuItem::separator());

            // Quit item wired to Action::Quit with ⌘Q so unsaved changes prompt the user (#4295)
            let quit_accel = Accelerator::from_str("CMD+Q").ok();
            let quit_item = MenuItem::with_id(me.next_id(), t("Quit WordCraft"), true, quit_accel);
            me.actions.push((quit_item.id().clone(), Action::Quit));
            append(&app_menu, &quit_item);

            let mut subs = vec![app_menu];

            for (title, entries) in MENUS {
                let sub = Submenu::new(t(title), true);
                if title == "Window" {
                    me.add(&sub, t("Minimize"), Action::Minimize, None);
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
            if slot.as_ref().is_none_or(|m| m.lang != i18n::current().code()) {
                let events = slot.take().map(|m| m.events);
                *slot = Self::install(app, ctx, events);
            }
            let Some(me) = slot.as_mut() else { return };
            while let Ok(e) = me.events.try_recv() {
                let Some(action) = me.actions.iter().find(|(id, _)| id == e.id()).map(|(_, a)| a.clone()) else { continue };
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
                    Action::Minimize => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                    Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                    Action::Input(kind) => {
                        let events = input_events(kind);
                        if events.is_empty() {
                            // Paste with no text on the clipboard (a picture): the ribbon's Paste,
                            // which reads pictures, HTML and RTF itself (#45, #584, #687).
                            if let Err(err) = app.run("edit.paste", json!({})) {
                                app.status(err);
                            }
                        } else {
                            me.input.extend(events);
                        }
                    }
                }
                ctx.request_repaint();
            }
            for t in &mut me.tracked {
                let enabled = widgets::enabled(app, t.id);
                if enabled != t.enabled {
                    t.item.set_enabled(enabled);
                    t.enabled = enabled;
                }
            }
            if me.recent_paths.as_ref() != Some(&app.ui.recent) {
                me.rebuild_recent(app);
            }
        }

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

    /// Clicks go to `tx`, and wake the event loop so the next frame sees them.
    fn set_handler(tx: Sender<MenuEvent>, ctx: egui::Context) {
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = tx.send(e);
            ctx.request_repaint();
        }));
    }

    /// The input events the keyboard would have produced (as egui-winit makes them).
    fn input_events(kind: InputKind) -> Vec<egui::Event> {
        match kind {
            InputKind::Cut => vec![egui::Event::Cut],
            InputKind::Copy => vec![egui::Event::Copy],
            InputKind::Paste => {
                // As egui-winit does for ⌘V: a paste event only when there is text.
                let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default().replace("\r\n", "\n");
                if text.is_empty() { Vec::new() } else { vec![egui::Event::Paste(text)] }
            }
            InputKind::SelectAll => [true, false]
                .map(|pressed| egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: Some(egui::Key::A),
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                })
                .to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command the menus name is registered (a renamed command would silently vanish).
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

    fn all_entries() -> Vec<Entry> {
        let mut v = vec![ABOUT, SETTINGS];
        for (_, entries) in MENUS {
            v.extend_from_slice(entries);
        }
        v
    }

    #[test]
    fn json_params_parse() {
        for e in all_entries() {
            if let Entry::With(_, p, _) = e {
                assert!(serde_json::from_str::<serde_json::Value>(p).is_ok(), "{p}");
            }
        }
    }

    /// Every item does something on a sample document, as its ribbon button would: no command
    /// fails for want of parameters, and every dialog and Backstage page it names exists.
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
