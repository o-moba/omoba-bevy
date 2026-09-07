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

Implementation and current verification results are recorded in the task's
`evidence.md`, `evidence.json` and raw artifacts; the independent verdict must
pass every frozen criterion before this task is described as complete.
The operator instructions are in [the beta guide](2026-09-07-beta-test-guide.md).
Final measured results and delivery identity are added here after verification.

## External work that remains meaningful during beta

Human testers must establish objective comprehension, controls and balance;
bots cannot provide that feedback. The first human cohort should measure
match length and ability pacing. Actual target hardware needs FPS/frame-time
and memory observations. Remote UDP loss/jitter and non-macOS builds remain
separate coverage. Existing dependency-review dispositions from the previous
iteration are not silently promoted to a new security clearance. None of
these limitations prevents collecting controlled beta evidence; they do limit
claims about unattended public operation and cross-platform readiness.
