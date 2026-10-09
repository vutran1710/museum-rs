//! Addresses inside GitHub, written the way GitHub's raw URLs read: `owner/repo` for a repository,
//! `<ref or tag>/<path>` for one file in it.

use std::fmt;
use std::str::FromStr;

use crate::github::GithubError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoAddress {
    pub owner: String,
    pub repo: String,
}

/// `reference` is a branch, tag or commit for a committed file, and a release tag for an asset.
/// A branch containing `/` cannot be written this way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileAddress {
    pub reference: String,
    pub path: String,
}

fn segments<'a>(address: &'a str, expected: &str) -> Result<Vec<&'a str>, GithubError> {
    let segments: Vec<&str> = address.trim_matches('/').split('/').collect();
    match segments.iter().any(|segment| segment.is_empty()) {
        true => Err(GithubError::BadAddress {
            address: address.to_owned(),
            expected: expected.to_owned(),
        }),
        false => Ok(segments),
    }
}

impl FromStr for RepoAddress {
    type Err = GithubError;

    fn from_str(address: &str) -> Result<Self, GithubError> {
        let expected = "owner/repo";
        match segments(address, expected)?[..] {
            [owner, repo] => Ok(Self {
                owner: owner.to_owned(),
                repo: repo.to_owned(),
            }),
            _ => Err(GithubError::BadAddress {
                address: address.to_owned(),
                expected: expected.to_owned(),
            }),
        }
    }
}

impl FromStr for FileAddress {
    type Err = GithubError;

    fn from_str(address: &str) -> Result<Self, GithubError> {
        let expected = "<ref>/<path>";
        match &segments(address, expected)?[..] {
            [reference, path @ ..] if !path.is_empty() => Ok(Self {
                reference: (*reference).to_owned(),
                path: path.join("/"),
            }),
            _ => Err(GithubError::BadAddress {
                address: address.to_owned(),
                expected: expected.to_owned(),
            }),
        }
    }
}

impl fmt::Display for RepoAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}
