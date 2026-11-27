//! Word-style keytips: Alt (or F10) shows letter badges over ribbon tabs, then over a tab's
//! controls; numbered badges also reach the customized Quick Access Toolbar. The mechanism (phase
//! machine, badges, claimed keys) is the framework's `Keytips`; this module gives it WordCraft's
//! tabs, controls, legacy sequences and regions.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use craft::behavior::overlay::MinWidth;
use craft::keymap::CommandRef;
use craft::menu::{KeytipControl, Keytips, RibbonOutput};
use craft::ui::contract::Bind;
use craft::ui::contract::overlay::Anchor;
use craft::ui::contract::{Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::popover::Popover;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::inspect::{self, Recorded};
use egui::{Rect, Ui};

use crate::WordApp;

/// The key of the inspected region the title bar records over its quick access toolbar buttons.
pub(crate) fn qat_region() -> egui::Id {
    egui::Id::unique("wc_keytips_qat_region")
}

/// The key of the inspected region the title bar records over its Search box.
pub(crate) fn search_region() -> egui::Id {
    egui::Id::unique("wc_keytips_search_region")
}

/// The key of the inspected region the Backstage records over its page buttons.
pub(crate) fn backstage_region() -> egui::Id {
    egui::Id::unique("wc_keytips_backstage_region")
}

/// Per-context map of ribbon `menu_button` widget ids to the command id each opens a menu for,
/// so [`Keytips`] places badges on menu buttons without binding them to fire on pointer click.
fn menu_buttons_id() -> egui::Id {
    egui::Id::unique("wc_keytips_menu_buttons")
}

/// Records (or clears) the command id associated with a ribbon `menu_button` widget id so
/// [`show`] can attach `CommandRef` to its inspected element for language-independent badge placement.
pub(crate) fn note_menu_button(ctx: &egui::Context, widget_id: egui::Id, command_id: Option<&str>) {
    let existing = ctx.data(|d| d.get_temp::<Arc<HashMap<egui::Id, CommandRef>>>(menu_buttons_id()));
    let unchanged = match (&existing, command_id) {
        (Some(map), Some(id)) => map.get(&widget_id).is_some_and(|cmd| cmd.id == id && cmd.params.is_null()),
        (Some(map), None) => !map.contains_key(&widget_id),
        (None, None) => true,
        (None, Some(_)) => false,
    };
    if unchanged {
        return;
    }
    ctx.data_mut(|d| {
        let mut map = d.get_temp::<Arc<HashMap<egui::Id, CommandRef>>>(menu_buttons_id()).map(|a| (*a).clone()).unwrap_or_default();
        match command_id {
            Some(id) => {
                map.insert(widget_id, CommandRef::new(id));
            }
            None => {
                map.remove(&widget_id);
            }
        }
        d.insert_temp(menu_buttons_id(), Arc::new(map));
    });
}

/// Attaches noted `menu_button` commands to uncommanded root elements of a ribbon `Recorded`
/// snapshot so `craft::menu::Keytips` places badges on menu buttons regardless of translation.
fn with_menu_commands(ctx: &egui::Context, recorded: Arc<Recorded>) -> Arc<Recorded> {
    let Some(commands) = ctx.data(|d| d.get_temp::<Arc<HashMap<egui::Id, CommandRef>>>(menu_buttons_id())) else {
        return recorded;
    };
    if commands.is_empty() || !recorded.elements.iter().any(|e| e.part.is_none() && e.command.is_none() && commands.contains_key(&e.id)) {
        return recorded;
    }
    let mut patched = (*recorded).clone();
    for element in &mut patched.elements {
        if element.part.is_none()
            && element.command.is_none()
            && let Some(cmd) = commands.get(&element.id)
        {
            element.command = Some(cmd.clone());
        }
    }
    Arc::new(patched)
}

/// Word's tab letters, as Word for Windows shows them after Alt (File F, Design G, References S,
/// Review R, View W, Help Y). File opens the Backstage.
const TAB_LETTERS: [(&str, &str); 11] = [
    ("File", "F"),
    ("Home", "H"),
    ("Insert", "N"),
    ("Draw", "D"),
    ("Design", "G"),
    ("Layout", "P"),
    ("References", "S"),
    ("Mailings", "M"),
    ("Review", "R"),
    ("View", "W"),
    ("Help", "Y"),
];

/// Word's contextual table tabs take two letters (Alt, J, T and Alt, J, L), shown in a table.
const TABLE_TAB_LETTERS: [(&str, &str); 2] = [("Table Design", "JT"), ("Table Layout", "JL")];

/// The Search box's badge beside the tab badges (Word's Alt, Q).
pub const SEARCH_LETTER: &str = "Q";

/// Legacy menu keys (#1697): Word 2003 menu sequences Word for Windows still takes after Alt,
/// observed there. Hidden, as in Word: no badges, not in Customize Keyboard. They start only on
/// letters no tab keytip or Search uses (see the test).
fn legacy() -> &'static [(&'static str, CommandRef)] {
    static LEGACY: OnceLock<Vec<(&'static str, CommandRef)>> = OnceLock::new();
    LEGACY.get_or_init(|| {
        let cmd = CommandRef::new;
        let dialog = |name: &str| CommandRef::with_params("ui.dialog", serde_json::json!({ "name": name }));
        vec![
            ("VU", cmd("view.fullScreenView")),
            ("VP", cmd("view.printLayout")),
            ("VW", cmd("view.webLayout")),
            ("VO", cmd("view.outline")),
            ("VN", cmd("view.draft")),
            ("VD", cmd("view.navigationPane")),
            ("VL", cmd("view.ruler")),
            ("VZ", dialog("zoom")),
            ("EL", cmd("select.all")),
            ("EF", dialog("find")),
            ("EE", dialog("replace")),
            ("EG", dialog("goto")),
            ("IS", dialog("symbol")),
            ("IPF", cmd("insert.picture")),
            ("OF", dialog("font")),
            ("OP", dialog("paragraph")),
            ("TW", cmd("review.wordCount")),
            ("AIT", dialog("insertTable")),
        ]
    })
}

/// Cached `Phase::Tabs` keytip sequences (`SEARCH_LETTER` when Backstage is closed, plus Word
/// 2003 legacy menu keys on non-macOS).
fn sequences(backstage: bool) -> &'static [(&'static str, CommandRef)] {
    static WITH_SEARCH: OnceLock<Vec<(&'static str, CommandRef)>> = OnceLock::new();
    static SEARCH_ONLY: OnceLock<[(&'static str, CommandRef); 1]> = OnceLock::new();

    match (cfg!(target_os = "macos"), backstage) {
        (true, true) => &[],
        (true, false) => SEARCH_ONLY.get_or_init(|| [(SEARCH_LETTER, CommandRef::new("tools.search"))]),
        (false, true) => legacy(),
        (false, false) => WITH_SEARCH.get_or_init(|| {
            let mut v = legacy().to_vec();
            v.push((SEARCH_LETTER, CommandRef::new("tools.search")));
            v
        }),
    }
}

/// The letter badge for a tab (empty when it has none).
pub fn tab_letter(tab: &str) -> &'static str {
    TAB_LETTERS.iter().chain(&TABLE_TAB_LETTERS).find(|(t, _)| *t == tab).map(|(_, l)| *l).unwrap_or("")
}

/// The tab a badge the keytips reported belongs to.
fn tab_of_badge(badge: &str) -> Option<&'static str> {
    TAB_LETTERS.iter().chain(&TABLE_TAB_LETTERS).find(|(_, l)| *l == badge).map(|(t, _)| *t)
}

/// The tabs that carry a badge, in display order.
pub fn badge_tabs(app: &WordApp) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = TAB_LETTERS.iter().filter(|(_, l)| !l.is_empty()).map(|(t, _)| *t).collect();
    if crate::ribbon::in_table_public(app) {
        v.extend_from_slice(&["Table Design", "Table Layout"]);
    }
    v
}

/// Letters Alt+letter already drives elsewhere; only Alt-only (or bare) shortcuts count, so
/// Mod+/Ctrl+/Shift+ shortcuts like ⌘B don't reserve the alphabet.
fn reserved_letters(app: &WordApp) -> Vec<char> {
    let mut v = Vec::new();
    for spec in app.session.registry.all() {
        let Some(sc) = spec.shortcut.split(" / ").next() else { continue };
        if sc.contains("Mod+") || sc.contains("Ctrl+") || sc.contains("Shift+") {
            continue;
        }
        let Some(last) = sc.rsplit('+').next() else { continue };
        let mut cs = last.chars();
        if let Some(c) = cs.next()
            && cs.next().is_none()
            && c.is_ascii_alphanumeric()
        {
            v.push(c.to_ascii_uppercase());
        }
    }
    v
}

/// A command's shortcut letter, but only for Alt+letter shortcuts (the kind a keytip triggers).
fn shortcut_letter(app: &WordApp, id: &str) -> Option<char> {
    let sc = app.session.registry.get(id)?.shortcut;
    let sc = sc.split(" / ").next()?;
    if sc.contains("Mod+") || sc.contains("Ctrl+") || sc.contains("Shift+") {
        return None;
    }
    let mut cs = sc.rsplit('+').next()?.chars();
    let c = cs.next()?;
    (cs.next().is_none() && c.is_ascii_alphanumeric()).then(|| c.to_ascii_uppercase())
}

/// One ribbon control: what it is called and the command its button is bound to.
struct Control {
    tip: &'static str,
    command: CommandRef,
}

/// A tab's controls in the order they are drawn; mirrors `ribbon.rs`, so badges land in order.
fn controls(tab: &str) -> Vec<Control> {
    let c = |tip: &'static str, id: &'static str| Control { tip, command: CommandRef::new(id) };
    let m = c;
    let d =
        |tip: &'static str, name: &'static str| Control { tip, command: CommandRef::with_params("ui.dialog", serde_json::json!({ "name": name })) };
    let p = |tip: &'static str, id: &'static str, params: serde_json::Value| Control { tip, command: CommandRef::with_params(id, params) };
    match tab {
        "Home" => vec![
            m("Paste", "edit.paste"),
            c("Cut", "edit.cut"),
            c("Copy", "edit.copy"),
            c("Format Painter", "edit.formatPainter"),
            c("Bold", "format.bold"),
            c("Italic", "format.italic"),
            m("Underline", "format.underline"),
            c("Strikethrough", "format.strikethrough"),
            c("Subscript", "format.subscript"),
            c("Superscript", "format.superscript"),
            m("Text Effects and Typography", "format.outline"),
            m("Text Highlight Color", "format.highlight"),
            m("Font Color", "format.color"),
            m("Bullets", "para.bullets"),
            m("Numbering", "para.numbering"),
            m("Multilevel List", "para.multilevel"),
            c("Decrease Indent", "para.outdent"),
            c("Increase Indent", "para.indent"),
            c("Sort", "para.sort"),
            c("Show/Hide ¶", "view.marks"),
            c("Align Left", "para.alignLeft"),
            c("Center", "para.alignCenter"),
            c("Align Right", "para.alignRight"),
            c("Justify", "para.justify"),
            m("Line and Paragraph Spacing", "para.lineSpacing"),
            m("Shading", "para.shading"),
            m("Borders", "para.borders"),
            c("Find", "edit.find"),
            d("Replace", "replace"),
            m("Select", "select.all"),
            c("Dictate", "tools.dictate"),
            c("Editor", "review.spelling"),
        ],
        "Insert" => vec![
            m("Cover Page", "insert.coverPage"),
            c("Blank Page", "insert.blankPage"),
            c("Page Break", "insert.pageBreak"),
            m("Table", "insert.table"),
            c("Pictures", "insert.picture"),
            m("Shapes", "insert.shape"),
            c("Icons", "insert.icon"),
            c("Diagram", "insert.smartArt"),
            c("Chart", "insert.chart"),
            d("Link", "link"),
            d("Bookmark", "bookmark"),
            c("Cross-reference", "insert.crossReference"),
            c("Comment", "review.newComment"),
            m("Header", "insert.header"),
            m("Footer", "insert.footer"),
            m("Page Number", "insert.pageNumber"),
            c("Text Box", "insert.textBox"),
            c("Quick Parts", "insert.quickParts"),
            m("Decorative Text", "insert.wordArt"),
            m("Drop Cap", "insert.dropCap"),
            c("Signature Line", "insert.signatureLine"),
            d("Date & Time", "dateTime"),
            c("Object", "insert.object"),
            c("Equation", "insert.equation"),
            m("Symbol", "insert.symbol"),
        ],
        "Draw" => vec![
            c("Select", "draw.select"),
            c("Lasso", "draw.lasso"),
            c("Eraser", "draw.eraser"),
            c("Pen", "draw.pen"),
            c("Pencil", "draw.pencil"),
            c("Highlighter", "draw.highlighter"),
            m("Add Pen", "draw.addPen"),
            c("Ink to Shape", "draw.inkToShape"),
            c("Ink to Math", "draw.inkToMath"),
            c("Drawing Canvas", "insert.canvas"),
            c("Ink Replay", "draw.replay"),
        ],
        "Design" => vec![
            m("Themes", "design.theme"),
            m("Theme Colors", "design.themeColors"),
            m("Theme Fonts", "design.themeFonts"),
            m("Paragraph Spacing", "design.paragraphSpacing"),
            c("Effects", "design.effects"),
            c("Set as Default", "design.setDefault"),
            m("Watermark", "design.watermark"),
            m("Page Color", "design.pageColor"),
            m("Page Borders", "design.pageBorders"),
        ],
        "Layout" => vec![
            m("Text Direction", "layout.textDirection"),
            m("Margins", "layout.margins"),
            m("Orientation", "layout.orientation"),
            m("Size", "layout.size"),
            m("Columns", "layout.columns"),
            m("Breaks", "layout.break"),
            m("Line Numbers", "layout.lineNumbers"),
            m("Hyphenation", "layout.hyphenation"),
            c("Fit One Page", "layout.fitOnePage"),
            c("Position", "arrange.position"),
            c("Wrap Text", "arrange.wrap"),
            c("Bring Forward", "arrange.bringForward"),
            c("Send Backward", "arrange.sendBackward"),
            c("Selection Pane", "arrange.selectionPane"),
            c("Align", "arrange.align"),
            c("Group", "arrange.group"),
            c("Rotate", "arrange.rotate"),
        ],
        "References" => vec![
            m("Table of Contents", "references.toc"),
            m("Add Text", "references.addText"),
            c("Update Table", "references.updateToc"),
            c("Insert Footnote", "references.footnote"),
            c("Insert Endnote", "references.endnote"),
            c("Next Footnote", "references.nextFootnote"),
            c("Show Notes", "references.notes"),
            c("Researcher", "references.researcher"),
            c("Insert Citation", "references.citation"),
            c("Manage Sources", "references.sources"),
            m("Citation Style", "references.citationStyle"),
            c("Bibliography", "references.bibliography"),
            c("Insert Caption", "references.caption"),
            c("Insert Table of Figures", "references.tableOfFigures"),
            c("Update Table", "references.updateFigures"),
            c("Cross-reference", "insert.crossReference"),
            c("Mark Entry", "references.markEntry"),
            c("Insert Index", "references.index"),
            c("Update Index", "references.updateIndex"),
            c("Mark Citation", "references.markCitation"),
            c("Insert Table of Authorities", "references.tableOfAuthorities"),
        ],
        "Mailings" => vec![
            c("Envelopes", "mailings.envelopes"),
            c("Labels", "mailings.labels"),
            c("Start Mail Merge", "mailings.start"),
            c("Select Recipients", "mailings.recipients"),
            c("Edit Recipient List", "mailings.editRecipients"),
            c("Highlight Merge Fields", "mailings.highlightFields"),
            c("Address Block", "mailings.addressBlock"),
            c("Greeting Line", "mailings.greetingLine"),
            c("Insert Merge Field", "mailings.insertField"),
            c("Rules", "mailings.rules"),
            c("Match Fields", "mailings.matchFields"),
            c("Preview Results", "mailings.preview"),
            p("First Record", "mailings.first", serde_json::json!({})),
            p("Previous Record", "mailings.previous", serde_json::json!({})),
            p("Next Record", "mailings.next", serde_json::json!({})),
            p("Last Record", "mailings.last", serde_json::json!({})),
            c("Find Recipient", "mailings.findRecipient"),
            c("Check for Errors", "mailings.checkErrors"),
            c("Finish & Merge", "mailings.finish"),
        ],
        "Review" => vec![
            c("Spelling & Grammar", "review.spelling"),
            c("Thesaurus", "review.thesaurus"),
            d("Word Count", "wordCount"),
            c("Read Aloud", "review.readAloud"),
            c("Check Accessibility", "file.accessibility"),
            c("Translate", "review.translate"),
            c("Language", "review.language"),
            c("New Comment", "review.newComment"),
            c("Delete", "review.deleteComment"),
            c("Previous", "review.previousComment"),
            c("Next", "review.nextComment"),
            c("Show Comments", "view.commentsPane"),
            c("Track Changes", "review.trackChanges"),
            m("Display for Review", "review.markup"),
            c("Reviewing Pane", "review.changes"),
            m("Accept", "review.accept"),
            m("Reject", "review.reject"),
            c("Previous", "review.previousChange"),
            c("Next", "review.nextChange"),
            c("Compare", "review.compare"),
            c("Block Authors", "review.blockAuthors"),
            c("Restrict Editing", "review.restrict"),
            c("Hide Ink", "review.hideInk"),
        ],
        "View" => vec![
            c("Read Mode", "view.readMode"),
            c("Print Layout", "view.printLayout"),
            c("Web Layout", "view.webLayout"),
            c("Markdown Source", "view.markdownSource"),
            c("Outline", "view.outline"),
            c("Draft", "view.draft"),
            c("Focus", "view.focus"),
            c("Immersive Reader", "view.immersive"),
            c("Vertical", "view.vertical"),
            c("Side to Side", "view.sideToSide"),
            d("Zoom", "zoom"),
            c("100%", "view.zoom100"),
            c("One Page", "view.onePage"),
            c("Multiple Pages", "view.multiplePages"),
            c("Page Width", "view.pageWidth"),
            c("Switch Modes", "view.darkMode"),
            c("New Window", "view.newWindow"),
            c("Arrange All", "view.arrangeAll"),
            c("Split", "view.split"),
            c("View Side by Side", "view.sideBySide"),
            c("Synchronous Scrolling", "view.syncScroll"),
            m("Switch Windows", "view.switchWindows"),
            c("Full Screen", "view.fullScreen"),
            c("Macros", "tools.macros"),
        ],
        "Help" => vec![d("Help", "about"), c("Community Discord", "ui.discord"), d("Commands", "commands")],
        "Table Design" => {
            vec![m("Shading", "table.shading"), m("Borders", "table.borders"), c("Border Painter", "table.borderPainter")]
        }
        "Table Layout" => vec![
            m("Select", "table.selectCell"),
            c("View Gridlines", "table.viewGridlines"),
            c("Properties", "table.properties"),
            c("Draw Table", "table.draw"),
            c("Eraser", "table.eraser"),
            m("Delete", "table.deleteCells"),
            c("Insert Above", "table.insertRowAbove"),
            c("Insert Below", "table.insertRowBelow"),
            c("Insert Left", "table.insertColumnLeft"),
            c("Insert Right", "table.insertColumnRight"),
            c("Merge Cells", "table.merge"),
            c("Split Cells", "table.split"),
            c("Split Table", "table.splitTable"),
            m("AutoFit", "table.autofit"),
            c("Distribute Rows", "table.distributeRows"),
            c("Distribute Columns", "table.distributeColumns"),
            c("Sort", "table.sort"),
            c("Repeat Header Rows", "table.repeatHeader"),
            c("Convert to Text", "table.toText"),
            c("Formula", "table.formula"),
        ],
        _ => Vec::new(),
    }
}

/// Maps an ASCII alphanumeric keytip letter to a static single-character `&'static str`, avoiding
/// per-control `String` allocations when caching [`KeytipControl`].
fn ascii_upper_str(c: char) -> Option<&'static str> {
    const CHARS: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let idx = CHARS.find(c.to_ascii_uppercase())?;
    CHARS.get(idx..idx + 1)
}

/// Cached keytip controls and reserved shortcut letters per engine registry and active ribbon tab,
/// avoiding per-frame `registry.all()` scans and `KeytipControl` / `CommandRef` allocations while
/// idle.
pub(crate) struct KeytipCache {
    registry: Arc<wordcraft_engine::Registry>,
    reserved: Vec<char>,
    tab: String,
    tab_controls: Vec<KeytipControl<'static>>,
}

impl KeytipCache {
    fn build_tab_controls(app: &WordApp, tab: &str) -> Vec<KeytipControl<'static>> {
        let mut out: Vec<KeytipControl<'static>> = controls(tab)
            .into_iter()
            .map(|control| {
                let letters = shortcut_letter(app, &control.command.id).and_then(ascii_upper_str);
                KeytipControl::new(control.command, control.tip, letters)
            })
            .collect();
        out.push(KeytipControl::new(CommandRef::new("tools.search"), "Search", Some(SEARCH_LETTER)));
        out
    }

    fn new(app: &WordApp, tab: &str) -> Self {
        KeytipCache {
            registry: app.session.registry.clone(),
            reserved: reserved_letters(app),
            tab: tab.to_owned(),
            tab_controls: Self::build_tab_controls(app, tab),
        }
    }

    fn sync(&mut self, app: &WordApp, tab: &str) {
        let reg_changed = !Arc::ptr_eq(&self.registry, &app.session.registry);
        if reg_changed {
            self.registry = app.session.registry.clone();
            self.reserved = reserved_letters(app);
        }
        if reg_changed || self.tab != tab {
            self.tab.clear();
            self.tab.push_str(tab);
            self.tab_controls = Self::build_tab_controls(app, tab);
        }
    }
}

/// The Backstage's pages as cached static keytip controls.
fn backstage_controls() -> &'static [KeytipControl<'static>] {
    static CONTROLS: OnceLock<Vec<KeytipControl<'static>>> = OnceLock::new();
    CONTROLS.get_or_init(|| {
        crate::backstage::KEYTIPS
            .iter()
            .map(|(page, letter)| {
                KeytipControl::new(CommandRef::with_params("ui.backstage", serde_json::json!({ "page": page })), page, Some(letter))
            })
            .collect()
    })
}

/// Shows the keytips after the shell and the regions they place badges on, and acts on what the keys chose.
pub(crate) fn show(ui: &mut Ui, app: &mut WordApp, ribbon: &RibbonOutput<String>, screen_bar: Option<Rect>) {
    let ctx = ui.ctx().clone();
    // Alt during an object drag is part of the drag (Alt moves an object freely), not a keytip tap.
    // PLAN: `CanvasState::object_gesture` is canvas-w9's (cv.A10).
    let enabled = app.ui.keytips_enabled && app.canvas.object_gesture().is_none();

    // A tab the ribbon shows has an element id this pass; one in the overflow menu has none and gets no badge.
    // PLAN: assumed `RibbonOutput::tab_ids: Vec<(T, egui::Id)>` (ribbon-w3, S-L361).
    let tabs: Vec<(Label, &str, egui::Id)> = badge_tabs(app)
        .into_iter()
        .filter_map(|t| ribbon.tab_ids.iter().find(|(name, _)| name == t).map(|(_, id)| (Label::from(t), tab_letter(t), *id)))
        .collect();

    let cache = match app.keytip_cache.take() {
        Some(mut cache) => {
            cache.sync(app, &app.ui.tab);
            cache
        }
        None => KeytipCache::new(app, &app.ui.tab),
    };
    let controls: &[KeytipControl<'_>] = if app.ui.backstage { backstage_controls() } else { &cache.tab_controls };
    let sequences = sequences(app.ui.backstage);
    // A hidden toolbar has no badges, and a disabled command's button has none to press.
    let digits: Vec<CommandRef> = if app.ui.quick_access_toolbar {
        app.session
            .ribbon
            .qat()
            .iter()
            // PLAN: chrome-w8 gives `qat_enabled` a leading `ctx`.
            .filter(|id| crate::chrome::qat_enabled(&ctx, app, id))
            .map(|id| CommandRef::new(id.to_string()))
            .collect()
    } else {
        Vec::new()
    };

    let mut places: Vec<Arc<Recorded>> = Vec::new();
    let patch_menu_places = enabled && (app.keytips_active || !ctx.input(|i| i.events.is_empty()));
    places.extend(ribbon.controls.clone().map(|recorded| if patch_menu_places { with_menu_commands(&ctx, recorded) } else { recorded }));
    places.extend(inspect::recorded(&ctx, qat_region()));
    places.extend(inspect::recorded(&ctx, search_region()));
    if app.ui.backstage {
        places.extend(inspect::recorded(&ctx, backstage_region()));
    }

    let out = Keytips::new(enabled)
        .tabs(&tabs)
        .controls(controls)
        .sequences(sequences)
        .digits(&digits)
        .reserved(&cache.reserved)
        .places(&places)
        .show_over(ui);
    app.keytip_cache = Some(cache);
    // PLAN: `WordApp::keytips_active` is app-hub's (rb.A11).
    app.keytips_active = out.active;
    if let Some(tab) = out.tab.as_deref().and_then(tab_of_badge) {
        let _ = app.run("ui.tab", serde_json::json!({ "tab": tab }));
    }
    if let Some(pending) = &out.pending {
        // With no ribbon showing (Focus, Full Screen View) the hint hangs under the screen bar.
        if let Some(anchor) = ribbon.tabs.or(screen_bar) {
            legacy_hint(ui, pending, anchor);
        }
    }
}

/// The hint under `anchor` while a legacy menu key sequence waits for its next letter.
fn legacy_hint(ui: &mut Ui, pending: &str, anchor: Rect) {
    let keys: Vec<String> = pending.chars().map(String::from).collect();
    let text = crate::i18n::fmt(tl!("Old menu keys: Alt, {keys}. Keep typing, or press Esc to cancel."), &[("keys", &keys.join(", "))]);
    let mut open = true;
    Popover::new(tl!("Old menu keys"), Bind::bound(&mut open))
        .overlay(|overlay| overlay.anchor(Anchor::Rect(anchor)).min_width(MinWidth::None))
        .show(ui, |ui, _| {
            Text::new(text.clone()).show(ui);
        });
}

#[cfg(test)]
mod tests;
