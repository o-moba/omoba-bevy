# Bot practice, match chat and customizable reactions

Initialized and frozen before implementation: 2026-09-14. Base: 8872215.
Task: BOTS-SOCIAL-2026-09-14. User requested a game available to solo testers,
chat and picture reactions selected by holding their hero, with NFT customization
as the architectural direction. Work is isolated; no production deployment.

- AC1: Explicit Practice mode starts promptly with one human and fills vacant
  seats with visibly labelled server-controlled heroes. Late humans replace bots
  at a safe spawn without inheriting their identity/statistics; reconnect, full
  human capacity, round turnover and the bounded lifetime roster remain correct.
- AC2: Bots navigate actual map geometry, push lanes, approach/attack/cast using
  authoritative gameplay helpers and can retreat/die/respawn. No teleport,
  damage/cooldown bypass or fabricated player input. Practice awards no ranked
  credit. Existing Release remains human-only. Provide a simple host command.
- AC3: Match and team chat use server-owned sender identity and per-recipient
  filtering. Bounded Unicode text, request deduplication, rate limits, round/
  session binding and stale-packet rejection; authenticated operations use existing
  account signatures. Ephemeral chat does not require DB schema changes.
- AC4: Picture reactions appear briefly over their actual sender. Hold the local
  hero to open a reaction wheel on touch; provide desktop and visible-button
  alternatives. Moving/OS-cancel/invalid releases cancel; existing HUD gestures,
  gameplay orders and closing-frame input cannot leak into attacks/movement.
- AC5: Versioned catalog separates stable reaction/pack IDs and access policy
  from local image presentation. Free bundled reactions work now. Protected packs
  fail closed without verified entitlements. Define a trusted NFT-provider boundary
  compatible with future Ekza integration; do not claim existing avatar-only
  Passport verifies arbitrary sticker NFTs. No arbitrary network asset URLs.
- AC6: Relevant server/client/shared tests and actual UDP integration pass,
  native desktop/mobile-preview captures show chat and reactions, formatting and
  Clippy pass. Version, changelog, features, runbook and evidence report actual
  capabilities and physical-device/service limits. Preserve unrelated files.

Implementation boundary: one explicit practice arena per server process. Full
human arenas may reject additional entrants. Bound repeated late entries by
explicit safe round rollover rather than corrupting immutable receipts. Initial
practice statistics may remain local/unranked if no database is available; report
actual history behavior. No automatic ranked bot substitution, global chat,
voice, party matchmaking, NFT mint/marketplace, smart-contract deployment or
generic ownership API is implied. No new production dependency is planned.

Verification: reuse the prior isolated Cargo cache under a shared workspace lease;
keep new raw output in this task. Mark synthetic renderer fixtures honestly and
test input state transitions separately. All criteria must PASS before completion.
