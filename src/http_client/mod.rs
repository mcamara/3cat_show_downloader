//! HTTP client abstraction with trait-based design for testability.

use std::fmt::{Debug, Display};
use std::sync::{Arc, OnceLock};

use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use tracing::instrument;

mod error;
pub use error::Error;
#[cfg(test)]
pub mod mock;

/// A wrapper around `reqwest::Client`.
#[derive(Clone, Debug)]
pub struct HttpClient {
    client: Client,
}

/// Trait for HTTP client operations, enabling mock implementations in tests.
pub trait HttpClientTrait {
    /// Creates a new instance of the HTTP client.
    fn new() -> Self;

    /// Performs a GET request and deserializes the response.
    fn get<T, S>(
        &self,
        url: &str,
        headers: Option<(reqwest::header::HeaderName, &str)>,
    ) -> impl Future<Output = Result<T, Error<S>>>
    where
        T: DeserializeOwned,
        S: DeserializeOwned + Debug + Display;

    /// Parses an HTTP response into the expected type or an error type.
    fn format_response<T, S>(
        &self,
        response: Response,
    ) -> impl Future<Output = Result<T, Error<S>>>
    where
        T: DeserializeOwned,
        S: DeserializeOwned + Debug + Display;
}

/// Returns a shared singleton `HttpClient` instance.
pub fn http_client() -> Arc<HttpClient> {
    static INSTANCE: OnceLock<Arc<HttpClient>> = OnceLock::new();
    INSTANCE.get_or_init(|| Arc::new(HttpClient::new())).clone()
}

impl HttpClientTrait for HttpClient {
    fn new() -> Self {
        HttpClient {
            client: Client::new(),
        }
    }

    #[instrument(skip(self, headers))]
    async fn get<T, S>(
        &self,
        url: &str,
        headers: Option<(reqwest::header::HeaderName, &str)>,
    ) -> Result<T, Error<S>>
    where
        T: DeserializeOwned,
        S: DeserializeOwned + Debug + Display,
    {
        let mut response = self.client.get(url.to_string());
        if let Some((header_name, header_value)) = headers {
            response = response.header(header_name, header_value);
        }

        self.format_response(response.send().await?).await
    }

    #[instrument(skip_all)]
    async fn format_response<T, S>(&self, response: Response) -> Result<T, Error<S>>
    where
        T: DeserializeOwned,
        S: DeserializeOwned + Debug + Display,
    {
        if !response.status().is_success() {
            let response = response.json::<S>().await?;
            return Err(Error::Request(response));
        }

        let response = response.json::<T>().await.map_err(Error::RequestBodyRead)?;
        Ok(response)
    }
}
