//! Publishing through `Registry` into the in-memory store: a registry starts with an empty signed
//! index, a release is added, a published version never changes, and what is uploaded reads back.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;
use std::path::PathBuf;

use common::LINUX;
use common::MemIndex;
use common::MemStore;
use common::WIN;
use common::artifact;
use common::assert_outcome;
use common::fixture;
use common::registry;
use common::seeded;
use museum::NewRelease;
use museum::Registry;
use museum::ReleaseIndex;
use museum::VersionReq;
use rstest::rstest;
use tempfile::TempDir;

#[derive(Clone, Copy)]
enum Existing {
    Nothing,
    Fixture,
    Broken,
}

fn registry_with(existing: Existing, dir: &Path) -> Registry<MemStore, MemIndex> {
    let (store, index) = match existing {
        Existing::Fixture => seeded(&fixture()),
        Existing::Nothing | Existing::Broken => (MemStore::default(), MemIndex::default()),
    };
    *index.place().broken.lock().unwrap() = matches!(existing, Existing::Broken);
    registry(store, index, dir)
}

#[rstest]
#[case::new_version_added(Existing::Fixture, "0.6.0", &[LINUX], false, Ok(true))]
#[case::first_version_starts_from_empty_index(Existing::Nothing, "0.6.0", &[LINUX, WIN], false, Ok(true))]
#[case::same_version_same_digests_is_a_no_op(Existing::Fixture, "0.4.2", &[LINUX, WIN], false, Ok(false))]
#[case::same_version_different_digest_refused(Existing::Fixture, "0.4.2", &[LINUX, WIN], true, Err("Immutable"))]
#[case::published_index_reads_back(Existing::Fixture, "0.6.1", &[WIN, LINUX], false, Ok(true))]
#[tokio::test]
async fn publishing_updates_the_index(
    #[case] existing: Existing,
    #[case] version: &str,
    #[case] targets: &[&str],
    #[case] rebuilt: bool,
    #[case] expected: Result<bool, &str>,
) {
    let (built, downloads) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let files: Vec<(String, PathBuf)> = targets
        .iter()
        .map(|target| {
            let path = built.path().join(target);
            let suffix: &[u8] = if rebuilt { b"!" } else { b"" };
            std::fs::write(
                &path,
                [artifact("modbus", version, target).as_slice(), suffix].concat(),
            )
            .unwrap();
            (target.to_string(), path)
        })
        .collect();
    let release = NewRelease {
        package: "modbus",
        version: version.parse().unwrap(),
        interface: 2,
        files: &files,
    };
    let museum = registry_with(existing, downloads.path());

    let published = museum.publish(release).await;

    assert_outcome(&published.as_ref().map(|p| p.changed), &expected);
    let want = VersionReq::parse(&format!("={version}")).unwrap();
    let build = museum
        .resolve("modbus", targets[0], 2, &[want])
        .await
        .unwrap();
    let fetched = std::fs::read(museum.fetch(&build).await.unwrap().path).unwrap();
    assert_eq!(fetched, artifact("modbus", version, targets[0]));
}

#[rstest]
#[case::fresh_registry_initialised(Existing::Nothing, Ok(0))]
#[case::initialised_registry_refused(Existing::Fixture, Err("AlreadyInitialised"))]
#[case::init_index_failure_passed_through(Existing::Broken, Err("IndexStore(Broken)"))]
#[tokio::test]
async fn init_creates_an_empty_index(
    #[case] existing: Existing,
    #[case] expected: Result<usize, &str>,
) {
    let dir = TempDir::new().unwrap();
    let museum = registry_with(existing, dir.path());

    let initialised = museum.init().await;

    let packages = match initialised {
        Ok(_) => museum.index().await.map(|index| index.packages().len()),
        Err(e) => Err(e),
    };
    assert_outcome(&packages, &expected);
}
