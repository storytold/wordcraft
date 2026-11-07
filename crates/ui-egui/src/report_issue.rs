//! Help ▸ Report an Issue…: opens a new GitHub issue with the version and system filled in. Nothing
//! is created until the user submits it.

use serde_json::{Value, json};

use crate::WordApp;

/// Extensions WordCraft opens; any other one could be private text and is left out of the report.
const KNOWN: [&str; 11] = ["docx", "doc", "dotx", "dotm", "docm", "rtf", "odt", "txt", "md", "html", "htm"];

/// The new-issue address. The document is described by its file extension alone, never a name,
/// path or contents. Plain strings in, so it can move to a shared crate.
pub(crate) fn issue_url(version: &str, os: &str, document_ext: Option<&str>) -> String {
    let short = |s: &str| s.chars().filter(|c| !c.is_control()).take(16).collect::<String>();
    let ext: String = document_ext.unwrap_or("none").chars().filter(char::is_ascii_alphanumeric).take(8).collect();
    let (version, environment) = (short(version), format!("{}; document: {ext}", short(os)));
    let encode = |s: &str| {
        let keep = |b: &u8| b.is_ascii_alphanumeric() || b"-_.~".contains(b);
        s.bytes().map(|b| if keep(&b) { char::from(b).to_string() } else { format!("%{b:02X}") }).collect::<String>()
    };
    let body = format!("WordCraft {version}\n{environment}");
    let (v, e, b) = (encode(&version), encode(&environment), encode(&body));
    format!("https://github.com/storytold/wordcraft/issues/new?version={v}&environment={e}&os={e}&body={b}")
}

/// `help.reportIssue`: opens the form in the browser. Returns `{url}` and never opens a dialog.
pub(crate) fn run(app: &mut WordApp) -> Value {
    let ext = app.session.path.as_deref().and_then(|p| p.extension()?.to_str()).map(str::to_ascii_lowercase).filter(|e| KNOWN.contains(&e.as_str()));
    let os = if cfg!(target_arch = "wasm32") { "Web".into() } else { format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH) };
    let url = issue_url(env!("CARGO_PKG_VERSION"), &os, ext.as_deref());
    app.canvas.open_url = Some(url.clone());
    json!({"url": url})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_carries_version_and_system_only_and_stays_short() {
        let url = issue_url("0.1.0", "linux (x86_64)", Some("docx"));
        assert!(url.contains("?version=0.1.0&environment=linux%20%28x86_64%29%3B%20document%3A%20docx&os="), "{url}");
        // Hostile text and an unusual extension can't add a parameter, leak, or grow the address.
        let url = issue_url(&"\u{1F600}".repeat(99), &"&#=".repeat(99), Some("x.y/Z"));
        assert_eq!(url.matches('?').count() + url.matches('#').count(), 1, "{url}");
        assert!(url.is_ascii() && url.contains("document%3A%20xyZ") && url.len() < 2048, "{} bytes: {url}", url.len());
        assert!(issue_url("1", "w", None).contains("document%3A%20none"));
    }
}
