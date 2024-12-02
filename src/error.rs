#![allow(dead_code)]
#![allow(clippy::enum_variant_names)]

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    TvShowIdRetrievalError(String),
    DecodingError(String),
    EpisodeDoNotHaveVideoUrl(String),
    DownloadingError(String),
}

// region: -- Error Boilerplate

impl core::fmt::Display for Error {
    fn fmt(&self, fmt: &mut core::fmt::Formatter) -> std::result::Result<(), std::fmt::Error> {
        write!(fmt, "{self:?}")
    }
}

impl std::error::Error for Error {}
// endregion: -- Error Boilerplate
