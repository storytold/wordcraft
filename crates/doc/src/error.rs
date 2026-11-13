use super::position::StoryRef;

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum DocError {
    #[error("offset {0} is not a character boundary in the paragraph")]
    BadOffset(usize),
    #[error("no paragraph at {0}")]
    BadPath(String),
    #[error("no story {0:?}")]
    BadStory(StoryRef),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, DocError>;
