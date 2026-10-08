//! What a capture harness records of a row and of the catalog facts behind it. The harness
//! compares nothing itself: these are the values a still is judged against.
use super::category::{self, SkillKey};
use super::geometry::{self, AreaContext, GeoShape};
use super::signature::{self, ImpactSig};
use super::{SkillPresentation, bodies};
use crate::combat_visuals::CombatVisualRegistry;
use bevy::prelude::Vec2;
use shared::loadout::{SkillEffectState, SkillId, skill};

/// Whether a damage receipt of the row's own slot can exist.
pub(crate) fn can_damage(id: &str) -> bool {
    SkillKey::from_id(id).is_some_and(category::can_damage)
}

/// The class whose default kit owns the row.
pub(crate) fn home(id: &str) -> Option<&'static str> {
    SkillKey::from_id(id).map(|key| key.home().id())
}

/// The wire spelling of the kind of an effect, and the block of its row that draws it:
/// `body` for the effect of the first cast, `aux` for a secondary object.
pub(crate) fn binding(effect: &SkillEffectState) -> (&'static str, &'static str) {
    let block = if category::own_kinds(effect.skill).contains(&effect.kind) {
        "body"
    } else {
        "aux"
    };
    (category::kind_id(effect.kind), block)
}

/// The most mesh parts one body of the skill may have.
pub(crate) fn part_budget(id: SkillId) -> usize {
    if skill(id).ability.base_cooldown_secs >= bodies::LONG_COOLDOWN_SECS {
        bodies::MAX_PARTS_LONG_COOLDOWN
    } else {
        bodies::MAX_PARTS
    }
}

/// Radius of the area the first cast strikes around its caster, or around the spot the
/// cast carries the caster to. Other skills strike no such area.
pub(crate) fn cast_area_radius(id: SkillId) -> Option<f32> {
    let around_caster = AreaContext {
        origin: Vec2::ZERO,
        arrival: Some(Vec2::ZERO),
        direction: Vec2::X,
        recast: false,
    };
    match geometry::instant_area(id, &around_caster)? {
        GeoShape::Ring { radius, .. } => Some(radius),
        _ => None,
    }
}

/// The identity of a row as the identity gate compares it: the motion family of its
/// release, the key of its body and the kind of its impact.
pub(crate) fn identity(
    registry: &SkillPresentation,
    projectiles: &CombatVisualRegistry,
    id: &str,
) -> Option<serde_json::Value> {
    let identity = signature::identity(registry, projectiles, id)?;
    let (body, silhouette) = identity.body.key();
    Some(serde_json::json!({
        "motion": identity.motion,
        "body": [body, silhouette],
        "impact": match identity.impact {
            ImpactSig::None => serde_json::Value::Null,
            ImpactSig::Themed { kind, lead } => serde_json::json!([kind.id(), lead.id()]),
        },
    }))
}
