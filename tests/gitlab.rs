//! GitLab as a registry against a local server standing in for its API: project and file
//! addresses, the generic-package layout, and the package store serving and taking executables.

#![cfg(feature = "gitlab")]
#![allow(clippy::unwrap_used)]

mod common;

use std::time::Duration;

use common::LINUX;
use common::WIN;
use common::assert_outcome;
use futures_util::TryStreamExt;
use museum::Artifact;
use museum::Location;
use museum::Store;
use museum::StoreError;
use museum::StoreWriter;
use museum::Version;
use museum::gitlab::Gitlab;
use museum::gitlab::GitlabError;
use rstest::rstest;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;

const TIMEOUT: Duration = Duration::from_secs(5);
const PACKAGE: &str = "/api/v4/projects/acme%2Ftools%2Fplugins/packages/generic/modbus/0.4.2/acme-modbus-x86_64-unknown-linux-gnu";

#[derive(Clone, Copy)]
enum Address {
    Project,
    File,
    PackageFile,
}

#[rstest]
#[case::nested_project_accepted(Address::Project, "acme/tools/plugins", Ok(()))]
#[case::single_segment_project_refused(
    Address::Project,
    "acme",
    Err("'acme' is not a GitLab address; expected group/project")
)]
#[case::file_address_accepted(Address::File, "main/registry/index.json", Ok(()))]
#[case::file_without_path_refused(
    Address::File,
    "main",
    Err("'main' is not a GitLab address; expected <ref>/<path>")
)]
#[case::package_file_address_accepted(Address::PackageFile, "index/1.0.0/index.json", Ok(()))]
#[case::package_file_with_extra_segment_refused(
    Address::PackageFile,
    "index/1.0.0/x/index.json",
    Err("'index/1.0.0/x/index.json' is not a GitLab address; expected <package>/<version>/<file>")
)]
#[case::empty_segment_refused(
    Address::File,
    "main//index.json",
    Err("'main//index.json' is not a GitLab address; expected <ref>/<path>")
)]
fn gitlab_addresses_parse(
    #[case] kind: Address,
    #[case] written: &str,
    #[case] expected: Result<(), &str>,
) {
    let gitlab = Gitlab::new("acme/plugins").unwrap();
    let parsed = match kind {
        Address::Project => Gitlab::new(written).map(|_| ()),
        Address::File => gitlab.file(written).map(|_| ()),
        Address::PackageFile => gitlab.package_file(written).map(|_| ()),
    };
    assert_eq!(
        parsed.map_err(|e| e.to_string()),
        expected.map_err(str::to_owned)
    );
}

#[rstest]
#[case::artifact_linux(LINUX, "", "modbus/0.4.2", "modbus-x86_64-unknown-linux-gnu")]
#[case::windows_target_gets_exe(WIN, "", "modbus/0.4.2", "modbus-x86_64-pc-windows-msvc.exe")]
#[case::prefix_applied(LINUX, "acme-", "modbus/0.4.2", "acme-modbus-x86_64-unknown-linux-gnu")]
fn gitlab_location_follows_layout(
    #[case] target: &str,
    #[case] prefix: &str,
    #[case] group: &str,
    #[case] file: &str,
) {
    let store = Gitlab::new("acme/plugins")
        .unwrap()
        .prefix(prefix)
        .packages();
    let artifact = Artifact {
        package: "modbus",
        version: &Version::new(0, 4, 2),
        target,
    };
    assert_eq!(
        store.location(artifact),
        Location {
            group: group.into(),
            file: file.into()
        }
    );
}

fn artifact(version: &Version) -> Artifact<'_> {
    Artifact {
        package: "modbus",
        version,
        target: LINUX,
    }
}

async fn gitlab(server: &MockServer, authorised: bool) -> Gitlab {
    let gitlab = Gitlab::new("acme/tools/plugins")
        .unwrap()
        .host(&server.uri())
        .timeout(TIMEOUT)
        .prefix("acme-");
    if authorised {
        gitlab.headers(museum::headers::bearer("t0ken").unwrap())
    } else {
        gitlab
    }
}

#[rstest]
#[case::artifact_served(false, 200, Ok(b"binary".to_vec()), false, false)]
#[case::refused_404_is_not_found(false, 404, Err("Http(Refused { status: 404"), true, false)]
#[case::refused_401_is_unauthorized(false, 401, Err("Http(Refused { status: 401"), false, true)]
#[case::headers_sent(true, 200, Ok(b"binary".to_vec()), false, false)]
#[tokio::test]
async fn gitlab_packages_serve_artifacts(
    #[case] authorised: bool,
    #[case] status: u16,
    #[case] expected: Result<Vec<u8>, &str>,
    #[case] not_found: bool,
    #[case] unauthorized: bool,
) {
    let server = MockServer::start().await;
    let served = ResponseTemplate::new(status).set_body_bytes("binary");
    Mock::given(method("GET"))
        .and(path(PACKAGE))
        .respond_with(served)
        .mount(&server)
        .await;
    let store = gitlab(&server, authorised).await.packages();
    let version = Version::new(0, 4, 2);

    let opened = store.open(&(), artifact(&version)).await;
    let served: Result<Vec<u8>, GitlabError> = match opened {
        Ok(stream) => {
            stream
                .try_fold(Vec::new(), |mut all, chunk| async move {
                    all.extend_from_slice(&chunk);
                    Ok(all)
                })
                .await
        }
        Err(e) => Err(e),
    };

    assert_outcome(&served, &expected);
    assert_eq!(
        served.as_ref().err().is_some_and(StoreError::not_found),
        not_found
    );
    assert_eq!(
        served.as_ref().err().is_some_and(StoreError::unauthorized),
        unauthorized
    );
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|r| r.headers.contains_key("authorization") == authorised)
    );
}

#[rstest]
#[case::uploaded(true, 201, Ok(()), &["PUT"])]
#[case::anonymous_upload_is_read_only(false, 201, Err("Http(ReadOnly"), &[])]
#[case::refused_403_is_unauthorized(true, 403, Err("Http(Refused { status: 403"), &["PUT"])]
#[tokio::test]
async fn gitlab_packages_upload(
    #[case] authorised: bool,
    #[case] status: u16,
    #[case] expected: Result<(), &str>,
    #[case] requests: &[&str],
) {
    let server = MockServer::start().await;
    let token = header("authorization", "Bearer t0ken");
    Mock::given(method("PUT"))
        .and(path(PACKAGE))
        .and(token)
        .respond_with(ResponseTemplate::new(status))
        .mount(&server)
        .await;
    let store = gitlab(&server, authorised).await.packages();
    let location = store.location(artifact(&Version::new(0, 4, 2)));

    let uploaded = store.upload(&(), &location, b"binary".to_vec()).await;

    assert_outcome(&uploaded, &expected);
    assert_eq!(
        uploaded
            .as_ref()
            .err()
            .is_some_and(StoreError::unauthorized),
        status == 403
    );
    let received = server.received_requests().await.unwrap();
    assert_eq!(
        received
            .iter()
            .map(|r| r.method.to_string())
            .collect::<Vec<_>>(),
        requests
    );
    assert!(received.iter().all(|r| r.body == b"binary"));
}
