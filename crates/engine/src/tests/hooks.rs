//! Engine hook registration (document loaded, typing AutoFormat).

use std::sync::MutexGuard;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use wordcraft_doc::Document;

use crate::Session;
use crate::hooks::{self, register_on_document_loaded, register_on_text_autoformat, run_on_text_autoformat};

/// A session marker no other test uses, so a hook registered here is inert for every other test's
/// session even while the registries are shared across the whole test process.
const MARK: &str = "wordcraft-engine-hook-test";

/// The hook registries are process-wide. Holding this guard serialises the hook tests and empties
/// the registries before and after each one, so registration order inside a test is the only order
/// that can matter.
struct Hooks {
    _lock: MutexGuard<'static, ()>,
}

impl Hooks {
    fn new() -> Self {
        let lock = hooks::HOOK_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        hooks::reset_for_test();
        Hooks { _lock: lock }
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        hooks::reset_for_test();
    }
}

#[test]
fn document_loaded_hooks_run_after_set_document() {
    let _hooks = Hooks::new();
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    register_on_document_loaded(|_| {
        CALLS.fetch_add(1, Ordering::SeqCst);
    });
    let before = CALLS.load(Ordering::SeqCst);
    let mut s = Session::new(Document::new());
    s.set_document(Document::new());
    assert!(CALLS.load(Ordering::SeqCst) > before, "set_document runs document-loaded hooks");
}

#[test]
fn text_autoformat_hooks_stop_after_the_first_change() {
    let _hooks = Hooks::new();
    static ORDER: AtomicUsize = AtomicUsize::new(0);
    register_on_text_autoformat(|s, _| {
        if s.author != MARK {
            return Ok(false);
        }
        ORDER.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    });
    register_on_text_autoformat(|s, _| {
        if s.author != MARK {
            return Ok(false);
        }
        ORDER.fetch_add(10, Ordering::SeqCst);
        Ok(false)
    });
    let mut s = Session::new(Document::new());
    s.author = MARK.into();
    let before = ORDER.load(Ordering::SeqCst);
    assert!(run_on_text_autoformat(&mut s, "1").unwrap());
    let after = ORDER.load(Ordering::SeqCst);
    assert_eq!(after, before + 1, "only the first hook that returns true should run");
}

#[test]
fn text_autoformat_hook_runs_from_text_insert() {
    let _hooks = Hooks::new();
    static HIT: AtomicUsize = AtomicUsize::new(0);
    register_on_text_autoformat(|s, text| {
        if s.author == MARK && text == "z" {
            HIT.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        } else {
            Ok(false)
        }
    });
    let before = HIT.load(Ordering::SeqCst);
    let mut s = Session::new(Document::new());
    s.author = MARK.into();
    s.run("text.insert", &json!({"text": "z"})).unwrap();
    assert_eq!(HIT.load(Ordering::SeqCst), before + 1);
}
