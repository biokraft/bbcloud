

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
$ bb pr list --build
┌──────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│ ID   TITLE                      STATE   BUILD        SOURCE           →   TARGET   AUTHOR   REVIEWERS    │
╞══════════════════════════════════════════════════════════════════════════════════════════════════════════╡
│ 42   Cache session lookups      Open    SUCCESSFUL   feat/cache       →   main     dev      Dana ✓       │
│ 41   Fix token refresh window   Draft   FAILED       fix/token-clock  →   main     dev      Ash ·        │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

Across every repository you work in, not just this one:

```
$ bb pr mine
┌──────────────────────────────────────────────────────────────────────────────────────────────────┐
│ REPO             ID    TITLE                     STATE   ROLE       MINE      UPDATED            │
╞══════════════════════════════════════════════════════════════════════════════════════════════════╡
│ acme/api         225   Validate api responses    OPEN    reviewer   pending   4 hours ago        │
│ acme/web         206   Add guardrail hooks       OPEN    reviewer   pending   5 days ago         │
│ acme/api         198   Cache session lookups     OPEN    author     -         2 days ago         │
└──────────────────────────────────────────────────────────────────────────────────────────────────┘
```

## Install

```bash
brew install biokraft/tap/bb
```

Recommended: updates via `brew update && brew upgrade`, no Rust toolchain needed. (`brew upgrade`
alone does not refresh the tap, so a freshly published version can stay invisible.)

### Alternatives

| Method | Command | Requires |
| --- | --- | --- |
| Install script | `curl -fsSL https://raw.githubusercontent.com/biokraft/bbcloud/main/install.sh \| sh` | Nothing — detects platform, verifies checksum, installs to `~/.local/bin` |
| Prebuilt binary | Download from the [latest release](https://github.com/biokraft/bbcloud/releases/latest) | Manual `PATH` setup; verify against the matching `.sha256` |
| Nix | `nix profile install github:biokraft/bbcloud` | Nix with flakes enabled |
| `cargo binstall` | `cargo binstall bbcloud` | `cargo-binstall`, no compiler |
| `cargo install` | `cargo install bbcloud --locked` | Rust 1.88+ (a clone pins 1.97 via `rust-toolchain.toml`) |

Supported targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`.

The cargo routes install `bb` into `~/.cargo/bin` — add that to your `PATH` if the command isn't
found afterwards.

## Get started

Three commands, once per machine — step 1 is Homebrew here because it is the recommended route;
any of the [alternatives](#alternatives) works the same for steps 2 and 3.

```bash
brew install biokraft/tap/bb   # 1. install
bb auth login                  # 2. authenticate — walks you through creating a scoped token
bb skill install --global      # 3. teach your coding agents to drive bb
```

`bb auth login` prints the token URL and the exact scopes to grant, then verifies the token before
storing it in your OS keyring — see [Authenticate](#authenticate) for the scope table and for CI
machines that have no keyring.

`bb skill install --global` installs the agent skills under your home directory, so every project
picks them up; drop `--global` to install into the current project only. See
[Agent skills](#agent-skills) for what they contain.

Then, in any repository with a Bitbucket remote:

```bash
bb pr list --build   # this repository
bb pr mine           # every repository you work in
```

## Agent skills

This repository ships several [Agent Skills](.agents/skills/) — the portable `SKILL.md` format
that Claude Code, Codex, Cursor and OpenCode all read. `bitbucket-cloud` teaches the agent to
review pull requests through `bb` rather than ask you to open a browser: the `--json` contract,
the comment and reply flags, the exit codes, and what to do when a scope is missing. It also tells
the agent to answer comment threads and report them, and to leave the resolve decision to you.
`bbc-daily-brief` builds a ranked morning brief on top of `bb pr mine`, and is invoked only when
you explicitly ask for one. `bbc-open-pr` walks the agent through opening a pull request: it
suggests reviewers by scanning the recent history of the files the change touches, resolving each
name against Bitbucket before it is suggested, and it prints the drafted description back to you
for approval before creating anything.

The skill text ships inside the `bb` binary, so `bb skill install` needs no network and no
credentials. Run it on a terminal with none of `--skill`, `--all` or `--json`, and it asks which
skills to install; pick none and it exits 0 having written nothing. Pass `--all` to install every
skill without asking, or `--skill <name>` to install (or uninstall) exactly one. Any non-interactive
run — CI, piped stdin, or `--json` — installs every skill and prompts for nothing. It detects which
agents the project uses — `.claude/` means Claude Code, any of `.agents/`, `.cursor/`, `.opencode/`
means the portable location — and defaults to `.agents/skills/` if it finds none. Pass
`--agent agents|claude|all` to pick explicitly, or `--global` to install under your home directory
instead, so every project picks it up.

| Agent | Discovers skills in | Extra step |
| --- | --- | --- |
| [Codex](https://learn.chatgpt.com/docs/build-skills) | `.agents/skills/`, `~/.agents/skills/` | none |
| [Cursor](https://cursor.com/docs/skills) | `.agents/skills/`, `.cursor/skills/`, and the `~/` equivalents | none |
| [OpenCode](https://opencode.ai/docs/skills/) | `.opencode/skills/`, `.claude/skills/`, `.agents/skills/` | none |
| [Claude Code](https://code.claude.com/docs/en/skills) | `.claude/skills/`, `~/.claude/skills/` | none — `bb skill install` writes a symlink there |

Run `bb skill status` to see where each copy is installed and whether it is current, stale or
edited locally. Installed copies keep themselves current: when the running binary is newer than the
copy that wrote them — after `brew upgrade`, `cargo install` or `bb update` — the next `bb` command
refreshes them, so the instructions an agent reads never describe an older CLI. A locally edited
file is never overwritten; it is reported and left alone. Set `BB_SKILL_NO_AUTO_REFRESH=1` to manage
the files entirely by hand.

Run `bb skill uninstall` to remove every tracked copy (or `--global` to remove the ones under your
home directory instead). A locally edited copy is left alone unless you pass `--force`, same rule
as `install`.

Each agent loads the skill by itself when a task touches Bitbucket. To force it, name it:
*"use the bitbucket-cloud skill"*. If your tool reads no skills at all, paste the file into
`AGENTS.md` or `CLAUDE.md` — it is plain Markdown.

## Authenticate

Atlassian **removed Bitbucket Cloud app passwords on 2026-07-28.** `bb` uses an Atlassian API token,
sent as HTTP Basic auth with your account email as the username.

`bb auth login` walks you through it: it prints the token URL and the scopes below, prompts for
your email and the token — masked, never echoed — and verifies the token against `/user` before
storing it, so a token with the wrong scopes is rejected at login rather than at the first command
that needs them.

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
| `read:pullrequest:bitbucket` | `pr list`, `pr view`, `pr diff`, `pr files`, `pr commits`, `pr mine` |
| `write:pullrequest:bitbucket` | `pr create`, `pr comment`, `pr resolve`, `pr unresolve`, `pr request-changes` |
| `read:repository:bitbucket` | `branch list`, `repo list`, the default-reviewer lookup `pr create` does, and the workspace/repository scan `pr mine` does |
| `read:project:bitbucket` | `project list`, and the project picker `repo create` uses when `--project` is omitted |

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
bb pr list --needs-my-review              # only PRs waiting on your review
bb pr view 42 --unresolved                # the PR plus comment threads still needing action
bb pr build 42                            # one PR's checks: key, name, state, url
bb pr reviewers add 42 dana            # tag a reviewer; comma-separate for several
bb pr create main --title "Add caching"   # source branch inferred from your checkout
bb pr comment 42 -f src/auth.rs -l 88 -b "off by one"
bb pr resolve 42 998877                   # confirms first, then closes the thread
bb pr request-changes 42 --yes            # confirms first unless --yes is given
bb pr mine --role reviewer --build        # your PRs across every repo you can see
bb branch list --user alice
bb project list                                  # projects in the workspace
bb repo list --project ENG                       # repositories in one project
bb repo create api-gateway --project ENG         # private by default
bb repo create docs --project ENG --public       # explicit opt-in to public
bb update                                 # check for a newer release and update
```

`bb repo create` sends `is_private: true` unless you pass `--public`. Omitting the field is not
safe: the effective default depends on workspace configuration, so an omitted value can publish
source code. Everything else — the scm, fork policy, main branch name, wiki and issue tracker —
is left to Bitbucket and the workspace's own settings rather than overridden from here.

Omit `--project` in a terminal and you get a picker. Outside a terminal it is an error naming the
flag, never a prompt that will not be answered.

`bb pr list` also takes `--reviewer <name>`, `--author <name|@me>`, `--review-state
approved|changes-requested|pending`, `--state OPEN|MERGED|DECLINED|SUPERSEDED|DRAFT|ALL`,
`--build` (adds a `BUILD` column, a worst-wins rollup per pull request), and `--build-status
successful|failed|inprogress|stopped|none` (filters on that rollup and implies `--build`).

`bb pr mine` is the one command that is not repository-scoped. There is no Bitbucket api left that
lists which workspaces you belong to, so the workspace(s) to scan are resolved in this order:
`--workspace <slug>[,<slug>...]` (comma-separated, highest precedence), then the `BB_WORKSPACE`
env var (same syntax), then the workspace of the git remote in the current checkout. If none of
those apply — no flag, no env var, and not run inside a Bitbucket checkout — the command errors
instead of silently scanning nothing.

It also takes `--role author|reviewer|all`, `--state`, `--repo-limit <n>` (the most recently
updated repositories to scan per workspace, default 30 — a recency window, not the whole
workspace: a workspace with hundreds of repositories is only ever sampled, not fully covered), and
`--build`. A workspace the token cannot read is reported in a `partial` list rather than failing
the whole command.

`bb update` compares your version against the latest GitHub release. If Homebrew or cargo installed
`bb`, it prints the right upgrade command for that package manager instead of overwriting a file they
manage. For a standalone binary it verifies the download's checksum and replaces itself atomically.

Two things worth knowing that `--help` won't tell you:

**Everything speaks JSON.** Add `--json` to any command and pipe it to `jq` rather than parsing the
tables, whose layout is not a contract. Scripts and agents should default to it.

```bash
bb pr list --json | jq -r '.[] | select(all(.reviewers[]; .state != "approved")) | "\(.id)\t\(.title)"'
```

**`bb pr resolve` asks first.** It shows the thread it will close — the file and line, who raised
it, what it says — and waits for a yes. Without a terminal it fails and names `--yes`, so nothing
resolves in a script or under an agent unless the command line approves it. `bb pr unresolve`
reopens a thread, and needs no confirmation.

**`bb pr request-changes` and `bb pr no-request-changes` ask first too**, the same way: each shows
the pull request it is about to mark — id, title, author — and waits for a yes before requesting or
withdrawing a change request. Pass `--yes` (or `-y`) to skip the prompt. Without a terminal, both
fail and name `--yes` rather than hang, so nothing is marked in a script or under an agent unless
the command line approves it.

Shell completions make the rest discoverable:

```bash
bb completions zsh > ~/.zfunc/_bb         # also bash, fish, powershell, elvish
```

## Reference

| Flag / variable | Purpose |
|---|---|
| `--json` | machine-readable output, on every command |
| `-R, --repo` | act on `workspace/repo` instead of the current git remote |
| `BB_REPO` | default repository |
| `BB_WORKSPACE` | default workspace for `repo`, `project` and `pr mine`, same as `--workspace` |
| `BB_EMAIL`, `BB_TOKEN` | credentials for CI and other non-interactive use |
| `BB_API_BASE` | override the API base URL (testing) |
| `BB_UPDATE_API_BASE` | override the release-lookup API base URL for `bb update` (testing) |
| `BB_SKILL_NO_AUTO_REFRESH` | set to `1` to stop `bb` refreshing installed skill files when the binary version changes |
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
`cargo clippy --all-targets -- -D warnings`, and `cargo test --all` — CI enforces all three.

`rust-toolchain.toml` pins the exact toolchain used for those checks (currently 1.97), which rustup
auto-installs on first use but which a contributor building offline needs to already have.

Security reports: please use GitHub's
[private vulnerability reporting](https://github.com/biokraft/bbcloud/security/advisories/new)
rather than a public issue.

## License

MIT — see [LICENSE](LICENSE). This project is an independent Rust rewrite of the MIT-licensed PHP
`bb-cli`; see [NOTICE](NOTICE) for attribution.
