# SDK Twenty avatar collection

Added a reproducible local developer collection of 20 new avatars from the owner's Open Source Avatars archive. Selection hashes and provenance are versioned; generated models and SDK caches stay outside the shipped game assets.

The existing rendition builder supplies the five base animations. The actual Ekza SDK performs HTTP catalog discovery and model installation; the existing `ekza://` asset reader and independent game-server admission remain in use. No SDK version or production dependency change was needed.

Implementation: `assets-src/sdk-avatar-pack/selection.json`, `scripts/sdk_avatar_pack.py`, the `avatar-pack-install` CLI, focused catalog boundary tests, and an opt-in native QA runner. See [build/play instructions](../../assets-src/sdk-avatar-pack/README.md).

Initial evidence: all 20 renditions passed Assimp and runtime humanoid validation; all 20 cold SDK installations matched hashes; all 20 received normal server admission. Unknown and withdrawn IDs were rejected. Source portraits were visually reviewed. Native preview/gameplay verification is pending: the first run timed out while the Mac was locked. Task artifacts are under `.agent/tasks/SDK-AVATAR20-2026-10-01/`; this note must not be read as a completed visual sign-off.

Public Studio/Space publication and account-library grants are not implemented by this local development pack. Existing server catalog refresh uses a five-minute TTL; withdrawal was tested on a fresh server. Extreme body proportions, tails, wings and fingers still require actual visual review under skill motion.

Fresh nonvisual rerun: all 20 SDK installations and admissions passed again, including both rejection cases. Formatting and Clippy (QA and production configurations) pass; 1,354 Rust tests pass with 39 ignored, plus 155 script tests and 44 iOS tooling tests. The initial gate caught a missing QA screen-registry entry; it was added and the full script suite rerun. Native visual status remains blocked, not passed.

On the owner's subsequent commit/push/merge request, a fresh full `make check` completed successfully. The production publication runbook records live public API 404s and the required operator/creator/mobile steps. This merge preserves the incomplete native visual status; no production deployment or avatar publication is implied.
