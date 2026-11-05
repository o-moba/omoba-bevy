# Public multiplayer MVP (0.22.0-rc.1)

The public service consists of one UDP lobby, a bounded pool of independent game
processes on the same host, and PostgreSQL. The native client starts at the lobby,
keeps its saved device key, and moves to its assigned arena automatically.

## Player experience

Home and the party lobby offer three queues (LR-08); the searching, draft and
loading screens name the queue with a Rated/Unrated badge:

- **Casual PvP:** every human searching Casual plays together, whatever their
  rating or newcomer cohort; after 30 seconds server bots fill the empty seats.
  Ten humans start immediately. Never rated; awards casual XP.
- **Ranked:** ten humans of one rated cohort; never filled with bots. The
  searching screen shows how many players search Ranked, and after 5 minutes
  offers to also search Casual PvP without losing the place in Ranked.
- **vs Bots:** one player or one party against server bots, as soon as a worker
  is available.

The wire names predate the split: `humans_only` is Ranked, `quick` is Casual PvP,
so older protocol 11 clients keep working (an old Quick search plays Casual PvP).
The policy is in [match-progression.md › Queues](match-progression.md#queues-ranked-casual-pvp-and-vs-bots).

Players select an initial hero, inspect their assigned team, lock their choices,
see a shared three-second countdown, and load together. The server waits for all
allocated humans and a durable database acknowledgment before starting gameplay.
Teams are assigned automatically. Capacity waiting is distinct from player search.

Each worker has a frozen roster. Fresh players cannot enter a running match or
replace its bots. A disconnected participant can reclaim the same seat within
the reconnect window. Leaving returns to the lobby; temporary worker ports never
overwrite the saved lobby address. Play again after a saved result returns to the
lobby for a new queue and a new arena.

The game profile uses a private local device key; no wallet or Ekza account is
required for default heroes. Ekza library sign-in is a separate cosmetic account
connection. Existing portal device enrollment/recovery can link multiple devices
without exporting the private game key.

## Saved progress

PostgreSQL owns history, win/loss totals, progression and competitive rating.
Only the game server submits results. Allocation/start, checkpoints, terminal
intent and settlement use durable receipts and a private outbox; retries cannot
award rewards twice. Restarting a client keeps its key and retrieves its profile.

Completed Casual PvP and vs Bots games award 50 XP for a win or 25 XP for a
loss; they do not change competitive rating, even with ten humans. Eligible Ranked
matches keep the existing rating calculation and 150/100 win/loss XP.
Interrupted, development, custom-map and standalone practice matches do not grant
public progression rewards. This public casual policy is `public-casual-v1` with
reason `casual_queue` (Casual PvP) or `allocated_bots` (vs Bots).

In Ranked, newcomer cohorts (fewer than 20 rated matches) remain separated from
experienced players; compatible rating spread is bounded by the existing
300-point policy, shared with the store's rated-start check. Ranked can therefore
wait when the total online count is sufficient but the compatible count is not;
that is what the Casual PvP offer after 5 minutes is for. This MVP has one region/coordinator, not global
regional matchmaking or horizontal distributed lobby scheduling.

## Prepare a host

Build on the target OS/architecture. Native packages are host-specific; a macOS
package is not a Linux server build, Windows installer, or App Store/TestFlight
release. Do not use a development laptop capacity measurement as a production SLA.

PostgreSQL must support the ICU `und-x-icu` collation. Run the existing complete
schema migration once using an administration role, then use restricted runtime
roles as documented in `account-api/README.md`:

```sh
export OMOBA_DATABASE_URL='postgres://ADMIN@DB_HOST/omoba'
./omoba-account-api migrate
```

This prepares career schema v3 and portal schema v3, including optional supporter
tables. The game runtime validates schema and does not perform DDL. The account
HTTP service is optional for native play; it is needed for the existing portal
and account recovery/device management. It must remain behind trusted HTTPS
ingress with its own configured origin and secret; see the account API guide.

Run the lobby under the host's service supervisor with persistent writable state:

```sh
export OMOBA_DATABASE_URL='postgres://RUNTIME@DB_HOST/omoba'
export SERVER_ADDR='0.0.0.0:4000'
export OMOBA_SERVER_ROLE='lobby'
export OMOBA_MATCH_ROOT='/var/lib/omoba/matches'
export OMOBA_CAREER_OUTBOX='/var/lib/omoba/lobby-outbox'
export OMOBA_MATCH_BIND_IP='0.0.0.0'
export OMOBA_MATCH_PUBLIC_HOST='play.example.com'
export OMOBA_MATCH_FIRST_PORT='41000'
export OMOBA_MATCH_CAPACITY='16'
./server
```

Replace example hostnames and credentials. Allow UDP 4000 and the configured
worker range (41000–41015 in this example); publish the same reachable hostname to
clients. The first worker port must be nonzero and the range must fit in a UDP
port number. `GAME_SERVER_ADDR=play.example.com:4000 ./client` connects a native
client. Keep private state directories inaccessible to other users.

The lobby refuses to start when it or its workers bind beyond loopback while
`OMOBA_MATCH_PUBLIC_HOST` is unset or loopback. `OMOBA_MATCH_MAX_DURATION_SECS`
(default 2700, 45 minutes) caps every running match; a capped match, or one that no
human has attended for the 180 s reclaim window plus 30 s, settles as interrupted
and unrated, and its worker exits.

The packaged `launch-lobby.sh` validates required deployment values; review them
before launching. `OMOBA_MATCH_EXECUTABLE` can override the worker binary; by
default the lobby spawns the same server executable. Workers inherit the database
configuration, use a unique outbox, bind their reserved port, and publish atomic
status receipts. A coordinator lock prevents two lobbies sharing one match root.

## Capacity and operations

Capacity is bounded to 1–100 simultaneous arenas (default 16). Ten full-human
arenas can hold 100 people, but 100 people choosing solo bot play need 100 arenas
and run 900 AI heroes. Configure hardware, worker count, database connections and
egress for the intended mode mix. Each public match worker uses one PostgreSQL
connection. The lobby and standalone game career workers use pools of up to four
connections each; budget the optional portal/API pool, monitoring and administration
separately (the RU host checks `max_connections ≥ capacity + 32`). Do not claim
100-player readiness solely from setting capacity to 100.

The RU host runs 16 arenas (UDP 41000–41015) under cgroup limits from a per-arena
formula pending the LR-09 measurement: `MemoryMax = 16 MiB × arenas + 200 MiB`
(500M, `MemoryHigh` 450M), `TasksMax ≥ 3 × arenas + 16` (96), `CPUQuota` 190 % of
its 2 vCPU. Egress is the other budget: `arenas × 10 players × 250 kB/s` of
snapshots must stay under 60 % of the port, so 16 arenas (≈ 40 MB/s ≈ 320 Mbit/s)
need a ≥ 500 Mbit/s uplink and a 100 Mbit/s port carries about six full PvP
arenas; about 1000 concurrent players (100 arenas) need ≈ 2 Gbit/s and more than
one host (LR-57). A lobby restart there leaves the workers running
(`KillMode=process`) and the next lobby adopts them ([ops/vps/ru/README.md › Arena
capacity](../ops/vps/ru/README.md#arena-capacity-lr-09), › Upgrades and live matches).

Local capacity evidence on 2026-09-22 used an Apple M4 Pro (14 CPU cores, 48 GiB
RAM), a development build, loopback PostgreSQL and a 30-second observation after
all clients reached Running. Each synthetic human sent signed current-pose
movement commands at a requested 20 Hz; this exercises admission, signature
verification and dispatch, not active human combat or WAN conditions.

| Scenario | Arenas | Snapshot gap p95 / p99 | Peak worker+lobby RSS | Received payload during full run |
| --- | ---: | ---: | ---: | ---: |
| 100 humans, human-only | 10 | 65 / 84 ms | 161 MiB | 980 MB over 46.6 s |
| 100 humans, each with nine bots | 100 | 287 / 463 ms | 1,641 MiB | 1,027 MB over 66.5 s |

Both probes completed allocation, draft, loading and the observation period;
the bot-heavy case showed material scheduling/network degradation (maximum
snapshot gap 622 ms). Keep the default 16-arena cap until the actual host is
measured with the intended mode mix (the RU host: [ops/vps/ru/README.md › Arena
capacity](../ops/vps/ru/README.md#arena-capacity-lr-09), LR-09).

When every arena is busy, a search whose match is ready waits in line: the lobby gives it
its place and, once arenas have freed recently, an estimated wait, and the searching
screen offers Offline practice in one tap (it leaves the queue). The lobby rewrites
`<match root>/capacity.json` every 5 s and logs `CAPACITY_METRIC` lines when the line
forms, every minute while it lasts and when it clears; the RU host monitor alerts on a
2-minute wait. At 2048 endpoints or 2048 career clients the lobby answers a newcomer
"server full" instead of staying silent, as it does a new source of an IP that already
holds its `OMOBA_LIMIT_ENDPOINTS_PER_IP` share, and the game keeps trying; 2048 is the
lobby's player ceiling, the arenas run out first. A 100-arena configuration is a tested stress
case, not a recommended launch setting. Payload totals include startup/ramp-up
and exclude IP/UDP overhead; they also show that egress capacity matters. Raw
reports are in the task proof under `raw/capacity-human-100-v2` and
`raw/capacity-bot-100`.

Worker `status.json` reports ready/forming/running/settling/finished/failed/recovering,
heartbeat, epoch and result ID. Endpoint assignment waits for a worker readiness
receipt; process spawn alone is insufficient. Watch worker/lobby logs, stale
heartbeats, queue waits, database failures, CPU, RSS and egress. Back up PostgreSQL
and durable match/outbox directories and exercise restore before public launch (on the
RU host `ops/vps/ru/ru_backup.py` does both, LR-03).

A lobby restart rediscovers workers from immutable manifests. Crashed matches are
recovered as interrupted; simulation is not restored mid-frame. Recovery is scoped
to the original worker epoch, so one process cannot settle another worker's live
match or discard its terminal outbox. Unacknowledged results and failed database
recovery retain the allocation until durable recovery succeeds. Never delete a
live match directory or reuse its port manually. Completed directories are archived.

UDP admission validates return path before creating a gameplay endpoint, and
world replication requires authenticated manifest membership. Empty bootstrap
snapshots carry connection metadata only. Commands bind the
saved Ed25519 key, transport nonce, authentication nonce, epoch, match and monotonic
sequence. Per process, admission retains at most 2048 endpoint records (8 per source
IP by default, up to 2048 with `OMOBA_LIMIT_ENDPOINTS_PER_IP`; a new source beyond
2048, or beyond its IP's share, is answered `server_full`, never with more bytes than
its probe), accepts up to 64 probes per second after per-IP (8) and per-network (32)
budgets with 16 kept for new sources, and limits each endpoint to 120 packets per
second. Expired records are swept every 250 ms and the sender's own record is checked
on every datagram, so a full table costs no pass per packet. New profiles, arenas held by one IP and abandoned vs Bots allocations are
limited per source too (LR-16, `ops/vps/ru/README.md` "Admission limits").
Challenges expire after five seconds; validated paths expire after 30 seconds
without traffic. Public datagrams are capped at 12 KiB, with signed command payloads
capped at 8 KiB. Each receive pass stops after 128 packets or two milliseconds.
These limits bound application work; they are not a measured DDoS capacity.

A player who left a running match keeps the seat (a bot plays it). Play on the
lobby then answers with the in-match state and Resume, not a new queue entry,
until the match ends; the next FindMatch after that queues (LR-13).

A signed queue cancellation and signed Leave both clear lobby intent. Only explicit
FindMatch retries renew a waiting entry; ordinary authenticated Home heartbeats do
not prevent its 15-second expiry after lost cancellation packets. Cancelling an
unstarted worker requires the exact allocated account and session.

UDP is not encrypted and server
packets are not cryptographically authenticated: clients require a trusted server
address/network; this does not provide VPN/TLS confidentiality or protect against
an on-path attacker impersonating the server. Upstream network DDoS protection is
still an operator responsibility.

## Reproduce local verification

Use a disposable database, build current sources, migrate, and run:

```sh
cargo test --locked -p client --lib
OMOBA_TEST_DATABASE_URL='postgres://localhost/omoba_test' make test-postgres
export OMOBA_TEST_DATABASE_URL='postgres://localhost/omoba_test'
export HARNESS_SERVER_BIN="$PWD/target/debug/server"
node scripts/test_public_mvp.mjs --scenario=lifecycle --clients=1 --preference=bot_practice
node scripts/test_public_mvp.mjs --scenario=abandoned --clients=1 --preference=bot_practice
node scripts/test_public_mvp.mjs --scenario=recovery --clients=1 --preference=bot_practice --seconds=5
node scripts/test_public_mvp.mjs --clients=100 --preference=humans_only --seconds=30 --input-hz=20
node scripts/test_public_mvp.mjs --clients=100 --preference=bot_practice --seconds=30 --input-hz=20
node scripts/test_public_mvp.mjs --mix=bot_practice:5,quick:5,humans_only:10 --max-wait-secs=60
node scripts/test_public_mvp.mjs --scenario=capacity-wait --clients=1 --preference=bot_practice
```

`make verify-public-mvp` (with `OMOBA_TEST_DATABASE_URL`) builds the server, migrates the
database and runs the lifecycle scenario and the mixed load above (the load with
`--seconds=5`); CI's `postgres` job runs the same target.

`--mix` interleaves players of several queues; `--capacity` sets the lobby's arena count;
`--max-wait-secs` fails the run when a player needs longer to reach Running;
`--allow-large` lifts the client cap from 100 to 1000 (capacity stays at the pool's
maximum of 100); `--queue-only` makes every player sign in and FindMatch without a
handoff, so a thousand lobby clients run from one machine
(`--queue-only --allow-large --clients=1000 --capacity=1`; the `QUEUE_RESULT` line and
`report.queue` give sign-in times, lobby states, `server_full` replies, CPU and RSS);
`--limits-from=<unit>` (Linux) runs the lobby and its workers under a systemd unit's
memory, CPU and task limits. The report records, per arena type, the snapshot gaps, the
time to Running and the snapshot bytes per second per player, and the lobby's and
workers' CPU and RSS while everyone plays; `CAPACITY_RESULT` lines summarize it. The
probe lifts the per-source limits (LR-16) for its own lobby, because all its players come
from 127.0.0.1. `--scenario=capacity-wait` runs one arena: a second vs Bots player must
see its place in line and the lobby must report the busy pool.

The probe launches only loopback processes, tracks all process groups it creates,
drives real signed packets/draft/loading, saves raw logs and reports, and stops its
own processes. By default it sends signed current-pose Transform commands at a
requested 20 Hz per running human. It records actual sent counts, observed rate,
snapshot intervals and process CPU/RSS; stationary commands exercise authentication
and dispatch but do not replace a moving-client, combat-input or WAN test. It reads
`PROTOCOL_VERSION` from `shared/src/protocol.rs` and checks rosters on the live
scoreboard, because vision filters the players in a snapshot.

The recovery scenario requires local `ps` and `lsof`. It stops only the lobby to
verify adoption of a surviving worker, then signals the exact owned worker and
checks durable interrupted history and reassignment. It waits real clocks: an
adopted worker can take about 100 seconds to be declared stale and the recovery
child has a further 120-second gate. Allow several minutes; do not shorten database
leases or fabricate timestamps to make this check pass. The abandoned scenario
leaves a running bot practice and waits the real reaping (about 210 s): Play must
answer in-match until then and allocate a new match after. `--scenario=lobby-restart`
runs only the shorter adoption check.

Node uses built-in cryptography without an added package dependency. The task
evidence records actual results, machine conditions and measured limits. Current
session status and remaining checks are in
[the public MVP session note](progress/2026-09-22-public-mvp.md).
WAN loss/jitter, public DNS/firewall reachability, signed platform installers and
physical phone QA remain separate publication gates.

Ready confirmation can be rehearsed with `--scenario=ready-check --seconds=2
--input-hz=0`: the probe keeps one human unconfirmed for 15 seconds, asserts no
worker was spawned, checks preserved FIFO age, rejects a stale answer and seats
a replacement only after another unanimous confirmation.

`--scenario=termination --seconds=2 --input-hz=0` sends SIGTERM to its own running
worker and asserts a durable Interrupted result with no Elo/XP changes and one
played-match increment. Both scenarios launch their own loopback lobby and need
the same disposable database as the existing probes.

For full-roster Ranked recovery, use `--scenario=recovery --preference=humans_only
--clients=10`. `--scenario=early-recovery --preference=humans_only --clients=10
--seconds=2 --input-hz=0` kills the worker before the ordinary first ten-second
checkpoint. It verifies all ten profiles and histories, unchanged rating/XP,
the actual >=120 s recovery gate and re-entry into a unanimously accepted roster.
Casual (`--preference=quick`, one roster of up to ten) is accepted too. Recovery
keeps its real wall-clock gates; no timeout or lease is accelerated.


## Signed AI fill clients and a full Ranked rehearsal

Build the server and headless tooling in the repository's assigned cargo cache:

```sh
cargo build --locked -p server -p harness --bin server --bin bots
make bots BOTS_PUBLIC=1 BOTS=9 BOTS_SERVER=127.0.0.1:4000
```

Public fill clients default to Ranked (`BOTS_PREFERENCE=humans_only`). They keep
throwaway keys only in memory, explicitly accept the 15-second ready check,
follow the worker handoff, re-authenticate, lock/load and play with the existing
lane AI. `BOTS_PREFERENCE=quick` selects Casual and `bot_practice` selects vs Bots.
A single developer plus nine clients can form ten authenticated Ranked seats.

For an owned loopback lobby and disposable migrated PostgreSQL database:

```sh
export OMOBA_TEST_DATABASE_URL='postgres://postgres:postgres@127.0.0.1:5432/omoba_test'
python3 scripts/public_bots_rehearsal.py \
  --server-bin "$CARGO_TARGET_DIR/debug/server" \
  --bots-bin "$CARGO_TARGET_DIR/debug/bots" \
  --output .agent/ranked-ai-smoke
```

This requires exactly ten clients for Ranked, creates its own lobby/worker root,
raises only its own loopback admission limits and stops only processes it owns.
It checks each client's own identity/fresh snapshot rather than requiring all
opponents to be visible through fog of war. Output roots must be new.

Add `--require-victory --timeout 3600` to play the standard map/rules to a real
result. The clients request saved history and fresh profiles before exiting;
the rehearsal checks every participant's rating delta, XP and exactly one
history result. This can take a full match. The standard run does not alter the
server clock, map HP, damage, debug privileges or Ranked eligibility. A started
run is not a passed terminal proof; its `report.json` is authoritative after
completion.

For a separate **unrated** fast terminal-path check, add
`--test-map examples/maps/rehearsal-victory.json --require-victory --timeout 300`.
This explicit test map disables towers and gives Blue's base one HP. The runner
selects loopback-only `bots --rush-objectives`, which focuses normal movement
and casts on attackable structures rather than waiting for midgame rallies.
Its result
must have zero rating/XP credit, even from the Ranked queue. The runner copies
the fixture into its own root. `OMOBA_MATCH_TEST_MAP_CONFIG` is forwarded to an
allocated worker only if both actual lobby and worker binds are loopback; an
exposed lobby ignores it and logs why. Ordinary map/targeting/bot-difficulty
overrides are still stripped. This fast custom proof never substitutes for the
standard rated match above.
