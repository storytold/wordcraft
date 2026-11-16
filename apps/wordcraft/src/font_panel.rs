//! The Font key (⌘D) on macOS: the system font panel (`ui.showFonts` through
//! `Services::show_fonts`). The panel sends `changeFont:` to the font manager's target, an object
//! of ours here, which converts the selection's font the way the panel says and hands the family,
//! size, bold and italic to the app, which formats the selection with them.
//!
//! An exception to the workspace's no-`unsafe` rule (AGENTS.md): receiving `changeFont:` needs an
//! Objective-C class (`define_class!`), and setting the manager's target is marked unsafe.

use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender, channel};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSFont, NSFontManager, NSFontPanel, NSFontTraitMask};
use objc2_foundation::NSString;

/// What the user picked in the panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub family: String,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
}

struct Ivars {
    picks: Sender<Pick>,
    /// The font the panel converts: the selection's, then each pick.
    current: RefCell<Retained<NSFont>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `FontTarget` doesn't implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WordCraftFontTarget"]
    #[ivars = Ivars]
    struct FontTarget;

    impl FontTarget {
        // SAFETY: `changeFont:` takes the sender (the font manager) and returns nothing.
        #[unsafe(method(changeFont:))]
        fn change_font(&self, _sender: Option<&AnyObject>) {
            let manager = NSFontManager::sharedFontManager(self.mtm());
            let font = manager.convertFont(&self.ivars().current.borrow());
            let _ = self.ivars().picks.send(pick(&manager, &font));
            self.ivars().current.replace(font);
        }
    }
);

#[allow(unsafe_code)]
impl FontTarget {
    fn new(mtm: MainThreadMarker, picks: Sender<Pick>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { picks, current: RefCell::new(NSFont::systemFontOfSize(12.0)) });
        // SAFETY: NSObject's designated initializer, on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }
}

fn pick(manager: &NSFontManager, font: &NSFont) -> Pick {
    let traits = manager.traitsOfFont(font);
    Pick {
        family: font.familyName().map(|f| f.to_string()).unwrap_or_default(),
        size: font.pointSize(),
        bold: traits.contains(NSFontTraitMask::BoldFontMask),
        italic: traits.contains(NSFontTraitMask::ItalicFontMask),
    }
}

/// The hook for `Services::show_fonts` and where the picks arrive. Lives as long as the app: the
/// font manager doesn't keep its target alive.
pub struct FontPanel {
    target: Retained<FontTarget>,
    picks: Receiver<Pick>,
}

impl FontPanel {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let (tx, picks) = channel();
        FontPanel { target: FontTarget::new(mtm, tx), picks }
    }

    /// Shows the panel on the selection's font (`family`, `size` points, bold, italic).
    pub fn show(&self, family: &str, size: f64, bold: bool, italic: bool) {
        let mtm = self.target.mtm();
        let manager = NSFontManager::sharedFontManager(mtm);
        let mut traits = NSFontTraitMask::empty();
        traits.set(NSFontTraitMask::BoldFontMask, bold);
        traits.set(NSFontTraitMask::ItalicFontMask, italic);
        // A family the system doesn't have (a document's, a font WordCraft carries) shows the system font.
        let font =
            manager.fontWithFamily_traits_weight_size(&NSString::from_str(family), traits, 5, size).unwrap_or_else(|| NSFont::systemFontOfSize(size));
        self.target.ivars().current.replace(font.clone());
        manager.setSelectedFont_isMultiple(&font, false);
        set_target(&manager, &self.target);
        NSFontPanel::sharedFontPanel(mtm).orderFront(None);
    }

    /// What the panel does when the user picks a font, for `tests/mac_font_panel.rs`.
    #[allow(dead_code)]
    pub fn change_font_for_test(&self) {
        self.target.change_font(objc2::sel!(changeFont:), None);
    }

    /// The picks since the last call, oldest first.
    pub fn picks(&self) -> Vec<Pick> {
        self.picks.try_iter().collect()
    }
}

/// Formats the selection with what the user picked, as the ribbon's Font, Size, Bold and Italic do.
pub fn apply(app: &mut wordcraft_ui_egui::WordApp, pick: &Pick) {
    let family = app.previews.font_stored(&pick.family);
    for (id, params) in [
        ("format.font", serde_json::json!({"name": family})),
        ("format.size", serde_json::json!({"size": pick.size})),
        ("format.bold", serde_json::json!({"value": pick.bold})),
        ("format.italic", serde_json::json!({"value": pick.italic})),
    ] {
        if let Err(e) = app.run(id, params) {
            log::warn!("font panel: {id}: {e}");
        }
    }
}

#[allow(unsafe_code)]
fn set_target(manager: &NSFontManager, target: &FontTarget) {
    // SAFETY: the target responds to the manager's action (`changeFont:`), and `FontPanel` keeps it
    // alive for as long as the app runs, so the manager's unretained reference stays valid.
    unsafe { manager.setTarget(Some(target)) };
}
