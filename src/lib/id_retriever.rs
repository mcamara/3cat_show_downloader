use crate::error::Error;
use anyhow::Result;
use regex::Regex;
use tracing::{info, trace};

const TV3_TV_SHOW_API_URL: &str = "https://www.3cat.cat/3cat/{slug}/";

pub async fn get_tv_show_id(slug: &str) -> Result<i32, Error> {
    info!("Getting tv show information for slug: {}", slug);
    let response = reqwest::get(TV3_TV_SHOW_API_URL.replace("{slug}", slug).as_str())
        .await
        .map_err(|e| {
            Error::TvShowIdRetrievalError(format!(
                "Error getting tv show id: {} (is the tv show slug correct?)",
                e
            ))
        })?;
    let html_content = response.text().await.map_err(|e| {
        Error::TvShowIdRetrievalError(format!(
            "Error getting tv show id: {} (is the tv show slug correct?)",
            e
        ))
    })?;
    trace!("HTML content: {}", html_content);
    let re = Regex::new(r"programatv_id=(\d+)").unwrap();

    let matches: Vec<_> = re.captures_iter(&html_content).collect();

    if let Some(last_match) = matches.last() {
        if let Some(programatv_id) = last_match.get(1) {
            return Ok(programatv_id.as_str().parse().unwrap());
        }
    }

    Err(Error::TvShowIdRetrievalError(
        "No id found in the the tv show page".to_string(),
    ))
}
