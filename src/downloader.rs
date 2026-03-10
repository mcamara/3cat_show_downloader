//! Download logic for episode video and subtitle files.

use std::sync::Arc;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tracing::{info, instrument};

use crate::api_structs;
use crate::error::{Error, Result};
use crate::http_client::{HttpClient, HttpClientTrait};
use crate::models::Episode;
use crate::subtitle_cleaner;

const TV3_SINGLE_EPISODE_API_URL: &str =
    "https://dinamics.ccma.cat/pvideo/media.jsp?media=video&version=0s&idint={id}";

/// Fetches metadata for a single episode and downloads its video and subtitle files.
///
/// Retrieves the video URL and subtitles from the 3cat API, then streams
/// the files to the given directory using the shared [`Client`] for
/// connection pooling and the [`MultiProgress`] for concurrent progress bars.
/// When `skip_subtitles` is `true`, subtitle downloading is skipped entirely.
///
/// # Errors
///
/// Returns an error if the metadata fetch, download, or file I/O fails.
pub async fn fetch_and_download_episode(
    mut episode: Episode,
    http_client: &Arc<HttpClient>,
    multi_progress: &MultiProgress,
    client: &Client,
    directory: &str,
    skip_subtitles: bool,
) -> Result<()> {
    let tv3_tv_show_api_response = http_client
        .get::<api_structs::SingleEpisodeRoot, api_structs::Tv3Error>(
            TV3_SINGLE_EPISODE_API_URL
                .replace("{id}", &episode.id.to_string())
                .as_str(),
            None,
        )
        .await
        .map_err(|e| Error::Decoding(e.to_string()))?;

    for url in tv3_tv_show_api_response.media.url {
        if !url.active {
            continue;
        }
        episode.video_url = Some(url.file);
        break;
    }

    if let Some(subtitles) = tv3_tv_show_api_response.subtitles.first() {
        episode.subtitle_url = Some(subtitles.url.clone());
    }

    download_episode(&episode, directory, multi_progress, client, skip_subtitles).await
}

/// Downloads the video and subtitle files for an episode to the given directory.
///
/// Skips the download if the episode file already exists and is non-empty.
/// Uses the provided [`MultiProgress`] to render concurrent progress bars,
/// and the shared [`Client`] for connection pooling.
///
/// # Errors
///
/// Returns an error if downloading, file I/O, or path encoding fails.
#[instrument(skip_all, fields(episode_number = episode.episode_number))]
async fn download_episode(
    episode: &Episode,
    directory: &str,
    multi_progress: &MultiProgress,
    client: &Client,
    skip_subtitles: bool,
) -> Result<()> {
    if check_if_episode_exists(episode, directory).await? {
        info!("Episode {} already exists", episode.filename("mp4")?);
        return Ok(());
    }

    download_data(episode, directory, multi_progress, client, skip_subtitles).await
}

#[instrument(skip_all)]
async fn download_data(
    episode: &Episode,
    directory: &str,
    multi_progress: &MultiProgress,
    client: &Client,
    skip_subtitles: bool,
) -> Result<()> {
    let Some(video_url) = &episode.video_url else {
        return Err(Error::EpisodeDoesNotHaveVideoUrl(episode.filename("mp4")?));
    };

    let video_filename = episode.filename("mp4")?;
    let video_path = full_episode_path(episode, directory, "mp4")?;
    download_content(
        video_url,
        &video_path,
        &video_filename,
        multi_progress,
        client,
    )
    .await?;
    info!("Downloaded video to {video_path}");

    if skip_subtitles {
        return Ok(());
    }

    if let Some(subtitle_url) = &episode.subtitle_url {
        let subtitle_filename = episode.filename("vtt")?;
        let subtitle_path = full_episode_path(episode, directory, "vtt")?;
        download_content(
            subtitle_url,
            &subtitle_path,
            &subtitle_filename,
            multi_progress,
            client,
        )
        .await?;
        info!("Downloaded subtitle to {subtitle_path}");

        subtitle_cleaner::clean_vtt_file(std::path::Path::new(&subtitle_path))?;
    }

    Ok(())
}

#[instrument(skip_all)]
async fn check_if_episode_exists(episode: &Episode, directory: &str) -> Result<bool> {
    let video_path = full_episode_path(episode, directory, "mp4")?;
    let tmp_path = format!("{video_path}.tmp");

    // Clean up stale .tmp files from previous interrupted runs
    let _ = tokio::fs::remove_file(&tmp_path).await;

    let path = std::path::Path::new(&video_path);
    if !path.exists() {
        return Ok(false);
    }

    // Clean up 0-byte files left by previous failed downloads
    if let Ok(metadata) = path.metadata() {
        if metadata.len() == 0 {
            let _ = tokio::fs::remove_file(&video_path).await;
            return Ok(false);
        }
    }

    Ok(true)
}

fn full_episode_path(episode: &Episode, directory: &str, extension: &str) -> Result<String> {
    let path = std::path::Path::new(directory).join(episode.filename(extension)?);
    path.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| Error::InvalidPathEncoding(format!("{}", path.display())))
}

#[instrument(skip_all, fields(url, path))]
async fn download_content(
    url: &str,
    path: &str,
    label: &str,
    multi_progress: &MultiProgress,
    client: &Client,
) -> Result<()> {
    let tmp_path = format!("{path}.tmp");

    let result = download_to_file(url, &tmp_path, label, multi_progress, client).await;

    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return result;
    }

    tokio::fs::rename(&tmp_path, path)
        .await
        .map_err(|e| Error::Downloading(e.to_string()))?;

    Ok(())
}

/// Creates a styled progress bar for download tracking, registered with the [`MultiProgress`].
fn create_progress_bar(
    total_size: u64,
    label: &str,
    multi_progress: &MultiProgress,
) -> ProgressBar {
    let pb = multi_progress.add(ProgressBar::new(total_size));
    pb.set_style(
        ProgressStyle::with_template(
            "{prefix:.bold} [{bar:30.cyan/blue}] {percent}% ({bytes}/{total_bytes}) {bytes_per_sec} ETA {eta}",
        )
        .expect("valid progress bar template")
        .progress_chars("█░░"),
    );
    pb.set_prefix(label.to_string());
    pb
}

/// Creates a spinner-style progress bar when total size is unknown, registered with the [`MultiProgress`].
fn create_spinner(label: &str, multi_progress: &MultiProgress) -> ProgressBar {
    let pb = multi_progress.add(ProgressBar::new_spinner());
    pb.set_style(
        ProgressStyle::with_template("{prefix:.bold} {spinner:.cyan} ({bytes}) {bytes_per_sec}")
            .expect("valid spinner template"),
    );
    pb.set_prefix(label.to_string());
    pb
}

#[instrument(skip_all, fields(url, path))]
async fn download_to_file(
    url: &str,
    path: &str,
    label: &str,
    multi_progress: &MultiProgress,
    client: &Client,
) -> Result<()> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Downloading(e.to_string()))?;

    let mut file = File::create(path)
        .await
        .map_err(|e| Error::Downloading(e.to_string()))?;

    let pb = match response.content_length() {
        Some(total) => create_progress_bar(total, label, multi_progress),
        None => create_spinner(label, multi_progress),
    };

    let mut downloaded: u64 = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| Error::Downloading(e.to_string()))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|e| Error::Downloading(e.to_string()))?;
        downloaded += chunk.len() as u64;
        pb.set_position(downloaded);
    }

    pb.finish_and_clear();

    Ok(())
}
