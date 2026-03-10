//! CLI tool to download complete TV shows from 3cat.cat.

mod api_structs;
mod downloader;
mod episodes;
mod error;
mod http_client;
mod id_retriever;
mod models;
mod scheduler;
pub mod subtitle_cleaner;

use std::io;

use clap::Parser;
use indicatif::MultiProgress;
use tracing::{info, instrument};
use tracing_subscriber::fmt::MakeWriter;

/// Command-line arguments for the cat show downloader.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Slug of the TV show, for https://www.3cat.cat/3cat/bola-de-drac/ should be bola-de-drac
    #[arg(short, long)]
    tv_show_slug: String,

    /// Directory to save the episodes
    #[arg(short, long)]
    directory: String,

    /// Episode number to start from, default to the first one
    #[arg(short, long, default_value_t = 1)]
    start_from_episode: i32,

    /// Number of episodes to download concurrently (1-10)
    #[arg(short, long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=10))]
    concurrent_downloads: u8,

    /// Skip downloading subtitles
    #[arg(long, default_value_t = false)]
    skip_subtitles: bool,

    /// Fix (clean) previously downloaded subtitle files in the directory
    #[arg(short, long, default_value_t = false)]
    fix_existing_subtitles: bool,
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

    let http_client = http_client::http_client();
    let id = id_retriever::get_tv_show_id(&args.tv_show_slug).await?;

    if args.fix_existing_subtitles {
        subtitle_cleaner::fix_existing_subtitles(&args.directory)?;
    }

    let episodes = episodes::get_episodes(&http_client, id).await?;

    let episodes_to_download: Vec<_> = episodes
        .into_iter()
        .filter(|ep| {
            if ep.episode_number < args.start_from_episode {
                info!("Skipping episode {}", ep.episode_number);
                false
            } else {
                true
            }
        })
        .collect();

    if episodes_to_download.is_empty() {
        info!("No episodes to download");
        return Ok(());
    }

    info!(
        "Downloading {} episodes ({} at a time)",
        episodes_to_download.len(),
        args.concurrent_downloads,
    );

    let result = scheduler::download_all(
        episodes_to_download,
        args.concurrent_downloads,
        &http_client,
        &args.directory,
        &multi_progress,
        args.skip_subtitles,
    )
    .await;

    result
}
