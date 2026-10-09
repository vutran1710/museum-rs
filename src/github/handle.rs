//! `Github`: one repository and how it is reached, anonymously or with a token. It hands out the
//! executables store and the index stores for that repository.

use reqwest::header::HeaderValue;

use crate::github::GITHUB;
use crate::github::GITHUB_API;
use crate::github::GITHUB_RAW;
use crate::github::GithubError;
use crate::github::bearer;
use crate::github::files::GithubFile;
use crate::github::releases::GithubReleaseFile;
use crate::github::releases::GithubReleases;

#[derive(Clone, Debug)]
pub struct Github {
    pub(crate) client: reqwest::Client,
    pub(crate) token_env: Option<String>,
    pub(crate) prefix: String,
    owner: String,
    repo: String,
    web: String,
    api: String,
    raw: String,
}

impl Github {
    /// Read anonymously: downloads from `releases/download` and raw files.
    pub fn public(owner: &str, repo: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            token_env: None,
            prefix: String::new(),
            owner: owner.to_owned(),
            repo: repo.to_owned(),
            web: GITHUB.to_owned(),
            api: GITHUB_API.to_owned(),
            raw: GITHUB_RAW.to_owned(),
        }
    }

    /// Read and write through the API with the token in `token_env`.
    pub fn private(owner: &str, repo: &str, token_env: &str) -> Self {
        Self {
            token_env: Some(token_env.to_owned()),
            ..Self::public(owner, repo)
        }
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

    /// An index kept as asset `name` of release `tag`.
    pub fn release_file(&self, tag: &str, name: &str) -> GithubReleaseFile {
        GithubReleaseFile::new(self.releases(), tag, name)
    }

    /// An index committed at `path` on `branch`.
    pub fn file(&self, branch: &str, path: &str) -> GithubFile {
        GithubFile::new(self.clone(), branch, path)
    }

    pub(crate) fn token(&self) -> Result<Option<HeaderValue>, GithubError> {
        self.token_env.as_deref().map(bearer).transpose()
    }

    pub(crate) fn download_root(&self) -> String {
        format!(
            "{}/{}/{}/releases/download",
            self.web, self.owner, self.repo
        )
    }

    pub(crate) fn releases_url(&self) -> String {
        format!("{}/repos/{}/{}/releases", self.api, self.owner, self.repo)
    }

    pub(crate) fn contents_url(&self) -> String {
        format!("{}/repos/{}/{}/contents", self.api, self.owner, self.repo)
    }

    pub(crate) fn raw_root(&self, branch: &str) -> String {
        format!("{}/{}/{}/{branch}", self.raw, self.owner, self.repo)
    }
}
