# MVP playtest script (10–20 minutes)

Use this checklist in **one sitting** with a clean shell. Follow [RUNBOOK.md](../RUNBOOK.md) for commands and [README.md](../README.md) for controls.

**Goal:** Confirm the MVP build launches, connects, and is playable for a short session without undocumented steps.

---

## Before you start (about 2 minutes)

- [ ] Read `README.md` prerequisites and run `cargo build --workspace` if you have not built yet.
- [ ] Close stray game processes from earlier runs: `make stop`.
- [ ] Note your OS and GPU/driver version for the bug template if anything graphics-related fails.

**Expected:** Workspace builds without errors; `make stop` exits cleanly.

---

## Session A — Launch and connection (about 4–6 minutes)

- [ ] From the repo root, run `make start`.
- [ ] Confirm the **server** terminal shows a listening line for `0.0.0.0:4000` (or your overridden `SERVER_ADDR`).
- [ ] Confirm **both clients** eventually log `First snapshot received` (see `RUNBOOK.md` → Expected Startup Log Output).
- [ ] On each client window, complete **team** and **character** selection so both players spawn.

**Expected:** Two windows respond to input; no infinite hang on “Connecting…” after the server is up.

**If blocked:** Use the troubleshooting table in `RUNBOOK.md` (port mismatch, stale processes, wrong `GAME_SERVER_ADDR`).

---

## Session B — Match flow and basic combat (about 5–8 minutes)

- [ ] Verify the match leaves the lobby overlay and enters **running** gameplay for both clients.
- [ ] Move the camera (locked vs unlocked per `README.md`) and confirm the view follows your hero in locked mode.
- [ ] Select a target with **Tab** or **middle mouse**, then cast with **Q** or the on-screen skill button; repeat at least twice.
- [ ] Engage minions or structures long enough to see **damage**, **death/respawn**, or **base** pressure (no strict win required).
- [ ] If time allows, trigger **victory** or **rematch** UI by playing toward a base kill or waiting for a decisive outcome.

**Expected:** Abilities fire when a valid target exists; HUD shows progression fields consistent with [docs/features.md](features.md); victory/rematch overlays appear when the match ends.

---

## Session C — Clean shutdown (about 1–2 minutes)

- [ ] Stop foreground client with **Ctrl+C** if needed, then run `make stop` from the repo root.
- [ ] Confirm no `server` / `client` processes remain (Activity Monitor / Task Manager / `ps` as you prefer).

**Expected:** Port 4000 is free for a subsequent `make start`.

---

## After the session

- [ ] File notes using [bug-report-template.md](bug-report-template.md).
- [ ] Mark whether each issue is **MVP-blocking** or **deferrable** using [tasks/MVP-CHECKLIST.md](../tasks/MVP-CHECKLIST.md).

**Total time:** Approximately **12–18 minutes** at normal pace; stay within **20 minutes** by skipping optional victory/rematch if needed.
