# Combat Test sandbox — 2026-09-23

Task: TASK-COMBAT-SANDBOX-2026-09-23. Base: 9354bee. Isolated branch: codex/combat-sandbox. Version: 0.23.0-rc.1.

Added an opt-in local combat laboratory: direct hero selection, authoritative actor/progression/equipment controls, dummy damage measurements, configurable enemy AI, local two-client duels, minion controls, simulation pause/speed/frame stepping, actual animation graph inspection, combat geometry and persistent presets. The launcher isolates test preferences and local server settings from live career/Ekza configuration. Existing combat, movement, inventory and rendering paths remain the authority; there are no new production dependencies or deployment changes.

Native checks caught and repaired camera follow while the panel is modal and telemetry legibility. Independent review added coverage/fixes for subunit attack speed/damage, request IDs after session reclaim, explicit level assignment after XP gain, and preserving a forced preview while frame stepping. An existing macOS-only account test-fixture socket race was corrected without changing production account logic.

Verification includes the full shared/server/client serial suites (818 passed; 18 existing database fixtures explicitly ignored), transport/combat/release harnesses, real two-client UDP pause/step/damage, native screenshots for every motion preview and geometry, saved configuration loaded in new client/server processes, and a smoke of the documented launcher. Exact current command results and the independent criterion verdict are retained in `.agent/tasks/TASK-COMBAT-SANDBOX-2026-09-23/`.

The native automation injects real UI Interaction events; it does not claim physical mouse or iPad touch testing. Combat Test is desktop/local only, missing clips have explicit fallbacks, energy is absent, and the default saved-preset store is under target (normal rebuilds preserve it; cargo clean does not). Existing user Makefile/Xcode edits and the separate LAN Studio rehearsal are preserved.
