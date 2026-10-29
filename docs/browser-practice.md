# Browser practice

The browser package is real OMOBA combat, hosted locally in WASM. It reuses the
authoritative `common::offline::PracticeSession`, shared map/collision/hero kits
and the shipped Verdant 2D terrain/character art. It is an additive browser
renderer rather than the native client's UDP/account/filesystem stack.

It is explicitly **local practice with bots**. It does not join matchmaking,
settle a career match, earn rating/XP, use paid cosmetics, or persist a match.
The seven selectable kits are Dawnweaver, Wildspark, Cinderforge, Edgeweaver,
Stormfist, Veilstalker and Emberveil. Both teams' bots use Regular difficulty.

## Reproduce the package

Requirements: the repository's Rust toolchain/lockfile, installed
`wasm32-unknown-unknown` target and Trunk 0.21.x. Trunk supplies the matching
wasm-bindgen CLI version from Cargo.lock. No new third-party production library
is introduced: this package reuses Bevy 0.19.1, common and shared. Native shared
has no Bevy dependency; only its wasm target uses the engine's monotonic clock.

```sh
export CARGO_TARGET_DIR=/Volumes/Orico/omoba-build-cache/target-b
scripts/build-web.sh
python3 -m http.server 3186 --bind 127.0.0.1 --directory builds/browser-practice
```

The default target directory resolves to canonical cache B in the primary
checkout. Never build another worktree-local cache or run cargo clean.
`builds/browser-practice/BUILD_INFO.json` records mode, version, source commit,
dirty status, sizes and SHA-256 of every distributed file. The build script
copies only two map atlases and seven character sheets plus a license notice.
Do not confuse a built artifact with deployment, live availability or a complete
multiplayer browser release.

## Portal boundary

Copy the complete output directory into the portal's ignored `public/game/`.
Serve `index.html`, JS, WASM and `assets/` unchanged. Trunk emits relative URLs,
so loading `/game/index.html` does not request files at a foreign origin or the
site's `/assets` root. Correct WASM MIME type is `application/wasm`.

The portal should preflight BUILD_INFO, check `entry=index.html` and
`mode=local-practice`, then create its same-origin iframe only after the player
chooses to load the game. The loader posts `{type:'omoba-practice-ready'}` or
`{type:'omoba-practice-error'}` to `location.origin`. The parent must check both
event origin and iframe source. Ready means initialization succeeded, not that
a ranked/network match was joined. Frame embedding should be allowed only for
this same-origin game path; retain unrelated application framing restrictions.

Existing portal VK login remains independent. Neither the iframe URL nor its
message protocol accepts an account token, device key or profile ID. The bundle
contains no service URLs, payment integration or private credentials.

## Desktop controls

Click inside the game to focus its canvas. Arrow keys move; right-click orders
a route using the same collision/pathfinding map as the native game.

| Input | Action |
| --- | --- |
| Space or hold left click | Basic attack nearest visible hostile target |
| Q / W / E / R | Actual class skills aimed toward the cursor |
| F | Dash toward the cursor |
| B | Channel recall to base |
| Escape | Pause/resume the local simulation |
| N | Start a new practice with the current class |
| 1-7 or class buttons | Select a class and start a new practice |

The same controls and local-practice boundary appear inside the game. Health,
mana, gold, levels, score and skill cooldowns come from the authority's state.
Moving/attacking/casting can cancel recall according to normal game rules.
Closing or reloading the page discards the local match. Touch input and a full
mobile layout have not been accepted for this artifact.

## Browser smoke diagnostics

Trunk's initialization event exposes its WASM exports as
`window.__omobaPractice`. Read-only `omoba_practice_frames`,
`omoba_practice_actions`, `omoba_practice_x`, `omoba_practice_z` and
`omoba_practice_round` enable a real browser test to prove frames advance,
movement changes simulation position, inputs are sent and restart increments
the round. `omoba_practice_accepted_actions` and `omoba_practice_mana` verify
that the authority accepts actual skills and spends its real resource pool.
These expose no identity or durable account data. Browser screenshots
must show the actual compiled game, not a canvas replacement or static mock.
