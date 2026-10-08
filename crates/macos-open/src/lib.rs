//! Files opened from Finder ("Open With", double-click) or `open -a WordCraft file.docx` reach a
//! macOS app as the `application:openURLs:` Apple Event on the NSApplicationDelegate, not argv.
//!
//! winit 0.30.13 registers its own `WinitApplicationDelegate` in `EventLoop::new` and panics
//! if the delegate is replaced (its doc comment saying otherwise is stale). So instead of
//! setting a new delegate, this adds `application:openURLs:` to the class of winit's delegate.
//! It does so on `NSApplicationWillFinishLaunchingNotification`: the delegate exists by then,
//! and AppKit has not yet delivered the documents the app was launched with.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::PathBuf;

/// Receive files opened from Finder / `open` (`application:openURLs:`).
/// `on_open` runs on the main thread with the file paths. Call once, on the main thread,
/// before the event loop runs (before `eframe::run_native`), so files that launch the app
/// are not missed. No-op on non-macOS targets.
pub fn install(on_open: impl Fn(Vec<PathBuf>) + 'static) {
    #[cfg(target_os = "macos")]
    imp::install(Box::new(on_open));
    #[cfg(not(target_os = "macos"))]
    let _ = on_open;
}

#[cfg(target_os = "macos")]
mod imp {
    use std::cell::OnceCell;
    use std::path::PathBuf;
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{MainThreadMarker, ffi, sel};
    use objc2_app_kit::{NSApplication, NSApplicationWillFinishLaunchingNotification};
    use objc2_foundation::{NSArray, NSNotification, NSNotificationCenter, NSURL};

    type Callback = Box<dyn Fn(Vec<PathBuf>)>;

    thread_local! {
        static ON_OPEN: OnceCell<Callback> = const { OnceCell::new() };
    }

    /// `- (void)application:(NSApplication *)app openURLs:(NSArray<NSURL *> *)urls`
    extern "C-unwind" fn open_urls(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject, urls: *const NSArray<NSURL>) {
        // SAFETY: AppKit passes a valid NSArray<NSURL> (or nil) for the duration of the call.
        let Some(urls) = (unsafe { urls.as_ref() }) else { return };
        let paths: Vec<PathBuf> = urls.iter().filter_map(|u| u.to_file_path()).collect();
        if !paths.is_empty() {
            ON_OPEN.with(|cb| {
                if let Some(cb) = cb.get() {
                    cb(paths);
                }
            });
        }
    }

    /// Add `application:openURLs:` to the class of the application delegate winit registered.
    fn add_open_urls_method(mtm: MainThreadMarker) {
        let Some(delegate) = NSApplication::sharedApplication(mtm).delegate() else {
            eprintln!("wordcraft-macos-open: NSApplication has no delegate");
            return;
        };
        let delegate: &AnyObject = (*delegate).as_ref();
        let class: &AnyClass = delegate.class();
        // SAFETY: `open_urls` matches the `v@:@@` signature of `application:openURLs:`.
        // `class_addMethod` leaves an existing implementation alone and returns NO.
        let added = unsafe {
            let imp: Imp = std::mem::transmute::<extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *const NSArray<NSURL>), Imp>(open_urls);
            ffi::class_addMethod(std::ptr::from_ref(class).cast_mut(), sel!(application:openURLs:), imp, c"v@:@@".as_ptr())
        };
        if !added.as_bool() {
            eprintln!("wordcraft-macos-open: {} already implements application:openURLs:", class.name().to_string_lossy());
        }
    }

    pub fn install(on_open: Callback) {
        if MainThreadMarker::new().is_none() {
            eprintln!("wordcraft-macos-open: install() must be called on the main thread");
            return;
        }
        if ON_OPEN.with(|cb| cb.set(on_open)).is_err() {
            return;
        }
        // Posted on the main thread from -[NSApplication finishLaunching], after winit set its delegate.
        let block = RcBlock::new(|_: NonNull<NSNotification>| {
            if let Some(mtm) = MainThreadMarker::new() {
                add_open_urls_method(mtm);
            }
        });
        // SAFETY: the name is a valid NSString constant; with no queue the block runs synchronously
        // on the posting (main) thread. The notification center keeps the observer alive.
        let _observer = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(NSApplicationWillFinishLaunchingNotification),
                None,
                None,
                &block,
            )
        };
    }
}
