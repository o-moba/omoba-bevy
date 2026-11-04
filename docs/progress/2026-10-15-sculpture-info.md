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
close by the visible control, reopen and close by Escape. After the Mac was
unlocked, the native run captured five frames, reached the end of the 1,000-scalar
description, verified continued paw animation and recorded zero world-input
leaks. It consumed the same placement ID/hash created through OMOBA Web's real
BFF and approved through Studio's browser UI against disposable PostgreSQL.
The first automation attempts exposed a fixed-point click miss and visibility
ordering in the QA harness; bounded input delivery and a visible-mesh target fix
those test issues without changing production picking. Earlier failures remain
in the task artifacts.

CI also found three missing Chinese heading glyphs. The generated display-font
subset now adds only those three, preserving previous coverage and the package
budget. All 1,356 client tests pass (one existing ignored helper). Unsupported
emoji remain an existing bundled-font limitation, explicitly visible in the
stress-test description.

The local fixture uses real API/SQL and preseeded model publication, with
synthetic identities and local-file storage. Raw upload, worker execution,
hosted auth/storage, a multiplayer match and physical mobile remain outside
this evidence. The hosted dev still needs a separately approved rollout.

Raw results are under `.agent/tasks/sculpture-info/raw/game` and the coordinator's
`raw/native-connected` evidence directory. Final completion depends on the independent verifier.
No hosted deployment, crypto activation, credentials or release changes are made.
