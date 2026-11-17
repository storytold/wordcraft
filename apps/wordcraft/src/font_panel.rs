//! The Font key (⌘D) on macOS: the system font panel (`ui.showFonts` through
//! `Services::show_fonts`). The panel sends `changeFont:` to the font manager's target, an object
//! of ours here, which converts the selection's font the way the panel says and hands what changed
//! (family, size, bold, italic) to the app, which formats the selection with it.
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

/// What the user changed in the panel. Only that is applied, so a new size keeps each run's own
/// family, bold and italic in a mixed selection.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pick {
    pub family: Option<String>,
    pub size: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
}

/// A font as the app formats text with it.
#[derive(Debug, Clone, PartialEq)]
struct Face {
    family: String,
    size: f64,
    bold: bool,
    italic: bool,
}

impl Pick {
    fn between(before: &Face, after: &Face) -> Self {
        Pick {
            family: (after.family != before.family).then(|| after.family.clone()),
            size: (after.size != before.size).then_some(after.size),
            bold: (after.bold != before.bold).then_some(after.bold),
            italic: (after.italic != before.italic).then_some(after.italic),
        }
    }
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
            let before = self.ivars().current.borrow().clone();
            let font = manager.convertFont(&before);
            let pick = Pick::between(&face(&manager, &before), &face(&manager, &font));
            if pick != Pick::default() {
                let _ = self.ivars().picks.send(pick);
            }
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

fn face(manager: &NSFontManager, font: &NSFont) -> Face {
    let traits = manager.traitsOfFont(font);
    Face {
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
    /// The font the panel shows, so it changes only when the selection's does.
    shown: RefCell<Option<Face>>,
}

impl FontPanel {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let (tx, picks) = channel();
        FontPanel { target: FontTarget::new(mtm, tx), picks, shown: RefCell::new(None) }
    }

    /// Shows the panel on the selection's font (`family`, `size` points, bold, italic).
    pub fn show(&self, family: &str, size: f64, bold: bool, italic: bool) {
        let mtm = self.target.mtm();
        self.select(Face { family: family.into(), size, bold, italic });
        set_target(&NSFontManager::sharedFontManager(mtm), &self.target);
        NSFontPanel::sharedFontPanel(mtm).orderFront(None);
    }

    /// While the panel is open, it shows the font of the selection as that changes (another
    /// selection, or formatting from the ribbon), so a pick converts the selection's own font.
    pub fn follow(&self, app: &mut wordcraft_ui_egui::WordApp) {
        let mtm = self.target.mtm();
        if !NSFontPanel::sharedFontPanelExists(mtm) || !NSFontPanel::sharedFontPanel(mtm).isVisible() {
            return;
        }
        let st = app.session.run("format.state", &serde_json::json!({})).unwrap_or_default();
        let face = Face {
            family: st["font"].as_str().unwrap_or("").into(),
            size: st["size"].as_f64().unwrap_or(12.0),
            bold: st["bold"] == true,
            italic: st["italic"] == true,
        };
        if self.shown.borrow().as_ref() != Some(&face) {
            self.select(face);
        }
    }

    fn select(&self, face: Face) {
        let manager = NSFontManager::sharedFontManager(self.target.mtm());
        let mut traits = NSFontTraitMask::empty();
        traits.set(NSFontTraitMask::BoldFontMask, face.bold);
        traits.set(NSFontTraitMask::ItalicFontMask, face.italic);
        // A family the system doesn't have (a document's, a font WordCraft carries) shows the system
        // font, bold and italic as the selection is.
        let font = manager.fontWithFamily_traits_weight_size(&NSString::from_str(&face.family), traits, 5, face.size).unwrap_or_else(|| {
            // `convertFont:toHaveTrait:` adds one trait per call.
            [NSFontTraitMask::BoldFontMask, NSFontTraitMask::ItalicFontMask]
                .into_iter()
                .filter(|t| traits.contains(*t))
                .fold(NSFont::systemFontOfSize(face.size), |f, t| manager.convertFont_toHaveTrait(&f, t))
        });
        self.target.ivars().current.replace(font.clone());
        manager.setSelectedFont_isMultiple(&font, false);
        self.shown.replace(Some(face));
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

/// Formats the selection with what the user changed, as the ribbon's Font, Size, Bold and Italic do.
pub fn apply(app: &mut wordcraft_ui_egui::WordApp, pick: &Pick) {
    let family = pick.family.as_ref().map(|f| app.previews.font_stored(f));
    let runs = [
        family.map(|name| ("format.font", serde_json::json!({"name": name}))),
        pick.size.map(|size| ("format.size", serde_json::json!({"size": size}))),
        pick.bold.map(|value| ("format.bold", serde_json::json!({"value": value}))),
        pick.italic.map(|value| ("format.italic", serde_json::json!({"value": value}))),
    ];
    for (id, params) in runs.into_iter().flatten() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_changed_is_picked() {
        let before = Face { family: "Helvetica".into(), size: 12.0, bold: true, italic: false };
        let bigger = Face { size: 18.0, ..before.clone() };
        assert_eq!(Pick::between(&before, &bigger), Pick { size: Some(18.0), ..Pick::default() }, "bold stays each run's own");
        let plain = Face { bold: false, ..before.clone() };
        assert_eq!(Pick::between(&before, &plain), Pick { bold: Some(false), ..Pick::default() });
        assert_eq!(Pick::between(&before, &before), Pick::default());
    }
}
