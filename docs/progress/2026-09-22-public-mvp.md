# Public multiplayer MVP — 2026-09-22

Canonical workspace version: `0.22.0-rc.1`.
Task proof: `.agent/tasks/TASK-PUBLIC-MVP-2026-09-22/`.
Implementation branch/worktree: `codex/public-mvp` in `/private/tmp/omoba-public-mvp`.

This note records implementation and checks observed during the session. The task
proof records final Git delivery, native package identity and the independent
acceptance verdict. It does not certify a publicly deployed service or a signed
platform release.

## Consolidation

Main was fast-forwarded from `8d3e589` to `241be755`, preserving the verified
Verdant UI (`84a6a924`), mobile HUD (`c900df87`), team draft (`cf758119`) and shared
Run/VRM (`241be755`) commits. The original `omoba-logo.png` source art is included
in this delivery; its processed iOS icon was already committed previously.

Integrated the account persistence work from `44263767` and pinned Ekza Bevy SDK
0.7.0 at `42c39e4bf5aea2e3a051f86ba27cbf6e2c46d377`. Account restoration, logout
and revocation preserve current catalogue entitlement/revision handling and grid
scroll. Collection and the hero picker both expose logout. This cosmetic account
connection remains separate from the saved game profile.

The older Bevy 0.19 and gamepad branches remain isolated. Their historical evidence
does not justify introducing the engine migration into this release consolidation.
The MVP stays on Bevy 0.18 with no added production dependency.

## Implemented multiplayer boundary

One authenticated public lobby allocates independent worker processes and UDP
ports. Quick waits up to 30 seconds before bot fill; Wait for players requires ten
compatible humans; Play with bots allocates one human and nine bots. Rating and
newcomer cohort rules remain in force. Capacity is bounded and exposed to clients.

Allocation freezes profile/session/team membership. Workers keep the shared hero
draft, countdown and loading barrier, then wait for durable career-start approval.
Fresh accounts cannot observe the world, join a running match or replace a bot.
Existing participants can reconnect to the same player and loadout. Worker handoff
never overwrites the saved lobby endpoint. Saved results refresh in the postmatch
screen and play-again returns to the lobby. Explicit server authentication loss
invalidates stale signing authority and restarts the bounded login handshake.
Phone Home navigation uses a separate footer row so all three queue choices
retain 44-pixel touch targets without pushing secondary controls below the screen.

Cancellation checks uncovered and fixed two additional boundaries: only an exact
allocated account/session may cancel worker formation, and signed lobby Leave
clears queue intent even when an adjacent account CancelQueue was throttled.
If both cancellation packets are lost, waiting entries expire after 15 seconds
without an explicit FindMatch retry. Home heartbeats do not renew queue intent.

## Persistence and recovery

PostgreSQL transactionally settles server-owned results, preserving immutable
history and preventing duplicate awards. Approved completed public bot matches
use `public-casual-v1` / `allocated_bots`: authenticated humans receive 50 XP for a
win or 25 for a loss, with unchanged Elo and rated-match count. Bots have no career
profile. Eligible full-human matches keep the existing rating calculation and
150/100 win/loss XP. Interrupted, custom, development and standalone practice modes
do not receive these progression awards.

Public casual allocations require ten balanced frozen seats, actual bots,
authenticated human profiles and approved default configuration. Runtime checks
exclude custom tuning, god/speed modifiers and targeting QA before selecting this
policy. Database counters, receipt writes and rating/XP updates share the settlement
transaction; duplicate/replayed settlement returns the original receipt.

Public match workers use one PostgreSQL connection each. Lobby and standalone
game career pools use up to four each; portal/API and operator connections require
an additional budget. Current career/portal migrations and private per-worker
outboxes are required before service startup.

Lobby restart adopts live worker manifests without restarting the simulation.
Worker crash recovery retains the original epoch and replays its durable outbox
before an epoch-scoped expired-allocation sweep. The lobby never runs a global
sweep that could settle another worker before its outbox replay. An interrupted
match retains history without XP/Elo; a replacement assignment waits for durable
cleanup. An adopted worker may require the actual 100-second stale-heartbeat wait
followed by the recovery child's 120-second gate. This is interruption recovery,
not mid-frame simulation restoration.

## Public transport

Return-path proof precedes admission. Gameplay commands sign the account key,
session/authentication nonce, path nonce, epoch, match and monotonic sequence.
World replication requires allocated authenticated membership; empty bootstrap
snapshots carry no world entities.

Per-process limits are 512 endpoint records, 64 probes per second, 120 packets per
endpoint per second, 12 KiB public datagrams and 8 KiB inner signed commands.
Receive work stops after 128 packets or two milliseconds per pass. Challenges
expire after five seconds and validated paths after 30 seconds without traffic.
These are application bounds, not an external DDoS protection claim. UDP is not
encrypted and server packets are not cryptographically authenticated; deployment
requires a trusted server address/network and separate ingress protection.

## Recorded checks and publication boundary

Raw logs remain in the task proof directory; evidence and the independent verdict
bind the verified source and final delivery. Observed checks:

- `raw/verifier-checks.json`: the fresh verifier reran 805 Rust tests successfully: 478 client,
  248 server (including all opted-in database/resilience tests), 61 shared and
  18 passport. Formatting, strict Clippy and 15 packaging tests also pass.
- `raw/store-postgres-final.log`: all 15 selected real PostgreSQL career-store tests
  pass, including public casual XP, duplicate settlement, closed-pool reopen,
  interrupted/custom exclusions and cross-epoch recovery isolation.
- `raw/clippy-final.log` records a successful scoped Clippy run. Packaging and
  license/source checks retain their own logs and manifests; they do not certify
  other OS installers or native screen layout.
- `raw/live-lifecycle-final/report.json` is PASS for real allocation, draft/loading,
  signed cancellation, throttled-CancelQueue fallback through Leave, abandoned
  queue expiry with continuing heartbeats, authenticated stranger rejection, and
  same-account/session reconnect preserving player, progress and worker identity.
- `raw/independent-recovery-v3/report.json` is PASS for lobby adoption of the same
  live worker PID/port/epoch, worker crash, saved Interrupted history, unchanged
  XP/Elo and fresh assignment after durable cleanup. The crash-recovery phase took
  approximately 221 seconds using real timers. Prior failed probe runs remain;
  corrected assumptions were empty bootstrap snapshots and the API's self-profile
  response field, not relaxed product assertions.
- `raw/quick-two/report.json` passes the two-client Quick flow. Both 100-client
  probes pass: ten human-only arenas have 65 ms p95 snapshot gaps; 100 solo-bot
  arenas have 287 ms p95, 463 ms p99 and 622 ms maximum gaps. This is a measured
  bot-heavy degradation, not a recommendation to launch at 100 arenas. The
  operator guide retains a conservative default of 16 and records hardware,
  egress and stationary-input limitations. The first load attempt failed because
  the synthetic client did not retry UDP proof and echoed every repeated auth
  view; the corrected probe follows the native timer-based handshake.

The load probe defaults to 20 requested signed current-pose Transform commands per
second per running human, alongside normal pings and snapshot traffic. It records
actual sent rate/counts, datagrams/bytes, snapshot gaps and process CPU/RSS. The
stationary commands exercise signature verification and dispatch; they do not
establish moving-player combat quality, WAN latency or a production hardware SLA.

Native menu rendering passes all 13 phone-sized screens at 844×390 and all ten
desktop screens at 1280×720. The initial phone Home overflow is retained as failed
evidence; moving secondary navigation to a footer fixes it while preserving
44-pixel controls. The final Home and hero-picker images were visually inspected.
These are native desktop captures in the mobile UI profile, not physical phone QA.

No production 100-player SLA, physical phone acceptance,
production DNS/firewall/database restore acceptance, signed public installer,
TestFlight upload or public hosting is established by this note. Use
[the operator guide](../public-mvp.md), the final proof verdict and a declared target
host before publishing. Existing unrelated phone-practice services were preserved.
