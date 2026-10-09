//! Where an index file can live: a local file, any URL, a GitHub release asset, or a file committed
//! to a GitHub repository. Each store reads what is there and writes where it can.

#![cfg(feature = "github")]
#![allow(clippy::unwrap_used)]

mod common;

use std::time::Duration;

use common::github::Answer;
use common::github::fake_github;
use museum::HttpFile;
use museum::IndexError;
use museum::IndexStore;
use museum::LocalFile;
use museum::github::Github;
use rstest::rstest;
use tempfile::TempDir;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;
use wiremock::matchers::query_param;

#[derive(Clone, Copy)]
enum Place {
    Local,
    Http,
    ReleasePublic,
    ReleaseApi,
    RepoRaw,
    RepoToken,
}

#[derive(Clone, Copy)]
enum Op {
    Read,
    Write,
    WriteThenRead,
}

#[derive(Clone, Copy)]
enum Served {
    Nothing,
    Index,
    ExistingCommit,
}

fn authorised() -> museum::headers::HeaderMap {
    museum::headers::bearer("t0ken").unwrap()
}

const TIMEOUT: Duration = Duration::from_secs(5);

async fn serve(server: &MockServer, served: Served) {
    let raw = || header("accept", "application/vnd.github.raw");
    let json = header("accept", "application/vnd.github+json");
    let files: &[(&str, &str)] = match served {
        Served::Nothing | Served::ExistingCommit => &[],
        Served::Index => &[("index.json", "{}")],
    };
    for (file, body) in files {
        for at in [
            format!("/reg/{file}"),
            format!("/acme/plugins/releases/download/index/{file}"),
            format!("/acme/plugins/main/registry/{file}"),
        ] {
            Mock::given(method("GET"))
                .and(path(at))
                .respond_with(ResponseTemplate::new(200).set_body_string(*body))
                .mount(server)
                .await;
        }
        let contents = format!("/repos/acme/plugins/contents/registry/{file}");
        let found = ResponseTemplate::new(200).set_body_string(*body);
        Mock::given(method("GET"))
            .and(path(contents))
            .and(query_param("ref", "main"))
            .and(raw())
            .respond_with(found)
            .mount(server)
            .await;
    }
    if let Served::ExistingCommit = served {
        let existing =
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "sha": "abc" }));
        let contents = path("/repos/acme/plugins/contents/registry/index.json");
        Mock::given(method("GET"))
            .and(contents)
            .and(json)
            .respond_with(existing)
            .mount(server)
            .await;
    }
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(201))
        .mount(server)
        .await;
}

async fn exercise<I: IndexStore>(store: I, op: Op) -> Result<String, (String, bool)> {
    let refused = |e: I::Error| (format!("{e:?}"), e.not_found());
    let written = match op {
        Op::Read => String::new(),
        Op::Write | Op::WriteThenRead => store.write(b"{}".to_vec()).await.map_err(refused)?,
    };
    let read = match op {
        Op::Write => None,
        Op::Read | Op::WriteThenRead => Some(store.read().await.map_err(refused)?),
    };
    Ok(match read {
        Some(read) => String::from_utf8_lossy(&read).into_owned(),
        None => written,
    })
}

async fn commits(server: &MockServer) -> Vec<String> {
    let requests = server.received_requests().await.unwrap();
    let puts = requests.iter().filter(|r| r.method.as_str() == "PUT");
    puts.map(|r| {
        let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
        format!(
            "{} sha={}",
            r.url
                .path()
                .trim_start_matches("/repos/acme/plugins/contents/"),
            body["sha"].as_str().unwrap_or("none")
        )
    })
    .collect()
}

#[rstest]
#[case::local_file_round_trip(Place::Local, Op::WriteThenRead, Served::Nothing, Ok("{}"), &[])]
#[case::local_file_missing_is_not_found(Place::Local, Op::Read, Served::Nothing, Err(("Os {", true)), &[])]
#[case::http_file_read(Place::Http, Op::Read, Served::Index, Ok("{}"), &[])]
#[case::http_file_is_read_only(Place::Http, Op::Write, Served::Nothing, Err(("ReadOnly", false)), &[])]
#[case::github_release_file_read(Place::ReleasePublic, Op::Read, Served::Index, Ok("{}"), &[])]
#[case::github_release_file_written(Place::ReleaseApi, Op::Write, Served::Nothing, Ok("index/index.json"), &[])]
#[case::github_file_read_raw(Place::RepoRaw, Op::Read, Served::Index, Ok("{}"), &[])]
#[case::github_file_read_with_token(Place::RepoToken, Op::Read, Served::Index, Ok("{}"), &[])]
#[case::github_file_committed_over_existing(Place::RepoToken, Op::Write, Served::ExistingCommit, Ok("main:registry/index.json"), &["registry/index.json sha=abc"])]
#[case::github_file_without_authorization_is_read_only(Place::RepoRaw, Op::Write, Served::Nothing, Err(("Http(ReadOnly", false)), &[])]
#[case::github_file_missing_is_not_found(Place::RepoRaw, Op::Read, Served::Nothing, Err(("Http(Refused { status: 404", true)), &[])]
#[tokio::test]
async fn index_stores_read_and_write(
    #[case] place: Place,
    #[case] op: Op,
    #[case] served: Served,
    #[case] expected: Result<&str, (&str, bool)>,
    #[case] committed: &[&str],
) {
    let (dir, server) = (
        TempDir::new().unwrap(),
        fake_github(&[], Answer::Normal).await,
    );
    serve(&server, served).await;
    let uri = server.uri();
    let public = Github::new("acme/plugins")
        .unwrap()
        .timeout(TIMEOUT)
        .enterprise(&uri, &uri, &uri);
    let private = Github::new("acme/plugins")
        .unwrap()
        .headers(authorised())
        .timeout(TIMEOUT)
        .enterprise(&uri, &uri, &uri);

    let outcome = match place {
        Place::Local => {
            exercise(
                LocalFile::new(dir.path().join("index").join("index.json")),
                op,
            )
            .await
        }
        Place::Http => {
            exercise(
                HttpFile::new(&format!("{uri}/reg/index.json")).timeout(TIMEOUT),
                op,
            )
            .await
        }
        Place::ReleasePublic => {
            exercise(public.release_file("index/index.json").unwrap(), op).await
        }
        Place::ReleaseApi => exercise(private.release_file("index/index.json").unwrap(), op).await,
        Place::RepoRaw => exercise(public.file("main/registry/index.json").unwrap(), op).await,
        Place::RepoToken => exercise(private.file("main/registry/index.json").unwrap(), op).await,
    };

    match (&outcome, expected) {
        (Ok(summary), Ok(expected)) => assert_eq!(summary, expected),
        (Err((error, not_found)), Err((prefix, expected_not_found))) => {
            assert!(error.starts_with(prefix), "{error}");
            assert_eq!(*not_found, expected_not_found);
        }
        _ => panic!("got {outcome:?}, expected {expected:?}"),
    }
    assert_eq!(commits(&server).await, committed);
}
