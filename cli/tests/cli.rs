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
use github::writes;
use rstest::rstest;
use tempfile::TempDir;

const LINUX: &str = "x86_64-unknown-linux-gnu";

fn museum(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_museum"))
        .current_dir(dir)
        .args(args)
        .env("GITHUB_TOKEN", "t0ken")
        .env("MUSEUM_KEY_PASSWORD", "pw")
        .output()
        .unwrap()
}

fn refusal(output: &Output) -> Result<(), String> {
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

const FRESH: &[Release<'_>] = &[];
const INITIALISED: &[Release<'_>] = &[("index", &[("index.json", b"{}")])];

#[rstest]
#[case::fresh_repository_initialised(FRESH, false, None, Ok(()), &["POST /repos/acme/plugins/releases", "POST /upload/index?name=index.json", "POST /upload/index?name=index.json.minisig"])]
#[case::initialised_repository_refused(INITIALISED, false, None, Err("the registry already has an index"), &[])]
#[case::key_file_exists_refused(FRESH, true, None, Err("keys/museum.key"), &[])]
#[case::index_in_repo_initialised(FRESH, false, Some("main"), Ok(()), &["PUT /repos/acme/plugins/contents/index.json", "PUT /repos/acme/plugins/contents/index.json.minisig"])]
#[tokio::test]
async fn cli_init_writes_config_and_index(
    #[case] existing: &[Release<'_>],
    #[case] key_exists: bool,
    #[case] index_branch: Option<&str>,
    #[case] expected: Result<(), &str>,
    #[case] requests: &[&str],
) {
    let (dir, server) = (
        TempDir::new().unwrap(),
        fake_github(existing, Answer::Normal).await,
    );
    let api = server.uri();
    if key_exists {
        std::fs::create_dir_all(dir.path().join("keys")).unwrap();
        std::fs::write(dir.path().join("keys/museum.key"), b"in use").unwrap();
    }

    let mut args = vec![
        "init",
        "--registry",
        "https://github.com/acme/plugins",
        "--api-url",
        &api,
        "--secret-key",
        "keys/museum.key",
    ];
    args.extend(
        index_branch
            .map(|branch| ["--index-branch", branch])
            .into_iter()
            .flatten(),
    );
    let output = museum(dir.path(), &args);

    let outcome = refusal(&output);
    assert!(
        outcome
            .as_ref()
            .err()
            .is_none_or(|e| e.contains(expected.err().unwrap_or_default())),
        "{outcome:?}"
    );
    assert_eq!(outcome.is_ok(), expected.is_ok());
    assert_eq!(writes(&server).await, requests);
    assert_eq!(dir.path().join("museum.toml").exists(), expected.is_ok());
    if let Ok(config) = std::fs::read_to_string(dir.path().join("museum.toml")) {
        let public_key = config
            .split("public_keys = [\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let uploaded = github::uploads(&server).await;
        let signature = minisign::SignatureBox::from_string(&String::from_utf8_lossy(
            &uploaded["index.json.minisig"],
        ))
        .unwrap();
        let public_key = minisign::PublicKey::from_base64(public_key).unwrap();
        minisign::verify(
            &public_key,
            &signature,
            std::io::Cursor::new(&uploaded["index.json"]),
            true,
            false,
            false,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.path().join("keys/museum.key"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
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
            format!("POST /upload/index?name={index}.minisig"),
        ],
        Some(_) => vec![
            format!("PUT /repos/acme/plugins/contents/{index}"),
            format!("PUT /repos/acme/plugins/contents/{index}.minisig"),
        ],
    };
    [artifact.to_vec(), index].concat()
}

#[rstest]
#[case::json_registry(Some("index.json"), None, Ok(()))]
#[case::yaml_registry(Some("index.yaml"), None, Ok(()))]
#[case::toml_registry(Some("index.toml"), None, Ok(()))]
#[case::unknown_index_extension_refused(
    Some("index.ini"),
    None,
    Err("index must end in .json, .yaml or .toml")
)]
#[case::missing_config_refused(None, None, Err("museum.toml"))]
#[case::index_in_repo_published(Some("index.json"), Some("main"), Ok(()))]
#[tokio::test]
async fn cli_publishes_from_config(
    #[case] index: Option<&str>,
    #[case] index_branch: Option<&str>,
    #[case] expected: Result<(), &str>,
) {
    let (dir, server) = (
        TempDir::new().unwrap(),
        fake_github(&[], Answer::Normal).await,
    );
    let keys = minisign::KeyPair::generate_encrypted_keypair(Some("pw".into())).unwrap();
    std::fs::write(
        dir.path().join("museum.key"),
        keys.sk.to_box(None).unwrap().into_string(),
    )
    .unwrap();
    std::fs::write(dir.path().join("hello"), b"hello").unwrap();
    if let Some(index) = index {
        let config = format!(
            "registry = \"https://github.com/acme/plugins\"\nindex = \"{index}\"\nprefix = \"acme-\"\npublic_keys = [\"{}\"]\nsecret_key = \"museum.key\"\napi_url = \"{}\"\ntoken_env = \"GITHUB_TOKEN\"\n",
            keys.pk.to_base64(),
            server.uri()
        );
        let branch = index_branch
            .map(|branch| format!("index_branch = \"{branch}\"\n"))
            .unwrap_or_default();
        std::fs::write(dir.path().join("museum.toml"), config + &branch).unwrap();
    }

    let output = museum(
        dir.path(),
        &[
            "publish",
            "--package",
            "hello",
            "--version",
            "0.1.0",
            "--interface",
            "1",
            "--file",
            &format!("{LINUX}=hello"),
        ],
    );

    let outcome = refusal(&output);
    assert!(
        outcome
            .as_ref()
            .err()
            .is_none_or(|e| e.contains(expected.err().unwrap_or_default())),
        "{outcome:?}"
    );
    assert_eq!(outcome.is_ok(), expected.is_ok());
    let expected_writes = if expected.is_ok() {
        publish_writes(index.unwrap(), index_branch)
    } else {
        Vec::new()
    };
    assert_eq!(writes(&server).await, expected_writes);
}
