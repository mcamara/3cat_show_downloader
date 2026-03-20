//! yt-dlp detection and video downloading.
//!
//! When yt-dlp is available on PATH it is used as the download backend for
//! CCMA/3cat content. The CCMA extractor built into yt-dlp handles format
//! selection and subtitle extraction natively — no prior API call is needed.
//!
//! The video URL is constructed as `https://www.3cat.cat/3cat/x/video/{id}`.
//! The CCMA extractor only needs the numeric ID; the slug segment (`x`) is
//! arbitrary as long as it matches `[^/?#]+`.

use std::path::PathBuf;
use std::process::Stdio;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tracing::{info, instrument};

use crate::error::{Error, Result};
use crate::ffmpeg;
use crate::models::{MediaItem, SubtitleMode};
use crate::subtitle_cleaner;

const CCMA_VIDEO_URL_BASE: &str = "https://www.3cat.cat/3cat/x/video/";

/// Checks whether `yt-dlp` is available on the system PATH.
///
/// Runs `yt-dlp --version` and returns `true` when the process exits
/// successfully. Intended to be called once at startup.
pub async fn is_available() -> bool {
    Command::new("yt-dlp")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

/// Downloads a media item using yt-dlp.
///
/// Constructs a 3cat video URL from `item.id` and invokes yt-dlp with
/// `--progress --newline` so that each progress update is emitted on its own
/// stdout line. Those lines are parsed in real time to drive an
/// [`indicatif`] progress bar rendered through `multi_progress`.
///
/// Subtitles are handled according to `subtitle_mode`:
///
/// - [`SubtitleMode::Skip`]: no subtitle arguments are passed.
/// - [`SubtitleMode::Download`]: `--write-subs --sub-langs ca` downloads the
///   subtitle alongside the video and cleans it with [`subtitle_cleaner`].
/// - [`SubtitleMode::Embed`]: same as Download, but additionally runs the
///   cleaned subtitle through [`ffmpeg::embed_subtitles`] to produce an MKV
///   with an ASS track. Using `--embed-subs` is intentionally avoided because
///   it bypasses the VTT cleaning step, leaving the CSS colour classes from
///   CCMA subtitle files un-converted and producing empty/broken tracks.
///
/// # Errors
///
/// Returns [`Error::YtDlp`] if yt-dlp cannot be launched or exits with a
/// non-zero status.
#[instrument(skip_all, fields(media_id = item.id))]
pub async fn download(
    item: &MediaItem,
    directory: &str,
    subtitle_mode: SubtitleMode,
    multi_progress: &MultiProgress,
) -> Result<()> {
    let url = format!("{}{}", CCMA_VIDEO_URL_BASE, item.id);
    let output_filename = item.filename("%(ext)s")?;
    let output_template = std::path::Path::new(directory)
        .join(&output_filename)
        .to_str()
        .ok_or_else(|| Error::InvalidPathEncoding(output_filename.clone()))?
        .to_string();

    info!("Downloading with yt-dlp: {url}");

    let mut cmd = Command::new("yt-dlp");
    cmd.args([
        "--no-playlist",
        "--progress",
        "--newline",
        "-o",
        &output_template,
    ]);

    match subtitle_mode {
        SubtitleMode::Skip => {}
        SubtitleMode::Download | SubtitleMode::Embed => {
            // Always use --write-subs so the VTT lands on disk and can be
            // processed by our cleaning + embedding pipeline.  --embed-subs
            // is intentionally not used: it skips our cleaner and produces
            // broken subtitle tracks for CCMA content.
            cmd.args(["--write-subs", "--sub-langs", "ca,en,es"]);
        }
    }

    cmd.arg(&url);

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::YtDlp(format!("failed to run yt-dlp: {e}")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::YtDlp("failed to capture stdout from yt-dlp".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::YtDlp("failed to capture stderr from yt-dlp".to_string()))?;

    let pb = create_progress_bar(&output_filename, multi_progress)?;
    let pb_clone = pb.clone();

    let progress_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some((pos, msg)) = parse_download_progress(&line) {
                pb_clone.set_position(pos);
                pb_clone.set_message(msg);
            }
        }
    });

    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut buf).await;
        buf
    });

    let status = child
        .wait()
        .await
        .map_err(|e| Error::YtDlp(format!("failed to wait for yt-dlp: {e}")))?;

    let _ = progress_task.await;
    pb.finish_and_clear();

    if !status.success() {
        let stderr_output = stderr_task.await.unwrap_or_default();
        return Err(Error::YtDlp(format!(
            "yt-dlp exited with {}: {stderr_output}",
            status
        )));
    }

    if subtitle_mode == SubtitleMode::Skip {
        return Ok(());
    }

    // Collect subtitle files written by yt-dlp: {stem}.{lang}.vtt
    let subtitle_langs = ["ca", "en", "es"];
    let mut found: Vec<(PathBuf, &str)> = Vec::new();
    for &lang in &subtitle_langs {
        let vtt_path = std::path::Path::new(directory).join(item.filename(&format!("{lang}.vtt"))?);
        if vtt_path.exists() {
            found.push((vtt_path, lang));
        }
    }

    if found.is_empty() {
        return Err(Error::NoSubtitlesAvailable(item.title.clone()));
    }

    for (vtt_path, _) in &found {
        subtitle_cleaner::clean_vtt_file(vtt_path)?;
    }

    if subtitle_mode == SubtitleMode::Embed {
        let tracks: Vec<ffmpeg::SubtitleTrack> = found
            .into_iter()
            .map(|(path, lang)| ffmpeg::SubtitleTrack {
                path,
                lang_code: lang.to_string(),
            })
            .collect();

        let video_filename = item.filename("mp4")?;
        let video_path = std::path::Path::new(directory).join(&video_filename);
        let video_str = video_path
            .to_str()
            .ok_or_else(|| Error::InvalidPathEncoding(video_filename.clone()))?;

        match ffmpeg::embed_subtitles(video_str, &tracks).await {
            Ok(mkv_path) => info!("Subtitles embedded into video {mkv_path}"),
            Err(e) => {
                tracing::warn!("Failed to embed subtitles into {video_str}: {e}");
            }
        }
    }

    Ok(())
}

/// Parses a yt-dlp `--progress --newline` stdout line into a progress position
/// (0–1000, in tenths of a percent) and a display message.
///
/// Returns `None` for lines that are not download-progress lines.
fn parse_download_progress(line: &str) -> Option<(u64, String)> {
    // yt-dlp emits lines like:
    //   [download]  17.3% of    2.37GiB at    3.15MiB/s ETA 09:49
    //   [download] 100% of    2.37GiB in 00:13 at    3.15MiB/s
    let content = line.strip_prefix("[download]")?.trim();
    let pct_idx = content.find('%')?;
    let pct: f64 = content[..pct_idx].trim().parse().ok()?;
    let after = content[pct_idx + 1..].trim().to_string();
    Some(((pct * 10.0) as u64, format!("{pct:.1}% {after}")))
}

/// Creates a styled progress bar for a yt-dlp download.
fn create_progress_bar(label: &str, multi_progress: &MultiProgress) -> Result<ProgressBar> {
    let pb = multi_progress.add(ProgressBar::new(1000));
    pb.set_style(
        ProgressStyle::with_template("{prefix:.bold} [{bar:30.cyan/blue}] {msg}")
            .map_err(|e| Error::Downloading(e.to_string()))?
            .progress_chars("█░░"),
    );
    pb.set_prefix(label.to_string());
    Ok(pb)
}

#[cfg(test)]
mod tests {
    use indicatif::MultiProgress;

    use super::*;

    #[test]
    fn test_should_parse_download_progress_line() {
        let line = "[download]  17.3% of    2.37GiB at    3.15MiB/s ETA 09:49";
        let result = parse_download_progress(line);
        assert!(result.is_some());
        let (pos, msg) = result.unwrap();
        assert_eq!(pos, 173);
        assert!(msg.starts_with("17.3%"));
    }

    #[test]
    fn test_should_parse_complete_progress_line() {
        let line = "[download] 100% of    2.37GiB in 00:13 at    3.15MiB/s";
        let (pos, _) = parse_download_progress(line).unwrap();
        assert_eq!(pos, 1000);
    }

    #[test]
    fn test_should_return_none_for_non_progress_line() {
        assert!(parse_download_progress("[download] Destination: file.mp4").is_none());
        assert!(parse_download_progress("[info] Some info line").is_none());
        assert!(parse_download_progress("").is_none());
    }

    /// Smoke test: verifies that `is_available` returns without panicking.
    /// Ignored by default because yt-dlp may not be installed in all environments.
    /// Run with `cargo test -- --ignored` when yt-dlp is present on PATH.
    #[tokio::test]
    #[ignore]
    async fn test_should_detect_yt_dlp_availability() {
        assert!(is_available().await);
    }

    #[tokio::test]
    async fn test_should_fail_download_with_invalid_id() {
        let item = MediaItem {
            id: 1,
            title: "Test episode".to_string(),
            video_url: None,
            subtitle_url: None,
            episode_number: Some(1),
            tv_show_name: Some("Test show".to_string()),
        };
        let mp = MultiProgress::new();
        let result = download(&item, "/tmp", SubtitleMode::Skip, &mp).await;
        assert!(result.is_err());
    }
}
