//! What `Registry` guarantees around the store: the bootstrapped session, and the work `fetch_all`
//! shares between packages.

#![allow(clippy::unwrap_used)]

mod common;

use std::collections::BTreeSet;

use common::Fault;
use common::LINUX;
use common::WIN;
use common::assert_outcome;
use common::fixture;
use common::registry;
use common::seeded;
use museum::VersionReq;
use rstest::rstest;
use tempfile::TempDir;

fn wanted(written: &[(&str, &str)]) -> Vec<(String, VersionReq)> {
    written
        .iter()
        .map(|(p, w)| (p.to_string(), w.parse().unwrap()))
        .collect()
}

#[rstest]
#[case::bootstrapped_once_for_many_fetches(&[], Ok(()), 1, &[1])]
#[case::session_passed_to_every_open(&[], Ok(()), 1, &[1])]
#[case::unauthorized_rebootstraps_once_and_retries(&[Fault::Unauthorized], Ok(()), 2, &[1, 2])]
#[case::unauthorized_twice_gives_up(&[Fault::Unauthorized; 4], Err("Store(Unauthorized)"), 3, &[1, 2, 3])]
#[tokio::test]
async fn access_is_bootstrapped(
    #[case] faults: &[Fault],
    #[case] expected: Result<(), &str>,
    #[case] bootstraps: usize,
    #[case] sessions: &[usize],
) {
    let dir = TempDir::new().unwrap();
    let (store, index) = seeded(&fixture());
    let store = store.with_faults(faults);
    let log = store.log.clone();
    let registry = registry(store, index, dir.path());

    let fetched = registry
        .fetch_all(&wanted(&[("modbus", "^0.4"), ("opcua", "*")]), LINUX, 2)
        .await;

    for outcome in fetched.values() {
        assert_outcome(&outcome.as_ref().map(|_| ()), &expected);
    }
    assert_eq!(log.bootstraps(), bootstraps);
    assert_eq!(
        log.sessions(),
        sessions.iter().copied().collect::<BTreeSet<_>>()
    );
}

#[rstest]
#[case::one_index_read_for_many_packages(&[("modbus", "*"), ("opcua", "*"), ("modbus", "^0.4")], LINUX, &[("modbus", Ok("modbus-0.4.2")), ("opcua", Ok("opcua-1.0.0"))], 2)]
#[case::one_download_per_package(&[("modbus", "^0.4"), ("modbus", ">=0.4.1")], LINUX, &[("modbus", Ok("modbus-0.4.2"))], 1)]
#[case::conflict_reported_while_others_fetch(&[("modbus", "^0.4"), ("modbus", "^0.5"), ("opcua", "*")], WIN, &[("modbus", Err("Unresolvable(Conflict")), ("opcua", Ok("opcua-1.0.0"))], 1)]
#[tokio::test]
async fn fetch_all_shares_work(
    #[case] written: &[(&str, &str)],
    #[case] target: &str,
    #[case] expected: &[(&str, Result<&str, &str>)],
    #[case] downloads: usize,
) {
    let dir = TempDir::new().unwrap();
    let (store, index) = seeded(&fixture());
    let (log, place) = (store.log.clone(), index.store.0.clone());
    let registry = registry(store, index, dir.path());

    let fetched = registry.fetch_all(&wanted(written), target, 2).await;

    assert_eq!(fetched.len(), expected.len());
    for (package, outcome) in expected {
        let name = fetched[*package]
            .as_ref()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned());
        assert_outcome(
            &name,
            &outcome.map(|stem| format!("{stem}{}", std::env::consts::EXE_SUFFIX)),
        );
    }
    assert_eq!(place.reads(), 1);
    assert_eq!(log.opens(), downloads);
}
