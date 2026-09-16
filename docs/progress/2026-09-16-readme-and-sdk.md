# 2026-09-16 — Open Moba mission and developer entry points

The README now leads with a shared world of avatars and explains why Open Moba
also serves as a practical Ekza integration. Players, artists, modelers,
animators, designers, musicians and game developers have concrete starting
points. MISSION.md and CONTRIBUTING.md carry the same invitation while retaining
the existing rights, license and attribution terms.

## Documentation and command changes

- Link the live Open Moba/Ekza sites, Ekza Space, Bevy SDK, Stellar TypeScript SDK
  and creator protocol. Link the exact Bevy SDK commit consumed by the game for
  Passport details, since the SDK default branch differs.
- Lead local play with native bot practice. Explain the separate external-bot
  matchmaking workflow, LAN phone/PC connection, iPhone packaging and TestFlight.
- Bare `make` now prints `make help`. Help descriptions come from target comments;
  existing target recipes are unchanged. No game or toolchain command is run by
  command discovery.
- Correct stale runbook mode, control and UDP-framing guidance. Mark dated beta
  package notes as historical and distinguish source features from public
  packages, physical-device coverage and configured payment providers.

## Verified boundaries

The pinned Bevy SDK revision still has no declared license grant; that existing
issue is disclosed rather than covered by Open Moba's unrelated license. The
Stellar SDK source is linked without an unverified npm-install claim. Purchased
avatar support remains an opt-in staged Devnet desktop workflow with per-game
rendition approval. No universal compatibility or mobile Passport claim is made.

The companion portal repository URL returned an unauthenticated 404, so the
README links the public Account API guide instead of promising a public source
link. Licensing, dependency versions, production services and game behavior were
not changed. Existing uncommitted Supporter work was preserved.

## Verification

Executed `make` and `make help`; outputs match and list every phony target.
Compared all 17 pre-existing targets through `make -n` against the saved baseline:
recipes are unchanged, including their defaults. Checked LAN/output overrides,
relative document links, anchors and Git-versioned destinations. Independently
reviewed the mission, SDK claims, public URLs and controls against current source;
fixed the portal URL, log location and stale runbook descriptions found in review.

Evidence is retained under `.agent/tasks/README-SDK-2026-09-16/`. No game build or
live server restart is needed for these documentation/help changes. The source
version remains `0.20.0-rc.5`; this update has not been committed or pushed.
