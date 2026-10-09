//! Fetch and publish signed, per-target executables from a registry. The core owns bootstrapping,
//! signature checks, version resolution and installs; a `Store` says where bytes live and a
//! `ReleaseIndex` says what the index file looks like.

mod error;
mod index;
mod publish;
mod registry;
mod resolve;
mod store;

#[cfg(feature = "github")]
pub mod github;

pub use error::Error;
pub use error::RegistryError;
pub use index::Digest;
pub use index::IndexError;
pub use index::Release;
pub use index::ReleaseIndex;
pub use index::Signed;
#[cfg(feature = "json")]
pub use index::builtin::Json;
#[cfg(feature = "toml")]
pub use index::builtin::Toml;
#[cfg(feature = "toml")]
pub use index::builtin::TomlError;
#[cfg(feature = "yaml")]
pub use index::builtin::Yaml;
pub use index::store::IndexStore;
pub use index::store::LocalFile;
pub use minisign::SecretKey;
pub use minisign_verify::PublicKey;
pub use publish::NewRelease;
pub use publish::Published;
pub use registry::Fetched;
pub use registry::Options;
pub use registry::Registry;
pub use resolve::Build;
pub use resolve::Unresolvable;
pub use resolve::resolve;
pub use semver::Version;
pub use semver::VersionReq;
pub use store::Artifact;
pub use store::ByteStream;
pub use store::Location;
pub use store::Store;
pub use store::StoreError;
pub use store::StoreWriter;

/// Cargo's target triple of the host this library was built for.
pub const HOST_TARGET: &str = env!("HOST_TARGET");
