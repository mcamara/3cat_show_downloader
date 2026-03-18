//! CLI entry point for the 3cat media downloader.

use cat_show_downloader::{CatShowDownloaderArgs, MultiProgressWriter, run};
use clap::Parser;
use indicatif::MultiProgress;

fn main() -> anyhow::Result<()> {
    let multi_progress = MultiProgress::new();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(MultiProgressWriter::new(multi_progress.clone()))
        .init();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(CatShowDownloaderArgs::parse(), multi_progress))
}
