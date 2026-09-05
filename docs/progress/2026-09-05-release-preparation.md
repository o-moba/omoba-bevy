# Release preparation — iteration 02 — 2026-09-05

## Baseline and release identity

The [first audit](2026-09-05-3d-readiness-audit.md) is preserved unchanged at
`6214ed1` (implementation baseline `3a74303`). Its 204 passing Rust tests and
findings describe the starting point, not the current candidate. This separate
iteration implements the user's follow-up to save the analysis and prepare the
3D game for release.

Canonical candidate version: **0.18.0-rc.1**. Branch:
`feature/release-candidate-2026-09-05`. Worktree:
`/Users/wotori/git/ekza/omoba-bevy-release-candidate`. Unrelated art changes in the
original checkout are preserved. Frozen implementation contract and raw evidence:
`.agent/tasks/RELEASE-3D-2026-09-05/`.

The target is controlled native Models3d testing on the observed macOS 26.6
ARM64 / Apple M4 Pro host. No other OS, remote network, public deployment or
unattended release is certified. The [package and tester guide](2026-09-05-release-test-guide.md)
provides reproducible commands and the mandatory victory/second-round matrix.

## Implementation ledger

| Audit area / criteria | Current behavior | Evidence |
| --- | --- | --- |
| Admission/reconnect — AC2 | Duplicate Join cannot mutate hero state; reclaim transfers full authoritative state; active-session conflict is rejected. | Full-handler wounded/dead/cooldown regressions; launched real UDP duplicate/reconnect test. |
| Round/roster/debug policy — AC3 | Canonical reset rebuilds players, camps, bosses, waves, structures, buffs and clocks. Release rematch repeats formation/three-second countdown. Reserved seats count toward capacity; 30-second reclaim and ten-second empty-roster reset. Release rejects god/speed commands. | 64 server focused tests; actual UDP cast→Victory→rematch→Starting→Running fixture. |
| Join/wire — AC4 | Retry every two seconds, up to 15 attempts; authoritative admission and visible errors. Hello negotiates ≤1200-byte snapshots with bounded reassembly and epoch/round/tick ordering. Client retains at most 8 complete incoming snapshots. | Shared transport/order regressions; client first-Join-loss/rejection/old-round tests; real 5v5 framed and legacy compatibility checks. |
| Input/UI — AC5 | Current snapshot precedes modal resolution and input/send. Alt+right mouse held orbit, Space recovery, explicit debug flight, modal isolation. Inactive roster excluded; active roster scrolls. | Production input/scheduling/layout ECS regressions; native observation blocked by Computer Use config permission. |
| 3D presentation — AC6 | Local/remote action/death clips, deduplication and explicit round reset; shape+color allegiance; visible hotbar/action feedback and finite VFX. | Current client 131 tests pass, including real AnimationPlayer checks; all 16 hero GLBs have positive-duration idle/walk/attack/cast/death clips. Visual perception remains unverified. |
| Offline/package — AC7 | Mode-scoped optional 2D loads, SDK legacy choices use primitives, executable-relative or explicit asset root. New package and isolated smoke scripts. | Required hero/boss validators pass; package launch evidence pending final build. |
| Siege/bots/metrics — AC8 | One lane tower must fall before base damage. Bots legally upgrade, self-sustain and require health and an allied wave before structure range. Match logs capture outcome/progression/deaths/objectives/disconnects. | Siege/progression regressions ; 15 bot decision tests pass. No actual human pace measurement. |
| Current acceptance — AC1/9/10 | Version/changelog/features/dated ledger and release matrix updated. | Current locked workspace build and all 249 Rust tests pass; final lint/package/fresh verification in progress. |

Bot structure rule: enemy structures define conservative 26-unit danger circles.
Entry requires ≥50% HP and two living allied minions within 24 units, both closer to
the structure than the hero. Otherwise retreat or clip advance; protected bases
are never siege targets. Self-sustain requires effective need≥20% of the maximum resource,
legal unlock, enough mana and elapsed shared cooldown. Bots remain test support.

## Measured transport and deterministic progression

The current live5v5 probe reconstructed an **8438-byte** snapshot containing
10 players, 8 structures and 18 minions from 8 frames, maximum 1200 bytes; 240 complete snapshots
advanced monotonically. A legacy JSON probe measured this host's kernel send
ceiling at 9216 bytes without changing settings. Counts/size depend on runtime
state. Full payload budget 65,507 bytes, four retained assemblies, two-second expiry.
A new transport explicitly clears its ordering watermark; server epoch clock
rollback therefore requires Retry. Loopback results do not certify Wi-Fi/remote.

XP rewards remain unchanged: 32 XP per minion shared across the receiving team;
five recipients get 7/7/6/6/6 by stable identity. Nine opposing minions yield
63/63/54/54/54 XP per wave. The baseline assumes all opposing minions die at wave
spawn, excluding travel/combat/jungle/missed kills; first wave 10 s, later 60 s.

| Milestone | First two players | All five players |
| --- | ---: | ---: |
| Level 2 | 70 s | 70 s |
| Level 4 | 370 s | 430 s |
| Level 6 | 850 s | 970 s |

This lane-only estimate exposes a pacing concern: it is slower than the proposed
5–7 minute ultimate target even before travel/combat. The proposed 8–12 minute match
length is a design hypothesis, not a measured result. Record real full-roster
sessions before class/XP tuning. Gold is income with no spending path yet.

## Remaining certification gates

**Release certification: BLOCKED. Package: INTERNAL REVIEW ONLY.** These external obligations are separate
from implementation acceptance; neither tests nor this checklist make them PASS.

- **RG1:** five new testers, at least four onboarding successes in two minutes;
  observe Join and combat readability at 1280×720/1366×768. Current Computer Use
  call fails with error -10005: cannot read `/Users/wotori/.codex/config.toml`,
  permission denied (OS error 13). No screenshots or gestures were observed.
- **RG2:** five complete two-human/eight-bot 5v5 sessions through victory and a
  complete second round, with progression/outcome/confusion records.
- **RG3:** declared hardware/FPS budgets and 30 minute soak with frame/tick timing,
  memory and snapshot age/send error measurements.
- **RG4:** package gameplay on every supported OS and intended network with
  loss/reorder/jitter evidence. Current scope has only this macOS host/loopback.
- **RG5:** current dependency/provenance dispositions and exact final package
  identity. Current GitHub review found 16 open alerts (6 high, 6 medium, 4 low).
  The applicable crossbeam-channel issue is patched locally at 0.5.15; GitHub
  status does not close until the default branch is updated. Remaining alerts
  have platform/path dispositions in the [distribution review](2026-09-05-distribution-review.md). Four asset license conflicts
  block distribution: el-bueno, slime-green, slime-blue and wendigo-hollow. Their
  embedded/linked restrictions conflict with registry CC0 labels; the registry's
  own license covers metadata, not an override of avatar licenses. Resolve with
  rights-holder evidence or replacement assets. The package script requires an
  explicit internal-review mode and does not authorize redistribution.

The three optional 2D validators still fail for the known missing Orchard Comet
Centaur runtime sheet and absent world 2D task provenance. Their current raw
failures are retained; this milestone does not certify 2D. Audio, broader roster
polish, economy/shop and coordinated class balance remain follow-up work.
