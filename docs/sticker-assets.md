# Account-owned reaction assets

The multiplayer path already uses an authoritative `ReactionCatalog` and trusted `Entitlements`. `passport/src/entitlements.rs` grants configured avatar-companion packs only after a Passport ticket has been verified. `ReactionVisuals` controls image presentation separately; a local manifest never grants ownership. Sending a reaction carries only a bounded catalog ID, never an arbitrary URL or client-owned claim. Match server events determine who sees it; fog-of-war and mute filters still apply.

The iPhone interface opens the wheel by tapping the local avatar, or holding and dragging as before. Toolbar smile buttons are redundant on phone. Images should be cached before interaction; server-confirmed reaction events are the multiplayer source of truth.

## SDK sticker asset route to implement

1. Add a versioned `reaction-pack` capability to the Ekza asset schema/catalog: pack ID, approved immutable image digest/CID, atlas cells, bounds (PNG dimensions/bytes), license and creator attribution. Reuse the Studio submission/game-approval lifecycle from avatars/weapons.
2. Extend verified SDK/Passport entitlement tickets with approved pack IDs scoped to this game and account. Server validates issuer, expiry, audience and asset approval, then maps grants into `ReactionCatalog` entitlements. Client lists server-allowed IDs and can choose four wheel slots.
3. Download/cache approved images via the SDK's verified content path with strict byte/dimension limits. The image registry updates as assets become ready; missing images show a built-in placeholder instead of silently disappearing. Remote participants may render an approved pack without owning it; ownership only controls sending/equipping.
4. Keep reaction events ID-only, deduplicated, rate-limited and short-lived. Test revoked/expired ownership, unapproved IDs, corrupt/offline downloads, two peers, reconnection, muting and hidden enemies.

This task fixes gameplay presentation and checks network reactions. General account-owned sticker upload/download remains a follow-on SDK/Studio feature. Existing avatar-companion grants remain supported.
