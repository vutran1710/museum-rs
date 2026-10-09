//! Fetches `hello` from the YAML registry that lives in this repository's own releases,
//! verifies it, and runs it. The trusted key and prefix are read from examples/yaml/museum.toml.

use std::error::Error;
use std::time::Duration;

use museum::HOST_TARGET;
use museum::Museum;
use museum::Options;
use museum::PublicKey;
use museum::VersionReq;
use museum::YamlIndex;
use museum::github::GITHUB;
use museum::github::GithubPublic;
use museum::github::reqwest;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config: toml::Table = include_str!("yaml/museum.toml").parse()?;
    let key = config["public_keys"][0]
        .as_str()
        .ok_or("museum.toml has no public key")?;
    let prefix = config["prefix"]
        .as_str()
        .ok_or("museum.toml has no prefix")?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let store = GithubPublic::new(client, GITHUB, "vutran1710", "museum-rs").prefix(prefix);
    let options = Options {
        trusted_keys: vec![PublicKey::from_base64(key)?],
        download_dir: "target/museum/yaml".into(),
    };
    let museum = Museum::new(store, YamlIndex::default(), options)?;

    let wanted = [("hello".to_owned(), VersionReq::STAR)];
    for (package, fetched) in museum.fetch_all(&wanted, HOST_TARGET, 1).await {
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
