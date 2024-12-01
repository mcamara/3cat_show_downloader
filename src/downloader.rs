use crate::{
    error::{Error, Result},
    Episode,
};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

pub async fn download_episode(episode: &Episode, directory: &str) -> Result<()> {
    if check_if_episode_exists(episode, directory).await {
        println!("Episode {} already exists", episode.filename("mp4"));
        return Ok(());
    }

    download_data(episode, directory).await
}

async fn download_data(episode: &Episode, directory: &str) -> Result<()> {
    let Some(video_url) = &episode.video_url else {
        return Err(Error::EpisodeDoNotHaveVideoUrl(episode.filename("mp4")));
    };

    let video_path = full_episode_path(episode, directory, "mp4");
    download_content(video_url, &video_path).await?;
    println!("Downloaded video to {}", video_path);

    if let Some(subtitle_url) = &episode.subtitle_url {
        let subtitle_path = full_episode_path(episode, directory, "vtt");
        download_content(subtitle_url, &subtitle_path).await?;
        println!("Downloaded subtitle to {}", subtitle_path);
    }

    Ok(())
}

async fn check_if_episode_exists(episode: &Episode, directory: &str) -> bool {
    let video_path = full_episode_path(episode, directory, "mp4");
    std::path::Path::new(&video_path).exists()
}

fn full_episode_path(episode: &Episode, directory: &str, extension: &str) -> String {
    std::path::Path::new(directory)
        .join(episode.filename(extension))
        .to_str()
        .unwrap()
        .to_string()
}

async fn download_content(url: &str, path: &str) -> Result<()> {
    let response = reqwest::get(url)
        .await
        .map_err(|e| Error::DownloadingError(e.to_string()))?;

    let mut file = File::create(path)
        .await
        .map_err(|e| Error::DownloadingError(e.to_string()))?;

    let content = response
        .bytes()
        .await
        .map_err(|e| Error::DownloadingError(e.to_string()))?;

    file.write_all(&content)
        .await
        .map_err(|e| Error::DownloadingError(e.to_string()))?;

    Ok(())
}
