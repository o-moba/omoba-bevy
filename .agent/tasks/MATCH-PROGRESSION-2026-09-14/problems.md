# Verification issues — resolved

Final status: PASS for all frozen criteria, subject to the explicit beta limits
in `evidence.md`. Earlier failed logs are retained and are not final results.

- Initial compilation found incomplete concurrent test files, a class fixture
  typo and a Bevy visibility method-pointer mismatch. Fixed; subsequent full
  workspace and final component builds/tests pass.
- `raw/career-full-tests-3.log` had two UDP fixture failures after 298 client
  passes. Fixtures assumed immediate delivery; bounded socket readiness now
  exercises the actual receiver without bypassing signatures.
- Audits found discarded Start metadata, full-channel heartbeat starvation,
  stale MMR before requeue, late jobs reviving rejected starts, and oversized
  account payloads. Preserved allocations, owner fencing, independent heartbeat,
  nonblocking retained critical ACKs, fresh profile ACKs, rejection tombstones
  and framed Career replies resolve these. Actual PG regressions pass.
- `raw/workspace-tests-final.log` failed one new UI fixture that set only the
  last result instead of applying a server view to select it. The corrected test
  uses the production update path; full workspace and final 301 client tests pass.
- Initial desktop screenshots had missing CJK glyphs. Licensed Noto and text-script
  selection resolve this. Twelve final screenshots were inspected; CJK/Cyrillic
  names and negative loss deltas render correctly.
- Queue-only updates previously reset scroll. Same-page scroll is now retained.
  Practice results explicitly say they are not persisted rather than implying
  an endless save. Relevant client regressions pass.
- Initial Clippy warnings in `raw/clippy-final.log` were resolved with test-only
  helpers, small fixes, a boxed worker channel record and documented narrow
  wire-enum size exceptions. Final strict workspace Clippy exits 0.

Physical touch/IME, global load, production infrastructure, account recovery,
party/chat/replay features remain unverified or deferred, not passing claims.
