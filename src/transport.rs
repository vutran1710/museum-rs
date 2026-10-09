//! HTTP for every built-in store (GitHub today, others later): the implicit client with its
//! per-request timeout, sending and streaming with typed errors, and `HttpFile`, an index read from
//! any URL.

use std::time::Duration;

use futures_util::StreamExt;
use futures_util::TryStreamExt;
use reqwest::IntoUrl;
use reqwest::RequestBuilder;
use reqwest::Response;

use crate::headers::USER_AGENT;
use crate::index::IndexError;
use crate::index::store::IndexStore;
use crate::store::ByteStream;

const AGENT: &str = concat!("museum/", env!("CARGO_PKG_VERSION"));
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("could not reach {url}: {source}")]
    Unreachable { url: String, source: reqwest::Error },
    #[error("{url} answered {status}")]
    Refused { status: u16, url: String },
    #[error("{place} is read-only")]
    ReadOnly { place: String },
}

impl HttpError {
    pub fn unauthorized(&self) -> bool {
        matches!(
            self,
            Self::Refused {
                status: 401 | 403,
                ..
            }
        )
    }
}

impl IndexError for HttpError {
    fn not_found(&self) -> bool {
        matches!(self, Self::Refused { status: 404, .. })
    }
}

/// The client a store uses, created implicitly; every request carries the timeout.
#[derive(Clone, Debug)]
pub(crate) struct Http {
    client: reqwest::Client,
    timeout: Duration,
}

impl Http {
    pub(crate) fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub(crate) fn with_timeout(self, timeout: Duration) -> Self {
        Self { timeout, ..self }
    }

    pub(crate) fn get(&self, url: impl IntoUrl) -> RequestBuilder {
        self.client.get(url).timeout(self.timeout)
    }

    #[cfg_attr(not(any(feature = "github", feature = "gitlab")), allow(dead_code))]
    pub(crate) fn post(&self, url: impl IntoUrl) -> RequestBuilder {
        self.client.post(url).timeout(self.timeout)
    }

    #[cfg_attr(not(any(feature = "github", feature = "gitlab")), allow(dead_code))]
    pub(crate) fn put(&self, url: impl IntoUrl) -> RequestBuilder {
        self.client.put(url).timeout(self.timeout)
    }

    #[cfg_attr(not(feature = "github"), allow(dead_code))]
    pub(crate) fn delete(&self, url: impl IntoUrl) -> RequestBuilder {
        self.client.delete(url).timeout(self.timeout)
    }
}

pub(crate) async fn send(request: RequestBuilder, url: &str) -> Result<Response, HttpError> {
    let unreachable = |source| HttpError::Unreachable {
        url: url.to_owned(),
        source,
    };
    let response = request
        .header(USER_AGENT, AGENT)
        .send()
        .await
        .map_err(unreachable)?;
    match response.status() {
        status if status.is_success() => Ok(response),
        status => Err(HttpError::Refused {
            status: status.as_u16(),
            url: url.to_owned(),
        }),
    }
}

pub(crate) fn stream(response: Response, url: &str) -> ByteStream<HttpError> {
    let url = url.to_owned();
    Box::pin(response.bytes_stream().map(move |chunk| {
        chunk.map_err(|source| HttpError::Unreachable {
            url: url.clone(),
            source,
        })
    }))
}

pub(crate) async fn bytes(request: RequestBuilder, url: &str) -> Result<Vec<u8>, HttpError> {
    let chunks = stream(send(request, url).await?, url);
    chunks
        .try_fold(Vec::new(), |mut all, chunk| async move {
            all.extend_from_slice(&chunk);
            Ok(all)
        })
        .await
}

/// A file that does not exist is `None`; any other failure stays an error.
#[cfg_attr(not(any(feature = "github", feature = "gitlab")), allow(dead_code))]
pub(crate) fn optional(read: Result<Vec<u8>, HttpError>) -> Result<Option<Vec<u8>>, HttpError> {
    match read {
        Err(e) if e.not_found() => Ok(None),
        read => read.map(Some),
    }
}

/// An index served at `url`, e.g. from a CDN. Read-only.
#[derive(Clone, Debug)]
pub struct HttpFile {
    client: Http,
    url: String,
}

impl HttpFile {
    pub fn new(url: &str) -> Self {
        Self {
            client: Http::new(),
            url: url.to_owned(),
        }
    }

    /// How long one request may take. 300 seconds by default.
    pub fn timeout(self, timeout: Duration) -> Self {
        Self {
            client: self.client.with_timeout(timeout),
            ..self
        }
    }
}

impl IndexStore for HttpFile {
    type Error = HttpError;

    async fn read(&self) -> Result<Vec<u8>, HttpError> {
        bytes(self.client.get(&self.url), &self.url).await
    }

    async fn write(&self, _: Vec<u8>) -> Result<String, HttpError> {
        Err(HttpError::ReadOnly {
            place: self.url.clone(),
        })
    }
}
