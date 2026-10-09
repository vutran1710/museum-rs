//! `Museum`: the one door through which a host reads a registry — bootstrap, the verified index,
//! resolution and atomic installs into the download directory. Publishing lives in `publish`.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use futures_util::StreamExt;
use futures_util::future::join_all;
use minisign_verify::PublicKey;
use minisign_verify::Signature;
use semver::VersionReq;
use sha2::Digest as _;
use sha2::Sha256;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio::sync::OnceCell;

use crate::error::Error;
use crate::error::MuseumError;
use crate::error::io;
use crate::index::ReleaseIndex;
use crate::resolve::Build;
use crate::resolve::resolve;
use crate::store::ByteStream;
use crate::store::Object;
use crate::store::Store;
use crate::store::StoreError;

pub struct Options {
    pub trusted_keys: Vec<PublicKey>,
    pub download_dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub from_cache: bool,
}

pub struct Museum<S: Store, X: ReleaseIndex> {
    pub(crate) store: S,
    pub(crate) empty: X,
    pub(crate) trusted_keys: Vec<PublicKey>,
    download_dir: PathBuf,
    session: Mutex<Option<Arc<S::Session>>>,
    loaded: OnceCell<Arc<X>>,
}

impl<S: Store, X: ReleaseIndex> Museum<S, X> {
    pub fn new(store: S, index: X, options: Options) -> Result<Self, MuseumError<S, X>> {
        let download_dir =
            std::path::absolute(&options.download_dir).map_err(io(&options.download_dir))?;
        Ok(Self {
            store,
            empty: index,
            trusted_keys: options.trusted_keys,
            download_dir,
            session: Mutex::new(None),
            loaded: OnceCell::new(),
        })
    }

    pub async fn index(&self) -> Result<Arc<X>, MuseumError<S, X>> {
        let loaded = self
            .loaded
            .get_or_try_init(|| async { self.load(&self.trusted_keys).await.map(Arc::new) });
        loaded.await.cloned()
    }

    pub async fn resolve(
        &self,
        package: &str,
        target: &str,
        interface: u32,
        wants: &[VersionReq],
    ) -> Result<Build, MuseumError<S, X>> {
        Ok(resolve(
            &*self.index().await?,
            package,
            target,
            interface,
            wants,
        )?)
    }

    pub async fn fetch(&self, build: &Build) -> Result<Fetched, MuseumError<S, X>> {
        let name = format!(
            "{}-{}{}",
            build.package,
            build.version,
            std::env::consts::EXE_SUFFIX
        );
        let path = self.download_dir.join(&name);
        let cached = async {
            let mut file = tokio::fs::File::open(&path).await?;
            let (mut hasher, mut buffer, mut bytes) = (Sha256::new(), vec![0; 64 * 1024], 0);
            loop {
                match file.read(&mut buffer).await? {
                    0 => return std::io::Result::Ok((format!("{:x}", hasher.finalize()), bytes)),
                    read => {
                        hasher.update(&buffer[..read]);
                        bytes += read as u64;
                    }
                }
            }
        };
        if let Ok((sha256, bytes)) = cached.await
            && sha256 == build.digest.sha256
        {
            return Ok(Fetched {
                path,
                bytes,
                sha256,
                from_cache: true,
            });
        }
        tokio::fs::create_dir_all(&self.download_dir)
            .await
            .map_err(io(&self.download_dir))?;
        let part = self.download_dir.join(format!("{name}.part"));
        let object = Object::Artifact {
            package: &build.package,
            version: &build.version,
            target: &build.target,
        };
        let written = self.download(object, &part, &build.digest.sha256).await;
        if written.is_err() {
            let _ = tokio::fs::remove_file(&part).await;
        }
        let (sha256, bytes) = written?;
        tokio::fs::rename(&part, &path).await.map_err(io(&path))?;
        Ok(Fetched {
            path,
            bytes,
            sha256,
            from_cache: false,
        })
    }

    pub async fn fetch_all(
        &self,
        wanted: &[(String, VersionReq)],
        target: &str,
        interface: u32,
    ) -> BTreeMap<String, Result<Fetched, MuseumError<S, X>>> {
        let mut by_package: BTreeMap<&str, Vec<VersionReq>> = BTreeMap::new();
        for (package, want) in wanted {
            by_package.entry(package).or_default().push(want.clone());
        }
        let each = by_package.into_iter().map(|(package, wants)| async move {
            let fetched = match self.resolve(package, target, interface, &wants).await {
                Ok(build) => self.fetch(&build).await,
                Err(e) => Err(e),
            };
            (package.to_owned(), fetched)
        });
        join_all(each).await.into_iter().collect()
    }

    pub(crate) async fn load(&self, keys: &[PublicKey]) -> Result<X, MuseumError<S, X>> {
        let file_name = self.empty.file_name();
        let bytes = self.read(Object::Index { file_name }).await?;
        let signature = match self.read(Object::Signature { file_name }).await {
            Err(Error::Store(e)) if e.not_found() => {
                return Err(Error::BadSignature {
                    reason: format!("no {file_name}.minisig: {e}"),
                });
            }
            other => other?,
        };
        let signature = Signature::decode(&String::from_utf8_lossy(&signature)).map_err(|e| {
            Error::BadSignature {
                reason: e.to_string(),
            }
        })?;
        if !keys
            .iter()
            .any(|key| key.verify(&bytes, &signature, false).is_ok())
        {
            return Err(Error::BadSignature {
                reason: format!("{file_name} is not signed by a trusted key"),
            });
        }
        self.empty.decode(&bytes).map_err(Error::Index)
    }

    pub(crate) async fn read(&self, object: Object<'_>) -> Result<Vec<u8>, MuseumError<S, X>> {
        let mut stream = self.open(object).await?;
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(&chunk.map_err(Error::Store)?);
        }
        Ok(bytes)
    }

    async fn download(
        &self,
        object: Object<'_>,
        part: &Path,
        expected: &str,
    ) -> Result<(String, u64), MuseumError<S, X>> {
        let mut stream = self.open(object).await?;
        let mut file = tokio::fs::File::create(part).await.map_err(io(part))?;
        let mut hasher = Sha256::new();
        let mut bytes = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(Error::Store)?;
            hasher.update(&chunk);
            bytes += chunk.len() as u64;
            file.write_all(&chunk).await.map_err(io(part))?;
        }
        file.flush().await.map_err(io(part))?;
        let actual = format!("{:x}", hasher.finalize());
        if actual != expected {
            let object = self.store.location(object).to_string();
            return Err(Error::DigestMismatch {
                object,
                expected: expected.to_owned(),
                actual,
            });
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::Permissions::from_mode(0o755);
            tokio::fs::set_permissions(part, mode)
                .await
                .map_err(io(part))?;
        }
        Ok((actual, bytes))
    }

    async fn open(&self, object: Object<'_>) -> Result<ByteStream<S::Error>, MuseumError<S, X>> {
        self.authorised(|session| async move { self.store.open(&session, object).await })
            .await
    }

    /// Runs `call` with the session, re-bootstrapping once if the store reports it unauthorized.
    pub(crate) async fn authorised<T, F, Fut>(&self, call: F) -> Result<T, MuseumError<S, X>>
    where
        F: Fn(Arc<S::Session>) -> Fut,
        Fut: Future<Output = Result<T, S::Error>>,
    {
        let session = self.session(None).await?;
        match call(session.clone()).await {
            Err(e) if e.unauthorized() => {
                let session = self.session(Some(&session)).await?;
                call(session).await.map_err(Error::Store)
            }
            done => done.map_err(Error::Store),
        }
    }

    async fn session(
        &self,
        stale: Option<&Arc<S::Session>>,
    ) -> Result<Arc<S::Session>, MuseumError<S, X>> {
        let mut slot = self.session.lock().await;
        if let Some(current) = slot.as_ref()
            && stale.is_none_or(|stale| !Arc::ptr_eq(stale, current))
        {
            return Ok(current.clone());
        }
        let fresh = Arc::new(self.store.bootstrap().await.map_err(Error::Store)?);
        *slot = Some(fresh.clone());
        Ok(fresh)
    }
}
