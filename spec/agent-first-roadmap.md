# Agent-first roadmap and implementation spec

- Status: proposed
- Date: 2026-09-25
- Baseline: `main` at `08a4fc9`, `bbcloud` 0.23.0
- Verification baseline:
  - `cargo fmt --all --check` passes.
  - `cargo clippy --all-targets -- -D warnings` passes.
  - `cargo test --all` passes: 498 tests, 0 failures, 5 credential-gated live tests ignored.

## Product thesis

- `bb` is an agent-first Bitbucket Cloud CLI, not a GUI replacement and not an agent-specific
  integration.
- The durable interface is the command line plus canonical JSON. Agent Skills teach agents how to
  use that interface; they are not the product boundary.
- Read and discovery operations should be cheap, selective, and safe to automate.
- Writes that express a review verdict or hide a review point remain human-gated.
- A feature fits when it removes shell glue for any agent and any repository. It does not fit when
  it merely moves `bb` into an IDE, a TUI, an MCP server, or local Git-state management.
- JSON is the machine contract. A second compact syntax is deferred until measured token usage
  proves that selective JSON is insufficient.

## Decisions already settled

- Keep the single Rust binary and the current human/JSON split.
- Keep review resolution, change requests, approvals, and merges under human control.
- Do not add a TUI. It is a different interaction model and pulls focus away from terminal-native
  agent workflows.
- Do not add clone/worktree orchestration or offline review to the near-term roadmap. Those are
  plausible future products, but they require their own collision, cleanup, dirty-worktree, and
  credential contracts.
- Do not add a compact output format yet. Prefer selective JSON sections.
- Do not copy code from `stefanvonderkrone/bbr`; it has no declared project license.
- Do not copy `renelachmann/bb-tool` authentication, implicit `.env` loading, raw error echoing, or
  unbounded pagination. Reuse ideas only.
- Never introduce approval or merge automation. A human may approve or merge in Bitbucket; an
  agent must not manufacture that verdict through `bb`.

## Evidence from the repository audit

### Contract-breaking findings

- Pagination can currently send the API token to another origin. `Client::url` accepts any absolute
  HTTP URL and `paginate` feeds it the server-controlled `page.next`; the authorization header is
  attached afterward. The cross-origin redirect guard does not cover this path.
  Evidence: `src/api/mod.rs:82-98`, `src/api/mod.rs:202-228`.
- Clap usage errors exit 2, the published code for “not authenticated.” The shipped skills tell
  agents to trust exit 2, so a typo can be diagnosed as a credential failure.
  Evidence: `src/error.rs:26-32`, `src/main.rs:767-784`, `README.md:319-324`,
  `.agents/skills/bitbucket-cloud/SKILL.md:19-20`.
- Three JSON commands serialize human-formatted relative timestamps. Other commands preserve raw
  RFC3339 specifically for agents, so the contract is internally inconsistent.
  Evidence: `src/commands/pr_comments.rs:20-35`, `src/commands/repo.rs:41-54`,
  `src/commands/branch.rs:30-51`, `src/api/models.rs:176-179`, `src/commands/pr_mine.rs:433-435`.
- `pr view --unresolved` filters comments independently. A resolved root can therefore leave its
  replies in the unresolved output, producing an orphan `parent` id and contradicting the documented
  thread-scoped resolution model.
  Evidence: `src/commands/pr_comments.rs:49-71`, `src/commands/pr_comments.rs:300-307`.

### Agent-facing functional gaps

- `pr view` fetches the pull request but discards its description, draft state, reviewers, task
  count, comment count, and exact timestamps. An agent cannot summarise a pull request from
  `bb pr view` without a browser or extra ad hoc requests.
  Evidence: `src/commands/pr_comments.rs:73-106`, `src/api/models.rs:161-191`.
- The main agent skill currently locates the current branch's pull request with a `git` plus `jq`
  incantation. That is product logic leaking into the skill.
  Evidence: `.agents/skills/bitbucket-cloud/SKILL.md:61-65`.
- `pr create` lacks `--description-stdin`, although long descriptions are the normal case for
  `bbc-open-pr` and the same repository already uses stdin for comments and edits.
  Evidence: `src/main.rs:276-301`, `.agents/skills/bbc-open-pr/SKILL.md:119-169`.
- `repo list --json` drops the full repository slug, web URL, and clone URLs that the API model
  already decoded. `project list --json` drops the project UUID.
  Evidence: `src/commands/repo.rs:7-13`, `src/api/models.rs:241-293`,
  `src/commands/project.rs:7-12`.
- Reviewer-name resolution fetches the same three user pools once per name. Two explicit reviewers
  can cost roughly thirty requests on a large workspace before the pull request is created.
  Evidence: `src/users.rs:32-98`, `src/commands/pr.rs:268-295`.
- The open-PR skill spends an entire stage approximating the resolvable user pool because no command
  exposes it. Evidence: `.agents/skills/bbc-open-pr/SKILL.md:80-115`.
- The daily brief labels a pull request “ready to merge” from approval plus a green build. It does
  not check conflicts, unresolved threads, or tasks, so the label can be false.
  Evidence: `.agents/skills/bbc-daily-brief/SKILL.md:126-164`.
- Bitbucket documents a read-only pull-request conflicts endpoint. It is absent from `bb` even
  though conflict detection is central to the conversation this project was built around.
  Reference: <https://developer.atlassian.com/cloud/bitbucket/rest/api-group-pullrequests/#api-repositories-workspace-repo-slug-pullrequests-pull-request-id-conflicts-get>.

### Low-effort corrections worth doing regardless of feature work

- Make `pr comment --body` conflict with `--body-stdin`.
- Make `pr list --state` reject invalid values locally instead of returning a Bitbucket 400.
- Add `--limit` to `pr list`; avoid draining every page when no client-side filter is active.
- Apply the one-page optimization already used by `pr mine` to `repo list` and `branch list` when
  no local filter is active.
- Check the missing argument for `bb pr reviewers` before constructing repository context.
- Reject or warn when a comma-separated workspace list is passed to a command that can act on only
  one workspace; silently discarding every value after the first is dishonest.
- Add missing help text for pull-request ids and the `pr create` title/description flags.
- Make auth logout report keyring deletion failures instead of claiming success unconditionally.
- Recognize Nix-managed installs before `bb update` attempts to replace a read-only store binary.
- Add `permissions: contents: read` and `--locked` to CI.
- Add live smoke coverage before relying on new or high-traffic API paths.

## Assessment of the external tools

### `stefanvonderkrone/bbr`

- What it is:
  - A separate Zig terminal UI for reviewing Bitbucket pull requests.
  - Strong local review, diff navigation, durable local drafts, comment-anchor handling, and
    worktree-aware review.
  - Active on `main`, with no GitHub release and no declared project license.
- Useful concepts:
  - Treat a review as a durable state machine rather than a sequence of unrelated API calls.
  - Preserve comment-anchor lifecycle and outdated-comment information.
  - Keep local drafts separate from published review state.
  - Make selective review context a first-class operation.
- Ideas not adopted now:
  - TUI, local diff workspace, syntax highlighting, and interactive review navigation.
  - Native-code grammar bundles and terminal clipboard integrations.
- Adoption decision:
  - Reference-only, clean-room implementation where behaviour is desired.
  - No code, documentation, or assets may be copied without permission.

### `renelachmann/bb-tool`

- What it is:
  - A separate, MIT-licensed Python CLI also named `bb`.
  - Focused on workspace discovery, clone URLs, bulk clone/worktree setup, and PR export.
  - Small and auditable, but without releases, CI, HTTP integration fixtures, or a current
    credential contract.
- Useful concepts:
  - Print machine-useful repository identity and URLs instead of discarding them.
  - Select only the PR sections a task needs.
  - Make bulk operations dry-runnable and report per-item results.
- Ideas not adopted now:
  - Clone/worktree orchestration.
  - Writing JSON to a file by default; `bb` keeps stdout as the value in JSON mode.
  - Automatic `.env` loading, legacy authentication aliases, raw response errors, and unbounded
    pagination.
- Adoption decision:
  - Inspiration only. There is no coherent subsystem worth taking over as a dependency.

### Combined conclusion

- Neither project is a credible takeover target for `bbcloud`.
- `bbr` proves that review UX is richer, but its central product direction conflicts with this
  repository's terminal-agent thesis.
- `bb-tool` contributes a short list of useful discovery/export ideas, not an implementation base.
- The highest-value transfer is conceptual: fewer shell pipelines, explicit data contracts, bounded
  work, and honest partial results.

## High-impact, project-agnostic feature sets

- **Agent-safe command contract**
  - Make exit codes unambiguous, keep JSON stdout pure, preserve exact timestamps, confine
    credentials to the API origin, and report partial data explicitly.
  - This is the foundation for every other agent workflow. A pretty table with a lying exit code is
    not an agent interface.

- **Selective pull-request decision context**
  - Let one command expose the pull request body, draft state, reviewers, exact times, build state,
    conflicts, and unresolved threads, while allowing the caller to omit expensive sections.
  - Keep this read-only. The tool reports facts; it does not manufacture a merge verdict.

- **Ownership-aware reviewer suggestions**
  - Rank Bitbucket identities by recent authorship in the files a change touches.
  - Return direct UUIDs and evidence. Never tag anyone until the human chooses.
  - Move the fragile, unverified reviewer-routing procedure out of the Agent Skill and into tested
    CLI code.

- **Current-branch and pipe-friendly pull-request lifecycle**
  - Find the current branch's pull request without `git` plus `jq`.
  - Read long descriptions from stdin.
  - Use effective repository/project default reviewers.
  - Keep command output bounded so an accidental bare `--build` cannot fan out into thousands of
    requests.

- **Repository and resolvable-user discovery**
  - Return full repository identity, web URL, clone URLs, and raw timestamps from list commands.
  - Expose the exact user pool used for reviewer resolution, including which sources were readable.
  - Stop making the open-PR skill reconstruct private resolver state from historical pull requests.

- **Read-only merge-candidate triage**
  - Combine approvals, build state, conflicts, unresolved threads, and tasks into factual signals.
  - Avoid the unqualified phrase “ready to merge” until every relevant Bitbucket policy blocker is
    represented. “Approved, green, and no reported conflicts” is honest; “nothing left but merge”
    often is not.

- **Audience-aware pull-request packets**
  - Let the PR-authoring skill choose a concise, standard, or deep description profile based on the
    audience and the request, rather than imposing one universal essay.
  - The concise profile leads with a TL;DR, scope, risks, and review-critical facts; the standard
    profile keeps the current reviewer-first structure; the deep profile adds alternatives and
    implementation detail.
  - This belongs in the agent workflow, not in a provider-specific integration. The CLI supplies
    facts; the agent supplies prose and still asks before creating anything.

## Ranked backlog

### Low effort: up to one day

- P0: Reject cross-origin pagination targets before attaching credentials.
- P0: Exit 1 for usage errors; retain exit 0 for help/version and exit 2 only for authentication.
- P0: Filter `--unresolved` by resolved thread roots, including descendants.
- P0: Preserve raw RFC3339 values in JSON while retaining relative text for humans.
- P1: Add `pr create --description-stdin`.
- P1: Add `pr list --current`.
- P1: Expose description, draft, reviewers, counts, and exact times from `pr view`.
- P1: Add `pr view --metadata-only` so callers need not download every comment.
- P1: Add concise/standard/deep profiles to the PR-authoring skill.
- P1: Use `/effective-default-reviewers` for pull-request creation and reviewer candidate lookup.
- P1: Reject `pr comment --body` with `--body-stdin`.
- P1: Validate `pr list --state` locally.
- P1: Add `pr list --limit` and one-page behavior where correctness permits it.
- P1: Add the missing help text and root next-step guidance.
- P1: Make logout failures visible.
- P1: Expand live smoke coverage for diff, comments, statuses, and conflicts.
- P2: Add the remaining documentation and CI hygiene items from the audit.

### Medium effort: roughly two to five days

- P1: Add optional build and conflict sections to `pr view`.
- P1: Fetch each user-resolution pool once per command.
- P1: Return full identity and URLs from repository/project JSON.
- P1: Add a read-only command exposing the resolvable reviewer candidate pool.
- P2: Add ownership-aware reviewer suggestions.
- P2: Use Bitbucket query filtering for branch, author, and reviewer narrowing where live tests prove
  the syntax and semantics.
- P2: Add raw ordering to `pr mine` and a documented stable sort.
- P2: Bound and explain API error response bodies.
- P2: Detect duplicate skill layouts for diagnosis, but do not work around OpenCode's upstream
  duplicate-name bug inside `bb`.

## Technical implementation spec

### 1. Harden the agent contract

- Goal:
  - Make the existing interface safe to trust before adding more features.
- Public behaviour:
  - Exit 0: command succeeded, help, or version.
  - Exit 1: usage/configuration/general failure.
  - Exit 2: authentication required or rejected.
  - Exit 3: requested resource does not exist.
  - `--json` stdout contains one JSON value on success and nothing on failure.
  - JSON timestamps are RFC3339 or null; human output may render relative time.
- Implementation trace:
  - Add a small clap error renderer in `src/main.rs`.
    - Use `Cli::try_parse()`.
    - Print help/version normally and exit 0.
    - Map all parse/usage errors to exit 1.
    - Replace the custom `pr mine -R` conflict path's `.exit()` with printing plus exit 1.
  - Add integration tests in `tests/cli.rs`.
    - Unknown command exits 1.
    - Invalid state exits 1 before HTTP.
    - `pr mine -R` exits 1.
    - `--help` and `--version` still exit 0.
    - Usage failures write no stdout.
  - Change `Client::url` in `src/api/mod.rs` to return a validated URL.
    - Resolve relative paths against the configured API base.
    - Permit absolute pagination links only when their origin exactly matches the API base origin.
    - Reject scheme, host, or port changes before `request` adds authorization.
    - Keep same-origin redirect behavior unchanged.
  - Add tests in `tests/api_client.rs`.
    - A cross-origin `next` link fails before the second server receives any request.
    - A same-origin absolute `next` link still works.
    - A relative `next` link still works.
    - The error never contains credentials.
  - Make comment resolution filtering thread-scoped in `pr_comments::partition`.
    - Index resolved root ids first.
    - Treat a root comment as its own thread root; treat a reply's root as its `parent` id.
    - Omit every comment in a resolved thread when `unresolved` is true.
    - Keep a reply whose root is absent, because its state cannot be inferred safely.
  - Add unit tests for:
    - A resolved root with resolved and unresolved-looking replies.
    - An unresolved root with replies.
    - A reply whose root is absent.
    - General comments remaining visible under `--unresolved`.
  - Preserve raw timestamps additively first.
    - Add `created_on: Option<String>` to `CommentView`.
    - Add `updated_on: Option<String>` to repository and branch rows.
    - Keep existing display fields for compatibility.
    - Make the embedded skills read the raw fields.
  - Sort comments by parsed UTC instants, with a stable id tie-breaker; do not compare RFC3339
    strings or let missing timestamps float to the top accidentally.
- Verification:
  - `cargo test --test api_client --test cli --test pr_view`
  - `cargo test --all`
  - `cargo clippy --all-targets -- -D warnings`
  - Assert stdout purity in human and JSON modes.
- Done when:
  - A deliberately hostile `next` link cannot receive the Basic authorization header.
  - No usage error can be mistaken for missing authentication by exit code.
  - An agent receives exact comment/repository/branch times in JSON.

### 2. Make `pr view` the selective decision context

- Goal:
  - Replace the browser and multi-command shell pipeline with one honest read model.
- Command shape:
  - `bb pr view <id> [--metadata-only] [--unresolved] [--build] [--conflicts]`
  - Existing default behaviour remains: pull request plus all comments.
  - `--metadata-only` fetches the pull request and no comments; it conflicts with `--unresolved` and
    the existing `--comments-only`.
  - `--build` adds a build section at one extra request.
  - `--conflicts` adds a conflict section at one request, following the same-origin redirect.
  - `--unresolved` filters inline threads, not general comments.
- JSON shape:
  - Whenever `pull_request` is present, enrich it with fields already available from the object:
    - `id`, `title`, `description`, `state`, `draft`, `author`, `source`, `destination`, `url`.
    - `created_on`, `updated_on`, `comment_count`, `task_count`.
    - `reviewers[]` using the existing `{name,uuid,state}` contract.
  - Keep `general[]` and `inline[]` stable.
  - Add `build` only when requested:
    - `build_state`
    - `statuses[]`
  - Add `conflicts` only when requested:
    - `count`
    - `files[]` with `path`, `scenario`, and `message`.
  - Add an unresolved-thread summary when `--unresolved` is active:
    - Root thread count after resolution filtering.
    - Do not infer whether each thread is “waiting on me”; author identity alone is insufficient
      without the authenticated user's uuid and the last reply author.
- Implementation trace:
  - Extend the pull-request response request in `pr_comments::view` with `REVIEWER_FIELDS`, or fetch
    the individual object with enough partial fields to include reviewers and participants.
  - Build a private `PullRequestView` rather than hand-projecting JSON inline.
  - Add tolerant API models for `FileConflict`; all descriptive fields remain optional and an empty
    page means zero reported conflicts.
  - Reuse `pr_build::statuses_for` for the optional build section.
  - Fetch comments, build statuses, and conflicts concurrently only after validating flags; finish
    and clear the spinner before any output.
  - Gate every human-only section behind `Format::Human`; JSON is always one serde value.
  - Update the daily brief to request conflicts only for phase-two candidates.
    - Never label a pull request “ready to merge” from approval and build state alone.
    - Use “approved, green, and no reported conflicts” while branch policy coverage is incomplete.
    - Count unresolved root threads as visible outstanding work.
  - Update `bitbucket-cloud`, `bbc-daily-brief`, and any other embedded skill whose command map
    changes.
- Verification:
  - Wiremock tests for default, metadata-only, build, conflicts, unresolved, empty, and partial
    responses.
  - Assert cross-origin conflict redirects never replay credentials.
  - Assert `--json` stdout is exactly one value for every flag combination.
  - Add ignored live tests for `pr view`, statuses, and conflicts against `BB_LIVE_REPO` and
    `BB_LIVE_PR`.
- Done when:
  - An agent can answer “what does this PR say, who reviews it, what is green, what conflicts, and
    what is unresolved?” without opening a browser or inventing a second API client.
  - No command claims merge readiness from incomplete facts.

### 3. Add pipe-friendly current-branch and long-text flows

- Goal:
  - Remove recurring shell glue from the highest-frequency agent actions.
- Command changes:
  - `bb pr list --current [--json]`
    - Resolve the current symbolic branch with `git::current_branch`.
    - Ask Bitbucket for pull requests whose source branch matches.
    - Return zero or many rows; do not guess when several targets are open.
    - Fail clearly on detached HEAD or outside a checkout.
  - `bb pr create ... --description-stdin`
    - Mutually exclusive with `--description`.
    - Read the body before any reviewer lookup or write.
    - Trim one trailing newline, matching `pr edit --description-stdin`.
    - Work with a pipe or redirected file without placing the body in shell history.
  - `bb pr list --limit <n>`
    - Cap rendered rows.
    - When all filters are server-side and no local filter remains, request only the bounded first
      page.
    - Continue paginating when a local filter makes completeness necessary.
- Default-reviewer correction:
  - Replace the repository-only `/default-reviewers` lookup used by `pr create` with
    `/effective-default-reviewers` so project-inherited reviewers are included.
  - Model `{user,reviewer_type}` tolerantly and deduplicate by uuid.
  - Keep `--reviewer` and `--no-default-reviewers` authoritative; the correction only fixes the
    default path.
- Supporting corrections:
  - Make `--body` and `--body-stdin` conflict.
  - Validate state values before HTTP while preserving the existing public API as much as possible.
  - Add help text for all id positionals and create title/description flags.
- Implementation trace:
  - Add list arguments in `src/main.rs` and carry them in `ListArgs`.
  - Build and URL-encode a Bitbucket `q=source.branch.name="..."` filter.
  - Keep local filtering as a defensive postcondition until a live test proves server filtering.
  - Add a small reusable stdin-body reader for create/edit/comment semantics; do not build a general
    input framework.
  - Add live tests for `--current` and effective default reviewers because wiremock cannot prove
    Bitbucket query semantics.
  - Update `bbc-open-pr` to pipe the approved description file into `pr create`.
  - Replace the manual `git` plus `jq` current-PR recipe in `bitbucket-cloud` with `pr list
    --current`.
- Verification:
  - Integration tests for stdin bodies, flag conflicts, zero descriptions, current-branch matching,
    detached HEAD, multiple matches, and server-filter query encoding.
  - Assert no API request occurs before invalid stdin input is detected.
  - Assert large descriptions are byte-for-byte preserved except for the documented trailing
    newline.
- Done when:
  - Opening a reviewed pull request and finding the current one are native `bb` operations, not shell
    assembly.

### 4. Expose repository identity and fetch reviewer pools once

- Goal:
  - Make discovery commands useful to agents and remove repeated user-pool requests.
  - Eliminate the open-PR skill's historical-PR workaround without adding clone/worktree behavior.
- Repository/project output:
  - Keep the existing human table and existing JSON fields for compatibility.
  - Add `repo: Option<String>` from `Repository::full_name`.
  - Add `url: Option<String>` from the API's HTML link.
  - Add `clone_urls: {ssh: Option<String>, https: Option<String>}` from the API's clone links.
  - Add `project_name: Option<String>` and `updated_on: Option<String>` while preserving the current
    display-oriented `project`, `access`, and `updated` fields.
  - Add the project UUID to `project list --json`; keep the existing key, name, and access fields.
  - Do not synthesize clone URLs. The server-reported URL is authoritative.
- Refactor:
  - Introduce an internal `UserPool` value in `src/users.rs` that is loaded once, preserves source
    provenance, deduplicates by uuid, and records incomplete sources.
  - Resolve many names against that value without another network call.
  - Keep the current public `resolve_user` convenience function if compatibility requires it.
  - Update these callers to load one pool per command:
    - `pr list` with both reviewer and author filters.
    - `pr create --reviewer`.
    - `pr reviewers add|remove`.
- Candidate command:
  - `bb repo members --json`
  - Human table: name, nickname, uuid, sources.
  - JSON: `{ "users": [...], "partial": [...] }`.
  - A source is one of `workspace`, `repository`, or `default_reviewer`.
  - `partial` names pools that could not be read. The command must not imply a complete resolver
    pool when it is not complete.
- Implementation trace:
  - Extend the private repository/project output rows with the additive fields above.
  - Reuse `Repository::clone_url`, `Repository::html_url`, and the existing clone-link model rather than
    rebuilding URLs from the slug.
  - Add JSON integration assertions for full slug, URL, both clone protocols when reported, project
    UUID, and absent optional fields.
  - Refactor candidate loading before changing command output.
  - Add request-count tests: two reviewer names use one pool load, not two.
  - Add partial-source tests for 401/403/404 behaviour matching the resolver's current fallbacks.
  - Stop the candidate lookup as soon as authentication fails; do not spend later requests on a
    credential the server has already rejected.
  - Add the `repo members` Clap subcommand and tests for empty, partial, and JSON output.
  - Rewrite `bbc-open-pr` step 3 to use the command's exact pool. Remove the approximation built from
    historical pull requests.
- Verification:
  - `cargo test --test user_resolve --test user_resolve_cli --test pr_list --test pr_create
    --test pr_reviewers`
  - New `tests/repo_members.rs` for command integration.
  - Assert token names and workspace/repository identifiers never appear in output.
- Done when:
  - Resolving several reviewers costs one candidate-pool load.
  - The open-PR skill no longer claims the resolver pool is unknowable when `bb` can report it.

### 5. Suggest reviewers from file ownership, but never tag without a selection

- Goal:
  - Put Jan and Sean's file-ownership idea into tested, repository-agnostic CLI code instead of a
    long Agent Skill workaround.
- User-visible command:
  - Existing pull request: `bb pr reviewers suggest --pr <id> [--since 12mo] [--limit 5]
    [--file-limit 25]`.
  - Prospective pull request: `bb pr reviewers suggest <target> [source] [same options]`.
  - `source` defaults to the current branch for the prospective form.
  - `--pr` and the prospective target form are mutually exclusive.
- Behaviour:
  - Read changed paths from the pull-request diffstat or the source/target diffstat.
  - Query recent Bitbucket commit history for each path, restricted to the source branch and
    excluding the target when both refs are in the same repository.
  - Aggregate only commits whose author includes a Bitbucket user with a uuid.
  - Exclude the pull-request author and already tagged reviewers.
  - Return transparent evidence: recent commit count, matching files, and last commit date.
  - Do not produce an opaque magical score. Order by recent commit count, then recency, then name.
  - Never call `reviewers add` or `pr create` from this command.
- JSON shape:
  - `pull_request` or prospective `{source,target}` identity.
  - `since` cutoff.
  - `files_scanned`, `files_skipped`, and `history_complete`.
  - `errors[]` with `{path,message}` for per-path history that could not be read.
  - `suggestions[]` with `{name,uuid,commit_count,files,last_commit_on}`.
  - Partial or bounded history must be visible in the payload, not silently implied.
- Bounds:
  - Default history window: 12 months.
  - Default output: 5 people.
  - Default changed-file cap: 25, chosen deterministically from the diffstat.
  - At most 8 history requests in flight, matching the existing `pr mine` bound.
  - One page of 100 recent commits per path is enough for ranking; if every returned commit is
    inside the window, report `history_complete: false` rather than paginating without bound.
- Implementation trace:
  - Extend endpoint models with the source repository identity and commit author/date.
  - Add safe URL encoding for branch names used as path segments.
  - Add a bounded diffstat/history service under `src/commands/pr_reviewers.rs` or a small dedicated
    module; do not put the algorithm in the Skill.
  - Reuse `futures::stream::buffer_unordered(8)` and preserve per-path errors in the report instead
    of turning one unavailable history into a total command failure.
  - Add a human table and the JSON shape above.
  - Update `bbc-open-pr`:
    - Use suggestions from the prospective command before creation.
    - Show the evidence to the user.
    - Ask which people to invite.
    - Pass only the selected names or uuids to `pr create --reviewer`.
    - Never tag anyone else.
  - Add a live smoke test for path-filtered commit history and effective diffstat semantics.
- Verification:
  - Wiremock fixtures for same-repo history, fork-like missing history, renamed files, users without
    uuids, author exclusion, existing-reviewer exclusion, per-file partial failure, and deterministic
    bounds.
  - Assert no PUT/POST request is made by the suggestion command.
  - Assert the current open-PR skill names direct uuids when available and still asks the human.
- Done when:
  - Any agent can obtain defensible reviewer evidence through `bb` without reimplementing Git
    ranking, scraping historical pull requests, or tagging uninvited people.

### 6. Add audience-aware PR packet profiles

- Goal:
  - Preserve the useful part of the conversation about reviewer packets without hard-coding one
    person's style or adding a provider-specific agent integration.
  - Let the agent choose how much detail a reader needs while keeping the CLI factual.
- Profiles:
  - `concise`:
    - TL;DR.
    - Scope and files/areas changed.
    - Review-critical risks or decisions.
    - Verification performed and remaining uncertainty.
  - `standard`:
    - The current `bbc-open-pr` structure: first-paragraph summary, why, what changed, details, and
      testing.
  - `deep`:
    - The standard profile plus alternatives considered, edge cases, and implementation rationale.
- Selection rules:
  - Honour an explicit user request such as “short”, “reviewer-focused”, or “deep”.
  - If the audience or detail level is genuinely unclear, ask once before drafting.
  - Default to `standard`; do not ask a cosmetic question when the user already made the choice.
  - Never invent tests, metrics, risks, or reviewer concerns to fill a profile.
  - Never create the pull request until the exact rendered description is shown and approved.
- Implementation trace:
  - Update `.agents/skills/bbc-open-pr/SKILL.md` with the three profiles and one selection question.
  - Use the richer `pr view --metadata-only` facts as inputs; do not duplicate API knowledge in the
    skill.
  - Keep the existing no-emoji, Bitbucket-compatible Markdown rules.
  - Add skill-content tests for all profile headings, the approval gate, and the rule against
    fabricated verification.
  - Do not add a provider-specific flag or a new binary. The profile is an agent workflow choice,
    not a second machine format.
- Verification:
  - `cargo test --test skill`
  - `cargo test --all`
  - Manual fixture review with one concise, one standard, and one deep example using `acme` data.
- Done when:
  - A reviewer who wants the critical facts gets them first, while a maintainer can request the full
    rationale, without maintaining separate agent-specific skills.

### 7. Expand live endpoint assurance

- Goal:
  - Wiremock proves the client sends the intended request; it cannot prove the endpoint still exists.
- Harness changes:
  - Reuse `BB_LIVE_REPO` and add `BB_LIVE_PR` for repository- and pull-request-scoped tests.
  - Keep every test read-only and `#[ignore]`-gated.
  - Require `BB_LIVE_TEST=1`, resolvable credentials, and the relevant explicit fixture variables.
- Add live coverage for:
  - Pull-request metadata and comments.
  - Diff plus its same-origin redirect.
  - Diffstat and commits.
  - Commit statuses.
  - Conflicts.
  - Branch listing.
  - Effective default reviewers.
  - Candidate-pool endpoints, reporting expected partial permission rather than turning a known 403
    into a false regression.
- Verification:
  - `BB_LIVE_TEST=1 BB_WORKSPACE=<slug> BB_LIVE_REPO=<workspace>/<repo> BB_LIVE_PR=<id> cargo test
    --test live -- --ignored`
- Done when:
  - Every new API path in this roadmap has either a credential-gated live check or an explicit
    documented reason it cannot have one.

## Sequencing

- Phase 0, contract before convenience:
  - Cross-origin pagination guard.
  - Usage exit-code correction.
  - Thread-scoped unresolved filtering.
  - Raw timestamps in JSON.
  - Additional live tests for existing high-traffic reads.
- Phase 1, complete one-PR reads:
  - Enrich `pr view`.
  - Add metadata-only, build, and conflicts sections.
  - Correct the daily brief's readiness wording.
- Phase 2, remove shell glue:
  - `pr list --current`.
  - `pr create --description-stdin`.
  - Effective default reviewers.
  - Local state validation, limits, help, and body-flag conflict.
  - Add audience-aware profiles to the PR-authoring skill.
- Phase 3, make reviewer resolution efficient and inspectable:
  - Load user pools once.
  - Add `repo members`.
  - Rewrite the open-PR skill's approximation.
- Phase 4, ownership-aware suggestions:
  - Implement the bounded suggestion algorithm.
  - Keep the human selection gate.
- Operational rule:
  - One focused branch and pull request per phase item.
  - Run the local format, Clippy, and test gates before each pull request.
  - Do not hand-edit release versions, changelog entries, or tags; release-plz owns them.

## Documentation and release trace

- Update Clap help first so the command surface and README cannot describe different tools.
- Update the README usage and command map.
- Update every bundled Agent Skill whose commands or JSON fields change.
- Add tests that keep security invariants and human gates visible in skill text where applicable.
- Let the public API determine the semver bump; do not choose a version manually.
- Run the live smoke tests for every added or moved endpoint before release.
- Use conventional commit prefixes for changelog grouping, but rely on `cargo-semver-checks` for the
  actual version decision.

## Explicit non-goals

- No TUI.
- No MCP server.
- No automatic PR approval, merge, or decline.
- No unreviewed reviewer tagging from suggestion or creation flows; explicit reviewer commands remain
  available to humans and to agents acting on a user's named selection.
- No compact output language before measurement justifies one.
- No local/offline review engine.
- No clone, worktree, branch creation/deletion, commit, push, or merge orchestration.
- No implicit dotenv loading.
- No raw API-body error output.
- No cross-origin credential replay.
- No code or asset reuse from an unlicensed project.
- No provider-specific agent adapter; all features remain usable by humans, Claude Code, OpenCode,
  Codex, Cursor, scripts, and agents the project has never heard of.
