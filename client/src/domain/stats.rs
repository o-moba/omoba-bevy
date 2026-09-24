//! Combat pools replicated for every damageable entity (heroes, structures,
//! minions, neutrals).

use bevy::prelude::*;

pub use shared::hero_balance::{DEFAULT_MAX_HP as MAX_HP, MAX_MANA};

#[derive(Component, Clone, Copy, Debug)]
pub struct CombatStats {
    pub hp: f32,
    pub max_hp: f32,
    pub mana: f32,
    pub max_mana: f32,
}

impl Default for CombatStats {
    fn default() -> Self {
        Self {
            hp: MAX_HP,
            max_hp: MAX_HP,
            mana: MAX_MANA,
            max_mana: MAX_MANA,
        }
    }
}

impl CombatStats {
    pub fn is_alive(self) -> bool {
        self.hp > 0.0
    }
}
