# Verification fixes

- Initial client compile: Bevy 0.18 removed World::iter_entities; the custom GLB fallback's drawable guard used that unavailable API. Replace only the guard with supported component/archetype inspection, then recompile and test.

- First executable client test pass: 246 passed; seven existing loopback tests could not bind sockets inside the sandbox (Operation not permitted). Rerun with authorized local networking outside that restriction; no gameplay fix is indicated.

- First native launch found a PostUpdate ordering cycle: label placement requested both after global transform propagation and before UI layout, while Bevy UI layout precedes that propagation. Order after camera projection update/before UI layout and use the current root camera transform. Repeat native capture and full tests.

- Independent review: replaced hero-distance cosmetic culling with camera-view culling so distant minimap focus works; conflicting idle/walk aliases now retry built-in clip heuristics before discarding an animation set.

- Capture verifier initially compared full event JSON exactly, incorrectly rejecting f32 position round-trips (1.05 versus 1.0499999523162842). Numeric fields now use a strict 1e-5 absolute tolerance; typed identities/style/slot/death remain exact, with rejection regressions. Original failed report preserved.
- Full workspace regression exposed a jungle test's fixed total-XP assumption: the mixed wave awards existing legitimate team minion XP before the second camp kill. Retain exact camp reward proof while accounting for independently observed enemy-minion deaths.
- 2D native combat and event/label proof passed, but package checking found the pre-existing orchard-comet-centaur manifest entries refer to two absent PNGs. Investigate existing original artifacts and repair actual package content; do not ignore load errors or substitute mislabeled art.

- Final client suite exposed one remaining old fixture asserting ten physical sprite sheets. Updated it to require nine shipped sets, no draft-file load, and complete six-state rendering for every stable identity through the declared fallback. Production source unchanged by this fixture correction.

- Strict all-target Clippy requested three idiomatic Option early returns in changed damage sinks, and caught two pre-existing test-only style lints. Applied equivalent `?` early returns and minimal fixture cleanups; rerun strict Clippy and affected tests.

- Final resolution: all 453 workspace tests, strict Clippy, current native build, 112 affected-source rechecks, three current-server UDP checks and all six native capture scenarios pass. The incomplete Orchard art remains explicitly marked pending with a declared render fallback. Physical mobile and human balance tests remain outside this verification.
