# Rebase procedure for `dominance-rebased`

## Context

`dominance-rebased` is one squashed commit sitting on top of `aec796c` ("more sigfigs"). `main` has moved ~19 commits ahead since then, including a major language-family refactor that touches almost every file the dominance work touches.

The end goal is **not** to merge this branch as-is. The goal is to:

1. Rebase `dominance-rebased` onto current `main`, resolving conflicts once with full context.
2. Then split the rebased commit into several focused PRs against `main` — refactor PRs first (e.g. `cost.rs` → `cost/` module split), then additive PRs (`viz/stackpath.*`, `expts/stackpath.py`), then the actual dominance behavior changes.

This file is about **phase 1 only**: the rebase.

## Why incrementally

We're walking the rebase forward one upstream commit (or one logical milestone) at a time, instead of jumping straight to `origin/main`. The reason: the upstream history contains a few large refactors (language-family generic-ization, `Op` indirection, `Var`-in-`Op`) that are the actual source of pain. Doing them one at a time means each conflict resolution has a single conceptual axis, and Maddy can verify nothing was dropped before moving on.

## The load-bearing commits (suggested ladder)

| Step | Target | What it brings |
|---|---|---|
| 1 | `38d97e0` | Multi-abstraction reshape of `lib.rs`, `main.rs`, `results.rs` |
| 2 | `038d5af` | Cheap commits (`#54`, `#55`) + first wave of Op indirection |
| 3 | `351528c` | **Big.** `lang.rs` → `lang/` split, generic stitchlang across `pattern.rs`, `search.rs`, `best_first.rs`, `cost.rs`, `smc.rs` |
| 4 | `883e934` | **Big.** Variables become part of `Op`; rewrites `pattern.rs` heavily again |
| 5 | `origin/main` | Tail (`#66` structural discriminants + small commits) |

Steps 3 and 4 are the ones that warrant a plan-before-action. Steps 1, 2, and 5 are usually mechanical.

## What to watch for (human-eye decisions)

The dominance branch and main both modified many of the same files. Some upstream changes are things we **want** to absorb fully (e.g. the language-family generic types — those are the new world). Others are things where the dominance branch may have diverged intentionally (e.g. `best_first.rs` semantics, `search.rs` matching behavior).

When resolving conflicts:

- **Default to keeping main's version of structural/type changes.** Re-apply our intent on top.
- **Flag anything ambiguous** rather than guessing — these decisions need Maddy's eye, not Claude's.
- Watch for cases where main *added* a feature that overlaps with dominance work; we may want to keep both, or pick one.
- Watch for cases where main *removed or renamed* something the dominance work depended on.
- **Watch for cross-language fallout in the python wrappers.** Whenever an upstream commit reshapes the JSON schema of `RunResult` (e.g. flat fields collapsing into a `library: Vec<AbstractionResult>`), `expts/*.py` may auto-merge cleanly but read fields at the wrong path and silently get `None`. After resolving rust conflicts, grep `expts/` and `viz/` for `data.get(...)` / `data["..."]` lookups against any field that moved, and patch them in the same step. Don't leave it for "later" — it's the same conceptual change. Flag the fix in the notes.

## Per-step procedure

### 1. Tag the pre-rebase state

Use a semantic tag name tied to the upcoming step number and upstream target, so the full ladder is legible after the fact.

```sh
# e.g. before step 3 (target 351528c):
TAG="rebase-step-3-pre-351528c"
git tag "$TAG" HEAD
echo "Pre-rebase tag: $TAG"
```

After all steps complete, the tags form a complete chain of intermediate states:

- `rebase-step-1-pre-38d97e0` → original branch state (before any rebasing)
- `rebase-step-2-pre-038d5af` → state after step 1 landed
- `rebase-step-3-pre-351528c` → state after step 2 landed
- ... and so on, with `HEAD` as the final state.

**Do not delete these tags until the whole ladder is done and the resulting branch is split into PRs.** They are the only way to reconstruct what happened at each step, and they are what makes the `range-diff` comparison possible retroactively.

### 2. If the next target is a "big" commit (step 3 or 4), STOP and present a plan first

Before running `git rebase`, look at what the upstream commit changes (`git show <commit> --stat` and skim the diff in the files our branch also touches). Write a short plan:

- Which files will conflict
- What the upstream change is conceptually doing
- How we'll likely want to adapt our changes to the new structure
- Anything that looks like it might force a human decision

**Run that plan by Maddy and wait for approval before rebasing.**

For small/mechanical commits, skip the plan and just do it.

### 3. Rebase

```sh
git rebase <next-commit>
```

Resolve conflicts. When in doubt, prefer main's structure and re-apply our intent on top. If a conflict is genuinely ambiguous (not just mechanical), stop and ask Maddy.

### 4. Sanity check

```sh
cargo check
cargo test
```

If `cargo check` fails, the conflict resolution missed something structurally — investigate before continuing.

If a test fails, that's a content-level signal. Read the failure carefully:
- Did our dominance changes legitimately alter behavior? (May need expected-output regeneration — but only with Maddy's approval.)
- Or did we drop something from the upstream commit?

#### Test landscape (changes across the ladder)

| File | Added | Notes |
|---|---|---|
| `tests/integration_test.rs` | pre-existing | runs from step 0 onward |
| `tests/multi_abstraction_test.rs` | step 1 (`38d97e0` / #47) | available from step 1 onward |
| `tests/stitch_compat_test.rs` | mid-ladder (`4d94294` / #57) | available from step 3 onward; golden-file driven against `data/domains/{simple-arithmetic,stitch}/` and `data/expected_outputs/`; refactored to CLI in #69 (step 5) |

So `cargo test` becomes progressively more meaningful as the ladder progresses.

### 4b. Regression capture (every step)

Beyond `cargo test`, run the `./run.py` shortcut suite and paste its tail into the notes section. This exercises the python wrappers + JSON schema + actual search behavior across all domains, which the rust tests don't cover.

```sh
./run.py quick_eval
```

Capture for the notes:

- **`quick_eval` tail.** Grab roughly the last ~20 lines of stdout, then crop to start at the first line that begins with `dials [` (everything before that is per-domain progress; the post-`dials [` block is the per-domain summary table that's the actual regression signal).

> **Note on `quick_full_enum`:** earlier ladder steps also ran `./run.py quick_full_enum`, but post-step-1 it no longer terminates in reasonable time. Do not run it. The `quick_eval` suite is sufficient for regression capture.

If either run errors out, that's a real regression — investigate before continuing. If the numbers shift but it doesn't error, paste them anyway and flag the shift in the judgment-calls list; some of the dominance changes do legitimately alter search behavior, but a shift introduced *by the rebase step itself* is a bug.

### 4c. Perf check (after big steps only)

Kavi's standard perf gate, used in #63, #65, #66 PR descriptions:

```sh
hyperfine --warmup 1 \
  'cargo run --release -- -i data/domains/cogsci/nuts-bolts.json --num-steps 4 --num-particles 200000 --temperature 1000 --max-arity 2'
```

Baseline on main is ~7s/run. Each of the big refactor PRs landed within ~1% noise. Run this after **steps 3, 4, and 5** (and after the eventual full rebase) to catch perf regressions introduced by conflict resolution. Compare against a checkout of `main` for the absolute baseline:

```sh
# baseline
git switch main
hyperfine --warmup 1 'cargo run --release -- -i data/domains/cogsci/nuts-bolts.json --num-steps 4 --num-particles 200000 --temperature 1000 --max-arity 2'
git switch -

# our rebased branch
hyperfine --warmup 1 'cargo run --release -- -i data/domains/cogsci/nuts-bolts.json --num-steps 4 --num-particles 200000 --temperature 1000 --max-arity 2'
```

A bigger regression than ~5% post-step is worth flagging to Maddy — dominance changes do have perf implications, but a regression introduced *by the rebase step itself* (vs. by the dominance work) is a bug.

### 5. Show the comparison

The whole point of this incremental approach is that Maddy can verify nothing was dropped. After the rebase lands, present **both** of these:

```sh
# A. What the upstream commit did (ground truth)
git show <next-commit> --stat
git show <next-commit>

# B. How our squashed commit morphed across the rebase step
git range-diff "$TAG"^.."$TAG" HEAD^..HEAD
```

`range-diff` shows a per-hunk comparison of "our commit before" vs "our commit after" the rebase. If we dropped an upstream change while resolving conflicts, it shows up here. If we cleanly absorbed the upstream change, the diff is small and explainable.

**Write everything to `rebase-tooling/REBASE_NOTES.md`**, appending a new section at the bottom with the template shown in that file. Do not edit prior sections. Maddy reads only the latest section and replies based on it — so the section must be self-contained:

- The pre-rebase tag name and post-rebase HEAD sha
- Plan (for big steps)
- Conflicts encountered and how each was resolved
- Judgment calls flagged for human review
- `cargo check` and `cargo test` results
- `quick_eval` tail (cropped to start at the first `dials [` line)
- Perf numbers (big steps only): main baseline, this branch, ratio
- `range-diff` synthesis: what morphed in our commit, and whether anything from upstream looks dropped
- Status line: ready for review / blocked on X

Then **fold the updated `rebase-tooling/REBASE_NOTES.md` (and any other rebase-tooling edits) into the rebased squashed commit via `git commit --amend`** — the ladder model is one squashed commit per step on top of upstream, and the notes are part of that step's artifact. Don't leave them as a dangling working-tree change. (The pre-rebase tag preserves recovery state, so amending is safe.)

Do **not** report any of this to Maddy directly in chat. Just point her at `rebase-tooling/REBASE_NOTES.md`.

### 6. Wait

Don't proceed to the next step automatically. Let Maddy review the range-diff and either approve the next step or ask for adjustments.

## Recovery & inspection

The chain of pre-rebase tags means **every intermediate state is preserved**, not just the start and end. Each tag is a complete commit that can be checked out, diffed, or used as a base.

If a step goes wrong:

```sh
git rebase --abort                          # if mid-rebase
git reset --hard rebase-step-N-pre-<sha>    # if rebase completed but went badly
```

To inspect a past step retroactively:

```sh
# What did our commit look like before step 3?
git show rebase-step-3-pre-351528c

# How did our commit change across step 3?
git range-diff \
    rebase-step-3-pre-351528c^..rebase-step-3-pre-351528c \
    rebase-step-4-pre-883e934^..rebase-step-4-pre-883e934
```

To list all rebase tags in order:

```sh
git tag -l 'rebase-step-*' | sort
```
