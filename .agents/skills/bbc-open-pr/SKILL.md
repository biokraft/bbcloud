---
name: bbc-open-pr
description: Open a Bitbucket Cloud pull request with the `bb` CLI — suggest reviewers from the history of the files you changed, write a description a human can skim, and get the user's approval before either lands. Use this skill when the task is to open, raise or create a pull request on Bitbucket Cloud. Do not use it for GitHub or GitLab.
---

# Open a Bitbucket Cloud pull request

Opening a good pull request is three jobs: get the branch into a state Bitbucket can see,
write a description someone can act on in ten seconds, and put it in front of the people who
know the code. `bb pr create` does none of that for you — it takes a title, a target, and the
reviewers you name, and attaches whatever static list the repository has configured as default
reviewers when you name none.

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
history. Ask `bb` for bounded, evidence-backed suggestions before proposing anyone:

```bash
bb pr reviewers suggest <target> [source] --json
```

Use the target branch from Step 1, not a hardcoded `main`. The command reads the changed-file
diffstat, checks recent path history, and returns commit counts, matching files, and dates. It
also reports skipped files, per-path errors, and `history_complete`; never hide those facts.

The command excludes the pull-request author and current reviewers. It does not resolve arbitrary
Git names by email and never writes reviewer tags. Keep its `uuid` values when the user selects
people.

## Step 3 — resolve those names against Bitbucket

Suggestions already carry exact UUIDs. If the user names someone else, use
`bb repo members --json` to resolve that name against the same workspace, repository-permission,
and effective-default-reviewer pools used by reviewer resolution:

```bash
bb repo members -R <workspace>/<repo> --json
```

Use each returned `name`, `nickname`, and `uuid` to match the Git candidates. `partial` tells you
which pools could not be read; never present those results as complete. A candidate with no
plausible match goes under **could not be mapped** with the Git name. If a name is ambiguous,
keep the candidate unselected and let the user choose a UUID; do not guess.

Every check happens before any write, and so does `--reviewer`'s own resolution: one bad name
fails before a pull request is created.

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

Pass the body with `--description-stdin` when it came from a file or pipe; do not pass `-i`, which
opens an editor and prompts.

## Step 5 — the reviewer gate

If the user named reviewers when they invoked this skill, use exactly those and ask nothing.

Otherwise show your resolved suggestions — each with its recent-commit count, files, and date —
and ask which to add. Prefer the returned UUIDs in the eventual `--reviewer` list. Present it as
a pick, not a yes/no on the whole list: the user often wants two of your five.

**Never tag anyone the user did not pick.** A review request is a claim on someone's
attention. "No one" is a valid answer, and so is a name you did not suggest.

## Step 6 — create, then tag

Both gates are behind you, so the pull request can be created complete, in one call:

```bash
bb pr create <target> --title "<title>" --description-stdin --reviewer dana,ash --json < <path-to-body-file>
```

`--reviewer` is the whole reviewer set. The repository's default reviewers are not attached at
all, so the user's pick is exactly who ends up on the pull request — no one arrives uninvited and
there is nothing to clean up afterwards. Report the URL from the JSON.

When the user picked nobody, pass `--no-default-reviewers` and no `--reviewer`. Omitting both
attaches whatever static default list the repository has, which is not a pick the user made.

Every name resolves before the create, so one bad name opens nothing — fix the name and run the
same command again. Yourself is dropped rather than rejected.

If the reviewer set has to change after the fact — the user changes their mind, or a default list
came along from an earlier create — edit it in place:

```bash
bb pr reviewers <id> --json                 # who is tagged now, and each decision
bb pr reviewers remove <id> unwanted --json # untag; comma-separate for several
bb pr reviewers add <id> dana --json
```

`remove` errors if the name is not a reviewer on that pull request, so a mistyped removal cannot
look like it worked.

If the title or description has to change after the pull request exists, edit it in place —
print the new text back and get the user's yes first, exactly as in Step 4:

```bash
bb pr edit <id> --title "<title>" --json
bb pr edit <id> --description-stdin --json < <path-to-body-file>
```

## Never

- Never approve, merge or decline a pull request. Not supported, and not yours to do.
- Never resolve a comment thread on your own initiative.
- Never pass `-i` to `bb pr create` — it prompts, and you will hang.
- Never open the pull request before the description is approved and the reviewers are picked.
