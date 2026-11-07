# Current-source load diagnostic — 2026-11-07

**1000-client acceptance FAILED. No capacity finding is closed by this packet.**

The production server source is unchanged through efe32101: all263 recorded
production source hashes and the accepted native binary SHA match the terminal
recovery proof. This existing direct-rustc server has unoptimized root crates,
unlike Cargo's dev opt-level1 or a release build. These are local diagnostics,
not a release performance benchmark or RU host/uplink certification.

## Actual observations

| Run | Result | Observed received snapshots/player/s |
|---|---|---|
|10 clients, existing disposable DB,30s |PASS |median17.9565 |
|100 clients, same disposable DB,30s |PASS |median18.0097, p0517.8541 |
|10 clients, new PG18.6, actual restricted runtime grants,30s |PASS |median17.7648; producer tick span18.7442/s; accepted fraction0.94775 |
|1000 clients, same new PG18.6/runtime grants, requested120s |**FAIL before soak** |No valid all-Running population or cadence result |

The large run finishes ordinary sign-in at9.47s and opens Search after five
stable seconds at14.53s. It fails at109.69s because an owned server/generator
exits. Native logs contain connected-UDP `Connection refused` panics. The old
runner did not preserve the first exiting process before cleanup, so this does
**not** identify which process failed first, its time or the root cause.
The60 panic observations were read after cleanup and can include secondary
shutdown failures. Their last BOT_SAMPLE times are not panic timestamps.

The original process cleanup also failed: a group-signal PermissionError aborted
`stop_owned`, leaving86 owned processes alive after the DB controller removed its
fixture. The exact traceback is preserved in `original-cleanup-failure.log`.
Revalidation checked UID, executable and recorded groups before individual
SIGTERM; `manual-cleanup.json` records that the final owned-group inventory is
empty. No foreign process was signalled.14 additional rehearsal cases PASS,
including two actual-child lifecycle regressions injecting group denial. Cleanup
now attempts every child, uses a live-child fallback, bounds waiting and reports
unverified group cleanup explicitly. This fixes a separate cleanup bug; it does
not identify the original admission failure. Post-teardown DB errors cannot be
used to attribute the earlier failure to SQL.

Last observations include mixed lobby/forming/Running phases, cooldown,
authorization and busy/pending errors. Their stale cached presence cannot prove
that each error was still live at the same time or caused the first failure.
Do not attribute this to PostgreSQL permissions, server capacity, packet loss
or generator CPU without a discriminating reproduction. The earlier frozen
78070b27 1000-seat/60s result retains its historical scope; it does not validate
this current source or erase this rejection.

## Diagnostic change

The native harness reports the last fully accepted snapshot's producer metadata
per peer. The capacity runner compares accepted-count deltas with producer tick
spans under unchanged protocol/epoch/match authority. It rejects missing,
noninteger or contradictory metadata and changed authorities. It also retains
the initial samples and first observed owned-process exit codes before cleanup.

This differentiates an advancing producer sequence from missing complete
snapshots at the receiver. It does not independently timestamp sends, distinguish
kernel loss from generator starvation, or measure latency. Existing gameplay,
consent, authority checks, timers and production code are unchanged.

Seven focused Python cases, the actual socket transport regression (including
drained snapshots, replay rejection and a deliberate tick gap), strict Clippy
for harness/library, bot binary and transport test, formatting and diff checks
PASS. The last baseline/first-exit retention and early metadata-validation edits
and cleanup correction postdate the live run; they are not claimed to have had
a fresh full live acceptance. Cleanup has its two actual-child regression tests.
The build report records the actual source/binary/dependency commands, with
read-only existing libraries and no Cargo/graphics/cache build.

## Evidence and reproduction

`summary.json` lists SHA256/size for each gzip-compressed exact report/log.
Decompress with standard gzip tools. Compression avoids adding megabytes of
repeated fixture data to the diff; no raw report status or content was changed.
The10-client baseline was uniquely recovered from its real BOT_SAMPLE log and
exactly reproduces every wire metric; it is archived separately. The verifier
checks integrity and recomputes it, while explicitly preserving the1000 failure:

```sh
python3 docs/evidence/load-cadence-2026-11-07/verify.py
```

`rehearse_local.py` preserves the exact local controller executed here. It expects
the existing cached Docker image/owned base fixture and binaries under ignored
`.agent/codex-ranked-readiness/load-cadence`. It creates a uniquely named-for-this
attempt loopback-only DB with160 connections, applies the actual13/11 migrations
and grants to a nonsuperuser game role, then runs the ordinary capacity script.
Use a fresh owned output/name before a new attempt; the runner refuses to reuse
existing output directories. It does not download images or modify the base DB.
Both attempted DB containers/volumes were removed; the initial rejected setup
accepted UNIX readiness before DDL exited2, and the next requires TCP readiness.
That startup change does not establish the initial failure's exact cause.
No fixture DSN, password, auth/session nonce or private key is published.

Next: reproduce with the retained first-exit diagnostics and capture worker
status/health before teardown; distinguish admission/auth/SQL/timer or generator
failure. Record an optimized current build before performance qualification.
Then complete sustained1000-client cadence/latency and actual RU/identical-host
uplink acceptance. Physical/WAN/trust/recovery/release gates remain separate.
