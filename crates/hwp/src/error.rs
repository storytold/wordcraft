//! The crate's error type.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::fmt;

/// Why a document could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not a document of ours, or damaged beyond reading.
    Format(String),
    /// A limit that protects against hostile files was reached.
    TooLarge(&'static str),
    /// Password-protected or distribution-only (Hancom: 배포용) documents.
    Protected,
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Format(m) => write!(f, "not a readable Hangul document: {m}"),
            Error::TooLarge(what) => write!(f, "Hangul document too large: {what}"),
            Error::Protected => f.write_str("the Hangul document is password-protected or distribution-only"),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    pub(crate) fn format(m: impl fmt::Display) -> Self {
        Error::Format(m.to_string())
    }
}
