//! Inline calculations in the interface: the source box (Insert › Calculation, and a double-click
//! on a calculation), the unique-name prompt it shows before committing, Insert › Calculated
//! Value, Calculation Options, the tooltip on an errored or out-of-date region, the bar of a
//! region edited outside WordCraft, the right-click item that makes a table calculation data and
//! the marker on a data table's erroring heading.
//!
//! All the work is the engine's: the box ends in `insert.calculation` or `calc.setSource`, the
//! prompt asks `calc.checkName`, the picker runs `insert.calcValue`, the options `calc.settings`,
//! the bar `calc.resolveEdited` and the table item `calc.dataTable`. Shown through
//! the calculation dialog.

use egui::{Pos2, Ui, vec2};
use serde::Serialize;
use serde_json::{Value, json};
use wordcraft_doc::calc::{CalcRegion, CalcState, ColumnError, NumberStyle, ResultFormat, UnitSystem};
use wordcraft_doc::{Document, InlineObject, Pos, StoryRef};
use wordcraft_engine::Session;

use crate::WordApp;

/// The definition mark the box shows for a typed `:=` (U+2254 COLON EQUALS). The parser reads
/// either; what is stored is the ASCII `:=`.
pub(crate) const DEFINE_MARK: char = '\u{2254}';

/// `∧` and `∨` (U+2227, U+2228): the shortcuts and the Insert Symbol row offer them, and the box
/// shows them while the interface can draw them ([`logic_covered`]); else it shows the words
/// `and` and `or`, which the engine reads the same.
pub(crate) const AND_MARK: char = '\u{2227}';
pub(crate) const OR_MARK: char = '\u{2228}';

/// How many `<name>_N` the rename suggestion tries before leaving the field to the user.
const MAX_SUGGESTIONS: u32 = 99;

/// The object character a calculation sits on in its paragraph's text.
const OBJ_LEN: usize = '\u{FFFC}'.len_utf8();

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "form", rename_all = "camelCase")]
pub enum CalcDialog {
    /// The source box: a new calculation, or a region's source (`editing`).
    Source(CalcBox),
    /// Insert › Calculated Value.
    Value(ValueForm),
    /// References › Calculation › Options.
    Options(OptionsForm),
    ResultFormat(ResultFormatForm),
    Plot(PlotForm),
}

impl CalcDialog {
    pub fn name(&self) -> &'static str {
        match self {
            CalcDialog::Source(b) if b.editing.is_some() => "editCalculation",
            CalcDialog::Source(_) => "calculation",
            CalcDialog::Value(_) => "calcValue",
            CalcDialog::Options(_) => "calcOptions",
            CalcDialog::ResultFormat(_) => "calcResultFormat",
            CalcDialog::Plot(f) if f.editing => "editFunctionPlot",
            CalcDialog::Plot(_) => "functionPlot",
        }
    }

    /// The window title (English; the caller translates it).
    pub fn title(&self) -> &'static str {
        match self {
            CalcDialog::Source(b) if b.editing.is_some() => "Edit Calculation",
            CalcDialog::Source(_) => "Calculation",
            CalcDialog::Value(_) => "Calculated Value",
            CalcDialog::Options(_) => "Calculation Options",
            CalcDialog::ResultFormat(_) => "Result Format",
            CalcDialog::Plot(f) if f.editing => "Edit Function Plot",
            CalcDialog::Plot(_) => "Function Plot",
        }
    }

    /// Where the window goes: the source box opens at the caret; the others are centred.
    pub fn anchor(&self) -> Option<Pos2> {
        match self {
            CalcDialog::Source(b) => b.anchor,
            _ => None,
        }
    }
}

/// The dialog `ui.dialog` opens by `name`: `calculation` (a new one at the caret),
/// `editCalculation` (the calculation at the caret), `calcValue` or `calcOptions`.
pub fn open(name: &str, app: &mut WordApp) -> Option<CalcDialog> {
    Some(match name {
        "calculation" => CalcDialog::Source(CalcBox::new(None, "", caret_anchor(app))),
        "editCalculation" => {
            let (_, region) = calc_at_caret(app)?;
            CalcDialog::Source(CalcBox::new(Some(region.id), &region.source, caret_anchor(app)))
        }
        "calcValue" => CalcDialog::Value(ValueForm::read(app)),
        "calcOptions" => CalcDialog::Options(OptionsForm::read(&app.session.doc)),
        "functionPlot" => CalcDialog::Plot(PlotForm { editing: false, spec: Default::default(), message: String::new() }),
        "editFunctionPlot" => CalcDialog::Plot(PlotForm {
            editing: true,
            spec: wordcraft_engine::cmd::chart::selected_chart(&app.session)?.plot.clone()?,
            message: String::new(),
        }),
        "calcResultFormat" => {
            let (_, region) = calc_at_caret(app)?;
            let settings = app.session.doc.calc_settings.clone().unwrap_or_default();
            let default = settings.result_format.unwrap_or(ResultFormat { decimals: settings.precision.min(15), ..Default::default() });
            let format = region.result_format.unwrap_or(ResultFormat { decimals: region.precision.unwrap_or(default.decimals), ..default });
            CalcDialog::ResultFormat(ResultFormatForm {
                id: region.id,
                inherit: region.result_format.is_none() && region.precision.is_none(),
                format,
                message: String::new(),
            })
        }
        _ => return None,
    })
}

pub(crate) fn open_dialog(app: &mut WordApp, name: &str) -> bool {
    if let Some(dialog) = open(name, app) {
        app.calc_dialog = Some(dialog);
        true
    } else {
        false
    }
}

pub(crate) fn show_dialog(app: &mut WordApp, ctx: &egui::Context) {
    let Some(mut dialog) = app.calc_dialog.take() else { return };
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("calc-key-frame"), frame));
    let id = egui::Id::unique(("calculation-dialog", dialog.name()));
    static SUPERSCRIPTS: std::sync::Mutex<FaceMemo> = std::sync::Mutex::new(None);
    install_font(ctx, "calc-define", define_mark_face());
    install_font(ctx, "calc-logic", logic_mark_face());
    install_font(ctx, "calc-superscripts", borrowed_face(&SUPERSCRIPTS, &['⁻', '¹']));
    let mut open = true;
    let mut window = egui::Window::new(crate::tl!(dialog.title())).id(id).collapsible(false).resizable(false).open(&mut open);
    window = if let Some(at) = dialog.anchor() { window.fixed_pos(at) } else { window.anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO) };
    let closed = window
        .show(ctx, |ui| {
            ui.set_min_width(340.0);
            body(app, ui, &mut dialog)
        })
        .and_then(|response| response.inner)
        .unwrap_or(false);
    if open && !closed {
        app.calc_dialog = Some(dialog);
    } else {
        ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            input.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
        });
        app.canvas.focus_page_after_pass();
    }
}

pub(crate) fn held_keys_this_frame(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<u64>(egui::Id::unique("calc-key-frame"))) == Some(ctx.cumulative_frame_nr())
}

fn install_font(ctx: &egui::Context, name: &str, face: Option<std::sync::Arc<wordcraft_fonts::FontFace>>) {
    let key = egui::Id::unique(name);
    if let Some(face) = face
        && !ctx.data(|data| data.get_temp::<bool>(key)).unwrap_or(false)
    {
        let mut font = egui::FontData::from_owned(face.data().to_vec());
        font.index = face.index();
        use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
        ctx.add_font(FontInsert::new(name, font, vec![InsertFontFamily { family: egui::FontFamily::Proportional, priority: FontPriority::Lowest }]));
        ctx.data_mut(|data| data.insert_temp(key, true));
    }
}

fn cancel_key_pressed(ui: &Ui) -> bool {
    ui.input(|input| {
        input.key_pressed(egui::Key::Escape) || (cfg!(target_os = "macos") && input.modifiers.command && input.key_pressed(egui::Key::Period))
    })
}

fn buttons(ui: &mut Ui, label: &str) -> (bool, bool) {
    ui.add_space(8.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let cancel = ui.button(tl!("Cancel")).clicked() || cancel_key_pressed(ui);
        let confirm = ui.button(label).clicked() || ui.input(|input| input.key_pressed(egui::Key::Enter));
        (confirm, cancel)
    })
    .inner
}

/// Draws the dialog; returns true to close it.
pub fn body(app: &mut WordApp, ui: &mut Ui, d: &mut CalcDialog) -> bool {
    match d {
        CalcDialog::Source(b) => source_ui(app, ui, b),
        CalcDialog::Value(f) => value_ui(app, ui, f),
        CalcDialog::Options(f) => options_ui(app, ui, f),
        CalcDialog::ResultFormat(f) => result_format_ui(app, ui, f),
        CalcDialog::Plot(f) => plot_ui(app, ui, f),
    }
}

/// Just below the caret, on screen.
fn caret_anchor(app: &mut WordApp) -> Option<Pos2> {
    crate::canvas::caret_screen(app).map(|(p, h)| p + vec2(0.0, h + 4.0))
}

// ---------------------------------------------------------------------------------------------
// Sources and names

/// The source as the box shows it: `:=` as `≔` when the interface font can draw `≔` (`covered`),
/// else `≔` as `:=` (a missing glyph would show as a box).
pub fn display_source(source: &str, covered: bool) -> String {
    if covered { source.replace(":=", &DEFINE_MARK.to_string()) } else { stored_source(source) }
}

/// The source as it is stored: `≔` back to `:=`.
pub fn stored_source(shown: &str) -> String {
    shown.replace(DEFINE_MARK, ":=")
}

/// Whether the interface can draw `∧` and `∨` (`has_glyph` answers for one character).
pub fn logic_covered(mut has_glyph: impl FnMut(char) -> bool) -> bool {
    has_glyph(AND_MARK) && has_glyph(OR_MARK)
}

/// `∧` and `∨` written as the words `and` and `or`, outside quoted texts, with a space kept on
/// each side; and where a cursor at char index `cursor` goes in the result. For a box whose
/// font can't draw the symbols (a missing glyph would show as a box).
pub fn spell_logic(text: &str, cursor: usize) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut out_len = 0;
    let mut at = None;
    let mut in_text = false;
    for (i, &c) in chars.iter().enumerate() {
        if i == cursor {
            at = Some(out_len);
        }
        let word = match c {
            '"' => {
                in_text = !in_text;
                None
            }
            '\u{201C}' => {
                in_text = true;
                None
            }
            '\u{201D}' => {
                in_text = false;
                None
            }
            AND_MARK if !in_text => Some("and"),
            OR_MARK if !in_text => Some("or"),
            _ => None,
        };
        match word {
            Some(word) => {
                if chars.get(i.wrapping_sub(1)).is_some_and(|p| !p.is_whitespace()) && !out.ends_with(char::is_whitespace) {
                    out.push(' ');
                    out_len += 1;
                }
                out.push_str(word);
                out_len += word.len();
                if !chars.get(i + 1).is_some_and(|n| n.is_whitespace()) {
                    out.push(' ');
                    out_len += 1;
                }
            }
            None => {
                out.push(c);
                out_len += 1;
            }
        }
    }
    (out, at.unwrap_or(out_len))
}

/// The names a source defines (`σ` for `σ := P/A_b =`, `a` and `b` for `[a; b] := …`), in
/// order; none when it defines nothing or doesn't parse (its error then shows at the region).
pub fn defined_names_of(source: &str) -> Vec<String> {
    let Ok(stmt) = wordcraft_calc::parse(source) else { return Vec::new() };
    wordcraft_calc::names_defined_by(&stmt).into_iter().map(str::to_string).collect()
}

/// The source with its defined name `old` (in the head before `:=` or `≔`) renamed `new`; a
/// function's parameters stay (`f(x) := …` → `g(x) := …`), and in `[a; b] := …` the other names
/// do. `None` when the head doesn't start with the whole name (or, for `[a; b]`, hold it).
pub fn rename_source(source: &str, old: &str, new: &str) -> Option<String> {
    let head_end = [source.find(":="), source.find(DEFINE_MARK)].into_iter().flatten().min()?;
    let head = source.get(..head_end)?;
    let lead = head.len() - head.trim_start().len();
    if head.get(lead..)?.starts_with('[') {
        return rename_in_list(source, head_end, old, new);
    }
    let rest = head.get(lead..)?.strip_prefix(old)?;
    // The whole name: `Px` doesn't start with the name `P`.
    if rest.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let after = source.get(lead + old.len()..)?;
    Some(format!("{}{new}{after}", source.get(..lead)?))
}

/// [`rename_source`] for the head `[a; b; …]` (`source` up to `head_end`): the one name between
/// `[`, `;` and `]` that is all of `old`.
fn rename_in_list(source: &str, head_end: usize, old: &str, new: &str) -> Option<String> {
    let head = source.get(..head_end)?;
    let mut start = head.find('[')? + 1;
    for piece in head.get(start..)?.split([';', ']']) {
        let name_at = start + (piece.len() - piece.trim_start().len());
        if piece.trim() == old {
            return Some(format!("{}{new}{}", source.get(..name_at)?, source.get(name_at + old.len()..)?));
        }
        start += piece.len() + 1;
    }
    None
}

/// The first of `<name>_1`, `<name>_2`, … up to `<name>_99` that `calc.checkName` accepts (for the
/// region `editing`, if one is being edited); `None` if none is.
pub fn suggest_name(session: &mut Session, name: &str, editing: Option<u64>) -> Option<String> {
    (1..=MAX_SUGGESTIONS).map(|n| format!("{name}_{n}")).find(|candidate| check_name(session, candidate, editing).is_ok_and(|v| is_ok(&v)))
}

fn check_name(session: &mut Session, name: &str, editing: Option<u64>) -> Result<Value, String> {
    let mut q = json!({"name": name});
    if let Some(id) = editing {
        q["id"] = json!(id);
    }
    session.run("calc.checkName", &q).map_err(|e| e.to_string())
}

fn is_ok(v: &Value) -> bool {
    v.get("ok").and_then(Value::as_bool).unwrap_or(false)
}

type FaceMemo = Option<(u64, Option<std::sync::Arc<wordcraft_fonts::FontFace>>)>;

/// A face that covers every one of `chars`, which the interface fonts lack: the math font
/// equations are drawn with, else any document font that does. Looked up once per
/// [`wordcraft_fonts::FontDb::generation`] (kept in `memo`).
fn borrowed_face(memo: &std::sync::Mutex<FaceMemo>, chars: &[char]) -> Option<std::sync::Arc<wordcraft_fonts::FontFace>> {
    let db = wordcraft_fonts::FontDb::global();
    let generation = db.generation();
    let mut memo = memo.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((g, face)) = memo.as_ref()
        && *g == generation
    {
        return face.clone();
    }
    // Very large faces (system CJK collections) aren't borrowed by the interface.
    let usable = |f: &std::sync::Arc<wordcraft_fonts::FontFace>| chars.iter().all(|c| f.covers(*c)) && f.data().len() <= 16 << 20;
    let (math, _) = wordcraft_fonts::math::resolve_math("Cambria Math");
    let face =
        Some(db.face(&math.family, &math.style)).filter(usable).or_else(|| chars.first().and_then(|c| db.fallback_for(*c, u32::MAX)).filter(usable));
    *memo = Some((generation, face.clone()));
    face
}

/// The face the interface borrows to draw `≔` in the source box (its own fonts have none).
pub(crate) fn define_mark_face() -> Option<std::sync::Arc<wordcraft_fonts::FontFace>> {
    static MEMO: std::sync::Mutex<FaceMemo> = std::sync::Mutex::new(None);
    borrowed_face(&MEMO, &[DEFINE_MARK])
}

/// The face the interface borrows to draw `∧` and `∨` in the box and the Insert Symbol row.
pub(crate) fn logic_mark_face() -> Option<std::sync::Arc<wordcraft_fonts::FontFace>> {
    static MEMO: std::sync::Mutex<FaceMemo> = std::sync::Mutex::new(None);
    borrowed_face(&MEMO, &[AND_MARK, OR_MARK])
}

// ---------------------------------------------------------------------------------------------
// Finding regions

/// The calculation region of the equation at `at`, if it is one.
pub fn region_at<'a>(doc: &'a Document, at: &Pos) -> Option<&'a CalcRegion> {
    match doc.para_at(at)?.object_at(at.off)? {
        InlineObject::Equation { calc: Some(c), .. } => Some(c),
        _ => None,
    }
}

/// Where the body region `id` sits (found the way the engine finds it, table cells at any depth).
pub fn region_pos(doc: &Document, id: u64) -> Option<Pos> {
    wordcraft_calc::body_regions(doc).into_iter().find(|(_, _, c)| c.id == id).map(|(path, off, _)| Pos {
        story: StoryRef::Body,
        path: wordcraft_doc::Path(path),
        off,
    })
}

/// The calculation just after or just before the caret (a click on one puts the caret before
/// it), with where it sits.
fn calc_at_caret(app: &WordApp) -> Option<(Pos, CalcRegion)> {
    let focus = &app.session.sel.focus;
    let before = focus.off.checked_sub(OBJ_LEN).map(|off| Pos { off, ..focus.clone() });
    std::iter::once(focus.clone()).chain(before).find_map(|at| region_at(&app.session.doc, &at).map(|c| (at.clone(), c.clone())))
}

/// What the right-click menu adds when the caret is at a calculation: editing its source, inline
/// or display (inline lets text follow it on the line, e.g. a tab and a note), the alignment of a
/// display calculation, its equation number (numbering works like numbering a plain equation) and
/// recalculating. Empty anywhere else.
pub fn menu_items(app: &WordApp) -> Vec<(&'static str, &'static str, Value)> {
    let Some((at, c)) = calc_at_caret(app) else { return Vec::new() };
    let display = matches!(app.session.doc.para_at(&at).and_then(|p| p.object_at(at.off)), Some(InlineObject::Equation { display: true, .. }));
    let mut items = vec![("Edit Calculation", "calc.edit", json!({})), ("Result Format", "calc.resultFormat", json!({}))];
    if display {
        items.push(("Change to Inline", "equation.display", json!({"value": false})));
        items.push(("Align Left", "equation.justify", json!({"jc": "left"})));
        items.push(("Align Center", "equation.justify", json!({"jc": "centerGroup"})));
        items.push(("Align Right", "equation.justify", json!({"jc": "right"})));
    } else {
        items.push(("Change to Display", "equation.display", json!({"value": true})));
    }
    let number = if c.numbered { "Remove Equation Number" } else { "Add Equation Number" };
    items.push((number, "equation.number", json!({"value": !c.numbered})));
    items.push(("Recalculate", "calc.recalculate", json!({})));
    items
}

/// Whether the selection is exactly one calculation (its object character, either way round).
pub fn selected_calc(app: &WordApp) -> bool {
    let sel = &app.session.sel;
    let (a, b) = if sel.anchor.off <= sel.focus.off { (&sel.anchor, &sel.focus) } else { (&sel.focus, &sel.anchor) };
    a.story == b.story && a.path == b.path && b.off == a.off + OBJ_LEN && region_at(&app.session.doc, a).is_some()
}

/// The right-click menu's item for the innermost table at the caret: Use as Calculation Data, or
/// Stop Using as Calculation Data when it already is, with its `calc.dataTable` parameters. `None`
/// outside a table in the main text (the command refuses there).
pub fn data_table_item(app: &WordApp) -> Option<(&'static str, Value)> {
    let focus = &app.session.sel.focus;
    if focus.story != StoryRef::Body {
        return None;
    }
    let (table, _, _) = focus.path.cell()?;
    let marked = app.session.doc.table(StoryRef::Body, &table)?.calc.is_some();
    Some(if marked { ("Stop Using as Calculation Data", json!({"on": false})) } else { ("Use as Calculation Data", json!({"on": true})) })
}

// ---------------------------------------------------------------------------------------------
// The source box

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalcBox {
    /// What the box shows (`:=` as `≔` where the font has it; see [`display_source`] and
    /// [`stored_source`]).
    pub source: String,
    /// The region whose source this is; `None` for a new calculation.
    pub editing: Option<u64>,
    /// The unique-name prompt, while it shows.
    pub prompt: Option<NamePrompt>,
    /// Why the engine refused the last commit.
    pub message: String,
    /// Put the keyboard in the box on the next frame.
    #[serde(skip)]
    pub focus: bool,
    #[serde(skip)]
    pub anchor: Option<Pos2>,
    /// Where the box's cursor last was (char index), so a symbol picked from the menu goes there.
    #[serde(skip)]
    pub cursor: Option<usize>,
    /// Move the box's cursor to [`CalcBox::cursor`] on the next frame (after a symbol is inserted).
    #[serde(skip)]
    pub place_cursor: bool,
}

/// The symbols the box's menu offers, by group: Greek letters, operators, the units that are hard
/// to type, and the built-in constants (`g` is gravity unless the document chose the SI gram).
pub const SYMBOL_GROUPS: &[(&str, &[&str])] = &[
    (
        "Greek",
        &[
            "α", "β", "γ", "δ", "ε", "ζ", "η", "θ", "κ", "λ", "μ", "ν", "ξ", "ρ", "σ", "τ", "υ", "φ", "χ", "ψ", "ω", "Γ", "Δ", "Θ", "Λ", "Ξ", "Σ",
            "Φ", "Ψ", "Ω",
        ],
    ),
    ("Operators", &["≔", "·", "×", "±", "√", "²", "³", "⁻¹", "|", "°", "≤", "≥", "≠", "∑", "\u{2227}", "\u{2228}", "¬"]),
    ("Units", &["gm", "°F", "°C", "Δ°F", "Δ°C", "µm", "Å", "Ω", "%"]),
    ("Constants", &["π", "e", "g", "g_0", "c_0", "R_u"]),
];

/// What an Insert Symbol button says: the symbol, but `and` or `or` for `∧` and `∨` where the
/// interface can't draw them (`logic` is false).
pub fn symbol_label(symbol: &str, logic: bool) -> &str {
    match symbol {
        _ if logic => symbol,
        "\u{2227}" => "and",
        "\u{2228}" => "or",
        _ => symbol,
    }
}

/// `\name` shortcuts the box turns into symbols, as in Word's math autocorrect.
const SHORTCUTS: &[(&str, &str)] = &[
    ("alpha", "α"),
    ("beta", "β"),
    ("gamma", "γ"),
    ("delta", "δ"),
    ("epsilon", "ε"),
    ("zeta", "ζ"),
    ("eta", "η"),
    ("theta", "θ"),
    ("iota", "ι"),
    ("kappa", "κ"),
    ("lambda", "λ"),
    ("mu", "μ"),
    ("nu", "ν"),
    ("xi", "ξ"),
    ("pi", "π"),
    ("rho", "ρ"),
    ("sigma", "σ"),
    ("tau", "τ"),
    ("upsilon", "υ"),
    ("phi", "φ"),
    ("chi", "χ"),
    ("psi", "ψ"),
    ("omega", "ω"),
    ("Gamma", "Γ"),
    ("Delta", "Δ"),
    ("Theta", "Θ"),
    ("Lambda", "Λ"),
    ("Xi", "Ξ"),
    ("Pi", "Π"),
    ("Sigma", "Σ"),
    ("Phi", "Φ"),
    ("Psi", "Ψ"),
    ("Omega", "Ω"),
    ("sqrt", "√"),
    ("cdot", "·"),
    ("times", "×"),
    ("pm", "±"),
    ("deg", "°"),
    ("le", "≤"),
    ("ge", "≥"),
    ("ne", "≠"),
    ("sum", "∑"),
    ("wedge", "\u{2227}"),
    ("vee", "\u{2228}"),
    ("neg", "¬"),
];

/// A `\name` just finished at char index `cursor` (the character before the cursor ends it) turned
/// into its symbol: the new text and cursor. A space that ends the name is used up; any other
/// character stays. `None` when nothing before the cursor is a known shortcut.
pub fn expand_shortcut(text: &str, cursor: usize) -> Option<(String, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let end = cursor.checked_sub(1)?;
    let ender = *chars.get(end)?;
    if ender.is_alphanumeric() || ender == '\\' {
        return None;
    }
    let start = chars[..end].iter().rposition(|c| !c.is_ascii_alphabetic())?;
    if chars.get(start) != Some(&'\\') || start + 1 == end {
        return None;
    }
    let name: String = chars[start + 1..end].iter().collect();
    let symbol = SHORTCUTS.iter().find(|(n, _)| *n == name)?.1;
    let replacement = if ender == ' ' { symbol.to_string() } else { format!("{symbol}{ender}") };
    let mut out: String = chars[..start].iter().collect();
    out.push_str(&replacement);
    out.extend(chars.get(cursor..).unwrap_or_default());
    Some((out, start + replacement.chars().count()))
}

/// The unique-name prompt: the clash message and a new name to rename the definition to.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamePrompt {
    /// The name the source defines.
    pub name: String,
    /// `duplicate` or `reserved`.
    pub reason: String,
    pub message: String,
    /// The region that already defines the name (duplicates).
    pub existing: Option<u64>,
    /// The field, pre-filled with the first free `<name>_N`.
    pub new_name: String,
    /// Why Rename couldn't rewrite the source.
    pub error: String,
    #[serde(skip)]
    pub focus: bool,
}

/// What a commit led to.
#[derive(Clone, Debug, PartialEq)]
pub enum Commit {
    /// Inserted or changed: the box closes.
    Done,
    /// The name clashes: the prompt shows.
    Prompt,
    /// Refused (the message says why), or nothing to commit: the box stays.
    Stay,
}

impl CalcBox {
    pub fn new(editing: Option<u64>, source: &str, anchor: Option<Pos2>) -> CalcBox {
        CalcBox { source: source.to_string(), editing, prompt: None, message: String::new(), focus: true, anchor, cursor: None, place_cursor: false }
    }

    /// Put `symbol` at the box's cursor (at the end before the box has had one), leaving the
    /// cursor after it and the keyboard back in the box.
    pub fn insert_symbol(&mut self, symbol: &str) {
        let len = self.source.chars().count();
        let at = self.cursor.unwrap_or(len).min(len);
        let byte = self.source.char_indices().nth(at).map_or(self.source.len(), |(i, _)| i);
        self.source.insert_str(byte, symbol);
        self.cursor = Some(at + symbol.chars().count());
        self.place_cursor = true;
        self.focus = true;
    }

    /// Show `∧` and `∨` as the words `and` and `or` (see [`spell_logic`]), the cursor kept.
    pub fn spell_logic_marks(&mut self) {
        let cursor = self.cursor.unwrap_or_else(|| self.source.chars().count());
        let (text, at) = spell_logic(&self.source, cursor);
        self.source = text;
        if self.cursor.is_some() {
            self.cursor = Some(at);
        }
    }

    /// Enter: check the defined name, then insert the calculation or change the region's source.
    /// A clashing name shows the prompt instead; nothing is inserted under it.
    pub fn commit(&mut self, app: &mut WordApp) -> Commit {
        let source = stored_source(&self.source);
        if source.trim().is_empty() {
            return Commit::Stay;
        }
        // Each name of `[a; b] := …` in turn: the first that clashes gets the prompt.
        for name in defined_names_of(&source) {
            match check_name(&mut app.session, &name, self.editing) {
                Ok(v) if !is_ok(&v) => {
                    self.prompt = Some(NamePrompt::new(app, &name, &v, self.editing));
                    return Commit::Prompt;
                }
                Ok(_) => {}
                Err(e) => {
                    self.message = e;
                    return Commit::Stay;
                }
            }
        }
        let r = match self.editing {
            Some(id) => app.run("calc.setSource", json!({"id": id, "source": source})),
            None => app.run("insert.calculation", json!({"source": source})),
        };
        match r {
            Ok(_) => Commit::Done,
            Err(e) => {
                self.message = e;
                Commit::Stay
            }
        }
    }

    /// Rename: rewrite the defined name in the source to the prompt's new name, then commit
    /// (which checks the new name in turn).
    pub fn rename(&mut self, app: &mut WordApp) -> Commit {
        let Some(p) = self.prompt.as_mut() else { return Commit::Stay };
        let new_name = p.new_name.trim().to_string();
        match rename_source(&stored_source(&self.source), &p.name, &new_name) {
            Some(renamed) if !new_name.is_empty() => {
                self.source = renamed;
                self.prompt = None;
                self.commit(app)
            }
            _ => {
                p.error = crate::i18n::fmt(tl!("{name} can't be renamed in this calculation"), &[("name", &p.name)]);
                Commit::Stay
            }
        }
    }

    /// Go to existing: the caret moves to the region that defines the name, and the box closes.
    pub fn go_to_existing(&self, app: &mut WordApp) -> bool {
        let Some(at) = self.prompt.as_ref().and_then(|p| p.existing).and_then(|id| region_pos(&app.session.doc, id)) else { return false };
        app.run("caret.set", json!({"pos": at})).is_ok()
    }
}

impl NamePrompt {
    fn new(app: &mut WordApp, name: &str, check: &Value, editing: Option<u64>) -> NamePrompt {
        let reason = check.get("reason").and_then(Value::as_str).unwrap_or("reserved").to_string();
        let existing = check.get("existing").and_then(Value::as_u64);
        let mut message = check.get("message").and_then(Value::as_str).unwrap_or_default().to_string();
        // The engine checks without a layout; the page the existing definition is on helps.
        if reason == "duplicate"
            && let Some(page) = existing.and_then(|id| region_pos(&app.session.doc, id)).and_then(|at| page_number(app, &at))
        {
            message = crate::i18n::fmt(
                tl!("{name} is already defined on page {page} — choose a unique name"),
                &[("name", name), ("page", &page.to_string())],
            );
        }
        let new_name = suggest_name(&mut app.session, name, editing).unwrap_or_else(|| name.to_string());
        NamePrompt { name: name.to_string(), reason, message, existing, new_name, error: String::new(), focus: true }
    }
}

/// The page number (as shown) the position is on.
fn page_number(app: &mut WordApp, at: &Pos) -> Option<u32> {
    let layout = app.session.layout();
    layout.page_of(at).and_then(|i| layout.pages.get(i)).map(|p| p.number)
}

/// The box's text with `:=` shown as `≔` (or back, see [`display_source`]), and where a cursor
/// at char index `cursor` goes in it: `:=` becoming `≔` before the cursor moves it back one.
pub fn show_define_mark(text: &str, cursor: usize, covered: bool) -> (String, usize) {
    let shown = display_source(text, covered);
    let before: String = text.chars().take(cursor).collect();
    // A cursor between `:` and `=` lands after the mark.
    (shown, display_source(&before, covered).chars().count())
}

/// Applies [`show_define_mark`] to the box after an edit, moving its cursor to match.
fn fix_define_mark(ui: &Ui, out: &mut egui::text_edit::TextEditOutput, text: &mut String, covered: bool) {
    if display_source(text, covered) == *text {
        return;
    }
    let cursor: usize = out.cursor_range.map_or(text.chars().count(), |r| r.primary.index.into());
    let (shown, at) = show_define_mark(text, cursor, covered);
    *text = shown;
    out.state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
    out.state.clone().store(ui.ctx(), out.response.id);
}

fn error_line(ui: &mut Ui, message: &str) {
    if !message.is_empty() {
        ui.colored_label(ui.visuals().error_fg_color, message);
    }
}

fn source_ui(app: &mut WordApp, ui: &mut Ui, b: &mut CalcBox) -> bool {
    if b.prompt.is_some() {
        return prompt_ui(app, ui, b);
    }
    let label = ui.label(tl!("Calculation"));
    let font = egui::TextStyle::Body.resolve(ui.style());
    let covered = ui.fonts_mut(|f| f.has_glyph(&font, DEFINE_MARK));
    let logic = ui.fonts_mut(|f| logic_covered(|c| f.has_glyph(&font, c)));
    // What the font can't draw never reaches the box (a source read from a file may have `≔`).
    if !covered && b.source.contains(DEFINE_MARK) {
        b.source = stored_source(&b.source);
    }
    if !logic && b.source.contains([AND_MARK, OR_MARK]) {
        b.spell_logic_marks();
    }
    let mut out = egui::TextEdit::singleline(&mut b.source)
        .font(egui::TextStyle::Body)
        .id(egui::Id::unique("calc_source_box"))
        .desired_width(380.0)
        .hint_text(tl!("For example: P := 1500 lbf"))
        .show(ui);
    out.response.response = out.response.response.clone().labelled_by(label.id);
    if b.focus && !ui.input(|input| input.pointer.any_down()) {
        b.focus = false;
        out.response.request_focus();
    }
    if out.response.changed() {
        b.message.clear();
        let cursor: usize = out.cursor_range.map_or(b.source.chars().count(), |r| r.primary.index.into());
        if let Some((text, at)) = expand_shortcut(&b.source, cursor) {
            b.source = text;
            b.cursor = Some(at);
            b.place_cursor = true;
            // A `\wedge` or `\vee` the font can't draw never reaches the screen as a symbol.
            if !logic {
                b.spell_logic_marks();
            }
        }
    }
    if std::mem::take(&mut b.place_cursor) {
        let at = b.cursor.unwrap_or(b.source.chars().count());
        out.state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
        out.state.clone().store(ui.ctx(), out.response.id);
    }
    fix_define_mark(ui, &mut out, &mut b.source, covered);
    if let Some(r) = out.cursor_range {
        b.cursor = Some(r.primary.index.into());
    }
    ui.menu_button(tl!("Insert Symbol"), |ui| {
        for (group, symbols) in SYMBOL_GROUPS {
            ui.label(egui::RichText::new(tl!(*group)).small().weak());
            ui.horizontal_wrapped(|ui| {
                for s in *symbols {
                    if ui.button(symbol_label(s, logic)).clicked() {
                        b.insert_symbol(s);
                        if !logic {
                            b.spell_logic_marks();
                        }
                        ui.close();
                    }
                }
            });
        }
        ui.label(egui::RichText::new(tl!("Tip: type \\sigma, \\theta, \\Delta… then a space.")).small().weak());
    });
    error_line(ui, &b.message);
    ui.label(egui::RichText::new(tl!("Define with :=, show a result with = at the end (σ := P/A =).")).small().weak());
    let ok = if b.editing.is_some() { tl!("Update") } else { tl!("Insert") };
    let (ok, cancel) = buttons(ui, ok);
    if cancel {
        return true;
    }
    if ok {
        match b.commit(app) {
            Commit::Done => return true,
            Commit::Prompt => {}
            Commit::Stay => b.focus = true,
        }
    }
    false
}

fn prompt_ui(app: &mut WordApp, ui: &mut Ui, b: &mut CalcBox) -> bool {
    let Some(p) = b.prompt.as_mut() else { return false };
    ui.label(egui::RichText::new(stored_source(&b.source)).monospace());
    ui.add_space(4.0);
    error_line(ui, &p.message);
    ui.add_space(4.0);
    let label = ui.label(tl!("New name"));
    let field = ui.add(egui::TextEdit::singleline(&mut p.new_name).desired_width(240.0)).labelled_by(label.id);
    if std::mem::take(&mut p.focus) {
        field.request_focus();
    }
    if field.changed() {
        p.error.clear();
    }
    error_line(ui, &p.error);
    let duplicate = p.reason == "duplicate" && p.existing.is_some();
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let escape = cancel_key_pressed(ui);
    let (mut rename, mut go, mut cancel) = (enter, false, escape);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            cancel |= ui.button(tl!("Cancel")).clicked();
            if duplicate {
                go |= ui.button(tl!("Go to existing")).clicked();
            }
            rename |= ui.button(tl!("Rename")).clicked();
        });
    });
    if cancel {
        // Back to the box, with the source as typed.
        b.prompt = None;
        b.focus = true;
        return false;
    }
    if go {
        return b.go_to_existing(app);
    }
    if rename {
        match b.rename(app) {
            Commit::Done => return true,
            Commit::Prompt => {}
            Commit::Stay => {
                if b.prompt.is_none() {
                    b.focus = true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------------------------
// Insert › Calculated Value

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueForm {
    /// Every variable a value can quote (`calc.names`).
    pub names: Vec<String>,
    pub name: String,
    /// Unit to show the value in; empty: the variable's own.
    pub unit: String,
    /// Decimal places of its own (else the document's).
    pub precision_on: bool,
    pub precision: u8,
    /// The element to quote, as typed (`2`, or a row and a column `1, 0`); empty: the whole value.
    pub index: String,
    pub message: String,
}

/// The Element box's text as an `insert.calcValue` index: `None` when empty, else one or two
/// whole numbers separated by a comma. Anything else is refused with a hint.
pub fn parse_index(text: &str) -> Result<Option<Vec<u32>>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let parts: Option<Vec<u32>> = text.split(',').map(|p| p.trim().parse::<u32>().ok()).collect();
    match parts {
        Some(index) if (1..=2).contains(&index.len()) => Ok(Some(index)),
        _ => Err(tl!("Element: a whole number, or a row and a column such as 1, 0").to_string()),
    }
}

impl ValueForm {
    pub fn read(app: &mut WordApp) -> ValueForm {
        let names: Vec<String> = app
            .session
            .run("calc.names", &json!({}))
            .ok()
            .and_then(|v| v.get("names").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|n| n.get("name").and_then(Value::as_str).map(str::to_string))
            .collect();
        let precision = app.session.doc.calc_settings.clone().unwrap_or_default().precision.min(15);
        ValueForm {
            name: names.first().cloned().unwrap_or_default(),
            names,
            unit: String::new(),
            precision_on: false,
            precision,
            index: String::new(),
            message: String::new(),
        }
    }

    /// The `insert.calcValue` parameters (the Element box's index when it reads; [`parse_index`]
    /// refuses the rest before anything is sent).
    pub fn params(&self) -> Value {
        let mut p = json!({"name": self.name});
        if !self.unit.trim().is_empty() {
            p["unit"] = json!(self.unit.trim());
        }
        if self.precision_on {
            p["precision"] = json!(self.precision.min(15));
        }
        if let Ok(Some(index)) = parse_index(&self.index) {
            p["index"] = json!(index);
        }
        p
    }
}

fn value_ui(app: &mut WordApp, ui: &mut Ui, f: &mut ValueForm) -> bool {
    if f.names.is_empty() {
        ui.label(tl!("No calculation defines a variable yet. Insert one with Insert › Calculation."));
    } else {
        egui::Grid::new("calc_value_grid").num_columns(2).spacing(vec2(8.0, 6.0)).show(ui, |ui| {
            use craft::ui::contract::{Bind, Entry, Show as _};
            use craft::ui::pickers::Select;
            use craft::ui::runtime::env::Label;
            ui.label(tl!("Variable"));
            let entries: Vec<_> = f.names.iter().map(|name| Entry::new(name.clone(), Label::from(name.clone()))).collect();
            if let Some(name) = Select::new(Label::from(tl!("Variable")), Bind::given(f.name.clone()), &entries).show(ui).value() {
                f.name = name.clone();
            }
            ui.end_row();
            let label = ui.label(tl!("Unit (optional)"));
            ui.add(egui::TextEdit::singleline(&mut f.unit).desired_width(120.0)).labelled_by(label.id);
            ui.end_row();
            let own = ui.checkbox(&mut f.precision_on, tl!("Decimal places"));
            ui.add_enabled(f.precision_on, egui::DragValue::new(&mut f.precision).range(0..=15)).labelled_by(own.id);
            ui.end_row();
            // Optional: one element of a vector, or a row and a column of a matrix, counted from
            // the document's origin. The command refuses it on a single value.
            let label = ui.label(tl!("Element"));
            let field =
                ui.add(egui::TextEdit::singleline(&mut f.index).desired_width(120.0).hint_text(tl!("optional: 2, or 1, 0"))).labelled_by(label.id);
            if field.changed() {
                f.message.clear();
            }
            ui.end_row();
        });
    }
    error_line(ui, &f.message);
    let (ok, cancel) = buttons(ui, tl!("Insert"));
    if cancel {
        return true;
    }
    if ok && !f.names.is_empty() {
        if let Err(hint) = parse_index(&f.index) {
            f.message = hint;
            return false;
        }
        match app.run("insert.calcValue", f.params()) {
            Ok(_) => return true,
            Err(e) => f.message = e,
        }
    }
    false
}

// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, Serialize)]
pub struct ResultFormatForm {
    id: u64,
    inherit: bool,
    format: ResultFormat,
    message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlotForm {
    editing: bool,
    spec: wordcraft_doc::chart::PlotSpec,
    message: String,
}

fn plot_ui(app: &mut WordApp, ui: &mut Ui, f: &mut PlotForm) -> bool {
    for (label, value) in [
        (tl!("Independent variable"), &mut f.spec.variable),
        (tl!("Function or expression"), &mut f.spec.expression),
        (tl!("Range or vector"), &mut f.spec.range),
    ] {
        let label = ui.label(label);
        ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY)).labelled_by(label.id);
    }
    ui.label(tl!("Example range: 0, 0.1..10"));
    error_line(ui, &f.message);
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if cancel {
        return true;
    }
    if ok {
        let command = if f.editing { "chart.editFunctionPlot" } else { "insert.functionPlot" };
        match app.run(command, json!(f.spec)) {
            Ok(_) => return true,
            Err(error) => f.message = error,
        }
    }
    false
}

fn format_inputs(ui: &mut Ui, format: &mut ResultFormat) {
    let label = |style| match style {
        NumberStyle::General => tl!("General"),
        NumberStyle::Decimal => tl!("Decimal"),
        NumberStyle::Scientific => tl!("Scientific"),
        NumberStyle::Engineering => tl!("Engineering"),
        NumberStyle::Fraction => tl!("Fraction"),
    };
    let name = ui.label(tl!("Number format"));
    egui::ComboBox::from_id_salt("number-format")
        .selected_text(label(format.style))
        .show_ui(ui, |ui| {
            for style in [NumberStyle::General, NumberStyle::Decimal, NumberStyle::Scientific, NumberStyle::Engineering, NumberStyle::Fraction] {
                ui.selectable_value(&mut format.style, style, label(style));
            }
        })
        .response
        .labelled_by(name.id);
    let selector = |ui: &mut Ui, name, value: &mut u8| {
        ui.horizontal(|ui| {
            let label = ui.label(name);
            ui.add(egui::DragValue::new(value).range(0..=15)).labelled_by(label.id);
        });
    };
    if format.style == NumberStyle::Fraction {
        selector(ui, tl!("Level of accuracy"), &mut format.accuracy);
        ui.checkbox(&mut format.mixed_numbers, tl!("Use mixed numbers"));
    } else {
        selector(ui, tl!("Decimal places"), &mut format.decimals);
        ui.checkbox(&mut format.trailing_zeros, tl!("Show trailing zeros"));
        if format.style == NumberStyle::General {
            ui.checkbox(&mut format.engineering_exponents, tl!("Use engineering exponents"));
            selector(ui, tl!("Exponential threshold"), &mut format.exponential_threshold);
        }
        if matches!(format.style, NumberStyle::Scientific | NumberStyle::Engineering) {
            ui.checkbox(&mut format.e_notation, tl!("Show exponents as E±000"));
        }
    }
}

fn result_format_ui(app: &mut WordApp, ui: &mut Ui, f: &mut ResultFormatForm) -> bool {
    ui.checkbox(&mut f.inherit, tl!("Use document defaults"));
    ui.add_enabled_ui(!f.inherit, |ui| format_inputs(ui, &mut f.format));
    error_line(ui, &f.message);
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if cancel {
        return true;
    }
    if ok {
        let format = (!f.inherit).then_some(f.format);
        match app.run("calc.resultFormat", json!({"id": f.id, "resultFormat": format})) {
            Ok(_) => return true,
            Err(error) => f.message = error,
        }
    }
    false
}

// Calculation Options

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionsForm {
    /// `us` or `si`.
    pub system: String,
    /// Decimal places, 0–15.
    pub precision: u8,
    pub custom_format: bool,
    pub format: ResultFormat,
    /// Recalculate only on request.
    pub manual: bool,
    /// What `g` means: `gravity` (Mathcad) or `gram` (SI).
    pub g: String,
    /// Where arrays count from: 0 (Mathcad) or 1.
    pub origin: u8,
    pub message: String,
}

impl OptionsForm {
    /// The document's settings as they are (read from the document: `calc.settings` would
    /// recalculate).
    pub fn read(doc: &Document) -> OptionsForm {
        let s = doc.calc_settings.clone().unwrap_or_default();
        let system = match s.system {
            UnitSystem::UsCustomary => "us",
            UnitSystem::Si => "si",
        };
        let g = match s.g {
            wordcraft_doc::calc::GMeans::Gravity => "gravity",
            wordcraft_doc::calc::GMeans::Gram => "gram",
        };
        OptionsForm {
            system: system.into(),
            precision: s.precision.min(15),
            custom_format: s.result_format.is_some(),
            format: s.result_format.unwrap_or(ResultFormat { decimals: s.precision.min(15), ..Default::default() }),
            manual: s.manual,
            g: g.into(),
            origin: s.origin.min(1),
            message: String::new(),
        }
    }

    /// The `calc.settings` parameters.
    pub fn params(&self) -> Value {
        json!({"system": self.system, "precision": self.precision.min(15), "manual": self.manual, "g": self.g, "origin": self.origin.min(1), "resultFormat": self.custom_format.then_some(self.format)})
    }
}

fn options_ui(app: &mut WordApp, ui: &mut Ui, f: &mut OptionsForm) -> bool {
    ui.label(egui::RichText::new(tl!("Show results in")).strong());
    ui.radio_value(&mut f.system, "us".to_string(), tl!("US customary"));
    ui.radio_value(&mut f.system, "si".to_string(), tl!("SI (metric)"));
    ui.add_space(6.0);
    ui.checkbox(&mut f.custom_format, tl!("Customize the default result format"));
    if f.custom_format {
        format_inputs(ui, &mut f.format);
    } else {
        ui.horizontal(|ui| {
            let label = ui.label(tl!("Decimal places"));
            if ui.add(egui::DragValue::new(&mut f.precision).range(0..=15)).labelled_by(label.id).changed() {
                f.format.decimals = f.precision;
            }
        });
    }
    ui.add_space(6.0);
    ui.checkbox(&mut f.manual, tl!("Recalculate manually"))
        .on_hover_text(tl!("Results update only on References › Recalculate; until then they show out of date"));
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("g means")).strong());
    ui.radio_value(&mut f.g, "gravity".to_string(), tl!("Standard gravity (Mathcad): grams are gm"));
    ui.radio_value(&mut f.g, "gram".to_string(), tl!("Gram (SI): gravity is g_0"));
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("Arrays count from")).strong());
    ui.radio_value(&mut f.origin, 0, tl!("0 (Mathcad)"));
    ui.radio_value(&mut f.origin, 1, "1");
    ui.label(egui::RichText::new(tl!("Changing this renumbers every index, including quoted elements.")).small().weak());
    error_line(ui, &f.message);
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if cancel {
        return true;
    }
    if ok {
        match app.run("calc.settings", f.params()) {
            Ok(_) => return true,
            Err(e) => f.message = e,
        }
    }
    false
}

// ---------------------------------------------------------------------------------------------
// On the page: tooltips, the edited-outside bar and data tables' error markers

/// What hovering a region shows: an error's message, or that the result is out of date. An
/// edited-outside region has its bar instead ([`bar`]).
pub fn hover_text(state: &CalcState) -> Option<String> {
    match state {
        CalcState::Error(message) => Some(message.clone()),
        CalcState::Stale => Some(tl!("Out of date — recalculate (References › Recalculate)").to_string()),
        CalcState::Fresh | CalcState::EditedOutside => None,
    }
}

/// The edited-outside region the pointer is on, kept while the pointer is on it or on its bar.
#[derive(Clone)]
struct HoverBar {
    id: u64,
    at: Pos,
    /// The frame the pointer was last on the region.
    frame: u64,
    /// Where the bar was drawn last (screen).
    rect: Option<egui::Rect>,
}

fn bar_key() -> egui::Id {
    egui::Id::unique("calc_edited_bar")
}

/// The pointer is on the page at (`page`, `x`, `y`): over a calculation, an error or a stale
/// result shows as a tooltip, and an edited-outside region brings up its bar.
pub(crate) fn hover(app: &mut WordApp, ui: &Ui) {
    let ctx = ui.ctx();
    if app.modal_open() || app.canvas.context_menu_open || ctx.input(|input| input.pointer.any_down()) {
        return;
    }
    let Some(pointer) = ctx.pointer_hover_pos().filter(|p| app.canvas.shown.view.contains(*p)) else { return };
    let Some(page) = app.canvas.shown.pages.iter().find(|page| page.rect.contains(pointer)) else { return };
    let point = page.to_page(pointer);
    let page_index = page.page;
    let story = app.session.sel.focus.story;
    let Some((at, _)) = app.session.layout().equation_hit(page_index, point.x as f32, point.y as f32, story) else { return };
    let Some(region) = region_at(&app.session.doc, &at) else { return };
    if region.state == CalcState::EditedOutside {
        let rect = ctx.data(|d| d.get_temp::<HoverBar>(bar_key())).filter(|h| h.id == region.id).and_then(|h| h.rect);
        let frame = ctx.cumulative_frame_nr();
        ctx.data_mut(|d| d.insert_temp(bar_key(), HoverBar { id: region.id, at, frame, rect }));
        return;
    }
    if let Some(text) = hover_text(&region.state) {
        egui::Tooltip::always_open(ctx.clone(), ui.layer_id(), egui::Id::unique("calc_tip"), egui::PopupAnchor::Pointer).show(|ui| {
            ui.label(text);
        });
    }
}

/// The edited-outside region whose bar shows from the caret: the one the caret is on.
pub fn bar_target(app: &WordApp) -> Option<(u64, Pos)> {
    if app.session.view.read_mode || !app.session.sel.is_collapsed() {
        return None;
    }
    calc_at_caret(app).filter(|(_, c)| c.state == CalcState::EditedOutside).map(|(at, c)| (c.id, at))
}

/// The bar under a region edited outside WordCraft (the one hovered, else the one at the caret):
/// Restore from calculation, or Keep Word's equation (`calc.resolveEdited`).
pub(crate) fn bar(app: &mut WordApp, ctx: &egui::Context) {
    // Not over a dialog (the source box opens at the caret, where the bar would be).
    if app.modal_open() {
        return;
    }
    let pointer = ctx.pointer_hover_pos();
    let frame = ctx.cumulative_frame_nr();
    let hovered = ctx
        .data(|d| d.get_temp::<HoverBar>(bar_key()))
        .filter(|h| h.frame == frame || pointer.is_some_and(|p| h.rect.is_some_and(|r| r.expand(8.0).contains(p))));
    let target = hovered.as_ref().map(|h| (h.id, h.at.clone())).or_else(|| bar_target(app));
    let still_edited =
        |app: &WordApp, id: u64, at: &Pos| region_at(&app.session.doc, at).is_some_and(|c| c.id == id && c.state == CalcState::EditedOutside);
    let Some((id, at)) = target.filter(|(id, at)| still_edited(app, *id, at)) else {
        ctx.data_mut(|d| d.remove::<HoverBar>(bar_key()));
        return;
    };
    let hint = app.session.page_hint;
    let Some(eq) = app.session.layout().equation_box(&at, hint) else { return };
    let Some(anchor) = crate::canvas::page_to_screen(app, eq.page, eq.x, eq.bottom) else { return };
    let mut keep = None;
    let shown = egui::Area::new(bar_key().with("area")).order(egui::Order::Foreground).fixed_pos(anchor + vec2(0.0, 2.0)).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(tl!("This calculation was edited outside WordCraft."));
                if ui.button(tl!("Restore from calculation")).clicked() {
                    keep = Some("source");
                }
                if ui.button(tl!("Keep Word's equation")).clicked() {
                    keep = Some("word");
                }
            });
        });
    });
    match hovered {
        Some(mut h) => {
            h.rect = Some(shown.response.rect);
            ctx.data_mut(|d| d.insert_temp(bar_key(), h));
        }
        None => ctx.data_mut(|d| d.remove::<HoverBar>(bar_key())),
    }
    if let Some(keep) = keep {
        resolve(app, id, keep);
        ctx.data_mut(|d| d.remove::<HoverBar>(bar_key()));
    }
}

/// Runs `calc.resolveEdited` for the region `id`: `source` redraws it from its calculation,
/// `word` keeps the equation as edited (no longer a calculation).
pub fn resolve(app: &mut WordApp, id: u64, keep: &str) {
    let _ = app.run("calc.resolveEdited", json!({"id": id, "keep": keep}));
    app.canvas.focus_page_after_pass();
}

/// Where a data table's errors are marked: one marker per column with an error, on its heading
/// cell (an error about the whole table goes on the first cell), with every message for it, one
/// per line, in the order the table lists them.
pub fn error_markers(errors: &[ColumnError]) -> Vec<(u32, String)> {
    let mut out: Vec<(u32, String)> = Vec::new();
    for e in errors {
        let col = e.col.unwrap_or(0);
        match out.iter_mut().find(|(c, _)| *c == col) {
            Some((_, text)) => {
                text.push('\n');
                text.push_str(&e.message);
            }
            None => out.push((col, e.message.clone())),
        }
    }
    out
}

/// The data tables in `body` that have errors, at any depth, in document order, with their paths
/// (as the engine gives them: block index, then row, cell and block index for each table level)
/// and their [`error_markers`]. Run every frame, so it only looks at the tables' marks: a first
/// look holds only the tables still to look in and ends the walk when none has an error (the
/// usual case); after it, a path is made only for a table with an error or a cell holding a
/// table. Tables are walked with explicit stacks, as a file can nest them deep.
pub(crate) fn erroring_tables(body: &wordcraft_doc::Blocks) -> Vec<(wordcraft_doc::Path, Vec<(u32, String)>)> {
    use wordcraft_doc::{Block, Table};
    fn errors(t: &Table) -> Option<&wordcraft_doc::calc::CalcTable> {
        t.calc.as_deref().filter(|c| !c.errors.is_empty())
    }
    fn tables(blocks: &wordcraft_doc::Blocks) -> impl Iterator<Item = (usize, &Table)> {
        blocks.iter().enumerate().filter_map(|(i, b)| match &**b {
            Block::Table(t) => Some((i, t)),
            Block::Para(_) => None,
        })
    }
    // A first look, holding only the tables still to look in.
    let mut todo: Vec<&Table> = tables(body).map(|(_, t)| t).collect();
    let mut any = false;
    while let Some(t) = todo.pop() {
        if errors(t).is_some() {
            any = true;
            break;
        }
        todo.extend(t.rows.iter().flat_map(|r| &r.cells).flat_map(|c| tables(&c.blocks)).map(|(_, t)| t));
    }
    if !any {
        return Vec::new();
    }
    let index = |k: usize| u32::try_from(k).unwrap_or(u32::MAX);
    let mut out = Vec::new();
    let mut levels: Vec<(Vec<u32>, &wordcraft_doc::Blocks)> = vec![(Vec::new(), body)];
    while let Some((prefix, blocks)) = levels.pop() {
        for (i, t) in tables(blocks) {
            let at = |more: &[u32]| prefix.iter().copied().chain([index(i)]).chain(more.iter().copied()).collect::<Vec<u32>>();
            if let Some(c) = errors(t) {
                out.push((wordcraft_doc::Path(at(&[])), error_markers(&c.errors)));
            }
            for (r, row) in t.rows.iter().enumerate() {
                for (k, cell) in row.cells.iter().enumerate() {
                    if tables(&cell.blocks).next().is_some() {
                        levels.push((at(&[index(r), index(k)]), &cell.blocks));
                    }
                }
            }
        }
    }
    // A path sorts before the paths inside it and after those before it: document order.
    out.sort_by(|a, b| a.0.0.cmp(&b.0.0));
    out
}

/// The marker on a data table's erroring heading cell.
const TABLE_ERROR_MARK: &str = "⚠";

/// A red ⚠ at the top-right of each erroring heading cell of the body's data tables, their
/// messages on hover. Drawn over the page like the edited-outside [`bar`], never written into the
/// table's text; only where the cell is in view, and not over a dialog.
pub(crate) fn table_markers(app: &mut WordApp, ctx: &egui::Context) {
    if app.modal_open() {
        return;
    }
    let view = app.canvas.shown.view;
    let tables = erroring_tables(&app.session.doc.body);
    if tables.is_empty() {
        return;
    }
    static WARNING: std::sync::Mutex<FaceMemo> = std::sync::Mutex::new(None);
    install_font(ctx, "calc-warning", borrowed_face(&WARNING, &['⚠']));
    let layout = app.session.layout();
    let mut marks = Vec::new();
    for (path, errors) in tables {
        for (col, text) in errors {
            let c = col as usize;
            let range = wordcraft_engine::CellRange { story: StoryRef::Body, table: path.clone(), rows: (0, 0), cols: (c, c), stream: false };
            for (page, rect) in wordcraft_engine::cmd::table_mouse::cell_rects(&layout, &app.session.doc, &range) {
                marks.push((path.clone(), col, page, rect, text.clone()));
            }
        }
    }
    for (path, col, page, rect, text) in marks {
        let Some(corner) = crate::canvas::page_to_screen(app, page, rect.right(), rect.y) else { continue };
        if !view.contains(corner - vec2(1.0, -1.0)) {
            continue;
        }
        let id = egui::Id::unique("calc_table_marker").with((path.0, col, page));
        egui::Area::new(id).order(egui::Order::Foreground).pivot(egui::Align2::RIGHT_TOP).fixed_pos(corner).constrain(false).show(ctx, |ui| {
            let red = ui.visuals().error_fg_color;
            let font = egui::TextStyle::Body.resolve(ui.style());
            let label = if ui.fonts_mut(|fonts| fonts.has_glyph(&font, '⚠')) { TABLE_ERROR_MARK } else { "!" };
            let mark = ui.add(egui::Label::new(egui::RichText::new(label).color(red).strong()).sense(egui::Sense::hover()).selectable(false));
            if mark.hovered() {
                egui::Tooltip::always_open(ctx.clone(), ui.layer_id(), id.with("tip"), egui::PopupAnchor::Pointer).show(|ui| {
                    ui.label(&text);
                });
            }
        });
    }
}
