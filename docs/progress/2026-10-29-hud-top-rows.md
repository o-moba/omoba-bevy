# Shared phone HUD layout contract — 2026-10-29

The selected target is smaller (144×40 at reference size), and both it and all
four allied cells stay at the safe top. A dedicated passive enemy death row
sits beneath allies. Living nearby lock targets keep their right-side slots.
Settings/chat move into adjacent 44px cells below the death row.

`hud_layout/phone_rows.rs` is a std-only geometry implementation used directly
by `HudLayout::phone`, left-stack/movement reservation, and the ally renderer's
actual cell dimensions. The fast command is:

```
python3 scripts/check_phone_hud_layout.py
```

It compiles only that file with `rustc --test`, launches no game/Bevy/window/GPU,
and checks top anchors together, target center/size, real ally cell fit, passive
and live portrait regions, services/left stack, former cutoff continuity and a
deterministic width/inset grid. Deliberately lowered avatars/target and wrong
cell fit are rejected. These same tests also run under normal Cargo/CI tests.
The old tests allowed a lowered ally row; that unwanted rule is replaced.

On infeasible narrow 568/693 profiles, the 104px map sits below the header instead
of displacing avatars/target. Quick-buy/feed and joystick reservation follow its
actual bottom. This is explicit geometry behavior, not a full visual matrix.

Enemy death roots carry identity/art/public time only and cannot act as target
buttons or produce direction arrows. Alive/dead transitions reject queued stale
actions. Desktop keeps the shared right column. Focused ECS tests cover these
behaviors without rendering. The opt-in capture fixture waits for InMatch before
sending its one-shot touch, avoiding frontend-loading timing misses.

Evidence lives under `.agent/tasks/HUD-TOP-ROWS-20261029/`. Verification uses only
one English791×360 native preview of the changed top state. Android build450012
uses the existing Beta signer and shared cacheB. Actual build/install/test states
are recorded separately in the task receipts. No server/protocol, production,
release publication, dependency or CI configuration changes are part of this fix.

The fast contract passes 8 tests over 13365 layouts; test execution is about 0.14s,
about 1s including standalone compilation. Independent algorithm/runtime review
passes. The client passes 1533 tests, and formatting/both Clippy configurations
pass. One unchanged server settlement test failed in the broad parallel run and
passed when rerun in isolation with the same binary; remaining shared/doc tests
were completed separately. The Python suite retains its known single adjacent
CJK-font manifest failure (612 run, 9 skips); 55 iOS tooling tests pass without a
physical iOS run. Raw receipts preserve all failures and retries.

The final native state verifies both top anchors, the separate passive death row,
live right-side targets and neighboring settings/chat. Source/algorithm/visual
review passes. Compact name clipping retains the established policy; the
incidental Chat hover hint covers no painted portrait/control in the capture.

Clean runtime `9c85168` produced Beta0.45.0 build450012. It is installed over450011
on OPPO PKB110 with a matching APK hash and successful process launch. The phone
is asleep behind keyguard, so physical rendering/gameplay/multitouch remain
unverified. APK SHA256: `1a246bf1a8c3eb798442f5326ba8cda414afce79481754b8bba20a8817969f19`.
The retained APK, portable before/after comparison and raw receipts are inside
the task folder. Superseded debug APKs450009–450011 were removed while retaining
their proof/checksums/previews; shared Cargo caches remain untouched.
