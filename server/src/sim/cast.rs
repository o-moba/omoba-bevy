//! Ability casts and skill upgrades.
use crate::*;

/// Spends a skill point on the given slot, capped by the class ability's max rank.
pub(crate) fn apply_skill_upgrade(player: &mut ConnectedPlayer, slot: u8) {
    let Some(skill_slot) = SkillSlot::from_index(slot) else {
        return;
    };
    let def = ability_for_class_slot(player.state.hero_class, skill_slot);
    let s = skill_slot.index();
    if player.state.skill_points > 0 && player.state.ranks[s] < def.max_rank {
        player.state.ranks[s] += 1;
        if let Some(c) = &mut player.sandbox {
            c.ranks = player.state.ranks;
        }
        player.state.skill_points -= 1;
        println!(
            "Player {} upgraded {} (slot {}) to rank {}",
            player.state.id, def.id, s, player.state.ranks[s]
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
    if !caster.joined || caster.state.hp <= 0.0 {
        return;
    }
    // Authoritative kit resolution: class + slot -> ability definition.
    let def = ability_for_class_slot(caster.state.hero_class, skill_slot);
    if !caster.sandbox.as_ref().is_some_and(|c| c.unlock_all)
        && !unlocked_slots_for_level(caster.state.level)[skill_slot.index()]
    {
        return;
    }
    let rank = caster.state.ranks[skill_slot.index()].clamp(1, def.max_rank);
    let mana_cost = scaled_mana_cost(def, rank);
    if !caster.sandbox.as_ref().is_some_and(|c| c.infinite_resource)
        && caster.state.mana < mana_cost
    {
        return;
    }
    if !caster.sandbox.as_ref().is_some_and(|c| c.no_cooldowns)
        && caster.last_cast_at[skill_slot.index()].is_some_and(|last_cast| {
            now.duration_since(last_cast) < sandbox::effective_ability_cooldown(caster, skill_slot)
        })
    {
        return;
    }

    if sandbox::skill_recovery_remaining(caster, now) > 0.0 {
        return;
    }
    let effect_scale = rank_effect_scale(rank)
        * shared::hero_balance::ability_power_multiplier(
            caster.state.hero_class,
            caster.state.level,
        );
    if def.targeting == TargetingMode::SelfTarget {
        let Some(caster_mut) = world.players.get_mut(&caster_addr) else {
            return;
        };
        if !caster_mut
            .sandbox
            .as_ref()
            .is_some_and(|c| c.infinite_resource)
        {
            caster_mut.state.mana -= mana_cost;
        }
        caster_mut.last_cast_at[skill_slot.index()] = Some(now);
        sandbox::refresh_skill_cooldowns(caster_mut, now);
        record_player_action(caster_mut, skill_slot);
        if let Some(heal) = def.self_heal {
            caster_mut.state.hp =
                (caster_mut.state.hp + heal * effect_scale).min(caster_mut.state.max_hp);
        }
        if let Some(restore) = def.self_mana_restore {
            caster_mut.state.mana =
                (caster_mut.state.mana + restore * effect_scale).min(caster_mut.state.max_mana);
        }
        return;
    }

    let caster_team = caster.state.team;
    if caster.sandbox.is_none() && !vision::target_visible(caster_team, target, world, now) {
        return;
    }
    let (target_position, target_radius) = match target.kind {
        TargetKind::Player => {
            let Some(target_player) = world.players.values().find(|player| {
                player.joined
                    && player.state.id == target.id
                    && player.state.hp > 0.0
                    && player.state.team != caster_team
            }) else {
                return;
            };
            (
                Vec3f::new(
                    target_player.state.x,
                    target_player.state.y + AIM_HEIGHT,
                    target_player.state.z,
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
        caster.state.x,
        caster.state.y + CAST_SPAWN_HEIGHT,
        caster.state.z,
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
    if !caster_mut
        .sandbox
        .as_ref()
        .is_some_and(|c| c.infinite_resource)
    {
        caster_mut.state.mana -= mana_cost;
    }
    caster_mut.last_cast_at[skill_slot.index()] = Some(now);
    sandbox::refresh_skill_cooldowns(caster_mut, now);
    record_player_action(caster_mut, skill_slot);

    // Higher invested rank = proportionally more projectile damage; active
    // boss team buffs multiply the outgoing ability damage authoritatively.
    let rank_damage = def.projectile_damage.unwrap_or(0.0)
        * effect_scale
        * caster_mut.state.item_bonuses.damage_multiplier
        * world.team_buffs.damage_multiplier(caster_team, now);

    let projectile_id = world.next_projectile_id;
    world.next_projectile_id += 1;

    world.projectiles.insert(
        projectile_id,
        Projectile {
            state: ProjectileState {
                source_kind: CombatEntityKind::Player,
                style: ProjectileStyle::for_class(caster_mut.state.hero_class),
                action_slot: Some(slot),
                direction: [direction.x, direction.y, direction.z],
                id: projectile_id,
                owner_id: caster_mut.state.id,
                owner_team: caster_mut.state.team,
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
    player.state.action_sequence = player.state.action_sequence.wrapping_add(1);
    if player.state.action_sequence == 0 {
        player.state.action_sequence = 1;
    }
    player.state.action_kind = if slot == SkillSlot::Q {
        PlayerActionKind::Attack
    } else {
        PlayerActionKind::Cast
    };
    player.state.action_slot = slot.index() as u8;
}
