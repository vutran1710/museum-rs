//! The index listing every release. `ReleaseIndex` is the file format users implement; the
//! `IndexStore` it holds finds and keeps the file (`store`); JSON / YAML / TOML are built in
//! (`builtin`).

#[cfg(any(feature = "json", feature = "yaml", feature = "toml"))]
pub(crate) mod builtin;
pub(crate) mod store;

use std::collections::BTreeMap;

use semver::Version;

use crate::index::store::IndexStore;

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    any(feature = "json", feature = "yaml", feature = "toml"),
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct Release {
    pub interface: u32,
    pub targets: BTreeMap<String, Digest>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    any(feature = "json", feature = "yaml", feature = "toml"),
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct Digest {
    pub sha256: String,
    pub size_bytes: u64,
}

pub trait IndexError: std::error::Error + Send + Sync + 'static {
    /// No index exists yet: `init` may create one and the first `publish` starts empty.
    fn not_found(&self) -> bool;
}

/// A file format. The value given to `Registry::new` is the empty index; `decode` yields a loaded
/// one holding the same store.
pub trait ReleaseIndex: Clone + Send + Sync + 'static {
    type Store: IndexStore;
    type Error: std::error::Error + Send + Sync + 'static;

    fn store(&self) -> &Self::Store;
    fn decode(&self, bytes: &[u8]) -> Result<Self, Self::Error>;
    fn encode(&self) -> Result<Vec<u8>, Self::Error>;
    fn packages(&self) -> Vec<String>;
    fn releases(&self, package: &str) -> Vec<(Version, Release)>;
    fn insert(&mut self, package: &str, version: Version, release: Release);
}
