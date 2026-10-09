//! The built-in GitHub stores: releases of one repository, read anonymously (`GithubPublic`) or
//! with a token through the API (`GithubApi`, which also writes). Shared here: the registry string,
//! the release layout, and the HTTP error.

mod api;
mod public;

use std::fmt;
use std::str::FromStr;

use futures_util::StreamExt;

pub use api::ApiSession;
pub use api::GithubApi;
pub use public::GithubPublic;
pub use reqwest;
use reqwest::RequestBuilder;
use reqwest::Response;
use reqwest::header::USER_AGENT;

use crate::store::Location;
use crate::store::Object;
use crate::store::StoreError;

pub const GITHUB: &str = "https://github.com";
pub const GITHUB_API: &str = "https://api.github.com";
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

pub(crate) fn location(prefix: &str, object: Object<'_>) -> Location {
    let (group, file) = match object {
        Object::Index { file_name } => ("index".to_owned(), file_name.to_owned()),
        Object::Signature { file_name } => ("index".to_owned(), format!("{file_name}.minisig")),
        Object::Artifact {
            package,
            version,
            target,
        } => {
            let exe = if target.contains("windows") {
                ".exe"
            } else {
                ""
            };
            (
                format!("{package}-v{version}"),
                format!("{prefix}{package}-{version}-{target}{exe}"),
            )
        }
    };
    Location { group, file }
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
