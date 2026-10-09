//! `Github`: one repository and how it is reached, anonymously or with a token. It hands out the
//! executables store and the index stores for files in that repository.

use reqwest::header::HeaderValue;

use crate::github::GITHUB;
use crate::github::GITHUB_API;
use crate::github::GITHUB_RAW;
use crate::github::GithubError;
use crate::github::address::FileAddress;
use crate::github::address::RepoAddress;
use crate::github::bearer;
use crate::github::files::GithubFile;
use crate::github::releases::GithubReleaseFile;
use crate::github::releases::GithubReleases;

#[derive(Clone, Debug)]
pub struct Github {
    repo: RepoAddress,
    pub(crate) client: reqwest::Client,
    pub(crate) token_env: Option<String>,
    pub(crate) prefix: String,
    web: String,
    api: String,
    raw: String,
}

impl Github {
    /// `owner/repo`, read anonymously: downloads from `releases/download` and raw files.
    pub fn public(repo: &str) -> Result<Self, GithubError> {
        Ok(Self {
            repo: repo.parse()?,
            client: reqwest::Client::new(),
            token_env: None,
            prefix: String::new(),
            web: GITHUB.to_owned(),
            api: GITHUB_API.to_owned(),
            raw: GITHUB_RAW.to_owned(),
        })
    }

    /// `owner/repo`, read and written through the API with the token in `token_env`.
    pub fn private(repo: &str, token_env: &str) -> Result<Self, GithubError> {
        Ok(Self {
            token_env: Some(token_env.to_owned()),
            ..Self::public(repo)?
        })
    }

    /// Put in front of every executable's file name, e.g. `acme-`.
    pub fn prefix(self, prefix: &str) -> Self {
        Self {
            prefix: prefix.to_owned(),
            ..self
        }
    }

    /// Replace the default client, e.g. to set a timeout or a proxy.
    pub fn client(self, client: reqwest::Client) -> Self {
        Self { client, ..self }
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

    pub(crate) fn token(&self) -> Result<Option<HeaderValue>, GithubError> {
        self.token_env.as_deref().map(bearer).transpose()
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
