---
name: bitbucket-cloud
description: Read and answer Bitbucket Cloud pull request reviews with the `bb` CLI. Use this skill when the repository is hosted on Bitbucket Cloud, or when the task is to list, read, review, comment on, or open a pull request there. Do not use it for GitHub or GitLab.
license: MIT
---

# Bitbucket Cloud with `bb`

`bb` is a single binary that speaks the Bitbucket Cloud REST API. Use it for all pull request work.
Do not use `gh`. Do not ask the user to open the web UI.

## Rules

1. Add `--json` to every command, and parse the JSON. The tables are for humans, and their layout
   can change. One exception: read `bb pr diff <id>` as plain text.
2. Do not pass `-w` or `--web`. These flags start a browser.
3. Use the exit code, not the error text: `0` success, `1` error, `2` not authenticated,
   `3` not found.
4. Give a body to every comment. Use `--body` for one line. Use `--body-stdin` for more than one
   paragraph. Without a body and without a terminal, the command fails.
5. Add `-R workspace/repo` to act on another repository. The default comes from the git remote.

## Read a pull request

```bash
bb pr list --json                       # open pull requests
bb pr list main --state MERGED --json   # filter by target branch and state
bb pr view 42 --json                    # the pull request, plus all comments
bb pr view 42 --unresolved --json       # only the threads that still need an answer
bb pr diff 42                           # raw diff, plain text
bb pr files 42 --json                   # changed paths
bb pr commits 42 --json                 # commits, short hashes
```

`bb pr view` returns `{ pull_request, general[], inline[] }`. Each comment has `id`, `author`,
`timestamp`, `body`, `file`, `line` and `resolved`. Use the comment `id` to answer in the correct
thread.

Find the pull request for the current branch:

```bash
bb pr list --json | jq --arg b "$(git branch --show-current)" '.[] | select(.source == $b)'
```

## Answer a review

```bash
# answer inside the thread you address
bb pr comment 42 --reply-to 998877 --body "Fixed in 1a2b3c4." --json

# raise a new point on one line
bb pr comment 42 -f src/auth.rs -l 88 --body "This drops the error." --json

# more than one paragraph
printf 'Refactored as suggested.\n\nThe parser is now its own module.\n' \
  | bb pr comment 42 --body-stdin --json
```

`--line` needs `--file`. `--reply-to` accepts neither, because a reply inherits the location of its
parent.

Ask the author to change the code, or withdraw that request:

```bash
bb pr request-changes 42 --json
bb pr no-request-changes 42 --json
```

## Open a pull request

```bash
bb pr create main --title "Cache session lookups" --json
bb pr create main feat/cache --title "..." --description "..." --close-source-branch --json
bb pr create main,develop --title "..." --json      # one pull request per target
```

The source branch defaults to the current checkout. The title defaults to
`Merge <source> into <target>`. `bb` attaches the default reviewers of the repository, and removes
you from that list. Pass `--no-default-reviewers` to attach none. Do not pass `-i`, because it
prompts.

## Branches

```bash
bb branch list --json                       # newest commit first
bb branch list -u alice -n feat/ --json     # filter by author and by name
bb branch list --limit 20 --json
```

Both filters match a substring, and ignore case.

## Command map

| Command | Result |
|---|---|
| `bb pr list [target] [--state OPEN\|MERGED\|DECLINED\|SUPERSEDED]` | `[{id,title,author,source,destination,reviewers[],approvals[],url}]` |
| `bb pr view <id> [--unresolved] [--comments-only]` | `{pull_request,general[],inline[]}` |
| `bb pr diff <id>` | plain diff; `--json` wraps it as `{id,diff}` |
| `bb pr files <id>` | `[{status,path}]` |
| `bb pr commits <id>` | `[{hash,summary}]` |
| `bb pr comment <id> …` | `{id,pull_request,url}` |
| `bb pr create <target> [source] …` | `[{id,target,url}]` |
| `bb pr request-changes <id>` | `{requested_changes:<id>}` |
| `bb pr no-request-changes <id>` | `{unrequested_changes:<id>}` |
| `bb branch list …` | `[{branch,user,updated}]` |
| `bb auth status` | `{email,token,account}`, token redacted |
| `bb browse --print [--pr <id>\|--branches]` | `{url}` |

`timestamp` and `updated` hold a relative time, for example `3 days ago`. For an exact time, read
the commit or the diff.

## When a command fails

- **Exit 2** — no credentials. Ask the user to run `bb auth login`. Do not run it yourself, because
  it prompts for a token. In CI, set `BB_EMAIL` and `BB_TOKEN`.
- **Exit 3** — the pull request, the branch or the repository does not exist. Confirm the id, and
  confirm the repository with `bb auth status` and `-R`.
- **A 403 message** — the API token misses a scope. `pr list` and `pr view` need
  `read:pullrequest:bitbucket`. `pr comment`, `pr create` and `pr request-changes` need
  `write:pullrequest:bitbucket`. `branch list` and `pr create` also need
  `read:repository:bitbucket`.
- **`no bitbucket.org remote found`**, or **`no git repository here`** — `bb` cannot find the
  repository. Pass `-R workspace/repo`, or set `BB_REPO`.

Bitbucket Cloud removed app passwords on 2026-07-28. `bb` authenticates with an Atlassian account
email and an API token. Never suggest an app password.

## Environment

| Variable | Purpose |
|---|---|
| `BB_EMAIL`, `BB_TOKEN` | credentials for CI and other non-interactive use |
| `BB_REPO` | default repository, the same as `-R` |
| `NO_COLOR` | disable colour and spinners |

Install: `brew install biokraft/tap/bb`, or `cargo install bbcloud --locked`. Run `bb --help` and
`bb <command> --help` for the full surface. Source and issues:
<https://github.com/biokraft/bbcloud>.
