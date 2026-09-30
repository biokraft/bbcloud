

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

Across your workspaces, not just this repository:

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

### Staying current

`bb` asks the release API once a day whether a newer version exists, and when there is one it
prints a single line to **stderr** naming the version and the one command that upgrades your
install — `brew`, `cargo install` or `bb update`, whichever owns the binary:

```
warning: bb 0.20.0 is available (you have 0.19.4) — upgrade with: bb update
```

The answer is cached in `~/.config/bb/update-check.json`, so every other command reads a file
rather than the network. The notice always goes to stderr, never stdout, so it cannot corrupt
`--json` output — which is also how an agent shelling out to `bb` gets told to suggest the
upgrade. Set `BB_NO_UPDATE_CHECK=1` to switch the check off; failures (offline, rate limited,
read-only config directory) are silent and never affect the command you ran.

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
bb pr mine           # across your workspaces
```

## Agent skills

This repository ships four [Agent Skills](.agents/skills/) — the portable `SKILL.md` format that
Claude Code, Codex, Cursor and OpenCode all read. Each one carries what `--help` cannot: the house
rules, the decisions that stay with you, and what each failure means.

| Skill | What the agent does with it |
| --- | --- |
| `bitbucket-cloud` | Reads, reviews and answers pull requests through `bb` instead of sending you to a browser. It answers threads and reports them; resolving a thread, requesting changes and approving stay your decision. |
| `bbc-open-pr` | Opens a pull request: pushes the branch, suggests reviewers from the history of the changed files once you consent to that read, and shows you the description and the reviewer pick before anything is created. |
| `bbc-daily-brief` | Builds a ranked brief of what waits on you, from `bb pr mine` or, for one repository, from `bb pr list`. Read-only, and only when you ask for one. |
| `bbc-report-bug` | Files a bug about `bb` itself against this repository with `gh`: it reproduces the problem, replaces your workspace, repository, people, paths and URLs with placeholders, searches for a duplicate, and shows you the issue before it is filed. |

Every `bb skill` command is also spelled `bb skills`, whichever comes to hand first.

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
home directory instead). It deletes only files `bb` itself wrote. A locally edited copy is left
alone, and so is one `bb` merely found already in place — a copy you vendored and committed, say,
which matches the embedded text byte for byte. Both are reported and left on disk unless you pass
`--force`, same rule as `install`.

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

On macOS, the keychain item is read through Apple's `/usr/bin/security`, so upgrading `bb` does
not ask for your login password again. Credentials stored by `bb` 0.24.1 or older prompt one last
time, once for the email and once for the token; choose **Always Allow**.

### Token scopes

Grant the least you need. For the pull request workflow — listing, reading and commenting — four
scopes are enough:

| Scope | Needed for |
|---|---|
| `read:user:bitbucket` | **mandatory.** `bb auth login` verifies the token against `/user`, so login fails without it |
| `read:pullrequest:bitbucket` | `pr list`, `pr view`, `pr diff`, `pr files`, `pr commits`, `pr mine` |
| `write:pullrequest:bitbucket` | `pr create`, `pr comment`, `pr resolve`, `pr unresolve`, `pr request-changes`, `pr retarget`, `pr edit` |
| `read:repository:bitbucket` | `branch list`, `repo list`, `repo reviewers`, the default-reviewer lookup `pr create` does, `pr view --conflicts` (whose redirect target is a repository resource, not a pull-request one), and the workspace/repository scan `pr mine` does |
| `read:project:bitbucket` | `project list`, and the project picker `repo create` uses when `--project` is omitted |
| `admin:repository:bitbucket` | `repo create`. This is the only scope that permits creating a repository — no combination of the read and write scopes above is enough |

One gotcha worth knowing: `write:pullrequest:bitbucket` does **not** imply
`read:repository:bitbucket`, so `pr create` needs both. The same applies to `pr view --conflicts`:
the pull-request endpoint redirects to a repository file-conflict resource, so a token with only
`read:pullrequest:bitbucket` gets a 403 on that flag.

The same shape applies to the repository commands: `read:repository:bitbucket` lets you *list*
repositories but not create one, and `read:project:bitbucket` is a separate grant again — a token
carrying every read scope still gets a 403 from `project list` without it.

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
bb pr list --current                     # only PRs from the current branch
bb pr list --needs-my-review              # only PRs waiting on your review
bb pr list --limit 20                     # cap the returned rows
bb pr view 42 --unresolved                # the PR plus comment threads still needing action
bb pr view 42 --metadata-only --json      # header and reviewer state without comments
bb pr view 42 --build --conflicts --json  # add build and merge-conflict facts
bb pr build 42                            # one PR's checks: key, name, state, url
bb pr reviewers add 42 dana            # tag a reviewer; comma-separate for several
bb pr reviewers suggest --pr 42 --acknowledge-private-data --json # evidence-backed suggestions; read-only
bb pr reviewers suggest main --acknowledge-private-data --json    # prospective suggestions for a target
bb pr create main --title "Add caching"   # source branch inferred from your checkout
bb pr create main --description-stdin < body.md --no-default-reviewers
bb pr create main --reviewer dana,ash     # tag exactly these two, no default reviewers
bb pr create main,release/2.4 --title "Fix login" # one PR per target; --json returns an array
bb pr retarget 42 --to main               # fix a PR opened against the wrong branch
bb pr edit 42 --title "Cache lookups"     # fix a title; --description-stdin
bb pr comment 42 -f src/auth.rs -l 88 -b "off by one"
bb pr comment 42 --pending -b "nit"       # draft until you press Finish review
bb pr resolve 42 998877                   # confirms first, then closes the thread
bb pr request-changes 42 --yes            # confirms first unless --yes is given
bb pr mine --role reviewer --build        # PRs you review, across your workspaces
bb branch list --user alice
bb project list                                  # projects in the workspace
bb repo list --project ENG                       # repositories in one project
bb repo reviewers --json                          # reviewer-resolver users and partial sources
bb repo create api-gateway --project ENG         # private by default
bb repo create docs --project ENG --public       # explicit opt-in to public
bb update                                 # check for a newer release and update
```

`bb repo create` sends `is_private: true` unless you pass `--public`. Omitting the field is not
safe: the effective default depends on workspace configuration, so an omitted value can publish
source code. Everything else — the scm, fork policy, main branch name, wiki and issue tracker —
is left to Bitbucket and the workspace's own settings rather than overridden from here.

`repo list --json` preserves the full repository slug, project identity, web URL, clone URLs, and
raw update timestamp. `repo reviewers --json` lists the people a reviewer name can resolve to and
names any user pools the token could not read in `partial`. It is deliberately not called
`members`: two of its three sources — workspace membership and default-reviewer status — say nothing
about access to *this* repository, so a person it lists may not be taggable. Each row carries
`eligibility`: `explicit` when the user is in the repository's own permission configuration,
`unknown` when the row comes only from workspace membership or default-reviewer status. When any of
the three pools cannot be read, every command that writes a reviewer (`pr reviewers add`,
`pr create --reviewer`) refuses a name, because the unreadable pool may hide the person you meant —
pass a `{uuid}` instead.

Omit `--project` in a terminal and you get a picker. Outside a terminal it is an error naming the
flag, never a prompt that will not be answered.

`bb pr list` also takes `--reviewer <name>`, `--author <name|@me>`, `--review-state
approved|changes-requested|pending`, `--state OPEN|MERGED|DECLINED|SUPERSEDED|DRAFT|ALL`,
`--build` (adds a `BUILD` column, a worst-wins rollup per pull request), and `--build-status
successful|failed|inprogress|stopped|none` (filters on that rollup and implies `--build`). Build
status costs one request per pull request that passes the other filters, and `--build-status`
filters after those requests, so it costs the same as `--build` — narrow with the other filters
first.
`--state all` asks for every state as repeated query parameters, which is the form the API documents.

`--current` filters by the current symbolic branch, and refuses to run when `-R`/`BB_REPO` selects a
repository other than the checkout's — pairing a local branch name with an unrelated repository
returns confidently wrong rows, or none.

`--limit` is an output cap, not a page cap, and it defaults to **100**. Filtering — including
`--build-status` — is applied first, and the command keeps paging until it has that many matching
rows, so a match on page three is still found with `--limit 1`. The page size stays at 50 because
that is the largest value Bitbucket's pull-request endpoint is documented to accept. Because the
cap is silent, a repository with more than 100 matching pull requests shows the first 100 and no
warning; pass a larger `--limit` when you need the rest.

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

Things worth knowing that `--help` won't tell you:

**Everything speaks JSON.** Add `--json` to any command and pipe it to `jq` rather than parsing the
tables, whose layout is not a contract. Scripts and agents should default to it.

`bb pr view` includes the pull-request description, draft state, reviewers, exact timestamps,
comment and task counts, and the original comment timestamps. Use `--metadata-only` when comments
are unnecessary, `--build` to add statuses, and `--conflicts` to add reported merge conflicts.
Optional sections are omitted unless requested; `--json` stdout remains one JSON value.

`bb pr reviewers suggest` is read-only, and keeps two kinds of evidence apart. Each suggestion
reports `target_commits` — **target history**, who maintains the files the change touches — and
`source_commits` — **source history**, who worked on this branch — next to the total
`commit_count`. The target's commits are excluded from the source history only when both branches
live in the same repository — a fork's branch does not contain the target's commits, so excluding
them there would exclude nothing real. The report names the source and target
repositories and the exact commit each was read at, so the evidence is reproducible and a branch
pushed to mid-command cannot change what the report claims. A renamed file is read under both its
old and new path, and a **deleted** file under the path it had — a removal is the strongest possible
signal that somebody knows the code, so it is never dropped.

`--file-limit` bounds how many changed files are examined (default 25) and it really bounds the
requests made, not just the numbers reported. A report names `files_scanned`, `files_skipped`,
`paths_scanned` and `paths_skipped`: entries and paths are counted separately because a rename is
one entry and two paths. A single report reads at most 100 distinct paths, and `history_complete` is
`false` whenever anything was cut, so a suggestion list never implies it saw the whole change.

`--acknowledge-private-data` is required, because this reads private file paths, colleague names,
dates, and account ids out of commit history and into whatever is on the other end of the model.
Say what those categories are before running it.

Each suggestion carries `can_review`: `yes` when the person is in this repository's own permission
configuration, `no` when the user pool was read in full and they are not in it, and `unknown`
otherwise. Ownership of a file is not access to the repository, and the report never implies it is.
The author and current reviewers are excluded, and so is the authenticated user in prospective mode —
Bitbucket rejects a pull request's author as a reviewer. A rate limit, a server error, or a network
failure fails the command rather than returning an empty list, because an empty report that exits `0`
is indistinguishable from a retired endpoint. It never tags anyone; the user still chooses the set.

```bash
bb pr list --json | jq -r '.[] | select(all(.reviewers[]; .state != "approved")) | "\(.id)\t\(.title)"'
```

**`bb pr resolve` asks first.** It shows the thread it will close — the file and line, who raised
it, what it says — and waits for a yes. Without a terminal it fails and names `--yes`, so nothing
resolves in a script or under an agent unless the command line approves it. `bb pr unresolve`
reopens a thread, and needs no confirmation.

Only the first comment of an inline thread can be resolved — the one `bb pr view` shows with no
`parent` and with a `file`. On the prompt path `bb` refuses a reply or a general comment with a
message that says so. `--yes` skips that lookup, so Bitbucket answers the same mistake with a 403,
which `bb` reports as a missing scope. Check the id before you pass `--yes`.

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
| `BB_NO_UPDATE_CHECK` | set to `1` to stop `bb` checking once a day whether a newer release exists |
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
