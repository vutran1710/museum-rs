//! Fetches `hello` from the JSON registry that lives in this repository's own releases,
//! verifies it, and runs it. The trusted key and prefix are read from examples/json/museum.toml.

use std::error::Error;
use std::time::Duration;

use museum::HOST_TARGET;
use museum::Json;
use museum::Options;
use museum::PublicKey;
use museum::Registry;
use museum::VersionReq;
use museum::github::Github;
use museum::github::reqwest;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config: toml::Table = include_str!("json/museum.toml").parse()?;
    let key = config["public_keys"][0]
        .as_str()
        .ok_or("museum.toml has no public key")?;
    let prefix = config["prefix"]
        .as_str()
        .ok_or("museum.toml has no prefix")?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let gh = Github::public("vutran1710/museum-rs")?
        .prefix(prefix)
        .client(client);
    let index = Json::new(gh.release_file("index/index.json")?);
    let options = Options {
        trusted_keys: vec![PublicKey::from_base64(key)?],
        download_dir: "target/museum/json".into(),
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
