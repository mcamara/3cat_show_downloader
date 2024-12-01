#![allow(dead_code)]

use serde::de::DeserializeOwned;
use std::fmt::{Debug, Display};

#[derive(Debug)]
pub enum Error<S: DeserializeOwned + Debug + Display> {
    RequestError(S),
    RequestBodyReadError(reqwest::Error),
}

// region: --- Froms
impl<S> From<reqwest::Error> for Error<S>
where
    S: DeserializeOwned + Debug + Display,
{
    fn from(ex: reqwest::Error) -> Self {
        Error::RequestBodyReadError(ex)
    }
}
// endregion: --- Froms

// region: -- Error Boilerplate

impl<S> core::fmt::Display for Error<S>
where
    S: DeserializeOwned + Debug + Display,
{
    fn fmt(&self, fmt: &mut core::fmt::Formatter) -> std::result::Result<(), std::fmt::Error> {
        write!(fmt, "{self:?}")
    }
}

impl<S> std::error::Error for Error<S> where S: DeserializeOwned + Debug + Display {}
// endregion: -- Error Boilerplate
