use once_cell::sync::Lazy;
use regex::Regex;
use unidecode::unidecode;

static REGEX_CLEANER: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^a-z0-9A-Z\s-]").unwrap());

#[derive(Debug)]
pub struct Episode {
    pub title: String,
    pub video_url: String,
    pub episode_number: i32,
    pub season_number: i32,
    pub tv_show_name: String,
}

impl Episode {
    pub fn filename(&self) -> String {
        let input = self.title.clone();

        let unaccented = unidecode(&input);
        let title = REGEX_CLEANER.replace_all(&unaccented, "");

        // 3cat adds OVAs in the middle of seasons as episode 1, which is wrong, we add ova- to the filename
        if self.tv_show_name.to_lowercase().contains("ova") {
            format!("S{:02}E{:02} (OVA) - {}", self.season_number, self.episode_number, title)
        } else {
            format!("S{:02}E{:02} - {}", self.season_number, self.episode_number, title)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn episode_filename_test() -> Result<()> {
        let episode = Episode {
            title: "T1xC7 - Veureu una cosa al·lucinant i màgica!".to_string(),
            video_url: "".to_string(),
            episode_number: 7,
            season_number: 1,
            tv_show_name: "Tv show name".to_string(),
        };
        assert_eq!(
            episode.filename(),
            "S01E07 - T1xC7 - Veureu una cosa allucinant i magica"
        );

        let episode_ova = Episode {
            title: "T1xC7 - Veureu una cosa al·lucinant i màgica!".to_string(),
            video_url: "".to_string(),
            episode_number: 7,
            season_number: 1,
            tv_show_name: "Tv show name (OVA)".to_string(),
        };
        assert_eq!(
            episode_ova.filename(),
            "S01E07 (OVA) - T1xC7 - Veureu una cosa allucinant i magica"
        );
        Ok(())
    }
}
