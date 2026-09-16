# 2026-09-16 — shared accounts and cosmetic Supporter

Release source version: `0.20.0-rc.5`. This session implements the cosmetic
Supporter foundation across the native client, game server, Account API and
`omoba-web` player portal. It does not deploy billing or upload a TestFlight build.

## Delivered source

- Independent native device keys join one immutable career profile after explicit
  owner approval and proof of possession. The portal manages devices and one-use
  recovery codes. Old browser approvals must explicitly authorize the new scopes.
- Server-owned expiring grants authorize Solar, Lunar and Verdant auras. Native
  and portal previews explain the cosmetic effect. Combat and progression do not
  change. Revoked devices also release queued/reserved match seats.
- PostgreSQL records provider events, ownership, payment periods, refunds and
  preferences. Duplicate/stale events cannot extend or revive revoked periods.
- Website USDC checkout uses account-bound quotes, finalized Solana verification,
  payment discovery and recoverable recent invoices. It grants 30 prepaid days.
- The iOS build compiles a StoreKit 2 purchase/restore bridge. Transactions remain
  unfinished until the Account API accepts server verification. Apple receipt,
  notification and reconciliation policy is separately testable.

## Delivery limits

Apple's actual certificate/API verification service still requires approval to
add the official `@apple/app-store-server-library` production dependency. The
policy tests use injected synthetic verifiers and do not prove Apple signatures.
The Apple adapter fails closed without a configured verifier; do not enable
production purchases with a mock service.

Operator configuration, App Store products/credentials, terms/privacy purchase
links, a real Apple sandbox cycle, a Solana devnet wallet payment and physical
phone validation remain release gates. No production database, secrets, payment
account, deployed service or running practice server was changed in this session.

See [the operational guide](../supporter.md) and the current verification report
in `.agent/tasks/SUPPORTER-2026-09-16/`. Task-only checkouts isolate implementation;
verified source is copied back to the primary project directories for continued
work. Existing iPhone build scripts and earlier phone artifacts are preserved.
