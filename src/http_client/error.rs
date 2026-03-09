//! HTTP client error types.

use serde::de::DeserializeOwned;
use std::fmt::{Debug, Display};

/// Errors that can occur during HTTP client operations.
#[derive(Debug, thiserror::Error)]
pub enum Error<S: DeserializeOwned + Debug + Display> {
    /// The remote server returned an error response.
    #[error("request error: {0}")]
    Request(S),

    /// Failed to read the response body.
    #[error("request body read error: {0}")]
    RequestBodyRead(#[source] reqwest::Error),
}

impl<S> From<reqwest::Error> for Error<S>
where
    S: DeserializeOwned + Debug + Display,
{
    fn from(ex: reqwest::Error) -> Self {
        Error::RequestBodyRead(ex)
    }
}
