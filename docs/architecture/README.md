# Architecture

This folder maps the current architecture of OMOBA, an open-source MOBA. It covers the game workspace in this repository and the player portal (`omoba-web`). Engineers and coding agents can cite it when planning changes. The pages explain how the parts fit together and why. The details stay in the older documents linked below, which remain authoritative.

**Snapshot:** game `dev` at `6d34f98` (PR #107), portal `dev` at `604bc34` (PR #15), 2026-10-24. Each page ends with **Open questions**: anything the code or docs could not confirm.

## Documents

| Page | What it covers |
| --- | --- |
| [system.md](system.md) | Processes and services, server roles (standalone, lobby, match worker), matchmaking, communication paths and ports, contour × market build axes, hosts (production, dev VPS, planned RU), where state lives, fail-closed guards |
| [identity-and-accounts.md](identity-and-accounts.md) | Device key as the account, career handshake, signed requests, website pairing, web sessions, device enrollment and recovery, Sign in with VK ID, Ekza links, signature domains, keyed hashes, rate-limit buckets |
| [protocols.md](protocols.md) | Every cross-boundary contract: UDP wire protocol and framing, compatibility preflight, public admission and signed commands, lobby→worker handoff, Account API HTTP contract, portal BFF, Ekza contracts, market switches, change checklist |
| [data.md](data.md) | PostgreSQL layout (career core v3, portal v7), migrations, roles and grants, settlement outbox, match root, retention, advisory locks, secrets bound to data, client settings folder, backup and restore |
| [build-and-release.md](build-and-release.md) | Cargo workspace, compile-time settings and the environment scrub, `scripts/release.py` flavors, per-platform packagers, versioning, CI workflows, portal build and deploy |
| [methodology.md](methodology.md) | Branching (gitflow on `dev`), definition of done, local gate vs CI, testing layers, task proof loop, release docs, localization rules, UI verification budget, build-cache policy, working with coding agents |
| [decisions.md](decisions.md) | ADR log: ADR-001 onwards, most recorded retrospectively, plus proposals not yet decided |

## Authoritative detail lives elsewhere

These pages link to the documents below instead of repeating them. When they disagree, check the code. Then fix whichever is wrong.

- [../ARCHITECTURE.md](../ARCHITECTURE.md): code map (crates, client frame, server tick, protocol rules, adding content)
- [../markets.md](../markets.md): markets, feature switches, VK ID user flow, RU release runbook
- [../connection-profiles.md](../connection-profiles.md): contours, connection profiles, per-profile state
- [../../RUNBOOK.md](../../RUNBOOK.md): running servers and the client locally
- [../RELEASING.md](../RELEASING.md) and [../release-compatibility.md](../release-compatibility.md): release procedure and compatibility gate
- [../../account-api/README.md](../../account-api/README.md): Account API configuration, migrations, projection, backup (wire contract: [openapi.json](../../account-api/docs/openapi.json), which currently lags the code)
- [../../career-store/migrations/postgres/README.md](../../career-store/migrations/postgres/README.md): career schema rules
- [../supporter.md](../supporter.md), [../public-mvp.md](../public-mvp.md), [../party.md](../party.md): Supporter and devices, public service limits, parties
- [../../ops/vps/README.md](../../ops/vps/README.md), [../../ops/vps/dev/README.md](../../ops/vps/dev/README.md), [../../ops/vps/ru/README.md](../../ops/vps/ru/README.md): host runbooks
- `omoba-web` `docs/vercel.md`, `docs/markets.md`, `docs/TESTING.md`: portal deployment, markets and tests

**Historical:** [../ARCHITECTURE_CURRENT.md](../ARCHITECTURE_CURRENT.md) and [../ARCHITECTURE_REPORT.md](../ARCHITECTURE_REPORT.md) are Russian snapshots from 2026-09-25. They predate connection profiles, markets, VK ID and the `common` crate. Keep them for history. Do not cite them for current behaviour.

## Glossary

| Term | Meaning |
| --- | --- |
| Contour | `prod` or `dev` build line (the portal spells them `production`/`dev`). Dev builds are the separate **OMOBA Dev** app |
| Market | `global`, `ru` or `cn`: compile-time preset of the features `ekza`, `supporter` and `vk_sign_in` (`shared/src/market.rs`) |
| Flavor | One `release.py` build: contour × market plus feature and endpoint overrides |
| Standalone / lobby / match worker | The three roles of the one `server` binary (`OMOBA_SERVER_ROLE`) |
| Device key | The installation's Ed25519 key in `career_identity.json`. It is the account |
| Settings folder | `omoba-bevy[-<market>][-<contour>]`. Each folder has its own device key |
| Account API | `omoba-account-api`: loopback HTTP service over the career database |
| Portal / BFF | `omoba-web`: the Next.js backend-for-frontend. The browser holds only HttpOnly cookies |
| Portal ingress key | `X-Omoba-Portal-Key` (env `OMOBA_ACCOUNT_INGRESS_KEY`). The ingress requires it on non-native `/v1/*` routes |
| Ekza Registry / passport | External Ekza services for avatars, weapons, library and tickets. Used only in `ekza` builds |

## Known drift in older docs

Fix these in the older documents. Do not copy them into new pages.

- [../../account-api/docs/openapi.json](../../account-api/docs/openapi.json) omits the device, recovery-code and Supporter-recognition routes and still carries `info.version` `0.21.0-rc.1` ([protocols.md › Open questions](protocols.md#open-questions)).

The drift found while writing this set (portal schema version in `supporter.md`
and the Account API README, the DEV activation schema check, the VPS client
paragraph, the `common` crate in `ARCHITECTURE.md`, the CI trigger in
`CONTRIBUTING.md`) was fixed in the same change.

## How to keep this current

- **Same PR.** A change that alters something these pages describe updates the relevant page in the same pull request. Examples: a contract, a port, a role, a schema version, a build axis or a gate.
- **Map, not copy.** Put details in the authoritative doc or the code, and link to them from here. Add each fact once, on the page that owns it.
- **Decisions.** A change that sets or alters a contract, trust boundary, build or release topology, process rule, licence boundary or consequential pin gets an ADR. Append the next `ADR-NNN` to [decisions.md](decisions.md) in the same PR, using the fields described in [methodology.md](methodology.md#recording-an-architecture-decision). Never renumber or delete an ADR. Supersede it and update its status.
- **Open questions.** When one is answered, move the answer into the page body, or into an ADR, and delete the question.
- **Tracking.** `docs/*` is gitignored with a whitelist; `.gitignore` lists `!docs/architecture/` and `!docs/architecture/**`, so new pages here are tracked.
- **Snapshot line.** Update the snapshot line above whenever a page is re-verified against a newer `dev`.
