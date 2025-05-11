use crate::{
    downloader, info_3cat::get_episodes_from_slug, models::Episode, subtitles_fix,
    utils::error::Error,
};
use anyhow::{Ok, Result};
use std::{path::Path, process::Command};
use tracing::{debug, info};

pub async fn download_all_episodes(
    episode_start: i32,
    tv_show_slug: &str,
    directory: &Path,
    keep_all_files: bool,
) -> Result<()> {
    let episodes = get_episodes_from_slug(tv_show_slug).await?;
    let episodes_count = episodes.len();

    for (i, episode) in episodes.iter().enumerate() {
        if episode.episode_number < episode_start {
            info!("Skipping episode {}", episode.episode_number);
            continue;
        }

        info!(
            "Downloading episode {} of {}: {}",
            i + 1, episodes_count, episode.title
        );

        downloader::download_episode(&episode, directory).await?;
        subtitles_fix::fix_subtitles(&episode, directory, keep_all_files)?;
    }
    Ok(())
}

pub async fn download_episode(episode: &Episode, directory: &Path) -> Result<()> {
    let original_video_path = episode.original_video_path(directory);
    let fixed_video_path = episode.fixed_video_path(directory);
    debug!(
        "Video path: {} -> {}",
        original_video_path.to_string_lossy(),
        fixed_video_path.to_string_lossy()
    );
    if original_video_path.exists() || fixed_video_path.exists() {
        info!("Episode {} already exists", episode.base_filename());
        return Ok(());
    }

    download_video_ytdlp(episode, directory)?;
    info!(
        "Downloaded video to {}",
        original_video_path.to_string_lossy()
    );
    Ok(())
}

fn download_video_ytdlp(episode: &Episode, directory: &Path) -> Result<()> {
    let status = Command::new("yt-dlp")
        .args(["--write-subs", "-N", "10", "-o"])
        .arg(episode.original_video_filename())
        .arg(&episode.video_url)
        .current_dir(directory)
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(
            Error::DownloadingError(format!("Failed to download episode: {}", episode.video_url))
                .into(),
        )
    }
}
