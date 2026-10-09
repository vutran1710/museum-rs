//! Where an index file lives: the `IndexStore` trait, which finds and keeps one file, and
//! `LocalFile`. Network stores live with their transport (`github`).

use std::future::Future;
use std::path::PathBuf;

use crate::index::IndexError;

pub trait IndexStore: Clone + Send + Sync + 'static {
    type Error: IndexError;

    fn read(&self) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send;
    /// Returns a description of what was written, e.g. `index/index.json`.
    fn write(&self, index: Vec<u8>) -> impl Future<Output = Result<String, Self::Error>> + Send;
}

impl IndexError for std::io::Error {
    fn not_found(&self) -> bool {
        self.kind() == std::io::ErrorKind::NotFound
    }
}

/// An index file on disk.
#[derive(Clone, Debug)]
pub struct LocalFile {
    path: PathBuf,
}

impl LocalFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl IndexStore for LocalFile {
    type Error = std::io::Error;

    async fn read(&self) -> Result<Vec<u8>, std::io::Error> {
        tokio::fs::read(&self.path).await
    }

    async fn write(&self, index: Vec<u8>) -> Result<String, std::io::Error> {
        tokio::fs::create_dir_all(self.path.parent().unwrap_or(&self.path)).await?;
        tokio::fs::write(&self.path, index).await?;
        Ok(self.path.display().to_string())
    }
}
