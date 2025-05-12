use anyhow::Result;
use cat_show_downloader::{
    check_requirements, downloader::download_all_episodes, models::SeasonSelection,
    utils::error::Error,
};
use clap::Parser;
use std::path::PathBuf;
use tracing::info;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Slug of the TV show, for https://www.3cat.cat/3cat/bola-de-drac/ should be bola-de-drac
    #[arg(required = true)]
    tv_show_slug: String,

    /// Directory to save the episodes
    #[arg(short = 'd', long, default_value = "output")]
    directory: String,

    /// Episode number to start from
    #[arg(long, default_value_t = 1)]
    start_from_episode: i32,

    /// Season number to start from
    #[arg(long, default_value_t = 1)]
    start_from_season: i32,

    /// Season number to end at inclusive
    #[arg(long, default_value_t = 10)]
    end_at_season: i32,

    /// If set, all the original files and the intermediate files will be kept
    #[arg(short = 'k', long, default_value_t = false)]
    keep_all_files: bool,
}

#[tokio::main]
async fn main() {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .or_else(|e| {
                    println!("Using default log filter directive: {}", e);
                    tracing_subscriber::EnvFilter::try_new("info")
                })
                .unwrap(),
        )
        .with_writer(std::io::stderr)
        .compact()
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    tracing::info!("Starting TV3 downloader");
    if let Err(e) = inner_main().await {
        tracing::error!("Error: {}\n{}", e, e.backtrace());
        std::process::exit(1);
    }
}

async fn inner_main() -> Result<()> {
    let args = Args::parse();
    let directory = PathBuf::from(&args.directory);

    info!(
        "Started 3Cat show downloader for show {}",
        args.tv_show_slug
    );

    if !check_requirements() {
        return Err(Error::MissingDependencies.into());
    }

    // Create build directory if it doesn't exist
    std::fs::create_dir_all(&directory)
        .map_err(|e| Error::IoError(format!("Failed to create directory {}", args.directory), e))?;

    download_all_episodes(
        SeasonSelection {
            start: args.start_from_season,
            end: args.end_at_season,
        },
        args.start_from_episode,
        &args.tv_show_slug,
        &directory,
        args.keep_all_files,
    )
    .await
}
