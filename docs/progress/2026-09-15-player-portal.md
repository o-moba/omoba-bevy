# Player portal — 0.20.0-rc.1

Implementation worktree: `omoba-bevy-player-portal`, branch
`feature/player-portal-2026-09-15`, based on main e62b728 (0.19.0-rc.8).
The web application is a separate sibling repository, `omoba-web`, version 0.1.0.
The pending iPhone packaging work is not merged into this branch.

## Behavior

The native Profile panel can link a browser using an eight-character code. A bounded
HTTP worker looks up the challenge, and the player approves the displayed trusted
origin and requested scopes. Signing uses the existing local game identity; its
private key never enters the browser or worker. Closing the panel invalidates late
responses. The passport transport validates origin, limits, expiry and redirects.

The Rust Account API exposes existing career profiles, immutable match reports,
projected statistics, friends, nickname and opt-in publication settings. Browser
sessions and portal projection tables have a separate schema/version. The game core
schema remains v1 and only the trusted career worker settles results/rewards.

The game runtime now checks an existing schema without DDL. Run `migrate-career`
or `omoba-account-api migrate` as the migration owner before runtime startup.
`cargo run -p server` retains the normal server default. Limited role setup and
local/HTTPS configuration are described in [Account API operations](../../account-api/README.md).

The standalone server build also explicitly enables Bevy `std`. A process sample
found its no-std fallback spinning in scheduler sleep at roughly 99% of a core.
After enabling OS sleep, the same local setup measured about 0.6% CPU. This is a
startup/build correction, not a change to simulation rules.

## Validation

- Client career: 31 tests passed; shared: 55; passport: 8; API unit: 2.
- Four real PostgreSQL integration scenarios passed, including concurrent browser
  completion, same account identity, expired/cancelled proofs, privacy, idempotent
  friends, late projection, exact u64, session caps and least-privilege grants.
- Actual signed UDP game test passed: durable allocation, legal cast, victory,
  settlement, history and updated rating. Its siege setup accelerates victory and
  does not substitute for a full human match.
- API/server Clippy all-targets with warnings denied passed.
- Next production build, typecheck, BFF tests and real Chrome pairing/profile/history/
  statistics/friends/logout flow passed against the test database.
- Local load, SQL plans, API outage and backup/restore are documented in
  `../omoba-web/docs/VERIFICATION.md` (relative to this repository root).

Local raw evidence and frozen scope are in `.agent/tasks/PLAYER-PORTAL-2026-09-15/`.
Portable portal screenshots are in the web repository's `docs/screenshots/`.
All browser fixtures are explicitly synthetic; there are no product mock fallbacks.

## Release boundary

Code is uncommitted in the feature worktree, not merged/pushed to main. Public
hosting, production secrets, ingress and published downloads have not been changed.
Physical iOS/Android, desktop Safari, manual native confirmation and one continuous
human game-to-web journey still need verification. The release catalog stays empty
until verified public artifacts exist. These limitations mean full public P0 is not
claimed complete. Skins/NFT loadouts and passkeys remain P1.
