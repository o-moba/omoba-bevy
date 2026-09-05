# Evidence for repository preservation and 3D readiness assessment

The assessment is in `docs/progress/2026-09-05-3d-readiness-audit.md`. Product verdict: **NOT READY for unattended external playtests**. This evidence does not mark earlier unfinished tasks or failing product checks complete.

- Accumulated user changes preserved in snapshot `52aa773`; raw/snapshot-preservation.json checks every initial untracked path against its tree.
- Narrow follow-up in dedicated worktree/branch repairs SDK lock source, minion packaging and the 16-avatar offline roster, with workspace patch 0.17.1 and release docs. No production Rust source changed after the snapshot.
- Clean locked build, format, Clippy, hero/boss/minion asset validators and six existing session-flow scenarios pass.
- Initial workspace + remaining-package commands: 203 Rust tests pass, 1 full 5v5 fixture fails. Raw output remains in raw/test-workspace.log and raw/test-remaining.log. Probe raw/5v5-observations.json proves complete snapshots up to 8113 bytes, with an invalid local-only avatar shortened to null and no server-send errors. The test now derives a valid shipped avatar and asserts replication; targeted rerun passed at 8193 bytes (raw/udp-datagrams-fixed.log). Final full-workspace rerun passed **204 tests, 0 failed** (raw/test-workspace-final.log).
- Real release 1v1 probe confirms debug advantages accepted and duplicate/reconnect Join resets position/resources. Full state and server binary SHA are in raw/session-contract-observations.json; this was an observation probe, not a product PASS test.
- Sprite/readability asset validators fail on missing Orchard sheets. Full World2D validator fails because ignored task provenance is required in a clean checkout; its six negative self-tests pass.
- Native Models3d startup stayed alive for 20 seconds with autojoin, selected hero and minion models loaded; logs still contain optional sprite/SDK asset errors. No screenshot, manual controls or FPS claim is made.
- Raw check exit codes, hashes and limitations are recorded in evidence.json. Independent verifier will judge the current tree, rerun representative checks, and write verdict.json separately. Git delivery is verified against origin before completion.
