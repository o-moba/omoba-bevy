# Measured input profile and next-agent checkpoint — 2026-11-07

**Large v1 is REJECTED. The nonblocking v2 driver is prepared and unit/UDP
checked, but its live control and1000-client repetition have NOT RUN.**
This packet preserves the exact failure and next step for an agent handoff.
It changes only the native test harness and capacity tooling, not production
gameplay/server/store/SDK, dependencies, CI configuration or RU infrastructure.

The new loopback-only `bots --public --input-hz 20` option schedules signed
movement independently of snapshot arrival. It reuses only a current Running
snapshot less than2s old; cached reuse never advances receive freshness/counters.
The pose for a dead player remains `input_active:false`. Monotonic counters
advance only after the actual signed UDP send succeeds. The capacity runner
defaults to20Hz and requires every peer's measured average within2.5%; explicit
`--input-hz 0` retains the historical snapshot-paced profile.

## Recorded blocking v1 result

| Measured observation |10-client30s control |1000-client120s attempt |
|---|---|---|
|Overall verdict |PASS |FAIL: actual input outside requested profile |
|All Running after launch |15.894s |47.201s |
|Actual signed transforms/player/s, min–max |20.020–20.020 |18.163–19.300 |
|Actual signed transforms/player/s, median |20.020 |18.780 |
|Transforms sent using cached snapshots |647 |830414 |
|Minimum cached transforms per peer |63 |451 |

The large attempt retained1000 fresh signed AI peers in100 ten-seat Casual
arenas for120s, with all durable rosters present and zero observed authority
resets. It still fails the requested input gate. Preserve that rejection;
do not widen tolerance or call population alone a full-profile PASS.
This used the same opt1 server as the previous population packet, PG18.6,
career13/portal11 and actual restricted runtime grants, zero server bots.
The controller confirms complete cleanup, empty final owned-process inventory
and removal of its disposable DB/volume. No v1 process remains running.

`blocking-build.json.gz` binds the v1 bot to its actual sources/output hash.
`build.json.gz` describes the subsequent v2 harness/library, bot and UDP-test
builds. These binaries differ; v1 is not evidence for v2. The suspected source
of input underproduction was blocking receive time across10 peers. V2 drains
only buffered datagrams with256-packet/2ms bounds and preserves real fragment
assembly, ordering and Career authority fences. The scheduler skips excessive
catch-up rather than emitting a multi-period burst.

## Checks and exact continuation

Current source/output hashes match the v2 build manifest.36 harness-library,
4 real-UDP transport,23 Python capacity/lifecycle tests and5 invalid CLI cases
pass. All4 strict Clippy targets pass with workspace lints and denied warnings;
commands/exit codes are in `clippy-checks.json`. The new UDP case checks idle
polling, partial compressed frame retention and real receive counters.
The initial Python module-style invocation omitted `PYTHONPATH=scripts` and
failed imports; the corrected invocation below passes23 tests.

From `/Users/wotori/git/ekza/_workspace/worktrees/mm-game`, after acquiring a new
workspace lease and checking processes, source hashes and free space:

```sh
PYTHONPATH=scripts /opt/homebrew/bin/python3.13 -m unittest test_public_playing_capacity test_public_bots_rehearsal
/opt/homebrew/bin/python3.13 docs/evidence/input20-handoff-2026-11-07/verify.py
/opt/homebrew/bin/python3.13 .agent/codex-ranked-readiness/load-input20/rehearse_v2.py
```

The last command has **not** run. It creates a fresh owned
`omoba-mm-input20-20261107-v2` fixture and `load-input20/live-v2` output, then
10 clients/30s and1000 clients/120s sequentially, with cleanup in `finally`.
Do not run concurrent builds/downloads that confound the measurement. An
existing output directory requires inspection, not deletion/restart on sight.
The controller and build helper are copied here for portability; they still
refer to the existing local coherent graph and should not be mistaken for a
standalone platform build. Root is about12GiB free: no heavy Bevy/SDK build.

If v2 passes, preserve its independent manifest/results and continue cadence,
send gaps, per-command acceptance/input-to-snapshot latency, longer-duration,
Linux/RU-identical-host/uplink/WAN and physical acceptance. Kernel send counts
are not server acknowledgements. Shared snapshot sequence reservations include
unsent bootstrap ticks and do not measure packet loss. The original68 remains
62 done/5 partial/1 open; all critic/UI/physical/recovery/RU release gates remain.

Raw local artifacts: `.agent/codex-ranked-readiness/load-input20/`.
Only selected anonymous reports/build metadata are archived here; no raw signed
bot logs, tokens, private keys or fixture DSN are included. The verifier checks
archive hashes and recomputes both measured verdicts; it executes no live game.
