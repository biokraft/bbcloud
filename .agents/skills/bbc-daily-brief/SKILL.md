---
name: bbc-daily-brief
description: Produce a ranked, actionable daily brief of the user's Bitbucket Cloud pull requests across every repository. Use ONLY when the user explicitly asks for a daily brief, a standup summary, or "what needs my attention" across repositories. Never invoke this skill proactively, and never as a step inside another task.
license: MIT
---

# Daily brief

One ranked list of what needs the user's attention across every Bitbucket repository, built from
`bb`. Nothing else.

## Rules

1. **Explicit invocation only.** Produce a brief when the user asks for one in those words — a
   daily brief, a standup summary, what needs their attention. If the question is narrower ("what
   is failing on PR 42", "who is reviewing this branch"), answer it with plain `bb` commands and do
   not produce a brief.
2. The `bitbucket-cloud` skill's rules bind here too: `--json` on every command, never `-w` or
   `--web`, use exit codes rather than error text.
3. **Never resolve a comment thread.** A brief reports threads; it does not close them. This holds
   even when the thread looks answered.
4. Never write a comment, approve, or merge while building a brief. It is read-only.
5. Report an incomplete scan. If phase 1 returns a non-empty `partial`, the brief opens with one
   line naming those workspaces.
6. **Write to the user as "you".** The brief says "your review is pending", "you raised two
   threads", "Dana owes you a reply". Never write the brief in the first person — the reader is the
   person whose pull requests these are, not the agent. The json fields are still named `my_role`
   and `my_review_state`; that is the api's wording, not the brief's.

## Phase 1 — structural scan, cheap

```bash
bb pr mine --build --json
```

Returns `{ "pull_requests": [...], "partial": [...] }`. Each row carries `repo`, `id`, `title`,
`url`, `state`, `draft`, `author`, `my_role` (`author` | `reviewer` | `both`), `my_review_state`
(`approved` | `changes_requested` | `pending`, or `null` when the user is not a reviewer),
`reviewers[]`, `updated_on` (rfc3339), `comment_count` (every comment on the pull request, inline
and general, replies included — `null` when bitbucket did not return the field), and — because
`--build` was passed — `build_state` (worst-wins rollup: `failed` | `stopped` | `inprogress` |
`successful` | `none`) plus `build[]` for the individual checks.

There is no api call left that discovers which workspaces the user belongs to. The workspace(s)
scanned are resolved from `--workspace <slug>[,<slug>...]`, then `BB_WORKSPACE`, then the git remote
of the current checkout — see the `bitbucket-cloud` skill for the full precedence order.

`--role author` is one request to find who the user is, then one paginated call per workspace. The
reviewer half adds one repository-listing call per workspace and then one call per scanned
repository. Narrow with `--workspace <slug>` or `--repo-limit <n>` when the user asks about one
workspace.

**This scan is a recency window, not the whole workspace.** The reviewer half only ever looks at
the `--repo-limit` most recently updated repositories per workspace (default 30) — a workspace with
hundreds of repositories is covered only in a small slice. Never present a brief built this way as
a complete picture of the workspace; if certainty about one specific repository matters, use
`bb pr list -R <repo>` for that repository instead.

## One repository only

When the user scopes the request to a single repository — "only this repo", "just for
acme/api" — do not use `bb pr mine`. Use:

```bash
bb pr list -R <workspace>/<repo> --build --json
```

This is cheaper (one call plus the build fetches, rather than a 30-repository scan) and *more*
complete: `pr mine`'s reviewer half only covers the most recently updated repositories, while
`pr list` sees every pull request in the named repository.

The rows differ: `pr list` carries `reviewers[]` but no `my_role` or `my_review_state`. Work out
whether something waits on the user by finding their own uuid in `reviewers[]` and reading its
`state`, or let the CLI do it — `bb pr list --needs-my-review --json` returns exactly the pull
requests where the user is a reviewer and has not approved yet.

Ranking, thresholds and output format are identical in both modes.

## Phase 2 — enrich only the candidates

Phase 1 cannot read comment threads, only count them. Select candidates from phase 1 on structure
alone:

- every non-draft row where `my_role` is `reviewer` or `both`
- every row the user authored whose `build_state` is `failed` or `stopped`
- every row the user authored whose `my_review_state` is `changes_requested`
- every non-draft row the user authored whose `comment_count` is above `0` or `null`
- every row the user authored past the nudge threshold below

The `comment_count` rule is the one that catches a reviewer who commented without acting on the
approval: `my_review_state` stays `pending` in that case, so none of the other rules fire and the
pull request would drop out of the brief entirely. A count above zero does not mean something waits
on the user — the user's own comments are counted too, and threads may all be resolved — it only
means phase 2 has to look. `null` means bitbucket did not report a count, so it is treated the same
way: look rather than assume quiet.

Take at most 12 candidates. Order them by how likely they are to be blocking somebody: rows where
`my_role` is `reviewer` or `both` first, then the authored rows, each group oldest `updated_on`
first. The `comment_count` rule widens this set considerably — most authored pull requests have
some comment on them — so without that ordering a busy morning's twelve slots fill with the user's
own pull requests and a review they are holding up never gets enriched.

For each:

```bash
bb pr view <id> -R <repo> --unresolved --json
```

Nothing else gets enriched. Do not fetch comments for every row phase 1 returned.

A thread is **waiting on the user's answer** when it is an unresolved inline thread whose most
recent comment is somebody else's. Use `parent` to group replies into threads and the comment
`author` to decide whose the last word was.

## Staleness

Ages are in **working days** — Saturday and Sunday do not count, so a Monday brief does not accuse
everyone of ignoring the user all weekend.

| Situation | Threshold | Who owes |
|---|---|---|
| The user is a reviewer, `my_review_state` is `pending` | over 1 working day | the user |
| The user's pull request, a reviewer set `changes_requested` | over 1 working day | the user |
| The user's pull request, no reviewer has acted | over 2 working days | them — nudge |

## Ranking

This ladder, ties broken oldest first:

1. The user is a reviewer and a thread waits on their answer, or their review is `pending` past
   threshold — they are the bottleneck.
2. Their pull request has `changes_requested`, or unresolved threads waiting on their answer.
3. Their pull request's `build_state` is `failed` or `stopped`.
4. Their pull request is approved with `build_state` `successful` — ready to merge.
5. Their pull request is past the nudge threshold with no reviewer action — nudge a named reviewer.
6. Everything else — counted, never listed.

Drafts never appear in 1–5. They are not waiting on anybody; count them in the tail.

## Output

A one-line verdict, then labelled groups, then a count. At most 10 pull requests listed. No
preamble, no closing offer of help.

```
2 need you · 1 waiting on others · 1 quiet

🔴 YOU'RE BLOCKING
  [acme/api PR 225](https://bitbucket.org/acme/api/pull-requests/225)  Validate mapi responses
    Your review is pending · 4h old
    → bb pr view 225 -R acme/api --unresolved --json

  [acme/web PR 206](https://bitbucket.org/acme/web/pull-requests/206)  Add guardrail hooks
    💥 Build failed, changes requested by Dana · 5d old — oldest here
    → bb pr diff 206 -R acme/web

⏳ WAITING ON OTHERS
  [acme/api PR 221](https://bitbucket.org/acme/api/pull-requests/221)  Dana hasn't replied to your 2 threads · 3d

✅ READY TO MERGE
  [acme/api PR 198](https://bitbucket.org/acme/api/pull-requests/198)  Approved by Dana, build green · 2d

💤 1 quiet (1 draft)
```

### Linking, and one shape to never write

Every entry's identifier is a markdown link whose target is that row's own `url` field — never a
url you assembled yourself, and never the plain repository path.

**Never write a repository path followed by a hash and the number.** That shape is GitHub's
issue-reference syntax: chat clients and terminals silently rewrite it into a link to `github.com`,
so a brief about Bitbucket work sends the reader to a GitHub 404. Write
`[acme/api PR 225](<url>)` instead — the words `PR` and the id, with the real Bitbucket url as the
link target.

In the repo-scoped mode `bb pr list` supplies the same `url` field per row, so the rule is identical
there.

### The emoji vocabulary

Exactly five glyphs, each earning its place as a visual anchor. **Use no others** — a brief peppered
with decoration is harder to scan than one with none, which defeats the point.

| Glyph | Means | Where |
|---|---|---|
| 🔴 | this is on you | the `YOU'RE BLOCKING` heading |
| ⏳ | waiting on someone else | the `WAITING ON OTHERS` heading |
| ✅ | nothing left to do but merge | the `READY TO MERGE` heading |
| 💥 | a build is failing or stopped | on the entry line, before the reason |
| 💤 | nothing needed here | the quiet tail |

🔴, ⏳, ✅ and 💤 appear once each at most, on their own heading or tail line. 💥 is the only one that
repeats, and only on entries whose `build_state` is `failed` or `stopped`.

### Rules for that shape

- The verdict line is always present and carries no emoji, even when it reads `nothing needs you`.
- A group heading appears only when it has entries.
- `🔴 YOU'RE BLOCKING` holds ranking rungs 1–3, and every entry carries one command line prefixed `→`.
- `⏳ WAITING ON OTHERS` holds rung 5. It needs no command: name who owes the reply and how long it
  has been. Add a command only when there is something useful to run.
- `✅ READY TO MERGE` holds rung 4. Merging is the user's decision and `bb` cannot do it, so give no
  command — say it is approved and green.
- The quiet tail is a count with a parenthesised breakdown, never a list.
- Ages are short: `4h`, `3d`. Mark the oldest entry in a group with `— oldest here`.
- Address the user directly throughout, per rule 6.
