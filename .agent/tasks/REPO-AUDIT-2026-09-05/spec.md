# Repository snapshot and 3D playtest readiness audit

Frozen: 2026-09-05. Scope: preserve accumulated user changes, verify the current repository, and provide an evidence-based 3D readiness roadmap. This is an assessment, not implementation of the roadmap or release certification.

- AC1: Existing tracked modifications and intended untracked source/assets are preserved in a Conventional Commit; no secrets, local configuration or generated task media are included.
- AC2: Current build, format, test, lint, asset and startup checks are attempted; exact results and limitations are recorded. Failed product checks must remain visible; audit PASS does not mean release-ready.
- AC3: Architecture, code, gameplay, 3D presentation and UI/UX findings cite implementation evidence, identify player impact, prioritize fixes, and distinguish observations from hypotheses.
- AC4: The report defines a bounded first-playtest slice and measurable go/no-go criteria; a fresh independent verifier inspects the report and reruns representative checks.
- AC5: Snapshot and report are committed and pushed to origin, with remote SHA confirmed. Any repository changes beyond preservation/docs must be narrowly justified and implemented in a dedicated worktree after this freeze.

Constraints: no dependency additions, infrastructure changes, broad refactors or gameplay redesign in this assessment. Preserve unfinished 2D work and explicitly document its failures. No claim of manual UX/FPS verification without a real runtime observation. Task artifacts remain under this directory.

## Narrow packaging repair criteria (frozen before asset edits)

- AC6: Clean checkout uses a lockfile with the existing git SDK revision (no dependency upgrades), includes the existing two minion GLBs and provenance manifest, and the shipped avatar manifest references only the 16 committed offline avatars. The existing minion/hero asset validators pass in this worktree; local runtime-synced files in the original checkout are preserved. Patch version and release documentation describe these fixes.

- AC7 (frozen after fixture failure diagnosis, before test edit): the large real-UDP snapshot regression derives its avatar from the shipped roster, proves that identity round-trips, and retains the original >8KiB /10-player /8-structure /18-minion assertions. The targeted regression must pass with the16-avatar clean checkout; no transport, system settings or gameplay behavior changes.
