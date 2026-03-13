//! Movie ID retrieval and downloading from 3cat.

mod api_structs;

use regex::Regex;
use tracing::{info, instrument};

use crate::downloader;
use crate::error::{Error, Result};
use crate::models::{DownloadParams, MediaItem};

const MOVIES_CATALOG_URL: &str = "https://www.3cat.cat/3cat/tot-cataleg/pellicules/";

/// Fetches the 3cat movie catalog page and finds the movie ID for the given slug.
///
/// The catalog page embeds a `<script id="__NEXT_DATA__">` tag containing a JSON
/// payload with every available movie. This function extracts that JSON, parses it,
/// and searches for a movie whose `nom_friendly` field matches the provided slug.
///
/// # Errors
///
/// Returns [`Error::MovieIdRetrieval`] if the catalog page cannot be fetched,
/// the embedded JSON cannot be found or parsed, or no movie matches the slug.
#[instrument]
pub async fn get_movie_id(slug: &str) -> Result<i32> {
    let response = reqwest::get(MOVIES_CATALOG_URL)
        .await
        .map_err(|e| Error::MovieIdRetrieval(format!("Failed to fetch movie catalog: {e}")))?;

    let html = response
        .text()
        .await
        .map_err(|e| Error::MovieIdRetrieval(format!("Failed to read movie catalog body: {e}")))?;

    let re = Regex::new(
        r#"(?s)<script\s+id="__NEXT_DATA__"\s+type="application/json">\s*(.*?)\s*</script>"#,
    )?;

    let json_str = re
        .captures(&html)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str())
        .ok_or_else(|| {
            Error::MovieIdRetrieval("Could not find __NEXT_DATA__ script tag".to_string())
        })?;

    let catalog: api_structs::CatalogRoot = serde_json::from_str(json_str)
        .map_err(|e| Error::MovieIdRetrieval(format!("Failed to parse catalog JSON: {e}")))?;

    let movie = catalog
        .props
        .page_props
        .layout
        .structure
        .iter()
        .flat_map(|s| &s.children)
        .flat_map(|c| &c.final_props.items)
        .find(|item| item.nom_friendly == slug)
        .ok_or_else(|| {
            Error::MovieIdRetrieval(format!("Movie with slug '{slug}' not found in catalog"))
        })?;

    Ok(movie.id)
}

/// Downloads a movie from 3cat.
///
/// Constructs a [`MediaItem`] for the movie and delegates to the shared
/// download pipeline, which fetches the video URL and subtitles from the
/// 3cat API and streams the files to the output directory.
///
/// # Errors
///
/// Returns an error if the metadata fetch, download, or file I/O fails.
#[instrument(skip(params))]
pub async fn download(movie_id: i32, slug: &str, params: &DownloadParams) -> anyhow::Result<()> {
    info!("Downloading movie '{slug}' (id={movie_id})");

    let item = MediaItem {
        id: movie_id,
        title: slug.replace('-', " "),
        video_url: None,
        subtitle_url: None,
        episode_number: None,
        tv_show_name: None,
    };

    downloader::fetch_and_download_media(item, params).await?;

    Ok(())
}
