# RU release preparation — 2026-10-28

This delivery prepares dev sources for the next coordinated main release. It is
not a public launch, a server upgrade or a statement that every LR-68 item is done.
Source version: 0.45.0. Base: dev 120ce63b6e6f47986c45f8830429847fc571fe52.

## Registration follow-up (2026-10-28)

[Registration](../registration.md) now covers website-first default-profile creation,
game-first progress-preserving linking, all three VK ID methods, recoverable native
polling, quota separation and account-review/key-revocation races. Portal schema 10
and its restricted EXECUTE grant are required. Loopback browser and signed real-game
proof do not certify real VK/Mail/OK credentials or deployed public services. The
read-only DNS/VPS check still finds public RU login unavailable. No live changes.

## Routing delivered

- One default client can bind RU or International. First-time players confirm an
  account region and its documents; returning accounts restore their local binding
  automatically. No credential-based discovery across regions or IP geolocation.
- Lobby, Account API, portal, runtime integrations, purchase offers and local state
  follow that binding. Region-specific identities remain separate. No raw server
  configuration appears in production or dev player screens.
- A switch writes a pending region, closes the game transport, gates new regional
  actions and shows a blocking restart card. It cannot change a running process's
  region, merge profiles or copy a key. Active match/purchase/known unfinished
  StoreKit receipts block opening the selector. Already-started bounded HTTP calls
  can finish against their original authority; none is reissued to another region.
- Device enrollment/recovery/completion and native social-link replies carry
  `account_region`. Native transport rejects a foreign region before using a reply.
  Legacy replies may omit it; the exact trusted regional origin still owns them.
- Both production IPs are denied from dev. RU dev uses its own
  `lobby.dev.omoba.ru:4100`, `account.dev.omoba.ru`, `dev.omoba.ru` endpoints;
  unavailable RU dev does not fall back to global dev. This replaces LR-68's earlier
  shared-dev exception to keep each backend's authenticated account region true.
- Existing locked RU packaging remains. Store-channel rollout, StoreKit receipt
  ownership maps, Apple sign-in, unlink, device-wide preference migration, iOS
  Keychain and a final unified public app identity are not included in this slice.

## Existing code versus deployment evidence

Dev already includes native identity and recovery, legal acceptance gates,
moderation, server admission, update/compatibility handling, RU host tooling,
portal ingress fixes, backup/restore tools, queue behavior and the YooKassa tier
engine. Preserve those implementations. A code checkbox is not live acceptance.

The earlier provisioning record for 2026-10-28 reports the RU game host at
212.113.117.105 (`omoba-ru-server`), a fresh PostgreSQL database, a two-room cap,
main 0.44.0 / protocol 11, native service `omoba-ru-lobby`, daily local backups and
one verified off-host copy. This dev delivery does not change or re-probe production.
Dev already uses protocol 12; clients from this branch must not be distributed
against that protocol-11 production endpoint.

## Required before main publication and acquisition

| Gate | Remaining concrete result | Owner |
| --- | --- | --- |
| Regional dev authority | Provision a separate RU dev API/lobby/portal or explicitly isolated loopback stack; exercise real enrollment, recovery and returning-account routing | Ops |
| Coordinated game upgrade | Stage matching client/lobby/workers, verify compatibility, authenticated admission, two rooms and reconnect; prepare rollback before cutover | Owner + ops |
| RU account service and portal | Deploy the matching API and portal with all current migrations/grants and strict loopback ingress; reconcile the existing `omoba-ru-lobby` service with the host kit's `omoba-lobby-ru` naming before adopting the kit | Ops |
| DNS, TLS and downloads | Resolve omoba.ru/account/lobby to the RU host, validate HTTPS, current release catalog, direct downloads and checksums | Owner + ops |
| Real login | Configure the owner's VK ID app/callback and verify a real first login, existing login, cancellation, device pairing and recovery end to end | Owner credentials + device test |
| Player documents | Replace `0-draft` legal documents/placeholders with approved operator details, support contacts and final accepted version; confirm publication requirements | Owner |
| Android identity and packages | Confirm the public app identity and existing durable signing key, increment build code, build candidates from frozen main and record hashes | Owner + build |
| Physical-device acceptance | Install candidates on actual supported devices; verify first run, sign-in, online match, disconnect/reconnect, backgrounding and account recovery | Device QA |
| Operations | Recurring off-host backup, scheduled restore proof, monitoring/alerts, disk/log limits and measured room/RAM limits | Ops |
| Payments, if enabled | Finish portal pages and real provider/fiscal verification; keep unverified purchase entry points disabled in the distributed package | Owner + billing QA |

Global production Account API availability remains a separate gate. Selecting
International in a unified client does not create that service. Russian audience
onboarding can be released first through a verified locked-RU direct package while
full unified/store work remains unfinished.

## Verification scope

Run `make check`, disposable PostgreSQL tests, and the focused native RU 852×393
first-launch/switch harness. Record compile, desktop preview, uploaded artifacts,
physical devices and live deployment separately. Raw evidence is kept in
`.agent/tasks/ACCOUNT-REGION-ROUTING-20261028/`; the delivery record links results.
