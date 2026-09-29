# Shared combat and full-roster offline practice

Version: 0.31.0. Task: COMMON-OFFLINE-2026-09-29.

The new internal `common` crate owns the authoritative state, hero timers/stats, damage receipts, skill execution and passives, utility movement, entity simulation, bot controllers and practice controls formerly in the server. Server compatibility modules re-export the shared implementation. Server dispatch retains admission and pause checks before the shared gameplay command entry. The tick retains its preparation, skill/bot and world simulation ordering; career settlement and network broadcast remain server concerns.

The client offline module now owns only Bevy/channel adaptation, bundled-avatar admission and the practice banner. `common::offline::PracticeSession` advances an explicit local clock and accepts gameplay commands directly. There is no transport or persistence implementation in `common`; its only dependency is the existing shared contract crate. Socket address values remain actor keys for compatibility, and never open sockets. Plain scoreboard/career value types do not submit account results.

Practice configuration starts the local hero at level 6, spends available skill points using authoritative ranking, and fills a regular 5v5 lane roster. The normal practice controls provide a stationary nearby dummy or a configured same-class opponent. All 16 selections resolve the actual engine, including Energy, recasts, orb/terrain effects, control and passives. Leave recreates the combat session and the client restores its online endpoint.

The engine preserves its original AGPL-3.0-only licensing. No third-party production dependency or CI/deployment changes were added. The canonical workspace version and lockfile are synchronized, and iOS source freshness includes common. Asset discovery continues through the existing packaged/worktree asset root.

Validation is recorded under `.agent/tasks/COMMON-OFFLINE-2026-09-29/raw/`: migrated combat/server tests, deterministic all-class online/offline command/tick parity, actual entity damage/death and lifecycle tests, targeted client adapter tests, and one English 852×393 native smoke. The native smoke selects Dawnweaver, casts at a spawned dummy, opens practice controls, leaves and re-enters as Wildspark with no server. This viewport does not certify physical iOS or other devices/languages. Fresh verification remains a separate workflow step.
