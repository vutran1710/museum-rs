//! Version resolution over the fixture: the newest release matching every want for a package, the
//! host's target and its exact interface version.

#![allow(clippy::unwrap_used)]

mod common;

use common::LINUX;
use common::WIN;
use common::fixture;
use museum::Unresolvable;
use museum::VersionReq;
use museum::resolve;
use rstest::rstest;

fn wants(written: &[&str]) -> Vec<VersionReq> {
    written.iter().map(|w| w.parse().unwrap()).collect()
}

fn no_such_build(
    package: &str,
    target: &str,
    interface: u32,
) -> Result<&'static str, Unresolvable> {
    Err(Unresolvable::NoSuchBuild {
        package: package.into(),
        target: target.into(),
        interface,
    })
}

#[rstest]
#[case::newest_matching_wins("modbus", &["*"], WIN, 2, Ok("0.5.0"))]
#[case::range_caps_the_version("modbus", &["^0.4"], WIN, 2, Ok("0.4.2"))]
#[case::target_missing_falls_back("modbus", &["*"], LINUX, 2, Ok("0.4.2"))]
#[case::other_interface_never_chosen("modbus", &["*"], WIN, 3, Ok("9.0.0"))]
#[case::prerelease_needs_opt_in("modbus", &["^1"], WIN, 2, no_such_build("modbus", WIN, 2))]
#[case::unknown_package("nope", &["*"], WIN, 2, no_such_build("nope", WIN, 2))]
#[case::common_newest_wins("modbus", &["^0.4", ">=0.4.1"], WIN, 2, Ok("0.4.2"))]
#[case::tighter_bound_wins("modbus", &["^0.4", "<0.4.2"], WIN, 2, Ok("0.4.1"))]
#[case::star_does_not_narrow("modbus", &["*", "^0.4"], WIN, 2, Ok("0.4.2"))]
#[case::duplicate_requirement_counts_once("modbus", &["^0.4", "^0.4"], WIN, 2, Ok("0.4.2"))]
#[case::disjoint_ranges_conflict("modbus", &["^0.4", "^0.5"], WIN, 2, Err(Unresolvable::Conflict { package: "modbus".into(), wants: wants(&["^0.4", "^0.5"]) }))]
fn one_version_is_resolved_per_package(
    #[case] package: &str,
    #[case] written: &[&str],
    #[case] target: &str,
    #[case] interface: u32,
    #[case] expected: Result<&str, Unresolvable>,
) {
    let chosen = resolve(&fixture(), package, target, interface, &wants(written));
    assert_eq!(
        chosen.map(|build| build.version.to_string()),
        expected.map(str::to_owned)
    );
}
