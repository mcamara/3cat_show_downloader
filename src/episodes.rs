use crate::{Error, Result, api_structs, http_client::HttpClientTrait, models::Episode};
use std::sync::Arc;

const TV3_EPISODE_LIST_URL: &str = "https://www.3cat.cat/api/3cat/dades/?queryKey=%5B%22tira%22%2C%7B%22url%22%3A%22%2F%2Fapi.3cat.cat%2Fvideos%3F_format%3Djson%26no_agrupacio%3DPUAGR_LLSIGN%26tipus_contingut%3DPPD%26items_pagina%3D1500%26pagina%3D1%26sdom%3Dimg%26version%3D2.0%26cache%3D180%26https%3Dtrue%26master%3Dyes%26programatv_id%3D{tv_show_id}%26origen%3Dauto%26perfil%3Dpc%22%7D%5D";

pub(crate) async fn get_episodes<T>(http_client: &Arc<T>, tv_show_id: i32) -> Result<Vec<Episode>>
where
    T: HttpClientTrait,
{
    let mut episodes: Vec<Episode> = vec![];

    let url = TV3_EPISODE_LIST_URL.replace("{tv_show_id}", &tv_show_id.to_string());

    let tv3_tv_show_api_response = http_client
        .get::<api_structs::EpisodesRoot, api_structs::Tv3Error>(&url, None)
        .await
        .map_err(|e| Error::DecodingError(e.to_string()))?;

    let episode_list = tv3_tv_show_api_response.response.items.item;
    if episode_list.is_empty() {
        return Ok(episodes);
    }

    for item in episode_list {
        let title = if item.title.clone().unwrap_or_default().is_empty() {
            item.permatitle
        } else {
            item.title.unwrap_or_default()
        };

        episodes.push(Episode {
            id: item.id,
            title,
            video_url: None,
            subtitle_url: None,
            episode_number: item.number_of_episode,
            tv_show_name: item.tv_show_name,
        });
    }

    Ok(episodes)
}
