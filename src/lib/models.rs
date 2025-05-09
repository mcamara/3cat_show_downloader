use std::path::{Path, PathBuf};
use std::cell::OnceCell;

use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use tracing::debug;
use unidecode::unidecode;

use crate::error::Error;

static REGEX_CLEANER: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^a-z0-9A-Z\s-]").unwrap());
static REGEX_SUBTITLE_LANGUAGE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\.([a-z]+)\.fixed.vtt$").unwrap());

#[derive(Debug)]
pub struct Episode {
    pub title: String,
    pub video_url: String,
    pub episode_number: i32,
    pub season_number: i32,
    pub tv_show_name: String,
    cached_filename: OnceCell<String>,
}

impl Episode {
    pub fn new(
        title: String, 
        video_url: String, 
        episode_number: i32, 
        season_number: i32, 
        tv_show_name: String
    ) -> Self {
        Self {
            title,
            video_url,
            episode_number,
            season_number,
            tv_show_name,
            cached_filename: OnceCell::new(),
        }
    }

    pub fn filename(&self) -> &str {
        self.cached_filename.get_or_init(|| {
            let input = self.title.clone();
            let unaccented = unidecode(&input);
            let title = REGEX_CLEANER.replace_all(&unaccented, "");

            // 3cat adds OVAs in the middle of seasons as episode 1, which is wrong, we add ova- to the filename
            if self.tv_show_name.to_lowercase().contains("ova") {
                format!(
                    "S{:02}E{:02} (OVA) - {}",
                    self.season_number, self.episode_number, title
                )
            } else {
                format!(
                    "S{:02}E{:02} - {}",
                    self.season_number, self.episode_number, title
                )
            }
        })
    }
}

#[derive(Debug)]
pub struct Subtitle {
    path: PathBuf,

    /// ISO 639-3 language code
    language_code: String,
}

impl Subtitle {
    pub fn new(path: PathBuf) -> Result<Self> {
        let language_code = Self::get_subtitle_language_code(&path)?;
        Ok(Self {
            path,
            language_code,
        })
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn language_code(&self) -> &String {
        &self.language_code
    }

    fn get_subtitle_language_code(path: &Path) -> Result<String> {
        let language_code = Self::get_file_name_language_code(path).ok_or(Error::SubtitleError(
            "Failed to get language code".to_string(),
        ))?;

        let iso_639_3_code = isolang::Language::from_639_1(&language_code)
            .ok_or(Error::SubtitleError(format!(
                "Failed to convert ISO 639-1 language code '{}' to ISO 639-3",
                language_code
            )))?
            .to_639_3()
            .to_string();

        Ok(iso_639_3_code)
    }

    fn get_file_name_language_code(path: &Path) -> Option<String> {
        let filename = path.file_name()?.to_str()?;
        let captures = REGEX_SUBTITLE_LANGUAGE.captures(filename)?;

        let language_code = captures.get(1)?.as_str().to_string();
        debug!(
            "Language code: \"{}\" for filename \"{}\"",
            language_code, filename
        );

        Some(language_code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn episode_filename_test() -> Result<()> {
        let episode = Episode::new(
            "T1xC7 - Veureu una cosa al·lucinant i màgica!".to_string(),
            "".to_string(),
            7,
            1,
            "Tv show name".to_string(),
        );
        assert_eq!(
            episode.filename(),
            "S01E07 - T1xC7 - Veureu una cosa allucinant i magica"
        );

        let episode_ova = Episode::new(
            "T1xC7 - Veureu una cosa al·lucinant i màgica!".to_string(),
            "".to_string(),
            7,
            1,
            "Tv show name (OVA)".to_string(),
        );
        assert_eq!(
            episode_ova.filename(),
            "S01E07 (OVA) - T1xC7 - Veureu una cosa allucinant i magica"
        );
        Ok(())
    }
}
