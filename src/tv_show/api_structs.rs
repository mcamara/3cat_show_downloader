//! Serde deserialization structs for the 3cat TV show episode list API.

use serde::Deserialize;

pub use crate::api_structs::Tv3Error;

/// Root wrapper for the episode list response.
#[derive(Debug, Deserialize)]
pub struct EpisodesRoot {
    /// The main response payload.
    #[serde(rename(deserialize = "resposta"))]
    pub response: MainResponse,
}

/// Outer response containing the items collection.
#[derive(Debug, Deserialize)]
pub struct MainResponse {
    /// Collection of episode items.
    pub items: Items,
}

/// Wrapper around the episode item list.
#[derive(Debug, Deserialize)]
pub struct Items {
    /// Individual episode entries.
    pub item: Vec<Tv3Episode>,
}

/// A single episode as returned by the 3cat episode list API.
#[derive(Debug, Deserialize)]
pub struct Tv3Episode {
    /// Internal 3cat episode ID.
    pub id: i32,
    /// Sequential episode number within the show.
    #[serde(rename = "capitol")]
    pub number_of_episode: i32,
    /// Permanent URL-friendly title.
    #[serde(rename = "permatitle")]
    pub permatitle: String,
    /// Human-readable title (may be absent or empty).
    #[serde(rename = "titol")]
    pub title: Option<String>,
    /// Name of the TV show this episode belongs to.
    #[serde(rename = "programa")]
    pub tv_show_name: String,
}
