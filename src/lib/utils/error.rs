#![allow(dead_code)]
#![allow(clippy::enum_variant_names)]

use thiserror::Error;

use crate::{http_client, info_3cat::api_structs};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to retrieve TV show ID: {0}")]
    TvShowIdRetrievalError(String),
    #[error("Failed to retrieve episodes: {0}")]
    EpisodeRetrieveError(http_client::Error<api_structs::Tv3Error>),
    #[error("Failed to spawn yt-dlp command: {0}")]
    DownloadingError(std::io::Error),
    #[error("IO error, {1}: {0}")]
    IoError(String, std::io::Error),
    #[error("Error fixing subtitle: {0}")]
    SubtitleError(String),
    #[error("Failed to convert OsString to String: {0:?}")]
    OsStringError(std::ffi::OsString),
}

impl Error {
    pub fn io_error(message: &str, error: std::io::Error) -> Self {
        Error::IoError(message.to_string(), error)
    }

    pub fn subtitle_error(message: &str) -> Self {
        Error::SubtitleError(message.to_string())
    }
}
