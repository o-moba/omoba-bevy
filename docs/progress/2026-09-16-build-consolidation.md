# Build tooling consolidation — 2026-09-16

The primary repository now includes the physical iPhone builder, development
profile/signature validation, thirteen packaging tests, and installation
instructions previously left uncommitted in the iPhone worktree. Existing
`make play` / `make practice` and supervised desktop launch improvements were
already integrated; the older local-play working changes must not overwrite them.

Use `make iphone-check` and `make iphone` from the primary checkout, after
installing Xcode and the Rust `aarch64-apple-ios` target. The default local package
is retained in `builds/iphone`; the compiler cache is `target/iphone-cargo`.
Use a fresh `IPHONE_OUTPUT` for subsequent packages. Signing uses existing local
development credentials and is documented in `mobile/ios/README.md`.

The reviewed prop inventory and provenance checks were also transferred, fixing
the stale source inventory without changing model bytes or gameplay version.
No production dependency, CI, deployment or credential was changed.

## Current verification

- 28 asset gate tests, 13 physical iPhone packaging tests and 7 legal-notice tests
  passed against an export of the current sources with the local edits applied.
  Historical denied-content fixtures came from the existing immutable Git commit.
- All 11 local-launch tests passed with real loopback sockets permitted.
- The canonical `0.20.0-rc.3` client compiled for physical `aarch64-apple-ios`
  using Rust 1.93.1 / Xcode 26.2 / iOS 15 minimum. A locally cached matching iOS
  standard library supplied a temporary sysroot; ordinary setups use rustup.
- The resulting `.app` was signed with an existing development certificate and
  profile; signature verification passed. The IPA was checked byte-for-byte
  against the signed bundle. Packaged assets passed the candidate content gate.
- Installation and gameplay on a physical phone remain unverified.

The earlier iPhone `.ipa` and `target/mobile` directory were already absent when
this audit began. The replacement is under `builds/iphone-0.20.0-rc.3`, outside
disposable compiler caches. Signed artifacts contain personal provisioning data
and stay local/ignored; build scripts and instructions are versioned.

## Workspace cleanup boundaries

Ten clean merged working copies were removed after their ignored local evidence
was compressed and verified by SHA-256. The stopped portal test PostgreSQL 18
cluster was preserved in a complete, byte-verified offline archive before its
expanded data directory was removed. Browser/npm download cache was disposable;
web source, node_modules, screenshots and verification records were retained.

The five explicitly protected checkouts and the dirty local-play checkout remain.
The art branch still has one commit outside main. Avatar-roundtrip's 42 dirty
entries are deleted historical audit files; its HEAD is already in main. The
separate web repository has a local committed main and no remote configured.

Some older worktrees and asset backup directories are owned by root. Ordinary
cleanup and noninteractive sudo cannot remove them. An owner-run, exact-allowlist
cleanup wrapper rechecks every source hash, archive, clean status and merge
ancestry before deletion. Until that step, the full raw `client/assets` source
gate still sees archived legacy/downloaded models; the tracked packaged inventory
and clean-checkout tests pass. No protected worktree is on that cleanup allowlist.

Detailed local inventory, archives, restoration instructions and command logs:
`.agent/tasks/WORKSPACE-CLEANUP-2026-09-16/`. These local archives are not a remote
backup and must be kept until outstanding historical changes have been reviewed.
