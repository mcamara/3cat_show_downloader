//! Media item model, subtitle handling mode, download parameters, and filename generation.

use std::sync::Arc;

use indicatif::MultiProgress;
use regex::Regex;
use unidecode::unidecode;

use crate::error::Result;
use crate::http_client::HttpClient;

/// Controls how subtitles are handled during media downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleMode {
    /// Do not download subtitles at all.
    Skip,
    /// Download subtitles as separate `.vtt` files.
    Download,
    /// Download subtitles and embed them into the video file via ffmpeg.
    Embed,
}

/// Common parameters shared across media download operations.
///
/// All fields are cheaply cloneable so the struct can be shared across
/// spawned Tokio tasks without lifetime constraints.
#[derive(Clone, Debug)]
pub struct DownloadParams {
    /// Shared HTTP client instance.
    pub http_client: Arc<HttpClient>,
    /// How subtitles should be handled during downloads.
    pub subtitle_mode: SubtitleMode,
    /// Number of concurrent download tasks (1-10).
    pub concurrent_downloads: u8,
    /// Shared multi-progress bar renderer.
    pub multi_progress: MultiProgress,
    /// Output directory for downloaded files.
    pub directory: Arc<str>,
}

/// Represents a downloadable media item (TV show episode or movie).
#[derive(Debug)]
pub struct MediaItem {
    /// Internal 3cat media ID.
    pub id: i32,
    /// Title of the media item.
    pub title: String,
    /// URL to the video file, populated after fetching media details.
    pub video_url: Option<String>,
    /// URL to the subtitle file, populated after fetching media details.
    pub subtitle_url: Option<String>,
    /// Sequential episode number within the show (`None` for movies).
    pub episode_number: Option<i32>,
    /// Name of the TV show this episode belongs to (`None` for movies).
    pub tv_show_name: Option<String>,
}

impl MediaItem {
    /// Generates a sanitized filename for the media item with the given extension.
    ///
    /// For TV show episodes the filename includes the episode number as a prefix
    /// (e.g. `7-title.mp4`). Episodes whose show name contains "OVA" receive an
    /// additional `ova-` prefix. Movies produce a title-only filename (e.g.
    /// `title.mp4`).
    ///
    /// # Errors
    ///
    /// Returns an error if the internal regex patterns fail to compile.
    pub fn filename(&self, extension: &str) -> Result<String> {
        let slug = Self::slugify(&self.title)?;

        match (self.episode_number, &self.tv_show_name) {
            (Some(ep_num), Some(show_name)) if show_name.to_lowercase().contains("ova") => {
                Ok(format!("ova-{ep_num}-{slug}.{extension}"))
            }
            (Some(ep_num), _) => Ok(format!("{ep_num}-{slug}.{extension}")),
            (None, _) => Ok(format!("{slug}.{extension}")),
        }
    }

    /// Converts a title into a URL-friendly slug.
    fn slugify(title: &str) -> Result<String> {
        let lowercased = title.to_lowercase();
        let unaccented = unidecode(&lowercased);
        let re = Regex::new(r"[^a-z0-9\s-]")?;
        let cleaned = re.replace_all(&unaccented, "");
        let dash_replaced = cleaned.replace(' ', "-");
        let collapsed = Regex::new(r"-+")?.replace_all(&dash_replaced, "-");
        Ok(collapsed.trim_matches('-').to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_generate_correct_filename_for_episode() {
        let item = MediaItem {
            id: 1,
            title: "T1xC7 - Veureu una cosa al·lucinant i màgica!".to_string(),
            video_url: None,
            subtitle_url: None,
            episode_number: Some(7),
            tv_show_name: Some("Tv show name".to_string()),
        };
        assert_eq!(
            item.filename("mp4").unwrap(),
            "7-t1xc7-veureu-una-cosa-allucinant-i-magica.mp4"
        );
    }

    #[test]
    fn test_should_prefix_ova_in_filename() {
        let item = MediaItem {
            id: 1,
            title: "T1xC7 - Veureu una cosa al·lucinant!".to_string(),
            video_url: None,
            subtitle_url: None,
            episode_number: Some(7),
            tv_show_name: Some("Tv show name (OVA)".to_string()),
        };
        assert_eq!(
            item.filename("mp4").unwrap(),
            "ova-7-t1xc7-veureu-una-cosa-allucinant.mp4"
        );
    }

    #[test]
    fn test_should_generate_filename_for_movie() {
        let item = MediaItem {
            id: 42,
            title: "El secret de la cova".to_string(),
            video_url: None,
            subtitle_url: None,
            episode_number: None,
            tv_show_name: None,
        };
        assert_eq!(item.filename("mp4").unwrap(), "el-secret-de-la-cova.mp4");
    }
}
