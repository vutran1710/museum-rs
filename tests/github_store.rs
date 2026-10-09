//! The GitHub stores against a real local server: what each serves, and how refusals map onto
//! `StoreError`.

#![cfg(feature = "github")]
#![allow(clippy::unwrap_used)]

mod common;

use std::time::Duration;

use common::LINUX;
use common::github::Answer;
use common::github::Release;
use common::github::fake_github;
use common::github::writes;
use futures_util::TryStreamExt;
use museum::Artifact;
use museum::Store;
use museum::StoreError;
use museum::StoreWriter;
use museum::Version;
use museum::github::Github;
use museum::github::GithubError;
use museum::github::reqwest;
use rstest::rstest;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;

fn authorised() -> reqwest::header::HeaderMap {
    let token = reqwest::header::HeaderValue::from_static("Bearer t0ken");
    reqwest::header::HeaderMap::from_iter([(reqwest::header::AUTHORIZATION, token)])
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}
const ARTIFACT_PATH: &str =
    "/acme/plugins/releases/download/modbus-v0.4.2/acme-modbus-0.4.2-x86_64-unknown-linux-gnu";

#[derive(Clone, Copy)]
enum Server {
    Serving,
    Empty,
    Down,
    CutShort,
}

async fn cut_short_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _ = socket.read(&mut [0; 4096]).await;
        let _ = socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort")
            .await;
    });
    format!("http://{address}")
}

async fn read_all<S: Store<Error = GithubError>>(
    store: &S,
    session: &S::Session,
    artifact: Artifact<'_>,
) -> Result<Vec<u8>, GithubError> {
    let stream = store.open(session, artifact).await?;
    stream
        .try_fold(Vec::new(), |mut all, chunk| async move {
            all.extend_from_slice(&chunk);
            Ok(all)
        })
        .await
}

fn artifact(version: &Version) -> Artifact<'_> {
    Artifact {
        package: "modbus",
        version,
        target: LINUX,
    }
}

#[rstest]
#[case::artifact_served(Server::Serving, Ok(b"binary".to_vec()), false)]
#[case::refused_404_is_not_found(Server::Empty, Err("Refused { status: 404"), true)]
#[case::unreachable(Server::Down, Err("Unreachable"), false)]
#[case::body_cut_short_is_an_error(Server::CutShort, Err("Unreachable"), false)]
#[tokio::test]
async fn public_releases_serve_artifacts(
    #[case] server: Server,
    #[case] expected: Result<Vec<u8>, &str>,
    #[case] not_found: bool,
) {
    let mock = MockServer::start().await;
    let body = match server {
        Server::Serving => ResponseTemplate::new(200).set_body_bytes("binary"),
        _ => ResponseTemplate::new(404),
    };
    Mock::given(method("GET"))
        .and(path(ARTIFACT_PATH))
        .respond_with(body)
        .mount(&mock)
        .await;
    let base = match server {
        Server::Serving | Server::Empty => mock.uri(),
        Server::Down => "http://127.0.0.1:9".to_owned(),
        Server::CutShort => cut_short_server().await,
    };
    let store = Github::new("acme/plugins")
        .unwrap()
        .client(client())
        .enterprise(&base, &base, &base)
        .prefix("acme-")
        .releases();

    let session = store.bootstrap().await.unwrap();

    let served = read_all(&store, &session, artifact(&Version::new(0, 4, 2))).await;

    common::assert_outcome(&served, &expected);
    assert_eq!(
        served.as_ref().err().is_some_and(StoreError::not_found),
        not_found
    );
}

#[derive(Clone, Copy)]
enum Wanted {
    OnFirstPage,
    Artifact,
    Unpublished,
}

async fn private_api(status: u16) -> MockServer {
    let server = MockServer::start().await;
    let uri = server.uri();
    let token = || header("authorization", "Bearer t0ken");
    let page = |releases: serde_json::Value| ResponseTemplate::new(status).set_body_json(releases);
    let first = page(serde_json::json!([{ "tag_name": "modbus-v0.4.1", "upload_url": "", "assets": [{ "name": "acme-modbus-0.4.1-x86_64-unknown-linux-gnu", "url": format!("{uri}/assets/1") }] }]))
        .insert_header("link", format!("<{uri}/page/2>; rel=\"next\", <{uri}/page/2>; rel=\"last\"").as_str());
    let second = page(
        serde_json::json!([{ "tag_name": "modbus-v0.4.2", "upload_url": "", "assets": [{ "name": "acme-modbus-0.4.2-x86_64-unknown-linux-gnu", "url": format!("{uri}/assets/2") }] }]),
    );
    Mock::given(path("/repos/acme/plugins/releases"))
        .and(token())
        .respond_with(first)
        .mount(&server)
        .await;
    Mock::given(path("/page/2"))
        .and(token())
        .respond_with(second)
        .mount(&server)
        .await;
    for (asset, body) in [("/assets/1", "first"), ("/assets/2", "binary")] {
        let octets = header("accept", "application/octet-stream");
        Mock::given(path(asset))
            .and(token())
            .and(octets)
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;
    }
    server
}

#[rstest]
#[case::assets_resolved_through_api(200, Wanted::OnFirstPage, Ok(b"first".to_vec()), false)]
#[case::releases_listed_across_pages(200, Wanted::Artifact, Ok(b"binary".to_vec()), false)]
#[case::token_sent_on_every_request(200, Wanted::Artifact, Ok(b"binary".to_vec()), false)]
#[case::refused_401_is_unauthorized(401, Wanted::OnFirstPage, Err("Refused { status: 401"), true)]
#[case::unpublished_asset_is_not_found(200, Wanted::Unpublished, Err("MissingAsset"), false)]
#[tokio::test]
async fn private_releases_serve_artifacts(
    #[case] status: u16,
    #[case] wanted: Wanted,
    #[case] expected: Result<Vec<u8>, &str>,
    #[case] unauthorized: bool,
) {
    let server = private_api(status).await;
    let uri = server.uri();
    let store = Github::new("acme/plugins")
        .unwrap()
        .headers(authorised())
        .client(client())
        .enterprise(&uri, &uri, &uri)
        .prefix("acme-")
        .releases();
    let (first, published, unpublished) = (
        Version::new(0, 4, 1),
        Version::new(0, 4, 2),
        Version::new(0, 4, 3),
    );
    let object = match wanted {
        Wanted::OnFirstPage => artifact(&first),
        Wanted::Artifact => artifact(&published),
        Wanted::Unpublished => artifact(&unpublished),
    };

    let served = match store.bootstrap().await {
        Ok(session) => read_all(&store, &session, object).await,
        Err(e) => Err(e),
    };

    common::assert_outcome(&served, &expected);
    assert_eq!(
        served.as_ref().err().is_some_and(StoreError::unauthorized),
        unauthorized
    );
    assert_eq!(
        served.as_ref().err().is_some_and(StoreError::not_found),
        matches!(wanted, Wanted::Unpublished)
    );
    let requests = server.received_requests().await.unwrap();
    assert!(requests.iter().all(|r| {
        r.headers
            .get("authorization")
            .is_some_and(|v| v == "Bearer t0ken")
    }));
}

const UPLOAD: &str = "POST /upload/modbus-v0.4.2?name=modbus-0.4.2-x86_64-unknown-linux-gnu";
const CREATE: &str = "POST /repos/acme/plugins/releases";
const DELETE: &str = "DELETE /assets/modbus-v0.4.2/modbus-0.4.2-x86_64-unknown-linux-gnu";
const RELEASE_WITHOUT_ASSETS: &[Release<'_>] = &[("modbus-v0.4.2", &[])];
const RELEASE_WITH_ASSET: &[Release<'_>] = &[(
    "modbus-v0.4.2",
    &[("modbus-0.4.2-x86_64-unknown-linux-gnu", b"old")],
)];

#[rstest]
#[case::asset_uploaded_to_existing_release(RELEASE_WITHOUT_ASSETS, Answer::Normal, false, Ok(()), &[UPLOAD])]
#[case::release_created_when_missing(&[], Answer::Normal, false, Ok(()), &[CREATE, UPLOAD])]
#[case::existing_asset_replaced(RELEASE_WITH_ASSET, Answer::Normal, false, Ok(()), &[DELETE, UPLOAD])]
#[case::refused_403_is_unauthorized(RELEASE_WITHOUT_ASSETS, Answer::UploadRefused(403), false, Err("Refused { status: 403"), &[UPLOAD])]
#[case::malformed_listing_is_an_error(RELEASE_WITHOUT_ASSETS, Answer::MalformedListing, false, Err("Unreachable"), &[])]
#[case::malformed_created_release_is_an_error(&[], Answer::MalformedRelease, false, Err("Unreachable"), &[CREATE])]
#[case::anonymous_upload_is_read_only(RELEASE_WITHOUT_ASSETS, Answer::Normal, true, Err("ReadOnly"), &[])]
#[case::malformed_upload_answer_is_an_error(RELEASE_WITHOUT_ASSETS, Answer::MalformedUpload, false, Err("Unreachable"), &[UPLOAD])]
#[tokio::test]
async fn releases_upload(
    #[case] existing: &[Release<'_>],
    #[case] given: Answer,
    #[case] anonymous: bool,
    #[case] expected: Result<(), &str>,
    #[case] requests: &[&str],
) {
    let server = fake_github(existing, given).await;
    let uri = server.uri();
    let private = Github::new("acme/plugins").unwrap().headers(authorised());
    let github = if anonymous {
        Github::new("acme/plugins").unwrap()
    } else {
        private
    };
    let store = github
        .client(client())
        .enterprise(&uri, &uri, &uri)
        .releases();
    let location = store.location(artifact(&Version::new(0, 4, 2)));

    let uploaded = match store.bootstrap().await {
        Ok(session) => store.upload(&session, &location, b"binary".to_vec()).await,
        Err(e) => Err(e),
    };

    common::assert_outcome(&uploaded, &expected);
    assert_eq!(
        uploaded
            .as_ref()
            .err()
            .is_some_and(StoreError::unauthorized),
        given == Answer::UploadRefused(403)
    );
    assert_eq!(writes(&server).await, requests);
}
