//! Ability casts and skill upgrades.

use shared::clock::Instant;
use std::net::SocketAddr;

use shared::combat::{CombatEntityKind, ProjectileStyle};
use shared::wire::{GameState, ProjectileState, TargetId};
use shared::{
    AllyEffect, PlayerActionKind, SkillSlot, TargetingMode, ability_for_class_slot,
    rank_effect_scale, scaled_cast_range, scaled_mana_cost, unlocked_slots_for_level,
};

use crate::balance::{CAST_SPAWN_HEIGHT, PROJECTILE_LIFETIME, PROJECTILE_RADIUS, PROJECTILE_SPEED};
use crate::basic_attack::resolve_hostile_target;
use crate::entities::{ConnectedPlayer, Projectile, Vec3f};
use crate::game_world::GameWorld;
use crate::{hero_stats, hero_timers, vision};

/// Spends a skill point on the given slot, capped by the class ability's max rank.
pub fn apply_skill_upgrade(player: &mut ConnectedPlayer, slot: u8) {
    let Some(skill_slot) = SkillSlot::from_index(slot) else {
        return;
    };
    let equipped = hero_stats::equipped_skills(player);
    let def = equipped.ability(skill_slot);
    let s = skill_slot.index();
    if (player.modifiers.unlock_all || equipped.unlocked(player.hero.progress.level)[s])
        && player.hero.progress.skill_points > 0
        && player.hero.progress.ranks[s] < def.max_rank
    {
        player.hero.progress.ranks[s] += 1;
        player.hero.progress.skill_points -= 1;
        println!(
            "Player {} upgraded {} (slot {}) to rank {}",
            player.hero.identity.id, def.id, s, player.hero.progress.ranks[s]
        );
    }
}

/// Rank and level scale of every legacy ability primitive (damage, heal and
/// mana restore). Callers that judge whether a cast is useful read the same
/// number the accepted cast applies.
pub fn legacy_effect_scale(caster: &ConnectedPlayer, rank: u8) -> f32 {
    rank_effect_scale(rank)
        * shared::hero_balance::ability_power_multiplier(
            caster.hero.identity.hero_class,
            caster.hero.progress.level,
        )
}

fn nearby_living_allies(
    world: &GameWorld,
    caster_addr: SocketAddr,
    radius: f32,
) -> impl Iterator<Item = (SocketAddr, &ConnectedPlayer, f32)> {
    let caster = &world.players[&caster_addr].hero;
    world.players.iter().filter_map(move |(&address, player)| {
        let hero = &player.hero;
        if address == caster_addr
            || !player.joined
            || hero.identity.team != caster.identity.team
            || hero.hp <= 0.0
        {
            return None;
        }
        let distance_sq = (hero.x - caster.x).powi(2) + (hero.z - caster.z).powi(2);
        (distance_sq <= radius * radius).then_some((address, player, distance_sq))
    })
}

/// Tie-breaking by player ID makes selection independent of map iteration order.
pub(crate) fn lowest_injured_ally(
    world: &GameWorld,
    caster_addr: SocketAddr,
    radius: f32,
) -> Option<SocketAddr> {
    nearby_living_allies(world, caster_addr, radius)
        .filter(|(_, player, _)| player.hero.hp < player.hero.max_hp)
        .min_by(|(_, a, _), (_, b, _)| {
            (a.hero.hp / a.hero.max_hp)
                .total_cmp(&(b.hero.hp / b.hero.max_hp))
                .then_with(|| a.hero.identity.id.cmp(&b.hero.identity.id))
        })
        .map(|(address, _, _)| address)
}

pub fn handle_cast_request(
    world: &mut GameWorld,
    caster_addr: SocketAddr,
    target: TargetId,
    slot: u8,
    now: Instant,
) {
    if !matches!(world.game_state, GameState::Running) {
        return;
    }
    let Some(skill_slot) = SkillSlot::from_index(slot) else {
        return;
    };
    let Some(caster) = world.players.get(&caster_addr) else {
        return;
    };
    if !caster.joined || caster.hero.hp <= 0.0 || caster.hero.skills.loadout.is_some() {
        return;
    }
    if caster
        .hero
        .skills
        .control
        .stun_until
        .is_some_and(|until| until > now)
        || caster
            .hero
            .skills
            .advanced
            .charm
            .is_some_and(|(_, until)| until > now)
        || caster.hero.skills.advanced.immune(now)
    {
        return;
    }
    // Authoritative kit resolution: class + slot -> ability definition.
    let def = ability_for_class_slot(caster.hero.identity.hero_class, skill_slot);
    if !caster.modifiers.unlock_all
        && !unlocked_slots_for_level(caster.hero.progress.level)[skill_slot.index()]
    {
        return;
    }
    let rank = caster.hero.progress.ranks[skill_slot.index()].clamp(1, def.max_rank);
    let mana_cost = scaled_mana_cost(def, rank);
    if !caster.modifiers.infinite_resource && caster.hero.mana < mana_cost {
        return;
    }
    if !caster.modifiers.no_cooldowns
        && caster.timers.last_cast_at[skill_slot.index()].is_some_and(|last_cast| {
            now.duration_since(last_cast) < hero_stats::ability_cooldown(caster, skill_slot)
        })
    {
        return;
    }

    if hero_timers::skill_recovery_remaining(caster, now) > 0.0 {
        return;
    }
    let effect_scale = legacy_effect_scale(caster, rank);
    if def.targeting == TargetingMode::SelfTarget {
        let ally_to_heal = if let Some(AllyEffect::HealLowest { radius, amount }) = def.ally_effect
        {
            let Some(ally) = lowest_injured_ally(world, caster_addr, radius) else {
                return;
            };
            Some((ally, amount * effect_scale))
        } else {
            None
        };
        let Some(caster_mut) = world.players.get_mut(&caster_addr) else {
            return;
        };
        if !caster_mut.modifiers.infinite_resource {
            caster_mut.hero.mana -= mana_cost;
        }
        caster_mut.timers.last_cast_at[skill_slot.index()] = Some(now);
        record_player_action(caster_mut, skill_slot);
        if let Some(heal) = def.self_heal {
            caster_mut.hero.hp =
                (caster_mut.hero.hp + heal * effect_scale).min(caster_mut.hero.max_hp);
        }
        if let Some(restore) = def.self_mana_restore {
            caster_mut.hero.mana =
                (caster_mut.hero.mana + restore * effect_scale).min(caster_mut.hero.max_mana);
        }
        let owner = caster_mut.hero.identity.id;
        if let Some((ally, amount)) = ally_to_heal {
            let hero = &mut world.players.get_mut(&ally).unwrap().hero;
            hero.hp = (hero.hp + amount).min(hero.max_hp);
        }
        if let Some(AllyEffect::ShieldNearby {
            radius,
            amount,
            duration_secs,
        }) = def.ally_effect
        {
            let mut allies: Vec<_> = nearby_living_allies(world, caster_addr, radius)
                .map(|(_, player, distance_sq)| (distance_sq, player.hero.identity.id))
                .collect();
            allies.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            for target in std::iter::once(owner).chain(allies.into_iter().take(4).map(|(_, id)| id))
            {
                crate::skills::shield(
                    world,
                    target,
                    owner,
                    amount * effect_scale,
                    duration_secs,
                    now,
                );
            }
        }
        return;
    }

    let caster_team = caster.hero.identity.team;
    if !caster.modifiers.bypass_vision && !vision::target_visible(caster_team, target, world, now) {
        return;
    }
    let Some((target_position, target_radius)) = resolve_hostile_target(
        caster_team,
        target,
        &world.players,
        &world.minions,
        &world.structures,
        &world.neutrals,
    ) else {
        return;
    };

    let caster_position = Vec3f::new(
        caster.hero.x,
        caster.hero.y + CAST_SPAWN_HEIGHT,
        caster.hero.z,
    );
    let direction = Vec3f::new(
        target_position.x - caster_position.x,
        target_position.y - caster_position.y,
        target_position.z - caster_position.z,
    )
    .normalize_or_zero();

    if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
        return;
    }

    let dx = target_position.x - caster_position.x;
    let dz = target_position.z - caster_position.z;
    let horizontal_distance = (dx * dx + dz * dz).sqrt();
    if horizontal_distance > scaled_cast_range(def, rank) + target_radius {
        return;
    }

    let Some(caster_mut) = world.players.get_mut(&caster_addr) else {
        return;
    };
    if !caster_mut.modifiers.infinite_resource {
        caster_mut.hero.mana -= mana_cost;
    }
    caster_mut.timers.last_cast_at[skill_slot.index()] = Some(now);
    record_player_action(caster_mut, skill_slot);
    face_player_action(caster_mut, direction.x, direction.z);

    // Higher invested rank = proportionally more projectile damage; active
    // boss team buffs multiply the outgoing ability damage authoritatively.
    let rank_damage = def.projectile_damage.unwrap_or(0.0)
        * effect_scale
        * hero_stats::combat_bonuses(caster_mut).damage_multiplier
        * world.team_buffs.damage_multiplier(caster_team, now);

    let projectile_id = world.next_projectile_id;
    world.next_projectile_id += 1;

    world.projectiles.insert(
        projectile_id,
        Projectile {
            state: ProjectileState {
                source_kind: CombatEntityKind::Player,
                style: ProjectileStyle::for_class(caster_mut.hero.identity.hero_class),
                action_slot: Some(slot),
                direction: [direction.x, direction.y, direction.z],
                id: projectile_id,
                owner_id: caster_mut.hero.identity.id,
                owner_team: caster_mut.hero.identity.team,
                x: caster_position.x,
                y: caster_position.y,
                z: caster_position.z,
            },
            target,
            velocity: Vec3f::new(
                direction.x * PROJECTILE_SPEED,
                direction.y * PROJECTILE_SPEED,
                direction.z * PROJECTILE_SPEED,
            ),
            homing: true,
            guaranteed_hit: true,
            damage: rank_damage,
            radius: PROJECTILE_RADIUS,
            expires_at: now + PROJECTILE_LIFETIME,
        },
    );
}

pub fn record_player_action(player: &mut ConnectedPlayer, slot: SkillSlot) {
    player.hero.last_action.sequence = player.hero.last_action.sequence.wrapping_add(1);
    if player.hero.last_action.sequence == 0 {
        player.hero.last_action.sequence = 1;
    }
    // Animation category follows the authored skill role, while the receipt's
    // action_slot remains the physical binding used by the caller.
    let authored = player
        .hero
        .skills
        .loadout
        .map_or(slot, |l| l.skill(slot).slot);
    player.hero.last_action.kind = PlayerActionKind::for_cast(authored);
    player.hero.last_action.slot = slot.index() as u8;
    player.hero.last_action.yaw = None;
}

/// Cosmetic direction is captured only after acceptance. Movement cannot overwrite it.
pub fn face_player_action(player: &mut ConnectedPlayer, dx: f32, dz: f32) {
    player.hero.last_action.yaw = (dx.is_finite() && dz.is_finite() && dx * dx + dz * dz > 1e-6)
        .then(|| shared::math::hero_yaw_towards(dx, dz));
    if let Some(yaw) = player.hero.last_action.yaw {
        player.hero.yaw = yaw;
    }
}
