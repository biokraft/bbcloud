# Changelog

All notable changes to this project are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.15.1](https://github.com/biokraft/bbcloud/compare/v0.15.0...v0.15.1) - 2026-08-14

### Added

- *(skill)* give the daily brief five emoji anchors ([#26](https://github.com/biokraft/bbcloud/pull/26))

## [0.15.0](https://github.com/biokraft/bbcloud/compare/v0.14.0...v0.15.0) - 2026-08-14

### Fixed

- *(skill)* keep installed skills current, and make the daily brief readable ([#24](https://github.com/biokraft/bbcloud/pull/24))

## [0.14.0](https://github.com/biokraft/bbcloud/compare/v0.13.0...v0.14.0) - 2026-08-13

### Fixed

- *(pr)* make pr mine work against the current Bitbucket API ([#22](https://github.com/biokraft/bbcloud/pull/22))

## [0.13.0](https://github.com/biokraft/bbcloud/compare/v0.12.0...v0.13.0) - 2026-08-13

### Added

- *(pr)* cross-repo pr mine and a bbc-daily-brief agent skill ([#20](https://github.com/biokraft/bbcloud/pull/20))

## [0.12.0](https://github.com/biokraft/bbcloud/compare/v0.11.1...v0.12.0) - 2026-08-13

### Added

- *(pr)* build status in pr list and a new pr build command ([#18](https://github.com/biokraft/bbcloud/pull/18))

## [0.11.1](https://github.com/biokraft/bbcloud/compare/v0.11.0...v0.11.1) - 2026-08-12

## [0.11.0](https://github.com/biokraft/bbcloud/compare/v0.10.0...v0.11.0) - 2026-08-11

Two features: comment threads can now be closed and reopened from the shell, and the bundled agent
skill installs itself with a command instead of a `curl` recipe.

### Added

- **`bb pr resolve` and `bb pr unresolve`** — close a review thread once it is answered, or reopen one
  ([#10](https://github.com/biokraft/bbcloud/pull/10)).

      bb pr resolve 42 998877        # asks for confirmation first
      bb pr resolve 42 998877 --yes  # skip the prompt
      bb pr unresolve 42 998877      # reopen it

  Pass the id of the thread's **first** comment — the one whose `parent` is `null`, which
  `bb pr view --unresolved --json` gives you. A reply id fails, and so does a general comment, since
  only inline threads carry a resolution. `resolve` asks a human to confirm and fails outright with no
  terminal attached, so closing a thread stays a deliberate act rather than something a script does by
  accident.

- **`bb skill install`** — set up this repository's Agent Skill for the coding agents in your project
  ([#14](https://github.com/biokraft/bbcloud/pull/14)).

      bb skill install     # detects .claude/, .agents/, .cursor/, .opencode/ and sets them up
      bb skill status      # where it is installed, and whether it is current
      bb skill uninstall

  This replaces the manual `mkdir` + `curl` + symlink instructions. The skill text is embedded in the
  binary, so installation needs no network and the installed skill can never describe flags your
  binary lacks. Claude Code reads only `.claude/skills/`, so that path becomes a relative symlink to
  the `.agents/` copy — the manual link step is gone. Your own edits to an installed skill are never
  overwritten without `--force`, and `bb update` brings unmodified copies forward as the binary moves.
  The whole group works without authentication.

### Fixed

- **`bb update` blamed the wrong service and hid the real problem.** A GitHub rate limit surfaced as
  `bitbucket api error 403: cannot reach the release api` — wrong service, and the API had in fact
  answered. It now reports `release api error 403: github api rate limit reached — 60 requests per hour
  for unauthenticated access, retry after 14:42`, taking the retry time from the response.

- **The Homebrew upgrade hint did not work.** `bb update` suggested `brew upgrade bb`, which never
  refreshes taps, so a freshly published formula stayed invisible and the command reported "already
  installed" while a newer version existed. It now suggests `brew update && brew upgrade bb`.

Note on scope: resolving a thread is now supported, which the v0.10.0 note said it was not — that
changed here, deliberately, and it is gated behind a confirmation prompt. Approving, merging and
declining a pull request remain unsupported and stay human decisions.

## [0.10.0](https://github.com/biokraft/bbcloud/compare/v0.9.5...v0.10.0) - 2026-08-11

Reviewers become first-class: who is tagged, what each of them decided, and which pull requests are
waiting on you ([#12](https://github.com/biokraft/bbcloud/pull/12)).

### Added

- **`bb pr reviewers`** — see and change who reviews a pull request.

      bb pr reviewers 42                  # who is tagged, and what each decided
      bb pr reviewers add 42 alice        # tag someone (comma-separate for several)
      bb pr reviewers remove 42 bob       # untag someone

  Names are matched case-insensitively against the repository's users and its default reviewers. An
  ambiguous name lists the candidates rather than guessing; a `{uuid}` is always exact. Adding
  someone already tagged writes nothing, and removing someone who is not tagged is an error rather
  than a silent no-op.

- **Review-state filters on `bb pr list`** — most usefully `--needs-my-review`, for the pull requests
  where you are a reviewer and have not approved yet.

      bb pr list --needs-my-review
      bb pr list --reviewer alice
      bb pr list --author @me
      bb pr list --review-state approved   # or changes-requested, pending
      bb pr list --state draft             # also --state all

- **A `STATE` column** on `bb pr list` (`Draft` / `Open` / `Merged` / `Declined`), and a `REVIEWERS`
  column that marks each reviewer's decision: `✓` approved, `✗` changes requested, `·` no state yet.

### Fixed

- **`bb pr list` never showed reviewers.** The `REVIEWERS` and `APPROVED` columns had existed for
  several releases and were always empty. Bitbucket's paginated `/pullrequests` endpoint returns a
  reduced pull-request object that omits `reviewers`, `participants` and `draft`; they come back only
  when requested explicitly, and nothing was requesting them.

- **Name lookup could only find default reviewers.** Resolution went through
  `/workspaces/{workspace}/members`, which needs workspace scope that an ordinary repository token
  does not carry. The refusal was swallowed silently, so colleagues plainly visible in the reviewers
  column could not be named. Lookup is now repository-scoped, and a refused lookup warns instead of
  failing quietly.

### Changed

- **Breaking, `bb pr list --json` only:** `reviewers` is now an array of objects — `{"name", "uuid",
  "state"}`, where `state` is `approved`, `changes_requested` or `pending` — and the `approvals` array
  is gone. Both previously emitted empty arrays in every case, so no working script can depend on
  their contents. A `select(.approvals == [])` filter becomes
  `select(all(.reviewers[]; .state != "approved"))`.

Approving, merging, declining and resolving comment threads remain deliberately unsupported: those
stay human decisions.

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
