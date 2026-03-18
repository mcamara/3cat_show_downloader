//! Command-line argument definitions for the 3cat media downloader.

use clap::Parser;

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
}
