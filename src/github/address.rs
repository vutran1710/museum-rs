//! Addresses inside GitHub, written the way GitHub's raw URLs read: `owner/repo` for a repository,
//! `<ref or tag>/<path>` for one file in it.

use std::fmt;
use std::str::FromStr;

use crate::address::reference_and_path;
use crate::address::segments;
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

fn refused(address: &str, expected: &str) -> GithubError {
    GithubError::BadAddress {
        address: address.to_owned(),
        expected: expected.to_owned(),
    }
}

impl FromStr for RepoAddress {
    type Err = GithubError;

    fn from_str(address: &str) -> Result<Self, GithubError> {
        match segments(address).as_deref() {
            Some([owner, repo]) => Ok(Self {
                owner: (*owner).to_owned(),
                repo: (*repo).to_owned(),
            }),
            _ => Err(refused(address, "owner/repo")),
        }
    }
}

impl FromStr for FileAddress {
    type Err = GithubError;

    fn from_str(address: &str) -> Result<Self, GithubError> {
        let (reference, path) =
            reference_and_path(address).ok_or_else(|| refused(address, "<ref>/<path>"))?;
        Ok(Self { reference, path })
    }
}

impl fmt::Display for RepoAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}
