pub mod api_structs;

use std::sync::Arc;

use crate::{
    http_client::{http_client, HttpClientTrait},
    models::{Episode, SeasonSelection},
    utils::error::Error,
};
use anyhow::Result;
use regex::Regex;
use tracing::{debug, info, trace};

const TV3_TV_SHOW_API_URL: &str = "https://www.3cat.cat/3cat/{slug}/";
const TV3_SINGLE_EPISODE_PAGE_URL: &str = "https://www.3cat.cat/3cat/t/video/{id}/";
const TV3_EPISODE_LIST_URL: &str =
"https://www.3cat.cat/api/3cat/dades/?queryKey=%5B%22tira%22%2C%7B%22url%22%3A%22https%3A%2F%2Fapi.3cat.cat%2Fvideos%3F_format%3Djson%26ordre%3Dcapitol%26origen%3Dllistat%26perfil%3Dpc%26programatv_id%3D{tv_show_id}%26tipus_contingut%3DPPD%26items_pagina%3D1000%26pagina%3D1%26sdom%3Dimg%26version%3D2.0%26cache%3D180%26temporada%3DPUTEMP_{season_number}%26https%3Dtrue%26master%3Dyes%26perfils_extra%3Dimatges_minim_master%22%2C%22moduleName%22%3A%22BlocDeContinguts%22%7D%5D";

pub async fn get_tv_show_id(slug: &str) -> Result<i32, Error> {
    info!("Getting tv show information for slug: {}", slug);
    let response = reqwest::get(TV3_TV_SHOW_API_URL.replace("{slug}", slug).as_str())
        .await
        .map_err(|e| {
            Error::TvShowIdRetrievalError(format!(
                "Error getting tv show id: {} (is the tv show slug correct?)",
                e
            ))
        })?;
    let html_content = response.text().await.map_err(|e| {
        Error::TvShowIdRetrievalError(format!(
            "Error getting tv show id: {} (is the tv show slug correct?)",
            e
        ))
    })?;
    trace!("HTML content: {}", html_content);
    let re = Regex::new(r"programatv_id=(\d+)").unwrap();

    let matches: Vec<_> = re.captures_iter(&html_content).collect();

    if let Some(last_match) = matches.last() {
        if let Some(programatv_id) = last_match.get(1) {
            return Ok(programatv_id.as_str().parse().unwrap());
        }
    }

    Err(Error::TvShowIdRetrievalError(
        "No id found in the the tv show page".to_string(),
    ))
}

pub async fn get_episodes_from_slug(
    tv_show_slug: &str,
    selected_seasons: &SeasonSelection,
) -> Result<Vec<Episode>> {
    let http_client = http_client();
    let id = get_tv_show_id(tv_show_slug).await?;

    get_episodes_from_id(&http_client, id, selected_seasons).await
}

async fn get_episodes_from_id<T>(
    http_client: &Arc<T>,
    tv_show_id: i32,
    selected_seasons: &SeasonSelection,
) -> Result<Vec<Episode>>
where
    T: HttpClientTrait,
{
    debug!("Selecting seasons: {:?}", selected_seasons);

    let mut episodes: Vec<Episode> = vec![];
    for season_number in selected_seasons {
        let query_url = TV3_EPISODE_LIST_URL
            .replace("{tv_show_id}", &tv_show_id.to_string())
            .replace("{season_number}", &season_number.to_string());
        debug!("Querying URL: {}", query_url);
        let tv3_tv_show_api_response = http_client
            .get::<api_structs::EpisodesRoot, api_structs::Tv3Error>(&query_url, None)
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
