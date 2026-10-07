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
| Result (Victory / Defeat) | `07-result-fixture-{h}p.png` | desktop, phone | Base destroyed | `client/src/frontend/postmatch.rs` | synthetic Victory over a live match (real live score), labelled on frame; the result screen of post-match (DECISIONS R2.2) |
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

Offline practice from Home: hero picker, match, target, game menu, practice controls, return home and re-entry as Wildspark. Logical size without a scale override, so HiDPI displays give 2x frames; its pass flag also checks the saved server endpoint.

Harness: `client/src/qa/offline_qa.rs`; server: none; env: `OMOBA_OFFLINE_SMOKE_DIR={out} GAME_SERVER_ADDR=127.0.0.1:49999`

| Screen | Frame | Profiles | How a player gets there | Code | Note |
| --- | --- | --- | --- | --- | --- |
| Offline · home | `01-home.png` | desktop, phone | Launch without a server | `client/src/frontend/home.rs` |  |
| Offline · hero picker | `02-hero-picker.png` | desktop, phone | Home → Offline practice | `client/src/team.rs` |  |
| Offline · practice match | `03-practice.png` | desktop, phone | Offline practice → start | `client/src/match_hud.rs` |  |
| Offline · target lock and recall | `04-target-recall.png` | desktop, phone | Attack the practice dummy | `client/src/edge_hud.rs` |  |
| Offline · game menu | `05-game-menu.png` | desktop, phone | Escape in practice | `client/src/pause_menu.rs` |  |
| Offline · practice controls | `06-practice-controls.png` | desktop, phone | Game menu → Practice controls | `client/src/pause_menu.rs` |  |
| Offline · back home | `08-return-home.png` | desktop, phone | Game menu → Exit to home | `client/src/frontend/home.rs` |  |
| Offline · re-enter as Wildspark | `09-reentered-wildspark.png` | desktop, phone | Home → Offline practice → Wildspark → start | `client/src/match_hud.rs` |  |
| Offline · local chat and reaction | `03-local-chat.png` | phone | Send a local debug message and reaction | `client/src/social.rs` |  |

## 09-party-stage · `party-stage`

Production party and prematch rendering with clearly labelled synthetic PartyView and GameStateSnapshot data; no live party or draft progression is implied. Desktop 1280×720, phone 844×390 and tablet 1180×820 are desktop-build viewport simulations. Rotation and social scrolling use scripted raw touch input, not physical device input.

Harness: `client/src/qa/frontend_qa/party.rs`; server: none; env: `OMOBA_PARTY_QA=1 OMOBA_FRONTEND_QA_OUTPUT={out}`

| Screen | Frame | Profiles | How a player gets there | Code | Note |
| --- | --- | --- | --- | --- | --- |
| Party stage · solo | `01-party-solo.png` | desktop, phone, tablet | Home → Party & friends | `client/src/frontend/lobby.rs` | synthetic PartyView, labelled; production solo stage and empty seats |
| Party stage · three members | `02-party-three.png` | desktop, phone, tablet | Party → accept an invitation | `client/src/frontend/lobby.rs` | synthetic PartyView, labelled; three members with the local nonleader in the centre |
| Party stage · five members | `03-party-five.png` | desktop, phone, tablet | Party → invite friends until full | `client/src/frontend/lobby.rs` | synthetic PartyView, labelled; five members and long multilingual names |
| Party stage · heroes rotated | `04-party-rotated.png` | desktop, phone, tablet | Party → drag the heroes | `client/src/frontend/lobby.rs` | synthetic PartyView, labelled; scripted raw touch rotation, not physical device input |
| Party stage · social panel scrolled | `05-party-social-scroll.png` | desktop, phone, tablet | Party → scroll the social panel | `client/src/frontend/lobby.rs` | synthetic PartyView, labelled; scripted raw touch scrolling, not physical device input |
| Party stage · team draft | `06-party-draft.png` | desktop, phone, tablet | Party leader → play → team draft | `client/src/frontend/draft.rs` | synthetic GameStateSnapshot, labelled; production five-member draft rendering, no server timer exercised |
| Party stage · shared countdown | `07-party-countdown.png` | desktop, phone, tablet | Draft → all locked or selection deadline expires | `client/src/frontend/loading.rs` | synthetic GameStateSnapshot, labelled; production five-member countdown rendering, no live progression |
| Party stage · asset loading | `08-party-loading.png` | desktop, phone, tablet | Countdown → wait for asset readiness | `client/src/frontend/loading.rs` | synthetic GameStateSnapshot, labelled; production five-member loading rendering, no live asset-ready barrier exercised |

## Not captured yet

| Screen | How a player gets there | Code | What a capture needs |
| --- | --- | --- | --- |
| Settings · Language (English / 简体中文) | Game menu → Settings → Language | `client/src/pause_menu.rs` | PR #62 (i18n, 0.26.0) merged; then map it under the shell run and capture with OMOBA_LANGUAGE for each locale |
| Party lobby with live peers and invitations | Party → invite a friend | `client/src/frontend/lobby.rs` | a scripted friend peer for OMOBA_FRONTEND_QA_ACCEPT_PARTY; party-stage captures synthetic party layouts, not live invitation delivery |
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
- `client/src/qa/result_qa.rs`: result screen states (finalizing, saved, defeat on a rematch server, abandoned) as labelled fixtures and the loading shell failure, OMOBA_RESULT_QA_OUTPUT; no server (gap post-match-real stays open)
- `client/src/qa/help_qa.rs`: controls guide controller focus and Settings → Controls return (OMOBA_HELP_QA_SHOTS); frames duplicate hud-help/home-help/settings, not new screens
- `client/src/qa/standard_kits_qa.rs`: Focused standard-kit selection, effects and held-aim proof from a live local sandbox (OMOBA_STANDARD_QA_DIR, OMOBA_STANDARD_QA_CLASS); OMOBA_STANDARD_QA_PHASES=1 takes an idle baseline and three state-gated stills per skill (windup, release, impact or settled) for any of the 17 classes, OMOBA_STANDARD_QA_OFFSCREEN=1 hides the window and renders the main camera to an image; for a look at motion clips a phase run takes OMOBA_STANDARD_QA_AVATAR (the hero's rig) and OMOBA_STANDARD_QA_RELEASE_AT (release still at the clip's contact time or a fixed time); for a look at projectile bodies OMOBA_STANDARD_QA_FLIGHT=1 casts unit-target abilities from lane distance and adds one still of the basic attack's projectile in flight. OMOBA_STANDARD_QA_AIM=1 adds a still of the aim preview of every modular skill with its key held before the cast, and one more while the slot offers a recast. One English desktop viewport, scripted input, no device matrix.
- `client/src/qa/roster_qa.rs`: Focused remaining roster selection and live skill states (OMOBA_ROSTER_QA_DIR); OMOBA_ROSTER_QA_INSPECTION adds short/long/released skill holds. English 1280x720, scripted input; no device matrix.
- `client/src/qa/sdk_pack_qa.rs`: Focused local SDK collection proof (OMOBA_SDK_PACK_QA_OUTPUT): 20 live avatar previews plus representative server-admitted running avatars and hand mounts. English 1280x720, scripted selection; not public Studio publication or a device matrix.
- `client/src/qa/combat_polish_qa.rs`: Focused synthetic iPad combat presentation fixtures (OMOBA_COMBAT_POLISH_QA_DIR): vitals, upgrades, held attack, kill feed, concealment and rocket. English 1180x820 by default; OMOBA_IPHONE_UX_QA=1 selects focused 852x393 iPhone gameplay, empty attack, concealment, dash aim, recall and FPS/HUD settings with checked UI readback. Not physical-device or multiplayer proof.
- `client/src/qa/dagger_qa.rs`: Focused Adventurer class preview and normal offline Q/W/E/R casts, plus clearly labelled synthetic rare-hit receipt presentation (OMOBA_DAGGER_QA_DIR). English852x393 only; optional synthetic focus, no raw-touch/device/RNG claims.
- `client/src/qa/equipped_skills_qa.rs`: Focused synthetic accepted hybrid HUD and held-skill card (OMOBA_EQUIPPED_SKILLS_QA_DIR). English 852x393 only; actual input and UI readback, separate real-server harness proof. Not physical-device or multiplayer proof.
