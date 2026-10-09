//! `Github`: one repository, and the HTTP headers sent with every request to it (an `Authorization`
//! header makes private repositories readable and uploads possible). It hands out the executables
//! store and the index stores for files in that repository.

use std::time::Duration;

use crate::headers::ACCEPT;
use crate::headers::AUTHORIZATION;
use crate::headers::HeaderMap;
use crate::headers::HeaderValue;

use crate::github::GITHUB;
use crate::github::GITHUB_API;
use crate::github::GITHUB_RAW;
use crate::github::GithubError;
use crate::github::address::FileAddress;
use crate::github::address::RepoAddress;
use crate::github::files::GithubFile;
use crate::github::releases::GithubReleaseFile;
use crate::github::releases::GithubReleases;
use crate::transport::Http;

#[derive(Clone, Debug)]
pub struct Github {
    repo: RepoAddress,
    pub(crate) client: Http,
    pub(crate) headers: HeaderMap,
    pub(crate) prefix: String,
    web: String,
    api: String,
    raw: String,
}

impl Github {
    /// `owner/repo`, read anonymously until headers carry an `Authorization`.
    pub fn new(repo: &str) -> Result<Self, GithubError> {
        Ok(Self {
            repo: repo.parse()?,
            client: Http::new(),
            headers: HeaderMap::new(),
            prefix: String::new(),
            web: GITHUB.to_owned(),
            api: GITHUB_API.to_owned(),
            raw: GITHUB_RAW.to_owned(),
        })
    }

    /// Sent with every request, e.g. `Authorization: Bearer <token>`. With an `Authorization`
    /// header, release assets are read through the API, and uploads and commits become
    /// possible.
    pub fn headers(self, headers: HeaderMap) -> Self {
        Self { headers, ..self }
    }

    /// Put in front of every executable's file name, e.g. `acme-`.
    pub fn prefix(self, prefix: &str) -> Self {
        Self {
            prefix: prefix.to_owned(),
            ..self
        }
    }

    /// How long one request may take, from connecting to the last byte. 300 seconds by default.
    pub fn timeout(self, timeout: Duration) -> Self {
        Self {
            client: self.client.with_timeout(timeout),
            ..self
        }
    }

    /// GitHub Enterprise: replace the github.com web, API and raw roots.
    pub fn enterprise(self, web: &str, api: &str, raw: &str) -> Self {
        Self {
            web: web.to_owned(),
            api: api.to_owned(),
            raw: raw.to_owned(),
            ..self
        }
    }

    /// The executables, kept as release assets.
    pub fn releases(&self) -> GithubReleases {
        GithubReleases::new(self.clone())
    }

    /// An index kept as a release asset: `<tag>/<asset name>`.
    pub fn release_file(&self, address: &str) -> Result<GithubReleaseFile, GithubError> {
        let address: FileAddress = address.parse()?;
        Ok(GithubReleaseFile::new(
            self.releases(),
            &address.reference,
            &address.path,
        ))
    }

    /// An index committed to the repository: `<branch, tag or commit>/<path>`.
    pub fn file(&self, address: &str) -> Result<GithubFile, GithubError> {
        Ok(GithubFile::new(self.clone(), address.parse()?))
    }

    pub(crate) fn authorised(&self) -> bool {
        self.headers.contains_key(AUTHORIZATION)
    }

    /// The configured headers plus `Accept`.
    pub(crate) fn accepting(&self, accept: &'static str) -> HeaderMap {
        let mut headers = self.headers.clone();
        headers.insert(ACCEPT, HeaderValue::from_static(accept));
        headers
    }

    pub(crate) fn download_root(&self) -> String {
        format!("{}/{}/releases/download", self.web, self.repo)
    }

    pub(crate) fn releases_url(&self) -> String {
        format!("{}/repos/{}/releases", self.api, self.repo)
    }

    pub(crate) fn contents_url(&self) -> String {
        format!("{}/repos/{}/contents", self.api, self.repo)
    }

    pub(crate) fn raw_root(&self, reference: &str) -> String {
        format!("{}/{}/{reference}", self.raw, self.repo)
    }
}
