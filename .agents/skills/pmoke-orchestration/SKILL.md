---
name: pmoke-orchestration
description: Operate the 4-role bot orchestration (commander/worker/reviewer/ops-watch) on pmoke Kanban boards under pmoke repository policy. Use when triaging board work, dispatching implementation or review, merging task branches, or running ops routines for this repository. Do not use for unrelated generic work or live instrument operations.
---

# Pmoke orchestration

Run pmoke board work through four roles. A role is a responsibility, not a
second scheduler: the upstream dispatcher/cron owns scheduling, Kanban rows
plus git history own the truth, and Bot Chat/DM transcripts never do.

## Role lanes

- commander (Issue/board triage, assign, merge judgment): keeps one
  dispatcher owner per task, one Issue per durable goal, and a linked normal
  PR for each independently reviewable public implementation slice. Assigns
  only the four roster roles on new work. Merges only with reviewer verdict
  `pass` plus the gate set below; never bypasses checks, force-pushes, or
  pushes to `main` directly. Merge, deployment, release, push, and branch
  deletion additionally need the maintainer handoff in `AGENTS.md` and an
  explicit ask.
- worker (implementation and PR handoff): owns one task branch plus one
  isolated worktree, implements inside the task scope, and ends with the
  commit SHA, linked PR, and validation results on the task. Never invents
  task IDs, occupies the `main` checkout, or mutates Kanban or shared memory
  from a leaf.
- reviewer (read-only Review 1/2): verifies the candidate in a disposable
  detached worktree pinned to the candidate HEAD, leaves the integration
  worktree untouched, and records a terminal verdict with handoff (commit
  SHA, commands plus results, residual risks). Review 1 checks the Issue,
  design/API or public contract, compatibility, security, and rollback;
  Review 2 checks the complete staged diff, tests, generated outputs, and
  public-artifact hygiene. One correction wave per card; a second failed
  review blocks and escalates instead of looping.
- ops-watch (routines, reap, digest): owns cron routine prompts
  (self-contained, `[SILENT]` on no change) and worktree/branch reaping.
  Reap only `done`/`archived` tasks whose merge is confirmed, on a later
  tick, never immediately after merge. Dirty, unmerged, detached-unknown,
  and rollback branches go to commander judgment; never delete, reset, or
  stash them.

Legacy assignees on historic rows stay untouched; new work uses the four
roles only. Out-of-roster tasks are not dispatch targets.

## Issue and PR ownership

Every durable goal has one GitHub Issue with its purpose, scope, non-goals,
acceptance criteria, compatibility impact, and security impact. The commander
owns that Issue and keeps it separate from implementation status. Once
external publication is explicitly authorized and public implementation starts,
the worker opens a linked normal PR for the slice with `Refs #N`, keeps it open
for review from the beginning, and never uses a Draft phase. The PR records
only its current outcome, changed surface, validation evidence, blockers,
residual risks, and next work.

Review 1 covers the Issue, design, API or public contract, compatibility,
security, and rollback. Review 2 covers the complete staged diff, tests,
generated outputs, and public-artifact hygiene. The commander may merge only
when the acceptance criteria and relevant evidence are complete, the exact PR
head is current, required and relevant checks pass on that head, conversations
are resolved, and no blocking review remains. With explicit maintainer
authorization, verify deployment or release after merge when applicable, then
close the Issue explicitly.

## Kanban task shape

Every task body states scope, non-goals, the owned paths, and validation.
Parallel tasks declare the paths each owner may touch. A run becomes
`running` only after run identity, owner, workspace, lease deadline, and
the start event are durably recorded. Heartbeat every 60s while working,
`kanban_complete` with SHA plus results, `kanban_block` on stall; silent
exits are forbidden.

## Git and worktree flow

Start from current `main` on a private local branch first
(`wt/<task-id>`), one logical change per commit with conventional names
when commits were requested. Run the narrowest relevant validation before
committing, Review 1 on the working tree, stage only intended files,
Review 2 on the staged diff; any later edit restarts both reviews.
Reap records `git worktree list --porcelain` before and after and leaves
evidence on the task.

## Validation gates

Rust/Python lanes from `AGENTS.md` (`cargo fmt --check`, locked
check/test/clippy, Python unittest discovery), generated references only
via `cargo xtask docs-export`, website via `pnpm check`, dependencies via
`cargo deny`/`pnpm audit`/license checks. Skipped gates are reported as
skipped, never passed.

Builds, tests, and ordinary validation must not contact, trigger, fetch from,
screenshot, or otherwise operate live instruments. Use dummy transports,
loopback fixtures, and isolated temporary directories. Active hardware checks,
including `doctor --probe-fetch` or an equivalent probe, require separate
explicit authorization. A successful service, GUI, or native-link check never
proves hardware reachability.

## Evidence and stop conditions

Reports carry role, board/task/run/worktree scope, changes and
non-changes, validation with evidence class, residual risks, rollback,
and the next gate. Every autonomous mutation (merge, reap) links its
verdict and gate evidence on the task; evidence-free automation is
forbidden. Stop and report on merge failure, conflict, unresolved gates,
bypass needs, missing hardware/credentials, or ambiguous contracts. This
is a public repository: no secrets, raw captures, private URLs, machine
paths, or unreviewed logs in branches, commits, issues, or PRs.
