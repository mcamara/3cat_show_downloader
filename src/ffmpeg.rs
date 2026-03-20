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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tokio::process::Command;
use tracing::{info, warn};

use crate::error::{Error, Result};
use crate::subtitle_cleaner;

/// A subtitle track to embed into a video file.
#[derive(Debug)]
pub struct SubtitleTrack {
    /// Path to the cleaned `.vtt` file.
    pub path: PathBuf,
    /// ISO 639-1 language code (`"ca"`, `"en"`, `"es"`, …).
    pub lang_code: String,
}

/// Maps an ISO 639-1 code to `(ISO 639-2/T code, human-readable title)`.
fn subtitle_display(lang_code: &str) -> (&'static str, &'static str) {
    match lang_code {
        "ca" => ("cat", "Català"),
        "en" => ("eng", "English"),
        "es" => ("spa", "Español"),
        _ => ("und", "Unknown"),
    }
}

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

/// Embeds one or more VTT subtitle tracks into a video via ASS conversion.
///
/// Each cleaned VTT is converted to ASS format (preserving inline colour
/// styling), then all tracks are muxed into a Matroska (`.mkv`) container
/// together with the video and audio streams (copied without re-encoding).
/// Each subtitle track receives `language` and `title` metadata derived from
/// its [`SubtitleTrack::lang_code`].
///
/// On success the VTT files, the temporary ASS files, and the original
/// video file are deleted, and the path to the new `.mkv` file is returned.
/// On failure any temporary artefacts are cleaned up and an error is
/// returned.
///
/// # Errors
///
/// Returns [`Error::Ffmpeg`] when the `ffmpeg` process exits with a
/// non-zero status or fails to launch, or [`Error::SubtitleCleaning`]
/// if a VTT-to-ASS conversion fails.
pub async fn embed_subtitles(video_path: &str, subtitles: &[SubtitleTrack]) -> Result<String> {
    // Convert each cleaned VTT → ASS.
    let mut ass_paths: Vec<PathBuf> = Vec::with_capacity(subtitles.len());
    for track in subtitles {
        let ass_path = track.path.with_extension("ass");
        subtitle_cleaner::convert_vtt_file_to_ass(&track.path, &ass_path)?;
        ass_paths.push(ass_path);
    }

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

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y").arg("-i").arg(video_path);

    for ass_path in &ass_paths {
        cmd.arg("-i").arg(ass_path);
    }

    // Map all streams from the video input, then each subtitle input.
    cmd.arg("-map").arg("0");
    for i in 1..=ass_paths.len() {
        cmd.arg("-map").arg(format!("{i}"));
    }

    cmd.arg("-c").arg("copy").arg("-c:s").arg("ass");

    for (i, track) in subtitles.iter().enumerate() {
        let (iso2, title) = subtitle_display(&track.lang_code);
        cmd.arg(format!("-metadata:s:s:{i}"))
            .arg(format!("language={iso2}"))
            .arg(format!("-metadata:s:s:{i}"))
            .arg(format!("title={title}"));
    }

    cmd.arg("-f").arg("matroska").arg(&tmp_output);

    let output = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| Error::Ffmpeg(format!("failed to run ffmpeg: {e}")))?;

    // Always clean up the intermediate ASS files.
    for ass_path in &ass_paths {
        let _ = std::fs::remove_file(ass_path);
    }

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

    for track in subtitles {
        let _ = tokio::fs::remove_file(&track.path).await;
    }

    tokio::fs::remove_file(video_path)
        .await
        .map_err(|e| Error::Ffmpeg(format!("failed to remove original video file: {e}")))?;

    Ok(mkv_str)
}

/// Cleans and embeds subtitles into all matching videos in a directory.
///
/// Scans `directory` for `.vtt` files. Files named `{stem}.{lang}.vtt`
/// (where `lang` is one of `ca`, `en`, `es`) are grouped by `{stem}` so
/// that all available language tracks for a given episode are embedded in a
/// single ffmpeg pass. Plain `{stem}.vtt` files are treated as Catalan.
///
/// For each group a matching `{stem}.mp4` must exist.  If none is found a
/// warning is logged and the group is skipped.  All subtitle files in a
/// group are cleaned before embedding.
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

    // Group VTT files by video stem: stem → [(path, lang_code)]
    let mut groups: HashMap<String, Vec<(PathBuf, String)>> = HashMap::new();
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to read directory entry: {e}");
                continue;
            }
        };
        let path = entry.path();
        if let Some((stem, lang)) = parse_vtt_stem_and_lang(&path) {
            groups.entry(stem).or_default().push((path, lang));
        }
    }

    let mut embedded_count = 0u32;

    for (stem, vtt_files) in groups {
        let video_path = dir_path.join(format!("{stem}.mp4"));
        if !video_path.exists() {
            warn!(
                "No matching video found for {}, skipping",
                video_path.display()
            );
            continue;
        }

        let mut tracks: Vec<SubtitleTrack> = Vec::new();
        let mut failed = false;

        for (vtt_path, lang_code) in vtt_files {
            if let Err(e) = subtitle_cleaner::clean_vtt_file(&vtt_path) {
                warn!("Failed to clean {}: {e}", vtt_path.display());
                failed = true;
                break;
            }
            tracks.push(SubtitleTrack {
                path: vtt_path,
                lang_code,
            });
        }

        if failed || tracks.is_empty() {
            continue;
        }

        let Some(video_str) = video_path.to_str() else {
            warn!("Path contains invalid UTF-8: {}", video_path.display());
            continue;
        };

        match embed_subtitles(video_str, &tracks).await {
            Ok(mkv_path) => {
                info!("Embedded subtitles into {mkv_path}");
                embedded_count += 1;
            }
            Err(e) => {
                warn!(
                    "Failed to embed subtitles into {}: {e}",
                    video_path.display()
                );
            }
        }
    }

    info!("Embedded subtitles into {embedded_count} video file(s)");
    Ok(())
}

/// Parses a VTT file path into `(video_stem, lang_code)`.
///
/// Handles both `{stem}.{lang}.vtt` (language-tagged, e.g. `1-ep.ca.vtt`)
/// and `{stem}.vtt` (untagged, assumed Catalan).  Returns `None` for
/// non-`.vtt` paths.
fn parse_vtt_stem_and_lang(path: &Path) -> Option<(String, String)> {
    if path.extension()?.to_str()? != "vtt" {
        return None;
    }
    // Strip the .vtt extension: /foo/1-ep.ca.vtt → /foo/1-ep.ca
    let without_vtt = path.with_extension("");
    let lang_candidate = without_vtt
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    if matches!(lang_candidate, "ca" | "en" | "es") {
        // {stem}.{lang}.vtt — extract stem and use the language code.
        let stem = without_vtt.file_stem()?.to_str()?.to_string();
        Some((stem, lang_candidate.to_string()))
    } else {
        // Plain {stem}.vtt — assume Catalan.
        let stem = without_vtt.file_stem()?.to_str()?.to_string();
        Some((stem, "ca".to_string()))
    }
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
        let track = SubtitleTrack {
            path: PathBuf::from("/nonexistent/sub.vtt"),
            lang_code: "ca".to_string(),
        };
        let result = embed_subtitles("/nonexistent/video.mp4", &[track]).await;
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

    #[test]
    fn test_should_parse_language_tagged_vtt() {
        let path = Path::new("/foo/1-episode.ca.vtt");
        let (stem, lang) = parse_vtt_stem_and_lang(path).unwrap();
        assert_eq!(stem, "1-episode");
        assert_eq!(lang, "ca");
    }

    #[test]
    fn test_should_parse_untagged_vtt_as_catalan() {
        let path = Path::new("/foo/1-episode.vtt");
        let (stem, lang) = parse_vtt_stem_and_lang(path).unwrap();
        assert_eq!(stem, "1-episode");
        assert_eq!(lang, "ca");
    }

    #[test]
    fn test_should_parse_english_vtt() {
        let path = Path::new("/foo/1-episode.en.vtt");
        let (stem, lang) = parse_vtt_stem_and_lang(path).unwrap();
        assert_eq!(stem, "1-episode");
        assert_eq!(lang, "en");
    }

    #[test]
    fn test_should_return_none_for_non_vtt() {
        assert!(parse_vtt_stem_and_lang(Path::new("/foo/video.mp4")).is_none());
        assert!(parse_vtt_stem_and_lang(Path::new("/foo/video.ass")).is_none());
    }
}
