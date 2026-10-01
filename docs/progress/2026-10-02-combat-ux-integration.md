# Combat UX integration — 2026-10-02

The audit compared the uncommitted `feat/combat-ux` work with remote main
`dc5b113` and recent PR history. The 11-item package was not previously merged.
The offline screenshot-index repair already existed in main and was retained
without duplication.

## Preserved newer work

- Keep the SDK pin, avatar collection, handheld props and roster visual pipeline.
- Keep the new 3D effects; layer tactical trap outlines over their models.
- Combine combat UX capture modes with the newer roster/handheld modes, assigning
  separate stages to the HUD scenario and preserving release-frame capture.
- Publish version 0.32.0 after main's 0.31.1 and retain its release notes.

## Verification and delivery

Formatting and Python checks run in an isolated integration clone. Compilation,
Clippy, Rust tests, live gameplay harness, database tests and Android compile
validation use existing PR CI: another session holds the primary checkout and
its shared build caches. No third Cargo cache is created.

- [x] Audit main for prior integration and resolve overlaps.
- [x] Publish [integration PR #68](https://github.com/o-moba/omoba-bevy/pull/68).
  Its checks and merge record are the authoritative delivery status; merge is
  gated on green CI. Local format plus 155 script and 44 iOS tooling tests pass.
- [ ] Physical iPad/iPhone touch and human team-match playtest (follow-up).

Earlier native tablet captures in the original task document the pre-integration
build, not a new physical-device test. The primary local checkout and other
sessions' worktrees are not modified by this integration.
