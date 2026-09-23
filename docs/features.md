# Feature Inventory

## Offline character practice (0.23.0-rc.5)

Home → Offline practice → choose a bundled avatar/class → Start practice. The 3D client runs a small in-process practice simulation through its normal snapshot/render/input pipeline, with no socket listener or external server. Level six unlocks every skill; four enemy practice heroes include a stationary melee target and moving animation examples, and defeated targets recover after three seconds. Mana regenerates for repeated tests. Basic attacks, class skills and dash/haste use shared definitions. Practice is not full bot matchmaking: there are no lane waves, ranked results, inventory purchases or progression rewards. Leave practice in Game menu restores the saved online server; bundled characters work without Ekza login. Game menu has a fixed × header and fixed navigation footer, with independently scrollable content.

## Combat particles and healing butterflies (0.23.0-rc.3)

Magic orbs have orbiting sparks and fading tails; confirmed hits produce short radial bursts. Six symmetric forest flocks glow and flap, grant one injured living collector up to 5% maximum HP, then respawn after 30 seconds. Availability and HP belong to the server. A soft oval vignette frames the battlefield below the HUD in 3D and Sprite2d. See [rules, rendering limits and native capture instructions](forest-combat-vfx.md).

## Measured combat pacing and growth (0.23.0-rc.2)

Starting durability and Q intervals now leave an early response window; levels increase movement, basic attack speed/damage and skill power. All four skills share a short recovery interval, while basic attacks keep an independent clock. The client buffers the next skill and restores server cooldowns across reconnect. Default towers scale damage against heroes without shortening minion siege windows. The 48-encounter production-path benchmark measures a level-one median of 10.53s and level-ten mean of 4.24s; these are controlled stationary targets, not competitive win rates. See [formulas, research, limitations and reproduction](balance-tuning.md).

## Public multiplayer MVP (0.22.0-rc.2)

The public lobby offers Quick match (30-second bot fallback), Wait for players (ten humans only), and Play with bots. Independent worker processes own immutable ten-player rosters, automatic teams, shared draft/countdown/loading and a durable start barrier. New humans cannot replace bots after a match starts; existing participants can reconnect. PostgreSQL saves history and progression for approved allocated bot games, with competitive rating reserved for eligible bot-free PvP. Signed gameplay packets, bounded admission and isolated durable outboxes protect the public match boundary. See [launch, recovery and capacity instructions](public-mvp.md).

## Shared running and VRM skeletal motion (0.21.0-rc.5)

All 15 shipped playable 3D avatars use the engine's shared Run motion during normal movement. Local intent and remote movement drive the same state machine; Walk stays reserved for future debuffs. Idle and combat actions remain separate. Existing 2D running sprites are unchanged.

Validated VRM0/VRM1 skinned humanoids receive runtime clips adapted to their bone map and rest pose, including models with no embedded clips. Original skin bytes remain unchanged. This is skeletal compatibility for a documented subset, not complete VRM materials/face/hair support. Approved Studio models gain runtime Run through normal verified loading; accepting externally published clipless models still requires a versioned profile rollout. See [architecture, import and limits](humanoid-motion.md).

Canonical version: `0.23.0-rc.3`

## Team draft and shared loading

Normal Find match assigns a team and map side automatically. Each assigned team
can inspect accepted avatar/class choices, choose an intended Solo, Jungle, Mid,
Carry or Support role, and lock or unlock its choice. Duplicate and composition
warnings inform the team without banning the overlap needed with four classes
and five-player teams. The roster scrolls for larger configured teams.

All required human players lock before the shared three-second countdown. The
loading screen retains the frozen roster and shows actual readiness. A client
acknowledges only after map and final avatar dependencies load; the server waits
for every required participant and the durable career-start acknowledgment.
Thirty-second loading timeout returns the team to draft with retry/leave controls.
Requests bind epoch, match, generation and sequence; reconnect retains accepted
choices and a running match bypasses the draft. Legacy clients remain opt-out.

Collection opens facing forward without automatic rotation. Pointer and touch
drags own their gesture and rotate naturally. The avatar catalogue lists defaults
first, then approved Studio, saved and purchased entries with loading, empty,
cached and unavailable states. Refresh preserves selection and scroll. SDK
hash/size/humanoid and paid-ownership checks remain mandatory. A temporary v2
catalogue failure retains cache; only404 enables legacy feed fallback.

## Compact match HUD and utility actions

The minimap occupies the upper-left corner on both profiles, with current gold and
two unowned recommended quick purchases below it. Tapping gold opens the full shop,
which also shows equipment. Quick purchases use the existing base, price, life,
inventory, request/retry and server-receipt rules.

Phone movement stays at lower left; the lower-right circle places main attack at its center, surrounded by explicit
minion/structure targeting, four skill circles with segmented rank indicators, and
dash/haste utilities. One upgrade-mode control turns eligible skill circles into
full-size upgrade targets. Desktop retains the four centered bottom skill cards.
Compact vitals, score, K/D/A and icon controls leave the middle and lower-middle
battlefield visible. A selected target gets exact current/max HP at upper center.

The score opens a two-team table backed by the authoritative round ledger, with
nickname, K/D/A, level and earned gold. Earned gold excludes the initial wallet
and remains independent of spending; life/reconnect retain it and a new round
resets it. A missing legacy score field is shown as unavailable, never fabricated.

Dash travels at most5 units, respects static/live obstacles and map bounds, and
has a20-second cooldown. Haste gives1.4× speed for3 seconds with a25-second cooldown.
Requests have current-match identity and replay protection; server snapshots drive
cooldown/active state. Explicit dash acknowledgments handle movements smaller than
the normal network correction threshold and prevent delayed old movement packets
from undoing them. The development speed toggle remains separate.

Actual-client render coverage and focused behavior checks are retained in the
MOBA edge-HUD task evidence. Phone runs are desktop development previews and do
not certify physical-device input, performance or platform release readiness.

## Forest ambience and combat particles

The game renders twenty animated butterflies around forest anchors. Their wings
share geometry and materials; offscreen butterflies are hidden. Combat uses a
fixed pool of 128 reusable particle slots: melee arcs and sparks, magic rings and
motes, and ranged hit flashes. Saturation drops excess cosmetics without changing
damage. Match changes clear active effects; both 3D and sprite modes are supported.

Only accepted, deduplicated server hit events trigger these bursts. The existing
`combat_visuals.json` class/avatar/sprite overrides control impact color, scale
and lifetime. `client/src/game_vfx.rs` maps projectile styles to burst geometry;
new geometry or trajectories can be added there without modifying combat rules.
This is a lightweight Bevy mesh/material particle foundation, without an extra
GPU particle plugin or dynamic lights. Phone performance still needs device QA.
These source changes are newer than TestFlight 0.20.0 (4).

## Shared accounts and Supporter cosmetics

A native installation can enroll a separate device key into an existing account
through explicit portal approval, without exporting its private seed or merging
progress. Activation preserves the prior key and takes effect after restart.
The portal lists/revokes devices and issues eight one-time recovery codes. Codes
are stored only as keyed hashes; old codes are replaced when a new set is issued.
Revoked keys remain tombstoned and cannot silently become a different account.

Supporter provides three cosmetic aura styles (Solar, Lunar, Verdant) and a free
isolated preview. The game server reads account grants from PostgreSQL, enforces
expiry and revocation, and replicates the permitted style. Auras follow actor
visibility/death and never change combat, XP, rating, or matchmaking. The portal
shows billing source, paid period, renewal state and aura preferences.

Live aura VFX use three intersecting inclined orbits with bright cores, camera-facing glow sprites,
eight tapered trail segments per orbit, four rising glints and a subtle ground ring.
Each hero has a fixed 35 rendered elements; meshes/materials are shared and no dynamic
lights or per-frame particle spawning are needed. Sprite mode projects the same paths
with front/back layering. Native preview uses the same effect implementation.
This source update is newer than TestFlight 0.20.0 (4).

Apple and Solana are separate provider adapters granting the same account right.
Payment functionality is unavailable until the operator explicitly configures a
verified provider. Solana is a quoted 30-day USDC prepayment, with finalized-chain
verification and a recoverable account order list. StoreKit supports native
purchase/restore and retains unfinished transactions until server confirmation.
The Apple adapter requires an authenticated verifier and App Store setup; local
fixtures are not proof of a real sandbox purchase or App Review approval.

Core career and portal schemas are both version 3. Run owner migrations and the
updated runtime grants before deploying matching game/API releases. See
[Supporter operations](supporter.md) for rollout boundaries and test procedures.

## Project and developer entry points

The README introduces the open-avatar mission, verified Ekza Space/SDK projects,
creator contribution paths and this game's pinned Passport integration. It
separates the playable source beta from distribution, mobile and provider gates.
`make` / `make help` list commands and overrides without starting processes.

## Builds from the primary checkout

All native launch and mobile packaging scripts are in the main repository.
`make practice` / `make play` use current desktop sources; `make iphone-check`
and `make iphone` prepare a physical arm64 iPhone app. Existing development
credentials can sign it; installation and real-device gameplay require separate
verification. Retain `builds/` when clearing `target/` caches. See
[iPhone instructions](../mobile/ios/README.md).

The iPhone installer checks profile expiration, physical-device enrollment and
Developer Mode before updating and launching. Home playtest kits bundle the Mac
practice server, display its current LAN addresses and require no compilation or
database to run. Actual device gameplay remains a separate acceptance step.

`mobile/ios/prepare_testflight.py` prepares a development-signed Xcode archive from
an existing device build without changing the original. It adds the app icon,
API-reason manifest and build metadata. App Store distribution signing, Apple
validation/upload and TestFlight availability are separate, unverified account
steps; see the [TestFlight guide](../mobile/ios/TESTFLIGHT.md).

## Phone playtest polish

The phone attack gesture selects along a forward ray with a bounded visual handle;
it can lock distant visible enemies without increasing damage range. Release
revalidates the exact preview rather than substituting another enemy. Desktop
mouse targeting remains separate.

All four classes have four illustrated skills. A stationary phone hold opens
ability name, description, rank/unlock level, mana cost and cooldown; releasing
inspection never fires a skill. A quick tap casts, and a deliberate drag aims.
Desktop slots reuse the same presentation-only art while retaining shortcuts and
status labels. See `client/assets/ui/skills/PROVENANCE.md` for replaceable art.

Phone chat keeps input, Send and Close above the keyboard and supports iOS Return.
Bots retain combat targets, continue routes without waiting for their next planning
tick and avoid other living heroes; buffered remote poses and stable head anchors
reduce motion/nameplate jitter. Verdant river bands join without stacked coplanar
surfaces at the lane crossings. Physical-device acceptance remains a playtest step.

## Player portal integration

A separate Next.js player portal uses the Rust Account API and the same PostgreSQL
career data. The native Profile screen confirms a browser pairing with the existing
device-held key; the browser never receives that key. The API supplies private
history, participant-only match reports, derived class/rating statistics, friends,
nickname/privacy/language/timezone preferences and revocable browser sessions.
Game runtime and portal have distinct database privileges. Owner-run migrations are separate from runtime startup; current career and
portal schema versions are recorded in the Supporter section above. Public
downloads are a curated optional catalog. Passkeys, public installation packages
and arbitrary avatar loadout editing remain outside this portal implementation;
Supporter aura selection is described separately above. See [operations](../account-api/README.md).

## Music and sound

A bundled CC0 soundtrack and sixteen effect cues cover combat styles, local
player/match events and interface actions. The client deduplicates accepted combat
receipts, attenuates nearby effects and limits simultaneous voices. Music follows
match/menu state and mute/focus changes; missing audio does not block gameplay.
Master, music, effects and UI volume are separately saved in client preferences.
The scrolling settings menu supports mouse and touch. Stable cue IDs map to local
Ogg assets in a versioned manifest. See [audio controls and authoring](game-audio.md).

## Native bot practice and match communication

A standalone practice server admits a solo player immediately and fills remaining
seats with labelled server-controlled heroes. In that local mode, late humans replace bots safely;
bots use the actual navigation/combat rules. Practice retains a local scoreboard
and gives no permanent career credit. `make practice` launches client and server;
`make practice-server` hosts this mode without a local client. Publicly allocated
bot matches instead freeze the human roster and save approved human history and
50/25 win/loss XP, with no competitive rating change.

Joined participants can use team/match chat and four picture reactions. The
server validates sender identity, scope, audience and rate limits. PC/mobile UI
provides text entry, mute controls and a reaction wheel via long press on the
local hero, T or a visible button. A separate opt-in social packet stream preserves
world snapshot capacity and compatibility with older clients.

Reaction IDs/access policy and image presentation are separate versioned catalogs.
The free starter pack works now; generic NFT packs remain locked until a trusted
ownership provider is integrated. See [setup, authoring and scope](bot-practice-and-social.md).

## Front-end shell and pre-match hero select

The client opens on a front end instead of the live map. Screens are a Bevy state
machine in `client/src/frontend/`
(`Home -> Card -> Collection`, `Home -> HeroSelect -> Searching -> Loading -> InMatch
-> PostMatch`); every menu screen is modal for `client/src/input_context.rs`, so the
world cannot receive input behind them.

- **Home:** the player's card (nickname, level, rating, W/L from `ProfileSummary`), the
  card's hero rendered live in 3D, the last match result, live connection status, PLAY,
  and entry points to the collection, match history, friends and the account modal.
- **Hero select:** the class/avatar/side picker, reached from PLAY, with a live panel
  showing the chosen avatar in 3D, the class line and that class's Q/W/E/R kit. Locking
  in is what sends `ClientPacket::Join`, which is also the matchmaking queue entry, so
  the map is only built after the hero is chosen.
- **Searching:** the server's `QueueView`, or the match formation counters when the
  ranked queue is off, with a cancel that returns home and clears the queue entry.
- **Loading:** shown from "match found" until the local hero exists in a running match.
- **Collection:** every shipped, owned and community avatar with a live 3D preview on
  its own render layer (`client/src/frontend/preview.rs`). The model can be turned by
  dragging and played through any animation clip its glTF declares; avatars that are
  not unlocked for matches are marked view-only. An avatar can be put on the profile
  card or selected for the next match.
- **Profile card:** locally stored choice of main class, showcase avatar, accent colour
  and a win-gated title (`profile_card.json` beside the client preferences).
- **Result screen:** outcome, personal K/D/A and rating delta, "Play again" and "Back to
  menu". Leaving sends the `Leave` packet: the server releases the seat at once and the
  same client can lock in another hero straight away. The search screen's Cancel uses
  the same path on every server mode.
- **Failure paths:** a rejected join returns to a working picker that shows the reason; a
  reconnect in the middle of a match keeps the match on screen; with nothing committed
  the menus retry the connection on their own and never change screen under the player.

Automation is unaffected: `OMOBA_AUTOJOIN` and the `*_QA_DIR` screenshot harnesses
bypass the shell and boot straight into the world. `OMOBA_FRONTEND_QA_OUTPUT` captures
the shell screen by screen (at `OMOBA_QA_WIDTH` x `OMOBA_QA_HEIGHT`, default 1280x720)
and fails if a screen leaves the viewport; adding `OMOBA_FRONTEND_QA_FLOW=1` presses the
real buttons against a live server instead and records the screen sequence that
follows.

## Current Playable Surface

- **Persistent career and friends:** PostgreSQL stores immutable match results,
  historical nicknames/loadouts, K/D/A and accepted damage totals. Device-signed
  profiles expose history, progress, outcome-based rating and durable friendship
  requests/actions with online/in-game presence. Desktop and phone have separate
  dashboard layouts; the result remains available after the live round ends.
  Authenticated release queues use saved MMR and newcomer cohorts and wait for a
  durable allocation before starting. Guest practice remains explicitly unranked.
  See [career setup and release limits](match-progression.md).


- **Configurable map objects:** a validated server map profile supplies stable
  tower identities, lane positions/counts and HP/range/damage/cooldown settings.
  Ordered lane tiers control siege protection and base access. The profile is
  pinned at startup and reused on rematch; live objects drive both client modes
  and the minimap. Existing Verdant terrain/collision stays versioned and fixed.
  See [map customization](map-customization.md).
- **Reusable map presentation:** authored prop archetypes and stable instances
  support packaged model/palette overrides; solid replacements preserve the
  collision footprint. Live tower/base models and 2D sprite choices use the
  same presentation registry. Cosmetic settings do not alter server rules.

- **Combat presentation and skins:** Ranger arrows, Mage arcane bolts, Cleric
  holy bolts and Warrior crescents, with confirmed damage numbers and impacts.
  Packaged profiles choose class/action defaults and avatar/sprite overrides,
  custom projectile GLBs or animated PNG atlases, trail/impact settings and exact
  avatar animation-clip aliases. See [the contributor guide](combat-cosmetics.md).
  Cosmetic configuration has no gameplay authority.
- **Draft sprite handling:** an unfinished roster entry keeps its stable identity
  and portrait, is marked art pending, and renders an explicit fallback for old
  saved/network selections. The nine active sprite pairs have complete files.
- **Mixed minion waves:** two melee fighters (65 HP, 8 damage, 2.4 reach,
  0.95-second cooldown) and one caster (45 HP, 7 damage, 8 reach,
  1.2-second cooldown). Caster damage occurs on projectile arrival.
  Wave size, routes, cadence and rewards are preserved. Distinct weapons and
  replicated release poses communicate roles; both models are 50% taller.
  This is initial beta tuning, not a claim of competitive balance.

- **Jungle farming:** six ordinary camps, with mirrored skirmisher, bruiser and
  spitter encounters. All heroes can earn authoritative last-hit XP and gold;
  killed monsters respawn after 40 seconds. Ordinary camp kills restore 20%
  maximum HP to the living last hitter. Distinct 3D creatures and persistent
  camp markers expose living/depleted locations in desktop and phone UI.
  See [the implementation and verification record](progress/2026-09-11-jungle-camps.md).

- **Local source launch:** `make play` builds locked sources and runs local 3D
  practice with nine bots; closing the client stops its server and bots.

- **Purchased avatar passport (2026-09-11):** opt-in terminal pairing connects
  Omoba to the buyer's browser wallet. The owned library filters paid avatar
  choices; a trusted HTTP service consumes a one-use ticket before the game
  server admits the exact approved cosmetic. Free avatars remain available.
  Curated imports validate SHA-256, size, GLB skinning and required animation
  clips, then produce a public manifest shared by both binaries after restart.
  The native renderer and snapshots use that same protected slug, while class
  and gameplay stats remain independent. See the [setup and evidence scope](
  progress/2026-09-11-avatar-passport-roundtrip.md). This first delivery requires
  staged distribution and explicit development-service configuration.

- **Minion lane entry (2026-09-11):** outer-lane waves skip the unused corner
  beyond the base entrance, eliminating a 34-unit out-and-back detour. Both
  teams enter their assigned lane directly; mid, spawn spacing/speed/cadence,
  map artwork and tower anchors are preserved. See the
  [route verification record](progress/2026-09-11-minion-lane-entry.md).

- **Mouse targeting and phone attack lock (2026-09-11):** desktop left click
  selects, right click on a hostile approaches and repeats basic attacks, and
  right click on ground/minimap cancels attacks and moves. S/Backspace stop
  movement and attacks and clear the lock. Phone has a separate large ATTACK button plus Q/W/E/R. Tap
  attacks once; stationary hold repeats; drag extends a reticle to preview and
  lock the exact foe. Drag to X to cancel. Phone basic attacks never chase.
  Basic attacks cost no mana and have class/equipment damage and independent
  server-enforced cooldowns. Skills retain their existing balance and use the
  selected target. Protocol 2 rejects incompatible old peers and binds strikes
  to server epoch, match and monotonic request IDs. Rebuild client and server
  together. See the [controls and verification record](progress/2026-09-11-targeting.md).

- **Hero and environment readability (2026-09-11):** the desktop minimap now
  sits at upper left with reserved objective space and inventory fit at 960px.
  The mobile HUD retains its own layout. Heroes normalize to 2.1 world units;
  the 3D follow camera is 15% closer at the existing angle. Decorative reeds
  are half-height and grass fans 35% shorter with ground anchors preserved.
  Trees, rocks, creature sizes and the 236 solid collision polygons are unchanged.
  Saved previous defaults migrate once; deliberate custom scales remain.
  See the [measurements and comparison captures](progress/2026-09-11-hero-readability.md).

- **Native phone implementation candidate (2026-09-11):** landscape two-thumb
  controls share the existing movement, collision and legal ability paths with
  desktop. The compiled target OS selects one stable interface family:
  Android/iOS use mobile UI, Windows/macOS/Linux use desktop UI. Resizing,
  rotation and DPI only affect layout; desktop mobile preview is development-only.
  Per-finger capture, dead zones, drag cancellation and interruption
  cleanup protect simultaneous movement/casting. The phone layout reflows the
  HUD, entry, shop, help and results. A compact right-thumb ability fan follows
  the supplied Wild Rift reference; HP/mana/progression sit at upper right,
  with the minimap and shop at upper left. A server form removes the need for shell
  configuration. Android NativeActivity build scripts, mobile assets/preferences
  and early iOS Simulator scaffolding are included. These are implementation
  features, not proof of installable packages or mobile beta readiness. See the
  [implementation and remaining gates](progress/2026-09-11-mobile-beta.md)
  and [platform build guide](../mobile/README.md).

- **Forest navigation and minimap routes (2026-09-08):** right-click ground or
  the minimap to walk around trunks, solid rocks/walls and live towers/bases.
  A mint line and destination ring show the remaining route on the minimap;
  world movement-path gizmos are removed. The same generated obstacle data,
  spatial index and bounded A* module govern client routes, authoritative
  swept movement checks and fill-bot traversal. The minimap shows the shared
  solid forest footprint under portraits and the yellow camera rectangle.
  Input replacement, pending-cast cancellation and modal/orbit behavior remain
  intact. Canopies/grass are decorative; projectiles, vision and dynamic crowd
  planning are outside this collision change. See the
  [architectural/session record](progress/2026-09-08-forest-navigation.md).

- **HUD, camera and equipment iteration (2026-09-07, iteration 05):** default
  heroes are 26% larger with saved-default migration and unchanged creature
  sizes. Both teams see a diagonal midlane matching the minimap. The compact
  resource/ability/inventory HUD exposes a six-item base shop: 80 starting gold,
  one gold per second during play, class recommendations and authoritative
  purchases with round-bound retry receipts. Damage, Q attack rate, W/E/R haste,
  movement, HP and mana bonuses affect play; death/reconnect preserve equipment
  and rematch resets it. The minimap uses team-colored hero portraits,
  a separate local gold halo and actual camera ground coverage. Enemy hero
  markers use shared radial team detection; this is **not world/network fog of
  war**. See the [dated analysis and evidence](progress/2026-09-07-hud-shop-iteration.md)
  and [tester guide](progress/2026-09-07-beta-test-guide.md).

- **Complete-match beta preparation (2026-09-07, iteration 04):** dead-player
  progression preserves the respawn timer; full-roster lane XP and ordinary
  fill-bot ability selection support useful complete sessions. A bounded
  two-round real-UDP acceptance command observes normal release matchmaking,
  objectives, victory and clean rematch. Help has a clickable dismissal,
  Escape has modal precedence, Pause resumes/exits safely and the result panel
  explains automatic rematch. Native 720p UI captures and packaged practice,
  host and remote-join commands support the first cohort. See the
  [dated readiness record](progress/2026-09-07-beta-readiness.md) and
  [beta tester guide](progress/2026-09-07-beta-test-guide.md) for the actual
  evidence and coverage limits; this entry does not certify human balance,
  untested platforms or public Internet operation.

- **Verdant Confluence native arena (2026-09-05, iteration 03):** the supplied
  Blender scene now supplies the 3D environment, foliage and live faction
  structures. Walk surfaces match actor grounding, structures follow death and
  rematch, and cached original creatures replace the disputed imported models.
  The 15-hero roster and King Mutatio remain. A content-based package gate
  checks model bytes and provenance; an opt-in native capture scenario records
  the actual renderer. See the [integration record](progress/2026-09-05-verdant-runtime-release.md)
  and [current test guide](progress/2026-09-05-verdant-test-guide.md). This
  supersedes earlier roster, primitive-map and four-asset-blocker descriptions.

- **Native 3D release candidate (2026-09-05, iteration 02):** idempotent admission,
  full-state reconnect, complete rematches, release roster/debug policy, bounded
  framed UDP and ordered snapshots address the lifecycle/network audit findings.
  Shared input/modal rules, intentional orbit, scrollable roster, visible hotbar
  feedback, existing 3D action/death clips and non-color allegiance cues address
  the input/presentation findings. Offline mode-scoped loading and the native
  package remove developer-local runtime requirements. Protected bases, scenario
  bot upgrades/sustain/tower support and match metrics support controlled sessions.
  See the [current ledger](progress/2026-09-05-release-preparation.md) and
  [package/test guide](progress/2026-09-05-release-test-guide.md). External release
  gates remain blocked until actual evidence exists; the entries below describe
  historical feature delivery and are superseded where the ledger says so.

- **3D delivery and readiness audit (2026-09-05):** the locked SDK dependency,
  two offline slime minion models and 16-avatar shipped roster now survive a
  clean checkout. The [current assessment](progress/2026-09-05-3d-readiness-audit.md)
  records remaining reconnect/rematch, release debug-command, input and combat
  readability blockers. These findings qualify earlier feature descriptions;
  existing features are not a claim of unattended playtest readiness.

- **Pointer-first desktop/mobile combat (TASK-POINTER-COMBAT-MOBILE-01):** a
  primary click or touch tap resolves living hostile actors by their projected
  screen position with 48–68 logical-pixel hit radii, selects the exact
  authoritative `TargetId`, shows the existing marker, and requests Q. Empty
  ground moves on both input types; target, minimap, and UI presses are
  consumed before movement. Keyboard and 64-pixel hotbar buttons share one
  pending-cast path. An out-of-range unit cast follows the moving target until
  it enters the shared scaled range, emits once, and only then starts the local
  cooldown. Manual movement or invalidation cancels the pending request.
- **Stable 2D controls and proportional nameplates
  (TASK-2D-CONTROLS-FOLLOW-01):** click-to-move remains active independently
  of camera follow, `Y` toggles follow and clears a minimap focus when returning
  to the hero, and 2D right-click/Alt no longer capture the cursor or silently
  disable movement input. Hero labels are bounded by hero height and estimated
  text width, including the long Orchard Comet Centaur display name. Legacy 3D
  camera controls remain available.
- **2D production-readiness pass (TASK-2D-PRODUCTION-READINESS-01):** the
  orthographic camera now starts twice as close, and actor sizes are validated
  from occupied alpha pixels rather than transparent atlas cells. The six lane
  towers are one-to-one with authoritative owners and carry Green-square or
  Blue-diamond badges plus TOP/MID/BOT labels. The initial 18-minion wave uses
  larger state-aware sprites and the same non-color team language; owner
  removal recursively cleans its bounded visual cues. A headless ECS fixture
  proves exactly 24 primary proxies for six towers plus 18 minions, then 23
  after one minion disappears, with no duplicates on replay.
- **Complete UDP datagrams above 8 KiB:** client/harness receive storage is
  65,536 bytes and the server validates the whole serialized snapshot against
  the 65,507-byte IPv4 UDP payload ceiling before sending. Boundary tests cover
  8,191/8,192/8,193 bytes, malformed traffic, near-limit decode, and whole
  over-limit rejection. A real release 5v5 server test receives a complete,
  runtime-dependent snapshot satisfying `8192 < bytes <= 65507`, with 10
  players, 8 structures, and 18 minions. The measured macOS kernel send ceiling
  remains 9,216 bytes; current protocol-1 clients now use ≤1200-byte framed datagrams; this
  legacy whole-JSON path remains for compatibility scripts.

- **Genuine full-2D world (TASK-FULL-2D-WORLD):** `sprite2d` is now a true
  orthographic XY renderer rather than a billboard layer over the 3D arena.
  A centralized tested projection maps authoritative simulation XZ to render
  XY (`x → x`, `z → y`) and back for cursor picking. One `Camera2d` supports
  hero follow, bounded arrow-key free pan, clamped wheel zoom, resize-safe map
  edges, and minimap focus/recenter. `models3d` remains selectable and starts
  independently with its existing perspective scene.
- The deterministic 55×55 tiled map reproduces the authoritative three lanes,
  two bases, six towers, two base objectives, diagonal traversable river,
  three camps, and two boss pits. Original CC0 Higgsfield/Recraft terrain and
  prop atlases are declared in `client/assets/world2d/manifest.json`; forest
  and water visuals do not invent client-only collision. Static tiles plus
  props remain below 4,096 entities, transient VFX are capped at 256 and
  normally expire within two seconds, and cached atlas handles avoid per-frame
  asset creation. Validate with
  `python3 scripts/validate_world2d_assets.py --self-test`.
- Heroes, structures, team minions, normal neutrals, Wendigo, King Mutatio,
  projectiles, selection markers, health/mana bars, names, and combat effects
  all use Bevy 2D render components in this mode with deterministic foot-Y
  sorting and explicit layer bands. Gameplay, combat, AI, collision,
  matchmaking, buffs, victory, and reconnect remain server-authoritative.

- **Release-like 2D combat presentation (TASK-2D-RELEASE-VERTICAL-SLICE):**
  the five sprite heroes now cover idle, run, attack, cast, hit, and death.
  Accepted Q/W/E/R casts carry an authoritative monotonic cosmetic action
  sequence through snapshots, so local and remote clients play each one-shot
  once; HP loss interrupts with hit, death holds its last frame, and respawn
  resumes locomotion. Sprite manifest schema v2 keeps separate 8×2 locomotion
  and 8×4 action sheets with sheet/playback metadata. The same client-local
  mode now supplies art-directed billboards for towers, bases, team minions,
  normal neutrals, Wendigo, King Mutatio, and projectiles, plus bounded
  cast/hit/heal/death effects and a painted arena treatment. All visuals stay
  attached to the existing authoritative roots and do not change combat,
  collision, interpolation, AI, or map topology. The default 3D path remains
  intact. Asset contracts live in `client/assets/sprites/manifest.json` and
  `client/assets/presentation2d/manifest.json`; validate both with
  `python3 scripts/validate_sprite_assets.py --self-test`.

- **Selectable 2D sprite player visuals (TASK-2D-SPRITE-PROTOTYPE):** the
  pre-join screen selects either **3D Models** (the default and safe fallback)
  or **2D Sprites**. The sprite roster contains Mossback Teapot, Neon Axolotl
  Courier, Origami Storm Heron, Clockwork Turnip Oracle, and Void Jelly
  Astronaut; its five 2048×512 RGBA sheets follow an 8-column × 2-row contract
  (eight 6 fps idle frames, eight 12 fps run frames) declared once in
  `client/assets/sprites/manifest.json`. Sprite mode renders transparent,
  unlit camera-facing quads as children of the unchanged gameplay roots and
  chooses idle/run from local or interpolated remote movement with a 0.25 s
  idle grace. Renderer mode is client-local, while the optional validated
  sprite character id is replicated and retained through reconnect. Set
  `OMOBA_PLAYER_VISUAL_MODE=models3d|sprite2d` to choose the initial mode;
  unset or invalid values use `models3d`. Validate the offline asset contract
  with `python3 scripts/validate_sprite_assets.py --self-test`.
  The manifest and selection portrait strip now contain ten named slots. Four
  added heroes have complete runtime sheets; Orchard Comet Centaur still lacks
  its six generated runtime animation clips/sheets, so that selection remains
  an explicit release blocker rather than silently substituting another hero.

- **Matchmaking and gated match start (TASK-22):** in release mode (server
  default) players who join land in a queue; the match forms to a full 5v5
  roster, teams are assigned and balanced server-side, a 3-second countdown
  runs, and only then the match starts — a solo player cannot start an
  under-filled match. The client overlay walks through "Searching for
  match..." → "Waiting for players — X/10" → "Match found! Starting in N...".
  `OMOBA_MATCH_MODE=dev` keeps the instant-start dev flow (`make start`,
  `make server-dev`); `OMOBA_TEAM_SIZE` scales the roster (1–16 per team)
  for playtests. `make play-bots` / `make bots` fill the queue with UDP
  bots so one developer can walk the whole flow (see RUNBOOK.md).
- **Slime lane minions + visible camps (TASK-24):** lane minions are
  team-colored CC0 "Mimic Slime" models (green Classic / blue Water,
  Halloween Rising) with walk/attack animations driven by the replicated
  AI state, normalized to 0.6× hero height through the shared model-scale
  pipeline (`client/assets/minions/`, overrides keys
  `slime-green`/`slime-blue`). Decorative jungle boxes no longer spawn on
  neutral-camp or boss-pit anchors, so camps and raid bosses stand in open
  clearings instead of being hidden inside geometry.
- **Bot lane-push AI (TASK-23):** fill bots play once the match runs — each
  takes a lane, pushes its waypoints toward the enemy base, fights enemy
  players/minions with its class Q (server-authoritative ranges/cooldowns),
  sieges towers in reach, and rejoins the lane after a respawn
  (`harness/src/bot_ai.rs`). Simple nearest-target logic, no retreat or
  skill combos — built for playtesting matchmaking and basic playability.
- **Character scale normalization (TASK-20/21):** every character and boss GLB
  (legacy SDK models, roster avatars, raid bosses — authored anywhere from
  0.64 m to 2.41 m tall) is measured once in bind pose directly from the
  loaded glTF data and rescaled to the shared world-relative target height
  (default 1.15 world units, range 0.3–3.0), so all characters render at the
  same size by default (bosses keep their 3× presence multiplier). Persisted
  target heights from the legacy 0.26 scale migrate to the new default on
  load. Per-model size tweaks live in
  `client/assets/config/model_scale_overrides.json` (slug → multiplier,
  hot-reloaded while the game runs). `OMOBA_MEASURE_MODELS=1 cargo run -p
  client` runs a headless analyzer that prints the measured height table
  (`client/src/model_scale.rs`).
- **Spawn platform traversal (TASK-21):** the 46×46×0.7 base pads are
  walkable League-style — a client-side `MapLayout::terrain_height(x, z)`
  function describes the pad top plus a 6-unit ramp band that exactly matches
  four visible team-colored ramp slabs per pad. Local player gravity/jumps,
  remote players, and minions ground onto that surface; models rest on their
  measured foot offset. The server stays flat-ground authoritative (pure
  visual fake, no protocol changes).
- **Environment decoration (TASK-18):** the arena is dressed with stylized
  low-poly vegetation and props assembled purely from Bevy mesh primitives —
  3 tree variants (oak/pine/birch), 2 bush variants, grass tufts, 4 flower
  variants (white/yellow/red/violet), and 2 rock variants — placed by a
  deterministic seeded scatter (`client/src/decor.rs`, inline splitmix64
  PRNG, no external assets or new dependencies). Forest belts hug the arena
  edges, trees and boulders ring the jungle blocks, and grass/flowers fill
  the open meadow, while exclusion zones derived from the real map constants
  keep lanes, base pads, towers, neutral camp clearings, the river, and the
  jungle blocks clear. Purely cosmetic and client-side: no collision, no
  server or networking changes. Fixed budget: 396 props = 970 entities
  (ceiling 1200) spawned once at `Startup` under a single `DecorRoot`,
  reusing 5 shared mesh and 12 shared material handles for batching. **F4**
  toggles decoration visibility (client-local debug toggle, logged).
- **Hero classes (TASK-17):** four playable classes — Warrior, Mage, Ranger,
  Cleric — each with a distinct Q/W/E/R kit (16 ability definitions in the
  `shared` crate; projectile damage, self-heal, and self-mana-restore
  primitives with per-class numbers). The server resolves the kit
  authoritatively per player: per-slot cooldowns, unlock gating by level
  (Q@1/W@2/E@4/R@6), rank scaling up to max rank 3, and skill upgrades capped
  at the shared max rank. Class selection happens on the pre-join screen and
  is carried in the join packet.
- **CC0 VRM avatar roster (TASK-17):** 16 CC0 avatars (Open Source Avatars
  collections) staged as GLB under `client/assets/avatars/` with embedded
  retargeted animation clips (`idle`/`walk`/`attack`/`cast`/`death` from the
  Quaternius Universal Animation Library, CC0) and a provenance manifest
  (slug, name, collection, license, source URL, author, thumbnail). Avatar
  selection is a thumbnail grid on the pre-join screen; the chosen slug
  replicates to all clients, models load lazily, and every roster avatar
  plays idle when stationary and walk while moving (idle-grace hysteresis
  smooths snapshot interpolation). Unknown avatar slugs and class ids fall
  back safely (default model / Warrior).
  `OMOBA_AUTOJOIN=<class>:<slug>:<team>[:<sprite-id>]` joins without UI for
  automation.
- **Raid bosses with team buffs (TASK-19):** two epic neutral objectives built
  on the jungle-neutral system — **Wendigo** (bottom river/jungle pit, spawns
  at 60 s match time, 900 HP) and **King Mutatio** (top jungle pit, spawns at
  180 s, 1500 HP), in 180°-symmetric pits derived from the map formula. Bosses
  aggro when attacked, leash back to their pit at full HP, and respawn 180 s
  after death (camps keep 40 s). Killing a boss grants the killer's whole team
  a replicated timed buff: Wendigo's Favor (+15% ability damage, 90 s) or
  Mutatio's Might (+25% ability damage +2 HP/s regen, 90 s); a re-kill
  refreshes, both buffs stack multiplicatively, and the server applies the
  damage multiplier and regen authoritatively. The client renders each boss
  with its staged CC0 model (`client/assets/bosses/`, own manifest — boss
  slugs are never player-selectable) scaled to raid presence, with HP bar,
  floating nameplate, idle/walk animation from the replicated AI state, and a
  match-HUD indicator showing the local team's active buffs with remaining
  seconds. All tuning lives in named constants in `server/src/balance.rs`.

- Headless Bevy-scheduled authoritative UDP loop with periodic player snapshots; player mana regeneration and `projectile -> minion` damage now run through ECS/message-driven systems bridged to the current authoritative state maps.
- Server-authoritative hardening for player movement and casts: client transforms are speed/map clamped, non-finite positions are ignored, and casts require the authoritative caster position to be in range of the live target.
- Local multiplayer flow with server startup plus multi-client local play via `make start`.
- Team join flow with character selection and player spawning.
- Ekza Bevy SDK extraction: shared sibling `ekza-bevy-sdk` repository owns stable character ids, built-in 3D model manifest metadata, GLB validation, and Bevy model catalog loading for future dependency publishing.
- Ekza avatar store at runtime: through `ekza-bevy-sdk`, the client reads the public registry catalogue for templates approved for `omoba` / `desktop` / `humanoid-glb-v1`, shows the ones the paired wallet owns in a separate "Your Ekza avatars" group below the shipped "Default avatars", and installs a model on first use (verified size, SHA-256, GLB envelope and embedded `idle/walk/attack/cast/death` clips) under the private settings directory, mounted as the `ekza://` asset source. Other players' store avatars are fetched the same way on demand with the legacy model as a stand-in. The server admits a store slug only from a ticket consumed at the configured passport origin whose grant hashes to that slug. Wallet pairing starts from the "Connect Ekza wallet" button in the picker (browser approval, non-blocking); purchase happens on the Ekza web storefront. `scripts/ekza_publish.py` publishes an on-chain template into a registry catalogue with a baked Omoba rendition, and `scripts/ekza_demo.py` runs the whole loop locally on devnet.
- VRM avatar support: VRM 0.x avatars (glTF 2.0 binary with extra `VRM`/spring-bone/blendshape extensions) load through the existing glTF model catalog by staging them as `.glb` (the VRM extensions are `extensionsUsed`-only, so Bevy's loader ignores them and keeps the mesh + skeleton). Ships one selectable CC0 humanoid, `Paco` (ToxSam 100Avatars R3); see `ATTRIBUTION.md` and `scripts/convert_vrm_to_glb.py`. The avatar carries no animation clips, so it renders as a static skinned mesh via `NormalizeModelScale` like other models.
- Core combat loop with projectiles, structures, minions, death, respawn, mana regeneration, and base-destruction win condition.
- Map layout with three lanes and simple jungle blocks.
- Player progression with level-based XP thresholds, HP/mana scaling on level-up, and tracked skill points.
- In-game local HUD display for level and XP progression.
- Persistent local client preferences (graphics, character, optional server address, stable client session id) with safe clamping on load; override directory with `OMOBA_CLIENT_CONFIG_DIR` for tests or portable installs.
- In-game match HUD (below minimap): level, XP, skill points, upgrade key label (`U`), local HP/mana, target hints, objective line, per-slot class ability lines (name, effect numbers, cooldown/lock state), and F1 help reminder; bottom-right skill bar shows `Q`–`R` keys with the selected class's ability names and ranks.
- F1 toggle help overlay with movement, camera, targeting, casting, objective, and pause guidance; does not reset simulation when toggled. The panel is shown only while the match is `Running` (toggle state is preserved when returning to a live match so lobby/victory screens are not covered).

## Multiplayer Session Reliability

- **TASK-14 (client)**: Explicit session states, non-blocking wait when the server is down, bounded `WaitingForServer` timeout, stale snapshot detection while connected, UDP transport error thresholds, snapshot-channel disconnect detection when the UDP thread ends, full teardown (replicated entities + team overlay) on disconnect, manual reconnect via **Retry** (no silent rejoin into a match), pause menu auto-closes on **Disconnected**, minimap hidden unless **Connected**.
- Named timing constants and failure-detection summary: `docs/network-client-session.md` and `client/src/session_config.rs`.
- Join is authoritative on the server: the client may optimistically pick a team and character, but the snapshot for `your_id` is the source of truth for spawn side, team, and character.
- Repeated `Join` packets from the same UDP endpoint are deterministic: the last processed `Join` wins for team, character, spawn position, HP, mana, gold, and XP reset.
- If a client stops sending packets, the server removes that player after `PLAYER_TIMEOUT = 5s`; remaining clients stop receiving that player in snapshots after the timeout expires.
- Reconnect policy for this version: clients that send a valid stable `client_session_id` in `Join` can reclaim a timed-out player slot/id for a short server-side window. Legacy clients without a session id still use endpoint identity and reconnect as a new player.
- If the server restarts while clients stay open, the next packet from an existing client creates a fresh default session on the restarted server. Team and character return to defaults until that client sends `Join` again.
- The client applies only the latest queued snapshot per frame. This "last snapshot wins for the current frame" behavior is intentional for now and validated by the session-flow checks.

## Release Gaps Tracked In Tasks

- Runtime and startup stability hardening.
- Account-backed identity, cryptographic session authentication, and long-lived reconnect across server restarts.
- Publishable SDK packaging: registry metadata, versioning policy, examples, entitlement/auth hooks, and non-blocking asset delivery are still future work.
- Full reconnect slot reclaim across disconnects and NAT changes.
- Directional sprite movement, richer tooltip UX, and balance passes over the class kits.

## Release gate and balance (TASK-12)

- Authoritative tuning constants: `server/src/balance.rs` (see `docs/balance-tuning.md`).
- Release checklist, manual QA matrix, and readiness report: `docs/release-gate-checklist.md`, `docs/manual-qa-matrix.md`, `docs/release-readiness-report.md`.
- Live UDP QA smoke (two clients + cast): `make verify-task-12` or `python3 scripts/verify_task_12_qa_matrix_live_udp.py` (after `cargo build -p server`).
- Headless gameplay rule harness (typed Rust, no GPU/human): `make verify-gameplay` boots the real server on a per-test port and drives it with bot clients to assert god mode, the movement-authority clamp, and skill-point gating (`harness/` crate; see `docs/progress/2026-06-28-headless-gameplay-harness.md`).
- Expanded skill roster, tooltip UX, balance passes, and release-scale QA beyond the current cast-and-HUD surface.

## Operations and playtest documentation

- [README.md](../README.md) — setup, controls summary, links to tester docs.
- [RUNBOOK.md](../RUNBOOK.md) — startup, env vars, troubleshooting with recovery steps.
- [docs/playtest-script.md](playtest-script.md) — timeboxed MVP session checklist.
- [docs/bug-report-template.md](bug-report-template.md) — internal report format.
- [docs/mvp-scope-and-limitations.md](mvp-scope-and-limitations.md) — explicit MVP scope and limitations.
- [tasks/MVP-CHECKLIST.md](../tasks/MVP-CHECKLIST.md) — MVP-blocking vs deferrable classification.

## Open-source mission and licensing

The server is AGPL-3.0-only; the client and shared/reusable code are MPL-2.0.
Original documentation and identified Verdant art use CC-BY-4.0. Existing
third-party/CC0/OFL notices and user-avatar rights remain separate. Commercial
forks are permitted. See [licensing](../LICENSING.md), [mission](../MISSION.md)
and [source delivery](../SOURCE.md). Legal notices are included by all three
packagers; this does not certify store approval or completed source publication.

The [project mission](../MISSION.md) records the official long-term direction; it adds no restrictions to the standard licenses or to independently operated forks.

- **Stable target presentation (2026-09-15):** a terrain-anchored ring and a thinner
  screen frame track current target/camera transforms after movement and grounding.
  Animation bounds, model turning and decorative pulse/spin no longer move the
  selection marker. Applies to both interface profiles and 3D/2D rendering.

## Current SDK integration boundary

Open Moba pins a specific SDK Passport revision in all four consumers (client,
server, shared and Passport wrapper), rather than following SDK main. The SDK
provides character identifiers and validated Passport delivery/admission types.
The game wrapper performs HTTP delivery, size/hash and humanoid animation checks;
Bevy loads installed GLBs. Ordinary packaged roster avatars are loaded directly
through `AvatarAssetCache` in `client/src/world.rs`. The legacy SDK model catalog
is intentionally empty at startup, so `load_builtin_model_catalog` and its remote
downloader are not the active roster import path. Android uses the SDK without
its optional desktop Bevy/HTTP feature set. These boundaries should guide further
SDK extraction instead of claiming all avatar loading already lives in the SDK.


## Interface composition and combat visibility (0.21.0-rc.2)

The Verdant interface uses shared dark surfaces, champagne accents, jade primary
buttons, and packaged Inter/CJK fonts across menus and gameplay overlays. Home
organizes identity, the selected hero, and matchmaking into one composition.
Collection, card customization, hero selection, queue, loading, and results retain
their existing navigation and server-driven behavior.

In desktop matches the compact minimap, hero resources, four abilities, and
inventory form a tactical dock at the bottom. Objective, target, and boss-buff
information remains in a compact status panel above the abilities. The central
30% of the viewport's upper 30% stays free of persistent opaque HUD panels.
Landscape phone layouts retain the thumb controls, move hero/social status beside
the minimap, and place match status in the gap between the lower controls. This is
an interface change; world visibility and fog mechanics are unchanged.

Phone shell buttons and utility controls compensate for menu scale so their touch
areas remain usable. Essential shell text has a readable minimum size; modals use
real phone pixels rather than inheriting the shell shrink factor.

Help and settings use the same hierarchy and surfaces as the shell. Settings
adjusters have aligned label/value columns and retain scrolling. Account, history,
and friends use selected navigation tabs and consistent hover feedback. Purchases,
reconnect actions, upgrades, input gating, and profile persistence remain available.
Shell settings remain open before a connection is established; the phone utility
bar hides while the shop is open so the shop Close action stays reachable.
Help can be opened from the shell before joining and dismissed with its button
or Escape; phone menu scale is restored after closing. Automatic first-match
onboarding still waits for local admission.

`OMOBA_FRONTEND_QA_OUTPUT` also captures the game menu and settings at two scroll
positions, plus the server-address form. It requires essential navigation and action
controls to fit the viewport. These menu states are explicitly labeled fixtures. The beta UI harness
measures the north sightline, HUD text containment, control bounds, and unrelated
panel overlap in actual Bevy screenshots. Phone previews use the development
`OMOBA_TOUCH_CONTROLS=1` path and do not replace physical-device validation.


### Persistent Ekza account library

Hero selection and Collection connect to Ekza through `ekza-bevy-sdk` and expose
local sign out. The client stores its scoped opaque credential in
`ekza-store/account/session.json` under its private settings directory (0600 on
Unix), restores it with a fresh SDK library check, and refreshes every 15 seconds.
Registry outages retain credentials; authorization rejection clears them. Sign
out removes the local credential and invalidates pending background work;
Studio Account revokes access server-side. Public approved community models
remain free after logout. Catalogue identity, metadata, live entitlement changes
and scrolling stay intact when an account is restored or signed out.

## Xcode iOS archive workflow

Open `mobile/ios/Omoba.xcodeproj`, select the shared Omoba scheme and a physical
iOS destination, then use Product → Archive. A build phase compiles the current
locked Rust source and stages tracked assets plus matching symbols. Xcode owns
signing and Organizer distribution. Local signing/team overrides remain ignored.
See [TestFlight instructions](../mobile/ios/TESTFLIGHT.md); local unsigned archive
validation does not assert Apple upload acceptance.

The Rust staging phase refreshes the executable modification time even when Cargo
reuses its cached binary, allowing Xcode to invalidate its previous signing output.

## Structure movement consistency

Practice bot planning uses the physical structure footprint enforced by movement
authority, separate from combat target reach. Local structure movement uses the
same swept collision primitive and keeps planning clearance when sliding around
buildings, so ordinary 20 Hz position samples do not cut through a structure.

On iOS, winit may emit `RedrawRequested` informational messages and `AboutToWait`
event-order warnings. These concern the native window loop, not UDP sync errors.
Warnings remain visible; the movement fix does not change the native event loop.

## Base recovery and mobile result panels

During a running match, living joined heroes and bots recover 12% of maximum HP
per second inside their own base's shop zone. The server caps recovery at maximum
HP; opponents' bases, dead players and completed matches do not grant healing.

Settings scroll within a bounded body with a fixed Back action. Results use
nonshrinking cards inside their scroll area and fixed header/footer navigation.
Mobile drags use logical window coordinates, respect display/UI scaling and
clipping, and retain pointer ownership until release/cancel. Career buttons
activate on short release; scrolling cannot trigger a button underneath the finger.

Live K/D/A and team kills are read from the authoritative snapshot scoreboard.
Older running server binaries that omit that field show unavailable statistics;
restart the server from current source as well as rebuilding the client.

## Ekza connection and LAN Studio testing (0.22.0-rc.5)

Avatar collection account/catalogue messages update existing labels without replacing held controls. Wallet pairing has visible progress and approval-page retry. iOS opens approval links using UIKit on the main queue; scoped Ekza account sessions still restore from private storage.

An explicit Debug-only local Studio configuration and `scripts/ekza_lan.py` connect the game, account service and approved catalogue to the same local Registry. The SDK's exact private-host development exception is ignored by release builds. See [LAN walkthrough](ekza-lan.md) for the real author → curator → game-owner → player flow. No wallet is required for approved free avatars.

## Combat Test sandbox

The explicit local development launcher `python3 scripts/combat_test.py` opens a hero picker or directly enters a configured scenario without matchmaking. Its Dev Panel exposes authoritative progression, skill ranks/unlocks, health/mana, combat multipliers, all shipped items, reset and teleport; configurable enemy AI, damage dummy analytics, direct loopback 1v1, minion controls and simulation pause/speed/frame stepping share the real game rules. Native animation previews and geometry/state overlays support combat debugging. Named JSON presets survive ordinary rebuilds. Ordinary release/practice and career ratings reject sandbox mutation. See [Combat Test](combat-test.md) for complete launch/control/measurement semantics.


## Team vision and gameplay brush (0.23.0-rc.4)

The 3D battlefield uses server-owned shared radial sight from living allied heroes,
minions and structures. Unseen enemy actors are omitted from each recipient's
snapshot, including indirect projectile/event channels; the public scoreboard stays
available. Fresh target-locked attacks and bot/minion/tower acquisition require
visibility. Already-launched homing attacks can still land after concealment.

Ten mirrored, walkable tall-grass patches provide hero concealment. Entering the
same patch grants detection; an accepted hostile targeted attack reveals its caster
for two seconds within enemy radial sight. Soft fog follows the team sources on the
battlefield and minimap, and an in-brush label reports concealed/revealed status.
See [team vision rules and verification](team-vision.md). Terrain line of sight,
wards and true invisibility are outside this iteration; 2D presentation is paused.
