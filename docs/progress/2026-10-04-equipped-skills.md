# Equipped skill bindings and admitted control — 2026-10-04

Version **0.40.0 / protocol 9 / standard-kits-4** implements R16 and R17 on
`feat/playtest-round-two`. **Local validation passes.** This note does not
claim deployment, TestFlight availability or completion of the full visual
uplift plan. The frozen contract and raw evidence belong to
`.agent/tasks/EQUIPPED-SKILLS-20261004/`; the maintained behavior reference is
[equipped-skills.md](../equipped-skills.md).

## Implemented

The shared resolver accepts four unique compatible skills in arbitrary
bindings, including four ultimates. Authored roles still determine unlock
levels and bot upgrade priorities; the physical slot holds rank/cooldown state.
The shared `EquippedSkills` view supplies metadata to common authority and
desktop/mobile/controller input, aim, HUD, cards and upgrades. Absent recipes
have a documented preset/legacy fallback; malformed or mismatched supplied
recipes fail closed, including presentation.

Runtime effects, orb damage, Brittle/Concussion receipts, action animation and
recasts now preserve actual skill/binding identity. Follow-up costs moved from
the Stormfist core check to catalogue metadata: its three recast skills cost
25 across cores; other current skills retain free follow-ups. Negative resource
tests pin rejection without consuming a follow-up. Sandbox unlock overrides
remain available without bypassing upgrade points or rank caps.

Admitted control handling is extracted into a common helper with explicit
root/stun/Bluff/charm policies. The behavior change is separately tested:
admitted Bluff consumes eligible Brittle once through existing damage and
receipt processing. Rejected control preserves the mark and recall; existing
cast-payment rules and the documented unstoppable, player-charm and forced
displacement exceptions remain intact.

The existing opt-in local Combat Test configuration accepts an optional
validated actor recipe. Configurations apply atomically; accepted recipe edits
clear old skill state while retaining actor identity and request sequencing.
This is local authoring infrastructure, without a public class editor, account
recipe persistence or public matchmaking admission change.

## Final verification

The frozen AC1–AC5 contract passes on the current source. Raw results are in
`accepted-gate.log`, `harness-all.log` and `native-hybrid/` in the task directory:

| Check | Current result |
| --- | --- |
| Shared tests | PASS: 124 |
| Common tests | PASS: 131, including 12 equipped/control regressions |
| Server tests | PASS: 322; 3 ignored |
| Full client suite on final source | PASS: 921; 1 ignored |
| `make check` and final strict checks | PASS: formatting, workspace/all-target Clippy, no-QA client Clippy, 1,547 Rust tests (39 ignored), 201 Python tests |
| Fresh real-server harness, including mixed recipes | PASS: 24 unit and 26 integration scenarios over real local servers |
| English 852×393 affected-HUD capture | PASS: hybrid HUD and visibly rendered Last Spark hold card; synthetic presentation, separate from server proof |
| Independent source and image reviews | PASS; recast-cost, invalid-motion and local protocol-admission findings resolved |

No physical iPhone run, release package, deployment or TestFlight upload is
part of this verification. Previous 0.39 gate counts and screenshots remain
historical and do not certify the changed 0.40 source. R16/R17 are complete
in [the canonical tracker](../REFACTORING.md#post-programme-architecture-follow-ups--2026-10-04).

The first fixture startup timed out and a first-frame held-card capture was
rejected because its view model was ready before its text was painted. Both
are retained as failed evidence. The corrected fixture waits for visible card
and text bounds; its card reads Last Spark, Unlocks at level 6, 40 mana and
42.2s cooldown on Q. The successful earlier HUD frame is reused unchanged.
The only source change after capture is a Clippy-equivalent QA boolean rewrite;
production code and appearance are unchanged. Code, binary and capture hashes,
provenance and resolved findings are recorded in the task artifacts.

Under the prior old-build cleanup instruction, obsolete client-only build
artifacts were removed while Cargo was stopped; roughly 47 GiB was reclaimed
across two passes. Current artifacts, dependencies and cache B were retained.
No new dependencies, infrastructure changes, commits, pushes or deployments
were made in this architecture task.

## Remaining boundaries

The core still owns attack/resource identity. Duplicate skills are rejected;
overlapping singleton runtime states are not generalized into independent
user-defined instances. The legacy skills remain their own class-owned path.
Full phase-based movement/animation, public construction, recipe storage,
device performance and balance remain separate work. R18–R23 and the existing
release/playtest checklist remain open.
