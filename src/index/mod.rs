//! The index file listing every release: the `ReleaseIndex` trait users implement, and the
//! built-in JSON / YAML / TOML implementations (`builtin`).

#[cfg(any(feature = "json", feature = "yaml", feature = "toml"))]
pub(crate) mod builtin;

use std::collections::BTreeMap;

use semver::Version;

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

/// The value given to `Museum::new` is the empty index; `decode` yields a loaded one with the
/// same configuration.
pub trait ReleaseIndex: Clone + Send + Sync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    fn file_name(&self) -> &str;
    fn decode(&self, bytes: &[u8]) -> Result<Self, Self::Error>;
    fn encode(&self) -> Result<Vec<u8>, Self::Error>;
    fn packages(&self) -> Vec<String>;
    fn releases(&self, package: &str) -> Vec<(Version, Release)>;
    fn insert(&mut self, package: &str, version: Version, release: Release);
}
