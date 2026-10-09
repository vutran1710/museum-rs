//! What `Museum` guarantees around the store: the index signature, the bootstrapped session,
//! and the work `fetch_all` shares between packages.

#![allow(clippy::unwrap_used)]

mod common;

use std::collections::BTreeSet;

use common::Fault;
use common::LINUX;
use common::WIN;
use common::assert_outcome;
use common::fixture;
use common::keypair;
use common::registry;
use common::seeded;
use museum::Object;
use museum::ReleaseIndex;
use museum::VersionReq;
use rstest::rstest;
use tempfile::TempDir;

#[derive(Clone, Copy)]
enum Signed {
    By(usize),
    Missing,
    Garbage,
}

#[rstest]
#[case::signed_index_reads(Signed::By(0), false, &[0], Ok(()))]
#[case::missing_signature_refused(Signed::Missing, false, &[0], Err("BadSignature"))]
#[case::signature_not_decodable_refused(Signed::Garbage, false, &[0], Err("BadSignature"))]
#[case::tampered_index_refused(Signed::By(0), true, &[0], Err("BadSignature"))]
#[case::untrusted_key_refused(Signed::By(1), false, &[0], Err("BadSignature"))]
#[case::either_of_two_trusted_keys_accepted(Signed::By(1), false, &[0, 1], Ok(()))]
#[tokio::test]
async fn index_signature_is_enforced(
    #[case] signed: Signed,
    #[case] tampered: bool,
    #[case] trusted: &[usize],
    #[case] expected: Result<(), &str>,
) {
    let dir = TempDir::new().unwrap();
    let keys = [keypair(), keypair()];
    let store = seeded(
        &fixture(),
        &keys[if let Signed::By(signer) = signed {
            signer
        } else {
            0
        }],
    );
    let file_name = fixture().file_name().to_owned();
    let signature = format!("index/{file_name}.minisig");
    match signed {
        Signed::Missing => drop(store.objects.lock().unwrap().remove(&signature)),
        Signed::Garbage => drop(
            store
                .objects
                .lock()
                .unwrap()
                .insert(signature, b"not a signature".to_vec()),
        ),
        Signed::By(_) => {}
    }
    if tampered {
        let mut bytes = fixture().encode().unwrap();
        bytes.extend_from_slice(b"modbus 6.6.6 2 x86_64-unknown-linux-gnu 00 1\n");
        store.put(
            Object::Index {
                file_name: &file_name,
            },
            bytes,
        );
    }
    let trusted: Vec<_> = trusted.iter().map(|i| &keys[*i]).collect();
    let registry = registry(store, &trusted, dir.path());

    let index = registry.index().await;

    assert_outcome(&index.map(|_| ()), &expected);
}

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
    let keys = keypair();
    let store = seeded(&fixture(), &keys).with_faults(faults);
    let log = store.log.clone();
    let registry = registry(store, &[&keys], dir.path());

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
    let keys = keypair();
    let store = seeded(&fixture(), &keys);
    let log = store.log.clone();
    let registry = registry(store, &[&keys], dir.path());

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
    assert_eq!(log.opens_of("index/"), 2);
    assert_eq!(
        log.opens_of("modbus-v") + log.opens_of("opcua-v"),
        downloads
    );
}
