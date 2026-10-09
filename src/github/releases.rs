//! A repository's releases. `GithubReleases` keeps executables as release assets: downloaded
//! directly, or through the API when authorised, which also uploads. `GithubReleaseFile` keeps an
//! index as one asset of one release.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;

use reqwest::RequestBuilder;
use reqwest::header::CONTENT_TYPE;
use reqwest::header::LINK;

use crate::github::GithubError;
use crate::github::bytes;
use crate::github::handle::Github;
use crate::github::location;
use crate::github::send;
use crate::github::stream;
use crate::index::store::IndexStore;
use crate::store::Artifact;
use crate::store::ByteStream;
use crate::store::Location;
use crate::store::Store;
use crate::store::StoreWriter;

const JSON: &str = "application/vnd.github+json";
const OCTETS: &str = "application/octet-stream";

#[derive(Clone, Debug)]
pub struct GithubReleases {
    github: Github,
}

/// When authorised: every release's upload URL and every asset's API URL, listed once at bootstrap.
pub struct ReleasesSession {
    authorised: bool,
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

impl GithubReleases {
    pub(crate) fn new(github: Github) -> Self {
        Self { github }
    }

    fn get(
        &self,
        session: &ReleasesSession,
        location: Location,
    ) -> Result<(String, RequestBuilder), GithubError> {
        let client = &self.github.client;
        match session.authorised {
            false => {
                let url = format!("{}/{location}", self.github.download_root());
                Ok((
                    url.clone(),
                    client.get(url).headers(self.github.headers.clone()),
                ))
            }
            true => {
                let url = lock(&session.assets).get(&location).cloned();
                let url = url.ok_or(GithubError::MissingAsset { location })?;
                let request = client.get(&url).headers(self.github.accepting(OCTETS));
                Ok((url, request))
            }
        }
    }

    fn remember(session: &ReleasesSession, release: ApiRelease) -> String {
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

impl Store for GithubReleases {
    type Session = ReleasesSession;
    type Error = GithubError;

    async fn bootstrap(&self) -> Result<ReleasesSession, GithubError> {
        let session = ReleasesSession {
            authorised: self.github.authorised(),
            upload_urls: Mutex::default(),
            assets: Mutex::default(),
        };
        if !session.authorised {
            return Ok(session);
        }
        let mut next = Some(format!("{}?per_page=100", self.github.releases_url()));
        while let Some(url) = next {
            let response = send(
                self.github
                    .client
                    .get(&url)
                    .headers(self.github.accepting(JSON)),
                &url,
            )
            .await?;
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
        session: &ReleasesSession,
        artifact: Artifact<'_>,
    ) -> Result<ByteStream<GithubError>, GithubError> {
        let (url, request) = self.get(session, self.location(artifact))?;
        Ok(stream(send(request, &url).await?, &url))
    }

    fn location(&self, artifact: Artifact<'_>) -> Location {
        location(&self.github.prefix, artifact)
    }
}

impl StoreWriter for GithubReleases {
    async fn upload(
        &self,
        session: &ReleasesSession,
        location: &Location,
        bytes: Vec<u8>,
    ) -> Result<(), GithubError> {
        if !session.authorised {
            return Err(GithubError::ReadOnly {
                place: self.github.download_root(),
            });
        }
        let (client, json) = (&self.github.client, self.github.accepting(JSON));
        let existing = lock(&session.assets).remove(location);
        if let Some(url) = existing {
            send(client.delete(&url).headers(json.clone()), &url).await?;
        }
        let known = lock(&session.upload_urls).get(&location.group).cloned();
        let upload_url = match known {
            Some(upload_url) => upload_url,
            None => {
                let releases_url = self.github.releases_url();
                let body =
                    serde_json::json!({ "tag_name": location.group, "name": location.group });
                let created = send(
                    client.post(&releases_url).headers(json.clone()).json(&body),
                    &releases_url,
                )
                .await?;
                let unreachable = |source| GithubError::Unreachable {
                    url: releases_url.clone(),
                    source,
                };
                Self::remember(session, created.json().await.map_err(unreachable)?)
            }
        };
        let request = client
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

/// An index kept as asset `name` of release `tag`.
#[derive(Clone, Debug)]
pub struct GithubReleaseFile {
    releases: GithubReleases,
    index: Location,
}

impl GithubReleaseFile {
    pub(crate) fn new(releases: GithubReleases, tag: &str, name: &str) -> Self {
        Self {
            releases,
            index: Location {
                group: tag.to_owned(),
                file: name.to_owned(),
            },
        }
    }
}

impl IndexStore for GithubReleaseFile {
    type Error = GithubError;

    async fn read(&self) -> Result<Vec<u8>, GithubError> {
        let session = self.releases.bootstrap().await?;
        let (url, request) = self.releases.get(&session, self.index.clone())?;
        bytes(request, &url).await
    }

    async fn write(&self, index: Vec<u8>) -> Result<String, GithubError> {
        let session = self.releases.bootstrap().await?;
        self.releases.upload(&session, &self.index, index).await?;
        Ok(self.index.to_string())
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
