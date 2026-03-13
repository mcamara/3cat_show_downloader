//! Resolves a 3cat slug into a [`MediaType`] containing the internal ID.
//!
//! This module first attempts to resolve the slug as a TV show. If that
//! fails, it falls back to resolving it as a movie.

use tracing::{info, instrument};

use crate::error::{Error, Result};
use crate::movie;
use crate::tv_show;

/// The type of media identified by the resolver.
#[derive(Debug)]
pub enum MediaType {
    /// A TV show, carrying its internal `programatv_id`.
    TvShow(i32),
    /// A movie, carrying its internal ID and URL slug.
    Movie {
        /// Internal 3cat movie ID.
        id: i32,
        /// URL-friendly slug used to construct the movie page URL.
        slug: String,
    },
}

/// Resolves the given slug into a [`MediaType`] by first trying a TV show
/// lookup and falling back to a movie lookup.
#[instrument]
pub async fn get_media_id(slug: &str) -> Result<MediaType> {
    match tv_show::get_tv_show_id(slug).await {
        Ok(id) => {
            info!("Resolved slug as TV show (id={id})");
            Ok(MediaType::TvShow(id))
        }
        Err(_) => match movie::get_movie_id(slug).await {
            Ok(id) => {
                info!("Resolved slug as movie (id={id})");
                Ok(MediaType::Movie {
                    id,
                    slug: slug.to_owned(),
                })
            }
            Err(_) => Err(Error::MediaIdRetrieval(format!(
                "Could not resolve '{slug}' as a TV show or a movie"
            ))),
        },
    }
}
