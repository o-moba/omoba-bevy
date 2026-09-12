# Map customization verification

Task: `MAP-CUSTOMIZATION-2026-09-12`. Base: `e3e30fb`. Version: `0.19.0-rc.5`.

## Acceptance

| Criterion | Result | Evidence |
| --- | --- | --- |
| AC1 | PASS | Validated default and custom startup, exact 8/10 identities and combat overrides; invalid numeric/schema/geometry/placement failures; process-pinned profile and real rematch reconstruction. |
| AC2 | PASS | Tier damage/base gates, live movement collision, both-team melee strikes on offset towers and caster flight, actual minion targeting, reachable marched-lane placement and replication. |
| AC3 | PASS | Shared canonical Verdant layout, full-radius collision clearance and explicit geometry mismatch admission rejection. |
| AC4 | PASS | 942 real authored props (236 solids); actual A-B-A-B loaded mesh replacement and bounded ownership/cache; same-ID structure rebind,2D atlas and cue updates, missing asset fallback. |
| AC5 | PASS | Fresh 481 Rust tests,30 verifier checks, strict lint/format and five current-binary native scenarios with 19 exact-tick UDP-backed captures; fixed QA camera and 3025 actual2D terrain tiles. |
| AC6 | PASS | Version 0.19.0-rc.5, changelog/features, contributor example and guide, session note and compact raw proof prepared in the task commit; original four cinematic files preserved. Git delivery verified separately after commit. |

## Fresh current-source checks

- **481 Rust tests pass**, zero failed or ignored: `cargo test --workspace --locked`. [Raw log](raw/verified-workspace-tests.log.gz). Includes 270 client, 113 server, 40 shared and all live UDP harness suites. Earlier focused runs are rechecks, not additional unique tests.
- Strict `cargo clippy --workspace --all-targets --locked -- -D warnings` passes. [Raw log](raw/verified-clippy.log.gz).
- `cargo fmt --all -- --check` passes. [Raw log](raw/verified-format.log.gz).
- Current client/server native build passes. [Raw log](raw/verified-native-build.log.gz).
- Thirty capture-verifier negative/positive unit checks pass. [Raw log](raw/capture-verifier-tests.log.gz). These synthetic verifier fixtures are not native runtime evidence.

Cargo reused the existing native cache with client debug output and incremental compilation disabled. The live harness used the freshly rebuilt server via `HARNESS_SERVER_BIN`; local UDP sockets were permitted. Exact environment, hashes and command records are in [evidence.json](evidence.json).

## Native proof

Five real macOS native scenarios pass: default desktop 3D, two-tier desktop 3D, two-tier phone 3D (844×390), two-tier desktop 2D, and two-tier phone 2D. Nineteen captures match exact authoritative UDP epoch/round/tick, identities, configured positions/max HP and protection. Three 3D scenarios also prove all 942 authored props are drawable and repeated lantern→shrub→lantern→shrub mesh fingerprints return without accumulating owners/assets. Both 2D scenarios prove 3025 loaded terrain-atlas tiles are inside camera depth and ground is on-screen.

[Compact original records](raw/published-proof/captures.json) retain JSON and deterministic gzip copies of unmodified client/server logs and independent UDP snapshots. [Seven published PNGs](../../../docs/progress/2026-09-12-map/captures.json) are exact copies; hashes of all 19 locally retained originals are recorded. [Direct visual review](raw/manual-visual-review.json) confirms visible scene geometry, extra mid towers, prop substitution and separate desktop/phone controls. Camera movement and cosmetic swaps are scripted; structures are live authoritative objects.

## Corrections and limits

[Problems and fixes](problems.md) records strict-schema, collision broad-phase, real glTF hierarchy, presentation lifecycle, fixture pathfinding, effective tower order, walked-lane placement and QA camera findings. The first full test failure and old focused failures are retained in compressed logs. Preliminary 2D captures were visually rejected because the QA camera clipped terrain; final verified captures supersede them.

Terrain, roads, base pads and static collision retain the versioned Verdant geometry. This adds object tuning and cosmetic registries, not a terrain editor. Gameplay edits need a server restart; presentation JSON loads at client startup. New 2D art needs corresponding atlas metadata/rebuild. No physical Android/iOS, manual touch play, performance or human balance claim is made.

## Delivery

The task commit includes the implementation, version/changelog/features, contributor guide/example, session note and proof. Four unrelated cinematic source files are excluded and their original hashes retained in [preservation record](raw/preserved-cinematic-files.json). Main integration uses a fast-forward and a non-force push; the post-commit local `raw/git-delivery.json` records resulting local/remote hashes.
