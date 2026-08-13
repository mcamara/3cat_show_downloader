//! Download logic for media video and subtitle files.

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tracing::{debug, instrument, warn};

use crate::api_structs;
use crate::error::{Error, Result};
use crate::ffmpeg;
use crate::http_client::HttpClientTrait;
use crate::models::{DownloadParams, MediaItem, SubtitleMode};
use crate::subtitle_cleaner;
use crate::transcode;
use crate::yt_dlp;

const TV3_SINGLE_MEDIA_API_URL: &str =
    "https://dinamics.ccma.cat/pvideo/media.jsp?media=video&version=0s&idint={id}";

/// Fetches metadata for a single media item and downloads its video and subtitle files.
///
/// When `params.yt_dlp_available` is `true`, delegates entirely to yt-dlp,
/// which handles format selection and subtitle extraction without a prior API
/// call. Otherwise, retrieves the video URL and subtitles from the 3cat API
/// and streams the files using the built-in HTTP downloader.
///
/// The [`SubtitleMode`] inside `params` controls whether subtitles are
/// skipped, downloaded as separate files, or embedded into the video.
///
/// Returns `Ok(true)` when the subtitle pipeline failed for this item and the
/// video was saved without subtitles (only in the default tolerant mode — in
/// `--strict-subtitles` mode the error is propagated instead). The scheduler
/// uses this flag to print a final summary of episodes that may need a
/// `--skip-subtitles` re-run.
///
/// # Errors
///
/// Returns an error if the metadata fetch, download, or file I/O fails.
pub async fn fetch_and_download_media(
    mut item: MediaItem,
    params: &DownloadParams,
) -> Result<bool> {
    if params.yt_dlp_available {
        let already_exists =
            check_if_media_exists(&item, &params.directory, params.auto_naming).await?;
        if already_exists {
            debug!(
                "Media item already exists: {} — applying reencode only",
                item.filename("(1080p)", params.auto_naming)?
            );
            // Re-run the post-download pipeline on the existing file. We
            // delegate to yt_dlp::download, which resolves the actual file
            // yt-dlp wrote (or that already exists), embeds subtitles if
            // asked, and then re-encodes. This way a previously downloaded
            // episode gets the reencode applied (and the subs pipeline
            // re-run) without hitting the network a second time.
        }
        ensure_subdir(&item, &params.directory, params.auto_naming).await?;
        let subtitle_failed = yt_dlp::download(
            &mut item,
            &params.directory,
            params.subtitle_mode,
            &params.multi_progress,
            params.strict_subtitles,
            params.auto_naming,
            params.reencode_preset,
            &params.reencode_capabilities,
            params.request_delay_ms,
        )
        .await?;
        return Ok(subtitle_failed);
    }

    let api_response = params
        .http_client
        .get::<serde_json::Value, api_structs::Tv3Error>(
            TV3_SINGLE_MEDIA_API_URL
                .replace("{id}", &item.id.to_string())
                .as_str(),
            None,
        )
        .await
        .map_err(|e| Error::Decoding {
            context: format!(
                "single episode url={}",
                TV3_SINGLE_MEDIA_API_URL.replace("{id}", &item.id.to_string())
            ),
            source: Box::new(e),
        })?;

    // The single-media endpoint returns a heterogeneous shape:
    // - `media.url` may be a single object `{file,active,...}` OR an array of
    //   such objects (when multiple qualities are available).
    // - `subtitols` may be absent, an empty array, or an array of objects.
    // Walk it defensively instead of pinning a single schema — we only care
    // about the first active video URL and the first subtitle URL.
    let mut video_urls: Vec<&serde_json::Value> = Vec::new();
    if let Some(url_value) = api_response.get("media").and_then(|m| m.get("url")) {
        match url_value {
            serde_json::Value::Array(arr) => video_urls.extend(arr.iter()),
            other => video_urls.push(other),
        }
    }
    for url in video_urls {
        let active = url.get("active").and_then(|a| a.as_bool()).unwrap_or(false);
        if !active {
            continue;
        }
        if let Some(file) = url.get("file").and_then(|f| f.as_str()) {
            item.video_url = Some(file.to_string());
            break;
        }
    }

    let subtitle_url = api_response
        .get("subtitols")
        .and_then(|s| match s {
            serde_json::Value::Array(arr) => arr.first(),
            other => Some(other),
        })
        .and_then(|sub| sub.get("url"))
        .and_then(|u| u.as_str())
        .map(str::to_string);
    if let Some(url) = subtitle_url {
        item.subtitle_url = Some(url);
    } else if params.subtitle_mode != SubtitleMode::Skip {
        if params.strict_subtitles {
            return Err(Error::NoSubtitlesAvailable(item.title.clone()));
        }
        warn!(
            "No subtitles available for '{}'. Saving video without subtitles; rerun with --strict-subtitles to abort on missing subs.",
            item.title
        );
        item.subtitle_failed = true;
    }

    let reqwest_client = params.http_client.inner();
    let subtitle_failed = download_media(
        &mut item,
        &params.directory,
        &params.multi_progress,
        reqwest_client,
        params.subtitle_mode,
        params.strict_subtitles,
        params.auto_naming,
        params.reencode_preset,
        &params.reencode_capabilities,
    )
    .await?;
    Ok(subtitle_failed || item.subtitle_failed)
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
#[allow(clippy::too_many_arguments)]
// Per-task context assembled from DownloadParams; keeping them explicit avoids an ad-hoc struct for a single callsite.
#[instrument(skip_all, fields(media_id = item.id))]
async fn download_media(
    item: &mut MediaItem,
    directory: &str,
    multi_progress: &MultiProgress,
    client: &Client,
    subtitle_mode: SubtitleMode,
    strict_subtitles: bool,
    auto_naming: bool,
    reencode_preset: transcode::ReencodePreset,
    capabilities: &ffmpeg::EncoderCapabilities,
) -> Result<bool> {
    if check_if_media_exists(item, directory, auto_naming).await? {
        debug!(
            "Media item already exists: {}",
            item.filename("mkv", auto_naming)?
        );
        return Ok(false);
    }

    ensure_subdir(item, directory, auto_naming).await?;

    let (final_path, subtitle_failed) = download_data(
        item,
        directory,
        multi_progress,
        client,
        subtitle_mode,
        strict_subtitles,
        auto_naming,
    )
    .await?;

    if !reencode_preset.is_off() {
        // Tolerate re-encode failure: warn and keep the original file so the
        // user still has a playable episode.
        if let Err(e) =
            transcode::reencode(&final_path, reencode_preset, capabilities, multi_progress).await
        {
            warn!("Re-encoding failed for {final_path}: {e}. Original file preserved.");
        }
    }

    // Final-stage guarantee: every downloaded file ends up as a `.mkv`,
    // regardless of which container yt-dlp / the built-in downloader
    // produced. Re-encode already writes MKV; subtitle embed already
    // produces MKV; this remux covers the "no-embed, no-reencode" case.
    // Idempotent: short-circuits when the file is already `.mkv`.
    if let Err(e) = ffmpeg::remux_to_mkv(std::path::Path::new(&final_path)).await {
        warn!("Remux to MKV failed for {final_path}: {e}. Original file preserved.");
    }

    Ok(subtitle_failed || item.subtitle_failed)
}

#[allow(clippy::too_many_arguments)]
// Same justification as download_media: per-task flat context built from DownloadParams once per episode.
#[instrument(skip_all)]
async fn download_data(
    item: &mut MediaItem,
    directory: &str,
    multi_progress: &MultiProgress,
    client: &Client,
    subtitle_mode: SubtitleMode,
    strict_subtitles: bool,
    auto_naming: bool,
) -> Result<(String, bool)> {
    let Some(video_url) = &item.video_url else {
        return Err(Error::MediaDoesNotHaveVideoUrl(
            item.filename("mkv", auto_naming)?,
        ));
    };

    let video_filename = item.filename("mkv", auto_naming)?;
    let video_path = full_media_path(item, directory, "mkv", auto_naming)?;
    download_content(
        video_url,
        &video_path,
        &video_filename,
        multi_progress,
        client,
    )
    .await?;
    debug!("Downloaded video to {video_path}");

    let mut subtitle_failed = false;
    let mut final_path = video_path.clone();

    if subtitle_mode != SubtitleMode::Skip
        && let Some(subtitle_url) = &item.subtitle_url
    {
        let subtitle_filename = item.filename("vtt", auto_naming)?;
        let subtitle_path = full_media_path(item, directory, "vtt", auto_naming)?;
        let video_path_for_subtitles = video_path.clone();
        let subtitle_result: Result<()> = async {
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
                let track = ffmpeg::SubtitleTrack {
                    path: std::path::PathBuf::from(&subtitle_path),
                    lang_code: "ca".to_string(),
                };
                match ffmpeg::embed_subtitles(&video_path_for_subtitles, &[track]).await {
                    Ok(mkv_path) => {
                        debug!("Subtitles embedded into video {mkv_path}");
                    }
                    Err(e) => {
                        warn!("Failed to embed subtitles into {video_path_for_subtitles}: {e}");
                        debug!("Downloaded subtitle to {subtitle_path}");
                    }
                }
            } else {
                debug!("Downloaded subtitle to {subtitle_path}");
            }

            Ok::<_, Error>(())
        }
        .await;

        match subtitle_result {
            Ok(()) => {
                // If embed succeeded, `embed_subtitles` renamed the file to
                // `.mkv` and removed the original `.mp4`. Surface the
                // post-embed path so re-encode targets the right file.
                let mkv_path = video_path_with_extension(&video_path_for_subtitles, "mkv");
                if std::path::Path::new(&mkv_path).exists() {
                    final_path = mkv_path;
                }
            }
            Err(e) => {
                if strict_subtitles {
                    return Err(e);
                }
                warn!(
                    "Subtitle pipeline failed for '{}': {e}. Saving video without subtitles; rerun with --skip-subtitles to suppress this.",
                    item.title
                );
                subtitle_failed = true;
            }
        }
    }

    Ok((final_path, subtitle_failed))
}

/// Returns `path` with its file extension replaced by `new_ext`.
fn video_path_with_extension(path: &str, new_ext: &str) -> String {
    let p = std::path::Path::new(path);
    match p.with_extension(new_ext).to_str() {
        Some(s) => s.to_string(),
        None => path.to_string(),
    }
}

/// Video file extensions produced by yt-dlp or the built-in HTTP downloader.
///
/// Used by [`check_if_media_exists`] to match any video file for a given stem
/// while ignoring subtitle (`.vtt`, `.ass`) and other non-video files.
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "webm", "ts", "m4v"];

/// Returns `true` when a non-empty video file with the same stem as `item`
/// already exists in `directory`, regardless of container extension.
///
/// This covers the case where yt-dlp chose an extension other than `.mkv`
/// (e.g. `.webm`) or where ffmpeg previously produced a `.mkv` after
/// embedding subtitles.  Stale `.tmp` files left by interrupted
/// downloads are cleaned up before the check.
#[instrument(skip_all)]
async fn check_if_media_exists(
    item: &MediaItem,
    directory: &str,
    auto_naming: bool,
) -> Result<bool> {
    // Derive the stem (e.g. "7-episode-title") to match any video extension.
    let filename = item.filename("mkv", auto_naming)?;
    let stem = std::path::Path::new(&filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::InvalidPathEncoding(filename.clone()))?;

    // With auto-naming, look inside the subdirectory; otherwise look at the
    // root directory.
    let search_dir = if let Some(sub) = item.subdirectory(auto_naming) {
        std::path::Path::new(directory).join(sub)
    } else {
        std::path::PathBuf::from(directory)
    };
    if !search_dir.exists() {
        return Ok(false);
    }

    let mut read_dir = tokio::fs::read_dir(&search_dir)
        .await
        .map_err(|e| Error::Downloading(e.to_string()))?;

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| Error::Downloading(e.to_string()))?
    {
        let entry_name = entry.file_name();
        let entry_path = std::path::Path::new(&entry_name);

        let Some(ext) = entry_path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !VIDEO_EXTENSIONS.contains(&ext) {
            continue;
        }

        let Some(entry_stem) = entry_path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if entry_stem != stem {
            continue;
        }

        let full_path = search_dir.join(&entry_name);
        let full_path_str = full_path
            .to_str()
            .ok_or_else(|| Error::InvalidPathEncoding(format!("{}", full_path.display())))?;
        if non_empty_file_exists(full_path_str) {
            return Ok(true);
        }
    }

    Ok(false)
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

/// Builds the absolute filesystem path where a media item's file should be
/// saved, taking the optional auto-naming subdirectory into account.
///
/// When `auto_naming` is `true`, a `Temporada XX/` (episodes) or
/// `Pel·lícules/` (movies) subdirectory is appended to the user-supplied
/// `directory`. The subdirectory is created on demand by the download path.
fn full_media_path(
    item: &MediaItem,
    directory: &str,
    extension: &str,
    auto_naming: bool,
) -> Result<String> {
    let filename = item.filename(extension, auto_naming)?;
    let mut path = std::path::PathBuf::from(directory);
    if let Some(sub) = item.subdirectory(auto_naming) {
        path.push(sub);
    }
    path.push(filename);
    path.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| Error::InvalidPathEncoding(format!("{}", path.display())))
}

/// Creates the auto-naming subdirectory for `item` if needed. No-op when
/// auto-naming is disabled or the subdirectory already exists.
#[instrument(skip_all, fields(media_id = item.id))]
async fn ensure_subdir(item: &MediaItem, directory: &str, auto_naming: bool) -> Result<()> {
    let Some(sub) = item.subdirectory(auto_naming) else {
        return Ok(());
    };
    let path = std::path::Path::new(directory).join(&sub);
    tokio::fs::create_dir_all(&path)
        .await
        .map_err(|e| Error::Downloading(format!("failed to create {path:?}: {e}")))?;
    Ok(())
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

    // `std::fs::rename` (not `tokio::fs::rename`) is intentional: on
    // Windows the tokio variant does not set
    // `MOVEFILE_REPLACE_EXISTING`, so it would fail when `path` already
    // exists. `std::fs::rename` uses `MoveFileExW` with the replace flag
    // on Windows and POSIX `rename(2)` (atomic replace) on Unix.
    std::fs::rename(&tmp_path, path).map_err(|e| Error::Downloading(e.to_string()))?;

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
/// Renders with a moving indeterminate bar plus elapsed time so the user
/// sees something moving instead of a single static line.
fn create_spinner(label: &str, multi_progress: &MultiProgress) -> Result<ProgressBar> {
    let pb = multi_progress.add(ProgressBar::new_spinner());
    pb.set_style(
        ProgressStyle::with_template(
            "{prefix:.bold} [{bar:30.cyan/blue}] {elapsed_precise} ({bytes}) {bytes_per_sec}",
        )
        .map_err(|e| Error::Downloading(e.to_string()))?
        .progress_chars("██░"),
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
