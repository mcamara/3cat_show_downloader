//! FFmpeg detection, encoder probing, container remux, and subtitle embedding.
//!
//! Provides functions to detect whether `ffmpeg` is installed, probe which
//! video encoders it has compiled in (so the re-encode stage can prefer
//! NVIDIA NVENC over software `libx264`/`libx265`), embed subtitles into
//! video files using Matroska containers with ASS subtitle tracks
//! (preserving VTT colour styling), batch-process existing downloads in a
//! directory, and losslessly remux any container into `.mkv` so every
//! download ends up in the same container regardless of what yt-dlp chose.
//!
//! The pipeline is: clean VTT → convert to ASS (with inline colour
//! overrides) → mux into MKV with `-c:s ass`.  This preserves the
//! `<c.white.background-black>` styling from 3cat VTT files, which
//! ffmpeg's WebVTT encoder and MP4's `mov_text` codec both strip.

use std::collections::{BTreeSet, HashMap};
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

// ---------------------------------------------------------------------------
// Encoder capability probe + container remux
// ---------------------------------------------------------------------------

/// Video encoder backend family.
///
/// Ordered by preference: `Nvenc` > `Amf` > `Qsv` > `LibSw`. Used by
/// [`EncoderCapabilities`] to pick the best available backend for a given
/// codec target so the re-encoder can use the GPU when present and fall back
/// transparently to software (`libx264` / `libx265`) when not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VcodecBackend {
    /// NVIDIA NVENC (`h264_nvenc`, `hevc_nvenc`).
    Nvenc,
    /// AMD AMF (`h264_amf`, `hevc_amf`).
    Amf,
    /// Intel Quick Sync Video (`h264_qsv`, `hevc_qsv`).
    Qsv,
    /// Software (`libx264`, `libx265`).
    LibSw,
}

/// Target codec family for an encoder pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcodecTarget {
    /// H.264 / AVC.
    H264,
    /// H.265 / HEVC.
    H265,
}

/// Snapshot of which video encoders are compiled into the host `ffmpeg`.
///
/// Populated by [`probe_encoders`] from `ffmpeg -encoders`. Both
/// [`Self::h264`] and [`Self::h265`] are sorted by backend preference so the
/// first element is always the best one available.
#[derive(Debug, Clone, Default)]
pub struct EncoderCapabilities {
    h264: Vec<VcodecBackend>,
    h265: Vec<VcodecBackend>,
}

impl EncoderCapabilities {
    /// Returns the available H.264 backends in preference order.
    pub fn h264(&self) -> &[VcodecBackend] {
        &self.h264
    }

    /// Returns the available H.265 backends in preference order.
    pub fn h265(&self) -> &[VcodecBackend] {
        &self.h265
    }

    /// Returns `true` when neither codec family has any registered backend.
    pub fn is_empty(&self) -> bool {
        self.h264.is_empty() && self.h265.is_empty()
    }

    /// Returns the preferred H.264 backend, falling back to software.
    pub fn preferred_h264(&self) -> VcodecBackend {
        self.h264.first().copied().unwrap_or(VcodecBackend::LibSw)
    }

    /// Returns the preferred H.265 backend, falling back to software.
    pub fn preferred_h265(&self) -> VcodecBackend {
        self.h265.first().copied().unwrap_or(VcodecBackend::LibSw)
    }

    /// Parses the textual output of `ffmpeg -encoders` into a capabilities
    /// snapshot. Each encoder appears on its own line as
    /// `<flags> <name> <description>`; we tokenise by whitespace and
    /// inspect the second token.
    pub fn parse(encoders_output: &str) -> Self {
        let mut h264: Vec<VcodecBackend> = Vec::new();
        let mut h265: Vec<VcodecBackend> = Vec::new();
        let mut seen_h264: BTreeSet<&str> = BTreeSet::new();
        let mut seen_h265: BTreeSet<&str> = BTreeSet::new();

        for line in encoders_output.lines() {
            let mut tokens = line.split_whitespace();
            let Some(name) = tokens.nth(1) else {
                continue;
            };
            let Some((backend, codec)) = classify_encoder(name) else {
                continue;
            };
            match codec {
                "h264" => {
                    if seen_h264.insert(name) {
                        h264.push(backend);
                    }
                }
                "h265" if seen_h265.insert(name) => {
                    h265.push(backend);
                }
                _ => {}
            }
        }

        h264.sort_by_key(backend_rank);
        h265.sort_by_key(backend_rank);

        Self { h264, h265 }
    }

    /// One-line human-readable summary suitable for `tracing::info!`.
    pub fn summary(&self) -> String {
        if self.is_empty() {
            return "no GPU encoder detected; CPU fallback (libx264/libx265)".to_string();
        }
        let h264 = self
            .h264()
            .iter()
            .map(|b| backend_short(*b))
            .collect::<Vec<_>>()
            .join(",");
        let h265 = self
            .h265()
            .iter()
            .map(|b| backend_short(*b))
            .collect::<Vec<_>>()
            .join(",");
        format!("encoder capabilities: h264=[{h264}], hevc=[{h265}]")
    }
}

fn classify_encoder(name: &str) -> Option<(VcodecBackend, &'static str)> {
    match name {
        "h264_nvenc" => Some((VcodecBackend::Nvenc, "h264")),
        "hevc_nvenc" => Some((VcodecBackend::Nvenc, "h265")),
        "h264_amf" => Some((VcodecBackend::Amf, "h264")),
        "hevc_amf" => Some((VcodecBackend::Amf, "h265")),
        "h264_qsv" | "h264_videotoolbox" => Some((VcodecBackend::Qsv, "h264")),
        "hevc_qsv" | "hevc_videotoolbox" => Some((VcodecBackend::Qsv, "h265")),
        "libx264" => Some((VcodecBackend::LibSw, "h264")),
        "libx265" => Some((VcodecBackend::LibSw, "h265")),
        _ => None,
    }
}

const fn backend_rank(backend: &VcodecBackend) -> u8 {
    match backend {
        VcodecBackend::Nvenc => 0,
        VcodecBackend::Amf => 1,
        VcodecBackend::Qsv => 2,
        VcodecBackend::LibSw => 3,
    }
}

fn backend_short(backend: VcodecBackend) -> &'static str {
    match backend {
        VcodecBackend::Nvenc => "nvenc",
        VcodecBackend::Amf => "amf",
        VcodecBackend::Qsv => "qsv",
        VcodecBackend::LibSw => "libsw",
    }
}

/// Maps a `(backend, target)` pair to the actual ffmpeg encoder name.
#[must_use]
pub fn backend_arg(backend: VcodecBackend, target: VcodecTarget) -> &'static str {
    match (backend, target) {
        (VcodecBackend::Nvenc, VcodecTarget::H264) => "h264_nvenc",
        (VcodecBackend::Nvenc, VcodecTarget::H265) => "hevc_nvenc",
        (VcodecBackend::Amf, VcodecTarget::H264) => "h264_amf",
        (VcodecBackend::Amf, VcodecTarget::H265) => "hevc_amf",
        (VcodecBackend::Qsv, VcodecTarget::H264) => "h264_qsv",
        (VcodecBackend::Qsv, VcodecTarget::H265) => "hevc_qsv",
        (VcodecBackend::LibSw, VcodecTarget::H264) => "libx264",
        (VcodecBackend::LibSw, VcodecTarget::H265) => "libx265",
    }
}

/// Runtime cache of the encoder capabilities probe.
static CAPS: tokio::sync::OnceCell<EncoderCapabilities> = tokio::sync::OnceCell::const_new();

/// Probes `ffmpeg -encoders` (once, cached for the process lifetime) and
/// returns a reference to the resulting capabilities.
///
/// Falls back to an empty [`EncoderCapabilities`] (CPU-only) when the probe
/// fails so callers can always dereference the result safely.
#[must_use]
pub async fn probe_encoders() -> &'static EncoderCapabilities {
    CAPS.get_or_init(|| async { probe_encoders_inner().await.unwrap_or_default() })
        .await
}

async fn probe_encoders_inner() -> Result<EncoderCapabilities> {
    let output = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-encoders")
        .output()
        .await
        .map_err(|e| Error::Ffmpeg(format!("failed to launch ffmpeg -encoders: {e}")))?;
    if !output.status.success() {
        return Err(Error::Ffmpeg(format!(
            "ffmpeg -encoders exited with {}",
            output.status
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(EncoderCapabilities::parse(&text))
}

/// Remuxes a media file into a Matroska (`.mkv`) container via stream copy.
///
/// Quality is preserved (no re-encode). On success the input file is removed
/// and the path to the new `.mkv` is returned. When the input is already a
/// `.mkv` this is a no-op (returns the same path).
///
/// Failures are returned to the caller; the original file is preserved when
/// the new muxer cannot be produced.
///
/// # Errors
///
/// Returns [`Error::Ffmpeg`] when the `ffmpeg` process cannot be launched or
/// exits non-zero, or when the atomic rename fails.
pub async fn remux_to_mkv(input: &Path) -> Result<PathBuf> {
    let output = input.with_extension("mkv");
    if output == *input {
        return Ok(output);
    }
    let tmp = input.with_extension("mkv.remux.tmp");

    let status = Command::new("ffmpeg")
        .arg("-y")
        .arg("-nostdin")
        .arg("-i")
        .arg(input)
        .arg("-c")
        .arg("copy")
        .arg("-f")
        .arg("matroska")
        .arg(&tmp)
        .status()
        .await
        .map_err(|e| Error::Ffmpeg(format!("failed to launch ffmpeg remux: {e}")))?;

    if !status.success() {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(Error::Ffmpeg(format!(
            "ffmpeg remux exited with {status} for {}",
            input.display()
        )));
    }

    // `std::fs::rename` (not `tokio::fs::rename`) for the same Windows
    // reason documented in `transcode::reencode`.
    std::fs::rename(&tmp, &output)
        .map_err(|e| Error::Ffmpeg(format!("remux rename failed: {e}")))?;
    let _ = tokio::fs::remove_file(input).await;
    Ok(output)
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

    // `std::fs::rename` for Windows replace-existing semantics — see
    // `transcode::reencode` for the full rationale.
    std::fs::rename(&tmp_output, &mkv_str)
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

    // -------------------------------------------------------------------
    // EncoderCapabilities::parse tests
    // -------------------------------------------------------------------

    const FFMPEG_ENCODERS_NVENC: &str = "\
Encoders:
 V..... = Video
 A..... = Audio
 S..... = Subtitle
 ------
 V..... libx264              libx264 H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10 (codec h264)
 V..... libx265              libx265 H.265 / HEVC (codec hevc)
 V..... h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)
 V..... hevc_nvenc           NVIDIA NVENC hevc encoder (codec hevc)
";

    const FFMPEG_ENCODERS_SOFTWARE_ONLY: &str = "\
Encoders:
 V..... libx264              libx264 H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10 (codec h264)
 V..... libx265              libx265 H.265 / HEVC (codec hevc)
";

    const FFMPEG_ENCODERS_MIXED: &str = "\
Encoders:
 V..... libx264              libx264 H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10 (codec h264)
 V..... hevc_qsv             HEVC / H.265 / MPEG-H HEVC (codec hevc)
 V..... hevc_nvenc           NVIDIA NVENC hevc encoder (codec hevc)
";

    #[test]
    fn test_should_parse_nvenc_capabilities() {
        let caps = EncoderCapabilities::parse(FFMPEG_ENCODERS_NVENC);
        // Fixture contains both libx264/libx265 and h264_nvenc/hevc_nvenc;
        // NVENC must be picked as the preferred backend for both codecs.
        assert_eq!(caps.preferred_h264(), VcodecBackend::Nvenc);
        assert_eq!(caps.preferred_h265(), VcodecBackend::Nvenc);
        assert!(caps.h264().contains(&VcodecBackend::Nvenc));
        assert!(caps.h265().contains(&VcodecBackend::Nvenc));
        assert!(!caps.is_empty());
    }

    #[test]
    fn test_should_parse_software_only_capabilities() {
        let caps = EncoderCapabilities::parse(FFMPEG_ENCODERS_SOFTWARE_ONLY);
        assert!(caps.h264().contains(&VcodecBackend::LibSw));
        assert!(caps.h265().contains(&VcodecBackend::LibSw));
        assert_eq!(caps.preferred_h264(), VcodecBackend::LibSw);
        assert_eq!(caps.preferred_h265(), VcodecBackend::LibSw);
    }

    #[test]
    fn test_should_prefer_nvenc_over_software_when_both_present() {
        let caps = EncoderCapabilities::parse(FFMPEG_ENCODERS_NVENC);
        assert_eq!(caps.preferred_h264(), VcodecBackend::Nvenc);
        assert_eq!(caps.preferred_h265(), VcodecBackend::Nvenc);
    }

    #[test]
    fn test_should_prefer_nvenc_over_qsv_when_both_present() {
        let caps = EncoderCapabilities::parse(FFMPEG_ENCODERS_MIXED);
        // h265 has both nvenc and qsv; nvenc must come first.
        assert_eq!(caps.h265()[0], VcodecBackend::Nvenc);
        assert!(caps.h265().contains(&VcodecBackend::Qsv));
    }

    #[test]
    fn test_should_deduplicate_repeated_encoder_names() {
        let repeated = "V..... h264_nvenc foo\nV..... h264_nvenc bar\n";
        let caps = EncoderCapabilities::parse(repeated);
        assert_eq!(caps.h264().len(), 1);
    }

    #[test]
    fn test_should_fall_back_to_libsw_when_no_h264_present() {
        // Build an artificial case with only h265 to exercise the fallback.
        let h265_only = "V..... hevc_nvenc nvenc hevc\n";
        let caps = EncoderCapabilities::parse(h265_only);
        assert_eq!(caps.preferred_h264(), VcodecBackend::LibSw);
        assert_eq!(caps.preferred_h265(), VcodecBackend::Nvenc);
    }

    #[test]
    fn test_should_return_empty_for_unrecognised_encoder_text() {
        let caps = EncoderCapabilities::parse("garbage line\nno encoder here\n");
        assert!(caps.is_empty());
    }

    #[test]
    fn test_backend_arg_returns_correct_ffmpeg_encoder_name() {
        assert_eq!(
            backend_arg(VcodecBackend::Nvenc, VcodecTarget::H264),
            "h264_nvenc"
        );
        assert_eq!(
            backend_arg(VcodecBackend::Nvenc, VcodecTarget::H265),
            "hevc_nvenc"
        );
        assert_eq!(
            backend_arg(VcodecBackend::LibSw, VcodecTarget::H264),
            "libx264"
        );
        assert_eq!(
            backend_arg(VcodecBackend::LibSw, VcodecTarget::H265),
            "libx265"
        );
        assert_eq!(
            backend_arg(VcodecBackend::Qsv, VcodecTarget::H265),
            "hevc_qsv"
        );
    }

    #[test]
    fn test_summary_includes_backend_names_when_present() {
        let caps = EncoderCapabilities::parse(FFMPEG_ENCODERS_NVENC);
        let s = caps.summary();
        assert!(s.contains("nvenc"), "summary should mention nvenc: {s}");
    }

    #[test]
    fn test_summary_reports_cpu_fallback_when_empty() {
        let caps = EncoderCapabilities::default();
        assert!(caps.summary().contains("CPU fallback"));
    }
}
