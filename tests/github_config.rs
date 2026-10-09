//! The GitHub registry string and release layout: what is accepted, what is refused with which
//! message, and where each object lives.

#![cfg(all(feature = "github", feature = "json"))]
#![allow(clippy::unwrap_used)]

mod common;

use common::LINUX;
use common::WIN;
use museum::Location;
use museum::Object;
use museum::Store;
use museum::Version;
use museum::github::GITHUB;
use museum::github::GithubConfig;
use museum::github::GithubPublic;
use museum::github::reqwest;
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

#[derive(Clone, Copy)]
enum Kind {
    Index,
    Signature,
    Artifact(&'static str),
}

#[rstest]
#[case::index(Kind::Index, "", "index", "index.json")]
#[case::signature(Kind::Signature, "", "index", "index.json.minisig")]
#[case::artifact_linux(
    Kind::Artifact(LINUX),
    "",
    "modbus-v0.4.2",
    "modbus-0.4.2-x86_64-unknown-linux-gnu"
)]
#[case::windows_target_gets_exe(
    Kind::Artifact(WIN),
    "",
    "modbus-v0.4.2",
    "modbus-0.4.2-x86_64-pc-windows-msvc.exe"
)]
#[case::prefix_applied(
    Kind::Artifact(LINUX),
    "acme-",
    "modbus-v0.4.2",
    "acme-modbus-0.4.2-x86_64-unknown-linux-gnu"
)]
fn github_location_follows_layout(
    #[case] kind: Kind,
    #[case] prefix: &str,
    #[case] group: &str,
    #[case] file: &str,
) {
    let store = GithubPublic::new(reqwest::Client::new(), GITHUB, "acme", "plugins").prefix(prefix);
    let version = Version::new(0, 4, 2);
    let object = match kind {
        Kind::Index => Object::Index {
            file_name: "index.json",
        },
        Kind::Signature => Object::Signature {
            file_name: "index.json",
        },
        Kind::Artifact(target) => Object::Artifact {
            package: "modbus",
            version: &version,
            target,
        },
    };
    assert_eq!(
        store.location(object),
        Location {
            group: group.into(),
            file: file.into()
        }
    );
}
