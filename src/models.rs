//! Episode model, subtitle handling mode, and filename generation.

use regex::Regex;
use unidecode::unidecode;

use crate::error::Result;

/// Controls how subtitles are handled during episode downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleMode {
    /// Do not download subtitles at all.
    Skip,
    /// Download subtitles as separate `.vtt` files.
    Download,
    /// Download subtitles and embed them into the video file via ffmpeg.
    Embed,
}

/// Represents a single TV show episode with its metadata.
#[derive(Debug)]
pub struct Episode {
    /// Internal 3cat episode ID.
    pub id: i32,
    /// Episode title from the API.
    pub title: String,
    /// URL to the video file, populated after fetching single-episode details.
    pub video_url: Option<String>,
    /// URL to the subtitle file, populated after fetching single-episode details.
    pub subtitle_url: Option<String>,
    /// Sequential episode number within the show.
    pub episode_number: i32,
    /// Name of the TV show this episode belongs to.
    pub tv_show_name: String,
}

impl Episode {
    /// Generates a sanitized filename for the episode with the given extension.
    ///
    /// # Errors
    ///
    /// Returns an error if the internal regex patterns fail to compile.
    pub fn filename(&self, extension: &str) -> Result<String> {
        let lowercased = self.title.to_lowercase();

        let unaccented = unidecode(&lowercased);
        let re = Regex::new(r"[^a-z0-9\s-]")?;
        let cleaned = re.replace_all(&unaccented, "");
        let dash_replaced = cleaned.replace(' ', "-");
        let collapsed = Regex::new(r"-+")?.replace_all(&dash_replaced, "-");
        let title = collapsed.trim_matches('-').to_string();

        // 3cat adds OVAs in the middle of seasons as episode 1, which is wrong, we add ova- to the filename
        if self.tv_show_name.to_lowercase().contains("ova") {
            Ok(format!("ova-{}-{}.{extension}", self.episode_number, title))
        } else {
            Ok(format!("{}-{}.{extension}", self.episode_number, title))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_generate_correct_filename() {
        let episode = Episode {
            id: 1,
            title: "T1xC7 - Veureu una cosa al·lucinant i màgica!".to_string(),
            video_url: None,
            subtitle_url: None,
            episode_number: 7,
            tv_show_name: "Tv show name".to_string(),
        };
        assert_eq!(
            episode.filename("mp4").unwrap(),
            "7-t1xc7-veureu-una-cosa-allucinant-i-magica.mp4"
        );
    }

    #[test]
    fn test_should_prefix_ova_in_filename() {
        let episode_ova = Episode {
            id: 1,
            title: "T1xC7 - Veureu una cosa al·lucinant!".to_string(),
            video_url: None,
            subtitle_url: None,
            episode_number: 7,
            tv_show_name: "Tv show name (OVA)".to_string(),
        };
        assert_eq!(
            episode_ova.filename("mp4").unwrap(),
            "ova-7-t1xc7-veureu-una-cosa-allucinant.mp4"
        );
    }
}
