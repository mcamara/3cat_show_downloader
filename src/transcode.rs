//! Post-download re-encoding to reduce file size.
//!
//! Uses `ffmpeg` to re-encode the video stream of an already-downloaded
//! episode with a more efficient codec/CRF combination. Audio and
//! subtitle streams are copied untouched, so quality loss is limited to
//! the video stream and the operation is reversible (the original file
//! is replaced atomically via a `.reencode.tmp` intermediate that gets
//! renamed on success).
//!
//! Failures are non-fatal: the original file is preserved when ffmpeg
//! exits non-zero, and the caller logs a warning. This keeps the
//! tolerant-subtitles behaviour consistent — a single broken episode
//! should not abort the whole batch.

use std::path::{Path, PathBuf};

use indicatif::MultiProgress;
use tokio::process::Command;
use tracing::{debug, instrument, warn};

use crate::error::{Error, Result};
use crate::ffmpeg::{EncoderCapabilities, VcodecBackend, VcodecTarget, backend_arg};

/// Re-encoding preset controlling the target codec and quality level.
///
/// Variants are ordered from no-op (`Off`) to maximum compression (`Max`).
/// All non-`Off` variants delegate to ffmpeg and prefer a hardware
/// encoder when the host advertises one (see [`EncoderCapabilities`]);
/// the software `libx264`/`libx265` fallback is used otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReencodePreset {
    /// No re-encoding — the file is left exactly as the downloader produced it.
    Off,
    /// H.264 at quality 26. Roughly -35% file size, near-transparent
    /// quality. Prefers `h264_nvenc` when available; otherwise `libx264`.
    Light,
    /// H.264 at quality 23. Roughly -50% file size, indistinguishable
    /// from the original at normal viewing distances (recommended sweet
    /// spot). Prefers `h264_nvenc`; otherwise `libx264`.
    Balanced,
    /// H.265 (HEVC) at quality 28. Roughly -65% file size, requires
    /// HEVC-capable hardware for smooth playback. Prefers `hevc_nvenc`;
    /// otherwise `libx265` with `ultrafast`. With a RTX 30xx/40xx-class
    /// GPU a 10-minute episode re-encodes in ≈5s.
    Max,
}

impl ReencodePreset {
    /// Returns `true` when no re-encoding should be performed.
    pub fn is_off(self) -> bool {
        matches!(self, Self::Off)
    }

    /// Parses a preset name from a CLI string. Case-insensitive.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Ffmpeg`] for unrecognised names so the CLI can
    /// surface a clean error message before any work begins.
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "light" => Ok(Self::Light),
            "balanced" => Ok(Self::Balanced),
            "max" => Ok(Self::Max),
            other => Err(Error::Ffmpeg(format!(
                "unknown --reencode preset '{other}'; expected one of: off, light, balanced, max"
            ))),
        }
    }
}

/// Resolved encoder pick for a given preset + capabilities snapshot.
///
/// Carries everything [`reencode`] needs to assemble the ffmpeg command
/// without consulting the probe at run time. Made `pub` so callers and
/// tests can introspect the choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcodecChoice {
    /// ffmpeg encoder name (e.g. `hevc_nvenc`, `libx264`).
    pub vcodec: &'static str,
    /// Target codec family.
    pub target: VcodecTarget,
    /// Backend family that produced the choice.
    pub backend: VcodecBackend,
    /// Quality value as a string (CRF for software, CQ for NVENC/AMF,
    /// global_quality for QSV).
    pub quality: &'static str,
    /// Whether this pick uses hardware acceleration (`Nvenc`/`Amf`/`Qsv`).
    pub is_gpu: bool,
}

/// Resolves a preset to its concrete encoder choice, taking the host's
/// advertised capabilities into account. Returns `None` when `preset` is
/// [`ReencodePreset::Off`] so callers can short-circuit.
#[must_use]
pub fn pick_vcodec(
    preset: ReencodePreset,
    capabilities: &EncoderCapabilities,
) -> Option<VcodecChoice> {
    if preset.is_off() {
        return None;
    }
    let (target, quality) = match preset {
        ReencodePreset::Off => unreachable!("guarded above"),
        ReencodePreset::Light => (VcodecTarget::H264, "26"),
        ReencodePreset::Balanced => (VcodecTarget::H264, "23"),
        ReencodePreset::Max => (VcodecTarget::H265, "28"),
    };
    let backend = match target {
        VcodecTarget::H264 => capabilities.preferred_h264(),
        VcodecTarget::H265 => capabilities.preferred_h265(),
    };
    let vcodec = backend_arg(backend, target);
    Some(VcodecChoice {
        vcodec,
        target,
        backend,
        quality,
        is_gpu: backend != VcodecBackend::LibSw,
    })
}

/// Appends encoder-specific rate-control + preset arguments to `cmd`.
///
/// NVENC/AMF use constant-quality (`-cq`); QSV uses `-global_quality`;
/// software uses `-crf`. The preset name (`p4` for NVENC,
/// `quality` for AMF, `veryfast` for QSV/libx264, `ultrafast` for
/// libx265) varies per backend.
fn append_rate_args(cmd: &mut Command, choice: &VcodecChoice) {
    match choice.backend {
        VcodecBackend::Nvenc => {
            cmd.arg("-preset")
                .arg("p4")
                .arg("-cq")
                .arg(choice.quality)
                .arg("-rc")
                .arg("vbr")
                .arg("-b:v")
                .arg("0");
        }
        VcodecBackend::Amf => {
            cmd.arg("-preset")
                .arg("quality")
                .arg("-cq")
                .arg(choice.quality);
        }
        VcodecBackend::Qsv => {
            cmd.arg("-preset")
                .arg("veryfast")
                .arg("-global_quality")
                .arg(choice.quality);
        }
        VcodecBackend::LibSw => {
            let sw_preset = match choice.target {
                VcodecTarget::H264 => "veryfast",
                VcodecTarget::H265 => "ultrafast",
            };
            cmd.arg("-preset")
                .arg(sw_preset)
                .arg("-crf")
                .arg(choice.quality);
        }
    }
}

/// Returns the human-readable rate-control label (`"CRF"`, `"CQ"`, `"Q"`)
/// for the chosen backend, suitable for log lines.
fn quality_label(choice: &VcodecChoice) -> &'static str {
    match choice.backend {
        VcodecBackend::Nvenc | VcodecBackend::Amf => "CQ",
        VcodecBackend::Qsv => "Q",
        VcodecBackend::LibSw => "CRF",
    }
}

/// Re-encodes a single downloaded file according to `preset`.
///
/// Streams the ffmpeg command output (stderr) through the supplied
/// [`MultiProgress`] so progress stays visible alongside other
/// downloads. On success the original file is replaced atomically;
/// on failure the original file is preserved and an error is returned.
///
/// The audio stream is copied without re-encoding (`-c:a copy`). Subtitle
/// streams (if any) are also copied with `-c:s copy`, so ASS tracks
/// inserted by [`crate::ffmpeg::embed_subtitles`] survive the round-trip
/// unchanged.
///
/// The encoder (NVIDIA NVENC vs AMD AMF vs Intel QSV vs software
/// `libx264`/`libx265`) is chosen from `capabilities`; pass the value
/// obtained from [`crate::ffmpeg::probe_encoders`] (or a custom
/// capabilities struct in tests).
///
/// # Errors
///
/// Returns [`Error::Ffmpeg`] when the process exits with a non-zero
/// status or fails to launch.
#[instrument(skip_all, fields(preset = ?preset, path = %input_path.as_ref().display()))]
#[allow(clippy::too_many_arguments)]
pub async fn reencode(
    input_path: impl AsRef<Path>,
    preset: ReencodePreset,
    capabilities: &EncoderCapabilities,
    multi_progress: &MultiProgress,
) -> Result<PathBuf> {
    let input = input_path.as_ref();
    if preset.is_off() {
        return Ok(input.to_path_buf());
    }

    let Some(choice) = pick_vcodec(preset, capabilities) else {
        return Ok(input.to_path_buf());
    };

    let tmp_output = input.with_extension("reencode.tmp");

    let label = format!(
        "Re-encoding {}",
        input
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("<input>")
    );
    let backend_tag = if choice.is_gpu { "GPU" } else { "CPU" };
    let backend_msg = format!(
        "{} {} {} ({backend_tag})",
        choice.vcodec,
        quality_label(&choice),
        choice.quality,
    );

    let pb = multi_progress.add(indicatif::ProgressBar::new(1000));
    pb.set_style(
        indicatif::ProgressStyle::with_template(
            "{prefix:.bold} [{bar:30.cyan/blue}] {percent}% {elapsed_precise} {msg}",
        )
        .map_err(|e| Error::Ffmpeg(e.to_string()))?,
    );
    pb.set_prefix(label.clone());
    pb.set_message(backend_msg.clone());
    // No `enable_steady_tick` — same rationale as the batch bar in
    // `scheduler.rs`. The bar repaints on every `progress_pb.set_position`
    // driven by `out_time_us` and on `progress=end`, which is enough to
    // make it animate without spamming the terminal.

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y")
        .arg("-nostdin")
        .arg("-hide_banner")
        .arg("-i")
        .arg(input)
        .arg("-c:v")
        .arg(choice.vcodec);

    append_rate_args(&mut cmd, &choice);

    // Force Matroska container on the temp output — `.reencode.tmp` has no
    // extension ffmpeg can infer, so without `-f matroska` it would reject
    // the muxer. `-progress pipe:1` drives the bar from `out_time_us` so
    // indicatif shows a real percentage instead of a moving indeterminate
    // bar. stderr is piped + drained so the intrinsic `frame=...` log stream
    // never leaks to the terminal (which would otherwise corrupt the
    // indicatif render).
    cmd.arg("-c:a")
        .arg("copy")
        .arg("-c:s")
        .arg("copy")
        .arg("-f")
        .arg("matroska")
        .arg("-progress")
        .arg("pipe:1")
        .arg(&tmp_output)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Ffmpeg(format!("failed to launch ffmpeg for re-encoding: {e}")))?;

    let stdout = child.stdout.take().ok_or_else(|| {
        Error::Ffmpeg("failed to capture ffmpeg stdout (progress pipe)".to_string())
    })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Ffmpeg("failed to capture ffmpeg stderr".to_string()))?;

    // Drain stderr so the OS pipe never fills up and back-pressures ffmpeg.
    let stderr_task = tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(_)) = lines.next_line().await {}
    });

    // Parse `-progress pipe:1` output. ffmpeg emits a sequence of
    // `key=value` lines followed by `progress=continue` (or `end`). We don't
    // know the total duration from this stream alone, so we animate the
    // bar monotonically from 0 → 99% on each `out_time_us` tick and jump
    // to 100% on `progress=end`. The result is a smooth bar that the user
    // can see advancing (instead of a static spinner) without an extra
    // `ffprobe` spawn per episode.
    let progress_pb = pb.clone();
    let progress_task = tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut lines = BufReader::new(stdout).lines();
        let mut last_pos: u64 = 0;
        while let Ok(Some(line)) = lines.next_line().await {
            if line.starts_with("out_time_us=") {
                let new_pos = (last_pos + 5).min(990);
                last_pos = new_pos;
                progress_pb.set_position(new_pos);
            } else if line.trim() == "progress=end" {
                progress_pb.set_position(1000);
            }
        }
    });

    let status = child
        .wait()
        .await
        .map_err(|e| Error::Ffmpeg(format!("failed to wait for ffmpeg reencode: {e}")))?;

    let _ = progress_task.await;
    let _ = stderr_task.await;
    pb.finish_and_clear();

    if !status.success() {
        let _ = tokio::fs::remove_file(&tmp_output).await;
        warn!(
            "ffmpeg exited with {} while re-encoding {}. Original file preserved.",
            status,
            input.display()
        );
        return Err(Error::Ffmpeg(format!(
            "ffmpeg exited with status {status} while re-encoding {}",
            input.display()
        )));
    }

    // Atomic rename: original is gone only once the new file is in place.
    // On Windows `tokio::fs::rename` does NOT pass
    // `MOVEFILE_REPLACE_EXISTING`, so it fails when `input` already
    // exists. `std::fs::rename` uses `MoveFileExW` with the replace flag
    // on Windows and POSIX `rename(2)` (which atomically replaces) on
    // Unix, so it is the right cross-platform choice for swapping the
    // re-encoded file into the original path.
    std::fs::rename(&tmp_output, input).map_err(|e| {
        Error::Ffmpeg(format!(
            "failed to replace {} with re-encoded version: {e}",
            input.display()
        ))
    })?;

    let new_size = tokio::fs::metadata(input)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    debug!(
        "Re-encoded {} to {} ({backend_tag}, {} bytes)",
        input.display(),
        choice.vcodec,
        new_size,
    );
    Ok(input.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::EncoderCapabilities;

    fn parse(text: &str) -> EncoderCapabilities {
        EncoderCapabilities::parse(text)
    }

    const NVENC_TEXT: &str = "\
V..... libx264              libx264 (codec h264)
V..... libx265              libx265 (codec hevc)
V..... h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)
V..... hevc_nvenc           NVIDIA NVENC hevc encoder (codec hevc)
";

    const SW_TEXT: &str = "\
V..... libx264              libx264 (codec h264)
V..... libx265              libx265 (codec hevc)
";

    #[test]
    fn test_should_parse_preset_names_case_insensitively() {
        assert_eq!(ReencodePreset::parse("off").unwrap(), ReencodePreset::Off);
        assert_eq!(ReencodePreset::parse("OFF").unwrap(), ReencodePreset::Off);
        assert_eq!(
            ReencodePreset::parse("Light").unwrap(),
            ReencodePreset::Light
        );
        assert_eq!(
            ReencodePreset::parse("BALANCED").unwrap(),
            ReencodePreset::Balanced
        );
        assert_eq!(ReencodePreset::parse("max").unwrap(), ReencodePreset::Max);
    }

    #[test]
    fn test_should_reject_unknown_preset() {
        assert!(ReencodePreset::parse("turbo").is_err());
        assert!(ReencodePreset::parse("").is_err());
    }

    #[test]
    fn test_is_off_only_true_for_off_variant() {
        assert!(ReencodePreset::Off.is_off());
        assert!(!ReencodePreset::Light.is_off());
        assert!(!ReencodePreset::Balanced.is_off());
        assert!(!ReencodePreset::Max.is_off());
    }

    #[test]
    fn test_pick_vcodec_returns_none_for_off_preset() {
        let caps = parse(NVENC_TEXT);
        assert!(pick_vcodec(ReencodePreset::Off, &caps).is_none());
    }

    #[test]
    fn test_pick_vcodec_prefers_nvenc_for_light() {
        let caps = parse(NVENC_TEXT);
        let choice = pick_vcodec(ReencodePreset::Light, &caps).unwrap();
        assert_eq!(choice.vcodec, "h264_nvenc");
        assert_eq!(choice.target, VcodecTarget::H264);
        assert_eq!(choice.backend, VcodecBackend::Nvenc);
        assert_eq!(choice.quality, "26");
        assert!(choice.is_gpu);
    }

    #[test]
    fn test_pick_vcodec_prefers_nvenc_for_balanced() {
        let caps = parse(NVENC_TEXT);
        let choice = pick_vcodec(ReencodePreset::Balanced, &caps).unwrap();
        assert_eq!(choice.vcodec, "h264_nvenc");
        assert_eq!(choice.quality, "23");
        assert!(choice.is_gpu);
    }

    #[test]
    fn test_pick_vcodec_prefers_hevc_nvenc_for_max() {
        let caps = parse(NVENC_TEXT);
        let choice = pick_vcodec(ReencodePreset::Max, &caps).unwrap();
        assert_eq!(choice.vcodec, "hevc_nvenc");
        assert_eq!(choice.target, VcodecTarget::H265);
        assert_eq!(choice.backend, VcodecBackend::Nvenc);
        assert_eq!(choice.quality, "28");
        assert!(choice.is_gpu);
    }

    #[test]
    fn test_pick_vcodec_falls_back_to_libx264_for_light_when_no_gpu() {
        let caps = parse(SW_TEXT);
        let choice = pick_vcodec(ReencodePreset::Light, &caps).unwrap();
        assert_eq!(choice.vcodec, "libx264");
        assert_eq!(choice.backend, VcodecBackend::LibSw);
        assert!(!choice.is_gpu);
    }

    #[test]
    fn test_pick_vcodec_falls_back_to_libx265_for_max_when_no_gpu() {
        let caps = parse(SW_TEXT);
        let choice = pick_vcodec(ReencodePreset::Max, &caps).unwrap();
        assert_eq!(choice.vcodec, "libx265");
        assert_eq!(choice.target, VcodecTarget::H265);
        assert!(!choice.is_gpu);
    }

    #[test]
    fn test_pick_vcodec_falls_back_to_libsw_when_capabilities_empty() {
        let caps = EncoderCapabilities::default();
        let light = pick_vcodec(ReencodePreset::Light, &caps).unwrap();
        assert_eq!(light.vcodec, "libx264");
        let max = pick_vcodec(ReencodePreset::Max, &caps).unwrap();
        assert_eq!(max.vcodec, "libx265");
    }
}
