//! Index stores outside releases: a file committed to a GitHub repository (`GithubFile`, written
//! through the contents API), and a file served at any URL (`HttpFile`, read-only).

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use reqwest::RequestBuilder;
use reqwest::header::ACCEPT;
use reqwest::header::AUTHORIZATION;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;

use crate::github::GithubError;
use crate::github::address::FileAddress;
use crate::github::bearer;
use crate::github::bytes;
use crate::github::handle::Github;
use crate::github::optional;
use crate::github::send;
use crate::index::Signed;
use crate::index::store::IndexStore;

/// An index committed at `<ref>/<path>`, next to `<path>.minisig`. Reads go to the raw
/// file, or through the contents API with the repository's token; commits always need a token (the
/// repository's, else `GITHUB_TOKEN`).
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

    fn headers(token: HeaderValue, accept: &'static str) -> HeaderMap {
        HeaderMap::from_iter([
            (AUTHORIZATION, token),
            (ACCEPT, HeaderValue::from_static(accept)),
        ])
    }

    fn get(&self, token: &Option<HeaderValue>, path: &str) -> (String, RequestBuilder) {
        let client = &self.github.client;
        match token {
            Some(token) => {
                let url = format!(
                    "{}/{path}?ref={}",
                    self.github.contents_url(),
                    self.address.reference
                );
                let request = client
                    .get(&url)
                    .headers(Self::headers(token.clone(), "application/vnd.github.raw"));
                (url, request)
            }
            None => {
                let url = format!("{}/{path}", self.github.raw_root(&self.address.reference));
                (url.clone(), client.get(url))
            }
        }
    }
}

impl IndexStore for GithubFile {
    type Error = GithubError;

    async fn read(&self) -> Result<Signed, GithubError> {
        let token = self.github.token()?;
        let (url, request) = self.get(&token, &self.address.path);
        let index = bytes(request, &url).await?;
        let (url, request) = self.get(&token, &format!("{}.minisig", self.address.path));
        let signature = optional(bytes(request, &url).await)?;
        Ok(Signed { index, signature })
    }

    async fn write(&self, signed: Signed) -> Result<Vec<String>, GithubError> {
        let token = bearer(self.github.token_env.as_deref().unwrap_or("GITHUB_TOKEN"))?;
        let json = Self::headers(token, "application/vnd.github+json");
        let client = &self.github.client;
        let mut written = Vec::new();
        for (path, contents) in [
            (self.address.path.clone(), signed.index),
            (
                format!("{}.minisig", self.address.path),
                signed.signature.unwrap_or_default(),
            ),
        ] {
            let url = format!("{}/{path}", self.github.contents_url());
            let lookup = client
                .get(format!("{url}?ref={}", self.address.reference))
                .headers(json.clone());
            let existing = optional(bytes(lookup, &url).await)?
                .and_then(|found| serde_json::from_slice::<Existing>(&found).ok());
            let body = serde_json::json!({
                "message": format!("museum: update {path}"),
                "content": STANDARD.encode(contents),
                "branch": self.address.reference,
                "sha": existing.map(|existing| existing.sha),
            });
            send(client.put(&url).headers(json.clone()).json(&body), &url).await?;
            written.push(format!("{}:{path}", self.address.reference));
        }
        Ok(written)
    }
}

/// An index served at `url`, e.g. from a CDN, next to `<url>.minisig`. Read-only.
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

    async fn read(&self) -> Result<Signed, GithubError> {
        let index = bytes(self.client.get(&self.url), &self.url).await?;
        let signature = format!("{}.minisig", self.url);
        let signature = optional(bytes(self.client.get(&signature), &signature).await)?;
        Ok(Signed { index, signature })
    }

    async fn write(&self, _: Signed) -> Result<Vec<String>, GithubError> {
        Err(GithubError::ReadOnly {
            place: self.url.clone(),
        })
    }
}
