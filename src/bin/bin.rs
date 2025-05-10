use anyhow::Result;
use cat_show_downloader::{
    api_structs, downloader, error::Error, http_client, http_client::HttpClientTrait, id_retriever,
    models::*, subtitles_fix,
};
use clap::Parser;
use tracing::info;
use std::{path::PathBuf, sync::Arc};

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

const TV3_SINGLE_EPISODE_PAGE_URL: &str = "https://www.3cat.cat/3cat/t/video/{id}/";

const TV3_EPISODE_LIST_URL: &str =
"https://www.3cat.cat/api/3cat/dades/?queryKey=%5B%22tira%22%2C%7B%22url%22%3A%22https%3A%2F%2Fapi.3cat.cat%2Fvideos%3F_format%3Djson%26ordre%3Dcapitol%26origen%3Dllistat%26perfil%3Dpc%26programatv_id%3D{tv_show_id}%26tipus_contingut%3DPPD%26items_pagina%3D1000%26pagina%3D1%26sdom%3Dimg%26version%3D2.0%26cache%3D180%26temporada%3DPUTEMP_{season_number}%26https%3Dtrue%26master%3Dyes%26perfils_extra%3Dimatges_minim_master%22%2C%22moduleName%22%3A%22BlocDeContinguts%22%7D%5D";

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

    // Create build directory if it doesn't exist
    std::fs::create_dir_all(&args.directory)
        .map_err(|e| Error::IoError(format!("Failed to create directory {}", args.directory), e))?;

    let http_client = http_client::http_client();
    let id = id_retriever::get_tv_show_id(&args.tv_show_slug).await?;

    let mut episodes = get_episodes(&http_client, id).await?;

    let episodes_count = episodes.len();

    for episode in episodes.iter_mut() {
        if episode.episode_number < args.start_from_episode {
            info!("Skipping episode {}", episode.episode_number);
            continue;
        }

        info!(
            "Downloading episode {} of {}: {}",
            episode.episode_number, episodes_count, episode.title
        );
        downloader::download_episode(episode, &args.directory).await?;
        subtitles_fix::fix_subtitles(episode, &directory)?;
        break;
    }

    Ok(())
}

async fn get_episodes<T>(http_client: &Arc<T>, tv_show_id: i32) -> Result<Vec<Episode>>
where
    T: HttpClientTrait,
{
    let mut episodes: Vec<Episode> = vec![];
    for season_number in 1..10 {
        let tv3_tv_show_api_response = http_client
            .get::<api_structs::EpisodesRoot, api_structs::Tv3Error>(
                TV3_EPISODE_LIST_URL
                    .replace("{tv_show_id}", &tv_show_id.to_string())
                    .replace("{season_number}", &season_number.to_string())
                    .as_str(),
                None,
            )
            .await
            .map_err(Error::EpisodeRetrieveError)?;

        let season_episodes = tv3_tv_show_api_response.response.items.item;
        if season_episodes.is_empty() {
            break;
        }

        for item in season_episodes {
            episodes.push(Episode::new(
                item.title,
                TV3_SINGLE_EPISODE_PAGE_URL.replace("{id}", &item.id.to_string()),
                item.number_of_episode,
                season_number,
                item.tv_show_name,
            ));
        }
    }

    Ok(episodes)
}
