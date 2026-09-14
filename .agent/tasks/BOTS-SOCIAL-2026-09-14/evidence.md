# Bot practice and social evidence

Task: BOTS-SOCIAL-2026-09-14. Base: 8872215. Version: 0.19.0-rc.7.
Final acceptance: **PASS — AC1 through AC6**, 2026-09-14.

## Scope

One explicit Practice arena per server process, immediate solo start, late human
replacement, server AI, Team/Match chat and four picture reactions. No deployment,
production dependency, replay, party queue, voice, wallet linking or NFT ownership
service was added. Practice receipts remain local with no permanent XP/MMR/history.
The live reaction sender grants the free pack only; generic NFT policy denies access.

## Acceptance mapping

| Criterion | Evidence |
| --- | --- |
| AC1 | `server/src/practice_tests.rs`: actual UDP solo/late admission, safe spawn and identity separation, reconnect/full seats, empty arena and 32-identity rollover including 16v16. |
| AC2 | Practice controller tests execute actual navigation, basic attacks, casts, ordinary damage/cooldowns, death/respawn and tower damage. Release/Dev never create native bots. Free bundled avatar/admission test preserves 2D kits. Python launcher tests exercise owned-process cleanup and mode isolation. |
| AC3 | `server/src/social/tests.rs`: three real UDP peers, long Unicode framed chat and team filtering, shared token budget, duplicates/stale scope, unsigned high-ID poisoning and same-epoch rematch assembly. Backend signature regression verifies tampering, nonce/session binding and no SQL job for chat. |
| AC4 | Client tests cover hold/drag/release, movement and OS cancel, wrong finger/invalid choice, IME, modal/closing-frame gameplay cancellation, stale events, mute, bounded logs and persistent bubble entities. Native QA sends actual Join/chat/reaction packets, injects native input messages and waits for authoritative echoes. |
| AC5 | Shared catalog/entitlement and client presentation tests cover bounded schemas, safe packaged paths, free fallback, access rejection and trusted grant scope. Original atlas and provenance/license are bundled. |
| AC6 | Logs below, measured native captures, manual image review, version/changelog/features/runbook/progress and final source/artifact hashes. No physical phone or global-load acceptance claim. |

## Commands and results

Cargo commands use `--locked --offline`, `CARGO_INCREMENTAL=0`, and the leased
cache at `../omoba-bevy-match-progression/.agent/tasks/MATCH-PROGRESSION-2026-09-14/target`.
For repeatable local checks, each invocation uses these existing-build overrides:

```sh
cargo --config profile.dev.debug=0 --config profile.test.debug=0 \
  --config profile.dev.opt-level=0 --config 'profile.dev.package."*".opt-level=0' ...
```

- `test --workspace --all-targets --locked --offline`: **599 passed**, 14 explicitly
  ignored PostgreSQL tests. Includes real Release lifecycle, matchmaking, jungle,
  framing, combat, map configuration and UDP harness tests. Log: `raw/workspace-tests-final.log`.
- After final avatar/HUD changes, `test -p client -p server -p shared --locked --offline`:
  **546 passed** (316 client, 177 server, 53 shared), 14 PostgreSQL tests ignored.
  Log: `raw/final-changed-packages.log`.
- Final QA focus handling: `test -p client --locked --offline`: **317 passed**.
  Log: `raw/client-tests-final-317.log`. The subsequent BOT label layer adjustment
  was verified by the current native build, strict Clippy and nine native captures.
- `test -p server --locked --offline -- --ignored --test-threads=1` with
  `OMOBA_TEST_DATABASE_URL` pointing at a disposable PostgreSQL 18 instance:
  **14 passed**. Log: `raw/postgres-tests-final.log`. Port 55441; the instance was
  stopped after verification. Runtime database/config files are excluded from Git.
- Launcher Python suite: **11 passed**, including native practice without harness,
  server-only mode, legacy play, inherited QA isolation and SIGTERM/unrelated process
  preservation. Log: `raw/launcher-tests-final.log`.
- `clippy --workspace --all-targets --locked --offline -- -D warnings`: PASS.
  `cargo fmt --all -- --check` and staged source/document whitespace checks: PASS.
  The whitespace command is `git diff --cached --check -- .
  ':(exclude).agent/tasks/BOTS-SOCIAL-2026-09-14/raw/**'`; untouched terminal logs
  keep Cargo's trailing blank lines. See their final logs.
- `build -p client -p server --locked --offline`: native binaries built successfully.
  Log: `raw/native-build-final.log`.

`run_native_qa.py` owns and stops each of its localhost server/client subprocesses,
uses isolated config directories, unsets the career backend, and captures desktop
1280×720, phone-preview 844×390 in 3D and phone-preview 844×390 in 2D. No synthetic
server events or player entities are installed. Its input and picker commitment
are scripted; physical devices and manual gestures are not verified.

## Review and limits

The first successful nine captures exposed two visual improvements beyond the
machine checks: desktop social controls needed to move below the minimap, and bots
needed bundled 3D models. Both were fixed and tested. Intermediate capture attempts
are clearly separated from final artifacts. Focus-loss telemetry records actual
OS cancellation; a requested focus flag alone cannot satisfy the QA gate.

See `problems.md` for initial compilation, transport, authentication, rendering and
QA setup findings and their resolutions. All external grants remain unwired. UDP
is not encrypted. Production load, physical iOS/Android controls and a deployed
account/NFT service require their own acceptance work.

## Final native matrix

`raw/native-qa-run.log` records a successful complete run against the final native
binary. Each directory below contains `01-chat.png`, `02-reaction-wheel.png`,
`03-confirmed-reaction.png`, a successful `qa-summary.json`, and client/server logs.
All nine PNGs were opened and visually reviewed after the final BOT label layer fix.

| Mode | Resolution | Directory | Result |
| --- | --- | --- | --- |
| Desktop, 3D | 1280×720 | `raw/desktop-3d/` | PASS, three captures |
| Mobile preview, 3D | 844×390 | `raw/phone-3d/` | PASS, three captures |
| Mobile preview, 2D | 844×390 | `raw/phone-2d/` | PASS, three captures |

The receipts contain actual server epochs, round/event IDs, native focus history,
and computed social-node bounds. The chat and wheel fit each screen; the confirmed
reaction is visible over the sender. Desktop social controls do not overlap the
HP/skills/equipment HUD. BOT labels render behind HUD cards. Mobile minimap bounds
in raw diagnostics are unscaled layout bounds; its existing transform scales it
into the visible top-left area, as the screenshots show. Hidden desktop skill-bar
nodes are not mobile social-layout acceptance targets.

The runner treats the fresh receipt, written after all GPU readbacks, as completion
and then stops only its own processes. It refuses nonempty capture directories to
prevent stale receipts passing a later run. The local PostgreSQL test instance and
all native QA children were stopped. No running public host was deployed.

`evidence.json` contains machine-readable acceptance/results and the accepted
capture matrix. `sha256.json` inventories all changed source/assets/docs and the
curated evidence artifacts, excluding itself to avoid a recursive digest. Failed
intermediate attempts remain in the local task directory; only final captures and
selected diagnostic logs are committed, without database data or account keys.
