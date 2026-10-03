//! TV show ID retrieval and episode downloading from 3cat.

mod api_structs;
mod episodes;

use regex::Regex;
use tracing::{info, instrument};

use crate::error::{Error, Result};
use crate::models::DownloadParams;
use crate::scheduler;

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
    extract_tv_show_id(&html_content)
}

fn extract_tv_show_id(html_content: &str) -> Result<i32> {
    let re = Regex::new(r#"programatv_id=(\d+)|"programaId":(\d+)"#)?;

    re.captures_iter(html_content)
        .last()
        .and_then(|captures| captures.get(1).or_else(|| captures.get(2)))
        .ok_or_else(|| Error::TvShowIdRetrieval("No id found in the tv show page".to_string()))?
        .as_str()
        .parse()
        .map_err(Into::into)
}

/// Downloads all episodes for a TV show, filtering and scheduling concurrently.
///
/// # Errors
///
/// Returns an error if episode fetching or any download task fails.
#[instrument(skip(params))]
pub async fn download(
    tv_show_id: i32,
    start_from_episode: i32,
    params: &DownloadParams,
) -> anyhow::Result<()> {
    let episodes = episodes::get_episodes(&params.http_client, tv_show_id).await?;

    let episodes_to_download: Vec<_> = episodes
        .into_iter()
        .filter(|ep| match ep.episode_number {
            Some(num) if num < start_from_episode => {
                info!("Skipping episode {num}");
                false
            }
            _ => true,
        })
        .collect();

    if episodes_to_download.is_empty() {
        info!("No episodes to download");
        return Ok(());
    }

    info!(
        "Downloading {} episodes ({} at a time)",
        episodes_to_download.len(),
        params.concurrent_downloads,
    );

    scheduler::download_all(episodes_to_download, params).await
}
