# Evidence: VRM1-RENDITION-20261022

All criteria PASS (see evidence.json). Raw unittest output: raw-unittest.txt.

- AC1/AC3: seed-san.vrm (VRM 1.0, 51 mapped bones) and a Studio-shaped synthetic
  VRM 1.0 build exit 0 with five clips; rendition rests facing -Z, `VRMC_vrm` kept.
- AC2: a CC0 roster avatar re-declared as VRM 1.0 (thumbs renamed, turned to +Z)
  bakes the same streams as its VRM 0.x original within 1e-5.
- AC4: rendition SHA-256 of all 15 roster avatars unchanged.
- AC5: `validate_humanoid_profile` OK for all three VRM 1.0 renditions;
  `validate-runtime-vrm` OK for two; seed-san fails the runtime validator on a
  source node scale (`bag_rope_L` [0.99975, 1, 0.99968]) — pre-existing limit.
- AC6: both-extension model exits 2 `build_failed`.
- Full `scripts` suite: 410 tests, 2 unrelated pre-existing local failures.
