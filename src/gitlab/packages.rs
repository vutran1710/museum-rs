//! A project's generic package registry: `GitlabPackages` keeps executables at
//! `packages/generic/<package>/<version>/<file>`, and `GitlabPackageFile` keeps an index there.

use futures_util::TryStreamExt;
use reqwest::RequestBuilder;

use crate::gitlab::Gitlab;
use crate::gitlab::GitlabError;
use crate::gitlab::location;
use crate::index::store::IndexStore;
use crate::store::Artifact;
use crate::store::ByteStream;
use crate::store::Location;
use crate::store::Store;
use crate::store::StoreWriter;
use crate::transport::HttpError;
use crate::transport::bytes;
use crate::transport::send;
use crate::transport::stream;

#[derive(Clone, Debug)]
pub struct GitlabPackages {
    gitlab: Gitlab,
}

impl GitlabPackages {
    pub(crate) fn new(gitlab: Gitlab) -> Self {
        Self { gitlab }
    }

    fn url(&self, location: &Location) -> String {
        format!("{}/packages/generic/{location}", self.gitlab.api())
    }

    fn get(&self, location: &Location) -> (String, RequestBuilder) {
        let url = self.url(location);
        (
            url.clone(),
            self.gitlab
                .client
                .get(url)
                .headers(self.gitlab.headers.clone()),
        )
    }

    async fn put(&self, location: &Location, bytes: Vec<u8>) -> Result<(), GitlabError> {
        if !self.gitlab.authorised() {
            return Err(HttpError::ReadOnly {
                place: self.url(location),
            }
            .into());
        }
        let url = self.url(location);
        send(
            self.gitlab
                .client
                .put(&url)
                .headers(self.gitlab.headers.clone())
                .body(bytes),
            &url,
        )
        .await?;
        Ok(())
    }
}

impl Store for GitlabPackages {
    type Session = ();
    type Error = GitlabError;

    async fn bootstrap(&self) -> Result<(), GitlabError> {
        Ok(())
    }

    async fn open(
        &self,
        _: &(),
        artifact: Artifact<'_>,
    ) -> Result<ByteStream<GitlabError>, GitlabError> {
        let (url, request) = self.get(&self.location(artifact));
        Ok(Box::pin(
            stream(send(request, &url).await?, &url).map_err(GitlabError::Http),
        ))
    }

    fn location(&self, artifact: Artifact<'_>) -> Location {
        location(&self.gitlab.prefix, artifact)
    }
}

impl StoreWriter for GitlabPackages {
    async fn upload(&self, _: &(), location: &Location, bytes: Vec<u8>) -> Result<(), GitlabError> {
        self.put(location, bytes).await
    }
}

/// An index kept as one generic package file, `<package>/<version>/<file>`.
#[derive(Clone, Debug)]
pub struct GitlabPackageFile {
    packages: GitlabPackages,
    location: Location,
}

impl GitlabPackageFile {
    pub(crate) fn new(packages: GitlabPackages, location: Location) -> Self {
        Self { packages, location }
    }
}

impl IndexStore for GitlabPackageFile {
    type Error = GitlabError;

    async fn read(&self) -> Result<Vec<u8>, GitlabError> {
        let (url, request) = self.packages.get(&self.location);
        Ok(bytes(request, &url).await?)
    }

    async fn write(&self, index: Vec<u8>) -> Result<String, GitlabError> {
        self.packages.put(&self.location, index).await?;
        Ok(self.location.to_string())
    }
}
