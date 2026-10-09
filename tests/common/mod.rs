//! Test doubles shared by the core tests: an in-memory `Store` for executables that counts calls
//! and fails on demand, an in-memory `ReleaseIndex` whose place counts reads, the release fixture
//! and minisign keys.

#![allow(dead_code)]

#[cfg(feature = "github")]
pub mod github;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use bytes::Bytes;
use futures_util::stream;
use museum::Artifact;
use museum::ByteStream;
use museum::Digest;
use museum::IndexError;
use museum::IndexStore;
use museum::Location;
use museum::Options;
use museum::PublicKey;
use museum::Registry;
use museum::Release;
use museum::ReleaseIndex;
use museum::Signed;
use museum::Store;
use museum::StoreError;
use museum::StoreWriter;
use museum::Version;
use sha2::Digest as _;
use sha2::Sha256;

pub const WIN: &str = "x86_64-pc-windows-msvc";
pub const LINUX: &str = "x86_64-unknown-linux-gnu";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Unauthorized,
    Broken,
    CutShort,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum MemError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("no {0}")]
    NotFound(Location),
    #[error("broken")]
    Broken,
    #[error("cut short")]
    CutShort,
    #[error("bootstrap failed")]
    BootstrapFailed,
}

impl StoreError for MemError {
    fn unauthorized(&self) -> bool {
        *self == MemError::Unauthorized
    }
    fn not_found(&self) -> bool {
        matches!(self, MemError::NotFound(_))
    }
}

#[derive(Default)]
pub struct Log {
    pub opened: Mutex<Vec<(usize, String)>>,
    pub bootstraps: AtomicUsize,
}

impl Log {
    pub fn bootstraps(&self) -> usize {
        self.bootstraps.load(Ordering::SeqCst)
    }
    pub fn sessions(&self) -> BTreeSet<usize> {
        self.opened
            .lock()
            .unwrap()
            .iter()
            .map(|(session, _)| *session)
            .collect()
    }
    pub fn opens(&self) -> usize {
        self.opened.lock().unwrap().len()
    }
}

#[derive(Default)]
pub struct MemStore {
    pub objects: Mutex<BTreeMap<String, Vec<u8>>>,
    pub faults: Mutex<VecDeque<Fault>>,
    pub bootstrap_fails: bool,
    pub log: Arc<Log>,
}

impl MemStore {
    pub fn put(&self, artifact: Artifact<'_>, bytes: Vec<u8>) {
        self.objects
            .lock()
            .unwrap()
            .insert(self.location(artifact).to_string(), bytes);
    }
    pub fn with_faults(self, faults: &[Fault]) -> Self {
        *self.faults.lock().unwrap() = faults.iter().copied().collect();
        self
    }
}

impl Store for MemStore {
    type Session = usize;
    type Error = MemError;

    async fn bootstrap(&self) -> Result<usize, MemError> {
        let generation = self.log.bootstraps.fetch_add(1, Ordering::SeqCst) + 1;
        if self.bootstrap_fails {
            Err(MemError::BootstrapFailed)
        } else {
            Ok(generation)
        }
    }

    async fn open(
        &self,
        session: &usize,
        artifact: Artifact<'_>,
    ) -> Result<ByteStream<MemError>, MemError> {
        let location = self.location(artifact);
        self.log
            .opened
            .lock()
            .unwrap()
            .push((*session, location.to_string()));
        let fault = self.faults.lock().unwrap().pop_front();
        let bytes = self
            .objects
            .lock()
            .unwrap()
            .get(&location.to_string())
            .cloned();
        match (fault, bytes) {
            (Some(Fault::Unauthorized), _) => Err(MemError::Unauthorized),
            (Some(Fault::Broken), _) => Err(MemError::Broken),
            (_, None) => Err(MemError::NotFound(location)),
            (Some(Fault::CutShort), Some(bytes)) => {
                let half = Bytes::from(bytes[..bytes.len() / 2].to_vec());
                Ok(Box::pin(stream::iter([Ok(half), Err(MemError::CutShort)])))
            }
            (None, Some(bytes)) => Ok(Box::pin(stream::iter([Ok(Bytes::from(bytes))]))),
        }
    }

    fn location(&self, artifact: Artifact<'_>) -> Location {
        let Artifact {
            package,
            version,
            target,
        } = artifact;
        Location {
            group: format!("{package}-v{version}"),
            file: format!("{package}-{version}-{target}"),
        }
    }
}

impl StoreWriter for MemStore {
    async fn upload(&self, _: &usize, location: &Location, bytes: Vec<u8>) -> Result<(), MemError> {
        self.objects
            .lock()
            .unwrap()
            .insert(location.to_string(), bytes);
        Ok(())
    }
}

/// Where a `MemIndex` lives: shared by every clone, counting reads, failing on demand.
#[derive(Default)]
pub struct MemPlace {
    pub signed: Mutex<Option<Signed>>,
    pub reads: AtomicUsize,
    pub broken: Mutex<bool>,
}

impl MemPlace {
    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl std::fmt::Debug for MemPlace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MemPlace")
    }
}

#[derive(Clone, Debug, Default)]
pub struct MemIndexStore(pub Arc<MemPlace>);

#[derive(Debug, thiserror::Error)]
pub enum MemStoreError {
    #[error("no index")]
    NotFound,
    #[error("broken")]
    Broken,
}

impl IndexError for MemStoreError {
    fn not_found(&self) -> bool {
        matches!(self, MemStoreError::NotFound)
    }
}

impl IndexStore for MemIndexStore {
    type Error = MemStoreError;

    async fn read(&self) -> Result<Signed, MemStoreError> {
        self.0.reads.fetch_add(1, Ordering::SeqCst);
        if *self.0.broken.lock().unwrap() {
            return Err(MemStoreError::Broken);
        }
        self.0
            .signed
            .lock()
            .unwrap()
            .clone()
            .ok_or(MemStoreError::NotFound)
    }
    async fn write(&self, signed: Signed) -> Result<Vec<String>, MemStoreError> {
        *self.0.signed.lock().unwrap() = Some(signed);
        Ok(vec!["mem:index".to_owned()])
    }
}

#[derive(Clone, Debug, Default)]
pub struct MemIndex {
    pub releases: BTreeMap<String, BTreeMap<Version, Release>>,
    pub store: MemIndexStore,
}

impl MemIndex {
    pub fn place(&self) -> &MemPlace {
        &self.store.0
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unreadable index line {0:?}")]
pub struct Unreadable(pub String);

impl ReleaseIndex for MemIndex {
    type Store = MemIndexStore;
    type Error = Unreadable;

    fn store(&self) -> &MemIndexStore {
        &self.store
    }
    fn decode(&self, bytes: &[u8]) -> Result<Self, Unreadable> {
        let mut index = MemIndex {
            releases: BTreeMap::new(),
            store: self.store.clone(),
        };
        for line in String::from_utf8_lossy(bytes).lines() {
            let unreadable = || Unreadable(line.to_owned());
            let fields: Vec<&str> = line.split(' ').collect();
            let [package, version, interface, target, sha256, size] = fields[..] else {
                return Err(unreadable());
            };
            let version: Version = version.parse().map_err(|_| unreadable())?;
            let interface: u32 = interface.parse().map_err(|_| unreadable())?;
            let digest = Digest {
                sha256: sha256.to_owned(),
                size_bytes: size.parse().map_err(|_| unreadable())?,
            };
            let versions = index.releases.entry(package.to_owned()).or_default();
            let release = versions.entry(version).or_insert(Release {
                interface,
                targets: BTreeMap::new(),
            });
            release.targets.insert(target.to_owned(), digest);
        }
        Ok(index)
    }
    fn encode(&self) -> Result<Vec<u8>, Unreadable> {
        let mut lines = String::new();
        for (package, versions) in &self.releases {
            for (version, release) in versions {
                for (target, digest) in &release.targets {
                    let (interface, sha256, size) =
                        (release.interface, &digest.sha256, digest.size_bytes);
                    lines.push_str(&format!(
                        "{package} {version} {interface} {target} {sha256} {size}\n"
                    ));
                }
            }
        }
        Ok(lines.into_bytes())
    }
    fn packages(&self) -> Vec<String> {
        self.releases.keys().cloned().collect()
    }
    fn releases(&self, package: &str) -> Vec<(Version, Release)> {
        self.releases
            .get(package)
            .into_iter()
            .flatten()
            .map(|(v, r)| (v.clone(), r.clone()))
            .collect()
    }
    fn insert(&mut self, package: &str, version: Version, release: Release) {
        self.releases
            .entry(package.to_owned())
            .or_default()
            .insert(version, release);
    }
}

pub fn artifact(package: &str, version: &str, target: &str) -> Vec<u8> {
    format!("{package}-{version}-{target}").into_bytes()
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn digest(bytes: &[u8]) -> Digest {
    Digest {
        sha256: sha256(bytes),
        size_bytes: bytes.len() as u64,
    }
}

pub fn fixture() -> MemIndex {
    let mut index = MemIndex::default();
    let rows: [(&str, &str, u32, &[&str]); 6] = [
        ("modbus", "0.4.1", 2, &[WIN, LINUX]),
        ("modbus", "0.4.2", 2, &[WIN, LINUX]),
        ("modbus", "0.5.0", 2, &[WIN]),
        ("modbus", "9.0.0", 3, &[WIN, LINUX]),
        ("modbus", "1.0.0-rc.1", 2, &[WIN, LINUX]),
        ("opcua", "1.0.0", 2, &[WIN, LINUX]),
    ];
    for (package, version, interface, targets) in rows {
        let targets = targets
            .iter()
            .map(|t| (t.to_string(), digest(&artifact(package, version, t))))
            .collect();
        index.insert(
            package,
            version.parse().unwrap(),
            Release { interface, targets },
        );
    }
    index
}

pub fn keypair() -> minisign::KeyPair {
    minisign::KeyPair::generate_unencrypted_keypair().unwrap()
}

pub fn trusted(keys: &minisign::KeyPair) -> PublicKey {
    PublicKey::from_base64(&keys.pk.to_base64()).unwrap()
}

pub fn sign(keys: &minisign::KeyPair, bytes: &[u8]) -> Vec<u8> {
    minisign::sign(None, &keys.sk, std::io::Cursor::new(bytes), None, None)
        .unwrap()
        .to_string()
        .into_bytes()
}

/// A store holding every artifact of `index`, and an empty index whose place holds `index` signed.
pub fn seeded(index: &MemIndex, signer: &minisign::KeyPair) -> (MemStore, MemIndex) {
    let store = MemStore::default();
    for package in index.packages() {
        for (version, release) in index.releases(&package) {
            for target in release.targets.keys() {
                store.put(
                    Artifact {
                        package: &package,
                        version: &version,
                        target,
                    },
                    artifact(&package, &version.to_string(), target),
                );
            }
        }
    }
    let empty = MemIndex::default();
    let bytes = index.encode().unwrap();
    *empty.place().signed.lock().unwrap() = Some(Signed {
        signature: Some(sign(signer, &bytes)),
        index: bytes,
    });
    (store, empty)
}

pub fn registry(
    store: MemStore,
    index: MemIndex,
    keys: &[&minisign::KeyPair],
    dir: &Path,
) -> Registry<MemStore, MemIndex> {
    let options = Options {
        trusted_keys: keys.iter().map(|k| trusted(k)).collect(),
        download_dir: dir.to_path_buf(),
    };
    Registry::new(store, index, options).unwrap()
}

pub fn assert_outcome<T: PartialEq + std::fmt::Debug, E: std::fmt::Debug>(
    actual: &Result<T, E>,
    expected: &Result<T, &str>,
) {
    match (actual, expected) {
        (Ok(actual), Ok(expected)) => assert_eq!(actual, expected),
        (Err(actual), Err(prefix)) => {
            assert!(format!("{actual:?}").starts_with(prefix), "{actual:?}")
        }
        _ => panic!("got {actual:?}, expected {expected:?}"),
    }
}
