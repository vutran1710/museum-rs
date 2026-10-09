//! `museum`: start a registry in a GitHub repository (`init`) and publish signed releases into it
//! (`publish`), driven by a `museum.toml` that `init` writes. The index lives in the `index`
//! release or, with `index_branch`, is committed to the repository. The registry logic lives in
//! `museum`.

use std::error::Error;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Args;
use clap::Parser;
use clap::Subcommand;
use museum::IndexStore;
use museum::Json;
use museum::NewRelease;
use museum::Options;
use museum::PublicKey;
use museum::Registry;
use museum::ReleaseIndex;
use museum::Toml;
use museum::Version;
use museum::Yaml;
use museum::github::GITHUB;
use museum::github::GITHUB_API;
use museum::github::GITHUB_RAW;
use museum::github::Github;
use museum::github::GithubConfig;
use museum::github::reqwest;

const PASSWORD_ENV: &str = "MUSEUM_KEY_PASSWORD";

#[derive(Parser)]
#[command(name = "museum", version, about)]
struct Cli {
    #[arg(long, global = true, default_value = "museum.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a signing key, museum.toml and an empty signed index in the repository.
    Init(InitArgs),
    /// Add a release: verify the index, upload the executables, then the re-signed index.
    Publish(PublishArgs),
}

#[derive(Args)]
struct InitArgs {
    #[arg(long)]
    registry: GithubConfig,
    #[arg(long, default_value = "index.json")]
    index: String,
    #[arg(long, default_value = "")]
    prefix: String,
    /// Where to write the new secret key. Keep it out of the repository.
    #[arg(long)]
    secret_key: PathBuf,
    #[arg(long, default_value = GITHUB_API)]
    api_url: String,
    #[arg(long, default_value = "GITHUB_TOKEN")]
    token_env: String,
    /// Commit the index on this branch instead of keeping it in the `index` release.
    #[arg(long)]
    index_branch: Option<String>,
}

#[derive(Args)]
struct PublishArgs {
    #[arg(long)]
    package: String,
    #[arg(long)]
    version: Version,
    #[arg(long)]
    interface: u32,
    #[arg(long = "file", value_parser = target_file, required = true)]
    files: Vec<(String, PathBuf)>,
    /// The signing key `init` wrote. Its location is per machine, so it is not in museum.toml.
    #[arg(long, env = "MUSEUM_SECRET_KEY")]
    secret_key: PathBuf,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    registry: GithubConfig,
    index: String,
    prefix: String,
    public_keys: Vec<String>,
    api_url: String,
    token_env: String,
    index_branch: Option<String>,
}

fn target_file(written: &str) -> Result<(String, PathBuf), String> {
    let (target, path) = written.split_once('=').ok_or("expected <target>=<path>")?;
    Ok((target.to_owned(), PathBuf::from(path)))
}

#[tokio::main]
async fn main() -> ExitCode {
    match museum_cli(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("museum: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn museum_cli(cli: Cli) -> Result<(), Box<dyn Error>> {
    let config = match &cli.command {
        Command::Init(args) => Config {
            registry: args.registry.clone(),
            index: args.index.clone(),
            prefix: args.prefix.clone(),
            public_keys: Vec::new(),
            api_url: args.api_url.clone(),
            token_env: args.token_env.clone(),
            index_branch: args.index_branch.clone(),
        },
        Command::Publish(_) => {
            let written = std::fs::read_to_string(&cli.config)
                .map_err(|e| format!("{}: {e}", cli.config.display()))?;
            toml::from_str(&written)?
        }
    };
    let (GithubConfig::Public { owner, repo } | GithubConfig::Private { owner, repo, .. }) =
        &config.registry;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?;
    let gh = Github::private(&format!("{owner}/{repo}"), &config.token_env)?
        .prefix(&config.prefix)
        .client(client);
    let gh = gh.enterprise(GITHUB, &config.api_url, GITHUB_RAW);
    match config.index_branch.clone() {
        None => {
            with_format(
                &gh,
                gh.release_file(&format!("index/{}", config.index))?,
                config,
                &cli,
            )
            .await
        }
        Some(branch) => {
            with_format(
                &gh,
                gh.file(&format!("{branch}/{}", config.index))?,
                config,
                &cli,
            )
            .await
        }
    }
}

async fn with_format<I: IndexStore>(
    gh: &Github,
    store: I,
    config: Config,
    cli: &Cli,
) -> Result<(), Box<dyn Error>> {
    match config
        .index
        .rsplit_once('.')
        .map(|(_, extension)| extension)
    {
        Some("json") => run(gh, Json::new(store), config, cli).await,
        Some("yaml" | "yml") => run(gh, Yaml::new(store), config, cli).await,
        Some("toml") => run(gh, Toml::new(store), config, cli).await,
        _ => Err(format!(
            "index must end in .json, .yaml or .toml, not '{}'",
            config.index
        )
        .into()),
    }
}

async fn run<X: ReleaseIndex>(
    gh: &Github,
    index: X,
    mut config: Config,
    cli: &Cli,
) -> Result<(), Box<dyn Error>> {
    let trusted_keys = config
        .public_keys
        .iter()
        .map(|key| PublicKey::from_base64(key))
        .collect::<Result<_, _>>()?;
    let options = Options {
        trusted_keys,
        download_dir: PathBuf::from("."),
    };
    let registry = Registry::github(gh, index, options)?;
    let password = std::env::var(PASSWORD_ENV).ok();
    match &cli.command {
        Command::Init(args) => {
            let password = password.ok_or(format!("set {PASSWORD_ENV} to protect the new key"))?;
            let keys = minisign::KeyPair::generate_encrypted_keypair(Some(password.clone()))?;
            let stored = keys.sk.to_box(Some("museum signing key"))?;
            let signing_key = minisign::SecretKey::from_box(stored.clone(), Some(password))?;
            let mut key_file = create_new(&args.secret_key, 0o600)?;
            let initialised: Result<_, Box<dyn Error>> = async {
                let config_file = create_new(&cli.config, 0o644)?;
                let uploads = registry.init(&signing_key).await.inspect_err(|_| {
                    let _ = std::fs::remove_file(&cli.config);
                })?;
                Ok((uploads, config_file))
            }
            .await;
            let (uploads, mut config_file) = initialised.inspect_err(|_| {
                let _ = std::fs::remove_file(&args.secret_key);
            })?;
            key_file.write_all(stored.into_string().as_bytes())?;
            config.public_keys.push(keys.pk.to_base64());
            config_file.write_all(toml::to_string_pretty(&config)?.as_bytes())?;
            uploads
                .iter()
                .for_each(|location| println!("uploaded {location}"));
            println!(
                "wrote {} and {}",
                cli.config.display(),
                args.secret_key.display()
            );
        }
        Command::Publish(args) => {
            let secret_key = minisign::SecretKey::from_file(&args.secret_key, password)?;
            let release = NewRelease {
                package: &args.package,
                version: args.version.clone(),
                interface: args.interface,
                files: &args.files,
            };
            let published = registry.publish(release, &secret_key).await?;
            published
                .uploads
                .iter()
                .for_each(|location| println!("uploaded {location}"));
            println!("index changed: {}", published.changed);
        }
    }
    Ok(())
}

/// Refuses to overwrite: a second `init` must never replace a key or config in use.
fn create_new(path: &Path, mode: u32) -> std::io::Result<std::fs::File> {
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, mode);
    #[cfg(not(unix))]
    let _ = mode;
    options
        .open(path)
        .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", path.display())))
}
