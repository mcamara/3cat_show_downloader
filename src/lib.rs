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
mod http_client;
mod media_resolver;
mod models;
mod movie;
mod scheduler;
pub mod subtitle_cleaner;
mod tv_show;

use std::io;
use std::sync::Arc;

use indicatif::MultiProgress;
use tracing::{info, instrument, warn};
use tracing_subscriber::fmt::MakeWriter;

pub use crate::cli::CatShowDownloaderArgs;
use crate::media_resolver::MediaType;
use crate::models::{DownloadParams, SubtitleMode};

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
    let ffmpeg_available = ffmpeg::is_available().await;

    if args.embed_existing_subtitles {
        if !ffmpeg_available {
            anyhow::bail!(
                "--embed-existing-subtitles requires ffmpeg, but ffmpeg was not found on PATH"
            );
        }
        ffmpeg::embed_existing_subtitles(&args.directory).await?;
    }

    let http_client = http_client::http_client();
    let media = media_resolver::get_media_id(&args.slug).await?;

    if args.fix_existing_subtitles {
        subtitle_cleaner::fix_existing_subtitles(&args.directory)?;
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

    let params = DownloadParams {
        http_client,
        subtitle_mode,
        concurrent_downloads: args.concurrent_downloads,
        multi_progress,
        directory: Arc::from(args.directory.as_str()),
    };

    let result = match media {
        MediaType::TvShow(id) => tv_show::download(id, args.start_from_episode, &params).await,
        MediaType::Movie { id, slug } => movie::download(id, &slug, &params).await,
    };

    result
}
