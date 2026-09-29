---
name: bitbucket-cloud
description: Reads, reviews and answers Bitbucket Cloud pull requests with the `bb` CLI — comments, review threads, diffs, build status, conflicts, reviewers and repository data. Use when the git remote is bitbucket.org, or when the task names a Bitbucket pull request, a review comment, or a `bb` command. Do not use to open a pull request, for a daily brief, or for GitHub or GitLab.
license: MIT
---

# Bitbucket Cloud with `bb`

`bb` is the CLI for the Bitbucket Cloud REST API. Use it for all pull request work. Do not use
`gh`, and do not send the user to the web UI.

This file holds what `bb --help` does not: the house rules, the decisions that belong to the user,
and what each failure means. For flags and syntax, read `bb <command> --help`.

## Operating contract

1. Add `--json` to every command and parse stdout. Tables are for humans, and their layout can
   change. One exception: read a diff as plain text with `bb pr diff <id>`. With `--json`, the
   diff is a string in `{id, diff}`.
2. Branch on the exit code, not on the error text: `0` success, `1` error, `2` not authenticated,
   `3` not found.
3. `bb` acts on the repository of the current checkout. For another repository, add
   `-R <workspace>/<repo>`. Never guess a workspace.
4. Read before you write. Use the smallest command that changes only the field the user asked for.
   After a write, check the JSON it returns.
5. Never resolve a thread, request changes, approve or merge on your own initiative. See
   [Verdicts belong to the user](#verdicts-belong-to-the-user).
6. Never pass `-w`/`--web` or `-i`/`--interactive`. The first opens a browser. The second waits for
   a human at a prompt.
7. Give every comment a body: `--body` for one line, `--body-stdin` for more. With neither and no
   terminal, the command fails.
8. On exit 2, exit 3 or a `forbidden` error, stop and tell the user the next step. Never fall back
   to a browser or to another provider.
9. `bb` can print `bb X.Y.Z is available …` on stderr. That is a notice, not a failure: the command
   succeeded, and stdout is clean. Tell the user once, and quote the upgrade command it names. Do
   not run the upgrade.
10. If a skill this file names is not installed, `bb skill install --all` installs the full set. It
    needs no credentials.

## Choose the command

| The user asks for | Run |
|---|---|
| what waits on my review | `bb pr list --needs-my-review --json` |
| my pull requests in every repository | `bb pr mine --json` |
| the pull request for this branch | `bb pr list --current --json` |
| one pull request and its comments | `bb pr view <id> --json` |
| the threads that still need an answer | `bb pr view <id> --unresolved --json` |
| is it ready to merge | `bb pr view <id> --build --conflicts --unresolved --json` |
| the header only | `bb pr view <id> --metadata-only --json` |
| what changed | `bb pr diff <id>`, `bb pr files <id> --json`, `bb pr commits <id> --json` |
| did CI pass | `bb pr build <id> --json` |
| who reviews, and what each decided | `bb pr reviewers <id> --json` |
| who should review | `bb pr reviewers suggest --pr <id> --acknowledge-private-data --json`, after the consent in [Reviewers](#reviewers) |
| open a pull request | the `bbc-open-pr` skill, not this one |
| a daily brief | the `bbc-daily-brief` skill, and only when the user asks for one |
| report a bug in `bb` | the `bbc-report-bug` skill, and only when the user asks for one |

### Read `pr view`

`bb pr view <id> --json` returns `{pull_request, comments_loaded, general[], inline[]}`. Each flag
adds one key, and the key is absent without its flag: `--unresolved` adds `unresolved_threads`,
`--build` adds `build` (`build_state`, `statuses[]`), and `--conflicts` adds `conflicts` (`count`,
`files[]`).

`--metadata-only` skips the comments and sets `comments_loaded` to `false`. Empty arrays then mean
"not fetched", not "none". It cannot be combined with `--unresolved` or `--comments-only`.

Each comment carries `id`, `author`, `created_on`, `body`, `file`, `line`, `resolved`, `pending` and
`parent`. `parent` is `null` on the first comment of a thread, and holds the parent's id on a reply.
To find the root of a thread, follow `parent` until it is `null`. Compute ages from `created_on`
(RFC 3339). `timestamp` is display text, such as "3 days ago".

## Answer a review

```bash
bb pr comment 42 --reply-to 998877 --body "Fixed in 1a2b3c4." --json
bb pr comment 42 -f src/auth.rs -l 88 --body "This drops the error." --json
printf 'Refactored.\n\nThe parser is now its own module.\n' | bb pr comment 42 --body-stdin --json
```

Answer in the thread you address. `--line` needs `--file`. `--reply-to` takes neither, because a
reply keeps the location of its parent.

`--pending` saves a draft comment. Only the user sees it until they select "Finish review" in
Bitbucket, and `bb` cannot publish it. Its `url` is private while it is pending, so do not share it.
If the JSON shows `"pending": false`, Bitbucket published the comment at once. Tell the user.

## Report threads, do not close them

Answer the comments. Report what you answered. The user closes the threads.

List the open threads, root comments only:

```bash
bb pr view 42 --unresolved --json | jq '.inline[] | select(.parent == null)'
```

Recommend the threads to close, then wait. Resolve only the ids the user names, one command for
each id, never in a loop:

```bash
bb pr resolve 42 998877 --yes --json
bb pr unresolve 42 998877 --json
```

Without `--yes`, `resolve` asks a human, and fails when there is no terminal. `--yes` answers that
prompt for the user, so use it only for an id the user approved.

Only the root of an inline thread can be resolved: `parent` is `null` and `file` is set. Check both
before you send. With `--yes`, `bb` skips its own check, and Bitbucket answers a reply or a general
comment with a `forbidden` error that looks like a scope problem.

## Verdicts belong to the user

Each verdict is a claim about someone else's work. None of them is yours to make.

**Never mark changes requested on your own initiative**, and never on a pull request you did not
just review. After you post comments that ask for changes, ask the user once. On a yes, the user
has answered the prompt, so pass `--yes`:

```bash
bb pr request-changes 42 --yes --json
```

Without `--yes`, the command asks a human, and fails when there is no terminal. A "no" at the
prompt exits `1`, so the mark cannot land by accident.

Withdraw a change request only after a re-review finds the points addressed. Offer it, and ask
first. Never withdraw it only to clear the way for a merge:

```bash
bb pr no-request-changes 42 --yes --json
```

**Never approve a pull request.** `bb` has no command for it, and that is deliberate. Do not use
another tool to approve. Approval is a human action, like a merge.

## Edit a pull request

These commands have no confirmation prompt, so the check is yours.

```bash
bb pr retarget 42 --to main --json
bb pr edit 42 --title "Cache session lookups" --json
bb pr edit 42 --description-stdin --json < body.md
```

Retarget when the user asks, or when you opened the pull request against the wrong branch. A
retarget keeps the review history. Close-and-reopen loses it. Only the destination changes: the
API cannot change the source branch. Bitbucket then computes a new diff, so inline comments on the
old base can show as outdated. Say so in your report.

`pr edit` changes only the fields you pass. `--description ""` clears the description. An edit that
changes nothing writes nothing. Show the user the new title or description, and wait for a yes
before you write.

## Create a repository

`bb repo create` writes to a shared workspace. Ask each time. Before you run it, repeat the
workspace, the name and the project key to the user. Never pass `--public` unless the user said
"public". If the project key is not known, run `bb project list --json` and ask. Do not infer the
key from the name.

## Reviewers

A reviewer name matches case-insensitively as a substring of a display name or a nickname. `bb`
searches three lists: the workspace members, the users in the repository's permissions, and the
effective default reviewers. An exact match wins over a longer substring match. An ambiguous or
unknown name is an error, and the error lists the candidates. A `{uuid}` in braces skips the search.

```bash
bb pr reviewers add 42 dana,ash --json
bb pr reviewers remove 42 ash --json
bb repo reviewers --json
```

`bb` resolves every name before it writes, so one bad name writes nothing. To add someone already
tagged writes nothing and exits `0`. To remove someone not tagged is an error. Bitbucket rejects
the author as a reviewer. That is its rule, not a bug.

A command that writes a reviewer refuses a name when one of the three lists cannot be read, because
the missing list can hide the person the user means. Pass the `{uuid}` then. `bb repo reviewers`
returns `{users[], partial[]}`. Each user has `name`, `nickname`, `uuid`, `sources` and
`eligibility`. `partial` names the lists that could not be read. `eligibility` is `explicit` only
for users in the repository's own permissions. A workspace member has no proven access to this
repository.

`bb pr reviewers suggest` is read-only. It reads commit history, so its output holds private file
paths, colleague names, dates and account ids. Name those four categories to the user, and get a
yes before you pass `--acknowledge-private-data`.

Each row in `suggestions[]` has `name`, `uuid`, `commit_count`, `target_commits`, `source_commits`,
`files`, `last_commit_on` and `can_review`. `target_commits` counts commits on the target branch:
the person maintains these files. `source_commits` counts commits only on this branch: the person
worked on this change. `can_review` is `yes` (in the repository's permissions), `no` (all lists were
read and the person is not in them) or `unknown`. **File ownership is not repository access.** Show
these facts, and let the user choose. When `history_complete` is `false`, say that the scan is
partial, and quote `files_skipped`, `paths_skipped` and `errors[]`.

## Cost and coverage

- `--build` costs one request per pull request, because Bitbucket reports builds per pull request.
  `bb pr list` fetches builds only for the rows that pass its other filters. Narrow the list first,
  then add `--build` or `--build-status <state>`. `--build-status` filters the output. It does not
  save requests.
- `build_state` is a worst-wins rollup: `failed` > `stopped` > `inprogress` > `successful` > `none`.
  One field tells you if anything failed. `none` means that no check reported, not that a check
  passed. To find the check that failed, read `statuses[]` from `bb pr build`, or `build[]` from a
  list row.
- `bb pr mine` is the only command that is not scoped to one repository. It returns
  `{pull_requests, partial}`, and each row carries `repo` and `my_role`. Its reviewer half reads only
  the `--repo-limit` most recently updated repositories per workspace (default 30). Never report
  its result as complete. `partial` lists the workspaces the token cannot read; mention them. For
  one repository, use `bb pr list -R <workspace>/<repo>` instead.
- Commands that act on a workspace take it from `--workspace`, then `BB_WORKSPACE`, then the remote
  of the current checkout. With none of these, they fail. Ask the user for the workspace.

## When a command fails

| Signal | Cause | Next step |
|---|---|---|
| exit 2 | no valid credentials | Ask the user to run `bb auth login`. It prompts, so do not run it yourself. In CI, set `BB_EMAIL` and `BB_TOKEN`. |
| exit 3 | the pull request, comment, branch or repository does not exist | Check the id, and the repository in `-R`. |
| exit 3 from `unresolve` | the thread is not resolved | Nothing to do. |
| `forbidden` from `resolve --yes` | the id is a reply or a general comment | Pass the root of an inline thread. |
| `forbidden` elsewhere | the token lacks a scope | `bb auth login --help` lists each scope and the commands that need it. |
| 409 from `resolve` | the thread is already resolved | Nothing to do. |
| `rate limited` | too many requests | Wait, then retry once. |
| `no bitbucket.org remote found`, `no git repository here` | no repository to act on | Pass `-R <workspace>/<repo>`, or set `BB_REPO`. |

`bb` authenticates with an Atlassian account email and an API token. Never suggest an app password:
Bitbucket removed them.
