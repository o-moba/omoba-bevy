# OMOBA Dev Xcode readiness — 2026-10-14

Source baseline: game `dev` at `89ed2c7`. Changes add a shared Omoba Dev scheme
(Debug Dev / Release Dev), pin its identity/endpoints over stale local settings,
select the same scheme in iOS release tooling and share the primary Cargo cache.
The production scheme remains Omoba. No runtime/gameplay or server change.

## Verification

- 52 iOS Python tests and 41 release tooling tests passed with Python 3.14.
- Xcode 26.2 resolved both schemes / all four configurations correctly, including
  stale local contour/server overrides; project plist validation passed.
- Release dry-run selected dev identity and endpoints.
- Current source contract probe against `lobby.dev.omoba.io:4100`: compatible,
  protocol 11, catalogue `standard-kits-7`, gameplay `wildspark-2026-10-09-reference`,
  geometry `verdant-confluence-compact-v2`.
- Registry read-only acceptance: `HOST-OK` and `DEV-OK`.
- Xcode archive completed successfully: `builds/Omoba-Dev-check.xcarchive`,
  `OmobaDev.app`, `space.ekza.omoba.dev`, display name `OMOBA Dev`, version 0.44.0.
  The archive retains a matching dSYM (UUID checked); code signing was disabled.
  This checks the modified working source based on `89ed2c7` (source_dirty=true),
  not a signed release candidate. No signed/exported/uploaded build is claimed.

## Remaining operational gates

Owner approval is required for Apple signing/profile setup and upload, Vercel
branch tracking and any deployment changes. App Store Connect requires sign-in
in the available browser; existence of the dev app record is unconfirmed.
TestFlight processing, tester assignment, installation, two-client gameplay,
disconnect/resume and the next match remain unverified. Dev supports one match
at a time; do not treat a healthy host or compatible manifest as a full playtest.
The old `v0.44.0-dev.1` draft targets `1f4ad10` and lacks the new profiles/app.

Local proof: `.agent/tasks/DEV-READY-2026-10-14/`.
The matching registry branch corrects stale tester/runbook claims. No production
operations, secrets, CI/CD settings, signed releases or external publication.
