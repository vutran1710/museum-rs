//! The publishing side of `Museum`: start a registry with an empty signed index, and add a release
//! — verified index first, a published version never changes, executables uploaded before the index
//! that lists them.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::PathBuf;

use minisign::SecretKey;
use semver::Version;
use sha2::Digest as _;
use sha2::Sha256;

use crate::error::Error;
use crate::error::MuseumError;
use crate::error::io;
use crate::index::Digest;
use crate::index::Release;
use crate::index::ReleaseIndex;
use crate::museum::Museum;
use crate::store::Location;
use crate::store::Object;
use crate::store::StoreError;
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
    pub uploads: Vec<Location>,
}

impl<S: StoreWriter, X: ReleaseIndex> Museum<S, X> {
    pub async fn init(&self, secret_key: &SecretKey) -> Result<Vec<Location>, MuseumError<S, X>> {
        match self
            .read(Object::Index {
                file_name: self.empty.file_name(),
            })
            .await
        {
            Err(Error::Store(e)) if e.not_found() => {
                self.write_index(&self.empty, secret_key).await
            }
            Ok(_) => Err(Error::AlreadyInitialised),
            Err(e) => Err(e),
        }
    }

    pub async fn publish(
        &self,
        release: NewRelease<'_>,
        secret_key: &SecretKey,
    ) -> Result<Published, MuseumError<S, X>> {
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
                self.store.location(Object::Artifact {
                    package,
                    version: &version,
                    target,
                }),
                bytes,
            ));
        }
        let new = Release { interface, targets };

        let mut index = match self.load(&self.trusted_keys).await {
            Err(Error::Store(e)) if e.not_found() => self.empty.clone(),
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
            uploads.push(location);
        }
        uploads.extend(self.write_index(&index, secret_key).await?);
        Ok(Published {
            changed: true,
            uploads,
        })
    }

    async fn write_index(
        &self,
        index: &X,
        secret_key: &SecretKey,
    ) -> Result<Vec<Location>, MuseumError<S, X>> {
        let bytes = index.encode().map_err(Error::Index)?;
        let signature = minisign::sign(None, secret_key, Cursor::new(&bytes), None, None)
            .map_err(Error::Sign)?;
        let file_name = self.empty.file_name();
        let mut uploads = Vec::new();
        for (object, contents) in [
            (Object::Index { file_name }, bytes),
            (
                Object::Signature { file_name },
                signature.to_string().into_bytes(),
            ),
        ] {
            let location = self.store.location(object);
            self.upload(&location, contents).await?;
            uploads.push(location);
        }
        Ok(uploads)
    }

    async fn upload(&self, location: &Location, bytes: Vec<u8>) -> Result<(), MuseumError<S, X>> {
        self.authorised(|session| {
            let bytes = bytes.clone();
            async move { self.store.upload(&session, location, bytes).await }
        })
        .await
    }
}
