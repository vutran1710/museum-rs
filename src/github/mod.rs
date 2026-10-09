//! GitHub as a registry: `Github` is one repository and how it is reached; it hands out the
//! executables store (`releases`) and index stores (`files`). `HttpFile` reads an index from any
//! URL. Shared here: the registry string, the artifact layout, tokens, and the HTTP error.

mod files;
mod handle;
mod releases;

use std::fmt;
use std::str::FromStr;

use futures_util::StreamExt;
use futures_util::TryStreamExt;

pub use files::GithubFile;
pub use files::HttpFile;
pub use handle::Github;
pub use releases::GithubReleaseFile;
pub use releases::GithubReleases;
pub use releases::ReleasesSession;
pub use reqwest;
use reqwest::RequestBuilder;
use reqwest::Response;
use reqwest::header::HeaderValue;
use reqwest::header::USER_AGENT;

use crate::index::IndexError;
use crate::store::Artifact;
use crate::store::Location;
use crate::store::StoreError;

pub const GITHUB: &str = "https://github.com";
pub const GITHUB_API: &str = "https://api.github.com";
pub const GITHUB_RAW: &str = "https://raw.githubusercontent.com";
const AGENT: &str = concat!("museum/", env!("CARGO_PKG_VERSION"));

/// A registry written as one string: `https://github.com/<org>/<repo>[?token_env=<VAR>]`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum GithubConfig {
    Public {
        owner: String,
        repo: String,
    },
    Private {
        owner: String,
        repo: String,
        token_env: String,
    },
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
        match (segments.as_slice(), token_env) {
            ([owner, repo], None) if named(owner) && named(repo) => Ok(Self::Public {
                owner: (*owner).to_owned(),
                repo: (*repo).to_owned(),
            }),
            ([owner, repo], Some(token_env)) if named(owner) && named(repo) && named(token_env) => {
                Ok(Self::Private {
                    owner: (*owner).to_owned(),
                    repo: (*repo).to_owned(),
                    token_env: token_env.to_owned(),
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
        match self {
            Self::Public { owner, repo } => write!(f, "{GITHUB}/{owner}/{repo}"),
            Self::Private {
                owner,
                repo,
                token_env,
            } => write!(f, "{GITHUB}/{owner}/{repo}?token_env={token_env}"),
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
    #[error("could not reach {url}: {source}")]
    Unreachable { url: String, source: reqwest::Error },
    #[error("{url} answered {status}")]
    Refused { status: u16, url: String },
    #[error("environment variable {env_var} holds no token")]
    MissingToken { env_var: String },
    #[error("no release asset {location}")]
    MissingAsset { location: Location },
    #[error("{place} is read-only")]
    ReadOnly { place: String },
}

impl IndexError for GithubError {
    fn not_found(&self) -> bool {
        StoreError::not_found(self)
    }
}

impl StoreError for GithubError {
    fn unauthorized(&self) -> bool {
        matches!(
            self,
            Self::Refused {
                status: 401 | 403,
                ..
            }
        )
    }
    fn not_found(&self) -> bool {
        matches!(
            self,
            Self::Refused { status: 404, .. } | Self::MissingAsset { .. }
        )
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

pub(crate) fn bearer(token_env: &str) -> Result<HeaderValue, GithubError> {
    let token = std::env::var(token_env).ok();
    let header = token.and_then(|token| HeaderValue::from_str(&format!("Bearer {token}")).ok());
    header.ok_or_else(|| GithubError::MissingToken {
        env_var: token_env.to_owned(),
    })
}

pub(crate) async fn bytes(request: RequestBuilder, url: &str) -> Result<Vec<u8>, GithubError> {
    let chunks = stream(send(request, url).await?, url);
    chunks
        .try_fold(Vec::new(), |mut all, chunk| async move {
            all.extend_from_slice(&chunk);
            Ok(all)
        })
        .await
}

/// A missing signature is `None`, for `Registry` to refuse; any other failure stays an error.
pub(crate) fn optional(read: Result<Vec<u8>, GithubError>) -> Result<Option<Vec<u8>>, GithubError> {
    match read {
        Err(e) if StoreError::not_found(&e) => Ok(None),
        read => read.map(Some),
    }
}

pub(crate) async fn send(request: RequestBuilder, url: &str) -> Result<Response, GithubError> {
    let unreachable = |source| GithubError::Unreachable {
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
        status => Err(GithubError::Refused {
            status: status.as_u16(),
            url: url.to_owned(),
        }),
    }
}

pub(crate) fn stream(response: Response, url: &str) -> crate::store::ByteStream<GithubError> {
    let url = url.to_owned();
    Box::pin(response.bytes_stream().map(move |chunk| {
        chunk.map_err(|source| GithubError::Unreachable {
            url: url.clone(),
            source,
        })
    }))
}
