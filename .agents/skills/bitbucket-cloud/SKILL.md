---
name: bitbucket-cloud
description: Read and answer Bitbucket Cloud pull request reviews with the `bb` CLI. Use this skill when the repository is hosted on Bitbucket Cloud, or when the task is to list, read, review, comment on, or open a pull request there. Do not use it for GitHub or GitLab.
license: MIT
---

# Bitbucket Cloud with `bb`

`bb` is a single binary that speaks the Bitbucket Cloud REST API. Use it for all pull request work.
Do not use `gh`. Do not ask the user to open the web UI.

## Rules

1. Add `--json` to every command, and parse the JSON. The tables are for humans, and their layout
   can change. One exception: read `bb pr diff <id>` as plain text.
2. Do not resolve a comment thread unless the user asks you to. Reply, then report what you
   answered. See [Report threads, do not close them](#report-threads-do-not-close-them).
3. Do not pass `-w` or `--web`. These flags start a browser.
4. Use the exit code, not the error text: `0` success, `1` error, `2` not authenticated,
   `3` not found.
5. Give a body to every comment. Use `--body` for one line. Use `--body-stdin` for more than one
   paragraph. Without a body and without a terminal, the command fails.
6. Add `-R workspace/repo` to act on another repository. The default comes from the git remote.
7. In a new checkout, run `bb skill install` to set up this skill. It needs no authentication.

## Read a pull request

```bash
bb pr list --json                          # open pull requests
bb pr list main --state MERGED --json      # filter by target branch and state
bb pr list --state all --json              # every state, not just OPEN
bb pr list --needs-my-review --json        # I'm a reviewer and haven't approved yet
bb pr list --reviewer dana --json       # PRs that person is tagged on
bb pr list --author @me --json             # PRs I opened; @me resolves the authenticated account
bb pr list --review-state approved --json  # my own state: approved | changes-requested | pending
bb pr list --build --json                  # add BUILD column: worst-wins rollup per PR
bb pr list --build-status failed --json    # only PRs whose build rolls up to FAILED
bb pr view 42 --json                       # the pull request, plus all comments
bb pr view 42 --unresolved --json          # only the threads that still need an answer
bb pr diff 42                              # raw diff, plain text
bb pr files 42 --json                      # changed paths
bb pr commits 42 --json                    # commits, short hashes
bb pr mine --json                          # my PRs across every repo: authored + I review
bb pr mine --role reviewer --build --json  # only ones waiting on me, with build state
```

`bb pr view` returns `{ pull_request, general[], inline[] }`. Each comment has `id`, `author`,
`timestamp`, `body`, `file`, `line`, `resolved` and `parent`. Use the comment `id` to answer in the
correct thread. `parent` is `null` on the first comment of a thread, and holds that comment's id on
a reply. `resolved` tells you whether the thread is closed.

`bb pr list` returns `state` (raw API value, e.g. `"OPEN"`), `draft` (bool), and `reviewers`, an
array of `{name, uuid, state}` where `state` is `approved`, `changes_requested` or `pending`.
There is no `approvals` field.

Find the pull request for the current branch:

```bash
bb pr list --json | jq --arg b "$(git branch --show-current)" '.[] | select(.source == $b)'
```

## Build status

```bash
bb pr build 42 --json           # every check on one PR: key, name, state, url
bb pr list --build --json       # a BUILD column across a list
```

States: `successful | failed | inprogress | stopped | none`. That vocabulary is
`build_state`'s; `statuses[].state` and `build[].state` carry Bitbucket's raw
uppercase value (`FAILED`, `INPROGRESS`, …). `none` means no check reported, or no
check this version recognises — not that a check passed.

`build_state` is a worst-wins rollup over every check on the pull request
(`failed` > `stopped` > `inprogress` > `successful` > `none`), so asking "did anything
fail" is one field read. `build[]` on a list row, and `statuses[]` on `bb pr build`,
carry each individual check — read those to say *what* failed.

`--build` costs one extra request per pull request, because Bitbucket exposes build
status only per pull request. Combine it with a narrowing filter (`--author @me`,
`--needs-my-review`, a target branch) rather than running it bare on a busy repository.
Statuses are fetched only for pull requests that survive the other filters.

## Across repositories

`bb pr mine` is the only command that is not repository-scoped. There is no api call left that
discovers which workspaces you belong to, so the workspace(s) to scan are resolved in this order:
`--workspace <slug>[,<slug>...]` (comma-separated, highest precedence), then the `BB_WORKSPACE`
env var (same syntax), then the workspace of the git remote in the current checkout — which is
what makes a bare `bb pr mine` work inside a checkout. If none of the three apply, the command
errors rather than silently scanning nothing.

It returns `{ "pull_requests": [...], "partial": [...] }`. Each row carries `repo`
(`workspace/repo`), `my_role` (`author` | `reviewer` | `both`), `my_review_state`, `updated_on`,
and — with `--build` — `build_state` and `build[]`.

`--role author` costs one request to find who you are, then one paginated call per workspace. The
reviewer half costs one request to find who you are, one repository-listing call per workspace,
then one call per scanned repository — for `--role all` both halves run, so it is one call to find
who you are plus, per workspace, one authored call and one listing call followed by one call per
scanned repository. A workspace the token cannot read is listed in `partial` rather than failing
the command; say so when reporting from a partial scan.

**The reviewer-side scan is a recency window, not full workspace coverage.** It covers only the
`--repo-limit` most recently updated repositories per workspace (default 30) — on a workspace with
hundreds of repositories, that is a small slice by design, not an oversight. Never report a
`pr mine` brief as a complete picture of a workspace. If the user needs certainty about a specific
repository, use `bb pr list -R <repo>` for that repository instead.

For a ranked morning brief built on this command, the separate `bbc-daily-brief` skill carries the
ranking rules. Use it only when the user explicitly asks for a brief.

## Answer a review

```bash
# answer inside the thread you address
bb pr comment 42 --reply-to 998877 --body "Fixed in 1a2b3c4." --json

# raise a new point on one line
bb pr comment 42 -f src/auth.rs -l 88 --body "This drops the error." --json

# more than one paragraph
printf 'Refactored as suggested.\n\nThe parser is now its own module.\n' \
  | bb pr comment 42 --body-stdin --json
```

`--line` needs `--file`. `--reply-to` accepts neither, because a reply inherits the location of its
parent.

## Report threads, do not close them

Answer the comments. Report what you answered. Let the user close the threads.

Never resolve a thread on your own initiative. A resolved thread hides a reviewer's point, and only
the user can decide that the point is settled. This is the rule for approval and merge too.

List the threads that are still open, root comments only:

```bash
bb pr view 42 --unresolved --json | jq '.inline[] | select(.parent == null)'
```

You can recommend a thread to close. Wait for the answer, then resolve only the ids the user names:

```bash
bb pr resolve 42 998877 --yes --json   # {resolved,pull_request}
bb pr unresolve 42 998877 --json       # reopen a thread
```

`bb pr resolve` asks a human to confirm, and fails when it has no terminal. `--yes` answers that
prompt for you, so use it only for an id the user approved. Use one command for each thread. Do not
put it in a loop.

Resolve the first comment of a thread — the id whose `parent` is `null`. A reply id fails, and a
general comment fails: only inline threads carry a resolution.

## Never create a repository on your own initiative

`bb repo create` is a write to a shared workspace. Ask the human first, every time, and repeat
back the workspace, the name and the project key you are about to use. A stray repository in a
shared workspace is somebody's cleanup job.

Never pass `--public`. The default is private; making a repository public is a decision for the
human to state in words, and if they have not said "public" then they have not said it.

If `--project` is unknown, run `bb project list` and ask which one — do not guess from the name.

## Ask the author to change the code

Marking a pull request as changes requested is a verdict on someone's work, so the user
gives it, not you.

After you post review comments that ask the author to change something, ask the user once
whether to mark the pull request. On a yes, run it with `--yes`, because they just answered
the question the prompt exists to ask:

```bash
bb pr request-changes 42 --yes --json
```

Without `--yes` the command confirms with a human and fails when it has no terminal, so it
cannot be marked by accident. Declining is an error, not a quiet success.

Never mark changes requested on your own initiative, and never on a pull request you did not
just review.

**Never approve a pull request.** No command does it and there is no substitute to reach for.
Approval is a human action, like merging.

Withdraw a change request only after a re-review finds the earlier points addressed — offer
it, ask first, and never do it to clear the way for a merge:

```bash
bb pr no-request-changes 42 --yes --json
```

## Retargeting

A pull request opened against the wrong destination branch is fixed in place — there is no need
to close it and open another, and doing so throws away its review history:

```bash
bb pr retarget 42 --to main --json
```

Only the destination moves; the API does not let a pull request's source branch change. The
pull request must be open, and retargeting one that already targets that branch makes no write
and exits 0. Bitbucket recomputes the diff afterwards, so inline comments anchored to the old
base may start reading as outdated — say so when you report the change.

Unlike `resolve` and `request-changes`, this needs no confirmation: it corrects a mistake rather
than hiding or asserting a review point. Retarget when the user asks, or when you opened the
pull request against the wrong branch yourself.

## Reviewers

```bash
bb pr reviewers 42 --json                     # list, same as `list`
bb pr reviewers add 42 dana,ash --json  # tag reviewers, comma-separated
bb pr reviewers remove 42 ash --json       # untag a reviewer
```

Names match case-insensitively as a substring of display name or nickname, against the
repository's user list plus its default reviewers. An exact match wins over a longer substring
match. Ambiguous or no match is an error, exit 1 — the error lists the candidates when ambiguous.
Pass `{uuid}` in braces to skip name matching entirely; every error message suggests it.

Every name is resolved before any write, so one bad name in `add 42 a,b` writes nothing. Adding
someone already tagged makes no write and exits 0. Removing someone not tagged is an error, exit
1, with no write. Bitbucket rejects the PR's author as a reviewer (400, exit 1) — that's the
API's rule.

Approving, merging and declining are not supported. Do not attempt them. Resolving a comment
thread is supported, but only on the user's request — see
[Report threads, do not close them](#report-threads-do-not-close-them).

## Open a pull request

```bash
bb pr create main --title "Cache session lookups" --json
bb pr create main feat/cache --title "..." --description "..." --close-source-branch --json
bb pr create main,develop --title "..." --json      # one pull request per target
bb pr create main --title "..." --reviewer dana,ash --json   # exactly these two reviewers
```

The source branch defaults to the current checkout. The title defaults to
`Merge <source> into <target>`. `bb` attaches the default reviewers of the repository, and removes
you from that list. Pass `--no-default-reviewers` to attach none. Do not pass `-i`, because it
prompts.

`--reviewer` names the whole reviewer set: the repository's default reviewers are not attached at
all, so nobody the user did not pick arrives on the pull request. It takes the same names
`bb pr reviewers add` does — a case-insensitive substring of a display name or nickname,
comma-separated, or a `{uuid}` taken verbatim — and resolves every one of them **before** the pull
request is created, so a name that does not resolve opens nothing. Yourself is dropped rather than
rejected, because bitbucket answers 400 when the author is tagged.

Prefer `--reviewer` over creating first and fixing the list afterwards. To change the set on a pull
request that already exists, use `bb pr reviewers add|remove <id> <names>`.

For the full workflow — suggesting reviewers from the history of the files you changed, and
writing a description a human can skim — use the `bbc-open-pr` skill. It is installed by
`bb skill install`.

## Branches

```bash
bb branch list --json                       # newest commit first
bb branch list -u alice -n feat/ --json     # filter by author and by name
bb branch list --limit 20 --json
```

Both filters match a substring, and ignore case.

## Command map

| Command | Result |
|---|---|
| `bb pr list [target] [--state OPEN\|MERGED\|DECLINED\|SUPERSEDED\|DRAFT\|ALL] [--reviewer] [--author] [--review-state] [--needs-my-review] [--build] [--build-status <state>]` | `[{id,title,state,draft,author,source,destination,reviewers[],url}]`, plus `build_state` and `build[{key,name,state,url}]` when `--build` or `--build-status` is given |
| `bb pr view <id> [--unresolved] [--comments-only]` | `{pull_request,general[],inline[]}` |
| `bb pr diff <id>` | plain diff; `--json` wraps it as `{id,diff}` |
| `bb pr files <id>` | `[{status,path}]` |
| `bb pr commits <id>` | `[{hash,summary}]` |
| `bb pr build <id>` | `{build_state,statuses[{key,name,state,url}]}` |
| `bb pr mine [--role author\|reviewer\|all] [--state] [--workspace] [--repo-limit] [--build]` | `{pull_requests[{repo,id,title,url,state,draft,author,my_role,my_review_state,reviewers[],updated_on}],partial[]}` |
| `bb pr comment <id> …` | `{id,pull_request,url}` |
| `bb pr resolve <id> <comment> --yes` | `{resolved,pull_request}`; only on the user's request |
| `bb pr unresolve <id> <comment>` | `{unresolved,pull_request}` |
| `bb pr reviewers <id>` / `list <id>` | `[{name,uuid,state}]` |
| `bb pr reviewers add <id> <names>` / `remove <id> <names>` | `[{name,uuid,state}]` |
| `bb pr create <target> [source] …` | `[{id,target,url}]` |
| `bb pr retarget <id> --to <branch>` | `{id,title,source,destination,url}` |
| `bb pr request-changes <id> --yes` | `{requested_changes:<id>}`; only on the user's request |
| `bb pr no-request-changes <id> --yes` | `{unrequested_changes:<id>}`; only on the user's request |
| `bb branch list …` | `[{branch,user,updated}]` |
| `bb project list` | the projects in a workspace |
| `bb repo list [--project KEY]` | the repositories in a workspace or project |
| `bb repo create <name> --project KEY` | create a repository, private by default |
| `bb auth status` | `{email,token,account}`, token redacted |
| `bb browse --print [--pr <id>\|--branches]` | `{url}` |

`timestamp` and `updated` hold a relative time, for example `3 days ago`. For an exact time, read
the commit or the diff.

## When a command fails

- **Exit 2** — no credentials. Ask the user to run `bb auth login`. Do not run it yourself, because
  it prompts for a token. In CI, set `BB_EMAIL` and `BB_TOKEN`.
- **Exit 3** — the pull request, the branch, the comment or the repository does not exist. Confirm
  the id, and confirm the repository with `bb auth status` and `-R`.
- **A 403 message** — the API token misses a scope. `pr list` and `pr view` need
  `read:pullrequest:bitbucket`. `pr comment`, `pr resolve`, `pr unresolve`, `pr create` and
  `pr request-changes` and `pr retarget` need `write:pullrequest:bitbucket`. `branch list` and `pr create` also need
  `read:repository:bitbucket`.
- **`is a reply`, or `is not on the diff`** — the id is not the first comment of an inline thread.
  Read `parent` from `bb pr view`, and pass the id that has none.
- **`already resolved`** — the thread is closed. Nothing to do.
- **`no bitbucket.org remote found`**, or **`no git repository here`** — `bb` cannot find the
  repository. Pass `-R workspace/repo`, or set `BB_REPO`.

Bitbucket Cloud removed app passwords on 2026-07-28. `bb` authenticates with an Atlassian account
email and an API token. Never suggest an app password.

## Environment

| Variable | Purpose |
|---|---|
| `BB_EMAIL`, `BB_TOKEN` | credentials for CI and other non-interactive use |
| `BB_REPO` | default repository, the same as `-R` |
| `NO_COLOR` | disable colour and spinners |

Install: `brew install biokraft/tap/bb`, or `cargo install bbcloud --locked`. Run `bb --help` and
`bb <command> --help` for the full surface. Source and issues:
<https://github.com/biokraft/bbcloud>.
