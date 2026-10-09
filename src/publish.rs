//! The publishing side of `Registry`: start a registry with an empty index, and add a release. A
//! published version never changes, and executables are uploaded before the index that lists them.

use std::collections::BTreeMap;
use std::path::PathBuf;

use semver::Version;
use sha2::Digest as _;
use sha2::Sha256;

use crate::error::Error;
use crate::error::RegistryError;
use crate::error::io;
use crate::index::Digest;
use crate::index::IndexError;
use crate::index::Release;
use crate::index::ReleaseIndex;
use crate::index::store::IndexStore;
use crate::registry::Registry;
use crate::store::Artifact;
use crate::store::Location;
use crate::store::Store;
use crate::store::StoreWriter;

pub struct NewRelease<'a> {
    pub package: &'a str,
    pub version: Version,
    pub interface: u32,
    pub files: &'a [(String, PathBuf)],
}

#[derive(Debug, PartialEq, Eq)]
pub struct Published {
    pub changed: bool,
    /// What was written: each executable's location, then where the index went.
    pub uploads: Vec<String>,
}

impl<S: Store, X: ReleaseIndex> Registry<S, X> {
    pub async fn init(&self) -> Result<Vec<String>, RegistryError<S, X>> {
        match self.empty.store().read().await {
            Err(e) if e.not_found() => self.write_index(&self.empty).await,
            Ok(_) => Err(Error::AlreadyInitialised),
            Err(e) => Err(Error::IndexStore(e)),
        }
    }

    async fn write_index(&self, index: &X) -> Result<Vec<String>, RegistryError<S, X>> {
        let bytes = index.encode().map_err(Error::Index)?;
        Ok(vec![
            index
                .store()
                .write(bytes)
                .await
                .map_err(Error::IndexStore)?,
        ])
    }
}

impl<S: StoreWriter, X: ReleaseIndex> Registry<S, X> {
    pub async fn publish(&self, release: NewRelease<'_>) -> Result<Published, RegistryError<S, X>> {
        let NewRelease {
            package,
            version,
            interface,
            files,
        } = release;
        let mut built = Vec::new();
        let mut targets = BTreeMap::new();
        for (target, path) in files {
            let bytes = tokio::fs::read(path).await.map_err(io(path))?;
            let sha256 = format!("{:x}", Sha256::digest(&bytes));
            targets.insert(
                target.clone(),
                Digest {
                    sha256,
                    size_bytes: bytes.len() as u64,
                },
            );
            built.push((
                self.store.location(Artifact {
                    package,
                    version: &version,
                    target,
                }),
                bytes,
            ));
        }
        let new = Release { interface, targets };

        let mut index = match self.load().await {
            Err(Error::IndexStore(e)) if e.not_found() => self.empty.clone(),
            loaded => loaded?,
        };
        match index
            .releases(package)
            .into_iter()
            .find(|(v, _)| *v == version)
        {
            Some((_, existing)) if existing == new => {
                return Ok(Published {
                    changed: false,
                    uploads: Vec::new(),
                });
            }
            Some(_) => {
                return Err(Error::Immutable {
                    package: package.to_owned(),
                    version,
                });
            }
            None => index.insert(package, version, new),
        }

        let mut uploads = Vec::new();
        for (location, bytes) in built {
            self.upload(&location, bytes).await?;
            uploads.push(location.to_string());
        }
        uploads.extend(self.write_index(&index).await?);
        Ok(Published {
            changed: true,
            uploads,
        })
    }

    async fn upload(&self, location: &Location, bytes: Vec<u8>) -> Result<(), RegistryError<S, X>> {
        self.authorised(|session| {
            let bytes = bytes.clone();
            async move { self.store.upload(&session, location, bytes).await }
        })
        .await
    }
}
