//! HTTP client error types.

use std::fmt::{Debug, Display};

use serde::de::DeserializeOwned;

/// Errors that can occur during HTTP client operations.
#[derive(Debug, thiserror::Error)]
pub enum Error<S: DeserializeOwned + Debug + Display> {
    /// The remote server returned an error response.
    #[error("request error: {0}")]
    Request(S),

    /// Failed to read the response body.
    #[error("request body read error: {0}")]
    RequestBodyRead(#[source] reqwest::Error),

    /// Response body could not be decoded into the expected JSON schema.
    /// Carries status, body length and a UTF-8 lossy preview for diagnosis.
    #[error(
        "response body decode error: status={status}, content_length={content_length}, preview={preview:?}: {source}"
    )]
    DecodeBody {
        /// HTTP status code of the response.
        status: reqwest::StatusCode,
        /// Length of the response body in bytes.
        content_length: usize,
        /// First ≤512 bytes of the body as UTF-8 lossy (may contain replacement chars).
        preview: String,
        /// Underlying serde_json parse error.
        #[source]
        source: serde_json::Error,
    },
}

impl<S> From<reqwest::Error> for Error<S>
where
    S: DeserializeOwned + Debug + Display,
{
    fn from(ex: reqwest::Error) -> Self {
        Error::RequestBodyRead(ex)
    }
}
