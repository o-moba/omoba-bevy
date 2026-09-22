# Public multiplayer MVP (0.22.0-rc.1)

The public service consists of one UDP lobby, a bounded pool of independent game
processes on the same host, and PostgreSQL. The native client starts at the lobby,
keeps its saved device key, and moves to its assigned arena automatically.

## Player experience

- **Quick match:** search compatible players for up to 30 seconds, then fill empty
  seats with server bots. A full compatible roster starts immediately.
- **Wait for players:** wait for ten compatible humans; never fill with bots.
- **Play with bots:** one human and nine bots, as soon as a worker is available.

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

Approved completed bot-filled games award 50 XP for a win or 25 XP for a loss;
they do not change competitive rating. Eligible bot-free PvP keeps the existing
rating calculation and 150/100 win/loss XP. Interrupted, development, custom-map
and standalone practice matches do not grant public progression rewards.
This public casual policy is `public-casual-v1` with reason `allocated_bots`.

Newcomer cohorts (fewer than 20 rated matches) remain separated from experienced
players; compatible rating spread is bounded by the existing 300-point policy.
Humans-only queues can therefore wait when the total online count is sufficient
but the compatible count is not. This MVP has one region/coordinator, not global
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
separately. Do not claim 100-player readiness solely from setting capacity to 100.

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
measured with the intended mode mix. A 100-arena configuration is a tested stress
case, not a recommended launch setting. Payload totals include startup/ramp-up
and exclude IP/UDP overhead; they also show that egress capacity matters. Raw
reports are in the task proof under `raw/capacity-human-100-v2` and
`raw/capacity-bot-100`.

Worker `status.json` reports ready/forming/running/settling/finished/failed/recovering,
heartbeat, epoch and result ID. Endpoint assignment waits for a worker readiness
receipt; process spawn alone is insufficient. Watch worker/lobby logs, stale
heartbeats, queue waits, database failures, CPU, RSS and egress. Back up PostgreSQL
and durable match/outbox directories and exercise restore before public launch.

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
sequence. Per process, admission retains at most 512 endpoint records, accepts up
to 64 probes per second, and limits each endpoint to 120 packets per second.
Challenges expire after five seconds; validated paths expire after 30 seconds
without traffic. Public datagrams are capped at 12 KiB, with signed command payloads
capped at 8 KiB. Each receive pass stops after 128 packets or two milliseconds.
These limits bound application work; they are not a measured DDoS capacity.

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
OMOBA_TEST_DATABASE_URL='postgres://localhost/omoba_test' cargo test --locked -p server
OMOBA_TEST_DATABASE_URL='postgres://localhost/omoba_test' cargo test --locked -p server --lib career_store -- --ignored --test-threads=1
export OMOBA_TEST_DATABASE_URL='postgres://localhost/omoba_test'
export HARNESS_SERVER_BIN="$PWD/target/debug/server"
node scripts/test_public_mvp.mjs --scenario=lifecycle --clients=1 --preference=bot_practice
node scripts/test_public_mvp.mjs --scenario=recovery --clients=1 --preference=bot_practice --seconds=5
node scripts/test_public_mvp.mjs --clients=100 --preference=humans_only --seconds=30 --input-hz=20
node scripts/test_public_mvp.mjs --clients=100 --preference=bot_practice --seconds=30 --input-hz=20
```

The probe launches only loopback processes, tracks all process groups it creates,
drives real signed packets/draft/loading, saves raw logs and reports, and stops its
own processes. By default it sends signed current-pose Transform commands at a
requested 20 Hz per running human. It records actual sent counts, observed rate,
snapshot intervals and process CPU/RSS; stationary commands exercise authentication
and dispatch but do not replace a moving-client, combat-input or WAN test.

The recovery scenario requires local `ps` and `lsof`. It stops only the lobby to
verify adoption of a surviving worker, then signals the exact owned worker and
checks durable interrupted history and reassignment. It waits real clocks: an
adopted worker can take about 100 seconds to be declared stale and the recovery
child has a further 120-second gate. Allow several minutes; do not shorten database
leases or fabricate timestamps to make this check pass. `--scenario=lobby-restart`
runs only the shorter adoption check.

Node uses built-in cryptography without an added package dependency. The task
evidence records actual results, machine conditions and measured limits. Current
session status and remaining checks are in
[the public MVP session note](progress/2026-09-22-public-mvp.md).
WAN loss/jitter, public DNS/firewall reachability, signed platform installers and
physical phone QA remain separate publication gates.
