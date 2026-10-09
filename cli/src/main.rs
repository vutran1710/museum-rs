//! `museum`: start a registry in a GitHub repository or GitLab project (`init`) and publish
//! releases into it (`publish`), driven by a `museum.toml` that `init` writes. The index lives in
//! the `index` release (GitHub) or the `index/1.0.0` generic package (GitLab), or with
//! `index_branch` is committed to the repository. The registry logic lives in `museum`.

use std::error::Error;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use clap::Parser;
use clap::Subcommand;
use museum::IndexStore;
use museum::Json;
use museum::NewRelease;
use museum::Options;
use museum::Registry;
use museum::ReleaseIndex;
use museum::StoreWriter;
use museum::Toml;
use museum::Version;
use museum::Yaml;
use museum::github::GITHUB;
use museum::github::GITHUB_API;
use museum::github::GITHUB_RAW;
use museum::github::Github;
use museum::github::GithubConfig;
use museum::github::GithubReleases;
use museum::gitlab::Gitlab;
use museum::gitlab::GitlabPackages;
use museum::headers;

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
    /// Create museum.toml and an empty index in the repository.
    Init(InitArgs),
    /// Add a release: upload the executables, then the updated index.
    Publish(PublishArgs),
}

#[derive(Args)]
struct InitArgs {
    /// `https://github.com/<owner>/<repo>`, or a GitLab project `https://<host>/<group>/<project>`.
    #[arg(long)]
    registry: String,
    #[arg(long, default_value = "index.json")]
    index: String,
    #[arg(long, default_value = "")]
    prefix: String,
    /// The API root, for GitHub Enterprise or a GitLab reached at another address.
    #[arg(long)]
    api_url: Option<String>,
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
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    registry: String,
    index: String,
    prefix: String,
    api_url: Option<String>,
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
    let token = std::env::var(&config.token_env)
        .map_err(|_| format!("set {} to an access token", config.token_env))?;
    let headers = headers::bearer(&token)?;
    match config.registry.starts_with("https://github.com/") {
        true => {
            let GithubConfig { owner, repo, .. } = config.registry.parse()?;
            let api = config.api_url.as_deref().unwrap_or(GITHUB_API);
            let gh = Github::new(&format!("{owner}/{repo}"))?
                .headers(headers)
                .prefix(&config.prefix)
                .enterprise(GITHUB, api, GITHUB_RAW);
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
        false => {
            let rest = config
                .registry
                .trim_start_matches("https://")
                .trim_end_matches('/');
            let (host, project) = rest.split_once('/').unwrap_or((rest, ""));
            let host = config.api_url.clone().unwrap_or(format!("https://{host}"));
            let gl = Gitlab::new(project)?
                .host(&host)
                .headers(headers)
                .prefix(&config.prefix);
            match config.index_branch.clone() {
                None => {
                    with_format(
                        &gl,
                        gl.package_file(&format!("index/1.0.0/{}", config.index))?,
                        config,
                        &cli,
                    )
                    .await
                }
                Some(branch) => {
                    with_format(
                        &gl,
                        gl.file(&format!("{branch}/{}", config.index))?,
                        config,
                        &cli,
                    )
                    .await
                }
            }
        }
    }
}

/// A forge the CLI publishes to; its registry keeps executables in the forge's own store.
trait Forge {
    type Store: StoreWriter;
    fn registry<X: ReleaseIndex>(
        &self,
        index: X,
        options: Options,
    ) -> Result<Registry<Self::Store, X>, Box<dyn Error>>;
}

impl Forge for Github {
    type Store = GithubReleases;
    fn registry<X: ReleaseIndex>(
        &self,
        index: X,
        options: Options,
    ) -> Result<Registry<GithubReleases, X>, Box<dyn Error>> {
        Ok(Registry::github(self, index, options)?)
    }
}

impl Forge for Gitlab {
    type Store = GitlabPackages;
    fn registry<X: ReleaseIndex>(
        &self,
        index: X,
        options: Options,
    ) -> Result<Registry<GitlabPackages, X>, Box<dyn Error>> {
        Ok(Registry::gitlab(self, index, options)?)
    }
}

async fn with_format<F: Forge, I: IndexStore>(
    forge: &F,
    index: I,
    config: Config,
    cli: &Cli,
) -> Result<(), Box<dyn Error>> {
    match config
        .index
        .rsplit_once('.')
        .map(|(_, extension)| extension)
    {
        Some("json") => run(forge, Json::new(index), config, cli).await,
        Some("yaml" | "yml") => run(forge, Yaml::new(index), config, cli).await,
        Some("toml") => run(forge, Toml::new(index), config, cli).await,
        _ => Err(format!(
            "index must end in .json, .yaml or .toml, not '{}'",
            config.index
        )
        .into()),
    }
}

async fn run<F: Forge, X: ReleaseIndex>(
    forge: &F,
    index: X,
    config: Config,
    cli: &Cli,
) -> Result<(), Box<dyn Error>> {
    let options = Options {
        download_dir: PathBuf::from("."),
    };
    let registry = forge.registry(index, options)?;
    match &cli.command {
        Command::Init(_) => {
            let mut config_file = create_new(&cli.config)?;
            let uploads = registry.init().await.inspect_err(|_| {
                let _ = std::fs::remove_file(&cli.config);
            })?;
            config_file.write_all(toml::to_string_pretty(&config)?.as_bytes())?;
            uploads
                .iter()
                .for_each(|location| println!("uploaded {location}"));
            println!("wrote {}", cli.config.display());
        }
        Command::Publish(args) => {
            let release = NewRelease {
                package: &args.package,
                version: args.version.clone(),
                interface: args.interface,
                files: &args.files,
            };
            let published = registry.publish(release).await?;
            published
                .uploads
                .iter()
                .for_each(|location| println!("uploaded {location}"));
            println!("index changed: {}", published.changed);
        }
    }
    Ok(())
}

/// Refuses to overwrite: a second `init` must never replace a config in use.
fn create_new(path: &Path) -> std::io::Result<std::fs::File> {
    let options = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .clone();
    options
        .open(path)
        .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", path.display())))
}
