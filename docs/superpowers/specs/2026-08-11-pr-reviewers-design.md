# PR reviewers, review state and list filters — design

**Status:** approved 2026-08-11

## Goal

Make reviewer information first-class in `bb`: show who is tagged on a pull request and what each
reviewer has decided, let a reviewer be added or removed, show which pull requests are waiting on the
authenticated user, show whether a pull request is a draft, and expose all of it as filters on
`bb pr list`.

Approving, merging, declining and resolving comment threads stay out of scope and remain unsupported.

## Motivation and the defect this fixes

`bb pr list` already renders `REVIEWERS` and `APPROVED` columns (`src/commands/pr.rs:52-62`), but they
are always empty. Verified against the live API on 2026-08-11: `bb pr list --json` for
`check24/solutions_console` PR 682 returned `"reviewers": []` and `"approvals": []` while the pull
request has reviewers in the web UI.

Root cause: Bitbucket's paginated `/pullrequests` endpoint returns a reduced pull-request object that
omits `reviewers`, `participants` and `draft`. Those fields are only present on the single-pull-request
`GET /pullrequests/{id}`, or when explicitly requested with the partial-response `fields` parameter.

So the columns are not a missing feature — they are a silently broken one.

## Data layer

`src/api/models.rs`:

- `PullRequest` gains `#[serde(default)] draft: bool`.
- `Participant` gains `role: Option<String>` and `#[serde(default)] approved: bool` alongside the
  existing `state`.
- `User` gains `account_id: Option<String>`.
- New `ReviewState` enum: `Approved`, `ChangesRequested`, `Pending`. Derived from a participant's
  `state` string (`"approved"`, `"changes_requested"`, anything else or absent → `Pending`).
- `PullRequest::reviewer_states() -> Vec<ReviewerState>` where
  `ReviewerState { name: String, uuid: Option<String>, state: ReviewState }`.
- `PullRequest::display_state() -> &'static str`.

### Why participants, not reviewers

`reviewers[]` is the tagged set and carries no decision. `participants[]` carries the decision but
includes people who merely commented. `reviewer_states()` therefore takes participants whose
`role == "REVIEWER"` as the primary source and unions in any entry from `reviewers[]` not already
present, as `Pending`. That way a reviewer who has not acted still appears, and a commenter who was
never tagged does not.

Matching between the two lists is by `uuid` when both sides have one, falling back to `display_name`.

### Draft

Bitbucket keeps `draft` as a boolean while `state` stays `OPEN`. `display_state()` returns `"Draft"`
when `draft` is true, otherwise the `state` string title-cased: `Open`, `Merged`, `Declined`,
`Superseded`, and `"-"` when `state` is absent. JSON output carries `state` and `draft` as separate
fields so no information is lost to the human rendering.

## Requesting the fields

`bb pr list` appends a partial-response parameter to its existing query:

```
fields=%2Bvalues.reviewers,%2Bvalues.participants,%2Bvalues.draft
```

`%2B` is a url-encoded `+`, which is Bitbucket's "add to the default set" prefix. A bare `+` in a query
string decodes as a space and the parameter is then ignored, so the encoding is load-bearing and a test
asserts the exact query string reaches the server.

## Name resolution — `src/users.rs` (new)

`pub async fn resolve_user(ctx: &Ctx, query: &str) -> Result<User>`

1. A query containing `@` or starting with `{` is treated as an email or UUID and used verbatim, with
   no lookup and no API call.
2. Otherwise the query is matched case-insensitively as a substring against each candidate's
   `nickname` and `display_name`.

Candidate pool, deduplicated by `uuid`:

- `GET /workspaces/{workspace}/members` (paginated)
- `GET /repositories/{workspace}/{repo}/default-reviewers` (paginated)
- for `pr reviewers add`/`remove`, the target pull request's own `reviewers[]`

A `403` on the members call is not fatal — the token may lack workspace scope. The pool degrades to
the remaining sources, which is enough to remove someone already tagged. Any other error propagates.

Outcomes:

- exactly one match → that user
- zero matches → `BbError::Config("no user matching `<query>` — pass an email or uuid to be exact")`
- more than one match → `BbError::Config` listing every candidate's display name, so the user can
  retry with something unambiguous

An exact case-insensitive equality match on `nickname` or `display_name` wins outright even when other
candidates match as substrings. Without that rule, a workspace containing both `ana` and `anastasia`
makes `ana` permanently unaddressable.

`resolve_user` never resolves to more than one user, and `add`/`remove` resolve every name they were
given before issuing any write, so a typo in the second name cannot leave a half-applied change.

## `bb pr reviewers` — `src/commands/pr_reviewers.rs` (new)

```
bb pr reviewers <id>                    # list reviewers and their state
bb pr reviewers add <id> <name>[,...]   # tag one or more reviewers
bb pr reviewers remove <id> <name>[,...]
```

`list` renders a `NAME | STATE` table from `reviewer_states()`; `--json` emits
`[{"name","uuid","state"}]` with `state` as `approved`, `changes_requested` or `pending`.

Bitbucket has no add-reviewer or remove-reviewer endpoint, so a mutation is:

1. `GET /pullrequests/{id}` for the current `title` and `reviewers`
2. compute the new reviewer set
3. `PUT /pullrequests/{id}` with `{"title": <existing title>, "reviewers": [{"uuid": …}, …]}`

`title` is included because the API rejects a `PUT` without it. Every other field is omitted and left
untouched — `PUT` on this endpoint is a partial update.

Behaviour:

- `add` is idempotent: a name already tagged is reported as already present, exit 0, no request sent
  if nothing would change.
- `remove` on a name that is not tagged is an error (`BbError::Config`), not a silent no-op — silence
  would let "remove Raigon" appear to succeed when it matched nobody.
- Removing the last reviewer is allowed; the pull request simply has none.
- Each command prints the resulting reviewer list so the outcome is visible in one step.

## `bb pr list` filters — `src/commands/pr_list.rs` (new)

| flag | meaning |
|---|---|
| `--state <s>` | as today, plus `draft`: `--state draft` requests `OPEN` from the API and keeps only `draft == true` |
| `--reviewer <name>` | pull requests where that person is tagged, resolved through `resolve_user` |
| `--author <name>` | pull requests opened by that person; `@me` means the authenticated account |
| `--review-state <approved\|changes-requested\|pending>` | the authenticated user's own state on the pull request |
| `--needs-my-review` | the authenticated user is a reviewer and their state is not `Approved` |

`--state` is the only server-side filter; the rest apply client-side to the fetched set, matching how
the existing `destination` positional argument already works. `--needs-my-review` and
`--review-state pending` overlap by design — the first is the ergonomic name, the second composes with
the other states.

`--needs-my-review`, `--review-state` and `--author @me` each need the current account, fetched once
per invocation with `GET /user` and reused. `--reviewer` and a named `--author` each cost one
resolution. When no flag needs it, no extra call is made.

Filters combine as AND. All of them, including `--state draft`, apply before the table is rendered, so
an empty result prints just the header.

### Columns

```
ID | TITLE | STATE | SOURCE | → | TARGET | AUTHOR | REVIEWERS
```

`REVIEWERS` is one column carrying both identity and decision, `name` followed by a mark:

- `✓` approved
- `✗` changes requested
- `·` no state yet

e.g. `Patrick ✓, Raigon ✗, Ana ·`. The `APPROVED` column is removed; it cannot express
changes-requested, and a second column of names doubles the table width for less information.

`--json` emits `reviewers` as `[{"name","uuid","state"}]` rather than the current `Vec<String>`, and
drops the `approvals` array. This is a breaking change to the JSON shape of `bb pr list`, taken
deliberately: the fields it changes have only ever emitted empty arrays, so nothing that works today
can depend on their contents.

## File layout

- `src/api/models.rs` — modify: `draft`, `Participant` fields, `User.account_id`, `ReviewState`,
  `ReviewerState`, `reviewer_states()`, `display_state()`
- `src/users.rs` — new: `resolve_user`, candidate collection, ambiguity errors
- `src/commands/pr_list.rs` — new: `list`, its filter struct, row building and rendering
- `src/commands/pr_reviewers.rs` — new: `list`, `add`, `remove`
- `src/commands/pr.rs` — modify: `list` moves out; keeps `Ctx`, `create`, `diff`, `files`, `commits`
- `src/commands/mod.rs`, `src/lib.rs` — modify: register the new modules
- `src/main.rs` — modify: the new flags and the `reviewers` subcommand
- `README.md` — modify: document the new commands and flags

`pr.rs` is 368 lines and `list` plus its filters is the largest thing in it, so list moves to its own
file rather than growing that one further. Shell completions are generated from the clap definition and
need no separate change.

## Error handling

Everything routes through the existing `BbError` and its exit codes — 0 ok, 1 error, 2 not
authenticated, 3 not found — with no new variants. Name-resolution failures, an unknown reviewer on
`remove` and an invalid `--review-state` value are all `BbError::Config`, exit 1. `--review-state` is a
clap enum, so an invalid value is rejected by clap before any request is made. A `404` from either the
`GET` or the `PUT` in a mutation surfaces as `BbError::NotFound`, exit 3.

## Testing

Integration tests under `tests/`, `wiremock` for HTTP, in the style of the existing suite. **Every test
sets `BB_KEYRING_DISABLE=1`** — a test that reaches the real OS keyring destroys the developer's stored
token, which has happened in this project before.

`tests/pr_list.rs` (new):
1. the request carries `fields=%2Bvalues.reviewers,%2Bvalues.participants,%2Bvalues.draft`
2. a reviewer who approved renders `✓`, one who requested changes `✗`, one with no state `·`
3. a tagged reviewer absent from `participants` still renders, as `·`
4. a commenter present in `participants` with `role: "PARTICIPANT"` does not render as a reviewer
5. `draft: true` renders `Draft`; `state: "DECLINED"` renders `Declined`
6. `--state draft` keeps only drafts
7. `--reviewer <name>`, `--author <name>`, `--author @me`, `--review-state approved`,
   `--needs-my-review` — one test each
8. two filters together AND rather than OR
9. `--json` emits the structured reviewer objects and no `approvals` key

`tests/pr_reviewers.rs` (new):
1. `reviewers <id>` renders name and state
2. `add` sends a `PUT` whose body is exactly `{title, reviewers}` with the union set
3. `add` of someone already tagged sends no `PUT` and exits 0
4. `remove` sends the reduced set
5. `remove` of an untagged name errors, exit 1, and sends no `PUT`
6. one bad name in `add a,b` means no `PUT` at all

`tests/user_resolve.rs` (new):
1. an email is used verbatim with no members call
2. a UUID in `{}` likewise
3. a substring resolves
4. an ambiguous substring errors and names every candidate
5. an exact match wins over a longer substring match
6. no match errors, mentioning the query
7. a `403` from `/workspaces/{ws}/members` falls back to the remaining pool and still resolves

## Verification

1. `cargo test` green, `cargo fmt --all --check` clean, `cargo clippy --all-targets -- -D warnings`
   clean.
2. Coverage does not regress below the current 90% floor.
3. Against the real API in `check24/solutions_console`: `bb pr list` shows non-empty reviewers with
   state marks on a pull request that has them — the exact case that is broken today — and
   `bb pr reviewers <id>` agrees with the web UI.
4. `bb pr list --json | jq` parses, and no human-facing text leaks into it.
