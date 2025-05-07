#![allow(dead_code)]
#![allow(clippy::enum_variant_names)]

use thiserror::Error;

use crate::{api_structs, http_client};

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
}
