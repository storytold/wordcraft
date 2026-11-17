//! The font panel's `changeFont:` reaches the app (`src/font_panel.rs`). AppKit needs the main
//! thread, so this test runs without the test harness.

#[cfg(target_os = "macos")]
#[path = "../src/font_panel.rs"]
#[allow(dead_code, unused_imports)] // its unit tests do not run in this harness-less binary
mod font_panel;

#[cfg(target_os = "macos")]
fn main() {
    use font_panel::FontPanel;
    // nextest lists the tests first (`--list --format terse`, then with `--ignored`).
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") {
            println!("change_font_reaches_the_app: test");
        }
        return;
    }
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        panic!("not on the main thread");
    };
    let panel = FontPanel::new(mtm);
    panel.show("Helvetica", 14.0, true, false);
    // What the panel sends when the user picks: with nothing changed there, nothing to format.
    panel.change_font_for_test();
    assert_eq!(panel.picks(), vec![]);
    // A real change through AppKit: the font manager's Bigger (⌘+) sends `changeFont:` to our target,
    // which picks the new size and nothing else.
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    let bigger = objc2_app_kit::NSButton::new(mtm);
    bigger.setTag(objc2_app_kit::NSFontAction::SizeUpFontAction.0 as isize);
    // SAFETY: `modifyFont:` reads only the sender's tag.
    #[allow(unsafe_code)]
    let press_bigger = || unsafe { objc2_app_kit::NSFontManager::sharedFontManager(mtm).modifyFont(Some(&bigger)) };
    press_bigger();
    let picks = panel.picks();
    assert_eq!(picks.len(), 1, "{picks:?}");
    let size_only = |p: &font_panel::Pick| p.family.is_none() && p.bold.is_none() && p.italic.is_none();
    assert!(picks[0].size.is_some_and(|size| size > 14.0 && size < 20.0) && size_only(&picks[0]), "{picks:?}");
    // With the panel open, another selection (30 pt here) becomes what the panel converts.
    let mut word =
        wordcraft_ui_egui::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), wordcraft_ui_egui::Services::default());
    word.run("text.insert", serde_json::json!({"text": "Big"})).unwrap();
    word.run("select.all", serde_json::json!({})).unwrap();
    word.run("format.size", serde_json::json!({"size": 30})).unwrap();
    panel.follow(&mut word);
    press_bigger();
    let picks = panel.picks();
    assert!(picks.len() == 1 && picks[0].size.is_some_and(|size| size > 30.0) && size_only(&picks[0]), "{picks:?}");
    // A family the system doesn't have shows the system font, bold and italic as the selection is.
    panel.show("No Such Family", 12.0, true, true);
    let manager = objc2_app_kit::NSFontManager::sharedFontManager(mtm);
    let shown = manager.selectedFont().expect("a font is shown");
    let both = objc2_app_kit::NSFontTraitMask::BoldFontMask | objc2_app_kit::NSFontTraitMask::ItalicFontMask;
    assert!(manager.traitsOfFont(&shown).contains(both), "{shown:?}");
    drop(app);
    println!("mac_font_panel: ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
