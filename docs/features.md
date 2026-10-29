# Feature Inventory

## Compact mobile battle HUD

The landscape touch HUD uses an 80 px attack disc, 50 px skill faces and 38 px
utility artwork at its reference scale. Utility and target-selection capture
regions remain at least 44 px; each finger retains its captured action while
crossing neighboring buttons. The joystick keeps its familiar size with an
inward anchor and a light transparent base. Recall, Haste and Dash sit along
the bottom, opening the upper battlefield. The score is a shallow ribbon; small
settings/chat discs beside the minimap have independent larger touch cells.
Ally portraits, quick-buy art, mana pills and cooldown detail follow the compact
phone presentation. Saved HUD group offsets remain bounded; desktop layout and
menu scaling keep their existing behavior.

The movement stick floats within the lower-left region below the information
stack: each fresh movement press chooses its base, drag moves the thumb, and
release stops movement while retaining the last base. Safe-edge clamping starts
neutral; a second finger cannot relocate the owned stick or shift combat buttons.
Phone kill notices appear below purchases on the left, with two compact rows or
one on short screens. Selecting a nearby visible enemy portrait also shows a
small gold direction arrow in camera-relative screen axes. Hidden/dead/stale
entities and public death timers supply no direction.

The selected-target plate is144×40px at reference scale and shares the top
safe edge with all four allied portraits. Public enemy death portraits are
passive32px cells below allies; they contain no click target, direction vector
or hidden position. Nearby living enemy lock targets retain44px right-side
cells independently. Settings/chat are neighboring44px cells below the death
row. If the narrowest phone cannot fit the map and all header blocks side by
side, the104px map moves below the header while allies and target remain at
the top. The left purchase/feed stack and movement bounds follow that map.

`python3 scripts/check_phone_hud_layout.py` compiles and runs the same std-only
Rust geometry used by the game, without Bevy, a window, assets or a GPU. Its
contract enforces simultaneous top anchors, centering, actual ally cells,
death/live separation and non-overlap over a deterministic width/inset grid;
negative cases reject the previous lowered-avatar geometry.

Verification uses one English landscape phone preview and focused geometry/input
tests. Physical Android play and long-session comfort are reported separately;
a desktop-rendered preview alone does not establish either.

## RU website and native registration

Website-first VK ID, OK and Mail.ru sign-in creates one default-stat regional
profile/session; repeat sign-in returns the same profile. Game-initiated unknown
subjects still require the active game's explicit signed link to preserve progress.
Device review binds the expected profile, temporary polling errors are recoverable,
status traffic cannot consume completion quota, and revoked keys cannot race a new
social link. See [registration](registration.md) for schema 10, source boundaries,
real local browser/signed-game proof and remaining live-provider rollout gates.

## Runtime account regions

Native capture launchers seed a returning Global routing record in their own
scratch configuration; the dedicated first-launch QA intentionally starts empty.

Default clients select RU or International before regional traffic and restore a
returning account's region automatically. Lobby, Account API, portal, legal gate,
Ekza features and purchase offers follow one immutable binding. Keys and saved
state remain in separate regional folders; switching stops regional traffic and
requires a cold restart. Player-facing raw server configuration is removed in
production and dev. Native enrollment replies expose their owning region and
foreign replies are rejected. Dev denies both production IPs. The focused native
RU phone-preview harness exercises first confirmation and switching; full store
rollout and physical devices remain release gates. See [profiles](connection-profiles.md)
and [release preparation](plans/ru-release-2026-10-28.md).

## Markets and sign in with VK ID

Regional server/API builds and locked client packages serve one market: `global` (today's product), `ru` (Russian servers,
no Ekza ecosystem, Hero Supporter auras sold only through YooKassa donations on
the RU portal, Russian by default) or `cn`
(prepared, not packaged yet). `release.py --market ru` makes the RU set with its
own servers, settings folder and Android app. RU players sign in with VK ID
(VK, OK, Mail.ru) on the RU portal and from Profile in the game. Website registration
can create a profile before a device exists; game-initiated linking preserves its
existing profile only after the active game key confirms. Game authentication uses
the enrolled device key. On the RU host the portal calls the Account API on loopback
and forwards each player's address, so sign-in limits stay per player;
`release.py preflight --market ru` checks that path
([topology](../ops/vps/ru/README.md#topology)). One tool, `ops/vps/ru/ru_host.py`,
sets a fresh Ubuntu host up in Russia (PostgreSQL with ICU, Caddy, firewall, key-only
SSH, secrets made on the host), ships the lobby, Account API and portal with rollback,
checks the whole host and proves a reboot needs no manual step; every service there
restarts on its own. `ops/vps/ru/ru_backup.py` backs the host up every night and before
every migration: the database and the match archive, encrypted and signed, go to storage
in Russia (14 daily and 8 weekly kept). A monthly drill restores the newest set into a
scratch database and checks it. The secrets are escrowed offline, and a missed backup
raises an alert. Each release runs the
[RU release checklist](../ops/vps/ru/release-checklist.md). See [markets](markets.md).

## Temporary community sculptures

One decorative slot beside the Wendigo pit displays the Registry-approved
24-hour placement. Exact-byte model validation, profile-isolated caching,
monotonic expiry and bounded revocation keep community art outside gameplay
simulation. Clicking or tapping the rendered sculpture opens its approved title,
credit and optional 1,000-character plain-text description in a scrollable card.
Opening and closing gestures cannot trigger movement or combat; revocation or
replacement closes stale cards. An original lucky waving cat exercises the
optional rotation clip, which continues while the card is open.
The coordinated Web/Studio pilot is free; paid crypto providers and hosted
deployment remain separate work. See [runtime details](community-sculptures.md).

## DEV account integration

Packaged OMOBA Dev uses `account.dev.omoba.io` and `app.dev.omoba.io` for account
confirmation and device-account features. Runtime overrides cannot redirect a DEV
app to production. The DEV Account API keeps Ekza library consent on DEV Registry
and Studio. The [DEV host helper](../ops/vps/dev/README.md#dev-account-api)
prepares its isolated runtime role and HTTPS ingress; activation verifies the exact
DEV binary and portal schema. Provider setup and tester availability remain separate.

## Dev iOS workflow

The shared **Omoba Dev** Xcode scheme builds the separate `space.ekza.omoba.dev`
app for the dev contour. Run and Archive select dev settings without editing
local contour values. The release script selects the same scheme. Apple signing,
TestFlight upload, tester assignment and physical-device checks remain separate
steps: [dev quickstart](dev-quickstart.md).

Android launch diagnostics: `make android-diagnose` records the connected phone's
OS, ABI, graphics capabilities, installed OMOBA version and app crash/exit evidence.
Read-only by default; explicit `--launch` is available. See
[mobile diagnostics](../mobile/README.md#silent-launch-failure-and-android-12).
Universal APKs cover multiple CPU ABIs; a GLES fallback is not implemented.

## 0.44.0 delivery

Wildspark and the creator/workshop integration are merged, the Linux production server is updated, and a matching Windows x64 candidate is built locally. See the [delivery record](progress/2026-10-10-release-044.md) for source, checksums, compatibility, rollback and testing limits.

## Dawnweaver reference iteration (next candidate)

Horizon Ray is now centre-weighted: a target whose centre is within 0.35 of the beam
axis takes the full 80, and damage eases (smoothstep) to 60% at its graze point, the
beam half-width 0.8 plus its own radius. The Radiance detonation and the re-applied
mark keep their full value. The caster stands still during the 0.8 s windup so the
warned ray leaves from her (basic attacks and the field recast stay allowed); the
client predicts the stop. A due ray survives a late server tick, a blocked spell no
longer marks, and marks include the team damage buff.

The fired ray draws its white core exactly at the damage core, with thin core lines
in the warning; core hits draw a brighter burst than edge hits, Radiance detonations
a gold pop, and the release an afterglow along the strip. A dedicated charge motion
fills the windup and a two-hand push with recoil fires the ray; the lane facing lasts
through the release. Every listener hears a rising charge when the warning appears,
gained at the nearest point of the lane, then a new synthesized beam voice; core hits
and Radiance bursts have their own cues. Direction skills whose effect ignores aim
distance (beam, linear projectile, returning shield, impact rocket) send the far point
of the aimed ray, so the server line meets the preview. Bots keep a Luminous Field
until an enemy hero is about to leave it, do not spend damage skills on towers and
lead the ray.

Receipts gain additive `core_hit` and `radiance_burst` flags (protocol 11 decodes
them as absent). Catalog `standard-kits-8` and gameplay revision
`dawnweaver-2026-10-11-reference` require matching client and server. See
[implementation and verification](progress/2026-10-11-dawnweaver-reference.md).

## Wildspark reference iteration (0.44.0 candidate)

Switchfire keeps a fast, stacking single-target repeater and a slower launcher:
each accepted rocket autoattack costs 4 mana and splashes in a 2 m radius.
Shockline is an aimed long-range shot that stops at its first enemy and slows it;
Snapline opens three mechanical traps during their 0.6 s arming period.
Last Spark now accelerates from 18 to 45 m/s at 54 m/s², passes minions, and
explodes at its first enemy hero. Its base damage rises from 50% to 100% over
20 m, then adds 25% of **each** nearby enemy's missing health. The blast radius
is 3 m (unit collision radii participate); five sufficiently wounded heroes can
all die in one blast. Healthy and out-of-range controls remain separate.

Original gunmetal/brass models add a rotating barrel assembly, launcher recoil,
opening trap jaws and finned rockets. Accepted W/R temporarily draw the launcher;
explicit skins remain selected. Barrel flashes follow the actual hand socket.
One server-confirmed detonation produces one compact ignition, expanding shockwave,
falling fragments and delayed embers, regardless of victim count. Damage numbers
remain per victim; a shielded detonation never prints invented damage. Fog hides
unseen blast centres, and a vanished projectile alone produces no explosion.

Protocol 11 remains additive-compatible; gameplay revision
`wildspark-2026-10-09-reference` and catalog `standard-kits-7` require matching
client and server. See [implementation and verification](progress/2026-10-09-wildspark-reference.md).

## Creator asset catalogue (first iteration)

The [asset catalogue exporter](progress/2026-10-10-asset-catalog.md) joins canonical class recipes,
skills, presentations and authored asset briefs into a versioned static snapshot.
It distinguishes licensed bundled handheld references, engine-owned effects and
motions, and planned projectile/prop bindings. The companion `omoba-web` gallery
uses the same snapshot; offline Blender thumbnails and content-addressed GLB
downloads show current defaults without requiring a running game or Account API.

An artist can prepare a requirement/revision-pinned brief and use the existing
Studio upload route. This is discovery and preparation: Studio does not yet
persist exact requirement bindings. The game equips a one-GLB
[Wildspark set](handheld-weapons.md#wildspark-set-repeater-and-launcher-in-one-glb)
with both Q guns, but no other coordinated sets or remote projectile/prop skins.
See the [iteration record](progress/2026-10-10-asset-catalog.md)
and parent [design PR #78](https://github.com/o-moba/omoba-bevy/pull/78).

## Idea: rating-unlocked skill slots

Recorded 2026-10-12, not scheduled: as a character's rating grows it could unlock
more than four skills, possibly up to about ten. The [idea and open questions](plans/rating-skill-slots.md)
cover which rating counts, active versus known skills, ranked fairness, controls
and the protocol impact.

## Planned: Studio model requirements by class and skill

The [proposal and TODO](plans/studio-skill-asset-catalog.md) describe a game-owned
requirements catalogue shared by Studio and the website: baseline 3D previews,
artist contributions to specific skill/model roles and eventual coordinated
cosmetic sets such as Wildspark’s two guns. The public discovery catalogue and local class workshop now exist,
and the game equips a one-GLB Wildspark set. Exact-slot Studio submissions and
other coordinated sets remain planned.

## First-match guidance and mobile comfort (0.44.0 candidate)

The guide starts with the objective: clear one lane with allied minions, then destroy the enemy main tower. Loading shows five advice cards, navigable with arrows or a single-finger horizontal swipe in the text area. Settings → HUD → Beginner tips remembers whether loading advice and the automatic first-match guide appear; manual Help remains available. Older preferences enable advice by default.

Creature health bars are smaller, especially for minions and neutrals. Melee, hit and tower effects use lower audio gains. The mobile profile starts at 0.8 camera zoom; saved zoom choices remain intact. Phone help cards grow to fit their text inside the scrolling guide. See [the integration review](progress/2026-10-09-first-match-merge-review.md).

## Class visual identity and skill fixes (0.44.0 candidate)

Every skill of every class has its own staged presentation. The 68 skills and
17 basic attacks each read a row of data: a cast accent, a motion from the
shared 55-clip library, a body built around the replicated geometry of the
effect, a hit recipe drawn on the confirmed receipt and a voice made from the
existing samples. No two skills share motion family, body and impact, and a
skill keeps its look on any button and in any recipe. Warrior and Warden basic
attacks no longer throw a blade, every legacy projectile ability has a flight
form of its own; the later Wildspark iteration above replaces its canister with a finned launcher rocket. Aim previews of the 48 modular skills are drawn from the server
rule. Status effects are mesh shapes read from replicated flags, and Northwall
shows each projectile it stops.

The presentation follows fixed honesty rules: a hit, a link or a number needs
an accepted combat receipt; a boundary equals the replicated geometry; an
effect that vanishes is not drawn as a hit; nothing of a hero in fog is drawn
or implied. A few looks are smaller than the shapes they replace for that
reason; the list
is in [combat cosmetics](combat-cosmetics.md#where-the-new-look-is-weaker).

The same candidate fixes skill mechanics that contradicted their own
descriptions, without changing a catalog number: hero-only picks for Patient
Curse, Fourfold Duel and Orbital Guard, no Thunder Kick on a tower, a root that
blocks the recast dash of Echo Strike and Iron Hook, a Thunder Pulse recast
only after a hit, Rift Seal on unprotected enemy structures, no doubled self
shield on Sheltering Leap and Anchor Step, orb and field effects that no
longer cut longer ones short, a Fault Line pillar and a Furnace Breath
telegraph replicated as the server uses them, and receipts from unseen heroes
that reach their victim without naming the attacker. Two of these fixes are
rule changes that await the owner's decision: Anchor Step lost its doubled
self shield together with Sheltering Leap, and Rift Seal now marks an
unprotected enemy structure. Practice bots heal and restore mana by need. The
client refuses, with a line of feedback, casts the server would drop: a Rift
Step onto a blocked point and an ally skill without an ally; a tap on
Sheltering Leap or Orbital Guard aims at an ally.

Gameplay revision `combat-2026-10-08-mechanics` with protocol 11 and catalog
standard-kits-6: client and server ship together. Verified on stills of one
English 1280x720 desktop viewport and by tests; no physical device, no phone
frame time, no multiplayer session with people, nothing judged in motion. See
[the iteration note](progress/2026-10-09-class-visual-identity.md).

## Boot splash (0.44.0 candidate)

A normal launch opens on a full-screen splash instead of an empty Home: the living Verdant arena artwork, the OMOBA title and tagline, the build label, a spinner and a progress line ("Loading the interface…", "Loading the world…", "Entering the arena…") fed by the startup fonts and world scenes. It stays at least 1.4 seconds and at most 15, blocks the menu underneath as the top modal (and the Escape and help keys), and leaves through a short dip into the base colour. On iOS the native launch screen shows the same dark base colour before the first engine frame (Xcode and TestFlight builds; `make iphone` bundles do not compile the asset catalog). Normal desktop launcher settings (`OMOBA_ASSET_DIR`, `OMOBA_MATCH_MODE`, `OMOBA_TEAM_SIZE`, `OMOBA_DEBUG_UI=0`) keep the splash. Harness, capture, sandbox and enabled debug-UI launches start without it; `OMOBA_BOOT_SPLASH_SHOTS=<dir>` captures it. There is no Android native splash theme yet.

## Mobile playtest quality (0.44.0 candidate)

The new candidate removes automatic account-history result popups, gives textured
buttons a single fill, separates collection navigation from its scrolling grid,
and makes phone previews and pause-menu actions more consistent. Brush darkness
no longer paints through scenery; concealed avatars use an opaque muted tint and
the eye-off indicator to preserve clean VRM surfaces. FPS measures render
submissions independently of the simulation clock.

Wildspark's rocket accepts grazing hero hits with a 1.05 m projectile radius;
long touch drags reduce angular sensitivity while keeping preview and release in
sync. Round starts and practice bot creation separate occupied spawn positions.
The dragon pit hosts Wind, Stone and Flame in that order, with no fourth spawn:
90 s initial delay, 75 s between kills/spawns, 180 s team buffs of +8% movement,
+20 armor and resistance, and +12% damage respectively. Buffs do not stack with
themselves, expire normally, affect respawned teammates and reset each round.
Wendigo is unchanged. Protocol 11 / standard-kits-6 requires matching client and
server; the gameplay revision of the candidate is now
`combat-2026-10-08-mechanics` (see the section above). See [verification status](progress/2026-10-06-mobile-playtest-quality.md).

## Release compatibility (0.41.0)

A stable pre-game check compares protocol, skill catalogue, map and gameplay revisions independently of release labels. Mismatches have an actionable explanation; unanswered checks remain unverified. All network endpoints are checked, while offline practice stays local. The release builder retains a contract manifest; the CLI compares artifacts and probes servers. [Contract, release sequence and rollout limits](release-compatibility.md).

## Retained online match seats (0.40.1)

Career polling preserves a registered server-controlled human while its transport is disconnected. The authenticated owner can reclaim the same hero; ordinary unauthenticated endpoints still lose admission. Protocol 9 remains compatible with iOS 0.40.0 (19). See [the Beta patch and deployment evidence](progress/2026-10-04-beta-0401.md).

## Equipped skill bindings and shared control (0.40.0)

Accepted recipes can bind four unique compatible skills to any Q/W/E/R buttons, including four different ultimates. Each skill keeps its authored unlock role (levels 1/2/4/6), rank limits, resource costs, cooldown, targeting and presentation when moved. Capability checks remain: weapon switching needs a Repeater core, and orb field/pull needs an equipped orb controller. Duplicate skills and malformed or mismatched recipes are rejected.

Desktop, phone and controller paths share the accepted metadata for casts, aim previews, localized names/icons, cards, upgrades and total cooldowns. Server remaining cooldown and recast state stay authoritative. Stormfist follow-up costs now follow the equipped skill across cores; admitted Bluff consumes Brittle once through the common online/offline control and damage paths. Existing control exceptions are documented explicitly.

Optional authored recipes are available through the existing opt-in local Combat Test configuration. Normal selection still offers presets; a public class editor, saved account recipes and website equip are future work. The current contract requires matching **protocol 9 / standard-kits-4** builds. Local verification passes: the complete strict gate, 50 harness tests including the live mixed-kit route, and two English 852×393 synthetic HUD states. Physical-device acceptance remains separate. See [the contract](equipped-skills.md) and [current evidence status](progress/2026-10-04-equipped-skills.md). Earlier version sections below describe their historical contracts.

## Configurable lane defense and lane brush (0.39.0)

The default arena has three towers per team on each lane: an outer tower, an inner tower and a final tower near the Nexus. All 18 towers and two bases are server-owned; siege protection follows the remaining active tiers. Map configuration can disable tiers, with ready one- and two-tower variants for faster matches. Six additional brush pockets touch the lane corridors and use the same authoritative concealment rules as jungle brush. Existing player movement, minion passage and original object identities remain intact. See [configuration](map-customization.md) and [verification](progress/2026-10-04-lane-defense.md). Matching protocol 8 client/server builds are required for the new compiled brush geometry.

## Touch movement response (0.38.0)

The thumb-stick reaches full speed at its visible travel limit (70% of the base radius), and its knob tracks the finger directly inside that range. Reversing from an extended forward drag to the opposite visible limit retains full running speed. The inner range still supports analog walking; slow, haste, root/stun and collision rules remain authoritative. See [measured regression and scope](progress/2026-10-04-movement-reversal.md).

## Adventurer dagger class (0.38.0)

Adventurer adds four close-range skills: Deadly Blow, Bluff, Backstab and Lethal Blow. Bluff briefly stuns and turns the victim away; rear geometry uses authoritative facing. A qualifying rear Backstab can leave a surviving enemy hero at 1 HP with a 2% authority-owned chance. The class uses the shared online/offline engine and recipe resolver, a hand-attached dagger and skill-owned humanoid animation. [Class contract and balance](adventurer.md) document exact limits; [implementation verification](progress/2026-10-04-adventurer.md) records current release status.

## Mobile playtest round two (0.37.0)

Quick taps assist projectile aim toward nearby visible opponents; drag aiming remains manual. Long-range targeting adds minimap intersection crosses, and nearby hostile towers show their attack area. Bots require living allied minion cover before entering it. Four river butterfly locations supplement the forest pickups; confirmed pickup and trap-trigger events have distinct sounds.

The compact score includes authoritative match time. Death timers start at 5 seconds, begin growing after minute 2 and cap at 35 seconds. The 16-item shop uses component credit, three price tiers and critical/lifesteal build paths. Hero kills award 500g, reduced for repeated defeats, with a bounded 100g assist pool. Postmatch shows both rosters, KDA and earned income. [Economy model and balance](economy.md) records assumptions and timing.

Allocated running matches retain disconnected heroes, default to bot control and accept authenticated teammate votes for idle/bot policy. A saved resume hint allows reopening the same match after restarting the app; the server validates the session and match identity. Phone draft selection uses tabs and per-character rotation. FPS visibility is saved separately from its limit; diagnostics distinguish measured app cadence and native display-link cadence. Physical iPhone performance remains to be measured.

The matching protocol 6 server and client are required for the new inventory types. This work is prepared locally; see [verification and rollout status](progress/2026-10-04-playtest-round-two.md).

## Mobile playtest follow-up (0.36.0)

Home always offers Offline Practice, including while connected. It runs the shared combat engine locally, produces no saved history or progression, and supports local debug chat, reactions and item purchases. Server bot matches remain a separate online option. Nearby alive enemies visible to the client have exact-identity targeting portraits; hidden enemies never enter this rail. Gold and quick-buy sit directly below the minimap. The remaining basic-attack instruction banners are removed.

Recall draws a cyan/gold channel around the hero for the authoritative timer and disappears on interruption. Recipe-based heroes send dash, haste and recall through the same signed server command path as legacy kits. Chat lines expire after 30 seconds, overhead messages after five seconds, reactions after 2.8 seconds and errors after four seconds; successful delivery has no persistent status banner. On phones, successful send dismisses the keyboard and reveals readable, touch-scrollable history. Only the latest overhead chat or emote appears per hero.

Allocated matches retain disconnected seats for up to three minutes without pausing combat. Expired reconnects return Home; terminal results block gameplay even when the final snapshot still says Running, and Play Again cannot reuse a retired allocation. The Home hero is larger with bounds-aware framing; phone navigation has smaller separated buttons. iOS requests the selected supported cadence, but actual FPS remains dependent on the device, workload and OS. See [implementation and verification](progress/2026-10-04-mobile-playtest.md).

## iPhone gameplay controls (0.35.0)

The compact phone HUD places skills and smaller minion/tower controls on one circle around attack, with a uniform 42-degree step. Skill sizes, order and vertical positions remain unchanged; the whole combat group moves 48 logical points left at normal scale to fit the additional right-side 44-point targets. Allied portraits sit beside the top of the map, the selected target is centered, and the bottom gap holds compact kill notices. Tap an allied portrait to follow that actor; movement, manual navigation and recentering return camera control. Finger panning is faster. Attack targeting has no instructional box, uses the current camera projection, and dash supports press-drag-release direction selection with a landing preview and camera recovery.

Return to base is a separate seven-second server-authoritative utility. Movement, damage, combat, death and disconnection interrupt it; successful completion preserves health, resources and cooldowns. Phone controls show channel time, while desktop uses B. The new commands require protocol 5 on both client and host.

Settings save a 60/120 FPS ceiling and bounded joystick/combat-group offsets, with a separate layout reset. iOS uses CADisplayLink; the OS and hardware can lower the requested rate. The small FPS label measures actual app frame cadence. Hidden local heroes retain their material fade and gain a crossed-eye marker. Tapping the hero opens the reaction wheel; server-confirmed reactions remain visible while an image loads. The [account-owned sticker contract](sticker-assets.md) records the future SDK/Studio integration separately from existing reaction entitlements.

## iOS export metadata (0.34.2)

The common iOS plist declares the reviewed game encryption exemption. Device packaging, Xcode archives and prepared TestFlight archives preserve the Boolean declaration. Previously uploaded builds still require their own App Store Connect answer. See [the assessment and reassessment triggers](../mobile/ios/EXPORT-COMPLIANCE.md).

## iPad combat polish (0.34.1)

Skill + badges directly upgrade eligible abilities and own a 44-point touch target, preventing world taps beneath them. Basic attacks no longer show routine no-target or aiming instruction panels. The timed kill feed sits below the top-right HUD. HP and mana retain fixed heights below the nickname. All surviving towers are public map landmarks, while attack visibility rules remain unchanged. A flying impact rocket carries eight metres of temporary team sight and a minimap marker, ground light and brighter exhaust. Server-confirmed concealment makes the local 3D avatar and handheld translucent; reveal restores their original materials. Shared avatar assets and opponents remain unchanged. Verification status: [session note](progress/2026-10-02-ipad-combat-polish.md).

## Hosted cosmetic equipment (0.33.0)

Avatars → Weapons and the pre-match draft expose class-default, empty hands, shipped props and approved free Studio weapons. The typed SDK downloads each exact approved rendition into the writable Ekza cache, verifies its hash/size and static attachment profile, and mounts it through the same asset source on desktop/mobile. Normal joins and draft choices carry the cosmetic item ID; server-owned catalogue reads authorize it independently. Other clients install the replicated item on demand. Equipment never changes combat statistics. See [the lifecycle implementation](progress/2026-10-01-asset-lifecycle.md) for acceptance evidence and limits.
## Combat UX (0.32.0)

Standard classes share dash and haste utilities again. Long skill aim uses blue
terrain-following corridors and a clipped minimap vector; traps have stronger
outlines and map markers. Camera panning keeps a stable heading. Touch map taps
and world taps issue routes, drags pan, and the stick overrides an active route.
Tablet minimaps are 20% larger; phone size stays unchanged. Mobile hero plates
show level, nickname, HP and resource above the character. Allied portraits/HP
sit beside the minimap; confirmed hero kills appear in a bounded timed feed using
public identities without exposing fog-hidden locations. Practice tools include
stationary and circular-moving nearby targets. See the [task TODO and evidence
scope](progress/2026-09-29-combat-ux.md).


## Ekza demo integration (0.31.1)

Client and server use the same pinned Ekza SDK main revision. The live publication procedure and remaining deployment gates are in [the demo progress note](progress/2026-10-01-ekza-demo-main.md). A local twenty-avatar catalogue is verified separately from real Studio publication.

## Local SDK Twenty collection (unreleased)

A developer launcher serves 20 additional curated avatars through the real Ekza SDK, with persistent downloads, existing humanoid animation retargeting and independent server admission. Generated models remain separate from bundled game assets. This is a local catalog, not public Space publication. See [build/play instructions](../assets-src/sdk-avatar-pack/README.md) and [verification status](progress/2026-10-01-sdk-avatar-pack.md).

## Handheld weapon pilot (unreleased)

Warrior uses a built-in sword. Combat Test can equip a sword, hammer, scepter or installed Ekza handheld on any class, or leave its hands empty. Props follow semantic VRM hand bones through idle, running and attacks; choices replicate to other actors and persist in sandbox presets. They do not alter damage or skills.

The existing Ekza SDK imports explicitly approved free `handheld-glb-v1` renditions with SHA-256/size checks and grip metadata. Native verification covers Agnes, Orion and an independently equipped opponent using a local HTTP catalog. The 0.32.0 runtime extends this pilot with a normal equipment picker and typed hosted catalogue. Paid ownership, finger posing and two-handed weapons remain future work. See [the contract](handheld-weapons.md) and [verification results](progress/2026-10-01-handheld-pilot.md).

## Full-roster Offline Practice (0.31.0)

Home → Offline Practice accepts all 16 classes without a server, account, matchmaking or gameplay socket. The internal `common` crate owns the same authoritative combat used online: all skills/passives, targeting, cooldowns/resources, projectiles, minions, towers, neutrals and bosses. Practice starts at level 6 with normal skill ranks and a 5v5 lane-bot roster. Debug tools can clear/restore bots, spawn a nearby stationary dummy, configure a same-class duel, or toggle god mode and speed. Snapshots use the normal HUD and effects; leaving clears local combat and restores the saved online address. Practice produces no saved career result or account rewards.

The extracted engine retains its AGPL-3.0-only source license. Database, passport admission, authentication and transport hosting remain outside it. See [the extraction note](progress/2026-09-29-common-offline.md).

## Deliberate skill inspection (0.30.1)

Skill descriptions open after holding the same skill for 1.5 seconds using a hotbar press, Q/W/E/R, a controller skill binding, or a stationary touch. Hover, short presses and basic attacks keep the combat view clear. Releasing the hold or opening a menu hides descriptions immediately. The standard-kit explanation panel follows the same rule; ordinary casting remains unchanged.

## Expanded standard roster (0.30.0)

Eleven modular presets now cover the standard roster. The nine additions are **Cinderforge, Edgeweaver, Stormfist, Veilstalker, Emberveil, Orbitwright, Riftshot, Chainkeeper and Frostguard**, each with a passive, basic profile and Q/W/E/R. Reusable mechanics include parry, charm, camouflage, Energy, persistent orbs, temporary terrain, forging, ally rescue and projectile interception. Keyboard, touch and controller use the same authoritative commands. The roster scrolls independently of avatar selection. See [implementation and remaining TODO](progress/2026-09-29-remaining-roster.md).

## Standard kits and reusable skill recipes (0.29.0)

**Dawnweaver** is a ranged control/support kit: spell marks empower attacks, a snare catches two targets, a returning projectile shields allies, a slowing field detonates on recast, and a telegraphed beam finishes the combo. **Wildspark** is a ranged carry: switch between a stacking repeater and Mana-consuming splash rockets, catch enemies with a slowing shot and armed traps, fire a long-range finisher and accelerate after qualifying takedowns. Both classes use any supported avatar and keep the existing level/skill-unlock system.

This work introduced reusable skill definitions separately from four-slot presets. Versioned `BuildRecipe` data chooses a core, passive and skills; the current resolver validates unique skill identities and capability dependencies, with arbitrary bindings added in [0.40.0](equipped-skills.md). The catalogue now has 48 reusable skills across twelve modular presets, alongside five legacy kits. Public construction, persistent user recipes and website synchronization remain deferred; authored recipes are limited to internal tests and the opt-in local laboratory. Numbers are initial tuning values, not a competitive balance guarantee.

Server-authoritative aim, cooldowns, costs, control, shields, weapon modes, effect lifetimes and fog-filtered snapshots use protocol 4. New peers reject incompatible protocol versions explicitly. Desktop, touch and controller paths share aimed casts; original icons and English/Russian/Chinese text accompany the hotbar, recast/mode/status feedback and effect geometry. As of 0.31.0, every kit also runs in socket-free Offline Practice through the same authoritative engine. See [implementation, architecture and remaining TODO](progress/2026-09-29-standard-kits.md).

## iPad phone-layout preview (0.28.6)

On an eligible landscape iPad, Settings → Graphics → iPhone 16 layout preview temporarily centres the actual game window at 852×393 logical points. Existing phone menus, HUD and window-local touch input work without a separate renderer. A native Return to iPad button remains outside the game; returning, backgrounding or a real change of the host geometry restores the tablet window, while tilting the tablet no longer does (0.44.0 candidate). The option is absent on phones, Android and desktops, cannot be enabled through a stale action, and is not saved in preferences. This previews layout using the game's phone safe-area policy, not iPhone GPU performance or exact physical screen size.

## Party and prematch stage (0.28.5)

The party lobby displays up to five live avatars, with the viewer in the centre foreground and teammates staggered behind. Mouse or touch dragging turns the heroes without moving the formation. Desktop, phone and tablet have a separate scrolling social panel for invitations, party members and online players. Current server presence is shown as “In lobby”; there is no invented ready vote. Bot practice and public quick match keep the existing leader-controlled party flow.

The same renderer serves the authoritative draft and shared countdown/loading preview. Class, avatar, lane and lock/readiness labels come from server snapshots. The local hero stays visible while choosing; teammates' models appear on confirmation, with empty stands labelled PICKING beforehand. Once the roster has gathered, a 30-second selection timer starts and can end early when everyone locks. At expiry, accepted selections lock after any pending avatar-admission check finishes; loading still waits for every client's assets. This does not add cross-server invitations, late joining or new matchmaking rules.

## First impression UI (0.28.4)

Home owns a unified, inset header on desktop and tablets, including its touch utility controls, with the build label retained inside the footer. Hero Select includes the selected 3D character on compact phones, larger portrait tiles, class icons, touch catalogue scrolling and a separate admission notice. Settings uses a shared icon rail, aligned cards, real volume/lighting sliders and fixed close/back actions. Touch scrolling does not change slider values. Background paintings reveal even when their window is unfocused.

## Tablet menus, Russian and offline lane practice (0.28.3)

Home's decorative frame uses equal viewport insets on tablets and desktops; its character and controls retain their fitted composition. The top shading fades from the screen edge, and tablets use a single Help/Menu/Server bar. Graphics settings share aligned label and control columns with fixed close/back navigation. Mobile menus leave matches without quitting the application; desktop retains Exit. Settings also includes Русский alongside English and 简体中文, with the choice saved locally.

Offline practice now spawns the authored towers/bases and minions on all three lanes for both teams. Minions march and fight, towers shoot, and player attacks/skills can damage valid enemy minions and unprotected structures. Waves and projectiles are bounded. Destroying a base stops its team's waves; practice remains open until Leave practice, and rejoining resets structures. This is local training, without career rewards or ranked settlement. The new build still requires compilation and device playtesting; only the prior build was captured on the connected iPad during this change.

## Tablet UI and interactive previews (0.28.1)

Touch input no longer implies phone-sized Home/Collection/picker/scoreboard layouts. Home centers its composition on tablets and preserves the living backdrop during data refreshes; touch drives subtle parallax and automatic detail reduction crossfades. The Home hero uses the painted platform, supports drag rotation and plays an available expressive animation on double tap, returning to idle. No new wave animation asset is shipped.

Avatars has a larger framed preview and All / Included / Studio filters. Settings separates Sound, Graphics, Camera and Language, keeping Controls and navigation reachable. The spacious offline picker keeps the live hero preview and omits online library/account controls. Match Score has framed team tables, avatar/class identity, portrait levels and local-player emphasis. Offline gameplay remains the existing skill sandbox; [tower/minion parity and richer practice tools](plans/offline-practice-parity.md) are a separate planned track.

## Verdant Crown release interface (0.28.0)

The critical player loop now uses the approved handoff rather than the foundation placeholders. The desktop and phone match HUD have compact framed vitals, abilities, inventory/quick-buy, target, score and minimap surfaces; loading and post-match have dedicated responsive layouts and controller focus; victory and defeat select their own painted scene.

Menus now sit in four layered living environments: Arena, Stage, Victory and Defeat. Each scene uses resolution-aware cover art, foreground parallax and restrained deterministic particles, with contrast bands and a dark focus keyline wherever controls float over the painting. Settings → Motion → Reduce motion is saved and switches to a static composite. Phones use 720p layers and can automatically retain only the plate if sustained menu frame time is too high. Career, Supporter and Settings use Arena only outside a match; when opened during play they keep the live world visible. See [the UI kit](ui-kit.md) and [implementation note](progress/2026-09-28-ui-p0-living-integration.md).

## Verdant Crown UI foundation (0.27.0)

The whole interface now draws from the approved Verdant Crown design: gold-framed emerald call-to-action slabs, dark glass secondary buttons, red danger buttons and team lock-in bars, Cinzel display titles and buttons (a serif Chinese face in 简体中文), Inter body text and Barlow Condensed numbers, a gold gamepad focus ring with a soft halo, and the Verdant colours on every existing screen. Menus scale up on large desktop windows (1920×1080 shows the 1280×720 layout at 1.5×), and the Home and party-lobby 3D previews sit on the menu background instead of a teal box. Settings shows the icon and font credits. For developers, `OMOBA_UI_GALLERY=1` opens a gallery of every kit component (buttons, icon buttons, steppers, tabs, toggle, slider, language cycle row, text field, panels and modal, list rows, badges, tooltips, toasts, bars, ability buttons, item slots and shop cards, hero tiles and portraits, scoreboard rows, timer rings, HUD plates) in every state, for desktop and phone, in English and Chinese. Design tokens are data mirrored from the design handoff and generated into typed code. Screens are not redesigned yet: the HUD, result, help, Home, draft, loading and settings screens follow in the next steps. See [the UI kit](ui-kit.md#verdant-crown-foundation-0270).

## Language setting: English and Simplified Chinese (0.26.0)

Settings → Language (pause menu, also from Home) switches the whole game between English and 简体中文 live: menus, career, match HUD, shop, scoreboard, phone controls, chat and social, account screens, help and the names and descriptions of heroes, abilities and items. The choice is saved with the other client preferences; `OMOBA_LANGUAGE=zh-Hans` starts a QA or capture run in Chinese. Dictionaries live in `client/i18n/<locale>/` and are embedded at build time, so another language is added by adding a folder of JSON files. Developer tools, logs, server-authored free text and proper names stay English. See [localization](i18n.md).

## Controller support (0.25.0)

Play with a gamepad on macOS, Windows, Linux and iPhone: left stick moves (camera-relative, analog), right stick aims, R2/RT attacks, L1/R1/L2 (LB/RB/LT) hold-to-aim and release-to-cast, L2+R2 (LT+RT) is the ultimate, Triangle/Y + skill upgrades and R3 locks a target. Menus are navigated with the D-pad or left stick through the UI kit's new focus layer, Cross/A selects, Circle/B goes back (the same signal as Esc), Options/Start opens the pause menu, D-pad right the shop and left the reactions. An idle controller never takes over from mouse, keyboard or touch. Android reads no controllers. Physical controller and iPhone playtests are still pending. See [the controller guide](controller.md).

## Party lobby and co-op vs bots (0.24.0)

Home → **Party & friends** opens a party lobby: invite anyone connected to the same server (friends first when career storage is on), accept or decline invites from a Home toast, and see every member's avatar in a 3D line-up with leader and status plates. The leader's **PLAY VS BOTS** moves the whole party to hero select; the party is seated on one team and bots fill every other seat. Works on a database-free practice server and on the public lobby, where Play with bots allocates one arena for the party and Casual PvP and Ranked never split a party (LR-08). See [the party guide](party.md). On desktop the lobby's **SERVER** field (Change → `host:port` → Enter) connects to another host and is remembered; release packages for every platform come from `scripts/release.py` ([releasing](RELEASING.md)).

## Five classes and the Warden jungler (0.23.0-rc.6)

A fifth class, **Warden**, completes one class per draft duty: Warrior → Solo (top), Mage → Mid, Ranger → Carry (bot), Cleric → Support (bot), Warden → Jungle. Choosing a class in the draft proposes its role; a deliberately chosen role is kept. Warden is a melee jungler (210 HP, Feral Swipe / Barkskin / Hunter's Mark / Primal Maul) whose server-authoritative passive **Forest Tracker** deals +35% damage to jungle camps (+15% on bosses) and earns +40% gold and +25% XP per camp kill; boss rewards are unchanged because their value is the team buff. Practice bots now fill a team as that composition, skipping classes a human already took; the bot Warden clears its own half's camps and falls back to mid lane when every camp is down. The skill atlas gains a fifth Warden row in the same painted style. See [the balance note](balance-tuning.md#five-classes-and-the-warden-0230-rc6).

## Offline character practice (0.23.0-rc.5)

Home → Offline practice → choose a bundled avatar/class → Start practice. The 3D client runs a small in-process practice simulation through its normal snapshot/render/input pipeline, with no socket listener or external server. Level six unlocks every skill; four enemy practice heroes include a stationary melee target and moving animation examples, and defeated targets recover after three seconds. Mana regenerates for repeated tests. Basic attacks, class skills and dash/haste use shared definitions. Practice is not full bot matchmaking: ranked results, inventory purchases and progression rewards remain unavailable; lane waves and structures were added in 0.28.3. Leave practice in Game menu restores the saved online server; bundled characters work without Ekza login. Game menu has a fixed × header and fixed navigation footer, with independently scrollable content.

## Combat particles and healing butterflies (0.23.0-rc.3)

Magic orbs have orbiting sparks and fading tails; confirmed hits produce short radial bursts. Six symmetric forest flocks glow and flap, grant one injured living collector up to 5% maximum HP, then respawn after 30 seconds. Availability and HP belong to the server. A soft oval vignette frames the battlefield below the HUD in 3D and Sprite2d. See [rules, rendering limits and native capture instructions](forest-combat-vfx.md).

## Measured combat pacing and growth (0.23.0-rc.2)

Starting durability and Q intervals now leave an early response window; levels increase movement, basic attack speed/damage and skill power. All four skills share a short recovery interval, while basic attacks keep an independent clock. The client buffers the next skill and restores server cooldowns across reconnect. Default towers scale damage against heroes without shortening minion siege windows. The 48-encounter production-path benchmark measures a level-one median of 10.53s and level-ten mean of 4.24s; these are controlled stationary targets, not competitive win rates. See [formulas, research, limitations and reproduction](balance-tuning.md).

## Public multiplayer MVP (0.22.0-rc.2)

The public lobby offers Casual PvP (30-second bot fallback, never rated), Ranked (ten humans of one cohort, never bot-filled) and vs Bots; see [LR-08](#ranked-casual-pvp-and-vs-bots-lr-08-unreleased). Independent worker processes own immutable ten-player rosters, automatic teams, shared draft/countdown/loading and a durable start barrier. New humans cannot replace bots after a match starts; existing participants can reconnect. PostgreSQL saves history and progression for approved allocated bot games, with competitive rating reserved for eligible bot-free PvP. Signed gameplay packets, bounded admission and isolated durable outboxes protect the public match boundary. See [launch, recovery and capacity instructions](public-mvp.md).

## Shared running and VRM skeletal motion (0.21.0-rc.5)

All 15 shipped playable 3D avatars use the engine's shared Run motion during normal movement. Local intent and remote movement drive the same state machine; Walk stays reserved for future debuffs. Idle and combat actions remain separate. Existing 2D running sprites are unchanged.

Validated VRM0/VRM1 skinned humanoids receive runtime clips adapted to their bone map and rest pose, including models with no embedded clips. Original skin bytes remain unchanged. This is skeletal compatibility for a documented subset, not complete VRM materials/face/hair support. Approved Studio models gain runtime Run through normal verified loading; accepting externally published clipless models still requires a versioned profile rollout. See [architecture, import and limits](humanoid-motion.md).

Canonical version: `0.32.0`

## Team draft and shared loading

Normal Find match assigns a team and map side automatically. Each assigned team
can inspect accepted avatar/class choices, choose an intended Solo, Jungle, Mid,
Carry or Support role, and lock or unlock its choice. Duplicate and composition
warnings inform the team without banning duplicate classes or roles. The shared stage displays up to five teammates.

The shared three-second countdown begins after everyone locks, or after the
selection deadline expires and pending avatar checks finish. The
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

## Market builds

One build serves one market (`shared/src/market.rs`). The global build is the
full product. A free-market build (`OMOBA_MARKET=ru` or `cn`) ships the same
heroes, packaged avatars, weapons and gameplay without the Ekza ecosystem and
without purchases: no Ekza store, account link, wallet pairing, Studio tab or
section, Ekza weapon library or catalogue, market hint, community sculpture or
"Support OMOBA"; its client pins closed loopback Ekza endpoints and its server
denies every hosted avatar or weapon. It opens in the market's language and keeps
its settings in `omoba-bevy-<market>[-<contour>]`. The Home build line names the
market (`· ru`); the startup log prints the full feature set.

## Shared accounts and Hero Supporter cosmetics

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
shows billing source, paid period, renewal state, aura preferences and lifetime
credited months. The player-facing Hero status is cosmetic and follows the same
verified active period. A public supporter directory links opted-in public
profiles to their existing statistics-aware pages; expired support remains
acknowledged, refunds remove its credit, and private profiles/statistics remain
private. The lifetime count is independent of truncated payment history.

Hero aura VFX combine three intersecting, feathered ribbon orbits, bright moving
heads, a gentler body glow, eight rising sparks and a subtle ground ring. One
orbit runs in the opposite direction. Each 3D orbit has a shared static ribbon
with core and glow layers, built from crossed sheets with 32 segments per sheet.
The 23 rendered elements replace the previous 35; counted geometry is 1,632
triangles per aura versus 13,056 previously, an eightfold reduction in triangles.
This is a geometry count, not a measured eightfold GPU or frame-time improvement.
Meshes and materials are preallocated and shared: animation changes transforms,
without per-frame geometry uploads, new dynamic lights or dependencies. The
body glow never replaces character materials. Sprite mode retains 35 elements
with separate front/back depth sorting. Native preview and live actors share the
effect implementation and the existing authorization/visibility cleanup.

The owner's Lineage II reference inspired the luminous orbital treatment; this
is an original OMOBA effect, not an exact recreation or reused game asset. New
local 5v5 practice captures show the 23-element ribbon effect at normal zoom,
during movement and close up, followed by server-driven removal. The desktop
frame-time sample includes VSync and other scene work; it establishes no isolated
GPU, power or mobile result. Earlier 35-element captures remain historical.

Apple and Solana are separate provider adapters granting the same account right.
Payment functionality is unavailable until the operator explicitly configures a
verified provider. Solana is a quoted 30-day USDC prepayment, with finalized-chain
verification and a recoverable account order list. StoreKit supports native
purchase/restore and retains unfinished transactions until server confirmation.
The Apple adapter requires an authenticated verifier and App Store setup; local
fixtures are not proof of a real sandbox purchase or App Review approval.

### Monthly Hero tiers (LR-67)

Russian players donate on the RU portal through YooKassa; every donation is a
30-day grant worth one tier point per ruble, and the donations covering now add up
to a tier: Rose (100), Jade (250), Azure (500), Crimson (1000) and Prism I–III
(2500, 5000, 10000). Each tier unlocks its own aura style on top of Solar, Lunar
and Verdant, which any active grant unlocks; everyone in a match sees it. One
shared rule decides what a player may wear (Account API, career store and the
game server, which re-checks every refresh), and a style the tier no longer
covers steps down to the best one it does. The four solid tiers are new palettes
of the 23-element aura; the Prism tiers shimmer through the rainbow in sync for
all wearers, faster and brighter per tier, Prism III with a crown ring inside the
same budget. The Supporter panel shows the tier with its end date and the tier it
steps down to, lists the styles and, on RU desktop and Android builds, opens
«Поддержать на сайте» (`<portal>/supporter`); iOS builds never link to a web
payment. Protocol 12 carries the new styles; portal schema 9 stores amounts,
points and the durable YooKassa attempts and refunds, and the Account API
settles re-read payments and refunds exactly once. The Account API starts a
donation (a 54-FZ receipt through «Чеки от ЮKassa», the Idempotence-Key stored
before the payment is created), takes YooKassa's webhook without trusting its
body, and reconciles open payments, lost creates (23 hours) and refunds every
minute; new charges pause by themselves when the refund scan is stale. The RU
portal pages follow in LR-67c. See [Hero tiers](supporter.md#hero-tiers) and
[YooKassa donations](supporter.md#yookassa-donations).

Core career schema is version 3 and portal schema is versions 1–7 and 9. Run owner migrations and the
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

All 16 classes have four individually illustrated skills (64 total), generated
with Higgsfield GPT Image 2.5. The action silhouettes distinguish binds, shields,
projectiles, leaps, traps and finishers, with coordinated class palettes and
circular crops for the hotbar. Presentation atlases remain independent of skill
recipes and gameplay IDs. A stationary 1.5-second phone hold opens
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

## Bot route-planning budget (LR-10, unreleased)

- A bot plans a route at most once per `ROUTE_INTERVAL` (800 ms), whatever emptied its route: a new target, a retreat, a dash, a failed plan or a walked route. At its destination it plans nothing. Until the next plan it walks a clear straight line to the destination; the turret clip still stops it at an uncovered tower's range. Behind terrain or a structure, or after a failed plan, it holds still.
- Route search reuses per-thread buffers: the static grid plus a bounding-box overlay of live structure discs, with stamped costs and visits instead of fresh full-grid vectors. A warm plan allocates only the route it returns; obstacle queries reuse a per-thread index list.
- Measured in the difficulty simulator: 10–13 plans per bot-minute at every level, down from 199–254. The ladder and every gating check hold. The simulator now reports plans that found no route and gates each level at 80 plans per bot-minute.
- Projectiles step in creation order, so two shots blocked by one wall in the same tick no longer depend on `HashMap` order.
- Code: `common/src/bots.rs`, `shared/src/navigation/search.rs`, `shared/src/navigation/mod.rs`, `common/src/sim/projectiles.rs`. See [the session note](progress/2026-10-24-lr-10.md).

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
  The Warden class clears camps faster and earns more from them.
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
  lock the exact foe without attacking (2026-09-24). Drag to X to cancel. A
  phone tap chases like a desktop order unless the stick is steering; a hero in
  reach turns to face its target when it strikes.

- **Practice sandbox (2026-09-24):** in local bot practice and in the offline
  practice playground the pause menu's "Practice sandbox" page toggles god mode, spawns stationary target dummies
  in front of the hero, clears or restores the bot roster, and starts a 1v1
  against one mid-lane opponent at a chosen level (skills ranked for that
  level) and gold (spent on items at base). Practice bots shop at base, rank
  every unlocked skill and cast their whole hostile-target kit. See
  [bot practice](bot-practice-and-social.md).
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
- Ekza avatar store at runtime: through `ekza-bevy-sdk`, the client reads the public registry catalogue for templates approved for `omoba` / `desktop` / `humanoid-glb-v1`, shows the ones the paired wallet owns in a separate "Your Ekza avatars" group below the shipped "Default avatars", and installs a model on first use (verified size, SHA-256, GLB envelope and embedded `idle/walk/attack/cast/death` clips) under the private settings directory, mounted as the `ekza://` asset source. Other players' store avatars are fetched the same way on demand with the legacy model as a stand-in. The server admits a store slug only from a ticket consumed at the configured passport origin whose grant hashes to that slug. Wallet pairing starts from the "Connect Ekza wallet" button in the picker (browser approval, non-blocking); purchase happens on the Ekza web storefront. `scripts/ekza_publish.py` publishes an on-chain template into a registry catalogue with a baked Omoba rendition (VRM 0.x or VRM 1.0 source; VRM 1.0 is turned to face -Z), and `scripts/ekza_demo.py` runs the whole loop locally on devnet.
- VRM avatar support: VRM 0.x avatars (glTF 2.0 binary with extra `VRM`/spring-bone/blendshape extensions) load through the existing glTF model catalog by staging them as `.glb` (the VRM extensions are `extensionsUsed`-only, so Bevy's loader ignores them and keeps the mesh + skeleton). Ships one selectable CC0 humanoid, `Paco` (ToxSam 100Avatars R3); see `ATTRIBUTION.md` and `scripts/convert_vrm_to_glb.py`. The avatar carries no animation clips, so it renders as a static skinned mesh via `NormalizeModelScale` like other models.
- Core combat loop with projectiles, structures, minions, death, respawn, mana regeneration, and base-destruction win condition.
- Map layout with three lanes and simple jungle blocks.
- Player progression with level-based XP thresholds, HP/mana scaling on level-up, and tracked skill points.
- In-game local HUD display for level and XP progression.
- Persistent local client preferences (graphics, character, optional server address, stable client session id) with safe clamping on load; override directory with `OMOBA_CLIENT_CONFIG_DIR` for tests or portable installs.
- In-game match HUD (below minimap): level, XP, skill points, upgrade key label (`U`), local HP/mana, target hints, objective line, per-slot class ability lines (name, effect numbers, cooldown/lock state), and F1 help reminder; bottom-right skill bar shows `Q`–`R` keys with the selected class's ability names and ranks.
- F1 toggle help overlay with movement, camera, targeting, casting, objective, and pause guidance; does not reset simulation when toggled. The panel is shown only while the match is `Running` (toggle state is preserved when returning to a live match so lobby/victory screens are not covered).

## Every match ends (LR-11, unreleased)

- A match worker whose humans all left (Leave, crash or lost connection) keeps their heroes under bot control for the 180 s reclaim window plus 30 s. If nobody comes back, the round settles as **Interrupted** (unrated, no winner, no XP, stored once) and the worker retires and frees its port.
- Any running match ends at a hard cap, `OMOBA_MATCH_MAX_DURATION_SECS` (default 2700 s = 45 minutes), also as **Interrupted**. Connected players get the terminal receipt and the result screen shows «Match interrupted». A standalone server starts a fresh round instead.
- The server logs `MATCH_METRIC event=abandoned_reaped` and `event=duration_capped`.
- A lobby that binds beyond loopback refuses to start while `OMOBA_MATCH_PUBLIC_HOST` is unset or loopback, because players would be sent to their own machine (LR-01).
- Code: `server/src/match_lifecycle.rs`, `server/src/match_pool.rs` (`public_worker_host`). See [the session note](progress/2026-10-24-lr-11.md).

## Ranked, Casual PvP and vs Bots (LR-08, unreleased)

- Home and the party lobby offer three queues on the public lobby: **Casual PvP** (the main button), **Ranked** and **vs Bots**. A party leader launches the whole party into the chosen queue.
- **Ranked**: ten humans of one rated cohort, rated when eligible, never filled with bots. **Casual PvP**: every human in the queue plays together whatever their rating or newcomer cohort; bots fill the empty seats after 30 s; never rated, 50/25 XP even with ten humans. **vs Bots**: at once, as before.
- The queue, not the head count, decides rating: `rated == (queue == Ranked) && eligibility`. The lobby and the store share one cohort policy, so a mixed party ({1000 newcomer, 1400 veteran}) can no longer produce a rated roster the store rejects; in Ranked it is told to play Casual PvP together.
- Searching, draft and loading name the queue with a Rated/Unrated badge. A Ranked search shows how many players search the queue and, after 5 minutes, offers **Also search Casual PvP** while keeping its place in Ranked (owner default O-1, [policy](match-progression.md#low-population-ranked-policy)).
- Older protocol 11 clients keep working: the wire names stay (`quick` = Casual PvP, `humans_only` = Ranked) and every new field is additive.
- Copy in en, ru and zh-Hans (`client/i18n/*/queue.json`); the orphan `home.button.humans_only` and the Quick match keys are removed.
- Code: `server/src/match_service.rs` (`select`, `waiting_view`), `career-store/src/matchmaking.rs` (`fits_cohort`, `same_cohort`), `server/src/match_allocation.rs` (`Manifest::match_mode`), `shared/src/match_service.rs`, `client/src/match_service.rs`, `client/src/frontend/{home,lobby,searching,draft,loading}.rs`. See [the session note](progress/2026-10-24-lr-08.md).

## Honest state after leaving a running match (LR-13, unreleased)

- A player who left a running match (Leave, crash or lost connection) and presses **Play** sees «You are still in a match · it ends in N min at the latest» with **Resume match**, instead of a "Match found · connecting…" spinner that never connected. N is the duration cap, or the abandoned-match reaping while no human is connected.
- **Resume** returns to the same hero in that match. Waiting on the screen also works: once the match ends, the search queues a new match by itself. **Cancel** returns Home.
- The lobby never adds a second queue entry for a profile that still holds a running seat, and a played match that ended frees the seat without an error code.
- Copy in en, ru and zh-Hans (`client/i18n/*/rejoin.json`).
- Code: `server/src/match_service.rs` (`Lobby::view`), `server/src/match_pool.rs` (`Slot::ran`), worker `status.json` `ends_by_ms`, `shared/src/match_service.rs` (`InMatch`), `client/src/match_service.rs`, `client/src/frontend/searching.rs`. See [the session note](progress/2026-10-24-lr-13.md).

## Client update path (LR-14, unreleased)

- A game that the servers no longer admit shows **Update required** on Home. The download button takes Play's place and opens the market portal's download page for this platform (`<portal>/download?platform=…`; `https://omoba.ru/download` for RU builds). It also names the newest release from the catalog. iPhone builds say to update through the App Store or TestFlight and link nowhere. A build without a portal (global today) shows no button and says to download the new version from the official site.
- A game newer than the servers shows «servers are switching to your version» and keeps checking every 30 s, so players who update before the server switch connect by themselves.
- A newer release in the market's catalog (`GET /v1/releases`, read at boot and after a mismatch) shows a dismissible **Update available** banner on Home.
- Catalog entries carry `channel` (`direct`), an optional `min_release` and `update_url` (`release.py catalog --min-release`). The game reads only its own channel; a game below `min_release` shows Update required even while the servers accept it, and `update_url` is the button target.
- The compatibility report carries an additive update notice: the server's release as the minimum and its portal's download page. Clients open only links on their own portal. Production texts no longer suggest choosing another server.
- Copy in en, ru and zh-Hans (`client/i18n/*/update.json`); the release rule is in [RELEASING.md](RELEASING.md#updates-and-contract-changes-lr-14).
- Code: `shared/src/compatibility.rs` (`UpdateNotice`, `ReleaseVersion`, `mismatch_side`, `download_page`), `server/src/public_transport.rs`, `passport/src/release_catalog.rs`, `client/src/update.rs`, `client/src/frontend/home.rs`, `client/src/net/status_ui.rs`. See [the session note](progress/2026-10-24-lr-14.md).

## RU downloads from the RU host (LR-23, unreleased)

- RU players download the game (Windows, Linux, macOS, Android) from the RU host at `omoba.ru/downloads/v<version>/`, served by Caddy on the portal's site; the Pinata IPFS copy of the same bytes is the mirror.
- `make release-ru-publish` copies the packages with rsync (a published file is never replaced), reads each one back over HTTPS and checks SHA-256 and size, pins the IPFS mirror, and writes `releases.json`: host URL, `mirror_url`, status «Открытая бета», region `ru`, channel `direct`. `make release-ru-verify-downloads` downloads everything the RU portal lists and checks it byte for byte.
- Downloads are counted per platform, version and day: Caddy logs only download requests (address shortened to /24 or /48, no headers, 90 days) and `ru_host.py downloads` prints the totals from the host; no address leaves it.
- The download path and the status label live in the RU profile of `scripts/markets.py` (`downloads_path`, `release_label`, `region`); the global market keeps its IPFS links and the `playtest` label.
- Code: `scripts/release.py` (`publish_to_host`, `release_catalog`, `downloads_report`), `scripts/markets.py`, `ops/vps/ru/Caddyfile.ru`, `ops/vps/ru/ru_downloads.py`, `ops/vps/ru/ru_host.py` (`downloads`, the download folder and log). See [RU downloads](../ops/vps/ru/README.md#downloads) and [the session note](progress/2026-10-24-lr-23.md).

## RU web entry point, Russian README and server bundle (LR-22, unreleased)

- The RU landing is the RU portal's own guest home at `omoba.ru` (omoba-web, hosted in Russia, no foreign analytics): the number of hero classes counted from the game's catalog, the Verdant Arena, Ranked, Casual PvP, vs Bots and Offline practice named as the game names them, download buttons from the RU release catalog with SHA-256 and sizes in МБ, the status «Открытая бета» in the page language, the legal, source and community links and the age sign `12+`. Signed-in players go from `/` to `/me`; the workshop is labelled experimental; no RU page shows donate, wallet, crypto or Ekza content. The omoba.io landing is unchanged.
- RU player packages carry a Russian README.txt: what the game is, install, sign-in with VK ID («Профиль» → «Войти через VK ID»), how progress survives a reinstall (it follows the VK-linked account; recovery codes, LR-18), support and community channels once the owner sets them, the package's system requirements, the documents, the release's source and the age sign. It has no hosting notes. The global and dev READMEs are unchanged byte for byte.
- The RU Linux player package no longer carries `omoba-account-api`: the RU lobby and Account API are their own server bundle (`release.py build server --market ru`, also in `make release-ru`), the only file `ru_host.py ship` accepts.
- The RU portal serves a public `/delete` page (`omoba.ru/delete`, linked from every legal footer): how to have an account deleted without signing in, in ru, en and zh. Until deletion ships in the game (LR-42, LR-69) it is the operator's e-mail procedure from the privacy policy: device keys refused once the request is confirmed, erasure within 30 days, match results kept anonymised. The support address appears once the owner sets it (LR-21).
- Code: `scripts/release.py` (`ru_player_readme`, `ru_system_requirements`, `build_server_bundle`, `stage_server_bundle`), `scripts/markets.py` (`player_readme`, `server_bundle`), `ops/vps/ru/ru_host.py` (`RU_PACKAGE`); omoba-web `src/components/guest-home.tsx`, `src/components/deletion.tsx`, `src/lib/releases.mjs`, `src/lib/market-profile.mjs` (`marketLanding`, `delete`). See [markets](markets.md#legal-documents-and-links-lr-06), [the RU server bundle](../ops/vps/ru/README.md#server-bundle) and [the session note](progress/2026-10-24-lr-22.md).

## Admission and abuse limits (LR-16, unreleased)

- One source (an address, or a household or carrier NAT behind one) can no longer lock everyone else out of login or hold the arenas: transport probes and compatibility replies are budgeted per IP and per /24 before the global budget, which keeps a reserve for new sources; one IP holds at most 8 endpoints and 8 career clients.
- New device keys create profiles only within 10 per IP and 5000 in all per 24 hours, counted durably in PostgreSQL (career schema 4, a salted hash of the address, never the address); existing keys always sign in. The refusal («Too many new profiles were created from your network today…») is shown in the player's language, and the client stops retrying it.
- One IP holds at most one arena alone (every human from it): a second vs Bots, or a bot-filled Casual match, from that IP gets «A match from your network is already running…». Three vs Bots allocations left unplayed within 10 minutes give that IP a 2-minute vs Bots cooldown.
- The lobby deletes profiles that never played and that nothing else holds (friend, portal link, sign-in, Supporter grant) 30 days after their key was created, 200 per hour, logging `PROFILE_RETENTION pruned=N`.
- A sign-in during a reconnect storm or a database restart waits instead of failing: one Login per endpoint at a time, a full worker queue is silent, and an unconnected database is waited for up to 10 s. A harness test reconnects 300 clients after a lobby restart without an error.
- The RU host ships kernel per-source rate limits for UDP 4000 and the arena range (`ops/vps/ru/nftables-omoba.conf`, unit `omoba-limits-ru`), installed and checked by `ru_host.py`. Every limit is a setting (`OMOBA_LIMIT_*`, `OMOBA_PROFILE_RETENTION_DAYS`) documented in [ops/vps/ru/README.md](../ops/vps/ru/README.md#admission-limits-lr-16).
- Copy in en, ru and zh-Hans (`client/i18n/*/admission.json`, `queue.failed.arena_limit`, `queue.failed.practice_cooldown`).
- Code: `server/src/admission.rs`, `server/src/public_transport.rs`, `server/src/career_backend.rs`, `server/src/match_service.rs`, `career-store/migrations/postgres/004_admission_limits.sql`, `career-store/src/career_store.rs` (`authenticate_from`, `prune_unplayed_profiles`), `harness/tests/admission.rs`. See [the session note](progress/2026-10-24-lr-16.md).

## Legal acceptance, About and age sign in the game (LR-06, unreleased)

- An RU build opens with a card before anything else: «Перед игрой», the age sign `12+`, links to the User Agreement and the Privacy Policy on `omoba.ru` (opened in the browser), the document version (`0-draft`), a language switch and «Я принимаю…». **Continue** stays disabled until the switch is on.
- Until the player accepts, the game opens no lobby connection, does not read the release catalog and offers no VK sign-in, device link or website pairing. The acceptance is kept in the market's settings folder (`legal_acceptance.json`: region, document version, time); a new document version asks again. It is not sent to a server yet.
- Settings → «Об игре» shows the build label, `12+`, both documents and the source code of this release (`github.com/o-moba/omoba-bevy/tree/v<version>-ru`). Support e-mail, VK community and Telegram appear there once the owner provides them. The boot splash shows `12+` too.
- RU package READMEs end with the age sign and the same links. `release.py catalog --market ru` refuses until the public tag `v<version>-ru` exists.
- The global build is unchanged: its market has no legal table, so no card, no About page and no gate.
- Copy in en, ru and zh-Hans (`client/i18n/*/legal.json`).
- Code: `shared/src/market.rs` (`Market::legal`, `Legal`), `scripts/markets.py` (`Legal`), `client/src/legal.rs`, `client/src/pause_menu.rs` (About tab), `scripts/release.py` (`legal_readme`, `require_public_source_tag`). See [markets](markets.md#legal-documents-and-links-lr-06) and [the session note](progress/2026-10-24-lr-06-client.md).

## RU host monitoring and alerts (LR-17 ops, unreleased)

- The owner hears about an RU outage on Telegram within minutes, without logging in to the host. Without a bot, every alert still goes to the journal.
- On the host, every RU unit and every PostgreSQL cluster unit names `OnFailure=omoba-alert-ru@%n.service`, so any failure alerts at once, also each crash before systemd restarts a service. A check every 5 minutes reports an enabled service or Caddy not running, a PostgreSQL cluster down (its `postgresql@<version>-main.service`, not the always-active `postgresql.service` wrapper, or no connection on port 5432), a stopped backup timer, free disk below 15% and a match worker whose heartbeat stopped. Reminders come hourly (daily for a timer job), and a "resolved" message follows once the problem is gone.
- Outside the host, an external probe runs every minute against the lobby's UDP compatibility check, the Account API `/health/ready` and the portal, and alerts after 2 failed rounds in a row. `release.py preflight` runs the same UDP check and, with `--contract`, compares the server with a release.
- A journald drop-in keeps the RU host's journal on disk, at most 1 GiB and 30 days.
- The host's one Telegram bot lives in `alerts.env`; the backup alerts (LR-03) use it too. The public names come from `scripts/markets.py`.
- Not yet: the lobby's queue, capacity and allocation metrics with their alerts, and bounded match archives (LR-17 server part).
- Code: `ops/vps/ru/ru_monitor.py`, `ops/vps/ru/omoba-alert-ru@.service`, `ops/vps/ru/omoba-monitor-ru.{service,timer}`, `ops/vps/ru/postgresql-alert-ru.conf`, `ops/vps/ru/alerts.env.example`, `ops/vps/ru/journald-omoba-ru.conf`, `ops/vps/ru/ru_host.py` (install and `status`), `scripts/market_probe.py`, `ops/probe/`, `scripts/release.py` (`preflight`). See [Monitoring and alerts](../ops/vps/ru/README.md#monitoring-and-alerts), [the external probe](../ops/probe/README.md) and [the session note](progress/2026-10-24-lr-17-ops.md).

## Protect progress (LR-18, unreleased)

- After the first completed match, and again after the first ranked match, Home shows a dismissible card: «Защитите прогресс: VK ID или код восстановления.» (Protect / Not now). A "Not now" hides it for three completed matches, or until the first ranked match if that comes sooner, so players who never play ranked are asked again too; it stops once the profile is protected or after three declines. The record (`protect_prompt.json`: declines, matches at the last decline) lives in the region's settings folder.
- Profile shows the account's protection (VK ID linked, unused recovery codes, or not protected yet) and a «Защитить прогресс» button. The Protect modal links VK ID where the region signs in with it and creates or replaces recovery codes in the game; replacing a set that still has unused codes needs a second press. The eight codes are shown once, grouped for reading, never stored or logged, and gone when the modal closes.
- The codes come from a native Account API route signed by the device key (`POST /v1/auth/devices/recovery-codes`, `omoba.recovery.native.v1`, single-use nonce), on the RU and dev ingress allowlists; no portal session and no schema change.
- «Привязать или восстановить аккаунт» offers «Ввести код восстановления» next to «Войти через VK ID» and the device request, and a request started from it asks for the code first.
- Progress follows the VK-linked account; RU package READMEs and the RU portal (guest home, sign-in, `/connect`, Devices; o-moba/omoba-web#20) say so. The VK-verified support procedure is [account recovery](account-recovery.md); Android keeps `allowBackup=false` ([ADR-026](architecture/decisions.md#adr-026-android-keeps-allowbackupfalse-progress-survives-a-reinstall-through-vk-id-or-recovery-codes)).
- Copy in en, ru and zh-Hans (`client/i18n/*/protect.json`).
- Code: `shared/src/device_account.rs` (`NativeRecoveryRequest`, `AccountProtection`), `account-api/src/devices.rs` (`native_recovery`), `passport/src/device_account.rs`, `client/src/career_protection.rs`, `client/src/career_identity.rs` (`sign_recovery_request`), `client/src/career_devices.rs`, `client/src/frontend/home.rs`, `scripts/release.py` (`progress_readme`). See [the session note](progress/2026-10-24-lr-18.md).

## Moderation minimum (LR-15, unreleased)

- Operators put sanctions on a profile with `omoba-account-api sanction add|list|lift` and look players up with `omoba-account-api player <Name#1234 | profile-id>`: `chat_mute`, `matchmaking_ban`, `ranked_ban`, `account_ban` and `name_reset`, each for a duration (`30m`, `7d`, …) or until lifted, with a reason and the operator's name. Procedure: [RUNBOOK.md › Moderation](../RUNBOOK.md#moderation-lr-15).
- Game servers read a profile's sanctions at sign-in and every 10 seconds, so applying or lifting one takes effect without a restart. An account ban refuses the game sign-in (and ends a signed-in session), every website session and new website sign-in; a matchmaking ban refuses every search, a ranked ban only Ranked; account and matchmaking bans hide the player from party presence and take them out of their party; a chat mute refuses their chat and drops what they sent in the last 30 seconds, while reactions stay.
- `omoba-account-api rename <player>` resets a name to `Player#NNNN` and locks renames (game and website) until the reset ends; party presence shows the new name within seconds.
- Chat kill switch: `OMOBA_CHAT=off` on a server (the RU lobby unit sets it, owner default O-9; match workers inherit it), or `omoba-account-api chat off` for every server of the database within seconds. Reactions are never switched off.
- The player sees each refusal in their language with its end in UTC: the suspended sign-in and the search refusals on the search screen and the career panel, «muted until …» or «chat is turned off» in the match chat, which then offers no keyboard. A suspended client signs in again every minute, so a lifted ban needs no restart.
- Every moderation action writes an audit row in the same transaction (`career_moderation_events`, career schema 5); rows are append-only and cannot be deleted before they are a year old. A sanctioned profile is never pruned as unplayed, so a ban on a throwaway key holds.
- Copy in en, ru and zh-Hans (`client/i18n/*/moderation.json`).
- Code: `shared/src/moderation.rs`, `career-store/migrations/postgres/005_sanctions.sql`, `career-store/src/moderation.rs`, `account-api/src/moderation.rs`, `server/src/career_backend.rs`, `server/src/social.rs`, `server/src/party.rs`, `server/src/match_service.rs`, `client/src/moderation.rs`, `client/src/social.rs`. See [the session note](progress/2026-10-24-lr-15.md).

## Arena capacity: full pool and full server (LR-09, unreleased)

- When every arena is busy, a search whose match is ready waits in line and says so: «All arenas are busy · you are number 2 in line · about 3 min». The place follows the order the lobby seats rosters (vs Bots, then Ranked, then Casual PvP); the estimate appears once arenas have freed in the last 15 minutes. A search that is not ready yet (Ranked 3/10) keeps showing its own progress.
- The searching screen then offers **Play offline practice while you wait** in one tap: it leaves the queue and opens the practice picker with the chosen hero. The same offer appears when the server is too full to sign in.
- A full server answers instead of staying silent: at 512 endpoints a new player's probe gets `server_full`, and at 512 career clients its sign-in gets the `server_full` code. The game shows «The server is full. Retrying…» and keeps trying.
- The lobby reports a busy pool: `<match root>/capacity.json` every 5 s and `CAPACITY_METRIC` journal lines; on the RU host `ru_monitor.py` alerts the owner after a 2-minute wait (`OMOBA_ALERT_CAPACITY_WAIT_SECONDS`).
- The load probe is ready for the RU host: mixed queues, the lobby unit's cgroup limits, a maximum time to Running, and per-arena-type snapshot gaps, bytes per player, CPU and RSS (`CAPACITY_RESULT`). The measurement itself and the final capacity need the host ([Arena capacity](../ops/vps/ru/README.md#arena-capacity-lr-09)).
- Copy in en, ru and zh-Hans (`client/i18n/*/capacity.json`, `admission.error.server_full`).
- Code: `server/src/capacity.rs`, `server/src/match_service.rs` (`Lobby::refresh_line`), `server/src/match_pool.rs`, `server/src/public_transport.rs`, `server/src/career_backend.rs`, `shared/src/match_service.rs` (`CapacityLine`), `shared/src/public_transport.rs` (`ServerFull`), `client/src/match_service.rs`, `client/src/frontend/searching.rs`, `client/src/net/{public_transport,transport,session}.rs`, `client/src/career_identity.rs`, `ops/vps/ru/ru_monitor.py`, `scripts/test_public_mvp.mjs`. See [the session note](progress/2026-10-24-lr-09.md).

## Multiplayer Session Reliability

- **TASK-14 (client)**: Explicit session states, non-blocking wait when the server is down, bounded `WaitingForServer` timeout, stale snapshot detection while connected, UDP transport error thresholds, snapshot-channel disconnect detection when the UDP thread ends, full teardown (replicated entities + team overlay) on disconnect, manual reconnect via **Retry** (no silent rejoin into a match), pause menu auto-closes on **Disconnected**, minimap hidden unless **Connected**.
- Named timing constants and failure-detection summary: `client/src/session_config.rs`.
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
out removes the local credential and invalidates pending background work, then
asks the registry to end that link (`DELETE /v1/account/session` through the
SDK's `AccountSession::disconnect`). The request runs in the background and is
best effort: signing out never waits for it, and offline the link stays valid
until it expires or Studio Account revokes it. A new pairing that replaces a
saved link (for example one a startup outage left unrestored) ends the old link
the same way once the new one is saved; the SDK refuses, without sending it, a
saved token for another server or game. Public approved community models
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

### Visible build identification

Home shows a compact, noninteractive build label at the lower-left on desktop/mobile: `v0.28.1 (14) · 52bf02d · Debug`, for example. Version and Debug/Release come from the executable; Xcode build number and source commit come from its bundled `assets/legal/XCODE-BUILD.json`. `*` marks a build with modified tracked sources. A desktop/unpackaged build or unavailable/mismatched receipt shows version and mode only. The label is available without connecting to a server. Existing installed builds need an app update to gain the label.

## Hosted beta and server selection (0.34)

Fresh clients default to `77.246.105.57:4000`. Home's server-link button opens the address editor on desktop and mobile. **OMOBA Beta** and **Localhost** prefill an editable host:port; **Connect** validates, reconnects and saves. Existing manual selections retain priority, as do explicit runtime/build overrides. Desktop party lobby also provides prefill buttons. Match-worker handoff remains temporary and never replaces the saved lobby address.

The beta lobby on `vds-eternal` supervises at most two independent matches on UDP41000–41001, with PostgreSQL-backed game profiles and durable match receipts. Studio/Registry and Ekza Space remain separate services. See [VPS operations](../ops/vps/README.md); physical mobile performance and capacity beyond two rooms are not certified by this rollout.

### Avatar preview framing (0.34.2)

The Home, collection and selection preview camera fits the actual glTF vertex silhouette after model normalization. In 0.35.0, a cached rotational envelope keeps the ground line at the Home platform while accounting for model width/depth, height and vertical origin without changing gameplay scale or colliders. Wide upper wings do not introduce an imaginary wide foot that would unnecessarily shrink the avatar. The opt-in `OMOBA_HOME_ONLY_QA=1` screenshot check waits for the avatar and captures only Home. Bind-pose geometry plus padding cover normal idle motion; extreme animation excursions and separately attached props are not dynamically measured.


### Playtest quality — 0.42.0

Practice bot matches expose zero cooldowns, disposable moving/attacking targets and a scrollable tool panel. Combat repairs cover Adventurer melee/point-blank targeting, cooldown reconciliation, Wildspark weapon modes and accepted-attack audio. Public enemy death portraits complement live target portraits without disclosing hidden coordinates. The compact Verdant arena shares navigation, collision, brush and objective placement across offline/server simulation; ten camps include timed fire/ice rewards. An original animated dragon, ranged handhelds, independent nexus rings and inner guard silhouettes improve 3D presentation.

Release protocol/catalogue/geometry revisions require matching 0.42-compatible peers. Existing production and TestFlight 0.41.0 are unchanged by this source iteration. See `docs/progress/2026-10-05-playtest-quality.md` for verification and limitations.

Allied portraits also show remaining respawn seconds. Nexus motion rotates the original central GLB armillary rings; the extra upper rings are removed.

### Engine baseline — 0.43.0

The game and Ekza SDK use Bevy 0.19.1 with Rust 1.95.0. Existing avatar/weapon account flows, humanoid animation and 0.42 gameplay remain supported. GLB scene labels are retained while engine scene types move to world serialization. The upgrade does not claim improved phone FPS without physical-device measurement.


## Community class workshop prototype

The companion portal adds `/workshop` alongside the public `/creators` gallery.
It composes the game’s 12 modular cores and 48 reusable skills, derives the current
core and skill visual requirements, saves private named recipes under the existing
OMOBA profile, and exports a safe native Combat Test preset. The catalogue still
browses all 17 classes and 68 abilities, including legacy nonmodular kits.

Shared Rust validation drives persistence and local import; gameplay dependencies
are exported from the same resolver. The Account API stores private versioned
builds in PostgreSQL and supports owner-bound Ekza library consent without
exposing credentials to the browser. This is a prototype for local gameplay and
community contribution, with no public/ranked custom-class admission or automatic
skin installation. See [community workshop](community-workshop.md) and
[native build import](class-workshop-game.md).

### Hero aura world verification (2026-10-15)

The current ribbon revision is captured on the actual Agnes roster character
in a server-backed local 5v5 practice match, in English at 1280×720. Normal,
moving and close views plus 24 sampled motion frames show 23 effect elements;
revocation clears the replicated aura and removes them all. The short 3.24-second
MP4 contains those 24 sampled frames, not a 60-fps recording. A separate warmed
180-frame sample per state at VSync 60 records similar whole-frame pacing with
the aura on and off; ongoing bots, jungle and HUD work prevent isolating its GPU
cost. The earlier 35-element body-glow captures remain historical. See
[Supporter operations](supporter.md#live-hero-world-verification) for results,
limits and the reproducible QA scope. No real payment or deployment is implied.
