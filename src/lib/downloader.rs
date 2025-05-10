use crate::{
    downloader, info_3cat::get_episodes_from_slug, models::Episode, subtitles_fix,
    utils::error::Error,
};
use anyhow::{Ok, Result};
use std::path::Path;
use tracing::info;

pub async fn download_all_episodes(
    episode_start: i32,
    tv_show_slug: &str,
    directory: &Path,
) -> Result<()> {
    let episodes = get_episodes_from_slug(tv_show_slug).await?;
    let episodes_count = episodes.len();

    for episode in episodes {
        if episode.episode_number < episode_start {
            info!("Skipping episode {}", episode.episode_number);
            continue;
        }

        info!(
            "Downloading episode {} of {}: {}",
            episode.episode_number, episodes_count, episode.title
        );

        downloader::download_episode(&episode, directory).await?;
        subtitles_fix::fix_subtitles(&episode, directory)?;
        break;
    }
    Ok(())
}

pub async fn download_episode(episode: &Episode, directory: &Path) -> Result<()> {
    let video_path = episode.original_video_path(directory);
    if video_path.exists() {
        info!("Episode {} already exists", episode.base_filename());
        return Ok(());
    }

    download_video_ytdlp(episode, directory)?;
    info!("Downloaded video to {}", video_path.to_string_lossy());
    Ok(())
}

fn download_video_ytdlp(episode: &Episode, directory: &Path) -> Result<()> {
    // Find the yt-dlp binary in the PATH
    let mut command = std::process::Command::new("yt-dlp");
    command
        .args(["--write-subs", "-N", "10", "-o"])
        .arg(episode.base_filename())
        .arg(&episode.video_url)
        .current_dir(directory);

    command
        .spawn()
        .map_err(|e| Error::DownloadingError(e).into())
        .map(|_| ())
}
