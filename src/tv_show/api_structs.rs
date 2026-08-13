//! Serde deserialization structs for the 3cat TV show episode list API.

use serde::Deserialize;

pub use crate::api_structs::Tv3Error;

/// Root wrapper for the episode list response.
///
/// Live shape:
/// `{"resposta": {"status": "OK", "items": {"num": 339, "item": [...]}, "paginacio": {...}}}`
#[derive(Debug, Deserialize)]
pub struct EpisodesRoot {
    /// The main response payload.
    #[serde(rename(deserialize = "resposta"))]
    pub response: MainResponse,
}

/// Outer response containing the items collection and optional metadata.
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // status, paginacio captured for forward-compat but not consumed yet
pub struct MainResponse {
    /// Server-reported status (`"OK"` on success). Optional for forward-compatibility.
    #[serde(default)]
    pub status: Option<String>,
    /// Collection of episode items.
    pub items: Items,
    /// Pagination metadata. Captured as raw JSON because the schema may evolve.
    #[serde(default)]
    pub paginacio: Option<serde_json::Value>,
}

/// Wrapper around the episode item list.
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // `num` captured for forward-compat but not consumed yet
pub struct Items {
    /// Total number of items reported by the API (optional; some pages omit it).
    #[serde(default)]
    pub num: Option<i32>,
    /// Individual episode entries. Heterogeneous: some entries (trailers, specials)
    /// omit `programa`, `titol_promo`, `entradeta_promo`, etc.
    pub item: Vec<Tv3Episode>,
}

/// A single episode as returned by the 3cat episode list API.
///
/// **All consumed fields are `Option<T>` with `#[serde(default)]`** because the
/// live API emits heterogeneous items in the same array (e.g., a 339-item list
/// where some entries lack `programa`, triggering `missing field 'programa'`).
#[derive(Debug, Deserialize)]
pub struct Tv3Episode {
    /// Internal 3cat episode ID.
    #[serde(default)]
    pub id: Option<i32>,
    /// Sequential episode number within the show (sometimes `-1` for specials).
    #[serde(rename = "capitol", default)]
    pub number_of_episode: Option<i32>,
    /// Permanent URL-friendly title.
    #[serde(default)]
    pub permatitle: Option<String>,
    /// Human-readable title (often present; sometimes empty or absent).
    #[serde(rename = "titol", default)]
    pub title: Option<String>,
    /// Name of the TV show this episode belongs to.
    /// This was the source of the reported `missing field 'programa'` error.
    #[serde(rename = "programa", default)]
    pub tv_show_name: Option<String>,
    /// Full 3Cat synopsis.
    #[serde(rename = "entradeta", default)]
    pub plot: Option<String>,
    /// Short promotional synopsis.
    #[serde(rename = "entradeta_promo", default)]
    pub promo_plot: Option<String>,
    /// Broadcast timestamp in `DD/MM/YYYY HH:MM:SS` form.
    #[serde(rename = "data_emissio", default)]
    pub aired: Option<String>,
    /// Source duration.
    #[serde(rename = "durada", default)]
    pub duration: Option<String>,
}
