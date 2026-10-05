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
