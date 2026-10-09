//! GitLab as a registry: `Gitlab` is one project and how it is reached; it hands out the
//! executables store (`packages`, the generic package registry) and index stores (a committed file,
//! or a generic package file). Shared here: the handle, the artifact layout, and `GitlabError`.

mod files;
mod packages;

use std::time::Duration;

pub use files::GitlabFile;
pub use packages::GitlabPackageFile;
pub use packages::GitlabPackages;

use crate::address::reference_and_path;
use crate::address::segments;
use crate::headers::AUTHORIZATION;
use crate::headers::HeaderMap;
use crate::index::IndexError;
use crate::store::Artifact;
use crate::store::Location;
use crate::store::StoreError;
use crate::transport::Http;
use crate::transport::HttpError;

pub const GITLAB: &str = "https://gitlab.com";

#[derive(Debug, thiserror::Error)]
pub enum GitlabError {
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error("'{address}' is not a GitLab address; expected {expected}")]
    BadAddress { address: String, expected: String },
}

impl StoreError for GitlabError {
    fn unauthorized(&self) -> bool {
        matches!(self, Self::Http(e) if e.unauthorized())
    }
    fn not_found(&self) -> bool {
        matches!(self, Self::Http(e) if e.not_found())
    }
}

impl IndexError for GitlabError {
    fn not_found(&self) -> bool {
        StoreError::not_found(self)
    }
}

fn refused(address: &str, expected: &str) -> GitlabError {
    GitlabError::BadAddress {
        address: address.to_owned(),
        expected: expected.to_owned(),
    }
}

/// Generic packages are addressed `<package>/<version>/<file>`, so the version stays out of the
/// name.
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
        group: format!("{package}/{version}"),
        file: format!("{prefix}{package}-{target}{exe}"),
    }
}

/// One GitLab project (`group/project`, subgroups allowed) and the headers sent with every request
/// to it. An `Authorization`, `PRIVATE-TOKEN` or `JOB-TOKEN` header makes private projects readable
/// and uploads and commits possible.
#[derive(Clone, Debug)]
pub struct Gitlab {
    project: String,
    host: String,
    pub(crate) client: Http,
    pub(crate) headers: HeaderMap,
    pub(crate) prefix: String,
}

impl Gitlab {
    pub fn new(project: &str) -> Result<Self, GitlabError> {
        let parts = segments(project)
            .filter(|parts| parts.len() >= 2)
            .ok_or_else(|| refused(project, "group/project"))?;
        Ok(Self {
            project: parts.join("/"),
            host: GITLAB.to_owned(),
            client: Http::new(),
            headers: HeaderMap::new(),
            prefix: String::new(),
        })
    }

    /// Sent with every request, e.g. `museum::headers::bearer(token)`.
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

    /// A self-managed GitLab, e.g. `https://gitlab.example.com`.
    pub fn host(self, host: &str) -> Self {
        Self {
            host: host.trim_end_matches('/').to_owned(),
            ..self
        }
    }

    /// The executables, kept in the project's generic package registry.
    pub fn packages(&self) -> GitlabPackages {
        GitlabPackages::new(self.clone())
    }

    /// An index kept as a generic package file: `<package>/<version>/<file>`.
    pub fn package_file(&self, address: &str) -> Result<GitlabPackageFile, GitlabError> {
        match segments(address).as_deref() {
            Some([package, version, file]) => {
                let location = Location {
                    group: format!("{package}/{version}"),
                    file: (*file).to_owned(),
                };
                Ok(GitlabPackageFile::new(self.packages(), location))
            }
            _ => Err(refused(address, "<package>/<version>/<file>")),
        }
    }

    /// An index committed to the project: `<branch, tag or commit>/<path>`.
    pub fn file(&self, address: &str) -> Result<GitlabFile, GitlabError> {
        let (reference, path) =
            reference_and_path(address).ok_or_else(|| refused(address, "<ref>/<path>"))?;
        Ok(GitlabFile::new(self.clone(), reference, path))
    }

    pub(crate) fn authorised(&self) -> bool {
        [AUTHORIZATION.as_str(), "private-token", "job-token"]
            .iter()
            .any(|name| self.headers.contains_key(*name))
    }

    pub(crate) fn api(&self) -> String {
        format!(
            "{}/api/v4/projects/{}",
            self.host,
            self.project.replace('/', "%2F")
        )
    }
}
