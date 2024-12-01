mod api_structs;
mod downloader;
mod error;
mod http_client;
mod models;

use clap::Parser;
use error::{Error, Result};
use http_client::HttpClientTrait;
use models::*;
use std::sync::Arc;

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

const TV3_EPISODE_LIST_URL: &str =
"https://www.3cat.cat/api/3cat/dades/?queryKey=%5B%22tira%22%2C%7B%22url%22%3A%22https%3A%2F%2Fapi.3cat.cat%2Fvideos%3F_format%3Djson%26ordre%3Dcapitol%26origen%3Dllistat%26perfil%3Dpc%26programatv_id%3D{tv_show_id}%26tipus_contingut%3DPPD%26items_pagina%3D1000%26pagina%3D1%26sdom%3Dimg%26version%3D2.0%26cache%3D180%26temporada%3DPUTEMP_{season_number}%26https%3Dtrue%26master%3Dyes%26perfils_extra%3Dimatges_minim_master%22%2C%22moduleName%22%3A%22BlocDeContinguts%22%7D%5D";

const TV3_TV_SHOW_API_URL: &str =
    "https://www.3cat.cat/_next/data/FVPxOFzCQemtHHqLOzDFF/3cat/{slug}/capitols/temporada/1.json";

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let http_client = http_client::http_client();
    let id = get_tv_show_id(&http_client, &args.tv_show_slug).await?;

    let mut episodes = get_episodes(&http_client, id).await?;

    for episode in episodes.iter_mut() {
        if episode.episode_number < args.start_from_episode {
            println!("Skipping episode {}", episode.episode_number);
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
            .map_err(|e| Error::DecodingError(e.to_string()))?;

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

async fn get_tv_show_id<T>(http_client: &Arc<T>, slug: &str) -> Result<i32>
where
    T: HttpClientTrait,
{
    let tv3_tv_show_api_response = http_client
        .get::<api_structs::Root, api_structs::Tv3Error>(
            TV3_TV_SHOW_API_URL.replace("{slug}", slug).as_str(),
            None,
        )
        .await
        .map_err(|e| Error::DecodingError(e.to_string()))?;

    Ok(tv3_tv_show_api_response.page_props.tv_show_data.id)
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
            .map_err(|e| Error::DecodingError(e.to_string()))?;

        let season_episodes = tv3_tv_show_api_response.response.items.item;
        if season_episodes.is_empty() {
            break;
        }

        for item in season_episodes {
            episodes.push(Episode {
                id: item.id,
                title: item.title,
                video_url: None,
                subtitle_url: None,
                episode_number: item.number_of_episode,
                tv_show_name: item.tv_show_name,
            });
        }
    }

    Ok(episodes)
}
