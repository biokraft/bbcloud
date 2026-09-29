---
name: bitbucket-cloud
description: Reads, reviews, comments on, and reports Bitbucket Cloud pull requests with the `bb` CLI. Use for pull-request discovery, review context, comments, build or conflict status, and repository data in a Bitbucket Cloud repository. Do not use for opening a pull request, producing a daily brief, GitHub, or GitLab.
license: MIT
---

# Bitbucket Cloud with `bb`

`bb` is one binary for the Bitbucket Cloud REST API. Use it for all pull request work; do not use
`gh`, and do not send the user to the web UI.

This file carries the things `bb --help` cannot: the house rules, which decisions are the user's,
and what each failure means. `bb --help` and `bb <command> --help` carry the flag surface — read
them for syntax, not this file.

## Rules

1. Add `--json` to every command and parse it. Tables are for humans and their layout changes. The
   one exception is `bb pr diff <id>`, which is plain text.
2. Never resolve a comment thread, mark changes requested, approve, or merge on your own initiative.
   Each is a verdict on someone else's work. See [Verdicts belong to the user](#verdicts-belong-to-the-user).
3. Never pass `-w` or `--web`. They start a browser.
4. Branch on the exit code, not the error text: `0` success, `1` error, `2` not authenticated,
   `3` not found.
5. Give every comment a body: `--body` for one line, `--body-stdin` for more. With neither and no
   terminal, the command fails.
6. Add `-R workspace/repo` to act on another repository. Never guess a workspace.
7. `bb` may print `bb X.Y.Z is available …` on **stderr**. That is a notice, not a failure: the
   command succeeded and stdout is untouched. Mention it once, quoting the command it names. Do not
   run the upgrade, and do not repeat it later in the session.
8. In a new checkout, `bb skill install` sets up the agent skills. It needs no authentication.

## Operating contract

1. Establish the repository from the checkout or `-R` before anything else.
2. Use `--json` for machine decisions and keep the command's exit code.
3. Read before writing. Use the smallest command that changes exactly the field asked for.
4. After a write, verify the returned JSON, or run one read-back command.
5. On exit 2, exit 3, or a scope error, stop and report the next action. Never substitute a browser
   or another provider.

## Choose the command

| The user asked for | Run |
|---|---|
| what needs me / what is open | `bb pr list --needs-my-review --json` |
| my own pull requests, any repo | `bb pr mine --json` |
| the pull request for this branch | `bb pr list --current --json` |
| one pull request, with its comments | `bb pr view <id> --json` |
| what still needs an answer | `bb pr view <id> --unresolved --json` |
| is it safe to merge | `bb pr view <id> --build --conflicts --unresolved --json` |
| header only, comments are not needed | `bb pr view <id> --metadata-only --json` |
| what changed | `bb pr diff <id>` (plain text), `bb pr files <id> --json` |
| did CI pass | `bb pr build <id> --json` |
| who reviews | `bb pr reviewers <id> --json` |
| who *should* review | `bb pr reviewers suggest --pr <id> --acknowledge-private-data --json` |
| open a pull request | the `bbc-open-pr` skill, not this one |
| a daily brief | the `bbc-daily-brief` skill, and only on request |

`pr view` returns `{pull_request, comments_loaded, general[], inline[]}`. `comments_loaded` is
`false` for `--metadata-only`, so empty arrays then mean comments were skipped, not that there are
none. `--unresolved` adds `unresolved_threads`; `--build` adds `build`; `--conflicts` adds
`conflicts`. Those three are omitted unless asked for, and `--metadata-only` cannot be combined
with `--unresolved` or `--comments-only`.

A comment's `parent` is `null` on the first comment of a thread and holds that comment's id on a
reply — that is how you find the id to resolve. `created_on` is the exact RFC3339 value; the
human-facing `timestamp` may read "3 days ago", so use `created_on` for any age you compute.

## Answer a review

```bash
bb pr comment 42 --reply-to 998877 --body "Fixed in 1a2b3c4." --json   # in the thread you address
bb pr comment 42 -f src/auth.rs -l 88 --body "This drops the error." --json
printf 'Refactored.\n\nThe parser is now its own module.\n' | bb pr comment 42 --body-stdin --json
```

`--line` requires `--file`. `--reply-to` accepts neither, because a reply inherits its parent's
location. To batch a review, add `--pending`: only the user sees those comments until they press
"Finish review" in Bitbucket, and `bb` cannot publish them. The returned `url` is private to them
while pending, so do not share it.

## Report threads, do not close them

Answer the comments. Report what you answered. Let the user close the threads.

List the threads still open, roots only:

```bash
bb pr view 42 --unresolved --json | jq '.inline[] | select(.parent == null)'
```

Recommend a thread to close, wait for the answer, then resolve only the ids the user names — one
command each, never in a loop:

```bash
bb pr resolve 42 998877 --yes --json
bb pr unresolve 42 998877 --json
```

`resolve` asks a human and fails without a terminal. `--yes` answers that prompt for you, so use
it only for an id the user approved. Resolve the **first** comment of a thread, the one whose
`parent` is `null`: a reply id and a general comment are both rejected, because only inline threads
carry a resolution.

## Verdicts belong to the user

Each of these is a claim about someone else's work, and none of them is yours to make.

**Never mark changes requested on your own initiative**, and never on a pull request you did not
just review. After you have posted comments asking for changes, ask the user once. On a yes, the
prompt has already been answered, so pass `--yes`:

```bash
bb pr request-changes 42 --yes --json
```

Without `--yes` the command confirms with a human and fails with no terminal, so it cannot be
marked by accident. Declining is an error, not a quiet success. Withdraw a change request only
after a re-review finds the earlier points addressed — offer it, ask first, and never to clear the
way for a merge:

```bash
bb pr no-request-changes 42 --yes --json
```

**Never approve a pull request.** `bb` has no command for it, and reaching for another tool to
approve is wrong — the prohibition is the rule, not a missing feature. Approval is a human action,
like merging. Bitbucket removed app passwords on 2026-07-28; `bb` authenticates with an Atlassian
account email and an API token, and you must never suggest an app password.

## Writes that need no permission

These correct a mistake rather than assert anything, so they carry no gate. Do them when asked, or
when you caused the mistake.

```bash
bb pr retarget 42 --to main --json     # a pull request opened against the wrong target
bb pr edit 42 --title "Cache session lookups" --json
bb pr edit 42 --description-stdin --json < body.md
```

Retargeting keeps the review history; closing and reopening throws it away. Only the destination
moves — the API cannot change a source branch. Bitbucket recomputes the diff afterwards, so inline
comments anchored to the old base may read as outdated; say so when you report it.

`pr edit` changes only the fields you pass. `--description ""` clears it, and a no-op edit writes
nothing. Never run it with no flag, because it prompts. Print the new title and description back and
wait for a yes before writing.

## Writes that do need permission

`bb repo create` writes to a shared workspace. Ask every time, and repeat back the workspace, the
name and the project key before running it. A stray repository is somebody's cleanup job. Never
pass `--public`: if the user has not said "public" in words, they have not said it. If `--project`
is unknown, run `bb project list` and ask — do not infer it from the name.

## Reviewers

Names match case-insensitively as a substring of display name or nickname, against the repository's
user list and its effective default reviewers; an exact match beats a longer substring. Ambiguous
or unknown is an error, and the error lists the candidates. Pass `{uuid}` in braces to skip matching
entirely — every error message suggests it. `bb repo reviewers --json` shows the same pool plus any
source that could not be read. It is not called `members` because membership says nothing about
access to *this* repository.

`bb pr reviewers suggest` is read-only and never calls an add or create endpoint. It needs
`--acknowledge-private-data`, because it reads private file paths, colleague names, dates and
account ids out of commit history: name those categories and get a yes before running it. It
reports `target_commits` — who maintains the changed files — separately from `source_commits`,
who else worked on this branch, and gives each person a `can_review` of `yes`, `no` or `unknown`.
**File ownership is not repository access**: show the `commit_count`, `files`, `last_commit_on` and
`can_review` before asking the user to choose. When `history_complete` is `false`, say the scan was
partial and quote `files_skipped` and `paths_skipped` rather than presenting a truncated list as
the full set.

Every name resolves before any write, so one bad name in `bb pr reviewers add 42 a,b` writes
nothing. Adding someone already tagged writes nothing and exits 0; removing someone not tagged is
an error with no write. Bitbucket rejects the author as a reviewer — that is its rule, not a bug.

For the whole open-a-pull-request workflow, use the `bbc-open-pr` skill. For the cross-repository
brief, the `bbc-daily-brief` skill, and only when the user asks for one. `bb pr mine` returns
`{pull_requests, partial}` and each row carries `repo` and `my_role`.

## What each read costs

`--build` is one extra request per pull request, because Bitbucket exposes build status only per
pull request. `bb pr list --build-status <state> --json` filters on the rollup before the per-row
fetch and is the cheaper way to ask.
`build_state` is a worst-wins rollup (`failed` > `stopped` > `inprogress` > `successful` > `none`) —
one field answers "did anything fail". Read `statuses[]` or `build[]` for *what* failed. `none` means
no check reported, not that one passed. Statuses are fetched only for pull requests that survive the
other filters, so narrow the list before asking for build state.

`bb pr mine` is the only command that is not repository-scoped. It scans the `--repo-limit` most
recently updated repositories per workspace (default 30) — a recency window, not full coverage, and
never report it as a complete picture of a workspace. A workspace the token cannot read is listed
in `partial` rather than failing the command; say so when reporting from a partial scan. For one
known repository, use `bb pr list -R <repo>` instead.

## When a command fails

- **Exit 2** — no credentials. Ask the user to run `bb auth login`; it prompts, so do not run it
  yourself. In CI, set `BB_EMAIL` and `BB_TOKEN`.
- **Exit 3** — the pull request, branch, comment or repository does not exist. Confirm the id, and
  confirm the repository with `bb auth status` and `-R`.
- **A 403** — the token misses a scope. Reads need `read:pullrequest:bitbucket`; writes need
  `write:pullrequest:bitbucket`; `pr view --conflicts` and `bb branch list` need
  `read:repository:bitbucket`.
- **`is a reply`, or `is not on the diff`** — you passed the wrong id. Read `parent` from
  `bb pr view` and pass the id that has none.
- **`already resolved`** — the thread is closed. Nothing to do.
- **`no bitbucket.org remote found`**, or **`no git repository here`** — pass `-R workspace/repo`,
  or set `BB_REPO`.

Source and issues: <https://github.com/biokraft/bbcloud>.
