//! Shared portal watcher. Inert in headless tests and on platforms handled by winit.
pub use craft_desktop::appearance::Watcher as DesktopTheme;

#[cfg(test)]
mod tests {
    #[test]
    fn a_new_app_does_not_watch_the_desktop() {
        let app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        assert!(!app.desktop_theme.watching());
    }
}
