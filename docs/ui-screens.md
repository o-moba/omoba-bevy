# UI screens

Generated from `scripts/ui_screen_map.json` by `python3 scripts/capture_ui_audit.py --write-doc`;
edit the JSON, not this file. Capture them all with `make ui-audit`.

## 01-menus · `shell`

Front-end shell screens driven directly; searching/loading/post-match have no real match behind them.

Harness: `client/src/qa/frontend_qa.rs`; server: dev; env: `OMOBA_FRONTEND_QA_OUTPUT={out} OMOBA_QA_CLEAN_FRAME=1`

| Screen | Frame | Profiles | How a player gets there | Code | Note |
| --- | --- | --- | --- | --- | --- |
| Home | `01-home.png` | desktop, phone | Launch the game | `client/src/frontend/home.rs` |  |
| Profile card | `02-profile-card.png` | desktop, phone | Home → Customize card | `client/src/frontend/card.rs` |  |
| Avatar collection | `03-collection.png` | desktop, phone | Home → Avatars | `client/src/frontend/collection.rs` |  |
| Hero select | `04-hero-select.png` | desktop, phone | Home → PLAY | `client/src/team.rs` |  |
| Searching | `05-searching.png` | desktop, phone | Hero select → Find match | `client/src/frontend/searching.rs` | layout only, no queue behind it |
| Loading (connecting) | `06-loading.png` | desktop, phone | Match found | `client/src/frontend/loading.rs` | layout only |
| Post-match | `07-post-match.png` | desktop, phone | End of a match | `client/src/frontend/postmatch.rs` | layout only, no result data |
| Game menu | `08-menu.png` | desktop, phone | Escape / MENU | `client/src/pause_menu.rs` |  |
| Settings · sound | `09-settings-sound.png` | desktop, phone | Game menu → Settings | `client/src/pause_menu.rs` |  |
| Settings · graphics (bottom) | `10-settings-graphics.png` | desktop, phone | Game menu → Settings, scrolled down | `client/src/pause_menu.rs` |  |
| Server entry | `11-server.png` | phone | Home → SERVER (phone) | `client/src/frontend/server_field.rs` |  |
| Home help | `12-home-help.png` | phone | Home → ? (phone) | `client/src/help_overlay.rs` |  |
| Home after help | `13-home-help-closed.png` | phone | Home → ? → close (phone) | `client/src/frontend/home.rs` |  |
| Party lobby (solo) | `14-party-lobby.png` | desktop, phone | Home → Party & friends | `client/src/frontend/lobby.rs` |  |

## 02-matchmaking · `flow`

Real button flow into a practice match: draft, countdown, loading, first in-match frame.

Harness: `client/src/qa/frontend_flow_qa.rs`; server: practice; env: `OMOBA_FRONTEND_QA_OUTPUT={out} OMOBA_FRONTEND_QA_FLOW=1`

| Screen | Frame | Profiles | How a player gets there | Code | Note |
| --- | --- | --- | --- | --- | --- |
| Team draft | `04-team-draft.png` | desktop, phone | PLAY → Find match (practice) | `client/src/frontend/draft.rs` |  |
| Pre-match countdown | `05-team-countdown.png` | desktop, phone | Draft → all locked | `client/src/frontend/loading.rs` |  |
| Loading (5v5 roster) | `06-team-loading.png` | desktop, phone | Countdown → map load | `client/src/frontend/loading.rs` |  |
| First in-match frame | `07-in-match.png` | desktop, phone | Loading → match | `client/src/match_hud.rs` |  |

## 03-in-match-hud · `match`

HUD, help, shop and a real purchase; result, target frames and scoreboard use labelled fixtures. Phones run without the edge pass: in 0.25.0 it stops on its radial-geometry check.

Harness: `client/src/qa/beta_ui_qa.rs`; server: dev; env: `OMOBA_VISUAL_QA_DIR={out} OMOBA_VISUAL_QA_SCENARIO=beta-ui OMOBA_BETA_UI_EDGE=1 OMOBA_BETA_UI_CLASS=mage OMOBA_QA_TEAM=green OMOBA_VISUAL_QA_TIMEOUT=220`; phone adds `OMOBA_BETA_UI_EDGE=0`

| Screen | Frame | Profiles | How a player gets there | Code | Note |
| --- | --- | --- | --- | --- | --- |
| In-match hero select (legacy) | `01-entry-{h}p.png` | desktop, phone | Join a running match | `client/src/team.rs` |  |
| In-match help | `02-help-{h}p.png` | desktop, phone | First join / F1 | `client/src/help_overlay.rs` |  |
| HUD | `03-gameplay-{h}p.png` | desktop, phone | In match | `client/src/match_hud.rs` |  |
| Shop | `04-shop-{h}p.png` | desktop, phone | In match at base → gold / P | `client/src/shop.rs` |  |
| Shop after purchase | `05-purchase-{h}p.png` | desktop, phone | Shop → buy Ember Blade | `client/src/shop.rs` |  |
| HUD after shopping | `06-shop-closed-{h}p.png` | desktop, phone | Shop → close | `client/src/match_hud.rs` |  |
| In-match result overlay | `07-result-fixture-{h}p.png` | desktop, phone | Base destroyed | `client/src/game_state.rs` | synthetic Victory, labelled on frame |
| Target frame · hero | `08-target-hero-fixture-{h}p.png` | desktop | Select an enemy hero | `client/src/edge_hud.rs` | synthetic target, labelled |
| Target frame · minion | `09-target-minion-fixture-{h}p.png` | desktop | Select a minion | `client/src/edge_hud.rs` | synthetic target, labelled |
| Target frame · structure | `10-target-structure-fixture-{h}p.png` | desktop | Select a tower | `client/src/edge_hud.rs` | synthetic target, labelled |
| Target frame · neutral | `11-target-neutral-fixture-{h}p.png` | desktop | Select a jungle creature | `client/src/edge_hud.rs` | synthetic target, labelled |
| Scoreboard | `12-scoreboard-fixture-{h}p.png` | desktop | Tab / score strip | `client/src/edge_hud.rs` | synthetic roster, labelled |
| HUD after scoreboard | `13-scoreboard-closed-{h}p.png` | desktop | Scoreboard → close | `client/src/edge_hud.rs` |  |

## 04-social-in-match · `social`

Chat, reaction wheel, confirmed reaction; phone adds the skill description on hold.

Harness: `client/src/qa/social_qa.rs`; server: practice; env: `OMOBA_SOCIAL_QA_OUTPUT={out}`; phone adds `OMOBA_SOCIAL_QA_SKILL_HELP=1`

| Screen | Frame | Profiles | How a player gets there | Code | Note |
| --- | --- | --- | --- | --- | --- |
| Match chat | `01-chat.png` | desktop, phone | In match → chat button | `client/src/social.rs` |  |
| Reaction wheel | `02-reaction-wheel.png` | desktop, phone | In match → reaction button / T | `client/src/social.rs` |  |
| Reaction over hero | `03-confirmed-reaction.png` | desktop, phone | Reaction wheel → pick | `client/src/social.rs` |  |
| Skill description (hold) | `04-skill-description.png` | phone | Hold an ability button (phone) | `client/src/mobile_controls.rs` |  |

## 05-career · `career`

Profile, friends, history, match detail, account and devices with labelled fixture data.

Harness: `client/src/qa/career_visual_qa.rs`; server: none; env: `OMOBA_CAREER_QA_OUTPUT={out}`

| Screen | Frame | Profiles | How a player gets there | Code | Note |
| --- | --- | --- | --- | --- | --- |
| Career · profile | `01-profile.png` | desktop, phone | Profile button | `client/src/career.rs` | fixture data, labelled |
| Career · friend code | `02-friends-code.png` | desktop, phone | Friends tab | `client/src/career.rs` | fixture data, labelled |
| Career · friends list | `03-friends-list.png` | desktop, phone | Friends tab, scrolled | `client/src/career.rs` | fixture data, labelled |
| Career · match history | `04-history.png` | desktop, phone | History tab | `client/src/career.rs` | fixture data, labelled |
| Career · match detail | `05-result.png` | desktop, phone | History → Details / Last match | `client/src/career.rs` | fixture data, labelled |
| Career · friend profile | `06-friend-profile.png` | desktop, phone | Friends → a friend | `client/src/career.rs` | fixture data, labelled |
| Career · website account | `07-website-account.png` | desktop, phone | Profile → Player website | `client/src/career_web.rs` | fixture data, labelled |
| Career · devices | `08-devices.png` | desktop, phone | Profile → Link or recover account | `client/src/career_devices.rs` | fixture data, labelled |
| Career · match detail (bottom) | `09-result-bottom.png` | desktop, phone | Match detail, scrolled | `client/src/career.rs` | fixture data, labelled |

## 06-supporter · `supporter`

Supporter aura panel; the harness uses its own fixed viewport sizes.

Harness: `client/src/qa/supporter.rs`; server: none; env: `OMOBA_SUPPORTER_QA_DIR={out}`

| Screen | Frame | Profiles | How a player gets there | Code | Note |
| --- | --- | --- | --- | --- | --- |
| Supporter · Solar aura | `solar-desktop.png` | desktop, phone | Profile → Support OMOBA | `client/src/supporter.rs` |  |
| Supporter · Lunar aura | `lunar-phone.png` | desktop, phone | Supporter → Lunar | `client/src/supporter.rs` |  |
| Supporter · Verdant aura | `verdant-phone.png` | desktop, phone | Supporter → Verdant | `client/src/supporter.rs` |  |

## 07-offline-practice · `offline`

Offline practice from Home: hero picker, match, target, game menu, settings. Logical size without a scale override, so HiDPI displays give 2x frames; its pass flag also checks the saved server endpoint.

Harness: `client/src/qa/offline_qa.rs`; server: none; env: `OMOBA_OFFLINE_SMOKE_DIR={out}`

| Screen | Frame | Profiles | How a player gets there | Code | Note |
| --- | --- | --- | --- | --- | --- |
| Offline · home | `01-home.png` | desktop, phone | Launch without a server | `client/src/frontend/home.rs` |  |
| Offline · hero picker | `02-hero-picker.png` | desktop | Home → Offline practice | `client/src/team.rs` |  |
| Offline · practice match | `03-practice.png` | desktop | Offline practice → start | `client/src/match_hud.rs` |  |
| Offline · attacked target | `04-attacked-target.png` | desktop | Attack the practice dummy | `client/src/edge_hud.rs` |  |
| Offline · game menu | `05-game-menu.png` | desktop | Escape in practice | `client/src/pause_menu.rs` |  |
| Offline · settings (top) | `06-settings-top.png` | desktop | Game menu → Settings | `client/src/pause_menu.rs` |  |
| Offline · settings (bottom) | `07-settings-bottom.png` | desktop | Settings, scrolled | `client/src/pause_menu.rs` |  |
| Offline · back home | `08-return-home.png` | desktop | Game menu → Exit to home | `client/src/frontend/home.rs` |  |

## Not captured yet

| Screen | How a player gets there | Code | What a capture needs |
| --- | --- | --- | --- |
| Settings · Language (English / 简体中文) | Game menu → Settings → Language | `client/src/pause_menu.rs` | PR #62 (i18n, 0.26.0) merged; then map it under the shell run and capture with OMOBA_LANGUAGE for each locale |
| Party lobby with members and invites | Party → invite a friend | `client/src/frontend/lobby.rs` | a scripted friend peer for OMOBA_FRONTEND_QA_ACCEPT_PARTY |
| Searching with a real queue | PLAY → Find match (release mode) | `client/src/frontend/searching.rs` | a release-mode server and queue peers |
| Post-match with a real result | Finish a match | `client/src/frontend/postmatch.rs` | a harness that plays a short match to the end |
| Game menu debug tools page, debug console, debug HUD | Game menu → Practice tools; OMOBA_DEBUG_UI=1 | `client/src/debug/tools_page.rs` | a harness run with OMOBA_DEBUG_UI=1 that opens the page |
| Gamepad focus ring and controller legend | Connect a controller | `client/src/gamepad/legend.rs` | synthetic gamepad input in a harness |
| Respawn countdown, defeat and rematch | Die / lose a match | `client/src/player/respawn_ui.rs` | a scripted death and a real defeat |
| Reconnecting and offline connection status | Lose the server | `client/src/frontend/mod.rs` | a harness that stops the server mid-session |
| Hero select join rejection and wallet/account flows | Join a full server; Connect wallet/account | `client/src/team.rs` | a rejecting server and wallet/account fixtures |
| Avatar collection with a live Studio registry | Avatars with Studio avatars | `client/src/qa/frontend_qa/avatar.rs` | OMOBA_AVATAR_QA with an SDK fixture service |
| Combat Test sandbox panel | Developer: OMOBA_COMBAT_SANDBOX=1 | `client/src/sandbox/ui.rs` | sandbox launch arguments; harness OMOBA_SANDBOX_QA_OUTPUT exists |
| HUD during a lane fight | Walk to mid in a practice match | `client/src/match_hud.rs` | scripts/capture_showcase.py lane scenes (flaky when the hero is blocked) |
| Phone target frames, scoreboard and utilities | Phone: select a target, open the score strip | `client/src/edge_hud.rs` | the phone edge pass (stops on its radial-geometry check in 0.25.0) |

## Other QA harnesses (not UI screens)

- `client/src/qa/frontend_qa/avatar.rs`: avatar collection with a Studio registry (gap avatar-studio)
- `client/src/sandbox/ui/qa.rs`: Combat Test panel (gap combat-test)
- `client/src/qa/audio_qa.rs`: audio settings duplicate of settings-sound; needs OMOBA_AUTOJOIN
- `client/src/qa/visual_qa.rs`: world render checks (Verdant, jungle, forest VFX), not UI
- `client/src/qa/map_qa.rs`: map prop swaps, not UI
- `client/src/qa/team_vision_qa.rs`: brush visibility, not UI
- `client/src/qa/targeting_qa.rs`: targeting proof (in-world markers), not UI screens
- `client/src/qa/combat_qa.rs`: projectile/impact proof, not UI
- `client/src/qa/forest_pickup_qa.rs`: forest pickups, not UI
- `client/src/qa/navigation_qa.rs`: minimap routing proof, not UI screens
- `client/src/qa/demo_qa.rs`: demo video director, not screenshots
- `client/src/qa/record_qa.rs`: demo video recorder, not screenshots
- `client/src/qa/animation_qa.rs`: animation helper, no frames
- `client/src/qa/ui_gallery.rs`: UI kit gallery (developer screen, OMOBA_UI_GALLERY=1 / OMOBA_UI_GALLERY_OUTPUT): every kit component and state, not a player screen
