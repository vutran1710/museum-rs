//! The `museum` binary against a local server standing in for GitHub and GitLab: `init` starts a
//! registry and writes museum.toml, `publish` uploads a release driven by that config.

#![allow(clippy::unwrap_used)]

#[path = "../../tests/common/github.rs"]
mod github;

use std::path::Path;
use std::process::Command;
use std::process::Output;

use github::Answer;
use github::Release;
use github::fake_github;
use github::uploads;
use github::writes;
use rstest::rstest;
use tempfile::TempDir;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path_regex;

const LINUX: &str = "x86_64-unknown-linux-gnu";
const GITHUB: &str = "https://github.com/acme/plugins";
const GITLAB: &str = "https://gitlab.com/acme/plugins";

fn museum(dir: &Path, args: &[&str], token: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_museum"));
    command
        .current_dir(dir)
        .args(args)
        .env_remove("GITHUB_TOKEN");
    if token {
        command.env("GITHUB_TOKEN", "t0ken");
    }
    command.output().unwrap()
}

fn assert_refused(output: &Output, expected: Result<(), &str>) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.success(), expected.is_ok(), "{stderr}");
    assert!(
        stderr.contains(expected.err().unwrap_or_default()),
        "{stderr}"
    );
}

/// The fake GitHub, plus GitLab's API taking every write.
async fn fake_forges(existing: &[Release<'_>]) -> MockServer {
    let server = fake_github(existing, Answer::Normal).await;
    for written in ["PUT", "POST"] {
        Mock::given(method(written))
            .and(path_regex("^/api/v4/"))
            .respond_with(ResponseTemplate::new(201))
            .mount(&server)
            .await;
    }
    server
}

const FRESH: &[Release<'_>] = &[];
const INITIALISED: &[Release<'_>] = &[("index", &[("index.json", b"{}")])];

#[rstest]
#[case::fresh_repository_initialised(GITHUB, FRESH, false, None, Ok(()), &["POST /repos/acme/plugins/releases", "POST /upload/index?name=index.json"])]
#[case::initialised_repository_refused(GITHUB, INITIALISED, false, None, Err("the registry already has an index"), &[])]
#[case::index_in_repo_initialised(GITHUB, FRESH, false, Some("main"), Ok(()), &["PUT /repos/acme/plugins/contents/index.json"])]
#[case::existing_config_refused(GITHUB, FRESH, true, None, Err("museum.toml"), &[])]
#[case::gitlab_index_in_repo_initialised(GITLAB, FRESH, false, Some("main"), Ok(()), &["POST /api/v4/projects/acme%2Fplugins/repository/files/index.json"])]
#[tokio::test]
async fn cli_init_writes_config_and_index(
    #[case] registry: &str,
    #[case] existing: &[Release<'_>],
    #[case] config_exists: bool,
    #[case] index_branch: Option<&str>,
    #[case] expected: Result<(), &str>,
    #[case] requests: &[&str],
) {
    let (dir, server) = (TempDir::new().unwrap(), fake_forges(existing).await);
    let api = server.uri();
    if config_exists {
        std::fs::write(dir.path().join("museum.toml"), b"in use").unwrap();
    }
    let mut args = vec!["init", "--registry", registry, "--api-url", &api];
    args.extend(
        index_branch
            .map(|branch| ["--index-branch", branch])
            .into_iter()
            .flatten(),
    );

    let output = museum(dir.path(), &args, true);

    assert_refused(&output, expected);
    assert_eq!(writes(&server).await, requests);
    assert_eq!(
        dir.path().join("museum.toml").exists(),
        expected.is_ok() || config_exists
    );
    assert_eq!(
        uploads(&server).await.get("index.json").map(Vec::as_slice),
        expected.ok().map(|_| b"{}".as_slice())
    );
}

fn publish_writes(registry: &str, index: &str, index_branch: Option<&str>) -> Vec<String> {
    match (registry, index_branch) {
        (GITLAB, _) => vec![
            format!(
                "PUT /api/v4/projects/acme%2Fplugins/packages/generic/hello/0.1.0/acme-hello-{LINUX}"
            ),
            format!("PUT /api/v4/projects/acme%2Fplugins/packages/generic/index/1.0.0/{index}"),
        ],
        (_, None) => vec![
            "POST /repos/acme/plugins/releases".to_owned(),
            format!("POST /upload/hello-v0.1.0?name=acme-hello-0.1.0-{LINUX}"),
            "POST /repos/acme/plugins/releases".to_owned(),
            format!("POST /upload/index?name={index}"),
        ],
        (_, Some(_)) => vec![
            "POST /repos/acme/plugins/releases".to_owned(),
            format!("POST /upload/hello-v0.1.0?name=acme-hello-0.1.0-{LINUX}"),
            format!("PUT /repos/acme/plugins/contents/{index}"),
        ],
    }
}

#[rstest]
#[case::json_registry(GITHUB, Some("index.json"), None, true, Ok(()))]
#[case::yaml_registry(GITHUB, Some("index.yaml"), None, true, Ok(()))]
#[case::toml_registry(GITHUB, Some("index.toml"), None, true, Ok(()))]
#[case::unknown_index_extension_refused(
    GITHUB,
    Some("index.ini"),
    None,
    true,
    Err("index must end in .json, .yaml or .toml")
)]
#[case::missing_config_refused(GITHUB, None, None, true, Err("museum.toml"))]
#[case::index_in_repo_published(GITHUB, Some("index.json"), Some("main"), true, Ok(()))]
#[case::missing_token_refused(
    GITHUB,
    Some("index.json"),
    None,
    false,
    Err("set GITHUB_TOKEN to an access token")
)]
#[case::gitlab_registry_published(GITLAB, Some("index.json"), None, true, Ok(()))]
#[tokio::test]
async fn cli_publishes_from_config(
    #[case] registry: &str,
    #[case] index: Option<&str>,
    #[case] index_branch: Option<&str>,
    #[case] token: bool,
    #[case] expected: Result<(), &str>,
) {
    let (dir, server) = (TempDir::new().unwrap(), fake_forges(&[]).await);
    std::fs::write(dir.path().join("hello"), b"hello").unwrap();
    if let Some(index) = index {
        let branch = index_branch
            .map(|branch| format!("index_branch = \"{branch}\"\n"))
            .unwrap_or_default();
        let config = format!(
            "registry = \"{registry}\"\nindex = \"{index}\"\nprefix = \"acme-\"\napi_url = \"{}\"\ntoken_env = \"GITHUB_TOKEN\"\n{branch}",
            server.uri()
        );
        std::fs::write(dir.path().join("museum.toml"), config).unwrap();
    }
    let built = format!("{LINUX}=hello");
    let args = [
        "publish",
        "--package",
        "hello",
        "--version",
        "0.1.0",
        "--interface",
        "1",
        "--file",
        &built,
    ];

    let output = museum(dir.path(), &args, token);

    assert_refused(&output, expected);
    let expected_writes = if expected.is_ok() {
        publish_writes(registry, index.unwrap(), index_branch)
    } else {
        Vec::new()
    };
    assert_eq!(writes(&server).await, expected_writes);
}
