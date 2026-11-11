/// Whether a craft-fonts manifest `scripts` field (comma-separated ISO 15924 codes) is embedded
/// in the web build. CJK collections are left out (the Japanese face is several megabytes).
/// Arabic, Georgian and Hebrew interface faces are small enough to keep, so a Hebrew interface
/// and "עברית" in the language list have glyphs without an installed font (the browser has none).
pub fn web_embeds_scripts(scripts: &str) -> bool {
    scripts.split(',').any(|s| matches!(s.trim(), "Arab" | "Geor" | "Hebr"))
}

/// The script tags a build embeds. `web` applies [`web_embeds_scripts`]. An empty list (no
/// `CRAFT_FONTS_DIR`) stays empty on every target.
#[cfg(test)]
fn embedded_scripts<'a>(scripts: impl IntoIterator<Item = &'a str>, web: bool) -> Vec<&'a str> {
    scripts.into_iter().filter(|s| !web || web_embeds_scripts(s)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_web_build_keeps_the_hebrew_face_and_a_fontless_build_keeps_none() {
        let manifest = ["Jpan", "Hans", "Kore", "Arab", "Geor", "Hebr", "Latn, Hebr"];
        let web = embedded_scripts(manifest, true);
        assert!(web.contains(&"Hebr"), "the pinned Hebrew face ships on the web");
        assert!(web.contains(&"Arab") && web.contains(&"Geor"), "Arabic and Georgian stay");
        assert!(web.contains(&"Latn, Hebr"), "a face that also covers Hebrew is kept");
        assert!(!web.iter().any(|s| matches!(*s, "Jpan" | "Hans" | "Kore")), "CJK stays out: {web:?}");
        assert_eq!(embedded_scripts(manifest, false), manifest, "native embeds every face");
        assert!(embedded_scripts(std::iter::empty(), true).is_empty());
        assert!(embedded_scripts(std::iter::empty(), false).is_empty());
    }
}
