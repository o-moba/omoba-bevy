# Combat presentation evidence

**Result: PASS.** Version `0.19.0-rc.4`, base `6d46076`. The containing commit carries the implementation, contributor documentation and this evidence.

## Acceptance criteria

| Criterion | Result | Evidence |
| --- | --- | --- |
| AC1 | PASS — Class-specific procedural arrows, arcane/holy bolts and crescent; unchanged authoritative travel and hero tuning. | `workspace_tests`, `current_udp`, `native_capture_matrix` |
| AC2 | PASS — Two melee and one caster per existing wave; role silhouettes, one release per sequence, actual ranged arrival, symmetric lanes and unchanged reward/cadence contracts. | `workspace_tests`, `current_udp`, `native_capture_matrix` |
| AC3 | PASS — Typed positive actual-damage receipts; overkill/protection/heal/reset handling, bounded history and labels, deduplication, 3D/2D camera projection. | `workspace_tests`, `affected_tests`, `current_udp`, `native_capture_matrix` |
| AC4 | PASS — Versioned packaged registry with class/action and identity overrides, GLB/PNG presentation, clip aliases, validated bounds and immutable fallback; contributor guide supplied. | `workspace_tests`, `strict_clippy`, `native_capture_matrix` |
| AC5 | PASS — Regression tests, current-server UDP proof and six real native scenarios pass; phone-preview/human-balance limitations explicit. | `workspace_tests`, `affected_tests`, `current_udp`, `capture_verifier`, `native_capture_matrix`, `native_build`, `format`, `strict_clippy` |
| AC6 | PASS — Version 0.19.0-rc.4, changelog/features/session guide, raw evidence and published frames included in this task commit; four unrelated original cinematic files excluded. | `format`, `strict_clippy` |

## Checks

The full workspace passed **453 tests, zero failed or ignored**. The later 112 affected tests and three UDP tests are rechecks, not extra unique tests. Cargo used the existing native target cache, disabled client debug/incremental output, and the rebuilt server via `HARNESS_SERVER_BIN`. Socket tests and native rendering ran with local networking permitted.

- `workspace_tests`: PASS — `cargo test --workspace --locked`. [Raw log](raw/workspace-tests-pass.log.gz).
- `strict_clippy`: PASS — `cargo clippy --workspace --all-targets --locked -- -D warnings`. [Raw log](raw/clippy-final.log).
- `format`: PASS — `cargo fmt --all -- --check`. [Raw log](raw/format-final.log).
- `native_build`: PASS — `cargo build -p client -p server --locked`. [Raw log](raw/native-build-current.log).
- `affected_tests`: PASS — `cargo test -p server -p omoba-passport --locked`. [Raw log](raw/affected-tests-final.log.gz).
- `current_udp`: PASS — `cargo test -p harness --test combat_feedback --locked -- --nocapture`. [Raw log](raw/udp-current.log.gz).
- `capture_verifier`: PASS — `python3 scripts/capture_combat.py --self-test`. [Raw log](raw/capture-verifier-tests.log).
- `native_capture_matrix`: PASS — `python3 .agent/tasks/COMBAT-FEEL-2026-09-12/run-captures.py`. [Raw log](raw/capture-matrix-final.log).

## Native evidence and provenance

Six scenarios each produced three real Bevy screenshots: ready, projectile flight and confirmed impact. Four classes in 3D, an 844 × 390 mobile UI preview, genuine sprite 2D and an ordinary mixed wave are covered. The independent UDP observer verifies projectile/event identities against the labels and rendered projectile roots. All six reports have `capture_pass: true`, no runtime errors and no synthetic damage.

The final client SHA-256 is `2ee67195c5584a91a841ef76a986f3f2552f99039c4c797c069c4a86ee9b3f74`, identical to all captures. Capture server SHA-256 is `0ae757606bf858233911bdb70b6b7449d5d389ed8f26de511191499c9464e496`; only three equivalent Option early-return cleanups followed. Rebuilt final server `a9b95c265d41eda364726964f57bbf29dd0739c5ee434179deae27eaf6ac34d9` passed its 105 tests and all three real UDP combat tests.

[Published frames and hashes](../../../docs/progress/2026-09-12-combat/captures.json); [session report](../../../docs/progress/2026-09-12-combat-presentation.md); per-scenario reports, client/server logs and lossless gzip observer streams are retained under `raw/published-proof/`. Champion setup uses initial developer placement and ordinary scripted commands; mixed waves use production routes and spawning.

## Limits

- No physical Android/iOS test or signed mobile package.
- Native input is scripted; champion initial placement uses a developer fixture. Normal-wave scenario uses production spawning/routing.
- No human competitive balance or mobile performance certification.
- Cosmetic assets are packaged with the client; no remote skin download, marketplace, entitlement or arbitrary executable plugin system.
- Orchard Comet Centaur animation art is still pending; nine active complete sprite packs plus an explicit stable-ID fallback.

## Repository preservation

The original main checkout has four unrelated untracked cinematic files (`build_cinematic.py`, `encode_cinematic.py`, `render_cinematic.py`, and `docs/progress/2026-09-09-verdant-cinematic.md`). They are excluded from this worktree and task commit. Main integration uses fast-forward operations and a non-forced push. No production dependency or deployment configuration changed.
