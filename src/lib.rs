//! Library crate for downloading TV shows and movies from 3cat.cat.
//!
//! This crate provides the core logic for resolving media slugs, fetching
//! episode metadata, downloading video and subtitle files, and optionally
//! embedding subtitles via ffmpeg. The binary crate (`main.rs`) is a thin
//! entry point that sets up tracing, builds the Tokio runtime, and delegates
//! to [`run()`].

mod api_structs;
mod cli;
mod downloader;
mod error;
mod ffmpeg;
pub mod gui;
mod http_client;
mod media_resolver;
mod models;
mod movie;
mod plex;
mod scheduler;
pub mod subtitle_cleaner;
mod transcode;
mod tv_show;
mod yt_dlp;

use std::io;
use std::sync::Arc;

use indicatif::MultiProgress;
use tracing::{info, instrument, warn};
use tracing_subscriber::fmt::MakeWriter;

pub use crate::cli::CatShowDownloaderArgs;
use crate::media_resolver::MediaType;
use crate::models::{DownloadParams, SubtitleMode};
use crate::transcode::ReencodePreset;

/// A [`MakeWriter`] implementation that routes output through [`MultiProgress::println`].
///
/// This ensures that tracing log lines are printed above the progress bars
/// without disrupting their rendering.
#[derive(Clone, Debug)]
pub struct MultiProgressWriter {
    mp: MultiProgress,
}

impl MultiProgressWriter {
    /// Creates a new writer that routes output through the given [`MultiProgress`].
    pub fn new(mp: MultiProgress) -> Self {
        Self { mp }
    }
}

/// Per-event writer that buffers bytes and flushes complete lines via [`MultiProgress::println`].
///
/// This type is an implementation detail of [`MultiProgressWriter`] and should
/// not be constructed directly.
#[derive(Debug)]
pub struct MultiProgressLineWriter {
    mp: MultiProgress,
    buf: Vec<u8>,
}

impl<'a> MakeWriter<'a> for MultiProgressWriter {
    type Writer = MultiProgressLineWriter;

    fn make_writer(&'a self) -> Self::Writer {
        MultiProgressLineWriter {
            mp: self.mp.clone(),
            buf: Vec::new(),
        }
    }
}

impl io::Write for MultiProgressLineWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.buf.is_empty() {
            let line = String::from_utf8_lossy(&self.buf).trim_end().to_string();
            self.mp.println(&line)?;
            self.buf.clear();
        }
        Ok(())
    }
}

impl Drop for MultiProgressLineWriter {
    fn drop(&mut self) {
        let _ = io::Write::flush(self);
    }
}

/// Runs the main application logic.
///
/// Resolves the provided slug as a TV show or movie, determines subtitle
/// handling mode, and dispatches the appropriate download workflow.
///
/// # Errors
///
/// Returns an error if media resolution, subtitle processing, or downloading
/// fails.
#[allow(clippy::let_and_return)] // Binding needed to satisfy Rust 2024 tail-expression drop order rules
#[instrument(skip_all)]
pub async fn run(args: CatShowDownloaderArgs, multi_progress: MultiProgress) -> anyhow::Result<()> {
    if args.start_from_episode < 1 {
        anyhow::bail!(
            "start_from_episode must be at least 1, got {}",
            args.start_from_episode
        );
    }

    // Strip trailing path separators (`\` or `/`) from `--directory` so
    // `Path::join("Temporada 01")` produces a clean path rather than
    // `"G:\\Series\\Mic3\\""\\Temporada 01` (which fails on Windows with
    // `os error 123`: "The filename, directory name, or volume label
    // syntax is incorrect"). Also strip a literal pair of surrounding
    // double quotes that some shells forward when the user types `-d "..."`.
    let mut directory = args.directory.trim().to_string();
    if directory.starts_with('"') && directory.ends_with('"') && directory.len() >= 2 {
        directory = directory[1..directory.len() - 1].to_string();
    }
    let mut directory = directory.trim_end_matches(['\\', '/']).to_string();
    info!("Output directory: {directory:?}");

    if args.fix_existing_subtitles {
        subtitle_cleaner::fix_existing_subtitles(&directory)?;
    }

    let (ffmpeg_available, yt_dlp_available) =
        tokio::join!(ffmpeg::is_available(), yt_dlp::is_available());

    if args.embed_existing_subtitles {
        if !ffmpeg_available {
            anyhow::bail!(
                "--embed-existing-subtitles requires ffmpeg, but ffmpeg was not found on PATH"
            );
        }
        ffmpeg::embed_existing_subtitles(&directory).await?;
    }

    let http_client = http_client::http_client();
    let media = media_resolver::get_media_id(&args.slug).await?;

    if args.plex_metadata {
        let tvdb_id = args
            .tvdb_series_id
            .ok_or_else(|| anyhow::anyhow!("--plex-metadata requires --tvdb-series-id"))?;
        let require_tools = !matches!(args.repair_existing, Some(cli::RepairExistingMode::Plan));
        plex::preflight(require_tools)?;
        let tv_show_id = match &media {
            MediaType::TvShow(id) => *id,
            MediaType::Movie { .. } => {
                anyhow::bail!("--plex-metadata currently supports TV shows only")
            }
        };
        if let Some(mode) = args.repair_existing {
            return plex::repair_existing(
                std::path::Path::new(&directory),
                tv_show_id,
                tvdb_id,
                matches!(mode, cli::RepairExistingMode::Apply),
                &http_client,
            )
            .await;
        }
        directory = plex::series_root(std::path::Path::new(&directory), tvdb_id)
            .to_string_lossy()
            .into_owned();
        info!("Plex series directory: {directory:?}");
    }

    let subtitle_mode = if args.skip_subtitles {
        SubtitleMode::Skip
    } else if ffmpeg_available {
        info!("ffmpeg detected, subtitles will be embedded into video files");
        SubtitleMode::Embed
    } else {
        warn!("ffmpeg not found, subtitles will be downloaded as separate .vtt files");
        SubtitleMode::Download
    };

    if args.strict_subtitles {
        info!("--strict-subtitles set: subtitle failures will abort the batch");
    } else {
        info!(
            "Subtitle failures will be tolerated (warn + skip). Pass --strict-subtitles to abort on failures."
        );
    }

    if args.auto_naming {
        info!(
            "--auto-naming set: files will be saved as 'Mic - <slug> - S{:02}E<ep> (<fmt>)' under Temporada XX/ (or Pel·lícules/ for movies); season defaults to {}",
            args.season, args.season
        );
    }

    if yt_dlp_available {
        info!("yt-dlp detected, using it as the download backend");
    }

    let reencode_preset = ReencodePreset::parse(&args.reencode)?;
    if !reencode_preset.is_off() && !ffmpeg_available {
        anyhow::bail!(
            "--reencode {} requires ffmpeg, but ffmpeg was not found on PATH",
            args.reencode
        );
    }
    if !reencode_preset.is_off() {
        info!(
            "--reencode {} set: each downloaded file will be re-encoded (audio + subs preserved)",
            args.reencode
        );
    }

    if args.request_delay_ms > 0 {
        info!(
            "--request-delay-ms {} set: a delay will be applied before each yt-dlp invocation to avoid 3cat's rate limiter",
            args.request_delay_ms
        );
    }

    // Probe encoder capabilities once so the re-encode stage can prefer the
    // GPU (NVIDIA NVENC / AMD AMF / Intel QSV) over software `libx264` /
    // `libx265`. The probe is best-effort: when it fails we silently fall
    // back to CPU encoders in `pick_vcodec`. A summary line is emitted at
    // info level so the user can see which backend is in use.
    let reencode_capabilities = ffmpeg::probe_encoders().await;
    let reencode_capabilities = Arc::new((*reencode_capabilities).clone());
    info!("{}", reencode_capabilities.summary());

    let auto_naming = args.auto_naming;
    let params = DownloadParams {
        http_client,
        subtitle_mode,
        concurrent_downloads: args.concurrent_downloads,
        multi_progress,
        directory: Arc::from(directory.as_str()),
        yt_dlp_available,
        strict_subtitles: args.strict_subtitles,
        auto_naming,
        reencode_preset,
        reencode_capabilities,
        request_delay_ms: args.request_delay_ms,
        plex_metadata: args.plex_metadata,
        tvdb_series_id: args.tvdb_series_id,
    };
    let season = i32::try_from(args.season).ok();

    let result = match media {
        MediaType::TvShow(id) => {
            tv_show::download(id, args.start_from_episode, &params, season).await
        }
        MediaType::Movie { id, slug } => movie::download(id, &slug, &params).await,
    };

    result
}
