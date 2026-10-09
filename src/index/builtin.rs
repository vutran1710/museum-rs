//! JSON, YAML and TOML indexes: one serde model, `package → version → release`, written by three
//! serde crates. Each holds the `IndexStore` its file lives in.

use std::collections::BTreeMap;

use semver::Version;

use crate::index::Release;
use crate::index::ReleaseIndex;
use crate::index::store::IndexStore;

type Model = BTreeMap<String, BTreeMap<Version, Release>>;

macro_rules! builtin_index {
    ($name:ident, $error:ty, $decode:expr, $encode:expr) => {
        #[derive(Clone, Debug)]
        pub struct $name<S> {
            store: S,
            releases: Model,
        }

        impl<S: IndexStore> $name<S> {
            pub fn new(store: S) -> Self {
                Self {
                    store,
                    releases: Model::new(),
                }
            }
        }

        impl<S: IndexStore> ReleaseIndex for $name<S> {
            type Store = S;
            type Error = $error;

            fn store(&self) -> &S {
                &self.store
            }
            fn decode(&self, bytes: &[u8]) -> Result<Self, Self::Error> {
                Ok(Self {
                    store: self.store.clone(),
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
    Json,
    serde_json::Error,
    serde_json::from_slice::<Model>,
    serde_json::to_vec_pretty
);

#[cfg(feature = "yaml")]
builtin_index!(
    Yaml,
    serde_yaml_ng::Error,
    serde_yaml_ng::from_slice::<Model>,
    |m: &Model| { serde_yaml_ng::to_string(m).map(String::into_bytes) }
);

#[cfg(feature = "toml")]
#[derive(Debug, thiserror::Error)]
pub enum TomlError {
    #[error(transparent)]
    Decode(#[from] toml::de::Error),
    #[error(transparent)]
    Encode(#[from] toml::ser::Error),
}

#[cfg(feature = "toml")]
builtin_index!(
    Toml,
    TomlError,
    |b: &[u8]| toml::from_slice::<Model>(b).map_err(TomlError::from),
    |m: &Model| toml::to_string_pretty(m)
        .map(String::into_bytes)
        .map_err(TomlError::from)
);
