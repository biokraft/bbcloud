# Changelog

All notable changes to this project are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.9.5](https://github.com/biokraft/bbcloud/compare/v0.9.4...v0.9.5) - 2026-08-11

## [0.9.4](https://github.com/biokraft/bbcloud/compare/v0.9.3...v0.9.4) - 2026-08-11

### Fixed

- *(api)* follow same-origin redirects so `pr diff` works

## [0.9.2](https://github.com/biokraft/bbcloud/compare/v0.9.1...v0.9.2) - 2026-08-06

### Documentation

- describe the steady-state release flow now that the first release has shipped

## 0.9.1

### Fixed

- The release checksum asset was named incorrectly, which made `bb update`'s self-update path and
  the `install.sh` installer unable to verify a downloaded binary.

## 0.9.0

First public pre-release.

### Added

- Pull requests: `bb pr list`, `view`, `diff`, `files`, `commits`, `create`, `comment`,
  `request-changes` and `no-request-changes`. `bb pr view --unresolved` shows only the comment
  threads that still need action, and `bb pr comment` posts general, inline and reply comments.
- Branches: `bb branch list`, filterable by last-commit author or name.
- `bb browse` opens a repository, pull request or branch page without invoking a shell.
- `bb completions` for bash, zsh, fish, powershell and elvish.
- `bb update` checks the latest release and either updates a standalone binary in place, after
  verifying its checksum, or prints the correct command for a Homebrew- or cargo-managed install.
- `--json` on every command, with stdout carrying only the serde value so output is safe to pipe
  into `jq`.
- Authentication with an Atlassian API token stored in the OS keyring. The token is never printed,
  never written to disk, and never sent anywhere except `api.bitbucket.org`. `BB_EMAIL` and
  `BB_TOKEN` cover CI and headless machines.
- Installation via Homebrew, crates.io, `cargo binstall`, prebuilt binaries for macOS (arm64,
  x86_64) and Linux (x86_64, aarch64), or the install script.
