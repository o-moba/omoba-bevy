# iPad movement and practice lane fixes

Version: 0.22.0-rc.3.

## Findings and changes

The full nine-bot real-map regression reproduced three stationary heroes near
their bases. Bot routes used the 3.0 combat target radius, whereas movement
authority enforces the base physical radius of 3.2, plus hero clearance. Their
accepted steering steps were repeatedly rejected by authority. Bot planning now
uses the physical footprint. Tests cover both teams, respawn, and returning to
lane movement after defending against a nearby enemy.

Local movement previously projected only each endpoint out of structures. Sliding
on the physical boundary at 60 FPS produces a chord inside the building when
sampled at 20 Hz. The regression diverged by 0.247 units at frame 26. Local
movement now preserves the same 0.15 clearance as planned routes and uses the
server's swept-disc collision primitive. Authoritative speed/teleport limits and
static forest collision remain intact. This proves and repairs one source of
rubber-banding; it does not rule out separate Wi-Fi loss or stalls on hardware.

The winit iOS app_state messages are an info event (`RedrawRequested`) and an
event-order warning (`AboutToWait`). They are not UDP errors or evidence of an
installation failure. We retain warnings and do not change the native event loop
or claim to repair that upstream warning.

## Verification and use

See `.agent/tasks/TASK-IPAD-MOVEMENT-BOTS-2026-09-23/` for before/after regressions,
full client/server/shared tests and independent verification. PostgreSQL-only
integration tests remain explicitly ignored without an isolated database.

Restart the practice server from current source and rebuild/run the iPad app in
Xcode. No device execution, TestFlight upload, or live playtest is claimed by
the automated checks. Preserve the user's Xcode signing and scheme settings.
