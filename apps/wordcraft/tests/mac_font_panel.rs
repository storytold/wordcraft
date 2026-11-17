//! The font panel's and colour panel's picks reach the app (`src/font_panel.rs`). AppKit needs the
//! main thread, so this test runs without the test harness.

#[cfg(target_os = "macos")]
#[path = "../src/font_panel.rs"]
#[allow(dead_code, unused_imports)] // its unit tests do not run in this harness-less binary
mod font_panel;

#[cfg(target_os = "macos")]
fn main() {
    use font_panel::{FontPanel, Line, Pick};
    use objc2_app_kit::{NSColor, NSFontManager, NSFontTraitMask};
    // nextest lists the tests first (`--list --format terse`, then with `--ignored`).
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") {
            println!("font_and_color_picks_reach_the_app: test");
        }
        return;
    }
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        panic!("not on the main thread");
    };
    let woken = std::rc::Rc::new(std::cell::Cell::new(0));
    let w = woken.clone();
    let panel = FontPanel::new(mtm, move || w.set(w.get() + 1));
    panel.show_for_test("Helvetica", 14.0, true, false);
    // What the panel sends when the user picks: with nothing changed there, nothing to format.
    panel.change_font_for_test();
    assert_eq!(panel.picks(), vec![]);
    assert_eq!(woken.get(), 0);
    // A real change through AppKit: the font manager's Bigger (⌘+) sends `changeFont:` to our target,
    // which picks the new size and nothing else, and wakes the app.
    let _app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    let bigger = objc2_app_kit::NSButton::new(mtm);
    bigger.setTag(objc2_app_kit::NSFontAction::SizeUpFontAction.0 as isize);
    // SAFETY: `modifyFont:` reads only the sender's tag.
    #[allow(unsafe_code)]
    unsafe {
        NSFontManager::sharedFontManager(mtm).modifyFont(Some(&bigger))
    };
    let picks = panel.picks();
    assert_eq!(picks.len(), 1, "{picks:?}");
    assert!(picks[0].size.is_some_and(|size| size > 14.0) && picks[0].family.is_none() && picks[0].bold.is_none(), "{picks:?}");
    assert_eq!(woken.get(), 1);
    // Colours from any picker arrive as sRGB: an sRGB one, a grey (the grey slider) and a CMYK one
    // (the CMYK sliders, older pickers).
    panel.change_color_for_test(&NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 0.0, 0.0, 1.0));
    panel.change_color_for_test(&NSColor::colorWithWhite_alpha(1.0, 1.0));
    panel.change_color_for_test(&NSColor::colorWithDeviceCyan_magenta_yellow_black_alpha(0.0, 0.0, 0.0, 1.0, 1.0));
    let colors: Vec<_> = panel.picks().into_iter().map(|p| p.color).collect();
    assert_eq!(colors.len(), 3, "{colors:?}");
    assert_eq!(colors[0], Some([255, 0, 0]));
    assert_eq!(colors[1], Some([255, 255, 255]));
    assert!(colors[2].is_some_and(|[r, g, b]| r < 60 && g < 60 && b < 60), "{colors:?}");
    // A colour that arrives twice for one change is applied once.
    let blue = NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 1.0, 1.0);
    panel.change_color_for_test(&blue);
    panel.change_color_for_test(&blue);
    assert_eq!(panel.picks(), vec![Pick { color: Some([0, 0, 255]), ..Pick::default() }]);
    // The panel's effects: double underline and single strikethrough (NSUnderlineStyle 9 and 1).
    let style = |v: isize| -> objc2::rc::Retained<objc2::runtime::AnyObject> { objc2_foundation::NSNumber::new_isize(v).into() };
    let changed = objc2_foundation::NSDictionary::from_retained_objects(
        &[objc2_foundation::ns_string!("NSUnderline"), objc2_foundation::ns_string!("NSStrikethrough")],
        &[style(9), style(1)],
    );
    panel.change_attributes_for_test(&changed);
    assert_eq!(panel.picks(), vec![Pick { underline: Some(Line::Double), strike: Some(Line::Single), ..Pick::default() }]);
    // A family the system doesn't have shows the system font, still bold and italic.
    panel.show_for_test("No Such Family", 14.0, true, true);
    let manager = NSFontManager::sharedFontManager(mtm);
    let traits = manager.traitsOfFont(&manager.selectedFont().expect("a selected font"));
    assert!(traits.contains(NSFontTraitMask::BoldFontMask | NSFontTraitMask::ItalicFontMask), "{traits:?}");
    println!("mac_font_panel: ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
