//! Index stores outside releases: a file committed to a GitHub repository (`GithubFile`, written
//! through the contents API), and a file served at any URL (`HttpFile`, read-only).

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::github::GithubError;
use crate::github::address::FileAddress;
use crate::github::bytes;
use crate::github::handle::Github;
use crate::github::optional;
use crate::github::send;
use crate::index::store::IndexStore;

/// An index committed at `<ref>/<path>`. Reads go to the raw file, or through the contents API when
/// the handle is authorised; commits need an `Authorization` header.
#[derive(Clone, Debug)]
pub struct GithubFile {
    github: Github,
    address: FileAddress,
}

#[derive(serde::Deserialize)]
struct Existing {
    sha: String,
}

impl GithubFile {
    pub(crate) fn new(github: Github, address: FileAddress) -> Self {
        Self { github, address }
    }
}

impl IndexStore for GithubFile {
    type Error = GithubError;

    async fn read(&self) -> Result<Vec<u8>, GithubError> {
        let FileAddress { reference, path } = &self.address;
        let client = &self.github.client;
        let (url, request) = match self.github.authorised() {
            true => {
                let url = format!("{}/{path}?ref={reference}", self.github.contents_url());
                let request = client
                    .get(&url)
                    .headers(self.github.accepting("application/vnd.github.raw"));
                (url, request)
            }
            false => {
                let url = format!("{}/{path}", self.github.raw_root(reference));
                (
                    url.clone(),
                    client.get(url).headers(self.github.headers.clone()),
                )
            }
        };
        bytes(request, &url).await
    }

    async fn write(&self, index: Vec<u8>) -> Result<String, GithubError> {
        let FileAddress { reference, path } = &self.address;
        if !self.github.authorised() {
            return Err(GithubError::ReadOnly {
                place: format!("{reference}:{path}"),
            });
        }
        let json = self.github.accepting("application/vnd.github+json");
        let client = &self.github.client;
        let url = format!("{}/{path}", self.github.contents_url());
        let lookup = client
            .get(format!("{url}?ref={reference}"))
            .headers(json.clone());
        let existing = optional(bytes(lookup, &url).await)?
            .and_then(|found| serde_json::from_slice::<Existing>(&found).ok());
        let body = serde_json::json!({
            "message": format!("museum: update {path}"),
            "content": STANDARD.encode(index),
            "branch": reference,
            "sha": existing.map(|existing| existing.sha),
        });
        send(client.put(&url).headers(json).json(&body), &url).await?;
        Ok(format!("{reference}:{path}"))
    }
}

/// An index served at `url`, e.g. from a CDN. Read-only.
#[derive(Clone, Debug)]
pub struct HttpFile {
    client: reqwest::Client,
    url: String,
}

impl HttpFile {
    pub fn new(client: reqwest::Client, url: &str) -> Self {
        Self {
            client,
            url: url.to_owned(),
        }
    }
}

impl IndexStore for HttpFile {
    type Error = GithubError;

    async fn read(&self) -> Result<Vec<u8>, GithubError> {
        bytes(self.client.get(&self.url), &self.url).await
    }

    async fn write(&self, _: Vec<u8>) -> Result<String, GithubError> {
        Err(GithubError::ReadOnly {
            place: self.url.clone(),
        })
    }
}
