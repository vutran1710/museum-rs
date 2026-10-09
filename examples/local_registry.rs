//! A whole registry in a temporary directory, driven from Rust with no network: a custom `Store`
//! keeps executables in a folder, and the index is a YAML file next to them. It starts the
//! registry, publishes this example's own binary as `demo 1.0.0`, then fetches it back as a host
//! would.

use std::error::Error;
use std::path::PathBuf;

use bytes::Bytes;
use museum::Artifact;
use museum::ByteStream;
use museum::HOST_TARGET;
use museum::LocalFile;
use museum::Location;
use museum::NewRelease;
use museum::Options;
use museum::PublicKey;
use museum::Registry;
use museum::Store;
use museum::StoreError;
use museum::StoreWriter;
use museum::Version;
use museum::VersionReq;
use museum::Yaml;

/// Executables kept as `<dir>/<package>-v<version>/<package>-<version>-<target>`.
struct Folder {
    dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
struct FolderError(#[from] std::io::Error);

impl StoreError for FolderError {
    fn unauthorized(&self) -> bool {
        false
    }
    fn not_found(&self) -> bool {
        self.0.kind() == std::io::ErrorKind::NotFound
    }
}

impl Store for Folder {
    type Session = ();
    type Error = FolderError;

    async fn bootstrap(&self) -> Result<(), FolderError> {
        Ok(())
    }

    async fn open(
        &self,
        _: &(),
        artifact: Artifact<'_>,
    ) -> Result<ByteStream<FolderError>, FolderError> {
        let location = self.location(artifact);
        let bytes = tokio::fs::read(self.dir.join(location.group).join(location.file)).await?;
        Ok(Box::pin(futures_util::stream::iter([Ok(Bytes::from(
            bytes,
        ))])))
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

impl StoreWriter for Folder {
    async fn upload(&self, _: &(), location: &Location, bytes: Vec<u8>) -> Result<(), FolderError> {
        let group = self.dir.join(&location.group);
        tokio::fs::create_dir_all(&group).await?;
        Ok(tokio::fs::write(group.join(&location.file), bytes).await?)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let dir = std::env::temp_dir().join("museum-local-registry");
    let _ = std::fs::remove_dir_all(&dir);

    // The publisher's key pair. A real pipeline loads its secret key; hosts only get the public
    // key.
    let keys = minisign::KeyPair::generate_unencrypted_keypair()?;
    let options = Options {
        trusted_keys: vec![PublicKey::from_base64(&keys.pk.to_base64())?],
        download_dir: dir.join("downloads"),
    };
    let index = Yaml::new(LocalFile::new(dir.join("index.yaml")));
    let registry = Registry::new(
        Folder {
            dir: dir.join("executables"),
        },
        index,
        options,
    )?;

    registry.init(&keys.sk).await?;
    let files = [(HOST_TARGET.to_owned(), std::env::current_exe()?)];
    let release = NewRelease {
        package: "demo",
        version: Version::new(1, 0, 0),
        interface: 1,
        files: &files,
    };
    let published = registry.publish(release, &keys.sk).await?;
    println!("published: {}", published.uploads.join(", "));

    let wanted = [("demo".to_owned(), VersionReq::parse("^1")?)];
    for (package, fetched) in registry.fetch_all(&wanted, HOST_TARGET, 1).await {
        let fetched = fetched?;
        println!(
            "fetched {package}: {} ({} bytes, sha256 {})",
            fetched.path.display(),
            fetched.bytes,
            fetched.sha256
        );
    }
    println!("\n{}", std::fs::read_to_string(dir.join("index.yaml"))?);
    Ok(())
}
