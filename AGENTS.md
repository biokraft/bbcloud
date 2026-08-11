# AGENTS.md

Guidance for AI coding agents working in this repository. `CLAUDE.md` is a symlink to this file,
because Claude Code reads that name.

## What this is

`bb` is a Bitbucket Cloud REST API CLI, written in Rust. It is an independent rewrite of an
unmaintained PHP client of the same name; see `NOTICE` for attribution. The rewrite is why several
hard rules below exist — the shell-invocation and cleartext-credential rules in particular encode
mistakes this implementation must not repeat.

**Auth context you must not get wrong:** Atlassian removed Bitbucket Cloud app passwords on
2026-07-28. Authentication is HTTP Basic with the **Atlassian account email** as the username
and an **API token** as the password. Never add, document, or suggest app passwords — a test
(`tests/cli.rs::help_does_not_mention_app_passwords`) fails the build if help text mentions them.

## Build and check

Cargo is installed via rustup and may not be on `PATH`. If `cargo` is not found:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

All three of these must exit 0 before any change is done:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

Toolchain is pinned to 1.97 in `rust-toolchain.toml`; MSRV is 1.88. Do not lower the pin —
current `keyring`, `reqwest`, and `wiremock` all require newer than 1.75.

## Development workflow

**Branches.** Start every change on a branch off `main`, named for what it is: `feat/…`,
`fix/…`, `docs/…`, `ci/…`. Never commit to `main` directly — it is protected and requires a pull
request with passing checks.

**Before opening a pull request**, run these locally and get all three green:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

One caveat, because it bites: CI additionally sets `RUSTFLAGS: -D warnings` for the whole
workflow, so a warning clippy tolerates locally can still turn CI red. If CI fails on a warning
you did not see locally, that is why.

**The pull request.** CI runs five jobs, and all must pass before merge: `test (macos-latest)`
and `test (ubuntu-latest)` cover the two supported platforms; `msrv 1.88` guards the version
floor the README promises, and needs `RUSTUP_TOOLCHAIN` set because otherwise `rust-toolchain.toml`'s
1.97 pin would override the action's toolchain choice; `coverage` uploads to Codecov and now
fails the build if the upload itself fails, because a silently skipped upload once left the
badge reading "unknown" for weeks; `publish dry run` catches crates.io packaging problems before
a real release depends on them.

**Commit messages are functional, not cosmetic.** release-plz parses them to build the changelog
and pick the next version: `feat:` earns a minor bump, `fix:` a patch, a `!` suffix or a
`BREAKING CHANGE:` trailer a major, and `test:`/`chore:`/`ci:` are skipped from the changelog
entirely. Get the prefix wrong and you get a wrong version number, so it matters more here than
in a repo where the changelog is written by hand.

**Releases are automated end to end. Never do any of these by hand:** edit `version` in
`Cargo.toml`, write `CHANGELOG.md`, or create a git tag. Merging to `main` makes release-plz open
a release PR carrying the version bump and generated changelog; merging *that* PR tags the
commit, publishes to crates.io, builds the four target binaries with their checksums, and updates
the Homebrew formula in `biokraft/homebrew-tap`.

Two prohibitions specific to this repository's history:

- **Never force-push `main`.** It was force-pushed once, while the repository was still private
  and being prepared; that period is over. Published tags point into the current history, and
  other people's clones descend from it.
- **Never `git push --tags`.** Tags come only from release-plz.

The `docs/` tree — design specs and execution records — lives on a separate remote outside this
repository, which is why you will not find it here.

## Hard rules

These are enforced by lints or tests. Breaking one breaks the build.

- **No `.unwrap()` / `.expect()` / `panic!` in `src/**` outside `#[cfg(test)]`.** `Cargo.toml`
  sets `clippy::unwrap_used = "deny"` and `expect_used = "deny"` package-wide. Test code may
  unwrap freely: `#[cfg(test)] mod tests` blocks inside `src/` carry a narrowly scoped
  `#[allow(clippy::unwrap_used)]`; files under `tests/` carry it crate-level.
- **No `#[allow(dead_code)]` anywhere in `src/`.** There is a library target, so `pub` items are
  reachable by definition and never warn. If something you add warns as dead, that is a signal it
  should not exist yet.
- **`#![forbid(unsafe_code)]`** in both `src/lib.rs` and `src/main.rs`.
- **Never invoke a shell.** All subprocess work goes through `std::process::Command` with an
  explicit argument vector (see `src/git.rs`), or through `open::that_detached` for URLs. No
  `sh -c`, no `format!` assembling a command line. The PHP version's `exec()` on a git-remote-derived
  string was a remote code execution bug; do not reintroduce the shape.
- **`reqwest`'s TLS is rustls only.** It is declared `default-features = false, features = ["json", "rustls-tls"]`.
  No OpenSSL in the HTTP path.
- **The Linux keyring backend does link OpenSSL.** `keyring`'s `vendored` feature (`Cargo.toml`)
  vendors both dbus and OpenSSL for the secret-service backend, so `openssl`, `openssl-sys`, and
  `openssl-src` appear in `Cargo.lock` and are compiled into the binary. This is a deliberate
  tradeoff, not an oversight: it is what lets `cargo install bbcloud` work on a stock Linux machine
  with no system packages.
- **Redirects are disabled** (`reqwest::redirect::Policy::none()` in `src/api/mod.rs`) so the
  `Authorization` header can never be replayed to another host. Every request carries a 10s connect
  and 30s total timeout.

## Secrets

The API token is the thing this codebase most needs to not leak.

- The token is always carried as `secrecy::SecretString`, never a plain `String` field.
- Anything shown to a user goes through `secret::redact`, which reveals at most the last four
  characters and nothing at all below 8 characters.
- `Credentials` has a **hand-written** `Debug` that prints `<redacted>`. Do not replace it with
  `#[derive(Debug)]`.
- Storage is the OS keyring only (`src/credentials.rs`). Never write a token to a file.
- `bb auth status` must never print the token. `bb auth show` does not exist and must not be added.
- Prefer `--body-stdin` style flags for secrets so they stay out of shell history and out of `ps`
  output.

When you touch anything credential-adjacent, add a test that asserts the secret string is absent
from the combined stdout+stderr, in **both** human and `--json` mode. That pattern is used in
`tests/auth.rs`.

## Architecture

```
src/main.rs            clap tree + dispatch, error rendering, exit codes
src/lib.rs             library target (so integration tests can import)
src/error.rs           BbError (thiserror) + exit_code()
src/secret.rs          redact(), SecretString re-export
src/credentials.rs     keyring get/set/delete, env override, legacy PHP config path
src/git.rs             injection-safe git invocation
src/repo.rs            RepoSlug parse/resolve, percent-encoded path(), browse_url()
src/api/mod.rs         Client: auth header, pagination, error mapping
src/api/models.rs      serde models, all Option-tolerant with documented fallbacks
src/output.rs          Format, tables, color, spinners, relative_time
src/commands/*.rs      one module per command group
```

**`Ctx` is the shared per-command context** and lives in `src/commands/pr.rs`:

```rust
pub struct Ctx { pub client: Client, pub slug: RepoSlug, pub format: Format }
```

`branch` imports it from there. That placement is historical — `Ctx` is repo-scoped rather than
PR-scoped, so moving it to its own module would be a reasonable cleanup, but do not move it as a
drive-by change since several modules depend on the path.

**Always build API URLs through `ctx.path(suffix)`.** It delegates to `api::repo_path` and applies
percent-encoding exactly once. Do not concatenate URLs by hand.

**Comment resolution is thread-scoped.** `POST` and `DELETE` on `…/comments/{id}/resolve` act on the
whole thread, so the id has to be the root of an inline thread — the comment with no `parent`, which
is why `bb pr view` exposes `parent`. The response body carries only the resolution, so
`pr_comments::resolve` discards it. The endpoint is strict:

| Status | Cause |
|---|---|
| 403 | the id is not a top-level comment, or the comment is not on the diff |
| 404 | the comment does not exist; on `DELETE`, also when it was never resolved |
| 409 | the thread is already resolved |

`pr_comments::resolvable` refuses a reply id and a general comment before the request goes out,
because `check()` renders every 403 as "the token may lack the required scope" — the wrong diagnosis
for both. Keep the two in step. The check runs only on the prompt path, since `--yes` skips the
lookup it needs.

**The resolve gate is deliberate.** `bb pr resolve` confirms with a human unless `--yes` is passed,
and errors instead of prompting when there is no terminal. Resolving hides a reviewer's point, and
the api cannot tell whether the point was addressed, so it stays a human decision like approval and
merge. Three properties to preserve: the confirmation happens **before** the `POST`
(`tests/pr_comment.rs` asserts this with `expect(0)`), the comment lookup that fills the prompt does
not run under `--yes`, and the `inquire` prompt stays on stderr so `--json` stdout remains pure. The
agent skill carries the matching rule — the agent never resolves on its own initiative — and must
stay in step. `unresolve` is not gated: it restores a point rather than hides one.

**Use `Client::paginate`** rather than hand-rolling a page loop. It follows `next` and caps at 100 pages.

## JSON output

Every command supports `--json`. The contract is strict: **in JSON mode, stdout contains only the
serde value.**

The catch: `output::print_table`, `success`, `info`, `warn`, and `heading` are **not** format-aware.
`print_table` even prints a "nothing to show" line on zero rows. So purity depends on every call
site gating itself:

```rust
match ctx.format {
    Format::Json => output::print_json(&rows)?,
    Format::Human => output::print_table(&["..."], rows),
}
```

Spinners write to stderr and are safe alongside JSON, but finish/clear them before printing.

When adding a command, test the **empty-result** case in JSON mode — that is where a stray prose
line usually escapes.

## Errors and exit codes

`BbError::exit_code()` is the single source of truth, and the README's table must match it:

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | general error |
| 2 | not authenticated |
| 3 | not found |

The HTTP layer already maps 401 → `Auth`, 403/429 → `Api`, 404 → `NotFound`, so command code should
not inspect status codes. `check()` deliberately prefers the API's own `error.message` and never
echoes a raw response body.

## Interaction conventions

- Never block on input that will not arrive. If stdin is not a terminal and a required value was not
  supplied, return an error naming the flag to use — do not prompt. Integration tests run with piped
  stdin and will hang the suite otherwise.
- Use `inquire::Password` (masked, no confirmation) for secrets and `inquire::Editor` for
  multi-paragraph prose like PR descriptions and comments. The `editor` feature is enabled.
- A browser that fails to open must never fail a command that already succeeded — discard the result
  of `open::that_detached`.

## Testing

- HTTP is tested against `wiremock`, never live Bitbucket. Point the client at a stub with
  `BB_API_BASE`.
- Integration tests drive the real binary via `assert_cmd`, setting `BB_EMAIL`, `BB_TOKEN`,
  `BB_REPO`, `BB_API_BASE`, and `NO_COLOR=1`.
- `BB_KEYRING_DISABLE=1` forces the keyring lookup to fail, for deterministic unauthenticated tests.
- Assert on parsed JSON structure rather than substrings where practical.
- Write tests that would actually fail if the behaviour regressed. A test that passes against the
  un-fixed code proves nothing.

## Environment variables

| Variable | Purpose |
|---|---|
| `BB_EMAIL`, `BB_TOKEN` | credentials for non-interactive/CI use, checked before the keyring |
| `BB_REPO` | default repository, same as `-R/--repo` |
| `BB_API_BASE` | override the API base URL (testing) |
| `BB_UPDATE_API_BASE` | override the release-lookup API base URL for `bb update` (testing) |
| `BB_KEYRING_DISABLE` | force keyring lookup failure (testing) |
| `NO_COLOR` | disable color and spinners |

## Releasing

Releases are automated. **Never hand-edit `version` in `Cargo.toml`, never write `CHANGELOG.md`
by hand, and never create a tag manually** — release-plz owns all three, and a manual edit
desynchronises the crate version from the git tag.

### Release flow

To cut a release:

1. Land the change on `main` with a conventional-commit message. `feat`, `fix`, `docs`, `refactor`
   and `perf` appear in the changelog; `test`, `chore` and `ci` are skipped. A `!` suffix
   (`feat!:`) marks a breaking change and forces a minor/major bump.
2. `release-plz` opens or updates a **release PR** carrying the version bump and the generated
   `CHANGELOG.md` entries, computed from the conventional commits since the last release. Review
   it like any other PR. The version is chosen by release-plz from the commit prefixes, not by
   hand: since the project is pre-1.0, a `feat:` commit produces a minor bump (e.g. 0.9.x →
   0.10.0) and a `fix:` produces a patch (e.g. 0.9.1 → 0.9.2). The one exception is a milestone
   version such as 1.0.0, once the project is ready for it — the maintainer sets that version
   deliberately as a conscious decision, not as routine practice.
3. Merge the release PR. That triggers, in order: the git tag `v<version>`, the GitHub Release,
   `cargo publish` to crates.io, then `release.yml` builds the four target triples and attaches
   `bbcloud-v<version>-<triple>.tar.gz` archives, each with its own `<archive>.sha256` file (there
   is no combined `SHA256SUMS` manifest), then the `formula` job renders a multi-platform
   `Formula/bb.rb` covering all four triples and pushes it straight to `biokraft/homebrew-tap`
   (no PR to merge — it authenticates with `TAP_PAT`).
4. `brew install biokraft/tap/bb` now serves the new version, on any of the four supported
   platforms.

Two constraints that are easy to break:

- The tag shape lives in **two** places — `git_tag_name = "v{{ version }}"` in `release-plz.toml`
  and the `tags: ["v*"]` trigger in `release.yml`. Change one and binaries stop being built.
- `release-plz.yml` uses `RELEASE_PLZ_TOKEN`, not the default `GITHUB_TOKEN`. A tag pushed with
  `GITHUB_TOKEN` cannot trigger `release.yml`, so the release would ship with no binaries.

### Bootstrapping the tap (first release only)

`release.yml`'s `formula` job pushes into `biokraft/homebrew-tap`, so that repository and an
initial `Formula/bb.rb` must exist before the first tag is pushed — a human creates this by hand,
once. Use the same multi-platform shape the automated job renders, and after the first release's
assets are up, replace every `REPLACE_WITH_SHA256_OF_<triple>` below with the contents of that
target's `.sha256` file from the release:

```ruby
class Bb < Formula
  desc "Bitbucket Cloud CLI"
  homepage "https://github.com/biokraft/bbcloud"
  license "MIT"
  version "0.0.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/biokraft/bbcloud/releases/download/v0.0.0/bbcloud-v0.0.0-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256_OF_aarch64-apple-darwin"
    else
      url "https://github.com/biokraft/bbcloud/releases/download/v0.0.0/bbcloud-v0.0.0-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256_OF_x86_64-apple-darwin"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/biokraft/bbcloud/releases/download/v0.0.0/bbcloud-v0.0.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_SHA256_OF_aarch64-unknown-linux-gnu"
    else
      url "https://github.com/biokraft/bbcloud/releases/download/v0.0.0/bbcloud-v0.0.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_SHA256_OF_x86_64-unknown-linux-gnu"
    end
  end

  def install
    bin.install "bb"
  end

  test do
    system "#{bin}/bb", "--version"
  end
end
```

### Repository secrets

| Secret | Unlocks |
|---|---|
| `RELEASE_PLZ_TOKEN` | the release PR, the tag, and the tag triggering `release.yml` |
| `CARGO_REGISTRY_TOKEN` | `cargo publish` to crates.io |
| `TAP_PAT` | pushing the generated formula straight to the tap (fine-grained: contents write, tap repo only) |
| `CODECOV_TOKEN` | the coverage upload; a missing value degrades gracefully |

### Fixing a bad release

Do not delete or move a published tag, and do not yank the crate for a cosmetic problem — crates.io
versions are immutable and Homebrew has already cached the URL. Land the fix as a normal `fix:`
commit and let release-plz cut the next patch.
