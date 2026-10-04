# Movement reversal investigation — 2026-10-04

## Reproduction and change

A captured touch gesture ran forward beyond the stick base, then reversed without lifting to the opposite visible knob limit. The old input required the entire base radius for full speed, although the visual travel was only 70% of that radius. With the 16% dead zone, that reverse gesture requested `(0.70 - 0.16) / (1.0 - 0.16) = 0.642857` speed. The real local ECS movement test measured 3.214 units/second instead of 5.0, failing on the first reverse frame. Equal full-radius inputs were already symmetric; there is no intentional backward speed penalty.

Full-speed input now uses the same 70% travel limit. The knob follows actual finger displacement, capped at that limit, instead of displaying a second compression of the already-remapped movement magnitude. The inner range retains smooth walking and the existing dead zone. Controller analog input, camera follow, collision, gameplay speed, network protocol and server tuning are unchanged.

## Verification scope

Focused headless checks use the actual touch capture and local movement systems at one phone viewport, 852 × 393 logical points. Forward/reverse checks cover axial and diagonal directions at 30/60/120 simulated FPS, including the first reverse frame, release, slow, haste and root/stun. Separate real offline practice and server movement regressions check authority symmetry. This is deterministic simulation evidence, not a physical iPhone recording or proof of every possible cause of the user's observation.

Current verification results and independent review are recorded in `.agent/tasks/MOVEMENT-REVERSAL-20261004/`. No additional native GPU build or screenshot sweep is required for this response-curve fix. Existing 0.38 work is retained, with no commit, deployment or TestFlight upload in this task.

Focused current-source verification passed: 32 touch-control tests, 18 local movement/navigation tests, 1 offline authority regression and 8 server movement tests (59 total). Formatting and whitespace checks passed. The new reversal test was observed failing before the fix and passing afterward. A full commit gate and physical phone validation belong to the subsequent release step.

Fresh independent verification passed AC1–AC4 and11 targeted direct-binary checks with no findings.
