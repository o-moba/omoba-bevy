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

Validation artifacts and exact source identities are recorded under
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
The shared 3D renderer now uses a broader HDR body halo without adding elements,
lights or changes to the character materials.

The final English 1280×720 run captured Agnes in a local 5v5 practice match with
server bots, at normal zoom, moving, and at closer zoom. A synthetic paid period
in the isolated PostgreSQL fixture admitted Solar through the ordinary signed
identity and snapshot pipeline. All 35 elements followed the actual player.
After revoking that period in the fixture database, the next server update
cleared the aura and removed every effect element. The regular selection marker
remained. A normal right-click moved the player clear of architecture before
the final capture. See the separate world verification artifacts for source
identity, native pixels and independent review; earlier preview-only evidence
remains historical and is not relabelled as a world test.
