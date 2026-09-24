//! Ability casts and skill upgrades.

use crate::balance::PROJECTILE_SPEED;
use std::net::SocketAddr;

use std::time::Instant;

use shared::unlocked_slots_for_level;

use crate::game_world::GameWorld;

use crate::balance::NEUTRAL_RADIUS;

use crate::entities::Vec3f;

use crate::balance::MINION_RADIUS;

use shared::wire::GameState;

use crate::entities::Projectile;

use crate::balance::CAST_SPAWN_HEIGHT;

use crate::hero_stats;

use crate::balance::AIM_HEIGHT;

use shared::wire::ProjectileState;

use shared::scaled_mana_cost;

use shared::combat::CombatEntityKind;

use shared::PlayerActionKind;

use shared::ability_for_class_slot;

use shared::SkillSlot;

use crate::hero_timers;

use shared::TargetingMode;

use crate::world::structure_radius;

use crate::vision;

use crate::balance::PROJECTILE_LIFETIME;

use shared::wire::TargetId;

use shared::wire::TargetKind;

use shared::combat::ProjectileStyle;

use shared::rank_effect_scale;

use crate::balance::PLAYER_HIT_RADIUS;

use crate::entities::ConnectedPlayer;

use crate::sim::towers::structure_is_protected;

use shared::scaled_cast_range;

use crate::balance::PROJECTILE_RADIUS;

/// Spends a skill point on the given slot, capped by the class ability's max rank.
pub(crate) fn apply_skill_upgrade(player: &mut ConnectedPlayer, slot: u8) {
    let Some(skill_slot) = SkillSlot::from_index(slot) else {
        return;
    };
    let def = ability_for_class_slot(player.hero.identity.hero_class, skill_slot);
    let s = skill_slot.index();
    if player.hero.progress.skill_points > 0 && player.hero.progress.ranks[s] < def.max_rank {
        player.hero.progress.ranks[s] += 1;
        player.hero.progress.skill_points -= 1;
        println!(
            "Player {} upgraded {} (slot {}) to rank {}",
            player.hero.identity.id, def.id, s, player.hero.progress.ranks[s]
        );
    }
}

pub(crate) fn handle_cast_request(
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
    if !caster.joined || caster.hero.hp <= 0.0 {
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
    let effect_scale = rank_effect_scale(rank)
        * shared::hero_balance::ability_power_multiplier(
            caster.hero.identity.hero_class,
            caster.hero.progress.level,
        );
    if def.targeting == TargetingMode::SelfTarget {
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
        return;
    }

    let caster_team = caster.hero.identity.team;
    if !caster.modifiers.bypass_vision && !vision::target_visible(caster_team, target, world, now) {
        return;
    }
    let (target_position, target_radius) = match target.kind {
        TargetKind::Player => {
            let Some(target_player) = world.players.values().find(|player| {
                player.joined
                    && player.hero.identity.id == target.id
                    && player.hero.hp > 0.0
                    && player.hero.identity.team != caster_team
            }) else {
                return;
            };
            (
                Vec3f::new(
                    target_player.hero.x,
                    target_player.hero.y + AIM_HEIGHT,
                    target_player.hero.z,
                ),
                PLAYER_HIT_RADIUS,
            )
        }
        TargetKind::Minion => {
            let Some(target_minion) = world.minions.get(&target.id) else {
                return;
            };
            if target_minion.state.hp <= 0.0 || target_minion.state.team == caster_team {
                return;
            }
            (
                Vec3f::new(
                    target_minion.state.x,
                    target_minion.state.y + MINION_RADIUS * 0.8,
                    target_minion.state.z,
                ),
                MINION_RADIUS,
            )
        }
        TargetKind::Structure => {
            let Some(target_structure) = world.structures.get(&target.id) else {
                return;
            };
            if target_structure.state.hp <= 0.0
                || target_structure.state.team == caster_team
                || structure_is_protected(&world.structures, target.id)
            {
                return;
            }
            (
                Vec3f::new(
                    target_structure.state.x,
                    target_structure.state.y,
                    target_structure.state.z,
                ),
                structure_radius(target_structure.state.kind),
            )
        }
        TargetKind::Neutral => {
            let Some(target_neutral) = world.neutrals.get(&target.id) else {
                return;
            };
            if target_neutral.dead_until.is_some() || target_neutral.state.hp <= 0.0 {
                return;
            }
            (
                Vec3f::new(
                    target_neutral.state.x,
                    target_neutral.state.y + NEUTRAL_RADIUS * 0.85,
                    target_neutral.state.z,
                ),
                NEUTRAL_RADIUS,
            )
        }
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

pub(crate) fn record_player_action(player: &mut ConnectedPlayer, slot: SkillSlot) {
    player.hero.last_action.sequence = player.hero.last_action.sequence.wrapping_add(1);
    if player.hero.last_action.sequence == 0 {
        player.hero.last_action.sequence = 1;
    }
    player.hero.last_action.kind = if slot == SkillSlot::Q {
        PlayerActionKind::Attack
    } else {
        PlayerActionKind::Cast
    };
    player.hero.last_action.slot = slot.index() as u8;
}
