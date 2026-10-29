//! Word-style keytips: Alt (or F10) shows letter badges over ribbon tabs, then over a tab's
//! controls; numbered badges also reach the customized Quick Access Toolbar. Typing never
//! reaches the document while active.

use egui::{Align2, CornerRadius, Rect, Stroke, Ui, Vec2};

use crate::WordApp;
use crate::theme::{Tokens, semibold};

/// egui ids the ribbon publishes its keytip state under (accessors, since `Id::new` isn't const).
pub fn phase_id() -> egui::Id {
    egui::Id::unique("wc_keytips_phase")
}
/// Stable id for the keytips' selected-tab state.
pub fn tab_id() -> egui::Id {
    egui::Id::unique("wc_keytips_tab")
}
/// Stable id for the keytips' letter badges.
pub fn letters_id() -> egui::Id {
    egui::Id::unique("wc_keytips_letters")
}
/// Stable id for the keytips' badge rectangles.
pub fn rects_id() -> egui::Id {
    egui::Id::unique("wc_keytips_rects")
}
/// The phase whose first control still has to take the keyboard focus (#1012).
fn focus_pending_id() -> egui::Id {
    egui::Id::unique("wc_keytips_focus_pending")
}
/// The ribbon control keytips gave the keyboard focus to.
fn focused_id() -> egui::Id {
    egui::Id::unique("wc_keytips_focused")
}

/// Keytips move the keyboard focus (what screen readers follow) as they go (#1012): Alt or F10
/// focuses the active tab, choosing a tab its first command. `resp` takes the focus when keytips
/// are waiting to focus a control of `phase`.
pub fn take_focus(ctx: &egui::Context, resp: &egui::Response, phase: Phase) {
    // While keytips hold the focus here, Tab and the arrows are theirs: egui reads them before
    // the keytips do and would otherwise move the focus to some other widget, which keeps it
    // once keytips give the keyboard back to the page (#1012 review).
    if keytip_focus(ctx) == Some(resp.id) {
        let lock = egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false };
        ctx.memory_mut(|m| m.set_focus_lock_filter(resp.id, lock));
    }
    if ctx.data(|d| d.get_temp::<Phase>(focus_pending_id())) != Some(phase) {
        return;
    }
    resp.request_focus();
    ctx.data_mut(|d| {
        d.remove::<Phase>(focus_pending_id());
        d.insert_temp(focused_id(), resp.id);
    });
}

/// The ribbon control keytips focused, while it keeps the focus.
fn keytip_focus(ctx: &egui::Context) -> Option<egui::Id> {
    let id = ctx.data(|d| d.get_temp::<egui::Id>(focused_id()))?;
    ctx.memory(|m| m.has_focus(id)).then_some(id)
}

/// Keytips ended: the document gets the keyboard back, unless something else (a dialog, the
/// Backstage, a text field) took it meanwhile.
fn release_focus(app: &mut WordApp, ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<Phase>(focus_pending_id()));
    let Some(id) = ctx.data_mut(|d| {
        let id = d.get_temp::<egui::Id>(focused_id());
        d.remove::<egui::Id>(focused_id());
        id
    }) else {
        return;
    };
    if !ctx.memory(|m| m.has_focus(id) || m.focused().is_none()) {
        return;
    }
    if app.dialog.is_some() || app.ui.backstage {
        ctx.memory_mut(|m| m.surrender_focus(id));
        return;
    }
    app.canvas.want_focus = true;
    if let Some((canvas, _)) = app.canvas.widget_id {
        ctx.memory_mut(|m| m.request_focus(canvas));
    }
}

/// A bare Alt press (either Alt key) arms or cancels keytips.
fn is_alt(key: egui::Key) -> bool {
    matches!(key, egui::Key::AltLeft | egui::Key::AltRight)
}

/// What the keytip state machine is showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// Keytips off; the ribbon behaves normally.
    #[default]
    Off,
    /// Badges over the tabs.
    Tabs,
    /// Badges over one tab's groups and buttons.
    Commands,
    /// Partway through a legacy menu key sequence (#1697); the letters so far are in
    /// `keytip_prefix`, and a hint stands in for the badges.
    Legacy,
}

/// Which ribbon item a letter is bound to, and what pressing it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Switch to a tab (and show its command badges).
    Tab(&'static str),
    /// Run a command, optionally with parameters.
    Cmd { id: &'static str, params: serde_json::Value },
    /// Open a dialog by name.
    Dialog { name: &'static str },
}

impl Target {
    fn run(self, app: &mut WordApp) {
        match self {
            Target::Tab(tab) => {
                let _ = app.run("ui.tab", serde_json::json!({ "tab": tab }));
            }
            Target::Cmd { id, params } => {
                let _ = app.run(id, params);
            }
            Target::Dialog { name } => {
                let _ = app.run("ui.dialog", serde_json::json!({ "name": name }));
            }
        }
    }
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
fn legacy() -> Vec<(&'static str, Target)> {
    let cmd = |id| Target::Cmd { id, params: serde_json::json!({}) };
    let dialog = |name| Target::Dialog { name };
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
}

/// A letter in a legacy menu key sequence: run it once complete, wait while it can still grow,
/// and ignore a letter that leads nowhere, as Word does.
fn legacy_key(app: &mut WordApp, name: &str) {
    let keys = format!("{}{name}", app.ui.keytip_prefix);
    let rows = legacy();
    if let Some((_, target)) = rows.iter().find(|(k, _)| *k == keys) {
        app.ui.keytips = Phase::Off;
        app.ui.keytip_prefix.clear();
        target.clone().run(app);
    } else if rows.iter().any(|(k, _)| k.starts_with(&keys)) {
        app.ui.keytips = Phase::Legacy;
        app.ui.keytip_prefix = keys;
    }
}

/// The letter badge for a tab (empty when it has none).
pub fn tab_letter(tab: &str) -> &'static str {
    TAB_LETTERS.iter().chain(&TABLE_TAB_LETTERS).find(|(t, _)| *t == tab).map(|(_, l)| *l).unwrap_or("")
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

/// One ribbon control: what it is called and what it does.
struct Control {
    tip: &'static str,
    target: Target,
}

/// A tab's controls in the order they are drawn; mirrors `ribbon.rs`, so badges land in order.
fn controls(tab: &str) -> Vec<Control> {
    let c = |tip: &'static str, id: &'static str| Control { tip, target: Target::Cmd { id, params: serde_json::json!({}) } };
    let m = c;
    let d = |tip: &'static str, name: &'static str| Control { tip, target: Target::Dialog { name } };
    let p = |tip: &'static str, id: &'static str, params: serde_json::Value| Control { tip, target: Target::Cmd { id, params } };
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
            c("SmartArt", "insert.smartArt"),
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
            m("WordArt", "insert.wordArt"),
            m("Drop Cap", "insert.dropCap"),
            c("Signature Line", "insert.signatureLine"),
            c("Date & Time", "insert.dateTime"),
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

/// The badge letter for every control of a tab, in drawing order.
/// A command keeps its own `Alt+letter` shortcut letter; the rest take a free name/word
/// initial, then any free letter — never a reserved or already-assigned one.
fn letters_for(app: &WordApp, tab: &str) -> Vec<String> {
    let reserved = reserved_letters(app);
    let ctrls = controls(tab);
    let n = ctrls.len();
    let mut out: Vec<Option<String>> = vec![None; n];
    let mut used: Vec<char> = Vec::new();

    // Candidate letters for a control: its own shortcut letter, then its name's letters, then
    // the initials of its words — all uppercase, alphanumeric only.
    let candidates = |ctl: &Control, own: Option<char>| -> Vec<char> {
        let mut v = Vec::new();
        v.extend(own);
        let words: Vec<&str> = ctl.tip.split_whitespace().filter(|w| w.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())).collect();
        if let Some(w) = words.first() {
            v.extend(w.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()));
        }
        if words.len() > 1 {
            v.extend(words.iter().map(|w| w.chars().next().unwrap_or(' ')).filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()));
        }
        v
    };

    let id_of = |ctl: &Control| match &ctl.target {
        Target::Cmd { id, .. } => *id,
        Target::Dialog { .. } => "ui.dialog",
        Target::Tab(_) => "ui.tab",
    };

    // Pass 1: own shortcut letters. First-come keeps it if two commands somehow share a letter.
    for (i, ctl) in ctrls.iter().enumerate() {
        if let Some(c) = shortcut_letter(app, id_of(ctl))
            && !used.contains(&c)
        {
            used.push(c);
            out[i] = Some(c.to_string());
        }
    }
    // Pass 2: name-derived letters for everything still unassigned.
    for (i, ctl) in ctrls.iter().enumerate() {
        if out[i].is_some() {
            continue;
        }
        let own = shortcut_letter(app, id_of(ctl));
        let chosen = candidates(ctl, own)
            .into_iter()
            .find(|c| !used.contains(c) && !reserved.contains(c))
            .or_else(|| ('A'..='Z').find(|c| !used.contains(c) && !reserved.contains(c)));
        match chosen {
            Some(c) => {
                used.push(c);
                out[i] = Some(c.to_string());
            }
            None => {
                // Two-letter badge fallback: a tab has at most ~20 controls, so this succeeds;
                // the "?n" form keeps assignment total rather than panicking.
                let two = ('A'..='Z').flat_map(|a| ('A'..='Z').map(move |b| format!("{a}{b}"))).find(|s| {
                    let mut ch = s.chars();
                    let (a, b) = (ch.next().unwrap_or('?'), ch.next().unwrap_or('?'));
                    !out.iter().flatten().any(|o| o == s) && !reserved.contains(&a) && !reserved.contains(&b)
                });
                let two = two.unwrap_or_else(|| format!("?{}", out.iter().flatten().count()));
                used.extend(two.chars());
                out[i] = Some(two);
            }
        }
    }
    out.into_iter().map(Option::unwrap_or_default).collect()
}

/// Pure letter assignment `(tab letters, per-tab control letters)`, for tests without a window.
#[cfg(test)]
fn letters(app: &WordApp) -> (Vec<(&'static str, String)>, Vec<(String, Vec<String>)>) {
    let tabs = badge_tabs(app).into_iter().map(|t| (t, tab_letter(t).to_string())).collect();
    let cmds = crate::ribbon::TABS
        .iter()
        .filter(|t| !tab_letter(t).is_empty() || crate::ribbon::in_table_public(app))
        .map(|t| ((*t).to_string(), letters_for(app, t)))
        .collect();
    (tabs, cmds)
}

/// A keytip control as the ribbon needs to see it: its tooltip and its command id.
pub struct ControlInfo {
    pub tip: &'static str,
    pub id: &'static str,
}

/// A tab's badgeable controls in drawing order, for the widgets to find their own badges.
pub fn controls_public(tab: &str) -> Vec<ControlInfo> {
    controls(tab)
        .into_iter()
        .map(|c| ControlInfo {
            tip: c.tip,
            id: match &c.target {
                Target::Cmd { id, .. } => id,
                Target::Dialog { .. } => "ui.dialog",
                Target::Tab(_) => "ui.tab",
            },
        })
        .collect()
}

/// A keytip badge: letters or toolbar numbers, high contrast in both themes.
pub fn badge(ui: &Ui, rect: Rect, letter: &str) {
    let count = letter.chars().count();
    if count == 0 || count > 4 {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let font = semibold(if count > 3 { 8.5 } else { 10.0 });
    let width = ui.ctx().fonts_mut(|f| f.layout_no_wrap(letter.to_string(), font.clone(), t.on_accent).size().x);
    let r = Rect::from_min_size(rect.min + Vec2::new(2.0, 1.0), Vec2::new((width + 4.0).max(15.0), 15.0));
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(3), t.accent);
    p.rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0, t.menu), egui::StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, letter, font, t.on_accent);
}

/// Record a tab badge over a tab's real rect, called as each tab is laid out. After a typed
/// prefix (J, or Quick Access Toolbar digits) only the badges it can still complete show.
pub fn record_tab(rect: Rect, tab: &str, prefix: &str, out: &mut Vec<(Rect, String)>) {
    let l = tab_letter(tab);
    if !l.is_empty() && l.starts_with(prefix) {
        out.push((rect, l.to_string()));
    }
}

/// Record a control badge over the control's real rect. `index` is the control's position in
/// the tab's drawing order, which is exactly the order `letters_for` assigns letters in.
pub fn record_control(rect: Rect, letter: String, out: &mut Vec<(Rect, String)>) {
    out.push((rect, letter));
}

/// The badge letter for the `index`th control of a tab.
pub fn letter_for(app: &WordApp, index: usize) -> String {
    letters_for(app, &app.ui.tab).get(index).cloned().unwrap_or_default()
}

/// The number of badgeable controls on a tab (so callers can index them in drawing order).
pub fn control_count(tab: &str) -> usize {
    controls(tab).len()
}

/// Handle a key while keytips are showing. Returns true when the key was consumed.
pub fn handle_key(app: &mut WordApp, key: egui::Key) -> bool {
    if app.ui.keytips != Phase::Off {
        match key {
            // Escape steps back a level: from a tab's commands to the tabs, then out.
            egui::Key::Escape => {
                app.ui.keytips = if app.ui.keytips == Phase::Commands { Phase::Tabs } else { Phase::Off };
            }
            k if is_alt(k) => {
                app.ui.keytips = if app.ui.keytips == Phase::Tabs { Phase::Off } else { Phase::Tabs };
            }
            k => {
                let Some(name) = keys::key_name(k).map(|n| n.to_ascii_uppercase()) else {
                    // A key with no keytip cancels the mode.
                    app.ui.keytips = Phase::Off;
                    app.ui.keytip_prefix.clear();
                    return true;
                };
                // Quick Access Toolbar badges are all digits; a pending prefix keeps its kind.
                let typed = if app.ui.keytip_prefix.is_empty() { name.as_str() } else { app.ui.keytip_prefix.as_str() };
                if app.ui.keytips == Phase::Tabs && typed.chars().all(|c| c.is_ascii_digit()) {
                    qat_key(app, &name);
                    return true;
                }
                match app.ui.keytips {
                    Phase::Tabs => {
                        let name = format!("{}{name}", app.ui.keytip_prefix);
                        let tabs = badge_tabs(app);
                        let hit = tabs.iter().copied().find(|t| tab_letter(t) == name);
                        // J waits for the table tab's second letter.
                        if hit.is_none() && tabs.iter().any(|t| tab_letter(t).starts_with(&name)) {
                            app.ui.keytip_prefix = name;
                            return true;
                        }
                        match hit {
                            Some(tab) => {
                                let _ = app.run("ui.tab", serde_json::json!({ "tab": tab }));
                                // File opens the Backstage: no ribbon commands to badge.
                                app.ui.keytips = if tab == "File" { Phase::Off } else { Phase::Commands };
                            }
                            // Q is the title bar's Search box, as in Word (#819).
                            None if name == SEARCH_LETTER => {
                                app.ui.keytips = Phase::Off;
                                let _ = app.run("tools.search", serde_json::json!({}));
                            }
                            // Word 2003's menu keys: off on macOS, where F10 still shows badges.
                            None if !cfg!(target_os = "macos") && legacy().iter().any(|(k, _)| k.starts_with(name.as_str())) => {
                                legacy_key(app, &name);
                                return true;
                            }
                            None => app.ui.keytips = Phase::Off,
                        }
                    }
                    Phase::Legacy => {
                        legacy_key(app, &name);
                        return true;
                    }
                    Phase::Commands => {
                        let tab = app.ui.tab.clone();
                        let ls = letters_for(app, &tab);
                        let ctrls = controls(&tab);
                        let mut hit: Option<Target> = None;
                        for (ctl, l) in ctrls.into_iter().zip(ls) {
                            if l == name {
                                hit = Some(ctl.target);
                                break;
                            }
                        }
                        app.ui.keytips = Phase::Off;
                        if let Some(t) = hit {
                            t.run(app);
                        }
                    }
                    Phase::Off => {}
                }
            }
        }
        app.ui.keytip_prefix.clear();
        return true;
    }
    false
}

/// The first nine buttons use one digit. Further buttons use a leading zero and equally wide
/// numbers, so no complete badge is the prefix of another, even for the maximum-size toolbar.
pub(crate) fn qat_label(index: usize, count: usize) -> String {
    if index < 9 {
        (index + 1).to_string()
    } else {
        let width = count.saturating_sub(9).to_string().len();
        format!("0{:0width$}", index.saturating_sub(8))
    }
}

fn qat_key(app: &mut WordApp, name: &str) {
    app.ui.keytip_prefix.push_str(name);
    // A hidden toolbar has no badges, so a digit cancels the mode and runs nothing (#1945).
    let ids = if app.ui.quick_access_toolbar { app.session.ribbon.qat() } else { Vec::new() };
    let mut hit = None;
    let mut pending = false;
    for (index, id) in ids.iter().enumerate() {
        let label = qat_label(index, ids.len());
        if label == app.ui.keytip_prefix {
            hit = Some((*id).to_string());
            break;
        }
        pending |= label.starts_with(&app.ui.keytip_prefix);
    }
    if hit.is_some() || !pending {
        app.ui.keytips = Phase::Off;
        app.ui.keytip_prefix.clear();
        if let Some(id) = hit
            && crate::chrome::qat_enabled(app, &id)
        {
            let _ = app.run(&id, serde_json::json!({}));
        }
    }
}

/// Alt (or F10) turns keytips on: the first press shows the tab badges.
pub fn activate(app: &mut WordApp) {
    if app.ui.keytips_enabled && app.ui.keytips == Phase::Off {
        app.ui.keytip_prefix.clear();
        app.ui.keytips = Phase::Tabs;
    }
}

use crate::keys;

/// Keytips are suppressed while the Backstage or a modal dialog is open, or while a text field
/// (not the canvas) has focus; the canvas lets Alt open keytips and `canvas_events` swallows
/// typing while they show. The ribbon control keytips focused themselves doesn't count.
pub fn suppressed(app: &WordApp, ctx: &egui::Context) -> bool {
    !app.ui.keytips_enabled
        || app.ui.backstage
        || app.dialog.is_some()
        || (ctx.egui_wants_keyboard_input() && !app.canvas.focused && keytip_focus(ctx).is_none())
}

/// Native backends report Alt through both key and modifier events, in either order. Track
/// the transition once; the key-up event can still carry the old `alt: true` modifier state.
/// A release is a keytip tap only if the Alt modifier was on during the press: AltGr (Right Alt
/// on Polish, German, Czech… layouts) is a Right Alt key without it on Linux and Ctrl+Alt on
/// Windows, and types characters instead (#2388).
fn alt_transition(app: &mut WordApp, pressed: bool, modifiers: egui::Modifiers) -> bool {
    let other_modifier = modifiers.ctrl || modifiers.shift || modifiers.command || modifiers.mac_cmd;
    if pressed {
        if !app.ui.alt_pressed {
            app.ui.alt_chord_used = false;
            app.ui.alt_modifier_seen = false;
        }
        app.ui.alt_pressed = true;
        app.ui.alt_chord_used |= other_modifier;
        app.ui.alt_modifier_seen |= modifiers.alt;
        false
    } else {
        let released = std::mem::take(&mut app.ui.alt_pressed);
        let seen = std::mem::take(&mut app.ui.alt_modifier_seen) || modifiers.alt;
        released && !std::mem::take(&mut app.ui.alt_chord_used) && !other_modifier && seen
    }
}

/// Read keytip input every frame, before layout, so the frame paints the new state. Suppression
/// cancels keytips so letters reach the text; Alt still reaches the rest of the UI (the
/// registry's Alt+letter shortcuts are untouched).
///
/// Keytips arm on a bare Alt *release* with no key pressed in between — the same chord Word
/// uses. On macOS they never arm from Alt at all, because Option is how you type accented
/// characters there and hijacking it would swallow the next letter.
pub fn logic(app: &mut WordApp, ctx: &egui::Context) {
    if suppressed(app, ctx) || !ctx.input(|i| i.focused) {
        // A ribbon control keytips focused hands the keyboard back to the page (the window
        // lost focus, say), or typing would go nowhere when it comes back.
        release_focus(app, ctx);
        app.ui.keytips = Phase::Off;
        app.ui.keytip_prefix.clear();
        app.ui.alt_pressed = false;
        app.ui.alt_chord_used = false;
        app.ui.alt_key_down = false;
        app.ui.alt_modifier_seen = false;
        app.keytip_rects.clear();
        ctx.data_mut(|d| {
            d.insert_temp(phase_id(), Phase::Off);
            d.remove::<Vec<(Rect, String)>>(rects_id());
            d.remove::<Phase>(focus_pending_id());
            d.remove::<egui::Id>(focused_id());
        });
        return;
    }
    let before = app.ui.keytips;
    // On macOS Option+key types accents, so Alt must never arm keytips there.
    const ALT_ARMS: bool = !cfg!(target_os = "macos");
    let events: Vec<egui::Event> = ctx.input(|i| i.events.clone());
    // Alt pressed or let go while dragging (an object, a selection) is part of the drag (Alt
    // moves an object freely), not a keytip tap.
    let dragging = app.canvas.obj_drag.is_some() || ctx.input(|i| i.pointer.any_down());
    let mut other_key = false;
    let mut alt_released = false;
    let mut f10 = false;
    for e in &events {
        match e {
            egui::Event::Key { key, pressed, modifiers, .. } => {
                if is_alt(*key) {
                    app.ui.alt_key_down = *pressed;
                    // Left Alt is never AltGr, whatever modifiers the event carries.
                    let modifiers = egui::Modifiers { alt: modifiers.alt || *key == egui::Key::AltLeft, ..*modifiers };
                    alt_released |= alt_transition(app, *pressed, modifiers);
                } else if *pressed {
                    other_key = true;
                    app.ui.alt_chord_used = true;
                    if *key == egui::Key::F10 && !modifiers.any() {
                        f10 = true;
                    }
                }
            }
            // Alt off while an Alt key is still down (AltGr on Linux turns Level 3 Shift on, not
            // Alt): the key's release ends the press, not this.
            egui::Event::ModifiersChanged(modifiers) if !app.ui.alt_key_down || modifiers.alt => {
                alt_released |= alt_transition(app, modifiers.alt, *modifiers);
            }
            egui::Event::PointerButton { pressed: true, .. } => {
                other_key = true;
                app.ui.alt_chord_used = true;
            }
            _ => {}
        }
        if dragging && app.ui.alt_pressed {
            app.ui.alt_chord_used = true;
        }
    }
    // Route letter/navigation keys to the keytips while they are showing.
    let mut consumed = Vec::new();
    for e in &events {
        if let egui::Event::Key { key, pressed: true, modifiers, .. } = e
            && app.ui.keytips != Phase::Off
            && !is_alt(*key)
            && (*key != egui::Key::F10 || modifiers.any())
            && handle_key(app, *key)
        {
            consumed.push(*key);
        }
    }
    // A badge can turn the mode off before canvas_events runs. Consume both the key and its
    // text event now, or the digit/letter would also be typed into the document (#620).
    if !consumed.is_empty() {
        ctx.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Key { key, pressed: true, .. } => !consumed.contains(key),
                egui::Event::Text(_) => false,
                _ => true,
            });
        });
    }
    if ALT_ARMS && alt_released && !other_key {
        app.ui.keytips = if app.ui.keytips == Phase::Tabs { Phase::Off } else { Phase::Tabs };
        app.ui.keytip_prefix.clear();
    } else if f10 {
        activate(app);
    }
    if app.ui.keytips == Phase::Off {
        app.ui.keytip_prefix.clear();
    }
    // The keyboard focus follows the keytips (#1012).
    match app.ui.keytips {
        now if now == before => {}
        Phase::Off => release_focus(app, ctx),
        now => ctx.data_mut(|d| {
            d.insert_temp(focus_pending_id(), now);
        }),
    }
    // Publish the state the ribbon widgets read to place their badges, and clear last frame's.
    ctx.data_mut(|d| {
        d.insert_temp(phase_id(), app.ui.keytips);
        d.insert_temp(tab_id(), app.ui.tab.clone());
        if matches!(app.ui.keytips, Phase::Commands) {
            d.insert_temp(letters_id().with(&app.ui.tab), letters_for(app, &app.ui.tab));
            d.insert_temp(rects_id(), Vec::<(egui::Rect, String)>::new());
        } else {
            d.remove::<Vec<(egui::Rect, String)>>(rects_id());
        }
    });
    app.keytip_rects.clear();
}

/// Paint the badges for the current phase over the ribbon. Called after the ribbon is laid out,
/// so every badge sits exactly on its own control and can never overlap another one.
pub fn show(app: &mut WordApp, ctx: &egui::Context, ui: &mut Ui) {
    if app.ui.keytips == Phase::Off || suppressed(app, ctx) {
        return;
    }
    // Tab badges were recorded during layout; command badges were published to egui memory by
    // the widgets themselves (they own their rects).
    let tabs = std::mem::take(&mut app.keytip_rects);
    if app.ui.keytips == Phase::Legacy {
        legacy_hint(app, ui, tabs.iter().map(|(r, _)| *r).reduce(|a, b| a.union(b)));
        return;
    }
    let cmds: Vec<(Rect, String)> = ui.data(|d| d.get_temp::<Vec<(Rect, String)>>(rects_id())).unwrap_or_default();
    for (r, l) in tabs.into_iter().chain(cmds) {
        badge(ui, r, &l);
    }
}

/// Where the legacy hint hangs from: the tab row, or, with no ribbon showing (Focus, Full Screen
/// View), the top of the window under the slim bar, so Alt, V still says it's waiting.
fn legacy_hint_anchor(tabs: Option<Rect>, window: Rect) -> Rect {
    tabs.unwrap_or_else(|| Rect::from_min_size(window.left_top(), Vec2::new(0.0, crate::chrome::SCREEN_BAR_HEIGHT)))
}

/// The hint under the tab row while a legacy menu key sequence waits for its next letter.
fn legacy_hint(app: &WordApp, ui: &Ui, tabs: Option<Rect>) {
    let tabs = legacy_hint_anchor(tabs, ui.ctx().content_rect());
    let t = Tokens::get(ui.ctx());
    let keys: Vec<String> = app.ui.keytip_prefix.chars().map(String::from).collect();
    let text = crate::i18n::fmt(tl!("Old menu keys: Alt, {keys}. Keep typing, or press Esc to cancel."), &[("keys", &keys.join(", "))]);
    let font = egui::FontId::proportional(12.0);
    let galley = ui.ctx().fonts_mut(|f| f.layout_no_wrap(text, font, t.text));
    let r = Rect::from_min_size(tabs.left_bottom() + Vec2::new(4.0, 2.0), galley.size() + Vec2::new(16.0, 10.0));
    let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("wc_legacy_keys")));
    p.rect_filled(r, CornerRadius::same(4), t.menu);
    p.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    p.galley(r.min + Vec2::new(8.0, 5.0), galley, t.text);
}

#[cfg(test)]
mod tests;
