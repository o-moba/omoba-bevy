# Native beta — 8 September 2026

Candidate version: **0.18.0-rc.3**. This is a controlled native 3D beta with
human players and fill bots, using the Verdant Confluence arena. The delivered
package is built and checked on macOS ARM64. Python 3.9+ is needed for the
convenience launchers; no Python packages, Rust toolchain or source checkout
are needed by testers. Other OS builds, an Internet matchmaking service and
a signed/notarized installer are not supplied.

## Play alone first

In Terminal, open the extracted package directory and run:

```sh
./practice.sh
```

This starts one release server, nine ordinary bots and your game window.
Choose a hero and class, then **Join**. The server forms two teams of five;
play begins after a three-second countdown. Team selection is a preference;
the server balances the final teams. Closing the game or pressing
Ctrl+C in the launcher terminal stops its server and bots too.
If port 4000 is occupied, use `./practice.sh --bind 127.0.0.1:4010`.

## Play with other testers

One operator hosts the match and states how many humans will join, including
the operator if they will play. For two humans and eight bots:

```sh
./host.sh --humans 2
```

Leave that terminal running. Each tester starts the game with the host's
IPv4 address (or a hostname resolving to IPv4) and UDP port:

```sh
./join-server.sh 192.168.1.20:4000
```

The operator can join from another terminal with
`./join-server.sh 127.0.0.1:4000`. For two game windows on one computer, use
distinct profiles, for example `./join-server.sh 127.0.0.1:4000 --profile tester-2`.
A profile preserves its reconnect identity; simultaneous windows must not
share one profile. `--humans` accepts 1–10; the launcher adds exactly
`10 − humans` bots. The match waits for all expected humans to press Join.
For ten humans, no bots are started.

Host and clients need UDP connectivity to the selected port. Prefer the same
LAN for the first session. Remote hosting requires an operator-provided
reachable address; this package does not configure firewalls, routers or a
public service. Stop the host with Ctrl+C when the session is over.

## Play a complete match

- Click open ground to move. Click an enemy to approach and attack with Q.
- Use Q/W/E/R or click the four ability buttons. Unlocks are levels 1/2/4/6;
  press U for skill upgrades. The hotbar shows mana, cooldown and rank.
- Follow allied minions down a lane. Destroy an enemy lane tower to remove
  the enemy base's protection, then destroy the base to win. The HUD states
  the current objective. Jungle camps and the two bosses are optional.
- Alt + right mouse orbits the 3D camera; Space recovers the hero view;
  Y toggles follow. The minimap can move your view.
- The first-run help has a **Play** button; Escape dismisses it. F1 reopens
  help. Escape opens the pause menu, which resumes or exits the game; an
  online match continues while the menu is open.
- Death is temporary: wait for the normal respawn at your base. Shared XP
  still counts while dead but cannot revive you early.
- After a base is destroyed, the result panel counts down to an automatic
  new round after ten seconds. Keep the game open: the server restores
  structures and player progression, forms the roster and counts down again.

Gold is currently a score/reward counter; this beta has no item shop. Bots
support complete sessions but are not a substitute for human balance testing.

## Recovery and reporting

Connection errors show a Retry action. Reconnect using the same profile to
reclaim your state during the 30-second reservation window. Do not start a
second window with that profile while the first is still connected. If a
player leaves, wait for reconnection or its reservation to expire; a new
round needs a full roster. When everybody leaves, the server resets after
the empty-roster grace period. Stop/restart the host for a fresh hosted session.

Every convenience launcher prints a `sessions/<timestamp-id>` directory with
`server.log`, `bots.log` and/or `client.log`. Send the operator that directory,
the package `BUILD.json`, OS/hardware, what happened and approximately when.
The host log contains `MATCH_METRIC` records for deaths, level-ups, objectives
and victory. Do not send the `user-data` directory; the logs and build identity
are sufficient for first triage.

## Operator acceptance before inviting the group

1. Run local practice, join, dismiss help and move/cast at 1280×720 or larger.
2. Host the announced human count; confirm ten total players, two teams of
   five, and the countdown. Verify one client's same-profile reconnect.
3. Play through lane tower destruction, base destruction, the result screen
   and the next round. Capture logs for each session, including unsuccessful ones.
4. During the beta, record first fight, first level-up, ultimate unlock, match
   duration and whether new players understand the objective. Record actual
   hardware and performance before setting minimum-system claims.

The dated readiness record accompanies the repository at
`docs/progress/2026-09-07-beta-readiness.md`. Automated normal-rules matches and
native image captures are separate evidence from human playtests. Human
onboarding/balance, hardware performance, remote loss/jitter, other platforms
and the remaining dependency review are not certified by those checks.

## Maintainer build and verification

```sh
python3 scripts/package_native.py --output /tmp/omoba-0.18.0-rc.3
python3 scripts/test_beta_launcher.py
```

The package is self-contained: `BUILD.json` identifies its version, revision,
source diff, platform/profile and shipped file hashes. `ASSET-REVIEW.json`
checks the actual packaged approved models and excludes the removed assets.
The default dev profile uses optimized dependencies; a separately built
release profile requires its own binary identity and performance evidence.
