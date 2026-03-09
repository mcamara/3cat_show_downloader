//! Download logic for episode video and subtitle files.

use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tracing::{info, instrument};

use crate::error::{Error, Result};
use crate::models::Episode;

/// Downloads the video and subtitle files for an episode to the given directory.
///
/// Skips the download if the episode file already exists and is non-empty.
///
/// # Errors
///
/// Returns an error if downloading, file I/O, or path encoding fails.
#[instrument(skip_all, fields(episode_number = episode.episode_number))]
pub async fn download_episode(episode: &Episode, directory: &str) -> Result<()> {
    if check_if_episode_exists(episode, directory).await? {
        info!("Episode {} already exists", episode.filename("mp4")?);
        return Ok(());
    }

    download_data(episode, directory).await
}

#[instrument(skip_all)]
async fn download_data(episode: &Episode, directory: &str) -> Result<()> {
    let Some(video_url) = &episode.video_url else {
        return Err(Error::EpisodeDoesNotHaveVideoUrl(episode.filename("mp4")?));
    };

    let video_path = full_episode_path(episode, directory, "mp4")?;
    download_content(video_url, &video_path).await?;
    info!("Downloaded video to {video_path}");

    if let Some(subtitle_url) = &episode.subtitle_url {
        let subtitle_path = full_episode_path(episode, directory, "vtt")?;
        download_content(subtitle_url, &subtitle_path).await?;
        info!("Downloaded subtitle to {subtitle_path}");
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
async fn download_content(url: &str, path: &str) -> Result<()> {
    let tmp_path = format!("{path}.tmp");

    let result = download_to_file(url, &tmp_path).await;

    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return result;
    }

    tokio::fs::rename(&tmp_path, path)
        .await
        .map_err(|e| Error::Downloading(e.to_string()))?;

    Ok(())
}

#[instrument(skip_all, fields(url, path))]
async fn download_to_file(url: &str, path: &str) -> Result<()> {
    let response = reqwest::get(url)
        .await
        .map_err(|e| Error::Downloading(e.to_string()))?;

    let mut file = File::create(path)
        .await
        .map_err(|e| Error::Downloading(e.to_string()))?;

    let content = response
        .bytes()
        .await
        .map_err(|e| Error::Downloading(e.to_string()))?;

    file.write_all(&content)
        .await
        .map_err(|e| Error::Downloading(e.to_string()))?;

    Ok(())
}
