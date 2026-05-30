# AGENTS.md — verbreel-engine

Authoritative review context for `git-ai-review`. Reviewers MUST treat
the linked files as ground truth alongside this document.

## Project rules

See `CLAUDE.md` (project-level architecture rules, crate dependency
order, write-ordering invariants, canonical JSON discipline).

## Current task brief

Active task lives in `.claude/brief.md` — read it in full before
reviewing the diff. It states:

- The specific verb / sub-system being implemented
- The acceptance criteria the diff must meet
- Established patterns to mirror (error mapping, v1 floor behavior,
  signature shape) — deviations are a fail unless justified

If the brief contradicts CLAUDE.md, the brief wins for the current
slice; CLAUDE.md wins for everything not covered by the brief.

## Spec references

- Spec root: `~/playground/verbreel-spec/spec/` (or
  `https://github.com/rdh073/verbreel-spec`)
- Conventions: `spec/commands/conventions.md` §0.1–§0.18
- Verb tables: `spec/commands/*.md` — the verb being touched in the
  diff has a row that fixes its name, signature, and error mapping

## Review priorities (in order)

1. Does the diff satisfy the brief's acceptance criteria?
2. Does any change break a published verb signature, error code, or
   event-log schema? If yes — `BREAKING CHANGE`, fail.
3. Is the fix at the root cause, or at the crash site?
4. Are forbidden patterns present? (catch-and-swallow, `?.` to mask
   null bugs, `setTimeout`/`sleep` to mask races, broad `except`,
   `_ = err`, premature abstraction, `SELECT *`, etc.)
5. Is the diff minimal? Reformatting / drive-by renames mixed with
   logic changes = fail.
6. Are tests included? Bug fix without regression test = fail. New
   verb without table-driven coverage = fail.
7. Comments: new comments must explain WHY (non-obvious constraint),
   not WHAT. PR/ticket/author references in comments = fail.

Be direct. Either `status=pass` or `status=fail` — no "looks good
with minor nits."

## What counts as a finding

A finding must meet ALL of:

1. **Meaningful impact** — accuracy, performance, security, or
   maintainability. Not aesthetics.
2. **Discrete and actionable** — one specific bug, not "the codebase
   has issues."
3. **Introduced by THIS diff** — pre-existing bugs are not flagged.
4. **Proportionate rigor** — don't demand input validation on a
   one-off script that has none elsewhere; don't demand exhaustive
   tests on a prototype.
5. **Provable, not speculative** — if you claim a change disrupts
   something else, name the file/function/line that breaks. "May
   affect X" without identification is not a finding.
6. **Not an intentional choice** — if the diff deliberately changes
   established behavior and the change is consistent with the brief,
   it is not a bug.
7. **The author would fix it if they knew** — not "technically
   suboptimal but they'd defend it."

If a candidate finding fails any of these, drop it. Prefer zero
findings over noise.

## Priority tagging

Every finding title starts with `[P0]`–`[P3]`:

- `[P0]` — Blocking. Universal break (BREAKING CHANGE on a published
  verb, data corruption, security hole, lost events). Drop everything.
- `[P1]` — Urgent. Forbidden pattern, missing regression test on a
  bug fix, root-cause vs crash-site fix.
- `[P2]` — Normal. Logic error that's not data-corrupting, missing
  test on new code path, naming that obscures meaning.
- `[P3]` — Nice to have. Don't open `status=fail` for P3-only.

Example title: `[P1] render.status returns BadArgs instead of Custom`.

## Comment construction

When you write a finding body:

- One paragraph. No line breaks inside natural language flow unless
  needed to set up a code fragment.
- ≤ 3 lines of code per inline snippet; wrap in fenced code or
  inline backticks.
- Cite the exact file:line range. Keep the range as short as
  possible — pinpoint the problem, do not span 20 lines.
- State the scenarios/inputs/environments under which the bug
  manifests. If it only breaks on a specific input shape, say so —
  severity depends on it.
- Matter-of-fact tone. No "Great job", no "Thanks for", no "nits,
  otherwise great." Helpful AI assistant, not effusive human reviewer.
- Where a concrete one-or-two-line replacement exists, give it as a
  ```suggestion block. Preserve exact leading whitespace (tabs vs
  spaces, count). Do not change outer indentation unless that IS the
  fix.

## verbreel-specific bug taxonomy (always at least P1)

- Mutation bypassing `verbreel_state::engine::apply()` — P0.
- In-memory patch applied before `events.jsonl` flush — P0 (write
  ordering, §0.8).
- Asset stored without content-addressed path `assets/<aa>/<sha256>.<ext>`
  — P0.
- Hashing via `serde_json::to_string` instead of `verbreel_canon::jcs::
  canonicalize` — P0.
- Verb signature deviates from `spec/commands/*.md` row (param names,
  order, types, error codes) — P0.
- Error mapped to `BadArgs` where the spec says `Custom`, or vice
  versa — P1.
- Crate import that violates the dependency order in `CLAUDE.md` — P1.
- New verb without table-driven test covering the brief's listed
  cases — P1.

## Project board protocol

- Board Status is auto-projected from PR/issue lifecycle by `board-sync.yml` — do NOT manually move the board. Mapping: issue assigned / draft PR open → In progress; PR ready-for-review → In review; PR merged → Done; PR closed unmerged → In progress.
- `.github/scripts/board-move.sh N "<Status>"` is a manual override for exceptions only. board-sync is not a gate and never blocks a PR.
- Always land changes via PR, not close-via-commit-message.
- Estimate must be set on the Backlog entry; no Estimate = 0 is allowed except for sub-issues that explicitly link to a parent estimate.
