# 2026-10-15: monthly Hero Supporter

This feature extends existing verified Supporter grants with a clearer cosmetic
Hero benefit and optional public recognition. It does not replace the payment
pipeline or create client-controlled paid status. The game and player portal
have separate feature branches based on dev, independent of community sculptures.

The Account API counts unique nonrevoked monthly periods across the entire
ledger, including expired history and future prepayments. Public recognition is
opt-in and requires the existing public profile setting. The directory exposes
only nickname, profile identity, credited months and current Hero status;
linked statistics retain their own privacy gate. A new migration defaults
acknowledgement off without changing existing entitlements.

The native aura reuses the existing Solar/Lunar/Verdant rendering and server
replication paths. Verification uses one English 1280×720 profile and distinguishes
free isolated preview from paid world authorization.

The following validation records the initial feature and body-glow renderer,
not the later ribbon revision described below. Artifacts and source identities are under
`.agent/tasks/hero-supporter/`. Native build, formatting, strict Clippy for the
client/API and the client without default features passed. The complete client
suite passed 1,348 tests (one existing ignored test); the new real-PostgreSQL
recognition suite passed all three tests. The existing Account API, career-store
and server PostgreSQL gate also passed, including restricted runtime-role and
payment-authority regressions. Existing workshop tests and new Hero tests use
their own guarded databases in one isolated local PostgreSQL cluster.

Actual native captures contain three palettes and 24 distinct motion frames at
English 1280×720. They prove the free preview using the shared world renderer;
they do not claim a live paid multiplayer session. The connected Web/API/DB
fixture uses explicitly seeded periods and the normal signed pairing protocol.
The planned release is the next feature release; this PR adds an Unreleased
entry and does not tag, publish or redistribute 0.44.0.

Operator limits remain: hosted dev accounts are disabled; Solana requires
configured RPC/network/mint/treasury/price and a real wallet check, while Apple
requires its actual verification service, monthly product and sandbox testing.
No hosted migration, provider setup, credentials, payments, release or deployment
is performed by this feature task.

## Follow-up: actual in-game Hero appearance

The owner requested direct live-world proof. The first connected practice match
showed the aura followed the character and disappeared after server revocation,
but its body glow was faint at normal camera distance on the daylight map.
That revision broadened the shared 3D HDR body halo without adding elements,
lights or changes to the character materials.

The earlier English 1280×720 run captured Agnes in a local 5v5 practice match with
server bots, at normal zoom, moving, and at closer zoom. A synthetic paid period
in the isolated PostgreSQL fixture admitted Solar through the ordinary signed
identity and snapshot pipeline. All 35 elements followed the actual player.
After revoking that period in the fixture database, the next server update
cleared the aura and removed every effect element. The regular selection marker
remained. A normal right-click moved the player clear of architecture before
the final capture. See the separate world verification artifacts for source
identity, native pixels and independent review; earlier preview-only evidence
remains historical and is not relabelled as a world test.


## Follow-up: continuous Hero orbit ribbons

The owner supplied a Lineage II image as inspiration for luminous orbital trails.
The new OMOBA effect is original rather than an exact recreation: three
intersecting orbits use static, feathered ribbons with bright heads, one
counter-rotating orbit and a gentler body glow. Each orbit has core and glow
layers of crossed sheets with 32 segments. Shared preallocated meshes/materials
animate through transforms, without per-frame geometry uploads, extra lights or
new libraries.

The 3D effect now contains 23 elements rather than 35. The geometry comparison
counts 13,056 triangles for the previous effect and 1,632 for the new one, an
eightfold reduction. This count is not a GPU/frame-time result. The 2D fallback remains
at 35 separately depth-sorted elements. Server-authoritative eligibility and
lifecycle cleanup are unchanged, as are gameplay, payment and privacy rules.

Earlier 35-element screenshots and their passing checks remain historical.
The new English 1280×720 local 5v5 practice run passed on the actual Agnes roster
player: normal, moving and close captures plus 24 sampled motion frames contain
23 aura elements; server revocation clears the replicated style and removes all
owned effect entities. The coordinator inspected the pixels. Another unit enters
the inactive view, so that image is not a pixel-identical scene comparison.
Evidence is under `.agent/tasks/hero-orbit-art/raw/world/`; the 3.24-second MP4
presents 24 sampled frames, not a 60-fps recording.

The Apple M4 desktop run at VSync 60 warmed each state for 60 frames, then sampled
180 frame deltas over approximately three seconds at the same position and zoom
0.55, with zero position drift. Active p50/p95 were 16.642792/18.045000 ms; off
p50/p95 were 16.618417/18.048333 ms. Bot, jungle and HUD work continued and differed
between the windows. This is a whole-frame pacing check, not an isolated GPU
cost measurement, power test or mobile-performance certification.

All 14 focused supporter tests pass, including stable entities/assets for ten
auras over 600 animation frames. Full `make check` passes: 2,090 Rust tests
(including 1,351 client tests), 382 Python tooling tests and 55 iOS packaging
fixture tests, plus formatting and strict workspace/no-QA Clippy. Existing
ignored integration/device tests remain excluded. A fresh reviewer separately
reran the 14 supporter tests and QA timing-percentile test.

The preview camera now frames the complete orbit crown and ground ring. Its
final native rebuild and three palette recaptures pass; this preview-only change
does not alter the world renderer or the world evidence above. Final client
Clippy is recorded separately. No real payment, provider activation or deployment
is part of this follow-up.
