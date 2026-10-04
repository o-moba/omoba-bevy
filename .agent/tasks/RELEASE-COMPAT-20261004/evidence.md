# Verification: PASS

All AC1–AC5 pass; see evidence.json for individual checks and artifact SHA-256 values.

- Final current-source `make check`: **1,558 Rust tests passed**, 39 existing integration tests ignored; **203 Python tests passed**. Format, strict workspace Clippy and no-QA client Clippy pass.
- CLI matrix: identical contracts and a different release label exit 0; protocol/catalogue/map/gameplay/handshake mismatches exit 2; unanswered UDP endpoint exits 3. A real compiled standalone server also exits 0. No accounts or matches are created by the probe.
- Runtime test covers standalone, public lobby and worker. Reply-rate/size/nonce, stale report, malformed report, queued gameplay gating and both retry owners are covered.
- Native English mobile-profile capture at **852×393** passes. The real home screen shows the reason and keeps Offline Practice usable, with no overlap. The server report is synthetic; this is desktop-renderer layout evidence, not physical iPhone proof. The final menu retry fix changes no captured geometry.
- A fresh review found and fixed the menu's second automatic retry path after the first full gate; the final full gate includes that regression.

## Files and architecture

Shared contract/envelope: `shared/src/compatibility.rs`; CLI: `shared/examples/compatibility.rs`; dispatch/rate guard: `server/src/runtime/dispatch.rs`, `server/src/public_transport.rs`; client gate/lifecycle: `client/src/net/transport.rs`, `session.rs`, `status_ui.rs`; user messages: front-end/settings and EN/RU/zh-Hans dictionaries. `scripts/release.py` emits the shared manifest beside artifacts. `docs/release-compatibility.md`, release docs, changelog and progress note record rollout and rollback.

The envelope is independent of gameplay protocol 9. Release labels are informational. Manual compatibility revisions are not a proof of every gameplay behavior. Legacy clients keep their existing path; new clients need an upgraded server before distribution. Signed command admission is unchanged.

## Delivery state and limits

Changes are local in `feat/release-compatibility`, not committed, merged or deployed by this task. No CI configuration, secrets, production infrastructure or TestFlight state was changed. Native binaries reuse primary cache A; no mobile package was built. Screenshots/logs remain under this task's `raw/` directory. No cache was deleted. Disk space is now about 14 GiB, so another mobile/archive build needs disk space first.
