# Main consolidation — 2026-10-05

## Scope

The 0.42.0 candidate combines the complete pending playtest-quality iteration, the corrected authored Nexus ring animation and public enemy/ally respawn countdowns, and trailer-v2 capture tooling. Main's newer SDK/account/Studio integration remains in place. The combined version is 0.42.0, protocol 10, standard-kits-5, verdant-confluence-compact-v2, combat-2026-10-05.

The earlier reports that every change had been merged were incorrect: a clean primary checkout did not account for dirty sibling worktrees. This consolidation inventories every local/origin branch and worktree before cleanup.

## Branch and worktree disposition

- Pending `playtest-round-two`, `nexus-respawn` and `trailer-v2` source changes are combined in this candidate. Conflicts were reconciled without restoring the extra upper Nexus rings or dropping current map/tower changes, offline practice tests, or the noninteractive countdown label.
- Old `combat-ux` changes are already implemented and extended in current main. Its added functions/regressions remain present except the old respawning-target test, intentionally superseded by disposable practice targets. Do not reapply this historical patch over current code.
- Most historical branches are ancestors of main or merged through squash PRs. The original Verdant art and SDK account-persistence work are already present; the early controller prototype was superseded by PR #59.
- The unadopted Bevy 0.19 migration and closed environment/refactoring prototypes are archived, not silently enabled. The supported release still uses Bevy 0.18; no production dependencies or CI configuration are changed by consolidation.
- All original Git tips and complete history are preserved in `/Users/wotori/git/ekza/_workspace/backups/main-consolidation-20261005/omoba-bevy-before.bundle`. Dirty source patches include binary diffs and separate untracked-source archives. `branch-audit.json` records each branch's disposition. Deleting branch names does not destroy this recoverable history.
- Unrelated audit/layout work in the local `omoba-ui` repository is outside this game consolidation and remains untouched.

## Verification

Fresh combined-source `make check` passed: formatting, strict Clippy for the complete workspace and the no-QA client, **1,595 Rust tests passed / 39 explicitly ignored**, **171 script tests** and **45 mocked iOS tooling tests**. Independent candidate-asset and Verdant geometry/material/provenance validation passed. No new native device package was built; tooling tests use fixtures. Git diff whitespace checks passed.

Raw results and branch audit are recorded in `.agent/tasks/MAIN-CONSOLIDATION-20261005/`. Current-source GitHub CI (including live UDP harness, PostgreSQL and Android compile) must be checked after push; a manual iOS compile check uses the existing workflow. Previous worktree results are historical evidence, not substituted for these combined-source checks.

## Release readiness boundaries

Source consolidation does not update installed apps or the live server. Read-only SSH verification found `omoba-lobby` active at `/opt/omoba/releases/0.41.0-beta-361543a`. No server deployment, signing, TestFlight upload, release tag or GitHub release is performed by this consolidation.

Before testing 0.42 online, deploy its compatible server and distribute the matching client together. The version handshake intentionally rejects old peers. Remaining release checks include physical iPhone/iPad keyboard/chat stability, 120 Hz/frame pacing, touch combat and reconnect behavior, and final device verification of Nexus motion and both respawn portrait lists. Passing automated checks does not establish that no gameplay bugs remain.

With explicit user approval, old local iOS packages `iphone` and `mobile-0.34.0-15`, `mobile-0.34.1-16`, `mobile-0.35.0-17`, `mobile-0.36.0-18`, `mobile-0.40.0-19` were removed from `omoba-bevy/builds`. The latest `mobile-0.41.0-20` and both Cargo caches were retained.
