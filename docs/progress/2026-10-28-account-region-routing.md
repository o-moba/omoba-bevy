# Account region routing and RU release preparation — 2026-10-28

Version: 0.45.0. Base: dev 120ce63b6e6f47986c45f8830429847fc571fe52.
Work: feature/account-region-routing, isolated worktree, shared cargo cache A.
This delivery changes dev sources; no production deployment, published package,
CI configuration, DNS, secret or new production dependency is included.

## Behavior

A default native client confirms an account region before regional traffic.
Returning installations restore their authenticated account region automatically;
first-time players choose RU or International without configuring a server address.
An unambiguous legacy regional identity is adopted without copying its key.

The immutable region binding owns lobby, Account API, portal, legal documents,
features, purchase offers and regional state. RU keys, profiles, caches and match
resume never share the International folder. Ekza is closed in RU. Raw server
configuration is absent from Home, Party and phone controls in both contours.

Settings can request a different account region outside an active match/purchase
or known unfinished StoreKit receipts. It saves a pending choice, closes transport,
gates new regional actions and shows a blocking restart card. Desktop restart exits
the old process; phones require a cold close/open. No running process is rebound.
Already-started bounded HTTP calls stay with their original authority.

Native device and social-link replies identify `account_region`; clients reject a
foreign reply before using it. Legacy APIs may omit the additive field while their
exact trusted regional origin supplies the boundary. Metadata holds public IDs,
never private keys or bearer credentials, with bounded reads and private atomic writes.

Dev rejects both production IPs, including 212.113.117.105, and known foreign-region
worker hosts. RU dev packaging now uses a separate regional authority rather than
the global dev API; it needs provisioning. Desktop loopback rehearsals require both
API and portal on loopback. Existing locked RU packaging remains available.

## Changed areas

- `shared/src/region.rs`, client account-region, legal/startup/network/persistence
  integration: first binding, remembered accounts, runtime gates and cold switching.
- Career, frontend, Ekza and Supporter callers: runtime regional capabilities and
  purchase offers; no raw endpoint controls. Native enrollment transports and API
  replies enforce account ownership. Translation keys cover all shipped locales.
- Desktop/Android/Xcode packaging: regional dev defaults and production-IP guards.
- Tests: routing/auth/host/UI regressions, native preview harness, API PostgreSQL
  reply assertions and isolated random-port fixture support. The moderation
  formatter test now uses a fixed clock instead of an expired calendar deadline.
- Documentation: current profiles/markets/features/ADR status and a concrete
  [RU release checklist](../plans/ru-release-2026-10-28.md). The UI screen inventory
  marks the old server frame as retired.

## Verification

- `make check`: PASS, 2,383 Rust tests passed, workspace clippy and no-QA client
  clippy passed; Python suites ran 608 + 55 tests (9 optional script skips).
- `make test-postgres`: PASS, 511 tests including the PostgreSQL integrations, on
  task-owned PostgreSQL 18 at a random loopback port. Existing local databases on
  ports 5432 and 55581 were untouched. Fixture guards still require loopback and
  the explicit launcher's `omoba_test` database for random ports.
- RU-compiled `devices_postgres` and `social_postgres`: PASS, 12 tests, including
  actual response-region assertions and restricted portal-role operations.
- Frozen dev-contour native build: PASS. RU 852×393 desktop phone preview: 9/9
  checks passed (first confirmation, late RU key initialization, endpoint/features,
  immutable active region and pending restart). Both changed cards were inspected;
  the acceptance switch stays visible with Continue. No physical phone is claimed.
- Final UI inventory update: 5 targeted Python checks passed; `git diff --check`
  passed. Raw evidence: `.agent/tasks/ACCOUNT-REGION-ROUTING-20261028/raw/`.

The full gate used Python 3.13, `NO_PROXY=*` / `no_proxy=*` for direct local fixture
traffic and an explicit absent `OMOBA_UI_HANDOFF`, matching isolated CI. The sibling
omoba-ui handoff lists an older Chinese font size than the committed client font;
no assets were replaced from that unrelated checkout. Installed asset integrity,
font glyph coverage and package budget remain tested. Mock iOS-tooling test output
is not evidence of an actual device installation.

## Release boundary

This is the routing foundation, not all of LR-68/LR-69. Store channels, final public
app identity, Keychain persistence, device-wide preference migration, Apple sign-in,
unlink/deletion and regional StoreKit receipt ownership remain separate work.
The earlier RU provisioning record has main 0.44.0/protocol 11; dev already has
protocol 12. Publish no candidate against that endpoint before the coordinated
server/client upgrade and real account/portal/DNS checks. Signing, approved player
documents, recurring off-host backups/monitoring, real-provider checks and physical
device acceptance remain gates in the release checklist.

## Capture fixture follow-up

Ordinary screenshot/demo runners use a fresh scratch folder and therefore need
synthetic returning-player routing metadata after first-launch confirmation was
introduced. A Python helper now seeds only that Global selection, without copying
keys, accepting legal documents or overriding existing records. Shared showcase/UI
launchers and the standalone combat/map/vision/targeting/pickup/Verdant/demo runners
use it. The dedicated first-launch harness still starts empty; runtime gates are
unchanged. This follow-up changes Python/docs only; Rust files remain identical
to the locally verified 0.45.0 delivery.
