//! The text of a command's hover hint.

/// The hint a command shows under its title: the group it lives in, its description and, while it is off, why. Empty
/// parts are left out; an empty `reason` falls back to a generic line. `None` when nothing is left.
pub(crate) fn command_hint(group: &str, description: &str, reason: Option<&str>) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    // Both are the engine's English: the group a ribbon location (`Home › Font`), translated a segment at a time.
    if !group.is_empty() {
        lines.push(crate::i18n::location(group));
    }
    if !description.is_empty() {
        lines.push(tl!(description).to_string());
    }
    if let Some(reason) = reason {
        lines.push(if reason.is_empty() { tl!("Not available right now").to_string() } else { capitalised(tl!(reason)) });
    }
    if lines.is_empty() { None } else { Some(lines.join("\n")) }
}

/// `text` with its first letter in capitals (the engine's reasons are lower case).
fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
