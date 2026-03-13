//! CLI tool to download TV shows and movies from 3cat.cat.

mod api_structs;
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

use clap::Parser;
use indicatif::MultiProgress;
use tracing::{info, instrument, warn};
use tracing_subscriber::fmt::MakeWriter;

use crate::media_resolver::MediaType;
use crate::models::{DownloadParams, SubtitleMode};

/// Command-line arguments for the 3cat media downloader.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Slug of the TV show or movie (e.g. "bola-de-drac" from https://www.3cat.cat/3cat/bola-de-drac/)
    slug: String,

    /// Directory to save the downloaded files
    #[arg(short, long)]
    directory: String,

    /// Episode number to start from (ignored for movies)
    #[arg(short, long, default_value_t = 1)]
    start_from_episode: i32,

    /// Number of files to download concurrently (1-10)
    #[arg(short, long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=10))]
    concurrent_downloads: u8,

    /// Skip downloading subtitles
    #[arg(long, default_value_t = false)]
    skip_subtitles: bool,

    /// Fix (clean) previously downloaded subtitle files in the directory
    #[arg(short, long, default_value_t = false)]
    fix_existing_subtitles: bool,

    /// Clean and embed existing subtitle files into their matching video files (requires ffmpeg)
    #[arg(long, default_value_t = false)]
    embed_existing_subtitles: bool,
}

/// A [`MakeWriter`] implementation that routes output through [`MultiProgress::println`].
///
/// This ensures that tracing log lines are printed above the progress bars
/// without disrupting their rendering.
#[derive(Clone, Debug)]
struct MultiProgressWriter {
    mp: MultiProgress,
}

/// Per-event writer that buffers bytes and flushes complete lines via [`MultiProgress::println`].
#[derive(Debug)]
struct MultiProgressLineWriter {
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

fn main() -> anyhow::Result<()> {
    let multi_progress = MultiProgress::new();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(MultiProgressWriter {
            mp: multi_progress.clone(),
        })
        .init();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(multi_progress))
}

#[allow(clippy::let_and_return)] // Binding needed to satisfy Rust 2024 tail-expression drop order rules
#[instrument(skip_all)]
async fn run(multi_progress: MultiProgress) -> anyhow::Result<()> {
    let args = Args::parse();

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
