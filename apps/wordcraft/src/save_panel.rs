//! Save As on macOS, with Word for Mac's File Format menu below the name.
//!
//! rfd gives the Mac panel the save formats as allowed file types only, which shows no menu: saving
//! as PDF or Rich Text meant knowing to type the extension. These calls are safe in the
//! `objc2-app-kit` bindings (our workspace forbids unsafe). As in Word, picking a format renames the
//! file ("Brief.docx" becomes "Brief.pdf"), so the panel's own replace check asks about the file
//! that will be written. A menu action would need unsafe code (a target-action or a notification
//! observer); [`follow_menu`] looks at the menu five times a second while the panel is up instead.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchTime, MainThreadBound};
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSModalResponse, NSModalResponseOK, NSPopUpButton, NSSavePanel, NSStackView, NSTextField, NSView};
use objc2_foundation::{NSArray, NSEdgeInsets, NSPoint, NSRect, NSSize, NSString, NSURL};
use wordcraft_ui_egui::FileDialogAnswer;

/// Show the Save panel as a sheet on the app's window, the suggested `name` in `dir`, with a File
/// Format menu of `formats` (label and extensions, the suggested one first). The path picked, with
/// the chosen format's extension, goes back through `answer`; nothing, when cancelled.
pub fn ask(name: &str, dir: Option<&str>, formats: Vec<(String, Vec<String>)>, answer: Sender<FileDialogAnswer>, ctx: egui::Context) {
    let Some(mtm) = MainThreadMarker::new() else {
        let _ = answer.send(Err("the Save panel must be shown on the main thread".into()));
        return;
    };
    let panel = NSSavePanel::savePanel(mtm);
    panel.setNameFieldStringValue(&NSString::from_str(name));
    if let Some(dir) = dir.filter(|d| Path::new(d).is_dir()) {
        panel.setDirectoryURL(Some(&NSURL::fileURLWithPath_isDirectory(&NSString::from_str(dir), true)));
    }
    panel.setExtensionHidden(false);

    let menu =
        NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(260.0, 26.0)), false);
    for (label, _) in &formats {
        menu.addItemWithTitle(&NSString::from_str(label));
    }
    menu.selectItemAtIndex(0);
    let label = NSTextField::labelWithString(&NSString::from_str(wordcraft_ui_egui::tl!("File Format:")), mtm);
    let views: [&NSView; 2] = [&label, &menu];
    let row = NSStackView::stackViewWithViews(&NSArray::from_slice(&views), mtm);
    row.setSpacing(8.0);
    row.setEdgeInsets(NSEdgeInsets { top: 10.0, left: 20.0, bottom: 10.0, right: 20.0 });
    panel.setAccessoryView(Some(&row));

    let open = Arc::new(AtomicBool::new(true));
    let exts: Vec<Option<String>> = formats.iter().map(|(_, e)| e.first().cloned()).collect();
    follow_menu(MainThreadBound::new(panel.clone(), mtm), MainThreadBound::new(menu.clone(), mtm), exts, 0, open.clone());

    let shown = panel.clone();
    let handler = RcBlock::new(move |response: NSModalResponse| {
        open.store(false, Ordering::Relaxed);
        let picked = (response == NSModalResponseOK).then(|| shown.URL().and_then(|u| u.path())).flatten();
        let Some(path) = picked else {
            let _ = answer.send(Ok(Vec::new()));
            ctx.request_repaint();
            return;
        };
        let chosen = usize::try_from(menu.indexOfSelectedItem()).ok().and_then(|i| formats.get(i)).and_then(|(_, e)| e.first());
        let changed = menu.indexOfSelectedItem() != 0;
        let path = path.to_string();
        let file = chosen.map_or_else(|| path.clone(), |ext| chosen_path(&path, ext, changed));
        // The panel asked about replacing the file it named; a format changed in the menu names
        // another, which may be there too: ask again at that name, so the panel asks first.
        if file != path && Path::new(&file).exists() {
            let (name, dir) = split(&file);
            let mut formats = formats.clone();
            let i = formats.iter().position(|(_, e)| e.first() == chosen).unwrap_or(0);
            let first = formats.remove(i);
            formats.insert(0, first);
            let (answer, ctx) = (answer.clone(), ctx.clone());
            dispatch2::DispatchQueue::main().exec_async(move || ask(&name, dir.as_deref(), formats, answer, ctx));
            return;
        }
        let _ = answer.send(Ok(vec![file]));
        ctx.request_repaint();
    });
    match NSApplication::sharedApplication(mtm).keyWindow().or_else(|| NSApplication::sharedApplication(mtm).mainWindow()) {
        Some(window) => panel.beginSheetModalForWindow_completionHandler(&window, &handler),
        None => panel.beginWithCompletionHandler(&handler),
    }
}

/// While the panel is `open`: when the File Format menu moves off item `shown`, give the name in the
/// panel that format's extension (as [`chosen_path`] does), then look again in a fifth of a second.
fn follow_menu(
    panel: MainThreadBound<Retained<NSSavePanel>>,
    menu: MainThreadBound<Retained<NSPopUpButton>>,
    exts: Vec<Option<String>>,
    shown: isize,
    open: Arc<AtomicBool>,
) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    if !open.load(Ordering::Relaxed) {
        return;
    }
    let picked = menu.get(mtm).indexOfSelectedItem();
    if picked != shown
        && let Some(ext) = usize::try_from(picked).ok().and_then(|i| exts.get(i)).and_then(Option::as_deref)
    {
        let p = panel.get(mtm);
        let name = p.nameFieldStringValue().to_string();
        p.setNameFieldStringValue(&NSString::from_str(&chosen_path(&name, ext, true)));
    }
    let _ = DispatchQueue::main().after(DispatchTime::NOW.time(200_000_000), move || follow_menu(panel, menu, exts, picked, open));
}

/// A path's file name and folder.
fn split(path: &str) -> (String, Option<String>) {
    let p = Path::new(path);
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    (name, p.parent().map(|d| d.to_string_lossy().into_owned()))
}

/// The file to save: `path` from the panel, with the extension of format `ext`. A format changed
/// in the menu decides: it replaces a save format's extension in the name ("Brief.docx" as PDF is
/// "Brief.pdf") or is added after any other ("Smith v. Jones" is "Smith v. Jones.pdf"). With the
/// menu left alone, a name typed with a save format's extension keeps it, and one without gets
/// the menu's.
pub fn chosen_path(path: &str, ext: &str, changed: bool) -> String {
    let p = Path::new(path);
    let typed = p
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .filter(|e| wordcraft_engine::io::SAVE_EXTS.contains(&e.as_str()));
    match typed {
        Some(_) if !changed => path.to_string(),
        Some(_) => p.with_extension(ext).to_string_lossy().into_owned(),
        None => format!("{path}.{ext}"),
    }
}

#[cfg(test)]
mod tests {
    use super::chosen_path;

    /// The File Format menu decides the extension when it was changed; left alone, a typed
    /// extension stands.
    #[test]
    fn the_file_format_menu_picks_the_extension() {
        assert_eq!(chosen_path("/d/Brief.docx", "pdf", true), "/d/Brief.pdf");
        assert_eq!(chosen_path("/d/Brief.docx", "rtf", true), "/d/Brief.rtf");
        assert_eq!(chosen_path("/d/Brief.pdf", "docx", false), "/d/Brief.pdf", "a typed extension stands");
        assert_eq!(chosen_path("/d/Brief", "docx", false), "/d/Brief.docx");
        assert_eq!(chosen_path("/d/Smith v. Jones", "pdf", true), "/d/Smith v. Jones.pdf", "a dot in the name is no extension");
        assert_eq!(chosen_path("/d/Order.PDF", "docx", false), "/d/Order.PDF");
    }
}
