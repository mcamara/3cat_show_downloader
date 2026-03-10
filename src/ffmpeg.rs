//! FFmpeg detection and subtitle embedding utilities.
//!
//! Provides functions to detect whether `ffmpeg` is installed, embed
//! subtitles into video files using Matroska containers with ASS
//! subtitle tracks (preserving VTT colour styling), and batch-process
//! existing downloads in a directory.
//!
//! The pipeline is: clean VTT → convert to ASS (with inline colour
//! overrides) → mux into MKV with `-c:s ass`.  This preserves the
//! `<c.white.background-black>` styling from 3cat VTT files, which
//! ffmpeg's WebVTT encoder and MP4's `mov_text` codec both strip.

use std::path::Path;

use tokio::process::Command;
use tracing::{info, warn};

use crate::error::{Error, Result};
use crate::subtitle_cleaner;

/// Checks whether `ffmpeg` is available on the system PATH.
///
/// Runs `ffmpeg -version` and returns `true` when the process exits
/// successfully.  This is intended to be called once at startup, not
/// per-episode.
pub async fn is_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

/// Embeds a VTT subtitle file into a video via ASS conversion.
///
/// The cleaned VTT is first converted to ASS format (preserving inline
/// colour styling), then muxed into a Matroska (`.mkv`) container with
/// the video and audio streams copied without re-encoding.
///
/// On success the VTT file, the temporary ASS file, and the original
/// `.mp4` are deleted, and the path to the new `.mkv` file is returned.
/// On failure any temporary artefacts are cleaned up and an error is
/// returned.
///
/// # Errors
///
/// Returns [`Error::Ffmpeg`] when the `ffmpeg` process exits with a
/// non-zero status or fails to launch, or [`Error::SubtitleCleaning`]
/// if the VTT-to-ASS conversion fails.
pub async fn embed_subtitles(video_path: &str, subtitle_path: &str) -> Result<String> {
    let subtitle_p = Path::new(subtitle_path);
    let ass_path = subtitle_p.with_extension("ass");
    let ass_str = ass_path
        .to_str()
        .ok_or_else(|| {
            Error::Ffmpeg(format!(
                "path contains invalid UTF-8: {}",
                ass_path.display()
            ))
        })?
        .to_string();

    // Convert cleaned VTT → ASS (blocking I/O, but subtitle files are tiny)
    subtitle_cleaner::convert_vtt_file_to_ass(subtitle_p, &ass_path)?;

    let mkv_path = Path::new(video_path).with_extension("mkv");
    let mkv_str = mkv_path
        .to_str()
        .ok_or_else(|| {
            Error::Ffmpeg(format!(
                "path contains invalid UTF-8: {}",
                mkv_path.display()
            ))
        })?
        .to_string();
    let tmp_output = format!("{mkv_str}.muxed.tmp");

    let output = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            video_path,
            "-i",
            &ass_str,
            "-c",
            "copy",
            "-c:s",
            "ass",
            "-f",
            "matroska",
            &tmp_output,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| Error::Ffmpeg(format!("failed to run ffmpeg: {e}")))?;

    // Always clean up the intermediate ASS file
    let _ = std::fs::remove_file(&ass_path);

    if !output.status.success() {
        let _ = tokio::fs::remove_file(&tmp_output).await;
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Ffmpeg(format!(
            "ffmpeg exited with {}: {stderr}",
            output.status
        )));
    }

    tokio::fs::rename(&tmp_output, &mkv_str)
        .await
        .map_err(|e| Error::Ffmpeg(format!("failed to rename muxed file: {e}")))?;

    tokio::fs::remove_file(subtitle_path)
        .await
        .map_err(|e| Error::Ffmpeg(format!("failed to remove subtitle file: {e}")))?;

    tokio::fs::remove_file(video_path)
        .await
        .map_err(|e| Error::Ffmpeg(format!("failed to remove original video file: {e}")))?;

    Ok(mkv_str)
}

/// Cleans and embeds subtitles into all matching videos in a directory.
///
/// For each `.vtt` file in `directory`:
///
/// 1. The subtitle content is cleaned in-place (same as
///    [`--fix-existing-subtitles`](crate::subtitle_cleaner::fix_existing_subtitles)).
/// 2. A matching `.mp4` file (same stem) is located. If none exists a
///    warning is logged and the file is skipped.
/// 3. `ffmpeg` is invoked to embed the subtitle track into the video.
///
/// Failures for individual files are logged as warnings; processing
/// continues with the remaining files.
///
/// # Errors
///
/// Returns an error only if the directory itself cannot be read.
pub async fn embed_existing_subtitles(directory: &str) -> Result<()> {
    let dir_path = Path::new(directory);
    let entries = std::fs::read_dir(dir_path)
        .map_err(|e| Error::Ffmpeg(format!("cannot read directory: {e}")))?;

    let mut embedded_count = 0u32;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to read directory entry: {e}");
                continue;
            }
        };

        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("vtt") {
            continue;
        }

        // Clean the subtitle file first (same as --fix-existing-subtitles)
        if let Err(e) = subtitle_cleaner::clean_vtt_file(&path) {
            warn!("Failed to clean {}: {e}", path.display());
            continue;
        }

        let video_path = path.with_extension("mp4");
        if !video_path.exists() {
            warn!("No matching video found for {}, skipping", path.display(),);
            continue;
        }

        let Some(video_str) = video_path.to_str() else {
            warn!("Path contains invalid UTF-8: {}", video_path.display());
            continue;
        };
        let Some(subtitle_str) = path.to_str() else {
            warn!("Path contains invalid UTF-8: {}", path.display());
            continue;
        };

        match embed_subtitles(video_str, subtitle_str).await {
            Ok(mkv_path) => {
                info!("Embedded subtitles into {mkv_path}");
                embedded_count += 1;
            }
            Err(e) => {
                warn!(
                    "Failed to embed subtitles into {}: {e}",
                    video_path.display(),
                );
            }
        }
    }

    info!("Embedded subtitles into {embedded_count} video file(s)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_should_detect_ffmpeg_availability() {
        // This test just verifies the function runs without panicking.
        // The result depends on the host environment.
        let _available = is_available().await;
    }

    #[tokio::test]
    async fn test_should_fail_embed_with_nonexistent_files() {
        let result = embed_subtitles("/nonexistent/video.mp4", "/nonexistent/sub.vtt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_should_skip_vtt_without_matching_mp4() {
        let dir = std::env::temp_dir().join("ffmpeg_test_no_mp4");
        let _ = std::fs::create_dir_all(&dir);

        let vtt = dir.join("episode.vtt");
        std::fs::write(&vtt, "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\nHello").unwrap();

        // Should not fail — just warn and skip
        let result = embed_existing_subtitles(dir.to_str().unwrap()).await;
        assert!(result.is_ok());

        // The .vtt file should still exist (not deleted, since no matching .mp4)
        assert!(vtt.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
