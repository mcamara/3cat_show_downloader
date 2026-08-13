//! Fetches the episode list for a TV show from the 3cat API.

use std::sync::Arc;

use regex::Regex;
use tracing::instrument;

use super::api_structs;
use crate::error::{Error, Result};
use crate::http_client::HttpClientTrait;
use crate::models::MediaItem;

const TV3_EPISODE_LIST_URL: &str = "https://www.3cat.cat/api/3cat/dades/?queryKey=%5B%22tira%22%2C%7B%22url%22%3A%22%2F%2Fapi.3cat.cat%2Fvideos%3F_format%3Djson%26no_agrupacio%3DPUAGR_LLSIGN%26tipus_contingut%3DPPD%26items_pagina%3D1500%26pagina%3D1%26sdom%3Dimg%26version%3D2.0%26cache%3D180%26https%3Dtrue%26master%3Dyes%26programatv_id%3D{tv_show_id}%26origen%3Dauto%26perfil%3Dpc%22%7D%5D";

/// Extracts the episode number from a `permatitle` of the form
/// `"T1xC679 - La pilota és meva"` or `"T2xC12 - ..."`.
///
/// The 3cat API stopped returning the `capitol` field on the live episode
/// list and now only exposes `permatitle` / `capitol_temporada` (which is
/// always `-1` for these items). The episode number is recoverable from the
/// `T<n>xC<num>` prefix in the permatitle. Returns `None` when the prefix
/// is missing or malformed.
fn extract_episode_number_from_permatitle(permatitle: &str) -> Option<i32> {
    let re = Regex::new(r"^T\d+xC(\d+)\b").ok()?;
    let caps = re.captures(permatitle)?;
    let n: i32 = caps[1].parse().ok()?;
    if n <= 0 {
        return None;
    }
    Some(n)
}

#[instrument(skip(http_client))]
pub(crate) async fn get_episodes<T>(http_client: &Arc<T>, tv_show_id: i32) -> Result<Vec<MediaItem>>
where
    T: HttpClientTrait,
{
    let mut episodes: Vec<MediaItem> = vec![];

    let url = TV3_EPISODE_LIST_URL.replace("{tv_show_id}", &tv_show_id.to_string());

    let tv3_tv_show_api_response = http_client
        .get::<api_structs::EpisodesRoot, api_structs::Tv3Error>(&url, None)
        .await
        .map_err(|e| Error::Decoding {
            context: format!("episode list url={url}"),
            source: Box::new(e),
        })?;

    let episode_list = tv3_tv_show_api_response.response.items.item;
    if episode_list.is_empty() {
        return Ok(episodes);
    }

    for item in episode_list {
        // Skip items missing the minimum data needed to build a MediaItem.
        // The 3cat API returns heterogeneous items (e.g., trailers lacking
        // `programa` or `permatitle`); these would otherwise produce invalid
        // MediaItems downstream.
        let (Some(id), Some(perma)) = (item.id, item.permatitle.as_ref()) else {
            continue;
        };

        let title = match item.title {
            Some(t) if !t.is_empty() => t,
            _ => perma.clone(),
        };

        // Episode number resolution: prefer the `capitol` field when the API
        // returns a positive value. Fall back to parsing the `T1xC###` prefix
        // in the permatitle, which is the canonical scheme used by the 3cat
        // website. The current API on the `tira` endpoint omits `capitol` and
        // returns `capitol_temporada = -1`, so the permatitle fallback is the
        // primary source of truth today.
        let episode_number = item
            .number_of_episode
            .filter(|n| *n > 0)
            .or_else(|| extract_episode_number_from_permatitle(perma));

        episodes.push(MediaItem {
            id,
            title,
            video_url: None,
            subtitle_url: None,
            episode_number,
            tv_show_name: item.tv_show_name,
            season: None,
            subtitle_failed: false,
            plex: Some(crate::models::PlexEpisodeMetadata {
                season: 1,
                episode: episode_number.unwrap_or(id),
                tvdb_episode_id: None,
                plot: item
                    .plot
                    .filter(|v| !v.trim().is_empty())
                    .or(item.promo_plot),
                aired: item.aired.as_deref().and_then(crate::plex::parse_3cat_date),
                duration: item.duration,
            }),
        });
    }

    Ok(episodes)
}

#[cfg(test)]
mod tests {
    use super::extract_episode_number_from_permatitle;

    #[test]
    fn test_should_extract_episode_number_from_standard_permatitle() {
        assert_eq!(
            extract_episode_number_from_permatitle("T1xC679 - La pilota és meva"),
            Some(679)
        );
        assert_eq!(
            extract_episode_number_from_permatitle("T1xC7 - Veureu una cosa"),
            Some(7)
        );
        assert_eq!(
            extract_episode_number_from_permatitle("T2xC12 - Episode"),
            Some(12)
        );
    }

    #[test]
    fn test_should_return_none_when_permatitle_lacks_t_prefix() {
        assert_eq!(
            extract_episode_number_from_permatitle("El Mic fa de ratolí"),
            None
        );
        assert_eq!(extract_episode_number_from_permatitle("T1xC"), None);
        assert_eq!(extract_episode_number_from_permatitle(""), None);
    }
}
