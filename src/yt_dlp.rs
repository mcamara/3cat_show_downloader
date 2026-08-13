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
use tracing::{debug, info, instrument, warn};

use crate::error::{Error, Result};
use crate::ffmpeg;
use crate::models::{MediaItem, SubtitleMode};
use crate::subtitle_cleaner;
use crate::transcode;

const CCMA_VIDEO_URL_BASE: &str = "https://www.3cat.cat/3cat/x/video/";

/// Maximum number of retry attempts for a transient yt-dlp failure (HTTP
/// 503 / 429 / network errors). Used by [`spawn_ytdlp_with_retry`].
const YT_DLP_MAX_RETRIES: u32 = 5;

/// Initial backoff between yt-dlp retry attempts. Doubled on each
/// consecutive failure (capped by the tokio sleep so we never sleep more
/// than 30 s at a time).
const YT_DLP_INITIAL_BACKOFF_MS: u64 = 2_000;
const YT_DLP_MAX_BACKOFF_MS: u64 = 30_000;

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
#[allow(clippy::too_many_arguments)]
pub async fn download(
    item: &mut MediaItem,
    directory: &str,
    subtitle_mode: SubtitleMode,
    multi_progress: &MultiProgress,
    strict_subtitles: bool,
    auto_naming: bool,
    reencode_preset: transcode::ReencodePreset,
    capabilities: &ffmpeg::EncoderCapabilities,
    request_delay_ms: u64,
) -> Result<bool> {
    // Resolve the search directory so we can skip the network round-trip
    // when the video file already exists locally (e.g. when re-running
    // the tool to apply --reencode or to refresh subtitles).
    let mut search_dir = std::path::PathBuf::from(directory);
    if let Some(sub) = item.subdirectory(auto_naming) {
        search_dir.push(sub);
    }
    let canonical_prefix = canonical_auto_naming_prefix(item, auto_naming)?;
    let existing_video = find_video_by_prefix(&search_dir, &canonical_prefix);

    if let Some(video_path) = existing_video {
        debug!(
            "Skipping yt-dlp download: {} already exists, applying post-download pipeline only.",
            video_path.display()
        );
        return run_post_download_pipeline(
            item,
            directory,
            subtitle_mode,
            multi_progress,
            strict_subtitles,
            auto_naming,
            reencode_preset,
            capabilities,
            &video_path,
            &search_dir,
        )
        .await;
    }

    let url = format!("{}{}", CCMA_VIDEO_URL_BASE, item.id);
    let output_filename = item.filename("%(ext)s", auto_naming)?;
    let mut base_path = std::path::PathBuf::from(directory);
    if let Some(sub) = item.subdirectory(auto_naming) {
        base_path.push(sub);
        // Create the subdirectory eagerly so yt-dlp can write into it.
        tokio::fs::create_dir_all(&base_path)
            .await
            .map_err(|e| Error::YtDlp(format!("failed to create {base_path:?}: {e}")))?;
    }
    let output_template = base_path
        .join(&output_filename)
        .to_str()
        .ok_or_else(|| Error::InvalidPathEncoding(output_filename.clone()))?
        .to_string();

    // Build the search directory up front so we can both probe for
    // already-downloaded VTTs and write new ones into the right place.
    let mut search_dir = std::path::PathBuf::from(directory);
    if let Some(sub) = item.subdirectory(auto_naming) {
        search_dir.push(sub);
    }

    // Pre-launch delay: spreading requests out is the single most effective
    // way to avoid `HTTP 503 backend read error` from 3cat. The default
    // 1500 ms is applied between consecutive yt-dlp invocations across the
    // scheduler queue (one delay per episode, not per chunk).
    if request_delay_ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(request_delay_ms)).await;
    }

    // Two-pass strategy when subtitles are wanted: download the subtitle
    // tracks FIRST (without the video) so we know quickly whether 3cat has
    // them available, then download the video in a separate yt-dlp call.
    // This avoids burning bandwidth on a 200 MB video only to discover the
    // subtitles are missing, and isolates a 503 on either pass so we can
    // retry each one independently. `--embed-subs` is intentionally NOT
    // used even in this split mode: it bypasses our VTT cleaner and
    // produces broken ASS tracks for CCMA content.
    if subtitle_mode != SubtitleMode::Skip {
        info!("Fetching subtitles for {} with yt-dlp", item.title);
        run_ytdlp_with_retry(
            &url,
            &output_template,
            &[
                "--skip-download",
                "--write-subs",
                "--sub-langs",
                "ca,en,es",
                "--convert-subs",
                "vtt",
            ],
            multi_progress,
            &format!("subs {output_filename}"),
        )
        .await?;
    }

    // Collect subtitle files written by yt-dlp: {stem}.{lang}.vtt
    let subtitle_langs = ["ca", "en", "es"];
    let mut found: Vec<(PathBuf, &str)> = Vec::new();
    for &lang in &subtitle_langs {
        let vtt_path = search_dir.join(item.filename(&format!("{lang}.vtt"), auto_naming)?);
        if vtt_path.exists() {
            found.push((vtt_path, lang));
        }
    }

    let subtitles_present = !found.is_empty();
    if subtitle_mode != SubtitleMode::Skip && !subtitles_present {
        if strict_subtitles {
            return Err(Error::NoSubtitlesAvailable(item.title.clone()));
        }
        warn!(
            "No subtitles available for '{}'. Video will still be downloaded without subs; rerun with --strict-subtitles to abort on missing subs.",
            item.title
        );
        item.subtitle_failed = true;
    }

    // Second yt-dlp pass: download the video only. The subs were already
    // fetched in the first pass; if we did not pass `--write-subs` here,
    // yt-dlp will not download them again — and it has no `--no-subs`
    // flag, only `--no-write-subs`. We simply omit `--write-subs` from
    // the video pass to keep the second request focused on the media.
    info!("Downloading video for {} with yt-dlp", item.title);
    run_ytdlp_with_retry(
        &url,
        &output_template,
        &[],
        multi_progress,
        &output_filename,
    )
    .await?;

    if subtitle_mode == SubtitleMode::Skip {
        return Ok(false);
    }

    if !subtitles_present {
        // Re-encode + remux still need to run on the freshly downloaded
        // video. The early-return path used to live here, but it skipped
        // the re-encode entirely, which meant `--reencode max` was silently
        // ignored whenever the episode had no subtitles. Now we fall
        // through to the same re-encode + remux block as the happy path.
    }

    let mut cleaning_failed = false;
    for (vtt_path, _) in &found {
        if let Err(e) = subtitle_cleaner::clean_vtt_file(vtt_path) {
            if strict_subtitles {
                return Err(e);
            }
            warn!(
                "Subtitle cleaning failed for '{}' ({}): {e}. Saving video without cleaned subs.",
                item.title,
                vtt_path.display()
            );
            item.subtitle_failed = true;
            cleaning_failed = true;
        }
    }
    if cleaning_failed {
        return Ok(true);
    }

    if subtitle_mode == SubtitleMode::Embed && !found.is_empty() {
        let tracks: Vec<ffmpeg::SubtitleTrack> = found
            .into_iter()
            .map(|(path, lang)| ffmpeg::SubtitleTrack {
                path,
                lang_code: lang.to_string(),
            })
            .collect();

        // Locate the video file yt-dlp just wrote. We can't assume `.mp4`:
        // yt-dlp may pick `.webm`/`.mkv` depending on what 3cat serves.
        // The embed path needs the actual file on disk, not the synthetic
        // auto-naming filename (which has no extension in this layout —
        // it's the resolution tag, not the container extension).
        let canonical_prefix = canonical_auto_naming_prefix(item, auto_naming)?;
        match find_video_by_prefix(&search_dir, &canonical_prefix) {
            Some(video_path) => {
                let video_str = video_path
                    .to_str()
                    .ok_or_else(|| Error::InvalidPathEncoding(video_path.display().to_string()))?;
                match ffmpeg::embed_subtitles(video_str, &tracks).await {
                    Ok(mkv_path) => debug!("Subtitles embedded into video {mkv_path}"),
                    Err(e) => {
                        tracing::warn!("Failed to embed subtitles into {video_str}: {e}");
                    }
                }
            }
            None => {
                warn!(
                    "Could not locate video file under '{canonical_prefix}' to embed subtitles into. Skipping embed step."
                );
            }
        }
    }

    // Re-encode the video. find_video_by_prefix may now match a fresh .mkv
    // (if embed succeeded), or the original .mp4 otherwise. We always
    // attempt the re-encode even when `subtitle_mode == Embed` returned
    // early with no subtitles to embed — the video file is still on disk
    // and the user's `--reencode` preset should still apply.
    let canonical_prefix = canonical_auto_naming_prefix(item, auto_naming)?;
    match find_video_by_prefix(&search_dir, &canonical_prefix) {
        Some(final_video) => {
            if !reencode_preset.is_off() {
                if let Err(e) =
                    transcode::reencode(&final_video, reencode_preset, capabilities, multi_progress)
                        .await
                {
                    warn!(
                        "Re-encoding failed for {}: {e}. Original file preserved.",
                        final_video.display()
                    );
                }
            }
            // Final-stage guarantee: every downloaded file ends up as `.mkv`.
            // Re-encode already writes MKV; embed already produces MKV; this
            // remux covers the "no-embed, no-reencode" case. Idempotent.
            if let Err(e) = ffmpeg::remux_to_mkv(&final_video).await {
                warn!(
                    "Remux to MKV failed for {}: {e}. Original file preserved.",
                    final_video.display()
                );
            }
        }
        None => {
            warn!(
                "Could not locate yt-dlp output matching prefix '{canonical_prefix}' to re-encode. Skipping re-encode step."
            );
        }
    }

    Ok(item.subtitle_failed)
}

/// Extends the standard video prefix match to also accept `fdash-*.mp4.part`
/// fragments that yt-dlp leaves behind while a download is still in flight.
///
/// During a 339-episode batch yt-dlp's `*(ext)s` template can resolve to
/// `fdash-<uuid>.mp4` while the DASH segments are still being merged. The
/// final merged file is `.mp4` so the cleanup is implicit: once the parent
/// download finishes, the part files are gone and the regex matches the
/// real video. This function is best-effort and only used by the re-encode
/// fallback path.
#[allow(dead_code)]
fn find_video_or_part_fragment(dir: &std::path::Path, prefix: &str) -> Option<PathBuf> {
    find_video_by_prefix(dir, prefix).or_else(|| {
        let entries = std::fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if stem.starts_with(prefix)
                && stem.contains(".fdash-")
                && path.extension().and_then(|e| e.to_str()) == Some("mp4")
            {
                return Some(path);
            }
        }
        None
    })
}

/// Builds the canonical stem prefix used to locate the video file yt-dlp wrote
/// to disk. Mirrors the layout produced by
/// [`MediaItem::filename`](crate::models::MediaItem::filename) with
/// `auto_naming = true`, minus the trailing resolution tag — i.e.
/// `Mic - <slug> - S##E##` (without ` (1080p)`).
///
/// For the legacy (non-auto-naming) layout, returns the bare filename stem.
fn canonical_auto_naming_prefix(item: &MediaItem, auto_naming: bool) -> Result<String> {
    if !auto_naming {
        return Ok(item
            .filename("x", auto_naming)?
            .trim_end_matches(".x")
            .to_string());
    }

    // Reuse `filename` to derive the slug and tag the resolution tag onto it
    // for trimming. Cheap, lazy, and resilient to any future layout tweak.
    let with_probe = item.filename("PROBE_TAG_NOT_IN_USE", auto_naming)?;
    // Auto-naming layout always ends with ` (1080p)`; strip it for the prefix.
    let trimmed = with_probe.trim_end_matches(" (1080p)");
    debug_assert!(trimmed.len() < with_probe.len(), "auto-naming prefix trim");
    Ok(trimmed.to_string())
}

/// Runs only the post-download pipeline (cleaning, embedding, re-encoding)
/// for an episode whose video file already exists on disk. Mirrors the
/// steps `yt_dlp::download` performs after the yt-dlp process exits, but
/// skips the network round-trip.
///
/// This lets users re-run the tool with a different `--reencode` preset
/// (or to refresh subtitles) without re-downloading gigabytes of video.
#[allow(clippy::too_many_arguments)]
async fn run_post_download_pipeline(
    item: &mut MediaItem,
    _directory: &str,
    subtitle_mode: SubtitleMode,
    multi_progress: &MultiProgress,
    strict_subtitles: bool,
    auto_naming: bool,
    reencode_preset: transcode::ReencodePreset,
    capabilities: &ffmpeg::EncoderCapabilities,
    video_path: &std::path::Path,
    search_dir: &std::path::Path,
) -> Result<bool> {
    let mut subtitle_failed = false;

    if subtitle_mode != SubtitleMode::Skip {
        let subtitle_langs = ["ca", "en", "es"];
        let mut found: Vec<(PathBuf, &str)> = Vec::new();
        for &lang in &subtitle_langs {
            // Probe the VTT path under both the legacy (`(mp4).ca.vtt`) and
            // the current (`(1080p).ca.vtt`) naming conventions so files
            // produced by older binaries can still be picked up.
            for tag in ["1080p", "mp4"] {
                let probe = item.filename(&format!("({tag}).{lang}.vtt"), auto_naming)?;
                let vtt_path = search_dir.join(probe);
                if vtt_path.exists() {
                    found.push((vtt_path, lang));
                    break;
                }
            }
        }

        let mut cleaning_failed = false;
        for (vtt_path, _) in &found {
            if let Err(e) = subtitle_cleaner::clean_vtt_file(vtt_path) {
                if strict_subtitles {
                    return Err(e);
                }
                warn!(
                    "Subtitle cleaning failed for '{}' ({}): {e}.",
                    item.title,
                    vtt_path.display()
                );
                item.subtitle_failed = true;
                cleaning_failed = true;
            }
        }
        if cleaning_failed {
            subtitle_failed = true;
        } else if subtitle_mode == SubtitleMode::Embed && !found.is_empty() {
            let tracks: Vec<ffmpeg::SubtitleTrack> = found
                .into_iter()
                .map(|(path, lang)| ffmpeg::SubtitleTrack {
                    path,
                    lang_code: lang.to_string(),
                })
                .collect();
            let video_str = video_path
                .to_str()
                .ok_or_else(|| Error::InvalidPathEncoding(video_path.display().to_string()))?;
            match ffmpeg::embed_subtitles(video_str, &tracks).await {
                Ok(mkv_path) => {
                    debug!("Subtitles embedded into video {mkv_path}");
                }
                Err(e) => {
                    warn!("Failed to embed subtitles into {video_str}: {e}");
                }
            }
        }
    }

    // Re-encode the video. find_video_by_prefix may now match a fresh .mkv
    // (if embed succeeded), or the original .mp4 otherwise.
    if !reencode_preset.is_off() {
        let canonical_prefix = canonical_auto_naming_prefix(item, auto_naming)?;
        if let Some(latest) = find_video_by_prefix(search_dir, &canonical_prefix) {
            if let Err(e) =
                transcode::reencode(&latest, reencode_preset, capabilities, multi_progress).await
            {
                warn!(
                    "Re-encoding failed for {}: {e}. Original file preserved.",
                    latest.display()
                );
            }
        }
    }

    // Always end up with a `.mkv` file regardless of what the previous
    // stages produced.
    if let Err(e) = ffmpeg::remux_to_mkv(video_path).await {
        warn!(
            "Remux to MKV failed for {}: {e}. Original file preserved.",
            video_path.display()
        );
    }

    Ok(subtitle_failed || item.subtitle_failed)
}

/// Scans `dir` for a video file whose stem starts with `prefix`.
/// Returns the first match as a [`PathBuf`]. Returns `None` when no
/// candidate is found.
fn find_video_by_prefix(dir: &std::path::Path, prefix: &str) -> Option<PathBuf> {
    const VIDEO_EXTS: &[&str] = &["mp4", "mkv", "webm", "ts", "m4v"];
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !VIDEO_EXTS.contains(&ext) {
            continue;
        }
        let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if file_stem.starts_with(prefix) {
            return Some(path);
        }
    }
    None
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
///
/// The bar is configured with a synthetic 0..1000 scale so the visual
/// percentage is driven by indicat's `percent` field. The remaining time
/// estimate comes from yt-dlp's own message stream (already captured into
/// `msg` by `parse_download_progress`).
fn create_progress_bar(label: &str, multi_progress: &MultiProgress) -> Result<ProgressBar> {
    let pb = multi_progress.add(ProgressBar::new(1000));
    pb.set_style(
        ProgressStyle::with_template("{prefix:.bold} [{bar:30.cyan/blue}] {percent}% {msg}")
            .map_err(|e| Error::Downloading(e.to_string()))?
            .progress_chars("██░"),
    );
    pb.set_prefix(label.to_string());
    Ok(pb)
}

/// Returns `true` when the stderr output indicates a transient failure
/// that retries can recover (HTTP 503 / 429 / connection reset).
///
/// Examples of triggers in 3cat output:
/// - `HTTP Error 503: backend read error`
/// - `HTTP Error 429: Too Many Requests`
/// - `ConnectionResetError`
/// - `Temporary failure in name resolution`
fn is_transient_ytdlp_failure(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    [
        "http error 503",
        "http error 429",
        "connection reset",
        "temporary failure",
        "timed out",
        "backend read error",
        "server returned 5",
        "ssl: unexpected eof",
        "ssl: handshake",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// Runs `yt-dlp` with the given arguments, retrying transient failures
/// with exponential backoff. The progress bar is recreated on every
/// attempt so the user sees the retry as a fresh bar.
async fn run_ytdlp_with_retry(
    url: &str,
    output_template: &str,
    subtitle_args: &[&str],
    multi_progress: &MultiProgress,
    label: &str,
) -> Result<()> {
    let mut attempt: u32 = 0;
    let mut backoff_ms = YT_DLP_INITIAL_BACKOFF_MS;

    loop {
        attempt += 1;

        let mut cmd = Command::new("yt-dlp");
        cmd.args([
            "--no-playlist",
            "--progress",
            "--newline",
            "-o",
            output_template,
        ])
        .args(subtitle_args)
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

        let mut child = cmd
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

        let pb = create_progress_bar(label, multi_progress)?;
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

        if status.success() {
            return Ok(());
        }

        let stderr_output = stderr_task.await.unwrap_or_default();

        if attempt >= YT_DLP_MAX_RETRIES || !is_transient_ytdlp_failure(&stderr_output) {
            return Err(Error::YtDlp(format!(
                "yt-dlp exited with {status}: {stderr_output}"
            )));
        }

        warn!(
            "yt-dlp attempt {attempt}/{max} failed with transient error: {stderr}. Retrying in {backoff_ms} ms.",
            max = YT_DLP_MAX_RETRIES,
            stderr = stderr_output.lines().next().unwrap_or("").trim(),
        );

        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms * 2).min(YT_DLP_MAX_BACKOFF_MS);
    }
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
        let mut item = MediaItem {
            id: 1,
            title: "Test episode".to_string(),
            video_url: None,
            subtitle_url: None,
            episode_number: Some(1),
            tv_show_name: Some("Test show".to_string()),
            season: None,
            subtitle_failed: false,
            plex: None,
        };
        let mp = MultiProgress::new();
        let result = download(
            &mut item,
            "/tmp",
            SubtitleMode::Skip,
            &mp,
            false,
            false,
            transcode::ReencodePreset::Off,
            &crate::ffmpeg::EncoderCapabilities::default(),
            0,
        )
        .await;
        assert!(result.is_err());
    }
}
