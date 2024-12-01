use regex::Regex;
use unidecode::unidecode;

#[derive(Debug)]
pub struct Episode {
    pub id: i32,
    pub title: String,
    pub video_url: Option<String>,
    pub subtitle_url: Option<String>,
    pub episode_number: i32,
    pub tv_show_name: String,
}

impl Episode {
    pub fn filename(&self, extension: &str) -> String {
        let input = self.title.clone();
        let lowercased = input.to_lowercase();

        let unaccented = unidecode(&lowercased);
        let re = Regex::new(r"[^a-z0-9\s-]").unwrap();
        let cleaned = re.replace_all(&unaccented, "");
        let dash_replaced = cleaned.replace(" ", "-");
        let collapsed = Regex::new(r"-+").unwrap().replace_all(&dash_replaced, "-");
        let title = collapsed.trim_matches('-').to_string();

        // 3cat adds OVAs in the middle of seasons as episode 1, which is wrong, we add ova- to the filename
        if self.tv_show_name.to_lowercase().contains("ova") {
            format!("ova-{}-{}.{extension}", self.episode_number, title)
        } else {
            format!("{}-{}.{extension}", self.episode_number, title)
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
            id: 1,
            title: "T1xC7 - Veureu una cosa al·lucinant i màgica!".to_string(),
            video_url: None,
            subtitle_url: None,
            episode_number: 7,
            tv_show_name: "Tv show name".to_string(),
        };
        assert_eq!(
            episode.filename("mp4"),
            "7-t1xc7-veureu-una-cosa-allucinant-i-magica.mp4"
        );

        let episode_ova = Episode {
            id: 1,
            title: "T1xC7 - Veureu una cosa al·lucinant!".to_string(),
            video_url: None,
            subtitle_url: None,
            episode_number: 7,
            tv_show_name: "Tv show name (OVA)".to_string(),
        };
        assert_eq!(
            episode_ova.filename("mp4"),
            "ova-7-t1xc7-veureu-una-cosa-allucinant.mp4"
        );
        Ok(())
    }
}
