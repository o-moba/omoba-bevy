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

## Beta polish pass

- Home: the card's hero is rendered live in 3D (`AvatarPreview` on render layer 28) next
  to a right-hand action rail; the header carries the build version and the connection
  status, and a last-match line appears under the card once a result exists.
- Hero select: a live panel shows the chosen avatar in 3D with the class line and that
  class's Q/W/E/R kit; the wallet and account controls collapsed into a single strip
  under the grids; the header carries the connection status.
- The floating connection panel and the in-match career bar now stay out of every menu
  screen, which print their own status instead.
- `AvatarPreview` is a `FromWorld` resource so it exists during the startup state
  transition, when the first screen is entered.
- Layout holds at 1024x640 as well as 1280x720: the class row wraps, the picker drops to
  one avatar row on a short window, the hero panel and the collection preview are sized
  from the window, and the harness fails a run if a screen root leaves the viewport.

## Review pass (2026-09-21)

A code review of the branch found that the tests and captures only covered the happy
path - a live server and an accepted join - while every defect sat on a failure path.
Fixed, each with a test on that path:

- **Leaving did not leave.** `SessionUiCommand::LeaveMatch` only swapped the local UDP
  transport and kept the session id, so the server answered the next join with
  `SessionActive` or reclaimed the old seat with the old hero. There is now a `Leave`
  client packet (`server/src/career_runtime.rs::leave_match`): the seat and any queue
  entry go at once, nothing is kept for a reclaim, the endpoint and its career
  authentication stay. The search screen's Cancel uses the same path, because a signed
  `CancelQueue` did nothing on a practice or dev server. Server test:
  `a_deliberate_leave_frees_the_seat_and_lets_the_same_session_pick_again`.
- **A rejected join stranded the player** on a picker whose lock-in was dead and whose
  only recovery UI had been hidden. The driver now abandons the dead join
  (`ClientSession::abandon_join`) and hands the reason to the picker header through
  `JoinNotice`. Test: `a_rejected_join_returns_to_a_working_picker_with_the_reason`.
- **A reconnect looked like leaving.** The driver keyed "still mine" off
  `join_flow_committed`, which a teardown clears. It now uses
  `ClientSession::has_committed_join`, which survives a teardown. Tests:
  `a_reconnect_in_the_middle_of_a_match_keeps_the_match_on_screen`,
  `leaving_the_match_goes_home_and_stale_snapshots_cannot_pull_back`.
- **The transport owned a screen.** A pre-join teardown requested hero select from
  anywhere. It no longer requests anything; the menus retry the connection every five
  seconds on their own (`retry_connection_from_menus`).
- **Lock-in race.** `team_select_ui_system` is ordered before
  `ClientNetPipeline::SendCommands`, with a three-frame grace in the driver as a guard.
- **Preview leaking into the world.** Layer tagging runs until `SceneInstanceReady`
  instead of for 240 frames, the rig lives at y = -2000, and the model is released when
  no screen shows it. `NoAnimations` is reachable and refreshes the clip row.
- **Card, collection, small items.** Store/community showcase avatars survive loading
  (shape check instead of a catalogue lookup at startup); the collection grid scrolls;
  the accent swatch repaints; `automation_bypass` is decided once and covers
  `*_QA_OUTPUT`; the home footer stopped advertising F1.

The live-flow harness now also leaves the match and comes back with another class on the
other side. Against a practice server it records
`Home, HeroSelect, Searching, Loading, InMatch, Home, HeroSelect, Searching, Loading,
InMatch` with `rejoined_after_leave: true`, and the server logs `event=leave` with no
`Rejecting session id reuse` and no `Reclaiming`.

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
- Second screen capture at `OMOBA_QA_WIDTH=1024 OMOBA_QA_HEIGHT=640`: seven screens,
  `"status": "passed"`.
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
- A client built from this branch against an older server: `Leave` is ignored there, so
  leaving falls back to the five-second timeout and the old reclaim behaviour. Ship
  client and server together.
- The server still has no draft phase: hero select runs *before* the queue entry because
  `ClientPacket::Join` carries the loadout. A Wild-Rift-style pick after "match found"
  needs a protocol and server change (`Forming -> Drafting -> Starting`).
- The shell reads the career view from the same UDP session, so against a dead server the
  home screen shows an empty card with "Career profile not loaded yet".

## Phone pass for the iOS alpha build (2026-09-21)

The shell had only been checked at 1280x720 and 1024x640. An iPhone 16 Pro in landscape
is 874x402 logical pixels, and the client applies no UI scale on mobile, so every menu
was taller than the screen. The phone bar (?, MENU, SERVER) sat at z 30 under the shell,
which left a phone with no way to settings or to the server address from the menus.

- `frontend::scale_menus_to_the_window` sets `UiScale` to `height / 640` (clamped to
  0.55..1) while a menu screen is up, and back to 1 for the match.
- The phone picker keeps its absolute layout from `mobile_ui::adapt_phone_layout` at
  scale 1. Back moves under the class column, the hint starts beside it, and the
  desktop-only 3D side panel and wallet strip are hidden.
- The phone bar sits above the shell on the home screen and the picker; the other menus
  have their own Back in that corner.
- Evidence: `OMOBA_TOUCH_CONTROLS=1 OMOBA_QA_WIDTH=874 OMOBA_QA_HEIGHT=402` capture,
  seven screens, `passed` (`.agent/tasks/FRONTEND-SHELL-2026-09-20/artifacts-iphone/`).
  This is the touch UI on a desktop window, not a phone.
