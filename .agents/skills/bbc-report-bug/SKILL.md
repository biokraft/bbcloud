---
name: bbc-report-bug
description: File a bug report about the `bb` CLI itself against biokraft/bbcloud with the `gh` CLI — reproduce it, redact the user's private Bitbucket data, and get the user's approval before the issue is created. Use this skill when the user asks to report, file or open an issue about a `bb` bug. Do not use it to file issues in other repositories.
license: MIT
---

# Report a `bb` bug

`bb` is the Bitbucket Cloud CLI you have been running. This skill files a bug about **`bb`
itself** — wrong output, contradictory JSON, a command that fails against an endpoint
Atlassian retired — as a GitHub issue on `biokraft/bbcloud`.

The hard part is not writing the issue. It is that the evidence lives in the user's private
Bitbucket workspace and the issue is public forever. Every step below exists to keep those
two facts apart.

## Rules

1. **Explicit invocation only.** File an issue when the user asks for one — report this, file
   a bug, open an issue upstream. Never file one on your own initiative because a command
   looked wrong to you, and never as a silent step inside a larger task. A `bb` command that
   fails mid-task is something you tell the user about; whether it becomes an issue is their
   call.
2. **The target is always `biokraft/bbcloud`.** This skill does not file issues anywhere else.
   If the bug is in the user's own code, or in a repository `bb` was merely pointed at, say so
   and stop.
3. **Never create the issue without showing it first.** Filing is public and cannot be undone
   — a deleted issue still sits in anyone's mail. Print the exact title and body, then wait
   for the user to say yes. This is the same gate `bb pr resolve` and `bb pr request-changes`
   apply, for the same reason.
4. **Redact before you draft, not after.** See [Redaction](#redaction). Getting this wrong
   publishes a company's private repository names under the user's name.
5. **One issue per bug.** Search first; comment on the existing issue instead of opening a
   second.

## Step 1 — establish that it is a `bb` bug

Ask yourself what `bb` did that it should not have. A bug report needs a claim of the form
"`bb` printed X, the correct answer is Y". If you cannot state Y, you do not have a bug yet —
you have a question, and the answer may be in `bb --help` or the README.

Rule these out first, because each has a different fix and none of them is an issue:

| Looks like | Actually |
|---|---|
| exit code 2, "not authenticated" | the token is missing or expired — `bb auth login` |
| exit code 3 on a repository that exists | wrong `-R` slug, or the token cannot see it |
| a 403 mentioning a scope | the token lacks that scope — mint a new one |
| output is stale | Bitbucket's own eventual consistency; re-run before reporting |

**Re-run the command before you report it.** A single observation of a wrong value is not a
reproduction, and an intermittent result usually means the state changed underneath you rather
than that `bb` computed it wrong.

## Step 2 — reproduce, and record the evidence

Run the failing command with `--json`, and run whatever second command contradicts it. Capture
both outputs. A report that says "the state was wrong" without the two outputs side by side
cannot be acted on.

```bash
bb --version
uname -sm
```

Both go in the issue verbatim. A bug that only happens on one platform is a different bug.

If the report is that two commands disagree, diff them mechanically rather than by eye — a
claimed disagreement that turns out to be two different pull requests wastes everyone's time:

```bash
bb pr list -R <repo> --json > /tmp/a.json
bb pr mine --json > /tmp/b.json
```

Then compare the specific fields for the same id.

## Step 3 — redaction

Everything from the user's Bitbucket workspace is private. Replace it before it reaches the
draft:

- **Workspace, repository and project names** → `acme`, `acme/api`, `PROJ`. Keep the shape
  (`workspace/repo`), lose the name.
- **Human names, display names, emails, account uuids** → `Reviewer A`, `{uuid-1}`. Keep
  distinct people distinct, so a report about two reviewers still reads correctly.
- **Pull request titles, branch names, commit messages** → describe them (`a PR title`), or
  drop them. They leak roadmaps.
- **Pull request ids and numbers** → keep them. They are meaningless without the workspace and
  they make the report readable.
- **Tokens, `BB_TOKEN`, anything from `bb auth status`** → never include, redacted or not. If a
  token appears anywhere in captured output, the capture is discarded, not edited.

Redact the JSON too. Trim it to the fields the bug is about — a full `--json` dump is both a
leak and unreadable.

Before drafting, re-read your redacted evidence once and ask whether a stranger could name the
user's employer from it. If yes, redact again.

## Step 4 — search for a duplicate

```bash
gh issue list --repo biokraft/bbcloud --state all --limit 30 --search "<two or three key words>"
```

Search the symptom, not your theory about the cause — the existing issue was filed by someone
with a different theory. If a match exists, add your evidence as a comment instead:

```bash
gh issue comment <number> --repo biokraft/bbcloud --body-file <path>
```

The same approval gate applies to a comment.

## Step 5 — draft the issue

Write it to a file and pass `--body-file`; `--body` with a long string mangles code fences and
puts the content in shell history.

Use this shape. It is the shape maintainers of this repository already use, and the four
headings are what makes a report actionable:

```markdown
## What happened

One or two sentences. What `bb` printed, and what it should have printed. Name both commands
if two disagree.

## Repro

1. Numbered steps someone else can follow, with the exact commands.
2. Redacted output at the step where it goes wrong.

## Guess at cause

Optional, and label it a guess. Say which code path or endpoint you suspect and why. A wrong
guess in a well-evidenced report costs nothing; a guess presented as a finding costs trust.

## Environment

bb <version>, <os> <arch>
```

Title: what is wrong, not that something is wrong. `reviewer state disagrees between 'pr mine'
and 'pr list'` beats `bug in pr mine`.

## Step 6 — the approval gate

Print the title and the full body. Ask the user, plainly, whether to file it against
`biokraft/bbcloud`. Then wait.

Do not file if the answer is anything other than yes. Do not file a "close enough" version of
a body the user asked you to change — redraft, show it again, ask again.

On approval:

```bash
gh issue create --repo biokraft/bbcloud --title "<title>" --body-file <path>
```

Report the URL `gh` prints back.

## When `gh` is missing or unauthenticated

```bash
gh auth status
```

If `gh` is not installed or not logged in, stop and tell the user which of the two it is and
the one command that fixes it (`brew install gh`, or `gh auth login`). Do not fall back to
`curl` against the GitHub API, and do not ask the user to paste a token — you have a finished
issue body on disk, and the user opening the browser themselves is a fine outcome. Give them
the path to it.
