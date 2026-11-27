use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{Props, Show, ShowContainer};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::sections::GroupBox;
use egui::Ui;
use serde_json::{Value, json};

use super::WordApp;

/// Style Inspector: the paragraph and character levels at the caret, each with its style and the
/// direct formatting on top, and a button to clear each level.
pub(super) fn inspector(app: &mut WordApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    // Read-only, so call it directly: running the command every frame would pay for the command
    // machinery and consume per-command state (a pending `join_next_undo`) just to draw the pane.
    let r = wordcraft_engine::cmd::inspector::inspect(&app.session);
    let none = tl!("None").to_string();
    let para = &r["paragraph"];
    let chr = &r["character"];
    let pstyle = para["styleName"].as_str().unwrap_or("Normal").to_string();
    let cstyle = chr["styleName"].as_str().map(str::to_string);
    // Built-in style names show in the interface language; the document keeps the English ones.
    let shown = |id: &Value, name: &str| {
        id.as_str().and_then(|id| app.session.doc.styles.get(id)).map(crate::i18n::style_name).unwrap_or_else(|| name.to_string())
    };
    let pstyle_shown = shown(&para["style"], &pstyle);
    let cstyle_shown = cstyle.as_deref().map(|n| shown(&chr["style"], n));
    let pdirect = direct_list(&para["direct"]);
    let cdirect = direct_list(&chr["direct"]);
    let mut clear: Option<&str> = None;
    Stack::new().fill_width().show(ui, |ui, _| {
        GroupBox::new(tl!("Paragraph").to_owned(), None).show(ui, |ui, _| {
            if level_row(ui, "Style", &pstyle_shown, false, "Reset to Normal", pstyle == "Normal" && para["style"] == "Normal") {
                clear = Some("paragraphStyle");
            }
            let (txt, empty) = if pdirect.is_empty() { (none.clone(), true) } else { (pdirect.join(", "), false) };
            if level_row(ui, "Direct formatting", &txt, empty, "Clear Paragraph Formatting", empty) {
                clear = Some("paragraphFormatting");
            }
        });
        GroupBox::new(tl!("Characters").to_owned(), None).show(ui, |ui, _| {
            let (txt, empty) = match &cstyle_shown {
                Some(n) => (n.clone(), false),
                None => (none.clone(), true),
            };
            if level_row(ui, "Style", &txt, empty, "Clear Character Style", empty) {
                clear = Some("characterStyle");
            }
            let (txt, empty) = if cdirect.is_empty() { (none.clone(), true) } else { (cdirect.join(", "), false) };
            if level_row(ui, "Direct formatting", &txt, empty, "Clear Character Formatting", empty) {
                clear = Some("characterFormatting");
            }
        });
        Row::new().wrap(true).show(ui, |ui, _| {
            // PLAN: assumed `Props::tooltip(impl Into<Label>)` on the common props.
            if Button::new(tl!("Clear All").to_owned()).tooltip(tl!("Clear All Formatting").to_owned()).show(ui).clicked() {
                let _ = app.run("format.clear", json!({}));
                app.canvas.focus_page(&ctx);
            }
            if Button::new(tl!("Styles Pane").to_owned()).show(ui).clicked() {
                let _ = app.run("view.stylesPane", json!({"value": true}));
            }
        });
    });
    if let Some(level) = clear {
        let _ = app.run("styles.inspectorClear", json!({"level": level}));
        app.canvas.focus_page(&ctx);
    }
}

/// One level: a muted caption, its value (faint when `none`, which stands for no value), and a clear button
/// (disabled when there's nothing to clear). Returns true when the button was clicked.
fn level_row(ui: &mut Ui, caption: &str, value: &str, none: bool, tip: &str, nothing_to_clear: bool) -> bool {
    let mut clicked = false;
    Stack::new().fill_width().show(ui, |ui, _| {
        Text::new(tl!(caption).to_owned()).shade(Shade::Muted).show(ui);
        Row::new().show(ui, |ui, _| {
            Text::new(value.to_owned()).shade(if none { Shade::Faint } else { Shade::Normal }).show(ui);
            let clear = IconButton::new(crate::icons::spec("clear")).label(tl!(tip).to_owned()).enabled(!nothing_to_clear).show(ui);
            clicked = clear.clicked();
        });
    });
    clicked
}

/// Readable descriptions of the inspector's `direct` entries (`[{"prop", "value"}]`).
fn direct_list(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().take(64).map(|d| describe(d["prop"].as_str().unwrap_or(""), &d["value"])).collect()).unwrap_or_default()
}

/// Each `{"prop", "value"}` of a list (the Style Inspector's, or a formatting change's) as text.
pub(crate) fn describe_props(list: &[Value]) -> Vec<String> {
    list.iter().take(64).map(|d| describe(d["prop"].as_str().unwrap_or(""), &d["value"])).collect()
}

/// "Bold", "Not Italic", "Font: Arial", "Size: 14 pt"…
fn describe(prop: &str, v: &Value) -> String {
    let label = match prop {
        "font" => "Font",
        "size" => "Size",
        "bold" => "Bold",
        "italic" => "Italic",
        "underline" => "Underline",
        "underlineColor" => "Underline color",
        "strike" => "Strikethrough",
        "doubleStrike" => "Double strikethrough",
        "color" => "Font color",
        "highlight" => "Highlight",
        "shading" => "Shading",
        "vertAlign" => "Position",
        "caps" => "All caps",
        "smallCaps" => "Small caps",
        "hidden" => "Hidden",
        "spacing" => "Character spacing",
        "scale" => "Scale",
        "position" => "Raised/lowered",
        "outline" => "Outline",
        "shadow" => "Shadow",
        "emboss" => "Emboss",
        "engrave" => "Engrave",
        "align" => "Alignment",
        "indentLeft" => "Left indent",
        "indentRight" => "Right indent",
        "indentFirst" => "First line",
        "spaceBefore" => "Space before",
        "spaceAfter" => "Space after",
        "lineSpacing" => "Line spacing",
        "contextualSpacing" => "Don't add space between paragraphs of the same style",
        "mirrorIndents" => "Mirror indents",
        "keepNext" => "Keep with next",
        "keepLines" => "Keep lines together",
        "pageBreakBefore" => "Page break before",
        "widowControl" => "Widow/Orphan control",
        "outlineLevel" => "Outline level",
        "tabs" => "Tabs",
        "borders" => "Borders",
        "suppressHyphens" => "Don't hyphenate",
        "suppressLineNumbers" => "Suppress line numbers",
        "bidi" => "Right-to-left",
        "dropCap" => "Drop cap",
        "dropCapMargin" => "In margin",
        "dropCapDistance" => "Distance from text",
        "dropCapFont" => "Font",
        "style" => "Style",
        "numbering" => "Numbering",
        "lang" => "Language",
        other => other,
    };
    let label = tl!(label);
    let pt = |x: f64| crate::i18n::fmt(tl!("{n} pt"), &[("n", &trim_num(x))]);
    let hex = |c: &Value| -> Option<String> {
        let a = c.as_array()?;
        let ch = |i: usize| a.get(i).and_then(Value::as_u64).unwrap_or(0).min(255);
        Some(format!("#{:02X}{:02X}{:02X}", ch(0), ch(1), ch(2)))
    };
    let value = match (prop, v) {
        (_, Value::Bool(true)) => return label.to_string(),
        (_, Value::Bool(false)) => return crate::i18n::fmt(tl!("Not {name}"), &[("name", label)]),
        ("scale", Value::Number(n)) => format!("{}%", trim_num(n.as_f64().unwrap_or(100.0))),
        ("outlineLevel", Value::Number(n)) => match n.as_u64().unwrap_or(9) {
            l @ 0..=8 => crate::i18n::fmt(tl!("Level {n}"), &[("n", &(l + 1).to_string())]),
            _ => tl!("Body Text").to_string(),
        },
        ("dropCapFont", Value::String(s)) => s.clone(),
        ("dropCap", Value::Number(n)) => crate::i18n::fmt(tl!("{n} lines"), &[("n", &n.to_string())]),
        (_, Value::Number(n)) => pt(n.as_f64().unwrap_or(0.0)),
        ("align", Value::String(s)) => tl!(match s.as_str() {
            "center" => "Centered",
            "right" => "Right",
            "justify" => "Justified",
            "distribute" => "Distributed",
            _ => "Left",
        })
        .to_string(),
        ("color", Value::String(_)) => tl!("Automatic").to_string(),
        ("color", Value::Object(o)) => o.get("Rgb").and_then(hex).unwrap_or_default(),
        (_, Value::Array(_)) if prop != "tabs" => hex(v).unwrap_or_default(),
        ("lineSpacing", Value::Object(o)) => {
            let x = o.get("value").and_then(Value::as_f64).unwrap_or(1.0);
            match o.get("rule").and_then(Value::as_str) {
                Some("atLeast") => crate::i18n::fmt(tl!("At least {n}"), &[("n", &pt(x))]),
                Some("exactly") => crate::i18n::fmt(tl!("Exactly {n}"), &[("n", &pt(x))]),
                _ => crate::i18n::fmt(tl!("{n} lines"), &[("n", &trim_num(x))]),
            }
        }
        (_, Value::String(s)) => tl!(&camel_words(s)).to_string(),
        _ => return label.to_string(),
    };
    format!("{label}: {value}")
}

/// `1.50` → `1.5`, `12.0` → `12`.
fn trim_num(x: f64) -> String {
    let s = format!("{:.2}", if x.is_finite() { x } else { 0.0 });
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `dotDash` → `Dot dash` (enum values from the document model).
fn camel_words(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().take(64).enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
