use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use async_trait::async_trait;
use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;

use super::{Error, HttpClientTrait};

#[derive(Clone)]
pub struct MockHttpClient {
    _client: Client,
    responses: Arc<Mutex<Vec<String>>>,
}

impl MockHttpClient {
    pub fn new(responses: Vec<&str>) -> Self {
        let responses = responses.into_iter().map(|s| s.to_string()).collect();
        MockHttpClient {
            _client: Client::new(),
            responses: Mutex::new(responses).into(),
        }
    }
}

#[async_trait]
impl HttpClientTrait for MockHttpClient {
    fn new() -> Self {
        MockHttpClient::new(vec![])
    }

    async fn get<T, S>(
        &self,
        _url: &str,
        _headers: Option<(reqwest::header::HeaderName, &str)>,
    ) -> Result<T, Error<S>>
    where
        T: DeserializeOwned,
        S: DeserializeOwned + Debug + Display,
    {
        let mut responses = self.responses.lock().await;
        let response = responses.pop().unwrap_or_default();
        let parsed_response: T = serde_json::from_str(&response).unwrap();
        Ok(parsed_response)
    }

    async fn format_response<T, S>(&self, _response: Response) -> Result<T, Error<S>>
    where
        T: DeserializeOwned,
        S: DeserializeOwned + Debug + Display,
    {
        unreachable!()
    }
}
