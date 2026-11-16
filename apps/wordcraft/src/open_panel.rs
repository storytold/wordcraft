//! File › Open on macOS: the system Open panel as a window of its own, as TextEdit and Pages show
//! it, not a sheet on whichever document happens to be in front. Its answer opens in a new window.

use objc2::MainThreadMarker;
use objc2::rc::Weak;
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{NSArray, NSString};
use std::sync::mpsc::Sender;
use wordcraft_ui_egui::FileDialogAnswer;

/// Shows the panel and returns at once; `answer` gets the picked paths (none when cancelled).
pub fn show(mtm: MainThreadMarker, purpose: &str, answer: Sender<FileDialogAnswer>, ctx: eframe::egui::Context) {
    let panel = NSOpenPanel::openPanel(mtm);
    // The same filtering rfd gives the sheet: every filter's extensions in one list.
    let exts: Vec<_> = wordcraft_ui_egui::open_filters(purpose).iter().flat_map(|(_, exts)| exts.iter()).map(|e| NSString::from_str(e)).collect();
    #[allow(deprecated)] // UTType needs another framework; rfd uses this too.
    panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&exts)));
    // Weak: the panel keeps the handler alive until it closes, so a strong reference would leak it.
    let weak = Weak::from_retained(&panel);
    let handler = block2::RcBlock::new(move |response| {
        let picked = match weak.load() {
            Some(panel) if response == NSModalResponseOK => panel.URLs().iter().filter_map(|url| url.path()).map(|p| p.to_string()).collect(),
            _ => Vec::new(),
        };
        let _ = answer.send(Ok(picked));
        ctx.request_repaint();
    });
    panel.beginWithCompletionHandler(&handler);
}
