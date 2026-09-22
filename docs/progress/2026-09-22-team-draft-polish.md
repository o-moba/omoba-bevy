# Team draft and mobile combat-circle polish

Task: TASK-TEAM-DRAFT-POLISH-2026-09-22. Base c900df8, isolated branch codex/team-draft-polish.

The six requested corrections cover mobile combat geometry, shared character and
role choices, coordinated loading, preview orientation/drag, automatic sides, and
visible Studio avatars. Desktop skills retain their centered bottom placement.
An independent art-direction pass fixed the circle geometry before implementation.
The server owns roster generations, accepted choices, countdown and readiness.
Clients keep paid-ticket authorization and wait for actual asset dependencies.

Verification results and native captures are recorded in the task evidence.
Phone captures are desktop previews, not physical-device certification. The prior
supplemental social-capture renderer-teardown hang remains unresolved; this task
does not claim complete platform release certification or external Studio uptime.

Final verification ran759 Rust tests (457 client,10 passport,292 server/shared/harness)
with15 pre-existing database-dependent tests ignored. Current clippy with warnings
as errors, workspace formatting, diff checks and native client/server builds passed.
Eleven isolated native runs completed with clean exits and83 rendered screenshots:
three phone sizes and desktop HUD; desktop/phone four-peer drafts, countdown and
loading; solo practice leave/rejoin; default/SDK avatar preview and empty/unavailable
Studio states. Real UDP checks cover waiting for the last readiness acknowledgment,
loading dropout, re-draft and reconnect. Final client binary starts9f2b505c.

Native review found and fixed the legacy fallback spawning a hero during draft,
scaled-phone preview hit testing, simultaneous gesture/action ownership, scroll
ownership across rebuilt draft panels, and short-phone picker/scoreboard safe areas.
Failed initial runs remain in the evidence as superseded diagnostics.
