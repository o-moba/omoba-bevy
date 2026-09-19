# Ekza rendition builder — 2026-09-19

Goal: let the Ekza registry build Omoba's rendition of any creator's avatar without
copying Omoba's rules into the registry.

## Decision
A rendition profile's requirements and its builder belong to the game. The registry
stores the profile document and runs the builder as a configured external command,
the same way it already runs its USDZ converter. A new game is a new command, not a
registry change. Recorded in the Ekza umbrella `adr/0001-web2-first-asset-lifecycle.md`
(section 11) and `ARCHITECTURE.md`.

## Changes
- `scripts/ekza_build_rendition.py`: thin command around the existing
  `ekza_publish.build_omoba_rendition` and `validate_omoba_profile`. Contract is in
  the module docstring. Progress output of the shared build step goes to stderr so
  stdout stays one JSON line.
- `scripts/test_ekza_build_rendition.py`: contract tests (content-addressed output,
  idempotency, typed issues for non-glTF, non-VRM and unbuildable sources, limits
  equal to the published profile).

## Checks
- `python3 scripts/test_ekza_build_rendition.py`: 4 tests, OK.
- Real source `robert-source.vrm` (VRM 0): built in 0.23 s, 1,885,072 bytes, 5 clips
  on 52 bones, identical hash on a second run.

## Remaining risk
- The output is checked by the Python mirror of `omoba_passport::validate_humanoid_profile`,
  not by the Rust function itself. A parity test is planned (Ekza plan task T3.5).
- Retargeting assumes a T-pose rest; an A-pose VRM builds but may look wrong. Visual
  acceptance stays with the project owner who approves the submission.
