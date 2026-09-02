# Changelog

All notable changes to this project are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.20.0](https://github.com/biokraft/bbcloud/compare/v0.19.4...v0.20.0) - 2026-09-02

### Added

- *(pr)* tag exactly the chosen reviewers with `pr create --reviewer` ([#61](https://github.com/biokraft/bbcloud/pull/61))

## [0.19.3](https://github.com/biokraft/bbcloud/compare/v0.19.2...v0.19.3) - 2026-09-01

### Added

- *(pr)* `bb pr retarget <id> --to <branch>` moves an open pull request to a
  different destination branch, so one opened against the wrong base is fixed in
  place instead of being closed and reopened with its review history discarded
  ([#57](https://github.com/biokraft/bbcloud/pull/57)).

  It resends the existing title, which the api requires on the update, refuses a
  pull request that is not open with a message naming its state, and makes no
  write at all when the pull request already targets that branch. Bitbucket
  recomputes the diff, so inline comments anchored to the old base can start
  reading as outdated — the command says so. The source branch cannot be moved;
  that is the api's rule, not an omission. No confirmation prompt: retargeting
  corrects a mistake rather than hiding a review point like `pr resolve` or
  asserting a verdict like `pr request-changes`.

  The bundled agent skill documents the command, so `bb skill install` picks it
  up and agents can fix their own mis-targeted pull requests.

## [0.19.2](https://github.com/biokraft/bbcloud/compare/v0.19.1...v0.19.2) - 2026-09-01

### Added

- *(skill)* `bb skills` now works everywhere `bb skill` does. The singular reads oddly for a
  command group that manages four files, and "skills" is the noun every agent's own documentation
  uses, so it is what people type first. The singular stays canonical — every other group is
  singular (`pr`, `repo`, `branch`, `auth`, `project`), and renaming would break existing scripts —
  but the plural is a visible alias, so `bb --help` advertises it rather than leaving it for people
  to guess. Shell completions cover both ([#55](https://github.com/biokraft/bbcloud/pull/55))

## [0.19.1](https://github.com/biokraft/bbcloud/compare/v0.19.0...v0.19.1) - 2026-09-01

### Documentation

- *(agents)* correct how release-plz picks the version ([#53](https://github.com/biokraft/bbcloud/pull/53))

## [0.19.0](https://github.com/biokraft/bbcloud/compare/v0.18.2...v0.19.0) - 2026-09-01

Data loss: `bb skill uninstall` could delete `SKILL.md` files `bb` never wrote. If you keep skill
files under version control, upgrade before you next run it.

### Fixed

- *(skill)* `bb skill uninstall` now deletes only files `bb` itself wrote. `install` decided what
  to record from a content hash alone — and a hash match proves the bytes are identical, never
  that `bb` is what put them there. So a `SKILL.md` that already existed and happened to match was
  reported `unchanged`, which was accurate since `bb` wrote nothing and left git clean, and then
  recorded as a file `bb` owns. The next `uninstall` deleted it.

  The sharp case was this crate's own checkout, where `.agents/skills/*/SKILL.md` are the tracked
  sources `include_str!` compiles in: byte-identical to the embedded copies by construction, so
  `bb skill install` claimed all four and `bb skill uninstall` removed the crate's own sources and
  broke the build. Any project that vendors and commits a bundled skill was exposed to the same
  path.

  State entries now record whether `bb` created the file, at write time rather than inferred from
  a hash afterwards, and uninstall refuses anything else — restoring the guarantee its own
  documentation already made, that an untracked file is never touched. A refusal reports
  `refused_not_written` and stops tracking the file; `--force` still removes it. A hand-made
  Claude symlink is still removed, because the guard protects content and a link holds none, and
  the `.agents` file it points at keeps its own protection
  ([#51](https://github.com/biokraft/bbcloud/pull/51))

### Upgrading

Nothing to do, and no state file to migrate — entries written by earlier versions are treated as
`bb`'s own writes, so everything you installed before this release stays removable exactly as it
was.

One behaviour change worth knowing if you script against it: `bb skill uninstall --json` can now
report `refused_not_written` in a row's `outcome`, alongside the existing `removed`,
`refused_modified`, `refused_unsafe_path` and `absent`. If you vendored a copy of a bundled skill
and want `bb` to delete it anyway, pass `--force`.

This is a minor rather than a patch release only because that new value is an addition to a public
enum. There is no change to any command's arguments or behaviour beyond the fix above.

## [0.18.2](https://github.com/biokraft/bbcloud/compare/v0.18.1...v0.18.2) - 2026-09-01

v0.18.1 documented the scopes `repo create` and `project list` need. It documented them in the
README only — `bb auth login` went on printing the old four, so following the tool's own
walkthrough still produced a token that fails on those commands. This release fixes the
walkthrough and removes the duplicate list that let it drift. It also adds a fourth agent skill,
for reporting `bb` bugs upstream.

### Fixed

- *(auth)* `bb auth login` now names all six scopes. It listed four, omitting
  `read:project:bitbucket` and `admin:repository:bitbucket` — the two the README's table gained
  when `repo create` and `project list` shipped in v0.18.0. Anyone who followed the walkthrough
  minted a token that returned 403 on their first `repo create`, with nothing in the login output
  to explain which grant was missing. The list also existed three times over — in `SCOPES`, in the
  `auth login --help` text, and in the README — and two of the three fell behind; `--help` is now
  rendered from `SCOPES`, and a test asserts the README table and `SCOPES` hold the same set, so
  neither copy can drift alone again. Every pre-existing scope test iterated `SCOPES`, which is
  why all of them stayed green throughout ([#47](https://github.com/biokraft/bbcloud/pull/47))

### Added

- *(skill)* `bbc-report-bug`, a fourth bundled agent skill: it files a bug about `bb` itself
  against this repository with `gh`. An agent that trips over a `bb` bug mid-task holds the best
  evidence there is — the exact commands, the `--json` output, the version — and until now had
  nowhere to put it. That evidence is the problem, though, since it comes out of a private
  Bitbucket workspace and a GitHub issue is public forever, so the skill redacts workspace,
  repository, project and human names to placeholders before it drafts anything, never includes a
  token in any form, prints the finished issue and waits for you to approve it, and never files on
  its own initiative — the same gate `pr resolve` and `pr request-changes` apply. It is hardcoded
  to `biokraft/bbcloud` and cannot be pointed at another repository. Install it with
  `bb skill install --skill bbc-report-bug`
  ([#48](https://github.com/biokraft/bbcloud/pull/48))

### Upgrading

Nothing breaks and no token needs replacing. But if you authenticated before this release and
have not used `bb repo create` or `bb project list`, your token is probably missing the two
scopes the walkthrough never mentioned — run `bb auth login` again and grant all six from the new
list rather than waiting to be surprised by a 403.

Existing skill files are untouched by the upgrade. `bb skill install` will offer the new
`bbc-report-bug` alongside the three you already have.

## [0.18.1](https://github.com/biokraft/bbcloud/compare/v0.18.0...v0.18.1) - 2026-08-26

A follow-up to v0.18.0, from using it: the upgrade instruction `bb update` printed could not
work, and the new commands' token scope was undocumented.

### Fixed

- *(update)* the Homebrew hint printed a command that fails. `bb update` told Homebrew users to
  run `brew upgrade bb`, but Homebrew resolves an unqualified name against casks as well as
  formulae, and an unrelated cask named `bb` now exists — so the command ended in
  `Error: Cask 'bb' is not installed` and never touched the install it was meant to upgrade. The
  hint now names the formula in full: `brew update && brew upgrade biokraft/tap/bb`
  ([#45](https://github.com/biokraft/bbcloud/pull/45))
- *(update)* a skipped skill now says how to take the new version. Refusing to overwrite a
  locally edited `SKILL.md` is the right default, but after a release that adds commands — as
  v0.18.0 added three — it leaves that agent describing a `bb` that no longer exists, and the
  message said only that it had skipped the file. It now names the way out:
  `bb skill install --force` ([#45](https://github.com/biokraft/bbcloud/pull/45))

### Documentation

- *(readme)* the scope table documents what `repo create` needs:
  **`admin:repository:bitbucket`**, confirmed against a real token to be the only scope that
  permits creating a repository — no combination of the read and write scopes is enough. The table
  now also says plainly that `read:repository:bitbucket` lets you *list* repositories without
  being able to create one, and that `read:project:bitbucket` is a separate grant again: a token
  holding every read scope still gets a 403 from `project list`
  ([#43](https://github.com/biokraft/bbcloud/pull/43))

### Upgrading

If you installed with Homebrew and `brew upgrade bb` failed for you, this is why; use
`brew upgrade biokraft/tap/bb`. If `bb update` reported skipping a customized skill, run
`bb skill install --force` to pick up the v0.18.0 commands — that discards local edits to those
files, so check `bb skill status` first if you want to keep them.

## [0.18.0](https://github.com/biokraft/bbcloud/compare/v0.17.1...v0.18.0) - 2026-08-26

### Added

- *(repo)* `bb repo create <name> --project KEY` creates a repository in a project. It sends
  `is_private: true` unless you pass `--public`, because omitting the field does not reliably
  produce a private repository — the effective default depends on workspace configuration, so an
  omitted value can publish source code. Nothing else is overridden: the scm, fork policy, main
  branch name, wiki and issue tracker are left to Bitbucket and the workspace's own settings.
  Omit `--project` in a terminal and you get a picker; outside a terminal it is an error naming
  the flag, never a prompt that cannot be answered. No git side effects — no clone, no
  `git remote add` ([#41](https://github.com/biokraft/bbcloud/pull/41))
- *(repo)* `bb repo list [--project KEY]` lists a workspace's repositories, narrowing server-side
  by project key ([#41](https://github.com/biokraft/bbcloud/pull/41))
- *(project)* `bb project list` lists the projects in a workspace ([#41](https://github.com/biokraft/bbcloud/pull/41))
- All three take `--workspace`, falling back to `BB_WORKSPACE` and then to the workspace half of
  the git remote, the same order `bb pr mine` uses ([#41](https://github.com/biokraft/bbcloud/pull/41))
- The bundled agent skill now carries the matching rule: an agent never creates a repository on
  its own initiative, never passes `--public` unless the human said the word, and runs
  `bb project list` rather than guessing a project key ([#41](https://github.com/biokraft/bbcloud/pull/41))

### Fixed

- *(api)* a 403 no longer discards Bitbucket's own explanation. `Client::check` returned a fixed
  string for 403 without reading the response body, so the one status where the server names the
  missing privilege was the only one that threw that message away. It now prefers the API's
  `error.message` and appends the scope hint rather than replacing it
  ([#41](https://github.com/biokraft/bbcloud/pull/41))

### New token scopes

`bb project list`, and the picker `bb repo create` shows when `--project` is omitted, need
**`read:project:bitbucket`** — a token without it gets a 403. `bb repo list` needs only
`read:repository:bitbucket`, which existing tokens already carry for `bb branch list`. See the
scope table in the README.

### Breaking (library consumers only)

`api::models::Repository` gained `name`, `slug`, `description`, `is_private`, `project`,
`updated_on` and `links`, so a struct literal that constructed it from `full_name` alone no longer
compiles. Every field is `Option`, so deserialization is unaffected — this breaks construction,
not parsing. The `bb` binary is unaffected.

## [0.17.1](https://github.com/biokraft/bbcloud/compare/v0.17.0...v0.17.1) - 2026-08-17

### Documentation

- align the `bb pr mine` table's right border ([#39](https://github.com/biokraft/bbcloud/pull/39))

## [0.17.0](https://github.com/biokraft/bbcloud/compare/v0.16.0...v0.17.0) - 2026-08-17

### Added

- *(pr)* confirm before requesting or withdrawing changes, and teach the skill to ask ([#38](https://github.com/biokraft/bbcloud/pull/38))

### Documentation

- fix misaligned box borders in `bb pr mine` README table ([#37](https://github.com/biokraft/bbcloud/pull/37))
- improve README ([#34](https://github.com/biokraft/bbcloud/pull/34))

## [0.16.0](https://github.com/biokraft/bbcloud/compare/v0.15.3...v0.16.0) - 2026-08-14

### Added

- *(skill)* add bbc-open-pr and let skill install ask what to install ([#32](https://github.com/biokraft/bbcloud/pull/32))

## [0.15.3](https://github.com/biokraft/bbcloud/compare/v0.15.2...v0.15.3) - 2026-08-14

### Added

- *(auth)* walk the user through creating a scoped api token ([#30](https://github.com/biokraft/bbcloud/pull/30))

## [0.15.2](https://github.com/biokraft/bbcloud/compare/v0.15.1...v0.15.2) - 2026-08-14

### Fixed

- *(skill)* link the daily brief to Bitbucket, not to a GitHub 404 ([#28](https://github.com/biokraft/bbcloud/pull/28))

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
