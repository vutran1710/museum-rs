//! Where an index file lives: the `IndexStore` trait, which finds and keeps one file and its
//! `.minisig`, and `LocalFile`. Network stores live with their transport (`github`).

use std::future::Future;
use std::path::PathBuf;

use crate::index::IndexError;
use crate::index::Signed;

pub trait IndexStore: Clone + Send + Sync + 'static {
    type Error: IndexError;

    fn read(&self) -> impl Future<Output = Result<Signed, Self::Error>> + Send;
    /// Returns a description of each file written, e.g. `index/index.json`.
    fn write(
        &self,
        signed: Signed,
    ) -> impl Future<Output = Result<Vec<String>, Self::Error>> + Send;
}

impl IndexError for std::io::Error {
    fn not_found(&self) -> bool {
        self.kind() == std::io::ErrorKind::NotFound
    }
}

/// An index file on disk, next to its `.minisig`.
#[derive(Clone, Debug)]
pub struct LocalFile {
    path: PathBuf,
}

impl LocalFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn signature(&self) -> PathBuf {
        let mut signature = self.path.clone().into_os_string();
        signature.push(".minisig");
        signature.into()
    }
}

impl IndexStore for LocalFile {
    type Error = std::io::Error;

    async fn read(&self) -> Result<Signed, std::io::Error> {
        let index = tokio::fs::read(&self.path).await?;
        let signature = tokio::fs::read(self.signature()).await.ok();
        Ok(Signed { index, signature })
    }

    async fn write(&self, signed: Signed) -> Result<Vec<String>, std::io::Error> {
        tokio::fs::create_dir_all(self.path.parent().unwrap_or(&self.path)).await?;
        tokio::fs::write(&self.path, signed.index).await?;
        tokio::fs::write(self.signature(), signed.signature.unwrap_or_default()).await?;
        Ok(vec![
            self.path.display().to_string(),
            self.signature().display().to_string(),
        ])
    }
}
