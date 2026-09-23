# Forest and combat VFX

The client presents authoritative projectiles as class-specific weapons. Mage attacks use a luminous orb with rotating satellite sparks and a fading particle tail. Warrior attacks use an icy steel blade rather than the old segmented yellow crescent. Confirmed combat events trigger short expanding rings/slashes and radial sparks. Despawning a projectile does not imply an impact; damage, timing, collision and range remain unchanged.

## Healing butterflies

Six symmetric forest clearings host flocks of three glowing butterflies. Walk into the flock while injured to collect it. The server grants `min(missing HP, maximum HP × 0.05)` once, to one living joined player within 1.5 world units. Simultaneous contenders resolve by ascending player ID. Full-health, dead, respawning and out-of-range players do not consume a flock. Both teams can collect any flock. It returns after 30 seconds of simulation time; a new round restores all flocks.

The shared layout is checked for walkability and routes from both bases. Wings flap and drift within the collection radius; a soft glow works without bloom. The authoritative snapshot determines availability. A monotonic collection receipt drives the green collection burst and local HP message. Reconnect and round identity changes seed the receipt cursor without replaying history. The client never awards HP.

## Atmospheric framing and limits

A transparent-center oval mist softly darkens battlefield edges. It scales with the viewport, sits below HUD panels, and cannot intercept input. This is cosmetic atmosphere, not network fog of war or an enemy-visibility system. It works in 3D and Sprite2d.

Projectile presentation is capped at 384 roots in either mode, including the Sprite2d proxies. Hidden roots suppress trails and proxies. Rendering uses a fixed 256-particle pool, with at most 128 occupied by flight particles. There are 36 butterfly wings, 18 glow billboards and one 256×256 vignette texture. Impact and trail queues are drained even when the pool is saturated, avoiding delayed replay. Invisible/offscreen projectile roots do not emit flight particles; accepted combat feedback retains its existing visibility and history gates. Particle assets are reused across both render modes.

## Reproduction

Build the current client and server, then run `scripts/capture_combat.py` for real projectile/impact frames, or `scripts/capture_forest_pickups.py` for an available/flapping/collected/respawned sequence. Both accept explicit binary and asset paths. The pickup route uses one ordinary enemy Q to injure the hero, normal movement, and independent server snapshots to prove the actual heal and 30-second respawn. Use `--mode sprite2d` for fallback rendering and `--touch-controls --width 1280 --height 720` for a mobile-sized preview. These desktop native captures do not establish physical iPad performance.
