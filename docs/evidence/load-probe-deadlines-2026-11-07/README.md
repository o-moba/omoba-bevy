# Native load probe correctness — 2026-11-07

Two real probe defects are fixed. **Current1000-client acceptance still FAILS.**
No original capacity, cadence, RU-host or release finding is closed here.

## Fixes and discriminating checks

`Bot::recv_snapshot` and raw receive previously accepted a20ms poll deadline but
used the socket's fixed200ms timeout. Two real UDP tests send a packet80ms after
the deadline: both fail against the old receive implementation. Each blocking
read now uses the smaller of its remaining deadline and the ordinary200ms cap.
Both tests pass, and the late packet is preserved for the next poll. This avoids
one idle lobby peer unnecessarily blocking the other nine sequential AI peers.
The existing wire-count/drained-snapshot/replay test also passes.

The public AI client also ignored Career envelope sequence numbers. Native
`client/src/net/ingest.rs` already fences these before observing auth. A regression
against the unfiltered handler demonstrates an older authority-loss reply
resetting newer authenticated state. The harness now checks nonzero/current
epoch and a monotonic Career sequence before applying any view; transport reset
and new epochs reset its high-water mark. Wrong epochs cannot poison that mark,
repeated challenges preserve it, and a genuinely newer auth loss still resets
the session.35 complete harness-library cases and all3 UDP cases pass. Four
strict Clippy targets,8 capacity Python cases,14 rehearsal Python cases,
formatting and diff checks PASS. No production auth gate was weakened.

The runner saves the first observed exited generator index/code and bounded
active/archive worker status/health/log-tail observations **before cleanup**.
It omits auth/session fields and redacts fixture DB URLs. Eight Python cases
include archived-worker/bounds/privacy coverage. Diagnostics are observations,
not match/health authority. The native status additionally counts discarded
stale Career packets and genuine authority resets without recording credentials.

## Live results

All runs use the unchanged server7a736997… and fresh loopback PG18.6/current13/11
fixtures with the actual nonsuperuser runtime grants. Full production-source
closure (263 hashes) matches the terminal-replay proof. Existing server and newly
linked AI harness use unoptimized root crates, not Cargo dev opt1 or a release
performance build. No Cargo/graphics/cache build or RU change was performed.

| Probe revision |10-client30s control |1000-client requested120s |
|---|---|---|
|Receive deadlines fixed |PASS; Running19.83s, accepted median18.026Hz |FAIL106.26s, first observed generator49 exits101; no all-Running/soak/cadence result |
|Deadlines plus Career order fence |PASS; Running21.07s, accepted median18.052Hz |FAIL108.35s, generators83/89/93/95 observed exited101 in the same poll; no all-Running/soak/cadence result |

The first rejection records69 Running,6 Forming,5 Ready and1 Failed worker
receipts before teardown; one affected retired arena was observed while SQL
health was still reporting ready. That is a cached health observation, not proof
that every operation or the whole prior interval was healthy.

The final rejection records44 Running,20 Forming,10 Ready and2 Failed worker
receipts. Summed latest retained peer observations report **0 stale Career
packets and209 genuine authority resets**. These are cumulative per-peer
observations, not a simultaneous final population or an exhaustive event count.
Career reordering is therefore **not the established cause of this local failure**.
The two correctness fixes remain valid independently of that rejected hypothesis.
Next investigate auth refresh/expiry and transport transitions under load, with
an explicitly optimized current server/generator graph. Do not extend auth TTL,
relax authority, remove players or shorten real game/recovery timers to pass.

## Cleanup and evidence

The rehearsal controller intentionally refuses fallback signalling when a
process's executable identity cannot be verified. Its immediate post-cleanup
inventory twice met a transient identity-label change; the second captured
`(server)` with state S. The guard aborted before persisting the controller
summary/DB cleanup. This does not identify the large-run admission cause.
Subsequent fresh inventories were empty. Both owned DBs/volumes were then removed;
audit files preserve that result. The final fixture's13/11 schemas,160-connection
setting and nonsuperuser role flags were directly rechecked before deletion.
No unverified or foreign process was signalled. Improve the disposable controller
to wait for a stable inventory and always persist its cleanup audit before reuse.

`summary.json` records source/binary identities and SHA256/size of exact compressed
reports/logs. The intermediate deadline-only report carries its actual runner
source hashes; the complete latest build/source manifest belongs to the final
fenced run. Intermediate and final proofs are not relabelled as one binary.
No fixture key, auth/session nonce, signature, password or DB URL is published.

```sh
python3 docs/evidence/load-probe-deadlines-2026-11-07/verify.py
```

This verifies the archive and recomputes both10-client controls from retained
initial/final native samples; it does not run a new live game or accept1000 load.
Ignored raw runs remain under `.agent/codex-ranked-readiness/load-deadline` and
`load-career-order`; source/build helpers and negative-test executables/logs are
under `load-cadence`. Exact executed local controllers remain there as rejected
history; use fresh fixture/output names and correct cleanup before a new run.
The standard loopback-only `scripts/public_playing_capacity.py` remains the
actual entry point with `--allow-large --clients 1000 --seconds 120`, current
server/bots binaries and a
disposable `/omoba_test` DSN in the environment. It never provisions live infra.

Sustained load, latency, RU/identical-host capacity/uplink, WAN, physical devices,
trust provisioning and all remaining release gates stay open. The full CI for
this next source checkpoint is separate from these targeted local checks.
