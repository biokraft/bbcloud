---
name: bbc-report-bug
description: Files a bug report about the `bb` CLI itself against biokraft/bbcloud with the `gh` CLI — reproduces it, redacts private Bitbucket data, and gets the user's approval before it creates the issue. Use only when the user asks to report, file or open an issue about a `bb` bug. Do not use for a pull request, a daily brief, the user's own code, or another repository.
license: MIT
---

# Report a `bb` bug

This skill files a bug about **`bb` itself** — wrong output, JSON that contradicts itself, a
command that fails on an endpoint Atlassian retired — as a GitHub issue on `biokraft/bbcloud`.

The evidence lives in the user's private Bitbucket workspace, and the issue is public forever.
Every step below keeps those two facts apart.

## Operating contract

1. **Explicit invocation only.** File an issue only when the user asks: report this, file a bug, open
   an issue upstream. Never file one on your own initiative, and never as a silent step in a larger
   task. When a `bb` command fails mid-task, tell the user. The user decides if it becomes an issue.
2. **The target is always `biokraft/bbcloud`.** If the bug is in the user's code, or in a
   repository that `bb` acted on, say so and stop.
3. **Redact before you draft, not after.** A mistake here publishes a company's private repository
   names under the user's name.
4. **One issue per bug.** Search first. Comment on an existing issue instead of opening a second.
5. **Never create the issue without showing it first.** Filing is public, and a deleted issue stays
   in everyone's mail. Show the exact title and body, and wait for a yes. A comment on an existing
   issue needs the same yes.

## Step 1 — make sure it is a `bb` bug

A bug report makes a claim: "`bb` printed X, the correct answer is Y". If you cannot state Y, you
have a question, not a bug. The answer can be in `bb <command> --help` or the README.

These are not bugs. Each has its own fix:

| Looks like | Is |
|---|---|
| exit 2, "not authenticated" | a missing or expired token — `bb auth login` |
| exit 3 on a repository that exists | a wrong `-R` slug, or a token that cannot see the repository |
| `forbidden`, with a scope hint | a token without that scope — `bb auth login --help` lists them |
| stale output | Bitbucket's eventual consistency — run it again |

**Run the command again before you report it.** One wrong value is not a reproduction. An
intermittent result usually means the state changed, not that `bb` computed it wrong.

If `bb` printed `bb X.Y.Z is available …`, ask the user to upgrade and run the command again. The
bug may already be fixed.

## Step 2 — reproduce, and keep the evidence

Run the failing command with `--json`. Also run the command that contradicts it. Keep both outputs:
a report that says "the state was wrong" without them cannot be acted on.

```bash
bb --version
uname -sm
```

Both go in the issue as they are. A bug on one platform only is a different bug.

If two commands disagree, compare them field by field, not by eye. A claimed difference that turns
out to be two different pull requests wastes everyone's time:

```bash
evidence=$(mktemp -d)   # outside the working tree, so private output is never committed
bb pr list -R <workspace>/<repo> --json > "$evidence/a.json"
bb pr mine --json > "$evidence/b.json"
```

Then compare the fields in question for the same `id`. Delete `$evidence` when the issue is filed.

## Step 3 — redact

Everything from the user's Bitbucket workspace is private. Replace it before it reaches the draft:

- **Workspace, repository and project names, and every url that holds them** → `acme`,
  `acme/api`, `PROJ`. Keep the shape (`workspace/repo`), drop the name.
- **Names, display names, emails and account uuids** → `Reviewer A`, `{uuid-1}`. Keep different
  people different, so a report about two reviewers still reads correctly.
- **Pull request titles, branch names, commit messages and file paths** → describe them (`a PR
  title`), or drop them. They leak roadmaps.
- **Pull request and comment ids** → keep them. Without the workspace they mean nothing, and they
  make the report readable.
- **Tokens, `BB_TOKEN`, and the output of `bb auth status`** → never include, redacted or not. If
  a token appears in a capture, discard the capture. Do not edit it.

Redact the JSON too. Cut it to the fields the bug is about. A full `--json` dump leaks, and nobody
reads it.

Read the redacted evidence once more, and ask: can a stranger name the user's employer from this?
If yes, redact again.

## Step 4 — search for a duplicate

```bash
gh issue list --repo biokraft/bbcloud --state all --limit 30 --search "<two or three key words>"
```

Search for the symptom, not for your theory of the cause. The existing issue can come from someone
with a different theory. If an issue matches, add your evidence as a comment, after the same
approval as a new issue:

```bash
gh issue comment <number> --repo biokraft/bbcloud --body-file "$evidence/issue.md"
```

## Step 5 — draft the issue

Write the body to `"$evidence/issue.md"`, and pass it with `--body-file`. A long `--body` string
breaks code fences and puts the text in the shell history.

Use these four headings:

```markdown
## What happened

One or two sentences: what `bb` printed, and what it should print. Name both
commands if two disagree.

## Repro

1. Numbered steps, with the exact commands.
2. The redacted output at the step where it goes wrong.

## Guess at cause

Optional, and labelled as a guess. Name the code path or endpoint you suspect,
and why.

## Environment

bb <version>, <os> <arch>
```

The title says what is wrong, not that something is wrong: `reviewer state disagrees between
'pr mine' and 'pr list'`, not `bug in pr mine`.

## Step 6 — the approval gate

Show the title and the full body. Ask the user if you can file it against `biokraft/bbcloud`. Then
wait.

File only on a clear yes. If the user asks for a change, redraft, show the result, and ask again.
Never file a "close enough" version.

```bash
gh issue create --repo biokraft/bbcloud --title "<title>" --body-file "$evidence/issue.md"
```

Report the url that `gh` prints.

## When `gh` is missing or not logged in

```bash
gh auth status
```

If `gh` is not installed, or not logged in, stop. Tell the user which of the two it is, and the one
command that fixes it: install `gh` from <https://cli.github.com>, or run `gh auth login`. Never fall
back to `curl` on the GitHub API, and never ask for a token. Give the user the path to the finished
body file. To file it by hand is a good result.
