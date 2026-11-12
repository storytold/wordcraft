//! A count of the steps the layout's lookups take, for the linear-time tests (#3692): they check
//! that an input 8x the size costs about 8x the steps, which a busy machine can't skew the way it
//! skews a timing. Each walk that once ran again for every item it served (a line's clusters, a
//! page's items, the revision authors, the sections, a table's rows) counts the elements it
//! looks at, so a caller that walks it once per item again shows up as a quadratic count. Outside
//! tests the count is nothing.

#[cfg(test)]
thread_local! {
    /// The steps counted on this thread (layout runs on the thread that calls it).
    static STEPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Count `n` steps of a walk (nothing outside tests).
#[inline(always)]
pub(crate) fn steps(n: usize) {
    #[cfg(test)]
    STEPS.with(|s| s.set(s.get().saturating_add(n as u64)));
    #[cfg(not(test))]
    let _ = n;
}

/// The steps `f` takes.
#[cfg(test)]
pub(crate) fn count(f: impl FnOnce()) -> u64 {
    let before = STEPS.with(std::cell::Cell::get);
    f();
    STEPS.with(std::cell::Cell::get).saturating_sub(before)
}
