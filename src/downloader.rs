//! Download logic for media video and subtitle files.

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tracing::{info, instrument, warn};

use crate::api_structs;
use crate::error::{Error, Result};
use crate::ffmpeg;
use crate::http_client::HttpClientTrait;
use crate::models::{DownloadParams, MediaItem, SubtitleMode};
use crate::subtitle_cleaner;

const TV3_SINGLE_MEDIA_API_URL: &str =
    "https://dinamics.ccma.cat/pvideo/media.jsp?media=video&version=0s&idint={id}";

/// Fetches metadata for a single media item and downloads its video and subtitle files.
///
/// Retrieves the video URL and subtitles from the 3cat API using the
/// [`DownloadParams`] configuration, then streams the files to the output
/// directory. The [`SubtitleMode`] inside `params` controls whether subtitles
/// are skipped, downloaded as separate files, or embedded into the video via
/// ffmpeg.
///
/// # Errors
///
/// Returns an error if the metadata fetch, download, or file I/O fails.
pub async fn fetch_and_download_media(mut item: MediaItem, params: &DownloadParams) -> Result<()> {
    let api_response = params
        .http_client
        .get::<api_structs::SingleEpisodeRoot, api_structs::Tv3Error>(
            TV3_SINGLE_MEDIA_API_URL
                .replace("{id}", &item.id.to_string())
                .as_str(),
            None,
        )
        .await
        .map_err(|e| Error::Decoding(e.to_string()))?;

    for url in api_response.media.url {
        if !url.active {
            continue;
        }
        item.video_url = Some(url.file);
        break;
    }

    if let Some(subtitles) = api_response.subtitles.as_ref().and_then(|s| s.first()) {
        item.subtitle_url = Some(subtitles.url.clone());
    } else if params.subtitle_mode != SubtitleMode::Skip {
        return Err(Error::NoSubtitlesAvailable(item.title.clone()));
    }

    let reqwest_client = params.http_client.inner();
    download_media(
        &item,
        &params.directory,
        &params.multi_progress,
        reqwest_client,
        params.subtitle_mode,
    )
    .await
}

/// Downloads the video and subtitle files for a media item to the given directory.
///
/// Skips the download if the file already exists and is non-empty.
/// Uses the provided [`MultiProgress`] to render concurrent progress bars,
/// and the shared [`Client`] for connection pooling.
///
/// # Errors
///
/// Returns an error if downloading, file I/O, or path encoding fails.
#[instrument(skip_all, fields(media_id = item.id))]
async fn download_media(
    item: &MediaItem,
    directory: &str,
    multi_progress: &MultiProgress,
    client: &Client,
    subtitle_mode: SubtitleMode,
) -> Result<()> {
    if check_if_media_exists(item, directory).await? {
        info!("Media item already exists: {}", item.filename("mp4")?);
        return Ok(());
    }

    download_data(item, directory, multi_progress, client, subtitle_mode).await
}

#[instrument(skip_all)]
async fn download_data(
    item: &MediaItem,
    directory: &str,
    multi_progress: &MultiProgress,
    client: &Client,
    subtitle_mode: SubtitleMode,
) -> Result<()> {
    let Some(video_url) = &item.video_url else {
        return Err(Error::MediaDoesNotHaveVideoUrl(item.filename("mp4")?));
    };

    let video_filename = item.filename("mp4")?;
    let video_path = full_media_path(item, directory, "mp4")?;
    download_content(
        video_url,
        &video_path,
        &video_filename,
        multi_progress,
        client,
    )
    .await?;
    info!("Downloaded video to {video_path}");

    if subtitle_mode == SubtitleMode::Skip {
        return Ok(());
    }

    if let Some(subtitle_url) = &item.subtitle_url {
        let subtitle_filename = item.filename("vtt")?;
        let subtitle_path = full_media_path(item, directory, "vtt")?;
        download_content(
            subtitle_url,
            &subtitle_path,
            &subtitle_filename,
            multi_progress,
            client,
        )
        .await?;

        subtitle_cleaner::clean_vtt_file(std::path::Path::new(&subtitle_path))?;

        if subtitle_mode == SubtitleMode::Embed {
            match ffmpeg::embed_subtitles(&video_path, &subtitle_path).await {
                Ok(mkv_path) => {
                    info!("Subtitles embedded into video {mkv_path}");
                }
                Err(e) => {
                    warn!("Failed to embed subtitles into {video_path}: {e}");
                    info!("Downloaded subtitle to {subtitle_path}");
                }
            }
        } else {
            info!("Downloaded subtitle to {subtitle_path}");
        }
    }

    Ok(())
}

#[instrument(skip_all)]
async fn check_if_media_exists(item: &MediaItem, directory: &str) -> Result<bool> {
    let video_path = full_media_path(item, directory, "mp4")?;
    let mkv_path = full_media_path(item, directory, "mkv")?;
    let tmp_path = format!("{video_path}.tmp");

    // Clean up stale .tmp files from previous interrupted runs
    let _ = tokio::fs::remove_file(&tmp_path).await;

    // Check for .mkv first (previously embedded with subtitles)
    if non_empty_file_exists(&mkv_path) {
        return Ok(true);
    }

    if !non_empty_file_exists(&video_path) {
        return Ok(false);
    }

    Ok(true)
}

/// Returns `true` when `path` exists and has a non-zero size.
///
/// Zero-byte files left by previous failed downloads are cleaned up
/// and treated as non-existent.
fn non_empty_file_exists(path: &str) -> bool {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return false;
    }
    if let Ok(metadata) = p.metadata() {
        if metadata.len() == 0 {
            let _ = std::fs::remove_file(p);
            return false;
        }
    }
    true
}

fn full_media_path(item: &MediaItem, directory: &str, extension: &str) -> Result<String> {
    let path = std::path::Path::new(directory).join(item.filename(extension)?);
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
///
/// # Errors
///
/// Returns an error if the progress bar template is invalid.
fn create_progress_bar(
    total_size: u64,
    label: &str,
    multi_progress: &MultiProgress,
) -> Result<ProgressBar> {
    let pb = multi_progress.add(ProgressBar::new(total_size));
    pb.set_style(
        ProgressStyle::with_template(
            "{prefix:.bold} [{bar:30.cyan/blue}] {percent}% ({bytes}/{total_bytes}) {bytes_per_sec} ETA {eta}",
        )
        .map_err(|e| Error::Downloading(e.to_string()))?
        .progress_chars("█░░"),
    );
    pb.set_prefix(label.to_string());
    Ok(pb)
}

/// Creates a spinner-style progress bar when total size is unknown, registered with the [`MultiProgress`].
///
/// # Errors
///
/// Returns an error if the spinner template is invalid.
fn create_spinner(label: &str, multi_progress: &MultiProgress) -> Result<ProgressBar> {
    let pb = multi_progress.add(ProgressBar::new_spinner());
    pb.set_style(
        ProgressStyle::with_template("{prefix:.bold} {spinner:.cyan} ({bytes}) {bytes_per_sec}")
            .map_err(|e| Error::Downloading(e.to_string()))?,
    );
    pb.set_prefix(label.to_string());
    Ok(pb)
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
        Some(total) => create_progress_bar(total, label, multi_progress)?,
        None => create_spinner(label, multi_progress)?,
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
