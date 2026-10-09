//! Pure version resolution: the newest release matching every want for one package, the host's
//! target and its exact interface version.

use semver::Version;
use semver::VersionReq;

use crate::index::Digest;
use crate::index::ReleaseIndex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Build {
    pub package: String,
    pub version: Version,
    pub target: String,
    pub interface: u32,
    pub digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Unresolvable {
    #[error("no build of {package} for {target} speaking interface {interface}")]
    NoSuchBuild {
        package: String,
        target: String,
        interface: u32,
    },
    #[error("no single version of {package} satisfies {wants:?}")]
    Conflict {
        package: String,
        wants: Vec<VersionReq>,
    },
}

pub fn resolve<X: ReleaseIndex>(
    index: &X,
    package: &str,
    target: &str,
    interface: u32,
    wants: &[VersionReq],
) -> Result<Build, Unresolvable> {
    let mut distinct: Vec<VersionReq> = Vec::new();
    for want in wants {
        if !distinct.contains(want) {
            distinct.push(want.clone());
        }
    }
    let candidates: Vec<(Version, Digest)> = index
        .releases(package)
        .into_iter()
        .filter(|(_, release)| release.interface == interface)
        .filter_map(|(version, mut release)| {
            release
                .targets
                .remove(target)
                .map(|digest| (version, digest))
        })
        .collect();
    let chosen = candidates
        .iter()
        .filter(|(version, _)| distinct.iter().all(|want| want.matches(version)))
        .max_by(|a, b| a.0.cmp(&b.0));
    match chosen {
        Some((version, digest)) => Ok(Build {
            package: package.to_owned(),
            version: version.clone(),
            target: target.to_owned(),
            interface,
            digest: digest.clone(),
        }),
        None if distinct.len() > 1
            && distinct
                .iter()
                .all(|want| candidates.iter().any(|(version, _)| want.matches(version))) =>
        {
            Err(Unresolvable::Conflict {
                package: package.to_owned(),
                wants: distinct,
            })
        }
        None => Err(Unresolvable::NoSuchBuild {
            package: package.to_owned(),
            target: target.to_owned(),
            interface,
        }),
    }
}
