//! The built-in JSON, YAML and TOML indexes: each reads back what it writes, and refuses malformed
//! or non-semver input.

#![cfg(all(feature = "json", feature = "yaml", feature = "toml"))]
#![allow(clippy::unwrap_used)]

mod common;

use common::assert_outcome;
use common::fixture;
use museum::JsonIndex;
use museum::ReleaseIndex;
use museum::TomlIndex;
use museum::YamlIndex;
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

fn decode<X: ReleaseIndex>(mut empty: X, input: Option<&str>) -> Decoded {
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
    empty = empty
        .decode(&bytes)
        .map_err(|e| Refusal(format!("{e:?}")))?;
    assert!(
        empty
            .packages()
            .iter()
            .all(|p| empty.releases(p) == written.releases(p))
    );
    let counts = empty
        .packages()
        .into_iter()
        .map(|p| (p.clone(), empty.releases(&p).len()))
        .collect();
    Ok((empty.file_name().to_owned(), counts))
}

fn read_back(file_name: &str) -> Result<Contents, &str> {
    Ok((
        file_name.to_owned(),
        vec![("modbus".to_owned(), 5), ("opcua".to_owned(), 1)],
    ))
}

#[rstest]
#[case::json(Format::Json, None, read_back("index.json"))]
#[case::yaml(Format::Yaml, None, read_back("index.yaml"))]
#[case::toml(Format::Toml, None, read_back("index.toml"))]
#[case::json_not_json(Format::Json, Some("not json"), Err("Error(\"expected ident\""))]
#[case::json_bad_semver_key(
    Format::Json,
    Some(r#"{"modbus":{"x.y":{"interface":2,"targets":{}}}}"#),
    Err("Error(\"unexpected character")
)]
#[case::yaml_bad_semver_key(
    Format::Yaml,
    Some("modbus:\n  x.y:\n    interface: 2\n    targets: {}\n"),
    Err("Error(\"modbus: unexpected character")
)]
#[case::toml_bad_semver_key(
    Format::Toml,
    Some("[modbus.\"x.y\"]\ninterface = 2\ntargets = {}\n"),
    Err("Decode(")
)]
fn builtin_index_reads_what_it_writes(
    #[case] format: Format,
    #[case] input: Option<&str>,
    #[case] expected: Result<Contents, &str>,
) {
    let decoded = match format {
        Format::Json => decode(JsonIndex::default(), input),
        Format::Yaml => decode(YamlIndex::default(), input),
        Format::Toml => decode(TomlIndex::default(), input),
    };
    assert_outcome(&decoded, &expected);
}
