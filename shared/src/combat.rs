//! Authoritative combat presentation data. These fields never grant damage authority.
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MinionKind {
    Caster,
    #[default]
    #[serde(other)]
    Melee,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectileStyle {
    Arrow,
    Arcane,
    Holy,
    Crescent,
    Claw,
    CasterBolt,
    TowerBolt,
    Bullet,
    Rocket,
    #[default]
    #[serde(other)]
    Standard,
}

impl ProjectileStyle {
    /// The class's basic attack and Q style (`projectile_style` in the hero catalog).
    pub fn for_class(class: crate::HeroClass) -> Self {
        crate::catalog::hero(class).projectile_style
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CombatEntityKind {
    Player,
    Minion,
    Structure,
    Neutral,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CombatEntity {
    pub kind: CombatEntityKind,
    pub id: u64,
}

/// One server-confirmed explosion, repeated on its victim receipts so visibility
/// filtering cannot discard the only copy. The client draws each id once per round.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AreaImpact {
    pub id: u64,
    pub skill: crate::loadout::SkillId,
    pub center: [f32; 2],
    pub radius: f32,
}

impl AreaImpact {
    pub fn valid(self) -> bool {
        self.id != 0
            && self.skill == crate::loadout::SkillId::WildRocket
            && self.center.into_iter().all(f32::is_finite)
            && self.radius.is_finite()
            && self.radius > 0.0
            && self.radius <= 32.0
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CombatEvent {
    pub id: u64,
    pub source: CombatEntity,
    pub target: CombatEntity,
    pub amount: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub style: ProjectileStyle,
    pub action_slot: Option<u8>,
    pub killed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area_impact: Option<AreaImpact>,
    /// Authoritative trap activation; false for ordinary attacks/spells.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub trap_triggered: bool,
    /// An accepted rear dagger hit reduced a surviving hero to exactly 1 HP.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub near_lethal: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explosion_metadata_is_optional_bounded_and_round_trips() {
        let ordinary = serde_json::to_value(CombatEvent::default()).unwrap();
        assert!(ordinary.get("area_impact").is_none());
        let area = AreaImpact {
            id: 7,
            skill: crate::loadout::SkillId::WildRocket,
            center: [1.0, 2.0],
            radius: 3.0,
        };
        let event = CombatEvent {
            area_impact: Some(area),
            ..Default::default()
        };
        assert_eq!(
            serde_json::from_str::<CombatEvent>(&serde_json::to_string(&event).unwrap()).unwrap(),
            event
        );
        assert!(area.valid());
        for radius in [0.0, -1.0, f32::NAN, 33.0] {
            assert!(!AreaImpact { radius, ..area }.valid());
        }
        assert!(
            !AreaImpact {
                center: [f32::INFINITY, 0.0],
                ..area
            }
            .valid()
        );
        assert!(!AreaImpact { id: 0, ..area }.valid());
    }

    #[test]
    fn near_lethal_receipt_defaults_off_and_round_trips_only_when_present() {
        let ordinary = CombatEvent::default();
        let wire = serde_json::to_value(&ordinary).unwrap();
        assert!(wire.get("near_lethal").is_none());
        assert!(
            !serde_json::from_value::<CombatEvent>(wire)
                .unwrap()
                .near_lethal
        );
        let triggered = CombatEvent {
            near_lethal: true,
            ..ordinary
        };
        let wire = serde_json::to_value(&triggered).unwrap();
        assert_eq!(wire["near_lethal"], true);
        assert_eq!(
            serde_json::from_value::<CombatEvent>(wire).unwrap(),
            triggered
        );
    }

    #[test]
    fn absent_fields_and_unknown_future_enums_are_inert() {
        assert_eq!(
            serde_json::from_str::<CombatEvent>("{}").unwrap(),
            CombatEvent::default()
        );
        assert_eq!(
            serde_json::from_str::<MinionKind>("\"future_role\"").unwrap(),
            MinionKind::Melee
        );
        assert_eq!(
            serde_json::from_str::<ProjectileStyle>("\"future_style\"").unwrap(),
            ProjectileStyle::Standard
        );
        assert_eq!(
            serde_json::from_str::<CombatEntityKind>("\"future_source\"").unwrap(),
            CombatEntityKind::Unknown
        );
    }
}
