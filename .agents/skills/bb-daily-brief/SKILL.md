---
name: bb-daily-brief
description: Produce a ranked, actionable daily brief of the user's Bitbucket Cloud pull requests across every repository. Use ONLY when the user explicitly asks for a daily brief, a standup summary, or "what needs my attention" across repositories. Never invoke this skill proactively, and never as a step inside another task.
license: MIT
---

# Daily brief

One ranked list of what needs the user's attention across every Bitbucket repository, built from
`bb`. Nothing else.

## Rules

1. **Explicit invocation only.** Produce a brief when the user asks for one in those words — a
   daily brief, a standup summary, what needs their attention. If the question is narrower ("what
   is failing on PR 42", "who is reviewing my branch"), answer it with plain `bb` commands and do
   not produce a brief.
2. The `bitbucket-cloud` skill's rules bind here too: `--json` on every command, never `-w` or
   `--web`, use exit codes rather than error text.
3. **Never resolve a comment thread.** A brief reports threads; it does not close them. This holds
   even when the thread looks answered.
4. Never write a comment, approve, or merge while building a brief. It is read-only.
5. Report an incomplete scan. If phase 1 returns a non-empty `partial`, the brief opens with one
   line naming those workspaces.

## Phase 1 — structural scan, cheap

```bash
bb pr mine --build --json
```

Returns `{ "pull_requests": [...], "partial": [...] }`. Each row carries `repo`, `id`, `title`,
`url`, `state`, `draft`, `author`, `my_role` (`author` | `reviewer` | `both`), `my_review_state`
(`approved` | `changes_requested` | `pending`, or `null` when I am not a reviewer), `reviewers[]`,
`updated_on` (rfc3339), and — because `--build` was passed — `build_state` (worst-wins rollup:
`failed` | `stopped` | `inprogress` | `successful` | `none`) plus `build[]` for the individual
checks.

`--role author` is two requests: one to find who you are, one paginated call across every
workspace. The reviewer half is one request to find who you are, one to list workspaces (skipped
when `--workspace` is given), one listing call per workspace, then one call per scanned
repository. Narrow with `--workspace <slug>` or `--repo-limit <n>` when the user asks about one
workspace.

## Phase 2 — enrich only the candidates

Phase 1 cannot see comments. Select candidates from phase 1 on structure alone:

- every non-draft row where `my_role` is `reviewer` or `both`
- every row I authored whose `build_state` is `failed` or `stopped`
- every row I authored whose `my_review_state` is `changes_requested`
- every row I authored past the nudge threshold below

Take at most 12 candidates, oldest `updated_on` first. For each:

```bash
bb pr view <id> -R <repo> --unresolved --json
```

Nothing else gets enriched. Do not fetch comments for every row phase 1 returned.

A thread is **waiting on my answer** when it is an unresolved inline thread whose most recent
comment is not mine. Use `parent` to group replies into threads and the comment `author` to decide
whose the last word was.

## Staleness

Ages are in **working days** — Saturday and Sunday do not count, so a Monday brief does not accuse
everyone of ignoring the user all weekend.

| Situation | Threshold | Who owes |
|---|---|---|
| I am a reviewer, `my_review_state` is `pending` | over 1 working day | me |
| My pull request, a reviewer set `changes_requested` | over 1 working day | me |
| My pull request, no reviewer has acted | over 2 working days | them — nudge |

## Ranking

One flat list, this ladder, ties broken oldest first:

1. I am a reviewer and a thread waits on my answer, or my review is `pending` past threshold — I am
   the bottleneck.
2. My pull request has `changes_requested`, or unresolved threads waiting on my answer — my move.
3. My pull request's `build_state` is `failed` or `stopped` — my move.
4. My pull request is approved with `build_state` `successful` — merge it.
5. My pull request is past the nudge threshold with no reviewer action — nudge a named reviewer.
6. Everything else — counted, never listed.

Drafts never appear in 1–5. They are not waiting on anybody; count them in the tail.

## Output

At most 10 lines, ranked, then one tail count. No preamble, no closing offer of help.

```
1. acme/api#42 — 2 unresolved threads from patrick, oldest 3d. You're blocking the review.
   → bb pr view 42 -R acme/api --unresolved --json
2. acme/web#17 — build FAILED, changes requested by dana, 1d.
   → bb pr diff 17 -R acme/web
3. acme/api#39 — approved by patrick, build green, 2d. Ready to merge.
   → open acme/api#39 in bitbucket to merge
+6 quieter (3 drafts, 3 waiting on others)
```

Every line names `repo#id`, the reason, the age, and one command to act on it. When nothing needs
attention, say so in one line and stop.
