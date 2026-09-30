---
name: bbc-daily-brief
description: Produces a ranked, actionable brief of the user's Bitbucket Cloud pull requests across repositories with the `bb` CLI. Use ONLY when the user explicitly asks for a daily brief, a standup summary, or what needs their attention across repositories. Never invoke this skill proactively. Do not use for a single pull request, to open a pull request, or as a step in another task.
license: MIT
---

# Daily brief

One ranked list of what needs the user's attention across their Bitbucket repositories, built from
`bb`, and nothing else. The brief is read-only.

## Operating contract

1. **Explicit request only.** Produce a brief only when the user asks for one: a daily brief, a
   standup summary, what needs their attention. For a narrower question — "what is failing on PR
   42", "who reviews this branch" — answer with plain `bb` commands, and produce no brief.
2. The `bitbucket-cloud` rules apply: `--json` on every command, never `-w` or `--web`, and branch
   on exit codes, not on error text. On exit 2, ask the user to run `bb auth login`. Never run
   `bb auth logout`.
3. **Never write to Bitbucket.** Do not comment, approve, merge or request changes while you build a
   brief. **Never resolve a comment thread**, even when it looks answered. A brief reports threads.
   It does not close them.
4. Run the cheap scan first. Enrich only the ranked candidates.
5. Report an incomplete scan in one line under the verdict: a non-empty `partial`, a workspace with
   more repositories than the scan read, or a list that returned exactly `--limit` rows. Name the
   numbers: "reviews read in 30 of 140 repositories in acme".
6. **Write to the user as "you"**: "your review is pending", "you raised two threads", "Dana owes
   you a reply". The reader owns these pull requests, not the agent. The JSON fields `my_role` and
   `my_review_state` keep the API's wording. The brief does not.

## Phase 1 — the scan

```bash
bb pr mine --build --json
```

It returns `{pull_requests[], partial[]}`. Each row has `repo`, `id`, `title`, `url`, `state`,
`draft`, `author`, `my_role` (`author`, `reviewer` or `both`), `my_review_state` (`approved`,
`changes_requested`, `pending`, or `null` when the user is not a reviewer), `reviewers[]` (`name`,
`uuid`, `state`), `updated_on` (RFC 3339) and `comment_count`. `comment_count` counts every comment,
replies included, and is `null` when Bitbucket did not report it. `--build` adds `build_state`
(worst-wins: `failed`, `stopped`, `inprogress`, `successful`, `none`) and `build[]`.

The workspaces come from `--workspace <slug>[,<slug>…]`, then `BB_WORKSPACE`, then the remote of the
current checkout. No API lists the workspaces of a user, so with none of these the command fails.
Ask the user for the workspace slugs then.

**The scan is a recency window.** The reviewer half reads only the `--repo-limit` most recently
updated repositories per workspace (default 30). Never present the brief as a complete picture of
a workspace. `--role author` skips the reviewer half, and costs one request per workspace.

To state the coverage, count each workspace once:

```bash
bb repo list --workspace <slug> --limit 10000 --json
```

It returns every repository the token can see, at one request per 100 repositories. The array
length is the denominator for rule 5.

## One repository only

When the user limits the request to one repository — "only this repo", "just acme/api" — do not
use `bb pr mine`. Use two exact lists:

```bash
bb pr list -R <workspace>/<repo> --needs-my-review --build --limit 1000 --json   # you review, and have not approved
bb pr list -R <workspace>/<repo> --author @me --build --limit 1000 --json        # you opened it
```

`pr mine` skips a repository outside its recency window. These lists never do, but each returns at
most `--limit` rows (default 100), and says nothing when it cuts. Always pass `--limit`. If a list
returns exactly that many rows, it is incomplete: report it under rule 5. A row from the first list
has the role `reviewer`, and a row from the second the role `author`. These rows have no `my_review_state`, `updated_on` or `comment_count`. Take every row as a
candidate, and read those facts in phase 2. The user's entry in `pull_request.reviewers[]` is the
one whose `name` equals `account` from `bb auth status --json`.

Ranking, thresholds and output are the same in both modes.

## Phase 2 — enrich the candidates

Phase 1 counts comments but cannot read them. Choose the candidates from phase 1 alone:

- every non-draft row where `my_role` is `reviewer` or `both`
- every row the user authored whose `build_state` is `failed` or `stopped`
- every row the user authored where an entry in `reviewers[]` has the state `changes_requested`
- every non-draft row the user authored whose `comment_count` is above `0`, or `null`
- every non-draft row the user authored whose `build_state` is `successful` and whose reviewers all
  approved
- every row the user authored that is past the nudge threshold below

The `comment_count` rule catches a reviewer who commented but gave no verdict. That reviewer's
state stays `pending`, so no other rule fires. A count above zero does not prove that something
waits on the user — the user's own comments count too. It only means that phase 2 must look. Treat
`null` the same way.

Take at most 12 candidates. Put the rows where `my_role` is `reviewer` or `both` first, then the
authored rows. In each group, put the oldest `updated_on` first. Without this order, the user's own
pull requests fill the slots, and a review the user holds up is never enriched.

For each candidate:

```bash
bb pr view <id> -R <repo> --unresolved --conflicts --build --json
```

Enrich nothing else. Do not fetch comments for every row from phase 1.

A thread **waits on the user** when it is an unresolved inline thread and its most recent comment
is by somebody else. Group comments into threads: follow `parent` until it is `null`. Compare each
comment's `author` with `account` from `bb auth status --json`.

## Staleness

Ages count **working days**. Saturday and Sunday do not count, so a Monday brief does not blame
anyone for the weekend.

| Situation | Threshold | Who owes |
|---|---|---|
| The user reviews, and the review is `pending` | over 1 working day | the user |
| The user's pull request has a reviewer at `changes_requested` | over 1 working day | the user |
| The user's pull request, and no reviewer has acted | over 2 working days | the reviewers — nudge |

## Ranking

Use this ladder. Break ties oldest first.

1. The user reviews, and a thread waits on the user's answer, or the review is `pending` past the
   threshold. The user is the bottleneck.
2. The user's pull request has `changes_requested`, or unresolved threads that wait on the user.
3. The user's pull request has a `build_state` of `failed` or `stopped`.
4. Merge candidate: the user's pull request has at least one reviewer, **every** reviewer approved,
   and phase 2 shows `build.build_state` `successful`, `conflicts.count` `0`, `unresolved_threads`
   `0` and `pull_request.task_count` `0`. A pull request with no reviewers is not a candidate: an
   empty list passes "every reviewer approved" by vacuous truth.
5. The user's pull request is past the nudge threshold, and no reviewer has acted. Name a reviewer
   to nudge.
6. Everything else: count it, never list it.

Drafts never appear in 1–5. Nobody waits on a draft, so count it in the tail.

A merge candidate is a list of facts, not permission to merge. The user decides. Use the build
state from phase 2, never from phase 1: a build that failed after phase 1 must not show as green.

## Output

A one-line verdict, then the groups, then a count. List at most 10 pull requests. No preamble, and
no closing offer of help.

```
2 need you · 1 waiting on others · 1 merge candidate · 1 quiet

🔴 YOU'RE BLOCKING
  [acme/api PR 225](https://bitbucket.org/acme/api/pull-requests/225)  Validate mapi responses
    Your review is pending · 2d old
    → bb pr view 225 -R acme/api --unresolved --json

  [acme/web PR 206](https://bitbucket.org/acme/web/pull-requests/206)  Add guardrail hooks
    💥 Build failed, changes requested by Dana · 5d old — oldest here
    → bb pr build 206 -R acme/web --json

⏳ WAITING ON OTHERS
  [acme/api PR 221](https://bitbucket.org/acme/api/pull-requests/221)  No review from Dana yet · 3d

✅ MERGE CANDIDATE
  [acme/api PR 198](https://bitbucket.org/acme/api/pull-requests/198)  Approved by Dana, build green · 2d

💤 1 quiet (1 draft)
```

### Links

Each entry's identifier is a markdown link to that row's own `url` field. Never build a url
yourself, and never link to the repository path. `bb pr list` rows carry the same `url`.

**Never write a repository path, a hash and a number with no space between them.** That is GitHub's
issue-reference syntax. Chat clients and terminals turn it into a link to `github.com`, so the
reader lands on a GitHub 404. Write `[acme/api PR 225](<url>)`: the word `PR`, the id, and the
Bitbucket url as the target.

### Emoji

Use exactly these five glyphs, and no others. Each one is a visual anchor, and extra decoration
makes the brief harder to scan.

| Glyph | Means | Where |
|---|---|---|
| 🔴 | this is on you | the `YOU'RE BLOCKING` heading |
| ⏳ | this waits on someone else | the `WAITING ON OTHERS` heading |
| ✅ | no known blocker; the merge is still the user's decision | the `MERGE CANDIDATE` heading |
| 💥 | a build failed or stopped | on the entry line, before the reason |
| 💤 | nothing needed here | the quiet tail |

🔴, ⏳, ✅ and 💤 appear at most once each. 💥 can repeat, but only on entries whose `build_state`
is `failed` or `stopped`.

### Shape rules

- The verdict line is always present, and has no emoji — also when it reads `nothing needs you`. It
  counts each group that appears below it.
- A group heading appears only when the group has entries.
- `🔴 YOU'RE BLOCKING` holds rungs 1–3. Each entry has one command line that starts with `→`.
- `⏳ WAITING ON OTHERS` holds rung 5. Name who owes the reply and for how long. Add a command only
  when it is useful.
- `✅ MERGE CANDIDATE` holds rung 4. Give no command: `bb` cannot merge. Say that it is approved and
  green.
- The quiet tail is a count with a breakdown in parentheses, never a list.
- Ages are short: `4h`, `3d`. Mark the oldest entry in a group with `— oldest here`.
