//! Which craft-fonts faces a web (wasm32) build embeds. `build.rs` includes this file to filter
//! the manifest; the crate's tests check the list.

/// Scripts whose craft-fonts faces a web build embeds. A browser can't read the system's fonts
/// without asking, so a script with no embedded face draws as boxes. Only small faces fit the web
/// size budget: Georgian for the interface (about 230 KB, #3888) and Thai for documents (about
/// 220 KB), whose text otherwise has no face at all. The Japanese (4.7 MB) and Chinese (16 MB)
/// faces stay out.
pub(crate) const WEB_SCRIPTS: &[&str] = &["Geor", "Thai"];

/// Does a web build embed a face for `scripts`, a manifest line's comma-separated ISO 15924 list?
pub(crate) fn embed_on_web(scripts: &str) -> bool {
    scripts.split(',').any(|s| WEB_SCRIPTS.contains(&s.trim()))
}

#[cfg(test)]
mod tests {
    use super::embed_on_web;

    /// The Georgian and Thai manifest lines pass the web filter, whatever their spacing or other
    /// scripts; the large CJK faces and the other scripts don't.
    #[test]
    fn web_builds_embed_the_georgian_and_thai_faces_only() {
        assert!(embed_on_web("Geor"));
        assert!(embed_on_web("Thai"));
        assert!(embed_on_web(" Thai "), "manifest fields may carry spaces");
        assert!(embed_on_web("Latn, Thai"), "any listed script counts");
        for big in ["Jpan,Latn", "Hans,Latn", "Hang,Kore", "Arab", "Zsym", ""] {
            assert!(!embed_on_web(big), "{big} stays out of the web build");
        }
    }
}
