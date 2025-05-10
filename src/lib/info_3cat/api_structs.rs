use serde::Deserialize;
use std::fmt::Display;

// Developer note: 3cat API is a mess, a lot of data is nested in different places

#[derive(Debug, Deserialize)]
pub struct Tv3Error {}
impl Display for Tv3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Error")
    }
}

#[derive(Debug, Deserialize)]
pub struct EpisodesRoot {
    #[serde(rename(deserialize = "resposta"))]
    pub response: MainResponse,
}
#[derive(Debug, Deserialize)]
pub struct MainResponse {
    pub items: Items,
}

#[derive(Debug, Deserialize)]
pub struct Items {
    pub item: Vec<Tv3Episode>,
}

#[derive(Debug, Deserialize)]
pub struct Tv3Episode {
    pub id: i32,
    #[serde(rename = "capitol")]
    pub number_of_episode: i32,
    #[serde(rename = "permatitle")]
    pub title: String,
    #[serde(rename = "programa")]
    pub tv_show_name: String,
}
