//! JSON, YAML and TOML indexes: one serde model, `package → version → release`, written by three
//! serde crates.

use std::collections::BTreeMap;

use semver::Version;

use crate::index::Release;
use crate::index::ReleaseIndex;

type Model = BTreeMap<String, BTreeMap<Version, Release>>;

macro_rules! builtin_index {
    ($name:ident, $default_file:literal, $error:ty, $decode:expr, $encode:expr) => {
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name {
            file_name: String,
            releases: Model,
        }

        impl $name {
            pub fn named(file_name: &str) -> Self {
                Self {
                    file_name: file_name.to_owned(),
                    releases: Model::new(),
                }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::named($default_file)
            }
        }

        impl ReleaseIndex for $name {
            type Error = $error;

            fn file_name(&self) -> &str {
                &self.file_name
            }
            fn decode(&self, bytes: &[u8]) -> Result<Self, Self::Error> {
                Ok(Self {
                    file_name: self.file_name.clone(),
                    releases: $decode(bytes)?,
                })
            }
            fn encode(&self) -> Result<Vec<u8>, Self::Error> {
                $encode(&self.releases)
            }
            fn packages(&self) -> Vec<String> {
                self.releases.keys().cloned().collect()
            }
            fn releases(&self, package: &str) -> Vec<(Version, Release)> {
                self.releases
                    .get(package)
                    .into_iter()
                    .flatten()
                    .map(|(v, r)| (v.clone(), r.clone()))
                    .collect()
            }
            fn insert(&mut self, package: &str, version: Version, release: Release) {
                self.releases
                    .entry(package.to_owned())
                    .or_default()
                    .insert(version, release);
            }
        }
    };
}

#[cfg(feature = "json")]
builtin_index!(
    JsonIndex,
    "index.json",
    serde_json::Error,
    serde_json::from_slice::<Model>,
    serde_json::to_vec_pretty
);

#[cfg(feature = "yaml")]
builtin_index!(
    YamlIndex,
    "index.yaml",
    serde_yaml_ng::Error,
    serde_yaml_ng::from_slice::<Model>,
    |m: &Model| serde_yaml_ng::to_string(m).map(String::into_bytes)
);

#[cfg(feature = "toml")]
#[derive(Debug, thiserror::Error)]
pub enum TomlIndexError {
    #[error(transparent)]
    Decode(#[from] toml::de::Error),
    #[error(transparent)]
    Encode(#[from] toml::ser::Error),
}

#[cfg(feature = "toml")]
builtin_index!(
    TomlIndex,
    "index.toml",
    TomlIndexError,
    |b: &[u8]| toml::from_slice::<Model>(b).map_err(TomlIndexError::from),
    |m: &Model| toml::to_string_pretty(m)
        .map(String::into_bytes)
        .map_err(TomlIndexError::from)
);
