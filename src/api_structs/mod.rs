//! Serde deserialization structs for the shared 3cat media API responses.

use std::fmt::Display;

use serde::Deserialize;

/// Placeholder error type returned by the 3cat API on failure.
#[derive(Debug, Deserialize)]
pub struct Tv3Error {}

impl Display for Tv3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Error")
    }
}

/// Root wrapper for the single-media detail response.
#[derive(Debug, Deserialize)]
pub struct SingleEpisodeRoot {
    /// Video file metadata.
    pub media: SingleEpisodeMedia,
    /// Available subtitle tracks.
    #[serde(rename = "subtitols")]
    pub subtitles: Vec<SingleEpisodeSubtitles>,
}

/// Container for the list of video URLs.
#[derive(Debug, Deserialize)]
pub struct SingleEpisodeMedia {
    /// Available video file URLs with their active status.
    pub url: Vec<UrlMetadata>,
}

/// A single video URL entry from the media API.
#[derive(Debug, Deserialize)]
pub struct UrlMetadata {
    /// Direct URL to the video file.
    pub file: String,
    /// Whether this URL is currently active/available.
    pub active: bool,
}

/// A single subtitle track entry from the media API.
#[derive(Debug, Deserialize)]
pub struct SingleEpisodeSubtitles {
    /// Direct URL to the subtitle file.
    pub url: String,
}
