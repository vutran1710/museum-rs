//! GitHub as a registry: `Github` is one repository and how it is reached; it hands out the
//! executables store (`releases`) and index stores (`files`). Shared here: the registry string, the
//! artifact layout, and `GithubError`. HTTP itself lives in `transport`.

mod address;
mod files;
mod handle;
mod releases;

use std::fmt;
use std::str::FromStr;

pub use address::FileAddress;
pub use address::RepoAddress;
pub use files::GithubFile;
pub use handle::Github;
pub use releases::GithubReleaseFile;
pub use releases::GithubReleases;
pub use releases::ReleasesSession;

use crate::index::IndexError;
use crate::store::Artifact;
use crate::store::Location;
use crate::store::StoreError;
use crate::transport::HttpError;

pub const GITHUB: &str = "https://github.com";
pub const GITHUB_API: &str = "https://api.github.com";
pub const GITHUB_RAW: &str = "https://raw.githubusercontent.com";
/// A registry written as one string: `https://github.com/<org>/<repo>[?token_env=<VAR>]`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GithubConfig {
    pub owner: String,
    pub repo: String,
    /// The environment variable holding a token, for private registries.
    pub token_env: Option<String>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(String);

impl FromStr for GithubConfig {
    type Err = ConfigError;

    fn from_str(written: &str) -> Result<Self, Self::Err> {
        let refuse = |message: String| Err(ConfigError(message));
        let Some(rest) = written.strip_prefix("https://") else {
            return refuse("a registry is written https://<host>/<path>".to_owned());
        };
        let (rest, token_env) = match rest.split_once("?token_env=") {
            Some((rest, token_env)) => (rest, Some(token_env)),
            None => (rest, None),
        };
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        if host != "github.com" {
            return refuse(format!(
                "registries are read at github.com — not at '{host}'"
            ));
        }
        let segments: Vec<&str> = path.trim_end_matches('/').split('/').collect();
        let named = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        };
        match segments.as_slice() {
            [owner, repo] if named(owner) && named(repo) && token_env.is_none_or(named) => {
                Ok(Self {
                    owner: (*owner).to_owned(),
                    repo: (*repo).to_owned(),
                    token_env: token_env.map(str::to_owned),
                })
            }
            _ => {
                refuse("a github.com registry is https://github.com/<org>/<repository>".to_owned())
            }
        }
    }
}

impl fmt::Display for GithubConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{GITHUB}/{}/{}", self.owner, self.repo)?;
        match &self.token_env {
            Some(token_env) => write!(f, "?token_env={token_env}"),
            None => Ok(()),
        }
    }
}

impl TryFrom<String> for GithubConfig {
    type Error = ConfigError;

    fn try_from(written: String) -> Result<Self, Self::Error> {
        written.parse()
    }
}

impl From<GithubConfig> for String {
    fn from(config: GithubConfig) -> Self {
        config.to_string()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GithubError {
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error("no release asset {location}")]
    MissingAsset { location: Location },
    #[error("'{address}' is not a GitHub address; expected {expected}")]
    BadAddress { address: String, expected: String },
}

impl IndexError for GithubError {
    fn not_found(&self) -> bool {
        StoreError::not_found(self)
    }
}

impl StoreError for GithubError {
    fn unauthorized(&self) -> bool {
        matches!(self, Self::Http(e) if e.unauthorized())
    }
    fn not_found(&self) -> bool {
        matches!(self, Self::Http(e) if e.not_found()) || matches!(self, Self::MissingAsset { .. })
    }
}

pub(crate) fn location(prefix: &str, artifact: Artifact<'_>) -> Location {
    let Artifact {
        package,
        version,
        target,
    } = artifact;
    let exe = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    Location {
        group: format!("{package}-v{version}"),
        file: format!("{prefix}{package}-{version}-{target}{exe}"),
    }
}
