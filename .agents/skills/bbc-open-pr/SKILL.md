---
name: bbc-open-pr
description: Open a Bitbucket Cloud pull request with the `bb` CLI — suggest reviewers from the history of the files you changed, write a description a human can skim, and get the user's approval before either lands. Use this skill when the task is to open, raise or create a pull request on Bitbucket Cloud. Do not use it for GitHub or GitLab.
---

# Open a Bitbucket Cloud pull request

Opening a good pull request is three jobs: get the branch into a state Bitbucket can see,
write a description someone can act on in ten seconds, and put it in front of the people who
know the code. `bb pr create` does none of that for you — it takes a title and a target and
attaches whatever static list the repository has configured as default reviewers.

Work through the steps in order. Two of them stop and ask the user; neither is optional.

For the full command reference — flags, JSON shapes, exit codes — see the `bitbucket-cloud`
skill.

## Step 1 — pre-flight

```bash
git rev-parse --abbrev-ref HEAD                    # the source branch
git rev-parse --abbrev-ref --symbolic-full-name @{u} 2>/dev/null   # is it on the remote?
```

If the branch has no upstream, push it: `git push -u origin HEAD`. Bitbucket cannot open a
pull request for a branch it has never seen, and the failure message does not say so.

Determine the target branch rather than assuming `main`. The repository's main branch is the
usual answer, but a branch cut from a release or integration branch targets that one:

```bash
git log --oneline --decorate -1 "$(git merge-base HEAD origin/main)"
```

If the source branch's history does not descend from the main branch, ask the user what to
target. Whatever you determine here is "the target branch" for the rest of this skill — use it,
not a hardcoded `main`, in every merge-base command below.

**If the repository squash-merges, the pull request title becomes the commit message.** Where
the repository parses commits — a conventional-commit changelog, release automation — the
title must follow that convention, or it produces a wrong changelog entry or a wrong version
number. Check `CONTRIBUTING.md`, `AGENTS.md` or the last few commits on the main branch for
the convention before writing the title.

Do not run the repository's test suite here. Whatever asked you to open this pull request owns
that.

## Step 2 — find the people who know these files

The default reviewers are a static list. The people who wrote the code you changed are in the
history. Get the changed files first, then their history:

```bash
git diff --name-only "$(git merge-base HEAD origin/<target>)"..HEAD
git log --follow --format='%an|%ae|%ad' --date=short -- <file>
```

Use the target branch from Step 1 in place of `<target>` above — not `main`. For a branch cut
from a release or integration branch, diffing against `main` pulls in that branch's files too,
and ranks people who never touched this change.

`--follow` matters: a renamed file still reports the people who worked on it under its old
name, and those are exactly the people you want.

The `%ae` email is for telling apart two commits from the same person written under different
name spellings — it is never something to pass to `bb pr reviewers add`, which rejects an
email outright.

Rank candidates by **commits in the last twelve months**, not all-time count. Someone who
wrote one line in 2019 is not the reviewer; whoever has been maintaining the file is. Then:

- **Drop yourself.** Bitbucket rejects the pull request's author as a reviewer with a 400, so
  suggesting yourself only produces a failed write.
- **Drop candidates whose most recent commit to any changed file is older than about a year.**
  That is archaeology, not review capacity.
- Note, per candidate, which files earned them the suggestion and how many recent commits they
  have. The user needs that to choose.

## Step 3 — resolve those names against Bitbucket

Git records an author as a name and an email. `bb pr reviewers add` resolves a
case-insensitive substring of a Bitbucket display name or nickname, matched against workspace
members, the repository's permission-config users, and its default reviewers — a pool no `bb`
command lists. An unresolvable name fails at add time with exit 1 — after the pull request
already exists.

No command can tell you the full resolvable pool, so resolvability cannot be fully verified
before the write. The best available read-only approximation is the names Bitbucket already
shows on this repository's pull requests — anyone who has authored or reviewed recently is most
plausible as a reviewer for a new one:

```bash
bb pr list --state ALL --json      # each row's author and reviewers[]
```

Build a name pool from every `author` and every entry in `reviewers[]` across that list. Match
each candidate's git name against the pool the same way `bb pr reviewers add` does: a
case-insensitive substring.

- A candidate that matches the pool is a reasonably safe suggestion.
- A candidate that does not match the pool is not necessarily unresolvable — the pool is only an
  approximation — but suggest them labeled explicitly as **unverified**, so the user knows the
  add can still fail with exit 1.
- A candidate you cannot match at all — no plausible name in the pool, nothing close — goes
  under the heading `could not be mapped`, with their git name. Never silently drop them: the
  most likely reviewer is often behind a name-format mismatch, and the user can map them by hand
  in one word.

If a name is ambiguous, pass the account's `{uuid}` in braces to skip name matching entirely.
The only source for a uuid is `bb pr reviewers <pr-id> --json` on some pull request the person
is already tagged on — so this only works for people who have reviewed before. When no uuid is
available, ask the user for the person's exact Bitbucket display name instead of guessing.

Every one of these checks happens before any write: one bad name in a later `add` call fails
the whole call, but nothing has been created yet at this point.

## Step 4 — draft the description, then get it approved

Write the body, then **print the description back** to the user in full, exactly as it will
appear, and ask whether to open the pull request with it. If they ask for changes, redraft and
show it again. Do not create the pull request until the body is approved.

### The shape

Bitbucket Cloud renders no raw HTML in descriptions, so the `details`/`summary` collapsible
section — the GitHub idiom — renders as nothing at all. Progressive disclosure is done by **ordering**: the
thing that matters first, the detail far enough down that a skimmer never reaches it.

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

- **The first paragraph is the whole pull request for most readers.** Two to four sentences,
  no heading above it. Say what the change does, what it risks or costs, and what it does not
  touch. Someone who stops reading there must still know whether they need to care.
- **`## What changed` is one row per area, not per file.** Twelve rows for a twelve-file
  change is a restatement of `git diff --stat`, which the reader already has.
- **`## Details` and `## Testing` come last** and may be as long as they need to be. Reasoning,
  alternatives you rejected, and verification evidence go there.
- Use only markdown Bitbucket renders: headings, tables, fenced code, links, lists, emphasis.
- No emoji. No status badges. Nothing that needs a legend.

Pass the body with `--description`. For a long body, write it to a file and pass the file's
contents; do not pass `-i`, which opens an editor and prompts.

## Step 5 — the reviewer gate

If the user named reviewers when they invoked this skill, use exactly those and ask nothing.

Otherwise show your resolved suggestions — each with its recent-commit count and the files
behind it — and ask which to add. Present it as a pick, not a yes/no on the whole list:
the user often wants two of your five.

**Never tag anyone the user did not pick.** A review request is a claim on someone's
attention. "No one" is a valid answer, and so is a name you did not suggest.

## Step 6 — create, then tag

Both gates are behind you, so the pull request can be created complete:

```bash
bb pr create <target> --title "<title>" --description "<body>" --json
bb pr reviewers add <id> dana,ash --json
```

Two calls, because `bb pr create` has no reviewer flag. Report the URL from the first call's
JSON.

`bb pr create` attaches the repository's default reviewers and removes you from that list.
Pass `--no-default-reviewers` when the user's pick should be the whole list.

Every name is resolved before any write, so one bad name in `add <id> a,b` writes nothing.

## Never

- Never approve, merge or decline a pull request. Not supported, and not yours to do.
- Never resolve a comment thread on your own initiative.
- Never pass `-i` to `bb pr create` — it prompts, and you will hang.
- Never open the pull request before the description is approved and the reviewers are picked.
