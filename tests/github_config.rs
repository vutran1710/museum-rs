//! The GitHub registry string and release layout: what is accepted, what is refused with which
//! message, and where each object lives.

#![cfg(all(feature = "github", feature = "json"))]
#![allow(clippy::unwrap_used)]

mod common;

use common::LINUX;
use common::WIN;
use museum::Artifact;
use museum::Location;
use museum::Store;
use museum::Version;
use museum::github::FileAddress;
use museum::github::Github;
use museum::github::GithubConfig;
use rstest::rstest;

fn public(owner: &str, repo: &str) -> Result<GithubConfig, String> {
    Ok(GithubConfig::Public {
        owner: owner.into(),
        repo: repo.into(),
    })
}

const NOT_HTTPS: &str = "a registry is written https://<host>/<path>";
const NOT_REPO: &str = "a github.com registry is https://github.com/<org>/<repository>";

#[rstest]
#[case::public("https://github.com/acme/plugins", public("acme", "plugins"))]
#[case::trailing_slash_dropped("https://github.com/acme/plugins/", public("acme", "plugins"))]
#[case::private("https://github.com/acme/plugins?token_env=ACME_TOKEN", Ok(GithubConfig::Private { owner: "acme".into(), repo: "plugins".into(), token_env: "ACME_TOKEN".into() }))]
#[case::no_scheme_refused("github.com/acme/plugins", Err(NOT_HTTPS.into()))]
#[case::plaintext_refused("http://github.com/acme/plugins", Err(NOT_HTTPS.into()))]
#[case::other_host_refused("https://gitlab.com/acme/plugins", Err("registries are read at github.com — not at 'gitlab.com'".into()))]
#[case::org_only_refused("https://github.com/acme", Err(NOT_REPO.into()))]
#[case::extra_segment_refused("https://github.com/acme/plugins/extra", Err(NOT_REPO.into()))]
fn registry_string_parses_alike_from_config_file_and_flag(
    #[case] written: &str,
    #[case] expected: Result<GithubConfig, String>,
) {
    let from_flag = written.parse::<GithubConfig>().map_err(|e| e.to_string());
    let from_file =
        serde_json::from_str::<GithubConfig>(&format!("\"{written}\"")).map_err(|e| e.to_string());
    assert_eq!(from_flag, expected);
    assert_eq!(from_file.is_ok(), expected.is_ok());
    if let Ok(config) = from_file {
        assert_eq!(
            serde_json::to_string(&config).unwrap(),
            format!("\"{}\"", written.trim_end_matches('/'))
        );
    }
}

#[rstest]
#[case::artifact_linux(LINUX, "", "modbus-v0.4.2", "modbus-0.4.2-x86_64-unknown-linux-gnu")]
#[case::windows_target_gets_exe(
    WIN,
    "",
    "modbus-v0.4.2",
    "modbus-0.4.2-x86_64-pc-windows-msvc.exe"
)]
#[case::prefix_applied(
    LINUX,
    "acme-",
    "modbus-v0.4.2",
    "acme-modbus-0.4.2-x86_64-unknown-linux-gnu"
)]
fn github_location_follows_layout(
    #[case] target: &str,
    #[case] prefix: &str,
    #[case] group: &str,
    #[case] file: &str,
) {
    let store = Github::public("acme/plugins")
        .unwrap()
        .prefix(prefix)
        .releases();
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

#[derive(Clone, Copy)]
enum Address {
    File,
    Repo,
}

const NOT_A_FILE: &str = "is not a GitHub address; expected <ref>/<path>";

#[rstest]
#[case::file_address(Address::File, "main/index.json", Ok("main | index.json"))]
#[case::nested_file_path(
    Address::File,
    "v1.2/registry/index.yaml",
    Ok("v1.2 | registry/index.yaml")
)]
#[case::release_file_address(Address::File, "index/index.json", Ok("index | index.json"))]
#[case::releases_address(Address::Repo, "acme/plugins", Ok("acme/plugins"))]
#[case::file_without_path_refused(Address::File, "main", Err(NOT_A_FILE))]
#[case::releases_with_extra_segment_refused(
    Address::Repo,
    "acme/plugins/x",
    Err("is not a GitHub address; expected owner/repo")
)]
#[case::empty_segment_refused(Address::File, "main//index.json", Err(NOT_A_FILE))]
fn github_addresses_parse(
    #[case] kind: Address,
    #[case] written: &str,
    #[case] expected: Result<&str, &str>,
) {
    let parsed = match kind {
        Address::File => written
            .parse::<FileAddress>()
            .map(|a| format!("{} | {}", a.reference, a.path)),
        Address::Repo => Github::public(written).map(|_| written.to_owned()),
    };
    match (parsed, expected) {
        (Ok(parsed), Ok(expected)) => assert_eq!(parsed, expected),
        (Err(error), Err(message)) => {
            assert_eq!(error.to_string(), format!("'{written}' {message}"))
        }
        (parsed, expected) => panic!("got {parsed:?}, expected {expected:?}"),
    }
}
