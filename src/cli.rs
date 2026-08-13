//! Command-line argument definitions for the 3cat media downloader.

use clap::{Parser, ValueEnum};

/// Whether an existing Plex collection is only audited or actually repaired.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum RepairExistingMode {
    /// Print the proposed operations without changing files.
    Plan,
    /// Apply the validated operations one file at a time.
    Apply,
}

/// Command-line arguments for the 3cat media downloader.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct CatShowDownloaderArgs {
    /// Slug of the TV show or movie (e.g. "bola-de-drac" from https://www.3cat.cat/3cat/bola-de-drac/)
    pub(crate) slug: String,

    /// Directory to save the downloaded files
    #[arg(short, long)]
    pub(crate) directory: String,

    /// Episode number to start from (ignored for movies)
    #[arg(short, long, default_value_t = 1)]
    pub(crate) start_from_episode: i32,

    /// Number of files to download concurrently (1-10)
    #[arg(short, long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=10))]
    pub(crate) concurrent_downloads: u8,

    /// Skip downloading subtitles
    #[arg(long, default_value_t = false)]
    pub(crate) skip_subtitles: bool,

    /// Fix (clean) previously downloaded subtitle files in the directory
    #[arg(short, long, default_value_t = false)]
    pub(crate) fix_existing_subtitles: bool,

    /// Clean and embed existing subtitle files into their matching video files (requires ffmpeg)
    #[arg(long, default_value_t = false)]
    pub(crate) embed_existing_subtitles: bool,

    /// Abort the whole batch if a subtitle fails to download, clean, or embed.
    /// Without this flag, subtitle failures are logged as warnings and the
    /// affected episodes are reported in the final summary.
    #[arg(long, default_value_t = false)]
    pub(crate) strict_subtitles: bool,

    /// Save files using the `Mic - <episode_slug> - S<season>E<episode> (<ext>)`
    /// naming convention and organize episodes into `Temporada XX/` (or
    /// `Pel·lícules/` for movies) subdirectories under the output directory.
    #[arg(long, default_value_t = false)]
    pub(crate) auto_naming: bool,

    /// Season number used when `--auto-naming` is set. Defaults to 1.
    /// Zero-padded to two digits in the generated filename.
    #[arg(long, default_value_t = 1)]
    pub(crate) season: u32,

    /// Re-encode each downloaded episode with ffmpeg to reduce file size.
    /// Presets:
    ///   * `off`      (default) — leave files untouched.
    ///   * `light`    — H.264 CRF 26, ~-35% size, near-transparent quality.
    ///   * `balanced` — H.264 CRF 23, ~-50% size, recommended sweet spot.
    ///   * `max`      — H.265 (HEVC) CRF 28, ~-65% size, slower to encode.
    ///
    /// Prefers hardware encoders (NVIDIA NVENC) when the host ffmpeg has
    /// them compiled in; falls back transparently to software
    /// `libx264`/`libx265`. Requires `ffmpeg` on PATH. Failures are
    /// tolerated (warn + keep original).
    #[arg(long, value_parser = clap::value_parser!(String), default_value = "off")]
    pub(crate) reencode: String,

    /// Delay between consecutive yt-dlp invocations, in milliseconds.
    /// 3cat.cat tends to return `HTTP 503 backend read error` under
    /// sustained bursts of requests (a 339-episode batch is large enough
    /// to trip the rate limiter). The default of 1500 ms is conservative
    /// enough to keep the API happy while keeping total runtime
    /// reasonable. Set to 0 to disable.
    #[arg(long, default_value_t = 1500)]
    pub(crate) request_delay_ms: u64,

    /// Generate Plex-compatible names, NFO files, and redundant Matroska tags.
    #[arg(long, default_value_t = false)]
    pub(crate) plex_metadata: bool,

    /// TheTVDB series ID used for Aired Order matching (MIC3 is 280190).
    #[arg(long, requires = "plex_metadata")]
    pub(crate) tvdb_series_id: Option<u32>,

    /// Audit or repair an existing collection instead of downloading media.
    #[arg(long, value_enum, requires = "plex_metadata")]
    pub(crate) repair_existing: Option<RepairExistingMode>,
}
