//! Fetches `hello` from the YAML registry that lives in this repository's own releases,
//! verifies its digest, and runs it. The prefix is read from examples/yaml/museum.toml.

use std::error::Error;
use std::time::Duration;

use museum::HOST_TARGET;
use museum::Options;
use museum::Registry;
use museum::VersionReq;
use museum::Yaml;
use museum::github::Github;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config: toml::Table = include_str!("yaml/museum.toml").parse()?;
    let prefix = config["prefix"]
        .as_str()
        .ok_or("museum.toml has no prefix")?;

    let gh = Github::new("vutran1710/museum-rs")?
        .prefix(prefix)
        .timeout(Duration::from_secs(30));
    let index = Yaml::new(gh.release_file("index/index.yaml")?);
    let options = Options {
        download_dir: "target/museum/yaml".into(),
    };
    let registry = Registry::github(&gh, index, options)?;

    let wanted = [("hello".to_owned(), VersionReq::STAR)];
    for (package, fetched) in registry.fetch_all(&wanted, HOST_TARGET, 1).await {
        let fetched = fetched?;
        println!(
            "{package}: {} ({} bytes, from cache: {})",
            fetched.path.display(),
            fetched.bytes,
            fetched.from_cache
        );
        std::process::Command::new(&fetched.path).status()?;
    }
    Ok(())
}
