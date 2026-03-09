//! Retrieves the internal TV show ID from a 3cat show page.

use regex::Regex;
use tracing::instrument;

use crate::error::{Error, Result};

const TV3_TV_SHOW_API_URL: &str = "https://www.3cat.cat/3cat/{slug}/";

/// Fetches the HTML page for the given show slug and extracts the `programatv_id`.
#[instrument]
pub async fn get_tv_show_id(slug: &str) -> Result<i32> {
    let response = reqwest::get(TV3_TV_SHOW_API_URL.replace("{slug}", slug).as_str())
        .await
        .map_err(|e| {
            Error::TvShowIdRetrieval(format!(
                "Error getting tv show id: {e} (is the tv show slug correct?)"
            ))
        })?;
    let html_content = response.text().await.map_err(|e| {
        Error::TvShowIdRetrieval(format!(
            "Error getting tv show id: {e} (is the tv show slug correct?)"
        ))
    })?;
    let re = Regex::new(r"programatv_id=(\d+)")?;

    let matches: Vec<_> = re.captures_iter(&html_content).collect();

    if let Some(last_match) = matches.last() {
        if let Some(programatv_id) = last_match.get(1) {
            return Ok(programatv_id.as_str().parse()?);
        }
    }

    Err(Error::TvShowIdRetrieval(
        "No id found in the tv show page".to_string(),
    ))
}
