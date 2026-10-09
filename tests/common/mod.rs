//! Test doubles shared by the core tests: an in-memory `Store` that counts calls and fails on
//! demand, an in-memory `ReleaseIndex`, the release fixture and minisign keys.

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
use museum::ByteStream;
use museum::Digest;
use museum::Location;
use museum::Museum;
use museum::Object;
use museum::Options;
use museum::PublicKey;
use museum::Release;
use museum::ReleaseIndex;
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
    pub fn opens_of(&self, group_prefix: &str) -> usize {
        self.opened
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, at)| at.starts_with(group_prefix))
            .count()
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
    pub fn put(&self, object: Object<'_>, bytes: Vec<u8>) {
        self.objects
            .lock()
            .unwrap()
            .insert(self.location(object).to_string(), bytes);
    }
    pub fn with_faults(self, faults: &[Fault]) -> Self {
        *self.faults.lock().unwrap() = faults.iter().cloned().collect();
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
        object: Object<'_>,
    ) -> Result<ByteStream<MemError>, MemError> {
        let location = self.location(object);
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

    fn location(&self, object: Object<'_>) -> Location {
        let (group, file) = match object {
            Object::Index { file_name } => ("index".to_owned(), file_name.to_owned()),
            Object::Signature { file_name } => ("index".to_owned(), format!("{file_name}.minisig")),
            Object::Artifact {
                package,
                version,
                target,
            } => (
                format!("{package}-v{version}"),
                format!("{package}-{version}-{target}"),
            ),
        };
        Location { group, file }
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemIndex(pub BTreeMap<String, BTreeMap<Version, Release>>);

#[derive(Debug, thiserror::Error)]
#[error("unreadable index line {0:?}")]
pub struct MemIndexError(pub String);

impl ReleaseIndex for MemIndex {
    type Error = MemIndexError;

    fn file_name(&self) -> &str {
        "index.mem"
    }
    fn decode(&self, bytes: &[u8]) -> Result<Self, MemIndexError> {
        let mut index = MemIndex::default();
        for line in String::from_utf8_lossy(bytes).lines() {
            let fields: Vec<&str> = line.split(' ').collect();
            let [package, version, interface, target, sha256, size] = fields[..] else {
                return Err(MemIndexError(line.to_owned()));
            };
            let bad = |_| MemIndexError(line.to_owned());
            let version: Version = version
                .parse()
                .map_err(|_| MemIndexError(line.to_owned()))?;
            let interface: u32 = interface.parse().map_err(bad)?;
            let digest = Digest {
                sha256: sha256.to_owned(),
                size_bytes: size.parse().map_err(bad)?,
            };
            let release = index
                .0
                .entry(package.to_owned())
                .or_default()
                .entry(version)
                .or_insert(Release {
                    interface,
                    targets: BTreeMap::new(),
                });
            release.targets.insert(target.to_owned(), digest);
        }
        Ok(index)
    }
    fn encode(&self) -> Result<Vec<u8>, MemIndexError> {
        let mut lines = String::new();
        for (package, versions) in &self.0 {
            for (version, release) in versions {
                for (target, digest) in &release.targets {
                    let interface = release.interface;
                    let (sha256, size) = (&digest.sha256, digest.size_bytes);
                    lines.push_str(&format!(
                        "{package} {version} {interface} {target} {sha256} {size}\n"
                    ));
                }
            }
        }
        Ok(lines.into_bytes())
    }
    fn packages(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }
    fn releases(&self, package: &str) -> Vec<(Version, Release)> {
        self.0
            .get(package)
            .into_iter()
            .flatten()
            .map(|(v, r)| (v.clone(), r.clone()))
            .collect()
    }
    fn insert(&mut self, package: &str, version: Version, release: Release) {
        self.0
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

pub fn seeded(index: &MemIndex, signer: &minisign::KeyPair) -> MemStore {
    let store = MemStore::default();
    let bytes = index.encode().unwrap();
    store.put(
        Object::Signature {
            file_name: "index.mem",
        },
        sign(signer, &bytes),
    );
    store.put(
        Object::Index {
            file_name: "index.mem",
        },
        bytes,
    );
    for package in index.packages() {
        for (version, release) in index.releases(&package) {
            for target in release.targets.keys() {
                let object = Object::Artifact {
                    package: &package,
                    version: &version,
                    target,
                };
                store.put(object, artifact(&package, &version.to_string(), target));
            }
        }
    }
    store
}

pub fn registry(
    store: MemStore,
    keys: &[&minisign::KeyPair],
    dir: &Path,
) -> Museum<MemStore, MemIndex> {
    let options = Options {
        trusted_keys: keys.iter().map(|k| trusted(k)).collect(),
        download_dir: dir.to_path_buf(),
    };
    Museum::new(store, MemIndex::default(), options).unwrap()
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
