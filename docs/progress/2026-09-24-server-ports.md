# 2026-09-24 — Server ports: career, transport and clock behind traits

## Goal
Roadmap step 7, slices 2 and 3. `ServerRuntime` used a bound `UdpSocket`,
`Instant::now()` on its tick path and a `CareerBackend` that spawned the
PostgreSQL worker thread from the environment, so every runtime test bound a
real socket and no test could move time or drive the store deterministically.
The runtime now owns three ports (`Transport`, `Clock`, `CareerPort`) with a
process implementation and an in-memory one each.

## Changes
- `server/src/career_port.rs` (new): `trait CareerPort`, the twenty-four
  methods the runtime called on the backend plus the `test_*` hooks under
  `cfg(test)`. `CareerRuntime.backend: Box<dyn CareerPort>`;
  `CareerRuntime::new(backend)` takes it.
- `server/src/career_backend.rs`: the state machine is `CareerBackend<L:
  JobLink>`; `JobLink { enabled, try_send, try_recv }` replaces the `tx`/`rx`
  fields. `WorkerLink` is the production link (`CareerBackend`, default type
  parameter); `MemoryLink` (test-only) is `MemoryCareer` with `test_backend`
  (pending until a `test_ack_*` hook; jobs captured), `immediate` (acked at
  once: login creates a profile, start started, settle saved, access refresh
  keeps the key active) and `disabled` (no storage). The backend's own tests
  push replies into `link.replies` instead of swapping the channel. Nothing
  in the account logic (`handle`, `poll`, `record`) changed.
- `server/src/runtime/ports.rs` (new): `Transport` (`recv`, `send_to`,
  `local_addr`, test-only `peek`) with `UdpTransport` and `MemoryTransport`;
  `Clock` (`now`) with `SystemClock` and `ManualClock`.
- `server/src/runtime/mod.rs`: `transport` and `clock` fields replace
  `socket`; `with_ports(transport, clock, server_epoch, career, config, map)`
  is the one constructor, `new_with_map` and `new` wrap it with the process
  ports, `for_test(MemoryTransport, ManualClock, MemoryCareer, config)` with
  the memory ones. `run()` is unchanged apart from reading the sandbox's
  base time and the loopback check through the ports.
- `runtime/tick.rs`, `runtime/dispatch.rs`, `snapshot.rs`: `Instant::now()`
  on the tick path became `self.clock.now()` (prepare_tick, the receive
  loop and its budget, admission completions, the sandbox's roster and
  snapshot throttles). `snapshot.rs`, `social.rs`, `match_service.rs`,
  `career_runtime.rs`: sends go through `self.transport`.
- Tests: fixtures assign `Box::new(MemoryCareer::test_backend(epoch))`
  where they assigned `CareerBackend::test_backend(epoch)`; `rt.socket` is
  `rt.transport` (`local_addr`, `peek`). Converted to the socket-free
  fixture as proof: one practice test (join as a datagram), one
  career-runtime test, one new whole-runtime session-timeout test in
  `tests/sessions.rs`; one new test pins the immediate acknowledgements.
- Docs: `ARCHITECTURE.md` Server tick and roadmap line 7,
  `REFACTORING.md` row 7 and the step 7 section, `CHANGELOG.md`.

## Decisions
- The worker-vs-standalone split stays at the call sites. The eleven
  `match_service.worker()` sites decide whether a round is durable, public
  casual or rateable and whether to write `cancel.json`; the port records
  and acknowledges what it is handed. Putting that into the trait would make
  the in-memory store mirror allocation policy. If the sites keep growing,
  an `AllocationRules` value derived once from the worker manifest (like
  `MatchRules`) is the follow-up.
- `test_*` hooks are trait methods under `cfg(test)` rather than
  concrete-type-only: fixtures hold the store as `Box<dyn CareerPort>` and
  thirty call sites would otherwise need a downcast. `test_with_database`
  stays on the Postgres type.
- `MemoryCareer` is `CareerBackend<MemoryLink>`, not a second copy of the
  account logic: signature verification, replay protection and the record
  bounds are the parts worth testing, so the test store runs the same code
  over a different link.
- `CareerBackend`'s TTLs (`COSMETIC_CACHE_TTL`, `CHALLENGE_TTL`, presence)
  keep `Instant::now()`: they are the store's own clock, not the tick's.
  The sandbox's virtual clock keeps its own scaled, pausable `now` and
  starts from the injected clock. `run()`'s step timer stays real.
- `Transport::send_to` takes `&self` (as `UdpSocket` does) so the snapshot
  loops can hold `&self.world` while sending; `recv` takes `&mut self`.

## Checks
- `cargo fmt --all`, `cargo clippy --workspace --all-targets --no-deps -- -D warnings` clean.
- `cargo test -p shared -p server`: shared 78, server 282 (+3 ignored; was 280).
- `cargo test -p harness --no-run` builds; `cargo test -p client --lib` 543.
- Black-box harness run by the maintainer before merging.

## Remaining risks
- Fixtures other than the four converted still bind `127.0.0.1:0`; the
  UDP-level tests (`send_udp`, `peek`) still exercise the real socket path,
  which is intended.
- `MemoryCareer::immediate` answers only `Profile` and `SupporterStatus`
  actions with data; history, friends and renames come back as an error
  view, which no test relies on yet.
