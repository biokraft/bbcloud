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
┌────┬──────────────────────────┬─────────────────┬───┬────────┬────────┬────────────┬──────────┐
│ ID ┆ TITLE                    ┆ SOURCE          ┆ → ┆ TARGET ┆ AUTHOR ┆ REVIEWERS  ┆ APPROVED │
╞════╪══════════════════════════╪═════════════════╪═══╪════════╪════════╪════════════╪══════════╡
│ 42 ┆ Cache session lookups    ┆ feat/cache      ┆ → ┆ main   ┆ dev    ┆ Ada, Linus ┆ Ada      │
│ 41 ┆ Fix token refresh window ┆ fix/token-clock ┆ → ┆ main   ┆ dev    ┆ Linus      ┆          │
└────┴──────────────────────────┴─────────────────┴───┴────────┴────────┴────────────┴──────────┘
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
| `write:pullrequest:bitbucket` | `pr create`, `pr comment`, `pr resolve`, `pr request-changes` |
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
bb pr list                                # open PRs, with reviewers and approvals
bb pr view 42 --unresolved                # the PR plus comment threads still needing action
bb pr create main --title "Add caching"   # source branch inferred from your checkout
bb pr comment 42 -f src/auth.rs -l 88 -b "off by one"
bb pr resolve 42 998877                   # confirms first, then closes the thread
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
bb pr list --json | jq -r '.[] | select(.approvals == []) | "\(.id)\t\(.title)"'
```

**Resolving asks first.** `bb pr resolve` prints the thread it is about to close — where it sits, who
raised it, what it says — and waits for a yes. With no terminal it fails naming `--yes` rather than
prompting, so nothing resolves in a script or under an agent unless the command line says so
explicitly. Closing a reviewer's point is a decision, not a formality; `bb pr unresolve` reopens a
thread and needs no confirmation.

Shell completions make the rest discoverable:

```bash
bb completions zsh > ~/.zfunc/_bb         # also bash, fish, powershell, elvish
```

## Use it from an AI Agent

This repository ships an [Agent Skill](.agents/skills/bitbucket-cloud/SKILL.md) — the portable
`SKILL.md` format that Claude Code, Codex, Cursor and OpenCode all read. It teaches the agent to
review pull requests through `bb` rather than ask you to open a browser: the `--json` contract, the
comment and reply flags, the exit codes, and what to do when a scope is missing. It also draws the
line the CLI cannot: the agent answers comment threads and reports them, and leaves resolving to you.

Install it into a project:

```bash
mkdir -p .agents/skills/bitbucket-cloud
curl -fsSL https://raw.githubusercontent.com/biokraft/bbcloud/main/.agents/skills/bitbucket-cloud/SKILL.md \
  -o .agents/skills/bitbucket-cloud/SKILL.md
```

| Agent | Discovers skills in | Extra step |
| --- | --- | --- |
| [Codex](https://learn.chatgpt.com/docs/build-skills) | `.agents/skills/`, `~/.agents/skills/` | none |
| [Cursor](https://cursor.com/docs/skills) | `.agents/skills/`, `.cursor/skills/`, and the `~/` equivalents | none |
| [OpenCode](https://opencode.ai/docs/skills/) | `.opencode/skills/`, `.claude/skills/`, `.agents/skills/` | none |
| [Claude Code](https://code.claude.com/docs/en/skills) | `.claude/skills/`, `~/.claude/skills/` | link it, see below |

```bash
mkdir -p .claude/skills
ln -s ../../.agents/skills/bitbucket-cloud .claude/skills/bitbucket-cloud
```

To get the skill in every project, install it under your home directory instead: `~/.agents/skills/`
for Codex, Cursor and OpenCode, `~/.claude/skills/` for Claude Code.

Each agent loads the skill by itself when a task touches Bitbucket. To force it, name it:
*"use the bitbucket-cloud skill"*. If your tool reads no skills at all, paste the file into
`AGENTS.md` or `CLAUDE.md` — it is plain Markdown.

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
