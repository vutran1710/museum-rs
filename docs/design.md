# museum: design

Status: implemented in `museum` 0.1.0 and `museum-cli` 0.1.0. The README is the user guide; this
document records what the code guarantees and why it is split the way it is.

## 1. Purpose

A host program runs separate executables (drivers, plugins, agents) that are released on their own
schedule, for several platforms. museum is the library both sides use:

- **Hosts** ask for packages by semver range. museum resolves one version per package for the host's
  target triple and interface version, downloads it, proves it is the file the publisher signed, and
  installs it atomically.
- **Release pipelines** add a version, built for one or more targets. museum refuses to change a
  published version, signs the index, and uploads the files.

museum does not start, supervise or talk to the executables, and does not decide which ones a host
needs.

## 2. Terms

| Term | Meaning |
|---|---|
| package | A named executable, e.g. `modbus`. |
| version | A release's semantic version. Semver is mandatory. |
| target | A Rust target triple. `msvc` and `gnu` are different targets. `HOST_TARGET` is Cargo's exact `TARGET`, captured by `build.rs`. |
| interface version | An integer naming the host-to-package interface. A release is usable only if it equals the host's. |
| store | Where executables live and how access is obtained (`Store`). |
| index store | Where the one index file and its `.minisig` live (`IndexStore`). |
| format | What the index file looks like (`ReleaseIndex`). |

## 3. Shape

```
Registry<S: Store, X: ReleaseIndex>
  store: S                 executables: bootstrap() -> Session, open(Session, Artifact) -> bytes, location(Artifact)
  index: X                 the empty index; a format holding X::Store: IndexStore (read/write one file)
  session                  bootstrapped lazily, shared, replaced once on unauthorized
  loaded index             read, signature-verified, decoded once
  options                  trusted minisign keys, download directory
```

Three independent choices, each a trait a user can implement:

| Trait | Owns | Built in |
|---|---|---|
| `Store` (+ `StoreWriter` to publish) | executables, access, bootstrap | `GithubReleases` |
| `IndexStore` | finding and keeping the index file | `GithubReleaseFile`, `GithubFile`, `HttpFile`, `LocalFile` |
| `ReleaseIndex` | the file format only, no I/O | `Json`, `Yaml`, `Toml` |

Everything that must hold for every combination lives once in the core: bootstrapping, signature
verification, resolution, the immutability check, streaming with hashing, the cache, atomic installs.

Errors are `Error<SE, XE, IE>`: the store's, the format's and the index store's own error types come
back typed, aliased as `RegistryError<S, X>`.

## 4. Resolution

`resolve(index, package, target, interface, wants)` is pure:

1. Candidates: releases with the host's exact interface and a digest for the target.
2. The highest candidate matching every want (duplicates count once) wins.
3. None, and at least two distinct wants each match some candidate on their own: `Conflict`.
   Otherwise `NoSuchBuild`.

Pre-release rules are `semver::VersionReq::matches`'s. Packages do not depend on each other.

## 5. Fetching

1. Installed name: `<package>-<version><EXE_SUFFIX>` in the download directory. A relative directory is
   resolved against the current directory once, in `Registry::new`.
2. An existing file that re-hashes to the digest is kept (`from_cache`), with no store call.
3. Otherwise the artifact streams into `<name>.part` while being hashed. Any failure removes the
   `.part`. A digest mismatch is `DigestMismatch`.
4. On Unix the file gets mode `0755`; then it is renamed over any existing file.
5. `fetch_all` merges wants per package, reads the index once, and downloads each package once, in
   parallel.

## 6. Authenticity

- minisign (Ed25519), detached `.minisig` next to the index file.
- The signature is verified over the raw bytes before the format decodes them, for every store and
  format, private stores included. A token proves who reads, not who published.
- Hosts pass a list of trusted public keys, so a key is rotated by trusting old and new for a while.
- A missing, malformed, untrusted or non-matching signature is `BadSignature`. There is no unsigned
  fallback.
- Not covered: rollback, meaning someone serving an older, validly signed index.

## 7. Publishing

`Registry::init(secret_key)` writes an empty signed index and refuses if one exists.
`Registry::publish(release, secret_key)`, for a store implementing `StoreWriter`:

1. Hash each built file.
2. Read and verify the current index; a missing index starts from the empty one.
3. An existing version with different files is `Immutable`; identical files are a no-op.
4. Upload the executables, then write the re-signed index last, so a host never sees an entry whose
   file is missing.

## 8. GitHub

`Github::public("owner/repo")` reads anonymously; `Github::private("owner/repo", token_env)` reads and
writes through the API with a token. From one handle:

| Call | Gives | Layout |
|---|---|---|
| `gh.releases()` | executables `Store` | release `<package>-v<version>`, asset `<prefix><package>-<version>-<target>[.exe]` |
| `gh.release_file("<tag>/<name>")` | `IndexStore` | asset `<name>` + `<name>.minisig` of release `<tag>` |
| `gh.file("<ref>/<path>")` | `IndexStore` | file committed at a branch, tag or commit |

`.exe` follows the target, not the machine doing the work. Addresses are parsed up front
(`RepoAddress`, `FileAddress`); the first segment of a file address is the ref, so a branch name with
`/` cannot be addressed. One repository can host several registries when their index files and
prefixes differ.

## 9. CLI

`museum init` generates a password-protected key (password from `MUSEUM_KEY_PASSWORD`), reserves the
key file and `museum.toml` before uploading anything, writes an empty signed index, then writes both
files. `museum publish` reads `museum.toml` and takes the secret key from `--secret-key` or
`MUSEUM_SECRET_KEY`, since its location is per machine. The index lives in the `index` release, or
with `index_branch` is committed to the repository.

## 10. Quality bar

- Every test is an `rstest` table; HTTP is tested against a real local server, never a mocked client.
- `cargo llvm-cov --workspace --all-features --fail-under-lines 100`.
- `cargo +nightly fmt`, one `use` per item, no glob imports; clippy `-D warnings`; no `unwrap`/`expect`
  outside tests; `thiserror` for public errors.

## 11. Open questions

| # | Question | Today |
|---|---|---|
| Q1 | Rollback protection | Not covered. A generation number in the signature's trusted comment, refused when lower than the host last saw, would close it. |
| Q2 | More built-in stores (GitLab, S3, plain HTTPS directories) | Implement `Store` / `IndexStore`. |
| Q3 | Pruning old versions from the download directory | The host deletes files not in its current set. |
| Q4 | Builds of the example `hello` for Linux and Windows | Published for macOS only; would need cross-compilation in CI. |
