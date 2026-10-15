# 2026-10-15: clickable community sculpture information

Extended the temporary sculpture pilot with the approved immutable description
contract and a read-only card. Selection raycasts actual visible model triangles;
empty ground, foreground geometry and other modal panels cannot click through.
The card reuses the kit modal, fonts, close/back and scroll controls. Its input
latch blocks the full opening/closing gesture, including touch start, drag and
release, through the shared gameplay/camera policy. The entire approved placement
is checked each frame; authorization loss/replacement closes stale information.

The source GLB and bounded animation profile are unchanged. The native QA art
entry now uses the production UI/input plugins and low-level pointer/keyboard
messages to open the actual cat, scroll the full 1,000-scalar fixture description,
close by the visible control, reopen and close by Escape. The harness is designed to capture five frames including animation behind the
open card. The current native card run is pending because the Mac is locked;
no new native card screenshots or visual PASS are claimed. Automated ECS/input
tests are separate from this pending art proof. The native fixture uses real
API/SQL, but does not establish a multiplayer match or physical-mobile result.

Raw results are under `.agent/tasks/sculpture-info/raw/game` and the coordinator's
native evidence directory. Final completion depends on the independent verifier.
No hosted deployment, crypto activation, credentials or release changes are made.
