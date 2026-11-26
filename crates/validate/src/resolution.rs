//! Singleton source resolution. Absence requires a completed candidate discovery.

/// Why discovery or selection cannot establish one source.
pub(crate) type Reason = &'static str;

/// A singleton has one source, established absence, competing sources, or unavailable discovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resolution<T> {
    /// The uniquely observed candidate.
    One(T),
    /// Complete discovery found no candidate.
    Absent,
    /// Distinct candidates compete.
    Ambiguous(Reason),
    /// Discovery could not establish a candidate set.
    Unknown(Reason),
}

impl<T> Resolution<T> {
    /// Finish bounded discovery; an incomplete empty prefix never establishes absence.
    pub(crate) fn discovered(candidate: Option<T>, complete: bool) -> Self {
        match (candidate, complete) {
            (Some(value), _) => Self::One(value),
            (None, true) => Self::Absent,
            (None, false) => Self::Unknown("source discovery was not fully examined"),
        }
    }
}
