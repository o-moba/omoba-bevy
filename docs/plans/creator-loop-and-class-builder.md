# Creator asset loop first, class workshop second

Decision: 2026-10-05, following the owner's priority. This is an ordered product
backlog, not an implemented editor or a new production acceptance report.

## What already exists

- `shared::loadout::BuildRecipe` contains core, passive and four skill IDs.
  The shared resolver accepts four distinct compatible skills, including four
  ultimates, and rejects invalid dependencies/revisions. Equipped metadata
  already drives input, targeting, icons, costs, cooldowns and presentation.
- Combat Test supports explicit recipes and local named JSON preset save/load.
  It is opt-in; public Join/draft do not accept player-authored recipes. There is
  no public skill-picker, account recipe library or approved custom-class feed.
- Registry/Studio already provide avatar and weapon revisions, technical review,
  per-game submissions, separate game approval, exact rendition selection and
  revocation. The SDK verifies files; OMOBA independently admits them on its
  server and resolves peer equipment through the same approved IDs.
- The October 1–2 pilot records a hosted avatar/weapon route and two independent
  desktop clients. It does not prove fresh public signup, every upload operation
  through the browser, or current physical-phone acceptance. The pilot used
  technical accounts and some API-assisted preparation. Revalidate rather than
  reconstruct this existing pipeline.

Sources: [recipe contract](../equipped-skills.md),
[asset lifecycle](../progress/2026-10-01-asset-lifecycle.md),
[production avatar runbook](../ekza-production-avatars.md).
Ecosystem source owners and historical acceptance are in umbrella
`ARCHITECTURE.md`, `PLAN-WEB2-FIRST.md`, and
`doc/docs/developers/{typed-assets,demo-readiness}.md`.

## P0 — artist to player: next active product milestone

The release execution checklist for A1–A7 is owned by Registry at
`ekza-registry/docs/creator-loop-release-plan.md` (2026-10-05, R0–R5).
It specifies migration compatibility, browser grip preparation and a mandatory
physical-iPhone/desktop acceptance run with the app installed before new asset
IDs are created. These gates remain pending; historical pilot evidence is not
fresh mobile certification.

The acceptance unit is one fresh avatar **and** one fresh cosmetic weapon,
from artist upload to use by a player and observation by another player.
No hand-edited manifest, direct SQL approval, publisher credentials in the game,
or developer-only import may substitute for a product step.

- [ ] **A1 — Fresh creator walkthrough.** Use an ordinary account with usable
  signup/confirmation, correct original author/licence/source credits, and the
  regular Studio upload UI. Audit existing screens first; fix only actual
  blockers. Record fresh asset/revision IDs so a bundled/cached model cannot
  accidentally pass. Owner: `ekza-registry/web` and `backend`.
- [ ] **A2 — Remove irrelevant conversion coupling.** Current avatar processing
  in `backend/app/services/studio_worker.py` requires a validated USDZ before
  the revision reaches review, even for an OMOBA-only upload; weapons bypass
  that converter. Separate source technical publication from profile-specific
  output readiness. A validated OMOBA rendition should not wait for Mirror's
  USDZ. Preserve curator review, per-profile validation and exact-byte approval;
  Mirror must continue to receive only ready validated USDZ. Design an additive
  migration and negative tests before changing this state machine. Owner:
  Registry migrations/worker/API and corresponding Studio status views.
- [ ] **A3 — Visible progress and recovery.** Verify that upload, processing,
  technical review, preparation for OMOBA, game review and playable status are
  understandable; expose the next available action and useful failure/retry
  reason. Test processing interruption/retry and worker-offline queued state.
  Existing polling/lease machinery should be reused. The current Mac worker is
  acceptable for the supervised pilot; it is not an always-on processing SLA.
- [ ] **A3w — Artist-friendly weapon preparation.** Studio currently requires
  `asset.extras.ekza_handheld_v1` inside the uploaded GLB, but its generic model
  preview cannot author a grip or show an avatar hand. Add local browser
  preparation with hand selection, position/rotation/scale and preview using
  OMOBA's palm-frame rules; a Blender export template is optional author help,
  not a required release workaround. The resulting transform must
  be embedded in a new immutable file before hashing/approval, never patched
  silently after review. `weapon-build` validates/preserves bytes; it is not a
  grip generator. Owner: Registry Studio UI/profile tools with OMOBA's consumer
  validator. Acceptance: an artist can prepare, inspect and correct placement
  without asking an engineer to hand-edit GLB JSON.
- [ ] **A4 — Separate reviewer decision.** Game owner opens the exact avatar and
  weapon rendition, checks preview/animation or hand grip, then approves or
  rejects with a reason. Creator sees the result and can correct/resubmit.
  Upload/publication alone must never grant OMOBA eligibility. Owner: Registry.
- [ ] **A5 — Player round trip on the current game.** Fresh client cache discovers
  the approved assets through the SDK, shows download/ready/retry, previews and
  equips them, enters an ordinary server match; another independently cached
  client sees the same avatar/weapon and animation. Repeat on iPhone first,
  Android next. Verify restart/cached relaunch, interrupted download, attribution,
  proportions, hand alignment, loading time and memory. SDK/game owners fix
  boundary failures in their own repository. No game rebuild for each model.
- [ ] **A6 — Version and rejection proof.** New publication retains the old
  game-selected rendition until the new one is approved. A rejected or revoked
  selection is unavailable for new admission; an outage preserves the last
  successful approved catalogue instead of acting as a mass revocation. Record
  existing-match/cache behavior explicitly. Check malformed files, wrong hashes,
  unapproved assets and avatar/weapon selector isolation.
- [ ] **A7 — Repeatable handoff.** Document the exact role-based route, expected
  screens, asset IDs/hashes, deployed revisions and two-client/mobile results.
  Success means an artist and game owner can repeat it without an engineer.
  Only then scale to the remaining avatar collection.

A1 starts with a fresh walkthrough to identify real gaps. A2 is a concrete
source-confirmed architecture issue, not a claim that the old pilot never worked.
A3–A6 reuse implemented behavior and require current acceptance evidence; their
unchecked status does not mean every corresponding feature is absent.

IPFS transport, paid entitlement commerce and permanent conversion capacity are
separate milestones. Current published-file storage is Supabase Storage. Moving
storage or buying more infrastructure is not required to verify the free-asset
route and is not authorized by this plan.

## Artist discovery by class and skill — proposed extension

The [game asset requirements plan](studio-skill-asset-catalog.md) adds a separate
S0–S8 backlog for Studio → Game → Class → Skill → required models. It covers a
versioned game-owned export, baseline previews, targeted artist contributions,
shared website data and an eventual two-gun Wildspark cosmetic set. This is a
documentation proposal; current runtime/API behavior is unchanged. Read-only
discovery can proceed alongside A1–A7; contribution and runtime rollout reuse
the existing publication/approval gates. It does not require the class workshop.
The plan records a newer source audit, including Studio's existing grip preview;
older unchecked items above need revalidation before implementation.

## P1 — class workshop MVP after A1–A7 acceptance

The first editor belongs in OMOBA's existing training/workshop surface, close to
its mannequin, zero-cooldown and damage feedback. It reuses the current recipe
resolver and local preset storage. A website editor is a later client of the
same game-owned contract.

- [ ] **C1 — Build panel.** Choose an existing core and passive, then place four
  skill cards in Q/W/E/R. Show filters by effect/role, dependencies, mana,
  cooldown, range and unlock level using `EquippedSkills`; never duplicate the
  resolver in UI. Explain incompatible choices before Apply. Four ultimates
  remain a valid experiment, with their current level-6 lock clearly shown.
- [ ] **C2 — Immediate experiment.** Apply the validated recipe atomically in
  Combat Test, spawn passive/moving/aggressive targets, reset the trial, and
  compare damage, reach and cooldown behavior. Preserve authority and reject
  invalid edits without damaging the previous configuration.
- [ ] **C3 — Personal creations.** Add a name, description and optional approved
  avatar/weapon presentation; save, duplicate, reload and bounded import/export
  through existing local presets. Surface catalogue-revision incompatibility
  rather than silently substituting a different skill. Account synchronization
  is separate from local save, which already exists.
- [ ] **C4 — Online experimental classes.** Add game-owned immutable build IDs,
  revisions and recipe hashes, account storage and explicit permitted game modes.
  The server resolves/revalidates an allowed revision; draft, snapshots,
  reconnect, bots and results retain that exact recipe. Pin it for the whole
  match. Do not accept arbitrary client stat values/scripts or silently enable
  the local laboratory command path on production workers. Release the wire and
  catalogue changes together under the existing compatibility handshake.
- [ ] **C5 — Sharing and approval.** Account library/gallery, copy/remix attribution
  and private/unranked custom rooms first; curated competitive availability only
  after separate balance review. Optional web UI belongs to `omoba-web` with
  the game's account API. `omoba-ui` provides design assets, not game persistence.

A useful first experience: select four compatible skills, immediately try them
against the new practice targets, save the build, and later share a pinned build
with friends in a permitted custom mode. That loop is the product test, rather
than simply exposing a JSON form.

## Ownership and boundaries

| Concern | Owner / invariant |
| --- | --- |
| Avatar/weapon identity, attribution, versions and publication | Ekza Registry / Studio; shared asset lifecycle |
| Download, hash/size validation and cache | Ekza SDK; immutable approved files |
| Class core/passive/skill rules and validation | OMOBA `shared`; `common` executes the same rules online/offline |
| Online class admission and balance | OMOBA server; fixed validated revision for the match |
| Workshop, previews, input and animation | OMOBA client; existing skill-owned presentation and humanoid hand sockets |
| Future game account/gallery/web editor | OMOBA account API / `omoba-web`; expose a game-owned build contract |

Keep `appearance` (approved avatar/weapon IDs) separate from `build` (core,
passive and skills). Equipping a gun-shaped model must not grant shooting damage
or swap class abilities. Cosmetic publication approval is not competitive class
approval. A visual class preset can reference assets without adding another
asset store or turning global Registry records into OMOBA executable logic.

Do not represent every custom build as another hardcoded `HeroClass` enum
variant. Do not add a generic visual scripting engine or arbitrary executable
skills in this MVP. Entirely new skill logic and a multi-phase animation/effect
editor are later work; current skills already provide useful combinations.

## Validation of this planning change

Read-only source/document audit on 2026-10-05. Relative document links and diff
whitespace checked. No new gameplay code, deployment, migration, hosted
publication, browser upload or physical-device test was performed in this turn.
