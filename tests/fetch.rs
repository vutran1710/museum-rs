//! Fetching through `Registry` against the in-memory store: only verified files are left, the
//! download directory resolves, and failures keep their types.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;
use std::path::PathBuf;

use common::Fault;
use common::LINUX;
use common::artifact;
use common::assert_outcome;
use common::fixture;
use common::registry;
use common::seeded;
use museum::Artifact;
use museum::Build;
use museum::VersionReq;
use museum::resolve;
use rstest::rstest;
use tempfile::TempDir;

#[derive(Clone, Copy)]
enum Before {
    Empty,
    Damaged,
    OtherBuild,
    Intact,
}

fn build() -> Build {
    resolve(
        &fixture(),
        "modbus",
        LINUX,
        2,
        &[VersionReq::parse("^0.4").unwrap()],
    )
    .unwrap()
}

fn installed(dir: &Path) -> PathBuf {
    dir.join(format!("modbus-0.4.2{}", std::env::consts::EXE_SUFFIX))
}

fn files_in(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| entries.count())
        .unwrap_or(0)
}

#[derive(Clone, Copy)]
enum Dir {
    Absolute,
    Relative,
    Missing,
}

#[rstest]
#[case::arrives(Before::Empty, true, &[], Dir::Absolute, Ok(false), 1)]
#[case::digest_mismatch_left_nowhere(Before::Empty, false, &[], Dir::Absolute, Err("DigestMismatch"), 0)]
#[case::damaged_copy_overwritten(Before::Damaged, true, &[], Dir::Absolute, Ok(false), 1)]
#[case::other_build_overwritten(Before::OtherBuild, true, &[], Dir::Absolute, Ok(false), 1)]
#[case::intact_copy_kept_without_store_call(Before::Intact, false, &[Fault::Broken], Dir::Absolute, Ok(true), 1)]
#[case::interrupted_stream_left_nowhere(Before::Empty, true, &[Fault::CutShort], Dir::Absolute, Err("Store(CutShort)"), 0)]
#[case::relative_dir_resolved_against_current_dir(Before::Empty, true, &[], Dir::Relative, Ok(false), 1)]
#[case::missing_dir_created(Before::Empty, true, &[], Dir::Missing, Ok(false), 1)]
#[tokio::test]
async fn fetch_leaves_only_verified_files(
    #[case] before: Before,
    #[case] served_right: bool,
    #[case] faults: &[Fault],
    #[case] dir: Dir,
    #[case] expected: Result<bool, &str>,
    #[case] files_left: usize,
) {
    let temp = TempDir::new().unwrap();
    let relative = PathBuf::from("target").join(temp.path().file_name().unwrap());
    let given = match dir {
        Dir::Absolute => temp.path().to_path_buf(),
        Dir::Relative => relative.clone(),
        Dir::Missing => temp.path().join("a").join("b"),
    };
    let (store, index) = seeded(&fixture());
    let store = store.with_faults(faults);
    let right = artifact("modbus", "0.4.2", LINUX);
    if !served_right {
        store.put(
            Artifact {
                package: "modbus",
                version: &build().version,
                target: LINUX,
            },
            b"other".to_vec(),
        );
    }
    let on_disk = match before {
        Before::Empty => None,
        Before::Damaged => Some(right[1..].to_vec()),
        Before::OtherBuild => Some(artifact("modbus", "0.4.1", LINUX)),
        Before::Intact => Some(right.clone()),
    };
    if let Some(bytes) = on_disk {
        std::fs::write(installed(&given), bytes).unwrap();
    }
    let registry = registry(store, index, &given);

    let fetched = registry.fetch(&build()).await;

    assert_outcome(&fetched.as_ref().map(|f| f.from_cache), &expected);
    assert_eq!(files_in(&given), files_left);
    if let Ok(fetched) = fetched {
        assert_eq!(
            fetched.path,
            installed(&std::env::current_dir().unwrap().join(&given))
        );
        assert_eq!(std::fs::read(&fetched.path).unwrap(), right);
        assert_eq!(fetched.sha256, build().digest.sha256);
        #[cfg(unix)]
        if !fetched.from_cache {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&fetched.path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
        }
    }
    let _ = std::fs::remove_dir_all(relative);
}

#[derive(Clone, Copy)]
enum Broken {
    Store,
    Bootstrap,
    IndexRead,
    IndexDecode,
    DownloadDir,
    Nothing,
}

#[rstest]
#[case::store_error_passed_through(Broken::Store, "modbus", "Store(Broken)")]
#[case::bootstrap_error_passed_through(Broken::Bootstrap, "modbus", "Store(BootstrapFailed)")]
#[case::index_read_error_passed_through(Broken::IndexRead, "modbus", "IndexStore(Broken)")]
#[case::index_decode_error_passed_through(Broken::IndexDecode, "modbus", "Index(Unreadable(")]
#[case::no_such_build(Broken::Nothing, "nope", "Unresolvable(NoSuchBuild")]
#[case::download_dir_unwritable(Broken::DownloadDir, "modbus", "Io {")]
#[tokio::test]
async fn failures_are_typed(#[case] broken: Broken, #[case] package: &str, #[case] expected: &str) {
    let temp = TempDir::new().unwrap();
    let dir = temp.path().join("downloads");
    let (mut store, index) = seeded(&fixture());
    match broken {
        Broken::Store => store = store.with_faults(&[Fault::Broken]),
        Broken::Bootstrap => store.bootstrap_fails = true,
        Broken::IndexRead => *index.place().broken.lock().unwrap() = true,
        Broken::IndexDecode => *index.place().index.lock().unwrap() = Some(b"garbage".to_vec()),
        Broken::DownloadDir => std::fs::write(&dir, b"a file").unwrap(),
        Broken::Nothing => {}
    }
    let registry = registry(store, index, &dir);

    let fetched = match registry
        .resolve(package, LINUX, 2, &[VersionReq::STAR])
        .await
    {
        Ok(build) => registry.fetch(&build).await,
        Err(e) => Err(e),
    };

    assert_outcome(&fetched.map(|_| ()), &Err(expected));
}
