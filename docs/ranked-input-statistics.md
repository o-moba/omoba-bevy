# Ranked controls comparison

Touch and mouse share the same queue, matching weights and entry requirements.
The native client reports its selected controls with ordinary signed movement
packets. This is a declared control scheme, not hardware or operating-system
attestation. It contains no device identifier. Disposable automated actors and
old clients report unknown by default.

The server retains evidence per human seat: missing reports cannot erase a
known scheme, a switch becomes mixed, and AI takeover cannot supply human
metadata. Declaring controls alone does not reclaim an AFK seat. Checkpoints and
terminal receipts retain this evidence across worker recovery and retries;
existing append-only participant rows and old JSON receipts remain unchanged.

`CareerStore::input_statistics(receipt_limit)` reads at most the newest
1–10,000 settled receipts, then compares human seats in completed, actually
started Ranked games. It excludes bot and proved-abandon seats. Unknown legacy
or future schemes have their own visible row. Each row exposes matches and wins,
so callers can show a rate together with its sample size. Empty groups have zero
counts; they must not be represented as a measured 0% rate. This sample is not a
whole-season or randomized population, and differences do not establish causation.
The report includes scanned receipts and eligible games to disclose coverage.

The read-only example `ranked-input-stats` accepts `--receipt-limit` (default1000)
and an existing read-capable `DATABASE_URL`. It validates the existing schema,
returns anonymous aggregate JSON and never migrates or resets ratings. Do not
put connection credentials in screenshots, report files or command arguments.

Native/portal season presentation remains in the Ranked plan.
