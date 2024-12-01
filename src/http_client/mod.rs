use async_trait::async_trait;
use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use std::fmt::{Debug, Display};
use std::sync::{Arc, OnceLock};

mod error;
pub use error::Error;
pub mod mock;

#[derive(Clone, Debug)]
pub struct HttpClient {
    client: Client,
}

#[async_trait]
pub trait HttpClientTrait {
    fn new() -> Self;
    async fn get<T, S>(
        &self,
        url: &str,
        headers: Option<(reqwest::header::HeaderName, &str)>,
    ) -> Result<T, Error<S>>
    where
        T: DeserializeOwned,
        S: DeserializeOwned + Debug + Display;
    async fn format_response<T, S>(&self, response: Response) -> Result<T, Error<S>>
    where
        T: DeserializeOwned,
        S: DeserializeOwned + Debug + Display;
}

pub fn http_client() -> Arc<HttpClient> {
    static INSTANCE: OnceLock<Arc<HttpClient>> = OnceLock::new();
    INSTANCE.get_or_init(|| Arc::new(HttpClient::new())).clone()
}

#[async_trait]
impl HttpClientTrait for HttpClient {
    fn new() -> Self {
        HttpClient {
            client: Client::new(),
        }
    }

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

    async fn format_response<T, S>(&self, response: Response) -> Result<T, Error<S>>
    where
        T: DeserializeOwned,
        S: DeserializeOwned + Debug + Display,
    {
        if !response.status().is_success() {
            let response = response.json::<S>().await?;
            return Err(Error::RequestError(response));
        }

        let response = response
            .json::<T>()
            .await
            .map_err(Error::RequestBodyReadError)?;
        Ok(response)
    }
}
