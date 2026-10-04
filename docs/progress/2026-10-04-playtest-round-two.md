# Mobile playtest round two —0.37.0

Implementation is on `feat/playtest-round-two`, based on main4a7b65a1. Production remains0.35.0; the last uploaded iOS client is0.36.0(18). This task has not deployed or uploaded a release.

See [current features](../features.md), [economy model](../economy.md), and the local `.agent/tasks/PLAYTEST-ROUND2-20261004/` specification and evidence. Local verification is complete: 1,478 application/workspace tests and 46 additional network-harness tests passed, with 39 explicitly ignored external-infrastructure/report cases. Strict workspace/all-target and production-without-QA clippy, formatting, 155 Python script tests and 45 iOS helper unit tests passed. The helper tests use mocks; they did not upload or install a release.

Protocol6 requires a paired server/client rollout because the inventory enum gained item types that old clients cannot deserialize. No production dependencies, secrets or deployment configuration were changed.

Visual scope is English852×393 landscape on the native desktop renderer. It is not physical iPhone touch, suspension or120Hz performance certification. FPS instrumentation now exposes native maximum/callback rate separately from actual app cadence and the selected cap; sustained120Hz remains workload/device/OS dependent.

Implementation review caught and repaired two controller interactions: AI combat refreshed a retained hero's transport timestamp, and bot walking used its pre-skill origin after mobility casts. Authenticated reclaim now distinguishes a detached controller from a live client; bot navigation restarts from its actual position. A conservative displacement guard also prevents autonomous mobility skills from entering unsupported turret range.

Build hygiene: reused primary cache A. Under the existing obsolete-cache cleanup request, removed identified orphan compiler objects from earlier tasks, preserving current binaries and all retained release archives; detailed deletion records are in the task raw artifacts. No third cache or `cargo clean`.

Native evidence contains 11 frames across combat/FPS settings, draft/countdown, full two-team results, real offline shopping with component credit, and four recovery states. The disconnected Home action follows the measured banner height; teammate controls stay hidden while a modal is open. The first-match guide is dismissed through its real button in the fixture. Independent review and direct reruns cover shielded trap audio, the real UI recognizer and signed local UDP reclaim after a simulated240-second disconnect.

Final review also caught silent shielded trap triggers. Explicit authoritative activation receipts now produce one trap cue without inventing HP damage or damage statistics, and the normal fog filter removes hidden targets. The signed UDP test's intervening AI action refreshes liveness without actually damaging a target; physical app suspension and production database restart were not exercised.

The economy now has16 items with three tiers, full component credit, distinct speed/critical/lifesteal options and a500g hero bounty that decreases for repeated unanswered defeats. The reproducible model targets roughly3–4-minute middle-tier and8-minute major items for solo-lane farming. Respawn starts at5 seconds and grows to35 seconds; these are initial playtest values, not competitive balance certification.
