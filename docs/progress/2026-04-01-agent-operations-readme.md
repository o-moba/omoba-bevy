# Progress Report: Agent Operations Readme

- Date: 2026-04-01
- Scope: add a copy-paste runbook for starting single and parallel repository tasks with agents

## Changes

- Added `docs/agents/README.md` as the central copy-paste reference for agent task startup.
- Documented the universal start prompt, a shorter variant, a parallel-tasks prompt, and a concrete `TASK-02` example.
- Kept the prompts aligned with the repository rules for `repo-task-proof-loop`, `.agent/tasks/<TASK_ID>/` reuse, dedicated `git worktree` usage, and changelog/progress updates.

## Checks

- Verified the prompt text matches the current operational rules in Cursor and Claude guidance.
- Verified the new runbook lives under `docs/` and is suitable for reuse from future chats.

## Remaining Risks

- The prompt templates still rely on the operator providing the correct task filename and task id placeholders.
- If the workflow rules evolve later, the README should be updated together with the rule files to stay synchronized.
