//! An index committed to a GitLab project (`GitlabFile`), read and written through the repository
//! files API.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::gitlab::Gitlab;
use crate::gitlab::GitlabError;
use crate::index::store::IndexStore;
use crate::transport::HttpError;
use crate::transport::bytes;
use crate::transport::optional;
use crate::transport::send;

/// An index committed at `<ref>/<path>`. Reads work anonymously on public projects; commits need
/// the handle's token.
#[derive(Clone, Debug)]
pub struct GitlabFile {
    gitlab: Gitlab,
    reference: String,
    path: String,
}

impl GitlabFile {
    pub(crate) fn new(gitlab: Gitlab, reference: String, path: String) -> Self {
        Self {
            gitlab,
            reference,
            path,
        }
    }

    fn url(&self) -> String {
        format!(
            "{}/repository/files/{}",
            self.gitlab.api(),
            self.path.replace('/', "%2F")
        )
    }
}

impl IndexStore for GitlabFile {
    type Error = GitlabError;

    async fn read(&self) -> Result<Vec<u8>, GitlabError> {
        let url = format!("{}/raw?ref={}", self.url(), self.reference);
        Ok(bytes(
            self.gitlab
                .client
                .get(&url)
                .headers(self.gitlab.headers.clone()),
            &url,
        )
        .await?)
    }

    async fn write(&self, index: Vec<u8>) -> Result<String, GitlabError> {
        let (client, headers, url) = (&self.gitlab.client, self.gitlab.headers.clone(), self.url());
        if !self.gitlab.authorised() {
            return Err(HttpError::ReadOnly {
                place: format!("{}:{}", self.reference, self.path),
            }
            .into());
        }
        let lookup = client
            .get(format!("{url}?ref={}", self.reference))
            .headers(headers.clone());
        let exists = optional(bytes(lookup, &url).await)?.is_some();
        let body = serde_json::json!({
            "branch": self.reference,
            "content": STANDARD.encode(index),
            "encoding": "base64",
            "commit_message": format!("museum: update {}", self.path),
        });
        let request = if exists {
            client.put(&url)
        } else {
            client.post(&url)
        };
        send(request.headers(headers).json(&body), &url).await?;
        Ok(format!("{}:{}", self.reference, self.path))
    }
}
