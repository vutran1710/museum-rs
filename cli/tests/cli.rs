//! The `museum` binary against a local server standing in for GitHub: `init` starts a registry and
//! writes museum.toml, `publish` uploads a release driven by that config.

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

const LINUX: &str = "x86_64-unknown-linux-gnu";

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

const FRESH: &[Release<'_>] = &[];
const INITIALISED: &[Release<'_>] = &[("index", &[("index.json", b"{}")])];

#[rstest]
#[case::fresh_repository_initialised(FRESH, false, None, Ok(()), &["POST /repos/acme/plugins/releases", "POST /upload/index?name=index.json"])]
#[case::initialised_repository_refused(INITIALISED, false, None, Err("the registry already has an index"), &[])]
#[case::index_in_repo_initialised(FRESH, false, Some("main"), Ok(()), &["PUT /repos/acme/plugins/contents/index.json"])]
#[case::existing_config_refused(FRESH, true, None, Err("museum.toml"), &[])]
#[tokio::test]
async fn cli_init_writes_config_and_index(
    #[case] existing: &[Release<'_>],
    #[case] config_exists: bool,
    #[case] index_branch: Option<&str>,
    #[case] expected: Result<(), &str>,
    #[case] requests: &[&str],
) {
    let (dir, server) = (
        TempDir::new().unwrap(),
        fake_github(existing, Answer::Normal).await,
    );
    let api = server.uri();
    if config_exists {
        std::fs::write(dir.path().join("museum.toml"), b"in use").unwrap();
    }
    let mut args = vec![
        "init",
        "--registry",
        "https://github.com/acme/plugins",
        "--api-url",
        &api,
    ];
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

fn publish_writes(index: &str, index_branch: Option<&str>) -> Vec<String> {
    let artifact = [
        "POST /repos/acme/plugins/releases".to_owned(),
        format!("POST /upload/hello-v0.1.0?name=acme-hello-0.1.0-{LINUX}"),
    ];
    let index = match index_branch {
        None => vec![
            "POST /repos/acme/plugins/releases".to_owned(),
            format!("POST /upload/index?name={index}"),
        ],
        Some(_) => vec![format!("PUT /repos/acme/plugins/contents/{index}")],
    };
    [artifact.to_vec(), index].concat()
}

#[rstest]
#[case::json_registry(Some("index.json"), None, true, Ok(()))]
#[case::yaml_registry(Some("index.yaml"), None, true, Ok(()))]
#[case::toml_registry(Some("index.toml"), None, true, Ok(()))]
#[case::unknown_index_extension_refused(
    Some("index.ini"),
    None,
    true,
    Err("index must end in .json, .yaml or .toml")
)]
#[case::missing_config_refused(None, None, true, Err("museum.toml"))]
#[case::index_in_repo_published(Some("index.json"), Some("main"), true, Ok(()))]
#[case::missing_token_refused(
    Some("index.json"),
    None,
    false,
    Err("set GITHUB_TOKEN to a GitHub token")
)]
#[tokio::test]
async fn cli_publishes_from_config(
    #[case] index: Option<&str>,
    #[case] index_branch: Option<&str>,
    #[case] token: bool,
    #[case] expected: Result<(), &str>,
) {
    let (dir, server) = (
        TempDir::new().unwrap(),
        fake_github(&[], Answer::Normal).await,
    );
    std::fs::write(dir.path().join("hello"), b"hello").unwrap();
    if let Some(index) = index {
        let branch = index_branch
            .map(|branch| format!("index_branch = \"{branch}\"\n"))
            .unwrap_or_default();
        let config = format!(
            "registry = \"https://github.com/acme/plugins\"\nindex = \"{index}\"\nprefix = \"acme-\"\napi_url = \"{}\"\ntoken_env = \"GITHUB_TOKEN\"\n{branch}",
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
        publish_writes(index.unwrap(), index_branch)
    } else {
        Vec::new()
    };
    assert_eq!(writes(&server).await, expected_writes);
}
