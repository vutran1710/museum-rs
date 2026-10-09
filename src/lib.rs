//! Fetch and publish signed, per-target executables from a registry. The core owns bootstrapping,
//! signature checks, version resolution and installs; a `Store` says where bytes live and a
//! `ReleaseIndex` says what the index file looks like.

mod error;
mod index;
mod museum;
mod publish;
mod resolve;
mod store;

#[cfg(feature = "github")]
pub mod github;

pub use error::Error;
pub use error::MuseumError;
pub use index::Digest;
pub use index::Release;
pub use index::ReleaseIndex;
#[cfg(feature = "json")]
pub use index::builtin::JsonIndex;
#[cfg(feature = "toml")]
pub use index::builtin::TomlIndex;
#[cfg(feature = "toml")]
pub use index::builtin::TomlIndexError;
#[cfg(feature = "yaml")]
pub use index::builtin::YamlIndex;
pub use minisign::SecretKey;
pub use minisign_verify::PublicKey;
pub use museum::Fetched;
pub use museum::Museum;
pub use museum::Options;
pub use publish::NewRelease;
pub use publish::Published;
pub use resolve::Build;
pub use resolve::Unresolvable;
pub use resolve::resolve;
pub use semver::Version;
pub use semver::VersionReq;
pub use store::ByteStream;
pub use store::Location;
pub use store::Object;
pub use store::Store;
pub use store::StoreError;
pub use store::StoreWriter;

/// Cargo's target triple of the host this library was built for.
pub const HOST_TARGET: &str = env!("HOST_TARGET");
