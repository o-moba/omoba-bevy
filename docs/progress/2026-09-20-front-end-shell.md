# 2026-09-20 — Front-end shell and pre-match hero select

## Goal

Give the client a front end. Until now `client/src/lib.rs` added every gameplay plugin
at startup, `client/src/world.rs` built the 3D map in `Startup`, and the class/avatar
picker was an overlay on top of that already-running world, so a player chose a hero
inside a live match scene. There was no home screen, no way to look at a profile, an
avatar or a match history before queueing, and no separation between "in the menus" and
"in a match".

## Changes

- **New `client/src/frontend/`** with a Bevy `States` machine (`AppScreen`): `Home`,
  `Card`, `Collection`, `HeroSelect`, `Searching`, `Loading`, `InMatch`, `PostMatch`.
  This is the first `States` usage in the workspace; screen lifetime is now
  `DespawnOnExit`, not ad-hoc resource flags.
  - `home.rs` — profile card, PLAY, connection status, entries into the collection,
    match history, friends and the account modal. Rebuilds itself when the career view
    changes.
  - `card.rs` — the profile card and its editor: main class, showcase avatar, accent,
    win-gated title. Stored in `profile_card.json` next to the client preferences, so
    the preferences schema is untouched.
  - `collection.rs` + `preview.rs` — every shipped, owned and community avatar with a
    live 3D preview on render layer 28 (the supporter aura preview owns 29), rendered
    into an image through the same `RenderTarget::Image` pattern. Drag to turn, one
    button per animation clip the glTF actually declares.
  - `searching.rs` — the server's `QueueView`, or the formation counters when the ranked
    queue is off, plus cancel.
  - `loading.rs`, `postmatch.rs` — the two ends of a match.
- **`client/src/team.rs`**: the picker is now a screen (`OnEnter(AppScreen::HeroSelect)`),
  opaque instead of translucent, with a Back button, and locking in a side moves the
  shell to `Searching`. Thumbnails are preloaded at startup because the card and the
  collection need them without opening the picker.
- **`client/src/net.rs`**: `ClientSession::join_rejection`/`join_blocked` accessors,
  `SessionUiCommand::LeaveMatch` (drop the session and go home from the result screen),
  teardown asks for a screen through `PendingScreen` instead of spawning UI itself, and
  the connection panel moves to the bottom-right and hides on the pure menu screens.
- **`client/src/career.rs`**: `open_history_modal` / `open_friends_modal` /
  `open_profile_modal` so the shell reuses the existing career modals instead of
  duplicating them, and the in-match career bar is hidden behind the front end.
- **`client/src/input_context.rs`**: every menu screen counts as a modal, so world input
  stays inert behind the shell.
- **`client/src/frontend_qa.rs`**: `OMOBA_FRONTEND_QA_OUTPUT` captures the shell screen
  by screen and fails the run if a screen leaves a 1280x720 viewport.

## Checks

- `cargo fmt --all -- --check`, `cargo check -p client --locked --all-targets`: clean.
- `cargo test -p client`: 399 passed. Slices covering the rest of the workspace
  (`shared`, `omoba-passport`, `skills`, `arena-sync`, `server`, `harness`,
  `omoba-account-api`): 311 passed, 0 failed. A single `cargo test --workspace`
  invocation did not fit on the machine's disk.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: seven errors, all
  pre-existing on a clean tree (`account-api/src/devices.rs`,
  `server/src/passport_admission.rs`, `client/src/career_devices.rs`,
  `client/src/map_visuals/river.rs`) under clippy 1.93.
- Screenshot capture: six screens, `qa-summary.json` `"status": "passed"`, the collection
  capture reporting `preview_status: "Ready"` and five animation clips for `agnes`.
  Artifacts in `.agent/tasks/FRONTEND-SHELL-2026-09-20/artifacts/`.

## Remaining risks

- No live match was played through the new flow: `InMatch`/`PostMatch` are covered by a
  state-machine test, not by a real 5v5, and nobody clicked the buttons by hand.
- `SessionUiCommand::LeaveMatch` drops the session and opens a fresh transport. It is
  exercised by no test yet; a server-side seat reclaim after leaving is unverified.
- The server still has no draft phase: hero select runs *before* the queue entry because
  `ClientPacket::Join` carries the loadout. A Wild-Rift-style pick after "match found"
  needs a protocol and server change (`Forming -> Drafting -> Starting`).
- The shell assumes the career view arrives over the same UDP session, so on a dead
  server the home screen shows an empty card with "Career profile not loaded yet".
