//! Personal camp rewards shared by online and socket-free combat authority.
use shared::wire::NeutralCampType;
use std::time::{Duration, Instant};

pub const DURATION: Duration = Duration::from_secs(45);
pub const FIRE_DAMAGE: f32 = 4.0;
pub const ICE_DAMAGE: f32 = 2.0;
pub const ICE_MULTIPLIER: f32 = 0.9;
pub const ICE_SECONDS: f32 = 1.0;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JungleBuffs {
    fire_until: Option<Instant>,
    ice_until: Option<Instant>,
}
impl JungleBuffs {
    pub fn grant(&mut self, camp: NeutralCampType, now: Instant) {
        match camp {
            NeutralCampType::Bruiser => self.fire_until = Some(now + DURATION),
            NeutralCampType::Spitter => self.ice_until = Some(now + DURATION),
            _ => {}
        }
    }
    pub fn fire_damage(&self, now: Instant) -> f32 {
        if self.fire_until.is_some_and(|until| now < until) {
            FIRE_DAMAGE
        } else {
            0.0
        }
    }
    pub fn on_hit_damage(&self, now: Instant) -> f32 {
        self.fire_damage(now)
            + if self.ice_active(now) {
                ICE_DAMAGE
            } else {
                0.0
            }
    }
    pub fn ice_active(&self, now: Instant) -> bool {
        self.ice_until.is_some_and(|until| now < until)
    }
}
