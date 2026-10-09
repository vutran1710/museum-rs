//! The one error type every `Registry` operation returns, generic over the executable store's, the
//! index format's and the index store's own errors so callers can match on them.

use std::path::PathBuf;

use semver::Version;

use crate::index::ReleaseIndex;
use crate::index::store::IndexStore;
use crate::resolve::Unresolvable;
use crate::store::Store;

#[derive(Debug, thiserror::Error)]
pub enum Error<SE, XE, IE> {
    #[error("store: {0}")]
    Store(#[source] SE),
    #[error("index: {0}")]
    Index(#[source] XE),
    #[error("index store: {0}")]
    IndexStore(#[source] IE),
    #[error("index signature refused: {reason}")]
    BadSignature { reason: String },
    #[error(transparent)]
    Unresolvable(#[from] Unresolvable),
    #[error("the registry already has an index")]
    AlreadyInitialised,
    #[error("{package} {version} is already published with different files")]
    Immutable { package: String, version: Version },
    #[error("{object}: sha256 {actual}, index says {expected}")]
    DigestMismatch {
        object: String,
        expected: String,
        actual: String,
    },
    #[error("signing the index: {0}")]
    Sign(#[source] minisign::PError),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub type RegistryError<S, X> = Error<
    <S as Store>::Error,
    <X as ReleaseIndex>::Error,
    <<X as ReleaseIndex>::Store as IndexStore>::Error,
>;

pub(crate) fn io<SE, XE, IE>(
    path: &std::path::Path,
) -> impl FnOnce(std::io::Error) -> Error<SE, XE, IE> {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}
