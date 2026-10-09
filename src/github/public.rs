//! A public GitHub repository's releases, read anonymously from `releases/download`.

use crate::github::GithubError;
use crate::github::location;
use crate::github::send;
use crate::github::stream;
use crate::store::ByteStream;
use crate::store::Location;
use crate::store::Object;
use crate::store::Store;

pub struct GithubPublic {
    client: reqwest::Client,
    download_root: String,
    prefix: String,
}

impl GithubPublic {
    /// `base_url` is `GITHUB`, or a GitHub Enterprise host.
    /// The `client` carries the caller's timeout and proxy settings.
    pub fn new(client: reqwest::Client, base_url: &str, owner: &str, repo: &str) -> Self {
        Self {
            client,
            download_root: format!("{base_url}/{owner}/{repo}/releases/download"),
            prefix: String::new(),
        }
    }

    pub fn prefix(self, prefix: &str) -> Self {
        Self {
            prefix: prefix.to_owned(),
            ..self
        }
    }
}

impl Store for GithubPublic {
    type Session = ();
    type Error = GithubError;

    async fn bootstrap(&self) -> Result<(), GithubError> {
        Ok(())
    }

    async fn open(
        &self,
        _: &(),
        object: Object<'_>,
    ) -> Result<ByteStream<GithubError>, GithubError> {
        let url = format!("{}/{}", self.download_root, self.location(object));
        let response = send(self.client.get(&url), &url).await?;
        Ok(stream(response, &url))
    }

    fn location(&self, object: Object<'_>) -> Location {
        location(&self.prefix, object)
    }
}
