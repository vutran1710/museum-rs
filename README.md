<p align="center">
  <img src="assets/logo.svg" width="112" alt="museum logo">
</p>

<h1 align="center">museum</h1>

<p align="center">
  A registry for executables your program downloads at runtime:<br>
  semver resolution, one build per target, verified digests, atomic installs.
</p>

<p align="center">
  <a href="https://crates.io/crates/museum"><img src="https://img.shields.io/crates/v/museum.svg" alt="crates.io"></a>
  <a href="https://docs.rs/museum"><img src="https://docs.rs/museum/badge.svg" alt="docs.rs"></a>
  <a href="https://github.com/vutran1710/museum-rs/actions/workflows/publish.yml"><img src="https://github.com/vutran1710/museum-rs/actions/workflows/publish.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/coverage-100%25%20lines-brightgreen" alt="coverage">
</p>

Your program runs plugins, drivers or agents that ship on their own schedule. Publish them with `museum publish`; your program asks museum for `modbus ^0.4` and gets the right build for its platform, checked and installed.

<p align="center"><img src="assets/how-it-works.svg" alt="How museum works: the pipeline uploads executables then the index; the host reads the index, resolves a version, downloads, checks and installs it." width="100%"></p>

## Fetch, from your program

```sh
cargo add museum
```

```rust
use museum::{HOST_TARGET, Json, Options, Registry, VersionReq};
use museum::github::Github;

let gh = Github::new("acme/plugins")?.prefix("acme-");
let index = Json::new(gh.release_file("index/index.json")?);
let registry = Registry::github(&gh, index, Options { download_dir: "drivers".into() })?;

let wanted = [("modbus".to_string(), VersionReq::parse("^0.4")?)];
for (package, fetched) in registry.fetch_all(&wanted, HOST_TARGET, 2).await {
    println!("{package}: {}", fetched?.path.display());   // drivers/modbus-0.4.2
}
```

A public registry needs nothing else. For a private one, pass the header GitHub expects: `Github::new(..)?.headers(..)` with `Authorization: Bearer <token>`.

## Publish, from your pipeline

Get `museum` from the [releases page](https://github.com/vutran1710/museum-rs/releases) (Linux, macOS, Windows), or with `cargo install museum-cli`.

```sh
export GITHUB_TOKEN=$(gh auth token)

museum init --registry https://github.com/acme/plugins --index index.json --prefix acme-   # once
museum publish --package modbus --version 0.4.2 --interface 2 \
  --file x86_64-unknown-linux-gnu=target/release/modbus \
  --file x86_64-pc-windows-msvc=target/x86_64-pc-windows-msvc/release/modbus.exe
```

`init` writes a `museum.toml` to commit. Published versions never change; publishing the same files again does nothing.

## Swap any piece

<p align="center"><img src="assets/pieces.svg" alt="Registry is built from a Store for executables and a ReleaseIndex for the index format, which holds the IndexStore that finds the index file." width="100%"></p>

GitHub and JSON, YAML, TOML are built in. Implement `Store`, `IndexStore` or `ReleaseIndex` to keep executables on S3, the index on a CDN, or use another file format. [`examples/local_registry.rs`](examples/local_registry.rs) does the whole cycle with a custom store, in about 120 lines.

## What you can rely on

- **Right build:** the newest version matching every requirement for a package, for the exact target triple (`msvc` ≠ `gnu`) and interface version.
- **Intact file:** sha256 checked while streaming; a mismatch or a failed download leaves nothing behind.
- **Atomic install:** the file appears under its real name only once complete; a cached copy is re-hashed before reuse.
- **No museum-specific auth:** access is the store's business, through the headers it already uses.

## More

[API docs](https://docs.rs/museum) · [design notes](docs/design.md) · [examples](examples) · licensed MIT or Apache-2.0
