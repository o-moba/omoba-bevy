# MVP checklist — blocking vs deferrable

Use **MVP-blocking** for issues that stop a tester from completing [docs/playtest-script.md](../docs/playtest-script.md) using only [README.md](../README.md) and [RUNBOOK.md](../RUNBOOK.md).

Use **Later / deferrable** for improvements that do not block the documented playtest path.

## MVP-blocking (examples)

- [ ] `cargo build --workspace` fails on a supported setup without documented workarounds.
- [ ] Documented `make start` / `make stop` flow broken or inconsistent with actual ports/process cleanup.
- [ ] Client cannot obtain first snapshot on localhost when following `RUNBOOK.md` exactly.
- [ ] Match cannot enter **running** state from lobby with two local clients under documented steps.
- [ ] Crash or hang that prevents finishing the playtest script in under 20 minutes.

## Later / deferrable (examples)

- [ ] Reconnect slot reclaim and NAT-persistent identity ([docs/features.md](../docs/features.md)).
- [ ] Full skill tooltip UX and advanced balance passes.
- [ ] Hosted server, metrics dashboards, on-call procedures.
- [ ] Cosmetic polish, non-blocking UI glitches, and performance optimizations that still meet basic playability.

## How to label issues

- Tag or title prefix: **`mvp-blocker`** vs **`backlog`** (or your tracker’s equivalent).
- In prose reports, use the **MVP impact** section in [docs/bug-report-template.md](../docs/bug-report-template.md).
