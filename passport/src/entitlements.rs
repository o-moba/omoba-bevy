//! The passport half of the social entitlements: turning a consumed ticket
//! into a companion reaction grant. Moved out of `shared` (step 13) so the
//! shared model carries no Ekza SDK types.

use ekza_bevy_sdk::passport::{ConsumedTicket, ProtectedAvatar};
use shared::social::Entitlements;

/// Only call after successful one-use ticket consumption at the server's
/// trusted Passport API for the current admitted session. This checks the
/// returned exact avatar/rendition, not the network origin, expiry or session;
/// the trusted service/caller has already verified those. A client-created
/// ConsumedTicket must never reach this function as evidence.
pub fn grant_verified_avatar(
    entitlements: &mut Entitlements,
    expected: &ProtectedAvatar,
    consumed: &ConsumedTicket,
) -> Result<(), &'static str> {
    expected.validate_consumed_ticket(consumed)?;
    entitlements.grant_verified_avatar_id(&expected.avatar_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ekza_bevy_sdk::passport::{ProjectSupport, Rendition};
    use shared::social::{PackAccess, ReactionCatalog, ReactionDefinition, ReactionPack};

    #[test]
    fn companion_grants_require_exact_approved_avatar_response_and_stay_scoped() {
        // Trusted-service response fixture, not a real NFT ownership assertion.
        let expected = ProtectedAvatar {
            avatar_id: format!("solana:devnet:avatar-data:{}", "1".repeat(32)),
            support: ProjectSupport {
                project_id: "omoba".into(),
                platform: "desktop".into(),
                profile: "humanoid-glb-v1".into(),
                status: "approved".into(),
                rendition: Rendition {
                    id: "r1".into(),
                    url: "https://example.test/avatar.glb".into(),
                    sha256: "a".repeat(64),
                    size_bytes: 20,
                    format: "glb".into(),
                },
            },
        };
        let mut consumed = ConsumedTicket {
            wallet: "2".repeat(32),
            mint: "3".repeat(32),
            avatar_id: expected.avatar_id.clone(),
            expires_at: "2099-01-01T00:00:00Z".into(),
            support: expected.support.clone(),
        };
        let mut catalog = ReactionCatalog {
            schema_version: 1,
            packs: vec![ReactionPack {
                id: "base".into(),
                label: "Base reactions".into(),
                access: PackAccess::Free,
            }],
            reactions: vec![ReactionDefinition {
                id: "thumbs_up".into(),
                label: "Thumbs up".into(),
                pack_id: "base".into(),
            }],
        };
        for (id, avatar_id) in [
            ("companion", expected.avatar_id.clone()),
            (
                "other_companion",
                format!("solana:devnet:avatar-data:{}", "4".repeat(32)),
            ),
        ] {
            catalog.packs.push(ReactionPack {
                id: id.into(),
                label: id.into(),
                access: PackAccess::VerifiedAvatar { avatar_id },
            });
            catalog.reactions.push(ReactionDefinition {
                id: id.into(),
                label: id.into(),
                pack_id: id.into(),
            });
        }
        catalog.validate().unwrap();
        let mut entitlements = Entitlements::default();
        consumed.support.rendition.sha256 = "b".repeat(64);
        assert!(grant_verified_avatar(&mut entitlements, &expected, &consumed).is_err());
        assert!(!catalog.reaction_allowed("companion", &entitlements));
        consumed.support = expected.support.clone();
        grant_verified_avatar(&mut entitlements, &expected, &consumed).unwrap();
        assert!(catalog.reaction_allowed("companion", &entitlements));
        assert!(!catalog.reaction_allowed("other_companion", &entitlements));
        assert!(!catalog.reaction_allowed("companion", &Entitlements::default()));
    }
}
