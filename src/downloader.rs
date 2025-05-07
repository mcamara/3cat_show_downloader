use crate::{error::Error, Episode};
use anyhow::{Ok, Result};

pub async fn download_episode(episode: &Episode, directory: &str) -> Result<()> {
    let video_path = full_episode_path(episode, directory);
    if std::path::Path::new(&format!("{}.mp4", video_path)).exists() {
        println!("Episode {} already exists", episode.filename());
        return Ok(());
    }
    download_video_ytdlp(episode, directory)?;
    println!("Downloaded video to {}", video_path);
    Ok(())
}

fn full_episode_path(episode: &Episode, directory: &str) -> String {
    std::path::Path::new(directory)
        .join(episode.filename())
        .to_str()
        .unwrap()
        .to_string()
}

fn download_video_ytdlp(episode: &Episode, directory: &str) -> Result<()> {
    // Find the yt-dlp binary in the PATH
    let mut command = std::process::Command::new("yt-dlp");
    command
        .args(["--write-subs", "-N", "10", "-o"])
        .arg(episode.filename())
        .arg(&episode.video_url)
        .current_dir(directory);

    command
        .spawn()
        .map_err(|e| Error::DownloadingError(e).into())
        .map(|_| ())
}
