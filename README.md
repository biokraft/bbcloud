# bb — Bitbucket Cloud CLI

[![CI](https://github.com/biokraft/bbcloud/actions/workflows/ci.yml/badge.svg)](https://github.com/biokraft/bbcloud/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/biokraft/bbcloud/branch/main/graph/badge.svg)](https://codecov.io/gh/biokraft/bbcloud)
[![crates.io](https://img.shields.io/crates/v/bbcloud.svg)](https://crates.io/crates/bbcloud)
[![release](https://img.shields.io/github/v/release/biokraft/bbcloud?sort=semver)](https://github.com/biokraft/bbcloud/releases/latest)
[![MSRV](https://img.shields.io/badge/MSRV-1.88-blue)](https://github.com/biokraft/bbcloud)
[![license](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-brightgreen)](https://github.com/rust-secure-code/safety-dance)

Open pull requests, read every comment, and write replies — without leaving the shell or opening a
browser tab.

One binary, no runtime to install. Your API token lives in your OS keyring and is never printed,
never written to disk, and never sent anywhere except `api.bitbucket.org` over TLS. `bb update` is
the one command that talks to another host — it queries the GitHub Releases API without sending any
credentials.

```
$ bb pr list
┌────┬──────────────────────────┬───────┬─────────────────┬───┬────────┬────────┬───────────────────────────┐
│ ID ┆ TITLE                    ┆ STATE ┆ SOURCE          ┆ → ┆ TARGET ┆ AUTHOR ┆ REVIEWERS                 │
╞════╪══════════════════════════╪═══════╪═════════════════╪═══╪════════╪════════╪═══════════════════════════╡
│ 42 ┆ Cache session lookups    ┆ Open  ┆ feat/cache      ┆ → ┆ main   ┆ dev    ┆ Patrick ✓, Raigon ✗, Ana · │
│ 41 ┆ Fix token refresh window ┆ Open  ┆ fix/token-clock ┆ → ┆ main   ┆ dev    ┆ Linus ·                   │
└────┴──────────────────────────┴───────┴─────────────────┴───┴────────┴────────┴───────────────────────────┘
```

## Install

```bash
brew install biokraft/tap/bb
```

Recommended: updates via `brew upgrade`, no Rust toolchain needed.

### Alternatives

| Method | Command | Requires |
| --- | --- | --- |
| Install script | `curl -fsSL https://raw.githubusercontent.com/biokraft/bbcloud/main/install.sh \| sh` | Nothing — detects platform, verifies checksum, installs to `~/.local/bin` |
| Prebuilt binary | Download from the [latest release](https://github.com/biokraft/bbcloud/releases/latest) | Manual `PATH` setup; verify against the matching `.sha256` |
| `cargo binstall` | `cargo binstall bbcloud` | `cargo-binstall`, no compiler |
| `cargo install` | `cargo install bbcloud --locked` | Rust 1.88+ (a clone pins 1.97 via `rust-toolchain.toml`) |

Supported targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`.

The cargo routes install `bb` into `~/.cargo/bin` — add that to your `PATH` if the command isn't
found afterwards.

## Authenticate

Atlassian **removed Bitbucket Cloud app passwords on 2026-07-28.** `bb` uses an Atlassian API token,
sent as HTTP Basic auth with your account email as the username.

1. Create a token at <https://id.atlassian.com/manage-profile/security/api-tokens>, selecting the
   scopes below.
2. Run `bb auth login` and paste it — the input is masked and never echoed.

```bash
bb auth login     # prompts, verifies the token, then stores it in the OS keyring
bb auth logout    # removes the stored credentials
bb auth status    # shows the account; the token is always redacted to ****last4
```

### Token scopes

Grant the least you need. For the pull request workflow — listing, reading and commenting — four
scopes are enough:

| Scope | Needed for |
|---|---|
| `read:user:bitbucket` | **mandatory.** `bb auth login` verifies the token against `/user`, so login fails without it |
| `read:pullrequest:bitbucket` | `pr list`, `pr view`, `pr diff`, `pr files`, `pr commits` |
| `write:pullrequest:bitbucket` | `pr create`, `pr comment`, `pr request-changes` |
| `read:repository:bitbucket` | `branch list`, and the default-reviewer lookup `pr create` does |

One gotcha worth knowing: `write:pullrequest:bitbucket` does **not** imply
`read:repository:bitbucket`, so `pr create` needs both.

### CI and headless machines

There is no keyring on a CI runner, and on Linux the keyring backend is secret-service, which is
absent on servers. Set the credentials in the environment instead — they are checked **before** the
keyring, so this also works as a local override:

```bash
export BB_EMAIL='you@example.com'
export BB_TOKEN='...'
bb pr list --json
```

### Check it works

```bash
bb --version
bb auth status                              # exits 2 until you log in
cd any-bitbucket-repo && bb pr list
```

## Usage

`bb --help` lists every command, and `bb <command> --help` documents its flags. The shape is
`bb <noun> <verb>`:

```bash
bb pr list                                # open PRs, with state and per-reviewer decisions
bb pr view 42 --unresolved                # the PR plus comment threads still needing action
bb pr create main --title "Add caching"   # source branch inferred from your checkout
bb pr comment 42 -f src/auth.rs -l 88 -b "off by one"
bb branch list --user alice
bb update                                 # check for a newer release and update
```

`bb update` compares your version against the latest GitHub release. If Homebrew or cargo installed
`bb`, it prints the right upgrade command for that package manager instead of overwriting a file they
manage. For a standalone binary it verifies the download's checksum and replaces itself atomically.

One thing worth knowing that `--help` won't tell you:

**Everything speaks JSON.** Add `--json` to any command and pipe it to `jq` rather than parsing the
tables, whose layout is not a contract. Scripts and agents should default to it.

```bash
bb pr list --json | jq -r '.[] | select(all(.reviewers[]; .state != "approved")) | "\(.id)\t\(.title)"'
```

Shell completions make the rest discoverable:

```bash
bb completions zsh > ~/.zfunc/_bb         # also bash, fish, powershell, elvish
```

## Use it from Claude Code

Copy this whole block into your project's `CLAUDE.md` so your agent drives PR review through `bb`
instead of asking you to open a browser.

```markdown
## Bitbucket via `bb`

This project uses Bitbucket, not GitHub. Use the `bb` CLI for all pull request work — never `gh`,
and never ask the user to open the web UI. Run `bb --help` to discover commands.

### Rules

- **Always pass `--json`** and parse that. Never scrape the human-readable tables; their layout is
  not a contract. The one exception is `pr diff`, where `--json` wraps the whole diff in an escaped
  JSON string — read that one as plain text.
- Never pass `-w`/`--web` — it tries to launch a browser.
- For multi-paragraph comment bodies use `--body-stdin` and pipe the text in. Interior blank lines
  and indentation are preserved; only trailing newlines are trimmed.
- Exit codes are meaningful: 0 success, 1 error, 2 not authenticated, 3 not found. Branch on those
  rather than matching error strings.
- Add `-R workspace/repo` to act on a repository that is not the current checkout.

### Reading review feedback

    bb pr list --json                        # find the PR
    bb pr view <id> --json                   # full PR plus general and inline comments
    bb pr view <id> --unresolved --json      # only threads still needing action
    bb pr diff <id>                          # raw diff
    bb pr files <id> --json                  # changed paths

Each entry under `.inline[]` has `file`, `line`, `author`, `body`, `resolved`, and `id`. Use that
`id` to reply in the right thread.

### Responding to review feedback

    # reply in the thread you are addressing
    bb pr comment <id> --reply-to <comment-id> --body "Fixed in <short-sha>."

    # raise a new point on a specific line
    bb pr comment <id> -f path/to/file.rs -l 88 --body "This drops the error."

    # multi-paragraph body
    printf 'Refactored as suggested.\n\nSplit the parser out.\n' | bb pr comment <id> --body-stdin

### Opening a PR

    bb pr create main --title "Short imperative summary"

### Reviewers

    bb pr reviewers 682                     # who is tagged, and what each decided
    bb pr reviewers add 682 patrick         # tag someone (comma-separate for several)
    bb pr reviewers remove 682 raigon       # untag someone

Names are matched case-insensitively against workspace members and the repository's default
reviewers. An ambiguous name errors and lists the candidates; pass a `{uuid}` to be exact.

### Filtering the list

    bb pr list --needs-my-review            # waiting on you
    bb pr list --reviewer patrick           # tagged for someone else
    bb pr list --author @me                 # yours
    bb pr list --state draft                # drafts only
    bb pr list --review-state approved      # ones you already approved

`bb pr list` shows a STATE column (Draft / Open / Merged / Declined) and a REVIEWERS column
marking each reviewer: `✓` approved, `✗` changes requested, `·` no state yet.
```

## Reference

| Flag / variable | Purpose |
|---|---|
| `--json` | machine-readable output, on every command |
| `-R, --repo` | act on `workspace/repo` instead of the current git remote |
| `BB_REPO` | default repository |
| `BB_EMAIL`, `BB_TOKEN` | credentials for CI and other non-interactive use |
| `BB_API_BASE` | override the API base URL (testing) |
| `BB_UPDATE_API_BASE` | override the release-lookup API base URL for `bb update` (testing) |
| `NO_COLOR` | disable colour and spinners |

| Exit code | Meaning |
|---|---|
| 0 | success |
| 1 | general error |
| 2 | not authenticated |
| 3 | not found |

## Platform support

macOS (arm64, x86_64) and Linux (x86_64, aarch64), both covered by CI. Windows is not supported.

## Contributing

Issues and pull requests are welcome. Before opening a PR, run `cargo fmt --all --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` — CI enforces all three.

`rust-toolchain.toml` pins the exact toolchain used for those checks (currently 1.97), which rustup
auto-installs on first use but which a contributor building offline needs to already have.

Security reports: please use GitHub's
[private vulnerability reporting](https://github.com/biokraft/bbcloud/security/advisories/new)
rather than a public issue.

## License

MIT — see [LICENSE](LICENSE). This project is an independent Rust rewrite of the MIT-licensed PHP
`bb-cli`; see [NOTICE](NOTICE) for attribution.
