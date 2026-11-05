//! A count of the characters a paragraph's object lookups walk, for the linear-time tests of the
//! crates that use them: an input 8x the size must cost about 8x the steps, which a busy machine
//! can't skew the way it skews a timing. Counted only in tests (this crate's, and other crates'
//! through the `test-support` feature, which they turn on as a dev-dependency); in the app the
//! count is nothing.

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// The steps counted on this thread.
    static STEPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Count `n` steps of a walk (nothing outside tests). Public for the walks other crates do over a
/// paragraph's text themselves (Markdown and HTML comment export, for one).
#[inline(always)]
pub fn steps(n: usize) {
    #[cfg(any(test, feature = "test-support"))]
    STEPS.with(|s| s.set(s.get().saturating_add(n as u64)));
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = n;
}

/// The steps `f` takes.
#[cfg(any(test, feature = "test-support"))]
pub fn count(f: impl FnOnce()) -> u64 {
    let before = STEPS.with(std::cell::Cell::get);
    f();
    STEPS.with(std::cell::Cell::get).saturating_sub(before)
}
