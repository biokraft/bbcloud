# Raising line coverage to 90% — design

**Status:** approved 2026-08-06

## Goal

Raise `bbcloud` line coverage from a measured 84.64% to at least 90%, by adding tests only.
No production code changes, and no coverage exclusions.

## Baseline

Measured with the same tool CI uses (`cargo llvm-cov --all-features --workspace`):

```
TOTAL   2031 lines   312 missed   84.64%
```

90% requires missed lines to fall to 203 or fewer, so this work must cover **at least 110 lines**.

Per-file starting point, worst first:

| file | missed | cover |
|---|---|---|
| `commands/update.rs` | 70 | 76.97% |
| `credentials.rs` | 52 | 66.88% |
| `commands/auth.rs` | 51 | 51.89% |
| `repo.rs` | 30 | 78.57% |
| `commands/pr.rs` | 28 | 90.04% |
| `git.rs` | 24 | 72.73% |
| `api/mod.rs` | 24 | 83.56% |
| `commands/pr_comments.rs` | 18 | 93.84% |
| `output.rs` | 8 | 95.90% |

## Constraints

- **No exclusions.** Genuinely untestable code stays in the denominator. The 90% is measured
  against the whole crate, including the interactive and browser-launching code that no test can
  reach.
- **Existing test style.** Integration tests under `tests/`, driving the real binary with
  `assert_cmd` and mocking HTTP with `wiremock`, exactly as `tests/api_client.rs` and
  `tests/update.rs` do today. Inline `#[cfg(test)]` unit tests only where the file already has
  them (`src/git.rs`, `src/repo.rs`, `src/output.rs`, `src/credentials.rs`).
- **No production changes.** Not a single line of `src/` behaviour changes. If a gap cannot be
  reached without a refactor, it stays uncovered.
- Every test must honour the existing safety rules: `BB_KEYRING_DISABLE=1` so no test can touch
  the real OS keyring, and no test may invoke a browser.

## Five test clusters

Estimated yield ≈ 114 lines against a need of 110. Because that margin is thin, the keyring
mock-builder tests described under "Deliberately left uncovered" are promoted to a contingency task,
run only if the five clusters measure below 90%.

### Cluster 1 — `auth` (~40 lines)

`login` is drivable without a TTY: with `--email <addr>` and `--token-stdin`, `would_prompt` is
false, so the non-interactive guard at `auth.rs:52` passes and no `inquire` prompt is reached.
Piping the token on stdin, pointing the API at a `wiremock` server that serves `/user`, and setting
`BB_KEYRING_DISABLE=1` (so `credentials::store` returns early) exercises `auth.rs:51,58-59,65-68,
79,86-89,92-97,99-110`.

Tests:
1. `login` success — asserts the verified account renders, exit 0, and that `--json` output is pure
   JSON.
2. `login` rejects an email without `@` (`auth.rs:80-84`).
3. `logout` warns when the legacy plaintext credential file exists — `HOME` set to a tempdir
   containing `.bitbucket-rest-cli-config.json` (`auth.rs:146-151`).
4. `status` still succeeds with a null account when `/user` fails (`auth.rs:119-124`).

### Cluster 2 — `repo::resolve` and `git` (~50 lines)

`repo::resolve()` is entirely untested (`repo.rs:85-121`), and covering it also drives
`git::current_branch`, `remote_url`, `remotes`, and `in_repo` (`git.rs:34-55`).

Tests:
1. `BB_REPO` set resolves without consulting git.
2. `BB_REPO` set to whitespace. **Corrected during implementation:** this does *not* fall through to
   git remote resolution as the code at `repo.rs:86-90` intends. `main.rs:21` declares `--repo` with
   `env = "BB_REPO"`, so clap fills the argument from the environment and `"   "` is parsed as a repo
   slug, which fails. The shipped test asserts that real behaviour, and `repo.rs:86-90` is
   unreachable dead code.
3. Outside a git repository, resolution fails with `no git repository here`.
4. In a git repo with no remotes, fails with `no git remotes configured`.
5. In a git repo whose only remote is not Bitbucket, fails with
   `no bitbucket.org remote found (checked 1)`.
6. `origin` pointing at Bitbucket resolves.
7. `origin` pointing elsewhere while a second remote points at Bitbucket still resolves — the
   fork/mirror case the comment at `repo.rs:96-98` describes.
8. `git::current_branch` on a detached HEAD returns `BbError::Git`. **Corrected during
   implementation:** the error does not come from `git.rs:36-40`. `git symbolic-ref --short HEAD`
   exits 128 on a detached HEAD rather than succeeding with empty output, so the friendly
   `"detached HEAD — cannot infer source branch"` message is unreachable and git's own
   `"not a symbolic ref"` stderr surfaces instead. The shipped test asserts that.
9. `git::remote_url` for a remote with no url returns `BbError::Git` (`git.rs:46-48`).

Temp git repos are created with `tempfile` and plain `git init`, following the pattern already in
`src/git.rs`'s test module.

### Cluster 3 — `api::Client` (~15 lines)

1. 403 maps to `BbError::Api { status: 403 }` (`api/mod.rs:85-88`).
2. 429 maps to `BbError::Api { status: 429 }` (`api/mod.rs:92-95`).
3. `put_json` has no test at all (`api/mod.rs:153-166`) — asserts the PUT method and body reach the
   server and the response deserialises. `post_json` and `delete` are already covered via
   `pr create` and the comment commands; only `put_json` is untested.
4. The `MAX_PAGES` cap at `api/mod.rs:180-181` — 101 distinct pages, each linking to the next,
   asserting pagination stops at 100.

### Cluster 4 — `pr create` validation (~5 lines)

Both fail before any HTTP request, so no mock is needed:
1. An empty target branch is rejected (`pr.rs:276-277`).
2. A source branch equal to the target is rejected (`pr.rs:279-281`).

### Cluster 5 — comment-rendering edge cases (~4 lines)

`pr_comments.rs:113` and `:129` are the "none" lines shown when a comment section is empty, and
`:134`/`:135` are the inline-location fallbacks for a comment with a file but no line, or with
neither. Three `wiremock` tests on `bb pr view` cover all four.

`output::heading` turns out to be covered already. `output.rs`'s residual gap is `warn`
(`output.rs:105-107`), which the cluster-1 `logout` legacy-file test covers, and the spinner body
(`output.rs:116-120`), which runs only when stderr is a terminal and so is unreachable from the test
harness — it joins the deliberately-uncovered list below.

## Deliberately left uncovered

Named here so the residual gap is a recorded decision, not an oversight:

- **`update.rs` self-update success path** (~50 lines). `self_update` calls
  `std::env::current_exe()` with no injection seam, so a real test would have the test binary
  replace itself mid-run. Covering it requires a production refactor, which this work excludes.
  `update.rs` therefore remains the weakest file.
- **`inquire` prompts** (`auth.rs:60-62,71-76`; `pr.rs:287-299`) — require a real TTY.
- **`open::that_detached` failure arms** (`browse.rs:38-40`, `pr.rs:355`) — no fakeable seam.
- **The spinner body** (`output.rs:116-120`) — guarded by `stderr().is_terminal()`, which is false
  under every test runner, so the tests always take the `ProgressBar::hidden()` path.
- **Real keyring read/write arms** (`credentials.rs:86-127`) — reachable via keyring's mock
  credential builder, but that mutates process-global state. Held in reserve, used only if the
  five clusters land short of 90%.
- Part of `credentials.rs`'s missed lines is its own `#[cfg(test)]` helper (`credentials.rs:189-203`);
  `cargo llvm-cov` counts test code in the denominator.

## Regression floor

The Codecov target moves to 90% via a committed `codecov.yml`, so a drop is reported on the pull
request. It stays advisory — the coverage job is not made to hard-fail under a threshold, so a
legitimate change is never blocked by an arithmetic cliff.

## Outcome

Line coverage reached **90.30%** — 197 missed lines of 2031, against a goal of ≤203. The keyring
contingency was not needed. Per file: `commands/auth.rs` 51.89% → 88.68%, `repo.rs` 78.57% → 97.14%,
`git.rs` 72.73% → 90.91%, `api/mod.rs` 83.56% → 99.32%, `commands/pr.rs` 90.04% → 91.81%,
`commands/pr_comments.rs` 93.84% → 94.86%. `commands/update.rs` stayed at 76.97%, as designed.

Writing these tests uncovered three pieces of **unreachable production code**, none of which this
branch fixed, because production changes were out of scope. Each is documented in a comment beside
the test that would otherwise look incomplete:

1. `repo.rs:86-90` — `main.rs:21` declares `--repo` with `env = "BB_REPO"`, so clap fills the
   argument before `resolve()` ever reads the variable. Side effect worth fixing: `BB_REPO="   "`
   now fails with `invalid repository` instead of falling back to git remotes.
2. `git.rs:36-40` — `git symbolic-ref` exits 128 on a detached HEAD, so the friendly message never
   renders and raw git plumbing reaches the user.
3. `pr_comments.rs:135` — `is_inline()` requires a non-empty `inline.path`, and `to_view()` derives
   `file` from that same path, so the `_ => "-"` location fallback cannot be reached.

A fourth discovery was a test-safety gap, and it *was* fixed here: the shared `bb()` helpers in
`tests/pr_create.rs` and `tests/pr_view.rs` never set `BB_KEYRING_DISABLE`, so those tests had been
running against the real OS keyring — the same class of defect that destroyed a developer's stored
token earlier in this project.

## Verification

1. `cargo llvm-cov --all-features --workspace --summary-only` reports TOTAL lines ≥ 90%.
2. `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` are all
   green.
3. `git diff --stat` touches only `tests/`, inline `#[cfg(test)]` modules in `src/`, `codecov.yml`,
   and `docs/`. Any change to production code in `src/` is a defect.
