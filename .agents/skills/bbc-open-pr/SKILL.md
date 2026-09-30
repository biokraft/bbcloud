---
name: bbc-open-pr
description: Opens a Bitbucket Cloud pull request with the `bb` CLI — pushes the branch, suggests reviewers from the history of the changed files, drafts a reviewer-first description, and gets the user's approval before it creates the pull request or tags anyone. Use when the user asks to open, raise or create a pull request and the git remote is bitbucket.org. Do not use to review an existing pull request, for a daily brief, or for GitHub or GitLab.
license: MIT
---

# Open a Bitbucket Cloud pull request

`bb pr create` takes a target, a title, a description and a reviewer list. It does not push the
branch, write the description or choose the reviewers. This skill does those, in order. Two steps
stop and ask the user. Do not skip them.

## Operating contract

- Add `--json` to every `bb` command, and branch on the exit code: `2` not authenticated (ask the
  user to run `bb auth login`; never run `bb auth logout`), `3` not found.
- Find the source and the target branch before you propose a title or reviewers.
- Collect evidence before you draft. Never invent ownership, tests or risks.
- Stop at the description gate and at the reviewer gate. Run `bb pr create` only after the user
  approves both.
- Tag only the reviewers the user picks. Pass them as `{uuid}` values when `bb` gave you one.
- Never pass `-i` or `-w` to `bb pr create`. `-i` waits at a prompt, and `-w` opens a browser.
- Never approve, merge or decline a pull request, and never resolve a comment thread.

## Step 1 — prepare the branch

```bash
git rev-parse --abbrev-ref HEAD                      # the source branch
git status -sb                                       # uncommitted work, and commits not pushed
git symbolic-ref --short refs/remotes/origin/HEAD    # the default branch, such as origin/main
```

Bitbucket builds the pull request from the remote branch, not from your checkout. If the branch
has no upstream, or has commits that are not pushed, push it: `git push -u origin HEAD`. If there
is uncommitted work, ask the user whether it belongs in this pull request. If `symbolic-ref` fails,
`git remote set-head origin --auto` sets the default branch.

Check that the branch has no open pull request yet:

```bash
bb pr list --current --json
```

If it returns a row, do not open a second one. Report its `url`, and offer to edit it instead.

The target is usually the default branch. A branch cut from a release or an integration branch
targets that branch instead. The merge base shows where the branch started:

```bash
git log --oneline --decorate -1 "$(git merge-base HEAD <default-branch>)"
```

If the source does not descend from the default branch, ask the user for the target. Use the
target you find here in every later step. Do not assume `main`. Pass it to `bb` as a branch name,
without the `origin/` prefix.

Find the title convention. If the repository squash-merges, the title becomes the commit message.
If the repository parses commits — conventional commits, release automation — a wrong title gives
a wrong changelog entry or a wrong version. Read `CONTRIBUTING.md`, `AGENTS.md` and
`git log --oneline -10 <target>` for the convention. Always pass `--title`: without it, `bb` uses
"Merge <source> into <target>".

Do not run the test suite here. The task that asked for this pull request owns that.

## Step 2 — suggest reviewers

The default reviewers are a static list. The people who know the changed files are in the history.

**This reads private data.** Tell the user that the command reads private file paths, colleague
names, dates and account ids from commit history, and get a yes. Without that yes, do not run it,
and do not guess reviewers.

```bash
bb pr reviewers suggest <target> --acknowledge-private-data --json
```

The source defaults to the current branch. The command reads the pushed branch and never writes.
It leaves out the authenticated user, because Bitbucket rejects the author as a reviewer.

Each row in `suggestions[]` holds two separate claims. `target_commits` counts commits on the
target branch: the person maintains the files. `source_commits` counts commits only on this branch:
the person worked on this change. Keep the two claims apart when you present them. A renamed file
is read under both paths, and a deleted file under its old path.

The report states how complete it is. `files_scanned` and `files_skipped` count changed files.
`paths_scanned` and `paths_skipped` count distinct paths. `history_complete` is `false` when
anything was cut, and `errors[]` names the paths that failed. Pass these facts on. Never present a
partial scan as complete.

`can_review` is separate, because file ownership is not repository access. `yes`: the person is in
the repository's permissions. `no`: all user lists were read, and the person is not in them.
`unknown`: no complete list was available. Never present a `no` or `unknown` row as a reviewer who
can be tagged. Show the name, and let the user decide.

## Step 3 — resolve other names

Suggestions already carry exact `uuid` values. If the user names someone else, look them up:

```bash
bb repo reviewers --json
```

It returns `{users[], partial[]}` from the same three lists that `--reviewer` searches. Match the
name against `name` and `nickname`, and take the `uuid`. Put a person with no plausible match under
**could not be mapped**, with the name the user gave. If a name matches more than one person, ask
the user to choose. Do not guess.

`eligibility` is `explicit` only for users in the repository's own permissions. An `unknown` row
comes from workspace membership or the default reviewers, with no proven access to this
repository. Present it as a candidate, not as a confirmed reviewer.

When `partial` is not empty, `--reviewer` refuses every name, because a list it cannot read can
hide the person the user means. Pass `{uuid}` values then.

## Step 4 — the description gate

Write the description to a file outside the working tree, so it is never committed:

```bash
body=$(mktemp)
```

Then **print the description back** to the user in full, exactly as it will appear, and ask for
approval. If the user asks for changes, redraft and show it again.
Do not create the pull request before the user approves the text.

### The shape

Bitbucket renders no raw HTML in descriptions, so the GitHub collapsible `details` block shows
nothing. Use order instead: what matters first, and detail low enough that a skimmer stops before
it.

```markdown
Caches session lookups, cutting p99 auth latency from 180ms to 12ms. No API or
schema change, and the cache is bypassed entirely when the store is unreachable.

## Why

Every authenticated request hit the session store, and the store is a single
instance shared with three other services…

## What changed

| Area | Change |
|---|---|
| `src/auth.rs` | an LRU in front of the session store |
| `src/config.rs` | `session_cache_ttl`, defaulting to 60s |

## Details

The TTL is deliberately short…

## Testing

`cargo test --all`, plus a load test at 2k rps…
```

### The rules

- **The first paragraph is the pull request for most readers.** Write two to four sentences, with
  no heading above them. Say what the change does, what it risks or costs, and what it does not
  touch. A reader who stops there must know whether to care.
- `## What changed` has one row per area, not per file. One row per file repeats
  `git diff --stat`, which the reader already has.
- `## Details` and `## Testing` come last, and can be as long as necessary. Put the reasoning, the
  rejected alternatives and the test evidence there.
- Use only what Bitbucket renders: headings, tables, fenced code, links, lists and emphasis.
- No emoji, no badges, nothing that needs a legend.

## Step 5 — the reviewer gate

If the user named reviewers in the request, use exactly those, and ask nothing.

Otherwise, show each candidate with its evidence: `target_commits`, `source_commits`, `files`,
`last_commit_on` and `can_review`. Ask which to add. Make it a pick, not a yes or no on the full
list: the user often wants two of your five. "No one" is a valid answer, and so is a name you did
not suggest.

**Never tag anyone the user did not pick.** A review request is a claim on someone's time.

## Step 6 — create

```bash
bb pr create <target> --title "<title>" --description-stdin --reviewer '{uuid-1},{uuid-2}' --json < "$body"
```

`--reviewer` is the full reviewer set. It replaces the repository's default reviewers, so only the
people the user picked are on the pull request. `bb` removes you from the list, because Bitbucket
rejects the author. If the user picked no one, pass `--no-default-reviewers` and no `--reviewer`.
With neither flag, `bb` attaches the default reviewers, and the user did not choose them.

`bb` resolves every reviewer before it creates anything, so a bad value opens nothing. Fix the
value, and run the same command again.

The JSON is an array with one `{id, target, url}` for each target. Report the `url`.

## After the pull request exists

To change the reviewers:

```bash
bb pr reviewers <id> --json                   # who is tagged, and what each decided
bb pr reviewers remove <id> ash --json        # an error if ash is not tagged
bb pr reviewers add <id> '{uuid-1}' --json
```

To change the title or the description, show the new text and get a yes, as in Step 4:

```bash
bb pr edit <id> --title "<title>" --json
bb pr edit <id> --description-stdin --json < "$body"
```
