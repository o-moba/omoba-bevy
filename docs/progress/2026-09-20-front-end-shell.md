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
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean. The gate was
  red before this task on eight findings unrelated to it (clippy 1.93 is newer than the
  last green run); they are fixed here: `!is_some_and` → `is_none_or` in
  `account-api/src/devices.rs`, a boxed large enum variant in
  `server/src/passport_admission.rs`, a flattened nested `format!` in
  `client/src/career_devices.rs`, constant assertions made `const` in
  `client/src/map_visuals/river.rs`, and `== false` → `!` in
  `account-api/tests/devices_postgres.rs`.
- `cargo test --workspace --locked -- --test-threads=1`: 713 passed, 0 failed,
  34 ignored (the pre-existing PostgreSQL/Apple cases).
- Screen capture (`OMOBA_FRONTEND_QA_OUTPUT`): six screens, `qa-summary.json`
  `"status": "passed"`, the collection capture reporting `preview_status: "Ready"` and
  five animation clips for `agnes`.
- Live flow capture (`OMOBA_FRONTEND_QA_FLOW=1` against
  `OMOBA_MATCH_MODE=practice cargo run -p server`): `qa-flow.json` `"status": "passed"`
  with `screen_trace: ["Home","HeroSelect","Searching","Loading","InMatch"]`,
  `join_committed_from_the_menus: false` and `local_hero_before_lock_in: false`. The
  harness presses the real buttons by setting `Interaction::Pressed`; the session drives
  every transition after the lock-in. `07-in-match.png` is the resulting first playable
  frame.
- Artifacts: `.agent/tasks/FRONTEND-SHELL-2026-09-20/`.

## Remaining risks

- No human ever clicked these buttons: both harnesses are synthetic, and nothing was run
  on a phone or in the mobile UI profile.
- The live run was practice mode (solo start, server bots). The ranked path (`Release`
  mode with a career backend) is covered by unit tests only.
- `SessionUiCommand::LeaveMatch` (the result screen's "Back to menu") drops the session
  and opens a fresh transport; no test covers the server-side seat reclaim after it.
- The server still has no draft phase: hero select runs *before* the queue entry because
  `ClientPacket::Join` carries the loadout. A Wild-Rift-style pick after "match found"
  needs a protocol and server change (`Forming -> Drafting -> Starting`).
- The shell reads the career view from the same UDP session, so against a dead server the
  home screen shows an empty card with "Career profile not loaded yet".
