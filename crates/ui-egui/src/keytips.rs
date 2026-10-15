//! Word-style keytips: Alt (or F10) shows letter badges over ribbon tabs, then over a tab's
//! controls; numbered badges also reach the customized Quick Access Toolbar. Typing never
//! reaches the document while active.

use egui::{Align2, CornerRadius, Rect, Stroke, Ui, Vec2};

use crate::WordApp;
use crate::theme::{Tokens, semibold};

/// egui ids the ribbon publishes its keytip state under (accessors, since `Id::new` isn't const).
pub fn phase_id() -> egui::Id {
    egui::Id::new("wc_keytips_phase")
}
pub fn tab_id() -> egui::Id {
    egui::Id::new("wc_keytips_tab")
}
pub fn letters_id() -> egui::Id {
    egui::Id::new("wc_keytips_letters")
}
pub fn rects_id() -> egui::Id {
    egui::Id::new("wc_keytips_rects")
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
            c("Drop Cap", "insert.dropCap"),
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
            p("Previous Record", "mailings.previous", serde_json::json!({})),
            p("Next Record", "mailings.next", serde_json::json!({})),
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
            vec![
                m("Shading", "table.shading"),
                m("Border Styles", "table.borderStyles"),
                m("Borders", "table.borders"),
                c("Border Painter", "table.borderPainter"),
            ]
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
            v.extend(w.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()));
        }
        if words.len() > 1 {
            v.extend(words.iter().map(|w| w.chars().next().unwrap_or(' ')).filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()));
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
    out.into_iter().map(|o| o.unwrap_or_default()).collect()
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
            egui::Key::Escape => {
                app.ui.keytips = Phase::Off;
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
                            None => app.ui.keytips = Phase::Off,
                        }
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
    let ids = app.session.ribbon.qat();
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
/// typing while they show.
pub fn suppressed(app: &WordApp, ctx: &egui::Context) -> bool {
    !app.ui.keytips_enabled || app.ui.backstage || app.dialog.is_some() || (ctx.egui_wants_keyboard_input() && !app.canvas.focused)
}

/// Native backends report Alt through both key and modifier events, in either order. Track
/// the transition once; the key-up event can still carry the old `alt: true` modifier state.
fn alt_transition(app: &mut WordApp, pressed: bool, modifiers: egui::Modifiers) -> bool {
    let other_modifier = modifiers.ctrl || modifiers.shift || modifiers.command || modifiers.mac_cmd;
    if pressed {
        if !app.ui.alt_pressed {
            app.ui.alt_chord_used = false;
        }
        app.ui.alt_pressed = true;
        app.ui.alt_chord_used |= other_modifier;
        false
    } else {
        let released = std::mem::take(&mut app.ui.alt_pressed);
        released && !std::mem::take(&mut app.ui.alt_chord_used) && !other_modifier
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
        app.ui.keytips = Phase::Off;
        app.ui.keytip_prefix.clear();
        app.ui.alt_pressed = false;
        app.ui.alt_chord_used = false;
        app.keytip_rects.clear();
        ctx.data_mut(|d| {
            d.insert_temp(phase_id(), Phase::Off);
            d.remove::<Vec<(Rect, String)>>(rects_id());
        });
        return;
    }
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
                    alt_released |= alt_transition(app, *pressed, *modifiers);
                } else if *pressed {
                    other_key = true;
                    app.ui.alt_chord_used = true;
                    if *key == egui::Key::F10 && !modifiers.any() {
                        f10 = true;
                    }
                }
            }
            egui::Event::ModifiersChanged(modifiers) => {
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
    let cmds: Vec<(Rect, String)> = ui.data(|d| d.get_temp::<Vec<(Rect, String)>>(rects_id())).unwrap_or_default();
    for (r, l) in tabs.into_iter().chain(cmds) {
        badge(ui, r, &l);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> WordApp {
        WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default())
    }

    fn key_event(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
    }

    fn input_frame(ctx: &egui::Context, app: &mut WordApp, events: Vec<egui::Event>) {
        ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| logic(app, ui.ctx())).drop_without_applying_deltas();
    }

    #[test]
    fn native_alt_taps_toggle_once_in_either_event_order() {
        let alt = egui::Modifiers { alt: true, ..Default::default() };
        // Key-up can precede ModifiersChanged and still carry alt=true. Some backends send
        // only modifier events; a fast tap can also fit entirely in a single frame.
        for key in [None, Some(egui::Key::AltLeft), Some(egui::Key::AltRight)] {
            for modifiers_first in [false, true] {
                for same_frame in [false, true] {
                    let ctx = egui::Context::default();
                    let mut a = app();
                    let mut press = vec![egui::Event::ModifiersChanged(alt)];
                    let mut release = vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)];
                    if let Some(key) = key {
                        if modifiers_first {
                            press.push(key_event(key, true, alt));
                            release.push(key_event(key, false, egui::Modifiers::NONE));
                        } else {
                            press.insert(0, key_event(key, true, egui::Modifiers::NONE));
                            release.insert(0, key_event(key, false, alt));
                        }
                    }
                    for expected in [if cfg!(target_os = "macos") { Phase::Off } else { Phase::Tabs }, Phase::Off] {
                        if same_frame {
                            input_frame(&ctx, &mut a, press.iter().chain(&release).cloned().collect());
                        } else {
                            input_frame(&ctx, &mut a, press.clone());
                            input_frame(&ctx, &mut a, release.clone());
                        }
                        assert_eq!(a.ui.keytips, expected, "key={key:?}, modifiers_first={modifiers_first}, same_frame={same_frame}");
                        assert!(!a.ui.alt_pressed);
                    }
                }
            }
        }
    }

    #[test]
    fn native_alt_chords_and_interrupted_taps_do_not_activate() {
        let ctx = egui::Context::default();
        let mut a = app();
        let alt = egui::Modifiers { alt: true, ..Default::default() };
        for modifier in [egui::Modifiers::CTRL, egui::Modifiers::SHIFT, egui::Modifiers::MAC_CMD] {
            input_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(modifier | alt)]);
            // Releasing the other modifier first must not turn this into a bare Alt tap.
            input_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(alt)]);
            input_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
            assert_eq!(a.ui.keytips, Phase::Off);
        }
        for event in [
            key_event(egui::Key::ArrowDown, true, alt),
            egui::Event::PointerButton { pos: egui::pos2(400.0, 300.0), button: egui::PointerButton::Primary, pressed: true, modifiers: alt },
        ] {
            input_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(alt)]);
            input_frame(&ctx, &mut a, vec![event]);
            // A repeated key-down must not forget the key/mouse chord.
            input_frame(&ctx, &mut a, vec![key_event(egui::Key::AltLeft, true, alt)]);
            input_frame(&ctx, &mut a, vec![key_event(egui::Key::AltLeft, false, alt), egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
            assert_eq!(a.ui.keytips, Phase::Off);
        }
        input_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(alt)]);
        ctx.run_ui(egui::RawInput { focused: false, events: vec![egui::Event::WindowFocused(false)], ..Default::default() }, |ui| {
            logic(&mut a, ui.ctx())
        })
        .drop_without_applying_deltas();
        input_frame(&ctx, &mut a, vec![key_event(egui::Key::AltLeft, false, alt), egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        assert_eq!(a.ui.keytips, Phase::Off, "returning after Alt+Tab must not arm keytips");
        input_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(alt)]);
        a.ui.backstage = true;
        input_frame(&ctx, &mut a, vec![]);
        a.ui.backstage = false;
        input_frame(&ctx, &mut a, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        assert_eq!(a.ui.keytips, Phase::Off, "suppression cancels the pending Alt tap");
    }

    #[test]
    fn tab_letters_match_word() {
        // Observed in Word for Windows (Microsoft 365): Alt shows these over the tabs.
        assert_eq!(tab_letter("File"), "F");
        assert_eq!(tab_letter("Home"), "H");
        assert_eq!(tab_letter("Insert"), "N");
        assert_eq!(tab_letter("Design"), "G");
        assert_eq!(tab_letter("Layout"), "P");
        assert_eq!(tab_letter("References"), "S");
        assert_eq!(tab_letter("Mailings"), "M");
        assert_eq!(tab_letter("Review"), "R");
        assert_eq!(tab_letter("View"), "W");
        assert_eq!(tab_letter("Help"), "Y");
        assert_eq!(tab_letter("Table Design"), "JT");
        assert_eq!(tab_letter("Table Layout"), "JL");
    }

    #[test]
    fn every_keytip_command_is_registered() {
        // The keytip tables name commands by id; a renamed or removed command must fail here,
        // not silently do nothing when its badge is pressed. Features not built yet are fine
        // when the Word feature catalog lists them (the ribbon shows them too); `ui.*` commands are
        // the app's own (`WordApp::run`).
        let a = app();
        let catalog: Vec<&str> = wordcraft_engine::catalog::entries().into_iter().map(|e| e.3).collect();
        let mut missing = Vec::new();
        for (tab, _) in TAB_LETTERS.iter().chain(&TABLE_TAB_LETTERS) {
            for ctrl in controls(tab) {
                if let Target::Cmd { id, .. } = ctrl.target
                    && a.session.registry.get(id).is_none()
                    && !catalog.contains(&id)
                    && !id.starts_with("ui.")
                {
                    missing.push(format!("{tab}: {id}"));
                }
            }
        }
        assert!(missing.is_empty(), "unregistered: {missing:#?}");
    }

    #[test]
    fn letters_are_unique_inside_every_tab() {
        let a = app();
        let (tabs, cmds) = letters(&a);
        for (_, ls) in &cmds {
            let mut seen = Vec::new();
            for l in ls {
                assert!(!seen.contains(l), "duplicate badge letter {l}");
                seen.push(l.clone());
            }
        }
        let mut tl = Vec::new();
        for (tab, l) in &tabs {
            assert!(!l.is_empty(), "tab {tab} has no badge");
            assert!(!tl.contains(l), "duplicate tab letter {l}");
            tl.push(l.clone());
        }
    }

    #[test]
    fn keytips_never_reuse_another_commands_shortcut_letter() {
        let a = app();
        let reserved = reserved_letters(&a);
        let (_, cmds) = letters(&a);
        for (tab, ls) in &cmds {
            let ctrls = controls(tab);
            let ids: Vec<String> = ctrls
                .iter()
                .map(|c| match &c.target {
                    Target::Cmd { id, .. } => (*id).to_string(),
                    _ => "ui.dialog".to_string(),
                })
                .collect();
            for (i, l) in ls.iter().enumerate() {
                let id = ids.get(i).map(|s| s.as_str()).unwrap_or("");
                if let Some(sc) = shortcut_letter(&a, id) {
                    assert_eq!(l, &sc.to_string(), "{tab}: {id} should keep its shortcut letter {sc}");
                } else if reserved.contains(&l.chars().next().unwrap_or('?')) {
                    panic!("{tab}: badge {l} collides with another command's shortcut");
                }
            }
        }
    }

    /// Alt+drag selects a column (#237); releasing Alt afterwards must not open keytips. A bare
    /// Alt tap still does (off macOS, where Option types accents).
    #[test]
    fn alt_click_or_drag_does_not_arm_keytips_on_release() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.ui.backstage = false;
        a.dialog = None;
        let alt = egui::Modifiers { alt: true, ..Default::default() };
        let key = |pressed, modifiers| egui::Event::Key { key: egui::Key::AltLeft, physical_key: None, pressed, repeat: false, modifiers };
        let pos = egui::pos2(400.0, 300.0);
        let frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            let input = egui::RawInput { events, ..Default::default() };
            ctx.run_ui(input, |ui| logic(a, ui.ctx())).drop_without_applying_deltas();
        };
        // Alt down, drag with the mouse over several frames, Alt up.
        frame(&mut a, vec![key(true, alt)]);
        frame(&mut a, vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: alt }]);
        frame(&mut a, vec![egui::Event::PointerMoved(pos + egui::vec2(60.0, 40.0))]);
        frame(&mut a, vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: alt }]);
        frame(&mut a, vec![key(false, egui::Modifiers::NONE)]);
        assert_eq!(a.ui.keytips, Phase::Off, "Alt+drag then Alt up leaves keytips off");
        // Alt held while a key is pressed in a later frame: also a chord.
        frame(&mut a, vec![key(true, alt)]);
        frame(&mut a, vec![egui::Event::Key { key: egui::Key::ArrowDown, physical_key: None, pressed: true, repeat: false, modifiers: alt }]);
        frame(&mut a, vec![key(false, egui::Modifiers::NONE)]);
        assert_eq!(a.ui.keytips, Phase::Off, "Alt+key then Alt up leaves keytips off");
        // A bare tap (press and release in separate frames) arms them.
        frame(&mut a, vec![key(true, alt)]);
        frame(&mut a, vec![key(false, egui::Modifiers::NONE)]);
        let tap = if cfg!(target_os = "macos") { Phase::Off } else { Phase::Tabs };
        assert_eq!(a.ui.keytips, tap);
    }

    #[test]
    fn keytip_state_machine() {
        let mut a = app();
        a.ui.backstage = false;
        a.dialog = None;
        a.canvas.focused = false;
        activate(&mut a);
        assert_eq!(a.ui.keytips, Phase::Tabs);
        assert!(handle_key(&mut a, egui::Key::Escape));
        assert_eq!(a.ui.keytips, Phase::Off);
        activate(&mut a);
        assert!(handle_key(&mut a, egui::Key::H));
        assert_eq!(a.ui.tab, "Home");
        assert_eq!(a.ui.keytips, Phase::Commands);
        assert!(handle_key(&mut a, egui::Key::F));
        assert_eq!(a.ui.keytips, Phase::Off);
        // Alt, F: the Backstage, and keytips end.
        a.ui.backstage = false;
        activate(&mut a);
        assert!(handle_key(&mut a, egui::Key::F));
        assert!(a.ui.backstage, "Alt, F opens File");
        assert_eq!(a.ui.keytips, Phase::Off);
        // Alt, W: View; Alt, R: Review (Word's letters, not V and S).
        a.ui.backstage = false;
        activate(&mut a);
        assert!(handle_key(&mut a, egui::Key::W));
        assert_eq!((a.ui.tab.as_str(), a.ui.keytips), ("View", Phase::Commands));
        a.ui.keytips = Phase::Off;
        activate(&mut a);
        assert!(handle_key(&mut a, egui::Key::R));
        assert_eq!(a.ui.tab, "Review");
    }

    /// Word's contextual table tabs take two letters (#1183): Alt, J, T is Table Design and
    /// Alt, J, L is Table Layout. J alone waits for the second letter.
    #[test]
    fn alt_j_t_and_alt_j_l_open_the_table_tabs() {
        let mut a = app();
        a.run("insert.table", serde_json::json!({"rows": 2, "cols": 2})).unwrap();
        assert!(crate::ribbon::in_table_public(&a));
        for (second, tab) in [(egui::Key::T, "Table Design"), (egui::Key::L, "Table Layout")] {
            a.run("ui.tab", serde_json::json!({"tab": "Home"})).unwrap();
            activate(&mut a);
            assert!(handle_key(&mut a, egui::Key::J));
            assert_eq!((a.ui.tab.as_str(), a.ui.keytips), ("Home", Phase::Tabs), "J waits for the second letter");
            assert!(handle_key(&mut a, second));
            assert_eq!((a.ui.tab.as_str(), a.ui.keytips), (tab, Phase::Commands));
            assert!(a.ui.keytip_prefix.is_empty());
            assert!(!letters_for(&a, tab).is_empty(), "{tab} badges its controls");
            a.ui.keytips = Phase::Off;
        }
        // J then a letter no table tab uses ends keytips without switching tabs.
        a.run("ui.tab", serde_json::json!({"tab": "Home"})).unwrap();
        activate(&mut a);
        handle_key(&mut a, egui::Key::J);
        handle_key(&mut a, egui::Key::X);
        assert_eq!((a.ui.tab.as_str(), a.ui.keytips), ("Home", Phase::Off));
        assert!(a.ui.keytip_prefix.is_empty());
        // Outside a table there is no J tab.
        let mut b = app();
        activate(&mut b);
        handle_key(&mut b, egui::Key::J);
        assert_eq!(b.ui.keytips, Phase::Off);
    }

    #[test]
    fn qat_keytips_follow_custom_order_and_skip_disabled_commands() {
        let mut a = app();
        a.run("tools.customizeRibbon", serde_json::json!({"qatAdd": "format.bold"})).unwrap();
        a.run("tools.customizeRibbon", serde_json::json!({"qatAdd": "format.italic"})).unwrap();
        a.run("tools.customizeRibbon", serde_json::json!({"qatMove": {"command": "format.bold", "by": -4}})).unwrap();
        let bold_index = a.session.ribbon.qat().iter().position(|id| *id == "format.bold").unwrap();
        assert_eq!(bold_index, 0);
        activate(&mut a);
        handle_key(&mut a, egui::Key::Num1);
        assert_eq!(a.session.run("format.state", &serde_json::json!({})).unwrap()["bold"], true);
        assert_eq!(a.ui.keytips, Phase::Off);

        let mut b = app();
        let before = b.session.doc.clone();
        activate(&mut b);
        handle_key(&mut b, egui::Key::Num2); // Undo is disabled in an untouched document.
        assert_eq!(b.session.doc, before);
        assert!(!b.session.can_redo());
        assert_eq!(b.ui.keytips, Phase::Off);
    }

    #[test]
    fn long_qat_badges_wait_for_the_complete_number_and_escape_cancels() {
        let mut a = app();
        let mut commands: Vec<String> =
            a.session.registry.all().iter().filter(|s| s.id != "format.bold").take(19).map(|s| s.id.to_string()).collect();
        commands.insert(9, "format.bold".into());
        a.session.ribbon.qat = Some(commands);
        activate(&mut a);
        handle_key(&mut a, egui::Key::Num0);
        handle_key(&mut a, egui::Key::Num0);
        assert_eq!(a.ui.keytips, Phase::Tabs);
        assert_eq!(a.session.run("format.state", &serde_json::json!({})).unwrap()["bold"], false);
        handle_key(&mut a, egui::Key::Num1); // 001: first button after the first nine.
        assert_eq!(a.session.run("format.state", &serde_json::json!({})).unwrap()["bold"], true);
        assert_eq!(a.ui.keytips, Phase::Off);
        activate(&mut a);
        handle_key(&mut a, egui::Key::Num0);
        handle_key(&mut a, egui::Key::Escape);
        assert!(a.ui.keytip_prefix.is_empty());
        assert_eq!(a.ui.keytips, Phase::Off);

        for count in [9, 10, 18, 19, 100, 200] {
            let labels: Vec<_> = (0..count).map(|i| qat_label(i, count)).collect();
            for (i, label) in labels.iter().enumerate() {
                assert!(
                    !labels.iter().enumerate().any(|(j, other)| i != j && other.starts_with(label)),
                    "ambiguous label {label} in {count} buttons"
                );
            }
        }
    }

    #[test]
    fn qat_digit_runs_once_and_never_types_into_the_document() {
        let mut a = app();
        a.session.ribbon.qat = Some(vec!["text.insert".into(), "format.bold".into()]);
        a.canvas.focused = true;
        a.canvas.want_focus = true;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        h.input_mut().events.push(key(egui::Key::F10));
        h.step();
        assert_eq!(h.state().ui.keytips, Phase::Tabs);
        h.input_mut().events.extend([key(egui::Key::Num2), egui::Event::Text("2".into())]);
        h.step();
        assert_eq!(h.state().ui.keytips, Phase::Off);
        assert_eq!(h.state_mut().session.run("format.state", &serde_json::json!({})).unwrap()["bold"], true);
        assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).is_empty());
        // Normal typing resumes immediately after the command.
        h.input_mut().events.push(egui::Event::Text("hello".into()));
        h.step();
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "hello");
        if !cfg!(target_os = "macos") {
            let alt = egui::Modifiers { alt: true, ..Default::default() };
            h.input_mut().events.extend([key_event(egui::Key::AltLeft, true, egui::Modifiers::NONE), egui::Event::ModifiersChanged(alt)]);
            h.step();
            h.input_mut().events.extend([key_event(egui::Key::AltLeft, false, alt), egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
            h.step();
            assert_eq!(h.state().ui.keytips, Phase::Tabs);
            h.input_mut().events.extend([key(egui::Key::Num2), egui::Event::Text("2".into())]);
            h.step();
            assert_eq!(h.state_mut().session.run("format.state", &serde_json::json!({})).unwrap()["bold"], false);
            assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "hello");
        }
    }

    #[test]
    fn keytips_option_persists_and_disabled_keytips_leave_f10_alone() {
        let mut a = app();
        activate(&mut a);
        a.run("ui.keytips", serde_json::json!({"value": false})).unwrap();
        assert_eq!(a.ui.keytips, Phase::Off);
        assert!(a.run("ui.keytips", serde_json::json!({"value": "false"})).is_err());
        let saved = serde_json::to_vec(&a.prefs()).unwrap();
        let mut b = app();
        b.apply_prefs(serde_json::from_slice(&saved).unwrap());
        activate(&mut b);
        assert_eq!(b.ui.keytips, Phase::Off);
        assert_eq!(b.run("ui.keytips", serde_json::json!({})).unwrap()["enabled"], false);
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::F10,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| logic(&mut b, ui.ctx()),
        );
        output.textures_delta.clear();
        assert_eq!(b.ui.keytips, Phase::Off);
        b.apply_prefs(serde_json::from_str("{}").unwrap());
        assert!(b.ui.keytips_enabled, "older preferences retain enabled KeyTips");
    }

    /// Alt, then Q opens Search like Alt+Q (#819); no tab owns Q.
    #[test]
    fn alt_then_q_opens_search() {
        let mut a = app();
        a.ui.backstage = false;
        a.dialog = None;
        assert!(TAB_LETTERS.iter().all(|(_, l)| *l != SEARCH_LETTER));
        activate(&mut a);
        assert!(handle_key(&mut a, egui::Key::Q));
        assert_eq!(a.ui.keytips, Phase::Off);
        assert!(matches!(a.dialog, Some(crate::dialogs::Dialog::Commands { .. })), "{:?}", a.dialog.as_ref().map(|d| d.name()));
    }
}
