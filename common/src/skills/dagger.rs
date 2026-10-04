//! Close, target-resolved dagger techniques. Recipes dispatch these actions on
//! any core; no class identity grants damage, control or proc eligibility.
use super::*;
use shared::loadout::Technique;
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;

/// Private keyed randomness belongs to the authoritative world, never a packet.
/// RandomState is independently seeded by std's platform entropy. Only accepted
/// eligible hits draw; neither request ids nor public match state seed the hash.
#[derive(Default)]
pub(super) struct Chance {
    key: RandomState,
    counter: u64,
    #[cfg(test)]
    forced: Option<bool>,
}
impl Chance {
    fn vital_break(&mut self) -> bool {
        #[cfg(test)]
        if let Some(outcome) = self.forced.take() {
            self.counter += 1;
            return outcome;
        }
        // Rejection sampling makes precisely one of fifty outcomes a proc.
        let ceiling = u64::MAX - u64::MAX % 50;
        loop {
            self.counter = self.counter.wrapping_add(1);
            if self.counter == 0 {
                self.key = RandomState::new();
            }
            let value = self.key.hash_one(self.counter);
            if value < ceiling {
                return value.is_multiple_of(50);
            }
        }
    }
}

pub(super) fn handles(action: Technique) -> bool {
    matches!(
        action,
        Technique::DaggerDeadlyBlow
            | Technique::DaggerBluff
            | Technique::DaggerBackstab
            | Technique::DaggerLethalBlow
    )
}

/// The 120-degree rear cone, measured from authoritative victim facing. A
/// coincident attacker has no rear direction and nonfinite data never qualifies.
fn in_rear(origin: [f32; 2], victim: [f32; 2], yaw: f32) -> bool {
    let delta = [origin[0] - victim[0], origin[1] - victim[1]];
    let length = delta[0].hypot(delta[1]);
    let forward = shared::math::hero_forward(yaw);
    length.is_finite()
        && length > 0.001
        && yaw.is_finite()
        && (delta[0] * forward[0] + delta[1] * forward[1]) / length <= -0.5
}
fn rear_hero(w: &GameWorld, origin: [f32; 2], target: TargetId) -> bool {
    target.kind == TargetKind::Player
        && w.players.values().any(|p| {
            p.joined
                && p.hero.hp > 0.0
                && p.hero.identity.id == target.id
                && in_rear(origin, [p.hero.x, p.hero.z], p.hero.yaw)
        })
}

pub(super) fn basic_multiplier(w: &GameWorld, owner: u64, target: TargetId) -> f32 {
    w.players
        .values()
        .find(|p| p.joined && p.hero.identity.id == owner)
        .filter(|p| {
            p.hero
                .skills
                .loadout
                .is_some_and(|l| l.passive() == PassiveId::DaggerMastery)
                && rear_hero(w, [p.hero.x, p.hero.z], target)
        })
        .map_or(1.0, |_| 1.1)
}

pub(super) fn cast(w: &mut GameWorld, addr: SocketAddr, slot: u8, aim: [f32; 2], now: Instant) {
    let Some(p) = w.players.get(&addr) else {
        return;
    };
    let Some(loadout) = p.hero.skills.loadout else {
        return;
    };
    let Some(skill_slot) = SkillSlot::from_index(slot) else {
        return;
    };
    let def = loadout.skill(skill_slot);
    let SkillEffect::Technique {
        action,
        damage,
        duration_secs,
        ..
    } = def.effect
    else {
        return;
    };
    if !handles(action)
        || p.hero
            .skills
            .advanced
            .charm
            .is_some_and(|(_, until)| until > now)
        || (!p.modifiers.no_cooldowns
            && (remaining(p.hero.skills.recovery_until, now) > 0.0
                || p.timers.last_cast_at[slot as usize].is_some_and(|at| {
                    now.saturating_duration_since(at) < hero_stats::ability_cooldown(p, skill_slot)
                })))
    {
        return;
    }
    let origin = [p.hero.x, p.hero.z];
    if distance(origin, aim) > def.ability.cast_range + 0.001 {
        return;
    }
    let team = p.hero.identity.team;
    let owner = p.hero.identity.id;
    let target = candidates(w)
        .into_iter()
        .filter(|c| {
            hostile(c, team)
                && (action != Technique::DaggerBluff || c.target.kind == TargetKind::Player)
                && distance(origin, c.pos) <= def.ability.cast_range + c.radius
                && distance(aim, c.pos) <= 1.0 + c.radius
                && (p.modifiers.bypass_vision
                    || crate::vision::target_visible(team, c.target, w, now))
                && (c.target.kind != TargetKind::Player
                    || w.players.values().any(|victim| {
                        victim.hero.identity.id == c.target.id
                            && !victim.modifiers.god_mode
                            && !victim.hero.skills.advanced.immune(now)
                            && (action != Technique::DaggerBluff
                                || remaining(victim.hero.skills.advanced.unstoppable_until, now)
                                    == 0.0)
                    }))
        })
        .min_by(|a, b| {
            distance(a.pos, aim)
                .total_cmp(&distance(b.pos, aim))
                .then_with(|| key(a.target).cmp(&key(b.target)))
        });
    let Some(target) = target else { return };
    let rank = p.hero.progress.ranks[slot as usize].clamp(1, def.ability.max_rank);
    let cost = scaled_mana_cost(&def.ability, rank);
    if !p.modifiers.infinite_resource && p.hero.mana < cost {
        return;
    }
    let rear = rear_hero(w, origin, target.target);
    let scale = shared::rank_effect_scale(rank)
        * shared::hero_balance::ability_power_multiplier(
            p.hero.identity.hero_class,
            p.hero.progress.level,
        )
        * hero_stats::combat_bonuses(p).damage_multiplier
        * w.team_buffs.damage_multiplier(team, now);
    let p = w.players.get_mut(&addr).unwrap();
    if !p.modifiers.infinite_resource {
        p.hero.mana -= cost;
    }
    p.timers.last_cast_at[slot as usize] = Some(now);
    p.hero.skills.recovery_until = Some(now + duration(def.windup_secs.max(0.15)));
    accepted_technique(p, now);
    crate::sim::cast::record_player_action(p, skill_slot);
    crate::sim::cast::face_player_action(p, target.pos[0] - origin[0], target.pos[1] - origin[1]);
    if action == Technique::DaggerBluff {
        let victim = w
            .players
            .values_mut()
            .find(|p| p.hero.identity.id == target.target.id)
            .unwrap();
        crate::recall::cancel(victim);
        let until = now + duration(duration_secs);
        victim.hero.skills.control.stun_until = Some(
            victim
                .hero
                .skills
                .control
                .stun_until
                .unwrap_or(now)
                .max(until),
        );
        victim.hero.skills.advanced.charm = None;
        // Victim forward points away from the caster, so the caster is behind.
        crate::sim::cast::face_player_action(
            victim,
            target.pos[0] - origin[0],
            target.pos[1] - origin[1],
        );
        victim.timers.last_movement_at = now;
        victim.timers.movement_slack = 0.0;
        return;
    }
    let rear_multiplier = if rear {
        match action {
            Technique::DaggerBackstab => 2.5,
            Technique::DaggerLethalBlow => 1.5,
            _ => 1.0,
        }
    } else {
        1.0
    };
    let src = source(owner, slot);
    let mut events = apply_hit(
        w,
        target.target,
        damage * scale * rear_multiplier,
        DamageType::Physical,
        src,
        team,
        false,
        false,
        false,
        now,
    );
    let survivor_hp = w
        .players
        .values()
        .find(|p| {
            p.hero.identity.id == target.target.id && target.target.kind == TargetKind::Player
        })
        .map_or(0.0, |p| p.hero.hp);
    // Actual HP movement is required: absorbed hits and sandbox infinite-HP
    // receipts cannot roll. A dead or already-one-HP target never rolls either.
    if action == Technique::DaggerBackstab
        && rear
        && survivor_hp > 1.0
        && survivor_hp < target.hp
        && events.iter().any(|e| {
            e.target.id == target.target.id
                && e.target.kind == CombatEntityKind::Player
                && e.amount > 0.0
        })
        && w.skill_runtime.dagger_chance.vital_break()
    {
        // The ordinary damage gate still checks immunity and consumes shields.
        // No on-hit recursion: a vital break can neither execute nor lifesteal.
        if let Some(event) = crate::combat_feedback::apply_player_nonlethal_damage(
            &mut w.players,
            target.target.id,
            survivor_hp - 1.0,
            now,
        ) {
            let mut event = src.annotate(event);
            event.near_lethal = true;
            events.push(event);
        }
    }
    w.skill_runtime.pending.extend(events);
}

#[cfg(test)]
mod tests;
