//! CLI tool to download complete TV shows from 3cat.cat.

mod api_structs;
mod downloader;
mod episodes;
mod error;
mod http_client;
mod id_retriever;
mod models;

use clap::Parser;
use tracing::info;

use error::Error;
use http_client::HttpClientTrait;

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
}

const TV3_SINGLE_EPISODE_API_URL: &str =
    "https://dinamics.ccma.cat/pvideo/media.jsp?media=video&version=0s&idint={id}";

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let args = Args::parse();

    let http_client = http_client::http_client();
    let id = id_retriever::get_tv_show_id(&args.tv_show_slug).await?;

    let mut episodes = episodes::get_episodes(&http_client, id).await?;

    for episode in episodes.iter_mut() {
        if episode.episode_number < args.start_from_episode {
            info!("Skipping episode {}", episode.episode_number);
            continue;
        }

        let tv3_tv_show_api_response = http_client
            .get::<api_structs::SingleEpisodeRoot, api_structs::Tv3Error>(
                TV3_SINGLE_EPISODE_API_URL
                    .replace("{id}", &episode.id.to_string())
                    .as_str(),
                None,
            )
            .await
            .map_err(|e| Error::Decoding(e.to_string()))?;

        for url in tv3_tv_show_api_response.media.url {
            if !url.active {
                continue;
            }
            episode.video_url = Some(url.file);
            break;
        }

        if let Some(subtitles) = tv3_tv_show_api_response.subtitles.first() {
            episode.subtitle_url = Some(subtitles.url.clone());
        }

        downloader::download_episode(episode, &args.directory).await?;
    }

    Ok(())
}
