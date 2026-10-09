//! A GitHub repository's releases through the API with a token — reads private repositories and
//! writes any: bootstrap lists every release and asset once; uploads create missing releases.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;

use reqwest::header::ACCEPT;
use reqwest::header::AUTHORIZATION;
use reqwest::header::CONTENT_TYPE;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;
use reqwest::header::LINK;

use crate::github::GithubError;
use crate::github::location;
use crate::github::send;
use crate::github::stream;
use crate::store::ByteStream;
use crate::store::Location;
use crate::store::Object;
use crate::store::Store;
use crate::store::StoreWriter;

const JSON: &str = "application/vnd.github+json";
const OCTETS: &str = "application/octet-stream";

pub struct GithubApi {
    client: reqwest::Client,
    releases_url: String,
    token_env: String,
    prefix: String,
}

pub struct ApiSession {
    token: HeaderValue,
    upload_urls: Mutex<HashMap<String, String>>,
    assets: Mutex<HashMap<Location, String>>,
}

#[derive(serde::Deserialize)]
struct ApiRelease {
    tag_name: String,
    upload_url: String,
    assets: Vec<ApiAsset>,
}

#[derive(serde::Deserialize)]
struct ApiAsset {
    name: String,
    url: String,
}

impl GithubApi {
    /// `api_url` is `GITHUB_API`, or a GitHub Enterprise API root.
    /// The `client` carries the caller's timeout and proxy settings.
    pub fn new(
        client: reqwest::Client,
        api_url: &str,
        owner: &str,
        repo: &str,
        token_env: &str,
    ) -> Self {
        Self {
            client,
            releases_url: format!("{api_url}/repos/{owner}/{repo}/releases"),
            token_env: token_env.to_owned(),
            prefix: String::new(),
        }
    }

    pub fn prefix(self, prefix: &str) -> Self {
        Self {
            prefix: prefix.to_owned(),
            ..self
        }
    }

    fn headers(token: &HeaderValue, accept: &'static str) -> HeaderMap {
        HeaderMap::from_iter([
            (AUTHORIZATION, token.clone()),
            (ACCEPT, HeaderValue::from_static(accept)),
        ])
    }

    fn remember(session: &ApiSession, release: ApiRelease) -> String {
        let upload_url = release
            .upload_url
            .split('{')
            .next()
            .unwrap_or_default()
            .to_owned();
        lock(&session.upload_urls).insert(release.tag_name.clone(), upload_url.clone());
        let mut assets = lock(&session.assets);
        for asset in release.assets {
            assets.insert(
                Location {
                    group: release.tag_name.clone(),
                    file: asset.name,
                },
                asset.url,
            );
        }
        upload_url
    }
}

impl Store for GithubApi {
    type Session = ApiSession;
    type Error = GithubError;

    async fn bootstrap(&self) -> Result<ApiSession, GithubError> {
        let missing = || GithubError::MissingToken {
            env_var: self.token_env.clone(),
        };
        let token = std::env::var(&self.token_env).ok();
        let token = token
            .and_then(|token| HeaderValue::from_str(&format!("Bearer {token}")).ok())
            .ok_or_else(missing)?;
        let session = ApiSession {
            token,
            upload_urls: Mutex::default(),
            assets: Mutex::default(),
        };
        let mut next = Some(format!("{}?per_page=100", self.releases_url));
        while let Some(url) = next {
            let request = self
                .client
                .get(&url)
                .headers(Self::headers(&session.token, JSON));
            let response = send(request, &url).await?;
            next = response
                .headers()
                .get(LINK)
                .and_then(|link| next_page(link.to_str().ok()?));
            let unreachable = |source| GithubError::Unreachable {
                url: url.clone(),
                source,
            };
            for release in response
                .json::<Vec<ApiRelease>>()
                .await
                .map_err(unreachable)?
            {
                Self::remember(&session, release);
            }
        }
        Ok(session)
    }

    async fn open(
        &self,
        session: &ApiSession,
        object: Object<'_>,
    ) -> Result<ByteStream<GithubError>, GithubError> {
        let location = self.location(object);
        let url = lock(&session.assets).get(&location).cloned();
        let Some(url) = url else {
            return Err(GithubError::MissingAsset { location });
        };
        let response = send(
            self.client
                .get(&url)
                .headers(Self::headers(&session.token, OCTETS)),
            &url,
        )
        .await?;
        Ok(stream(response, &url))
    }

    fn location(&self, object: Object<'_>) -> Location {
        location(&self.prefix, object)
    }
}

impl StoreWriter for GithubApi {
    async fn upload(
        &self,
        session: &ApiSession,
        location: &Location,
        bytes: Vec<u8>,
    ) -> Result<(), GithubError> {
        let json = Self::headers(&session.token, JSON);
        let existing = lock(&session.assets).remove(location);
        if let Some(url) = existing {
            send(self.client.delete(&url).headers(json.clone()), &url).await?;
        }
        let known = lock(&session.upload_urls).get(&location.group).cloned();
        let upload_url = match known {
            Some(upload_url) => upload_url,
            None => {
                let body =
                    serde_json::json!({ "tag_name": location.group, "name": location.group });
                let request = self
                    .client
                    .post(&self.releases_url)
                    .headers(json.clone())
                    .json(&body);
                let created = send(request, &self.releases_url).await?;
                let unreachable = |source| GithubError::Unreachable {
                    url: self.releases_url.clone(),
                    source,
                };
                Self::remember(session, created.json().await.map_err(unreachable)?)
            }
        };
        let request = self
            .client
            .post(&upload_url)
            .headers(json)
            .header(CONTENT_TYPE, OCTETS)
            .query(&[("name", &location.file)]);
        let uploaded = send(request.body(bytes), &upload_url).await?;
        let unreachable = |source| GithubError::Unreachable {
            url: upload_url.clone(),
            source,
        };
        let asset: ApiAsset = uploaded.json().await.map_err(unreachable)?;
        lock(&session.assets).insert(location.clone(), asset.url);
        Ok(())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn next_page(link: &str) -> Option<String> {
    link.split(',')
        .find(|part| part.contains("rel=\"next\""))
        .and_then(|part| part.split(';').next())
        .map(|url| {
            url.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_owned()
        })
}
