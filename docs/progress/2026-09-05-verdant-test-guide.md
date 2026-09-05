# Verdant Confluence native candidate test guide — 2026-09-05

Version: **0.18.0-rc.2**. **Controlled playtest candidate.** Public/unattended
release certification is **BLOCKED** until the external gates below have real
evidence. The package's `BUILD.json` identifies exact binaries/assets, source
revision, dirty-source fingerprint, host and build profile. Only the recorded
macOS ARM64 host is tested; no Windows/Linux distribution claim is made.

## Build and launch

Python 3.9+ and the locked Rust toolchain/dependencies are build prerequisites.
No Python packages are required. From a checkout:

```sh
python3 scripts/package_native.py --output /tmp/omoba-0.18.0-rc.2
```

Default profile is `dev` with the repository's optimized dependencies, for
controlled functional testing. `--profile release` creates an optimized build
that requires its own performance validation. Neither profile changes match
rules. First dependency acquisition can need internet. Packaged game startup
uses shipped assets without SDK downloads or a source checkout.

Run each command in a separate terminal, even with the terminal's directory
outside the checkout. Set `PACKAGE` to the absolute package directory:

```sh
PACKAGE=/tmp/omoba-0.18.0-rc.2
"$PACKAGE/launch-server.sh" > /tmp/omoba-server.log 2>&1
"$PACKAGE/launch-client.sh" > /tmp/omoba-client.log 2>&1
"$PACKAGE/launch-bots.sh" --count 8 --server 127.0.0.1:4000 > /tmp/omoba-bots.log 2>&1
```

Two human clients plus eight bots form 5v5. Use distinct `OMOBA_CLIENT_CONFIG_DIR`
directories for two clients on one computer; each config represents one session.
Choose a shipped roster hero. Both teams may choose the same hero to inspect
allegiance cues. Client address: `GAME_SERVER_ADDR=host:4000`; server bind:
`SERVER_ADDR=0.0.0.0:4000`. The protocol uses UDP port 4000. Configure a test
network deliberately; no public server is deployed by this package.

Release is the default: ten admitted players, balanced teams, three-second
countdown, first wave ten seconds after Running. Set `OMOBA_MATCH_MODE=dev`
explicitly for an instant-start solo diagnostic session. Development permits
server debug commands; release rejects them. Launcher keeps the debug UI off.
Stop terminal processes with Ctrl+C. Preferences remain in package `user-data`
unless overridden. Optional 2D content is not certified; do not switch this
candidate's fixed Models3d launcher to 2D for a release acceptance run.

## Controls and match rules

Click ground to move, click a hostile target to select/request Q; Q/W/E/R cast,
U upgrades. Hold Alt+right mouse to orbit; Space recenters; Y toggles follow.
Help/settings overlays consume gameplay input. A protected base requires one
of that team's lane towers to be destroyed first. Push with your minion wave.
Local/ally/enemy ring shapes supplement team colors. Read the hotbar and brief
action messages for mana, cooldown, locked skills and approach/protection.

Disconnected seats reserve full hero state for 30 seconds. All connected
players leaving resets the abandoned round after a ten-second grace, including
reservations. Rematches preserve connected identities/loadouts, reset all
gameplay state and wait for the release roster again. Repeated Join is
idempotent. Gold is recorded but has no shop/spending path in this milestone.

Scenario bots upgrade unlocked abilities, use class self-sustain and require
health plus an allied wave before entering enemy structure range. They are
test support, not evidence of human balance or coordinated strategy.

## Mandatory session matrix and captured evidence

For every run attach `BUILD.json`, OS/GPU, display size, logs, tester counts and
connection conditions. Record issues with timestamp, expected/actual behavior
and reproduction steps. Preserve server `MATCH_METRIC` lines: match identity,
start/victory duration, progression milestones, deaths, objectives and departures.

| Scenario | Required observation |
| --- | --- |
| Onboarding | New player reaches all roster choices and Join at 1280×720 and 1366×768; moves, attacks and names next objective within two minutes. |
| Input | Open help/settings/pause, click text/background, press QWER/U/Y; no unintended action. Orbit intentionally, then Space and resume movement. |
| Combat | Same hero on both teams; local/remote attack/cast/death/respawn, target protection, feedback and hotbar are readable at normal/far zoom. |
| Formation | Two humans plus eight bots; release formation/countdown enters Running once. |
| Reconnect | Disconnect a wounded/progressed player, reconnect within reservation; same state advances normally. Repeated Join cannot heal/reset. Active duplicate session shows rejection. |
| Full match | Play to victory; record winner/duration, deaths/objectives, first fight, levels 2/4/6, disconnects and confusion points. |
| Second round | Request rematch and complete another round. No old movement/cooldowns/buffs/camps/progression or ghost players; underfilled roster waits. |
| Abandonment | Leave all clients, wait over ten seconds; new group gets a clean match. |

Proposed pacing targets are hypotheses: first fight ≤60 seconds, level 2 ≤2
minutes, ultimate around 5–7 minutes, finish 8–12 minutes. The deterministic
shared-XP baseline is not a measured match and currently suggests a slower
ultimate with all five players sharing only minion XP. Tune from recorded play.

## Asset review and native visual checks

The four disputed models and their previews are excluded. The current package
has 15 retained imported heroes, King Mutatio, six original environment GLBs,
and procedural minions/guardian. `ASSET-REVIEW.json` must report PASS. The gate
compares actual bytes against a reviewed inventory, rejects denied content even
when renamed, checks required references and embedded actor permission metadata.
`BUILD.json` hashes the exact candidate. Retain both with feedback.

The supplied Blender scene is preserved. Runtime walking surfaces were adjusted
within the visual layer so heroes cross the bridge/river and mitered base ramps
without changing server routes or collision. Sanctuaries are rotated 45 degrees
to open the existing spawn paths. During testing walk off each base through a
side and corner ramp, cross all three river crossings, inspect both faction
minions and destroy a tower before checking the next round's restoration.

To reproduce actual renderer images with a fresh server, two scripted peers,
normal admission/movement and isolated configuration:

```sh
python3 scripts/capture_verdant.py --package /tmp/omoba-0.18.0-rc.2 --output /tmp/verdant-capture
```

This explicit QA mode captures overview, river, sanctuary and normal follow views after scene
readiness and adds tagged display fixtures built by production creature factories.
Its three source-directed cameras are supplemented by the normal perspective
follow camera. The report
marks it as automated renderer evidence, not a human play session or a benchmark.
Normal launcher startup leaves the capture mode disabled.

The [historical distribution review](2026-09-05-distribution-review.md) is copied
into the package along with attribution/provenance records. Its four-asset
finding is addressed through exclusion; the remaining dependency dispositions
and external gates still need their stated evidence.

## External certification gates — not certified

| Gate | Procedure and evidence needed for PASS |
| --- | --- |
| RG1 Human onboarding/visual clarity | Five new testers, four succeed within two minutes; observed target-resolution layout, modal/camera recovery and combat readability. Automated native screenshots cover rendering; they do not establish human success or interaction quality. |
| RG2 Full-match/pacing | Five consecutive two-human/eight-bot 5v5 sessions, each through victory and a complete second round; metrics and issue records. Automated shortened lifecycle proves logic only. |
| RG3 Stability/performance | Declare hardware/FPS budgets first; 30-minute soak with frame time, memory trend, server tick time, snapshot age/bytes and send errors. Process-alive startup is insufficient. |
| RG4 Platform/network | Validate package on every declared OS and intended LAN/Wi-Fi/remote environment with measured loss/reorder/jitter. Loopback is not remote-network certification. |
| RG5 Distribution review | Triage current dependency alerts for shipped/platform reachability; record dispositions and retain the passing asset inventory/final build identity and resolve remaining dependency dispositions. Historical alert counts are not a current review. |

## Transport budgets

Current client and scenario bots negotiate protocol 1 with Hello. Snapshot JSON
is split into ≤1200-byte UDP datagrams; maximum whole payload 65,507 bytes,
at most four retained assemblies with two-second expiry. Complete snapshots
are ordered by server epoch, round and tick; incomplete/stale data never applies.
The client input queue retains at most eight complete snapshots. Join retries
every two seconds, up to 15 attempts, with visible rejection/retry state.

Legacy scripts without Hello still receive whole JSON with the same 65,507-byte
application ceiling. The host kernel can impose a smaller send limit (9,216 on
the audited macOS host); server errors are reported, not silently truncated.
Use negotiated framing for new clients. Server epochs use start UTC nanoseconds;
a clock rollback requires a transport Retry to reset the ordering watermark.
