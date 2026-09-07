# Beta readiness — 7 September 2026, iteration 04

Task: `BETA-FULL-MATCH-2026-09-07`. Starting revision:
`797182b403d42464a1c0de2810a34773c495a667` (0.18.0-rc.2).
Work branch: `feature/beta-full-match-2026-09-07`.
Target: a controlled native beta on 8 September, with complete matches and
repeat rounds on the existing Verdant 3D arena.

## Assessment before changes

The previous candidate established offline packaging, authoritative lifecycle,
framed UDP, the new arena, approved replacement creatures and native renderer
evidence. It did **not** establish that ten ordinary participants could finish
a normal-duration match and then another round. Shortened server fixtures and
render-only display actors cannot prove that outcome.

Three observed defects directly affected a first tester: Pause → Restart
discarded the local hero while retaining committed admission, first-run help
had no clickable dismissal, and shared XP could restore HP to a dead player
before its scheduled respawn. The permanent connection banner overlapped the
minimap. Joining a remote host required shell environment configuration.

Progression was also unsuitable for the intended session length: the old
five-player shared-minion baseline reached level 6 only at 850–970 seconds,
before allowing for travel and combat. Bots only used Q and self-sustain;
sequential class assignment systematically split classes by faction. Existing
bots ran indefinitely without success/failure output or full-match assertions.

## Scope of this iteration

Keep the current architecture and address the match loop, onboarding and
operator workflow. Preserve authoritative release rules and use ordinary UDP
bots for acceptance. Improve shared progression and legal ability selection,
protect death/respawn invariants, remove the broken local restart, make help
dismissible, and review actual 720p UI readbacks. Provide practice, host and
remote-join launchers with process cleanup and explicit human/bot counts.

Do not add a shop, external matchmaking, new dependencies, infrastructure or
another renderer. The canonical candidate version advances to 0.18.0-rc.3;
the prerelease suffix preserves SemVer ordering from rc.2. “Beta” describes
the controlled test stage, not an unsupported claim of a finished product.

## Issues exposed by actual runs

Normal ten-peer matches exposed two fill-bot strategy defects that short tests
missed. Heroes continued fighting three separate lanes after an enemy base
was opened; level-6 regrouping now picks an open route. Groups then waited
outside the base because the old siege rule recognized only minion support;
a coordinated healthy hero group now also supplies ordinary tower-tanking
support. Solo and wounded bots retain caution. Structure HP, simulation speed
and attack rules remain unchanged.

Actual 720p captures exposed low-contrast HUD text on pale ground and the
Green sanctuary between the starting camera and hero. The HUD now has a dark
card; the initial/follow camera consistently views either home from its lane
side. The native capture also reproduced an intermittent render-thread shutdown
hang. QA now performs the same primary-window cleanup as the production Exit
button before requesting AppExit. Failed wrappers remain recorded even where
all image files existed.

## Evidence and delivery

The normal-rules two-round run passed, and a fresh verifier independently
replayed all 874 retained snapshots with the corrected checker. These are
actual ten-peer UDP matches in release mode, five players per team, with
ordinary 240-HP lane towers, 650-HP bases, waves, movement and wall-clock time.

| Observation | Round 1 | Round 2 |
| --- | --- | --- |
| Winner | Blue | Green |
| Running to victory | 462.29 s (7m42s) | 450.95 s (7m31s) |
| Admission | Ten stable identities, 5v5 | Same ten identities, 5v5 |
| Objectives | Lane tower before base | Lane tower before base |
| Reset/start | Normal countdown | Clean resources/progression/structures reset and normal countdown |

The first round reached level 2 at 61.29–70.91 seconds, level 4 at
178.67–200.89 seconds, and level 6 at 362.25–376.03 seconds (first–all players).
Q/E/R and self-sustain were observed across the run; one-second sampling is
not a complete cast log and does not prove every offensive slot in every round.
Maximum observed peer snapshot age was 7 ms on loopback; this is not an
Internet latency claim. No unexpected disconnect, abandonment or process
failure occurred in the successful two-round run.

Fresh current verification passed 280 Rust tests and 36 Python tests, strict
all-target Clippy, locked workspace build, formatting and diff checks. Both
source and packaged asset content gates passed. All 90 payload hashes and all
91 ZIP entries were independently checked, with executable permissions retained
and runtime profiles/logs excluded. A practice launch outside the checkout
admitted nine bots, connected the native client, and cleaned up its three owned
processes on termination.

Actual final-package Bevy captures passed with exit 0: four 1280×720 UI frames
in 8.11 seconds, and four Verdant renderer views in 42.40 seconds. Independent
image review confirmed reachable controls, readable HUD/results, no minimap
status overlap and a visible centered starting hero. The result image is
explicitly a synthetic UI fixture; the real victory evidence is the separate
normal-rules match trace. No human input session is claimed.

Implementation and immutable package source:
`bffea105aabc71c480e9ba36841cc66b18e43c45`, version **0.18.0-rc.3**,
macOS ARM64, dev profile with optimized dependencies, clean source.
The final task branch is `feature/beta-full-match-2026-09-07`; subsequent
session/evidence documentation does not alter the packaged production code.

- Native archive: `omoba-0.18.0-rc.3-macos-arm64.zip`, 166,797,958 bytes.
- ZIP SHA256: `bde71c7e1e1846df85f6ca13e5a0372c57f0b4dd331a6b37adc5d4c827535b85`.
- BUILD.json SHA256: `81a8f9d8fa1acf6519ef2b579605c00b14e236da07d5f7cac7dfc5160fb175f5`.
- Evidence: `.agent/tasks/BETA-FULL-MATCH-2026-09-07/`, including spec,
  evidence, fresh verdict and raw successful/failed attempts. The local proof
  archive preserves these separately from the native distribution; task files
  are intentionally gitignored.

The [beta guide](2026-09-07-beta-test-guide.md) is included as package TESTING.md.
Use `./practice.sh`, `./host.sh --humans 2`, or `./join-server.sh HOST:PORT`.

## Changed areas

| Files / area | Result |
| --- | --- |
| `server/src/progression.rs`, `balance.rs`, `release_tests.rs` | Dead XP/respawn invariant and measured full-roster progression |
| `harness/src/bot_ai.rs`, `bin/bots.rs` | Legal skills, paired classes, regrouping, supported sieges and telemetry |
| `client/src/pause_menu.rs`, `help_overlay.rs`, `net.rs`, `game_state.rs` | Safe resume/exit, dismissible onboarding, clear connection and rematch state |
| `client/src/camera.rs`, `match_hud.rs`, `minimap.rs`, `team.rs`, `sprite.rs` | Visible spawn view, readable HUD and uncluttered entry |
| `client/src/beta_ui_qa.rs`, `visual_qa.rs`, `scripts/capture_verdant.py` | Actual native UI/render readbacks and orderly QA window shutdown |
| `scripts/beta_launcher.py`, `package_native.py`, verifier/test scripts | Reproducible native hosting/joining, exact source/binary identity and acceptance checks |
| Cargo version/lock, CHANGELOG, feature inventory, README/RUNBOOK and dated guide | Versioned beta handoff |

The architecture remains the existing authoritative server and native client.
No production dependency, deployment service or infrastructure was added.
Protocol duplication, deeper module decomposition and product expansion are
follow-up work after observing the first human cohort, rather than a reason
to replace the working match loop before this beta.

## External work that remains meaningful during beta

Human testers must establish objective comprehension, controls and balance;
bots cannot provide that feedback. The first human cohort should measure
match length and ability pacing. Actual target hardware needs FPS/frame-time
and memory observations. Remote UDP loss/jitter and non-macOS builds remain
separate coverage. Existing dependency-review dispositions from the previous
iteration are not silently promoted to a new security clearance. None of
these limitations prevents collecting controlled beta evidence; they do limit
claims about unattended public operation and cross-platform readiness.
