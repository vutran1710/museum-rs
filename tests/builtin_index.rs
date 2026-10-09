//! The built-in JSON, YAML and TOML formats: each reads back what it writes through its store, and
//! refuses malformed or non-semver input.

#![cfg(all(feature = "json", feature = "yaml", feature = "toml"))]
#![allow(clippy::unwrap_used)]

mod common;

use common::assert_outcome;
use common::fixture;
use museum::IndexStore;
use museum::Json;
use museum::LocalFile;
use museum::ReleaseIndex;
use museum::Signed;
use museum::Toml;
use museum::Yaml;
use rstest::rstest;

#[derive(Clone, Copy)]
enum Format {
    Json,
    Yaml,
    Toml,
}

struct Refusal(String);

impl std::fmt::Debug for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

type Contents = (String, Vec<(String, usize)>);
type Decoded = Result<Contents, Refusal>;

async fn decode<X: ReleaseIndex>(empty: X, input: Option<&str>) -> Decoded {
    let source = fixture();
    let mut written = empty.clone();
    for package in source.packages() {
        for (version, release) in source.releases(&package) {
            written.insert(&package, version, release);
        }
    }
    let bytes = input.map_or_else(
        || written.encode().unwrap(),
        |input| input.as_bytes().to_vec(),
    );
    let files = empty
        .store()
        .write(Signed {
            index: bytes,
            signature: None,
        })
        .await
        .unwrap();
    let read = empty.store().read().await.unwrap();
    let decoded = empty
        .decode(&read.index)
        .map_err(|e| Refusal(format!("{e:?}")))?;
    assert!(
        decoded
            .packages()
            .iter()
            .all(|p| decoded.releases(p) == written.releases(p))
    );
    let counts = decoded
        .packages()
        .into_iter()
        .map(|p| (p.clone(), decoded.releases(&p).len()))
        .collect();
    let file_name = std::path::Path::new(&files[0])
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    Ok((file_name, counts))
}

fn read_back(file_name: &str) -> Result<Contents, &str> {
    Ok((
        file_name.to_owned(),
        vec![("modbus".to_owned(), 5), ("opcua".to_owned(), 1)],
    ))
}

#[rstest]
#[case::json(Format::Json, "index.json", None, read_back("index.json"))]
#[case::yaml(Format::Yaml, "index.yaml", None, read_back("index.yaml"))]
#[case::toml(Format::Toml, "index.toml", None, read_back("index.toml"))]
#[case::json_not_json(
    Format::Json,
    "index.json",
    Some("not json"),
    Err("Error(\"expected ident\"")
)]
#[case::json_bad_semver_key(
    Format::Json,
    "index.json",
    Some(r#"{"modbus":{"x.y":{"interface":2,"targets":{}}}}"#),
    Err("Error(\"unexpected character")
)]
#[case::yaml_bad_semver_key(
    Format::Yaml,
    "index.yaml",
    Some("modbus:\n  x.y:\n    interface: 2\n    targets: {}\n"),
    Err("Error(\"modbus: unexpected character")
)]
#[case::toml_bad_semver_key(
    Format::Toml,
    "index.toml",
    Some("[modbus.\"x.y\"]\ninterface = 2\ntargets = {}\n"),
    Err("Decode(")
)]
#[tokio::test]
async fn builtin_index_reads_what_it_writes(
    #[case] format: Format,
    #[case] file_name: &str,
    #[case] input: Option<&str>,
    #[case] expected: Result<Contents, &str>,
) {
    let dir = tempfile::TempDir::new().unwrap();
    let store = LocalFile::new(dir.path().join(file_name));

    let decoded = match format {
        Format::Json => decode(Json::new(store), input).await,
        Format::Yaml => decode(Yaml::new(store), input).await,
        Format::Toml => decode(Toml::new(store), input).await,
    };

    assert_outcome(&decoded, &expected);
}
