//! Application-level error types for the cat show downloader.

/// Application-level result type alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors that can occur during the cat show download process.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Failed to retrieve the TV show ID from the 3cat website.
    #[error("TV show ID retrieval failed: {0}")]
    TvShowIdRetrieval(String),

    /// Failed to decode an API response.
    #[error("decoding error: {0}")]
    Decoding(String),

    /// The episode does not have a video URL.
    #[error("episode does not have a video URL: {0}")]
    EpisodeDoesNotHaveVideoUrl(String),

    /// An error occurred during file downloading.
    #[error("downloading error: {0}")]
    Downloading(String),

    /// A regex compilation failed.
    #[error("regex error: {source}")]
    Regex {
        /// The underlying regex error.
        #[from]
        source: regex::Error,
    },

    /// Failed to convert a path to a UTF-8 string.
    #[error("path contains invalid UTF-8: {0}")]
    InvalidPathEncoding(String),

    /// Failed to parse a numeric value.
    #[error("parse int error: {source}")]
    ParseInt {
        /// The underlying parse error.
        #[from]
        source: std::num::ParseIntError,
    },
}
