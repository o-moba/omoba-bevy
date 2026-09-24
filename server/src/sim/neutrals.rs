//! Jungle camp and raid boss AI, damage and rewards.

use crate::entities::TeamBuffBalance;
use crate::shop::award_gold;

use shared::wire::NeutralCampType;

use shared::combat::CombatEntityKind;

use crate::neutrals::chase_neutral;

use crate::entities::ConnectedPlayer;

use crate::combat_feedback::damage_receipt;

use crate::balance::NEUTRAL_ATTACK_COOLDOWN;

use crate::neutrals::neutral_respawn_cooldown;

use crate::progression::grant_player_xp;

use std::net::SocketAddr;

use crate::entities::TeamBuffs;

use crate::neutrals::reset_neutral_at_anchor;

use shared::wire::NeutralAiState;

use crate::combat_feedback::apply_player_damage;

use crate::neutrals::neutral_template;

use crate::entities::Vec3f;

use crate::neutrals::neutral_aggro_and_leash;

use crate::game_world::TickCtx;

use shared::combat::ProjectileStyle;

use crate::entities::Neutral;

use std::collections::HashMap;

use std::time::Instant;

use crate::game_world::GameWorld;

use shared::combat::CombatEvent;

use crate::hero::Hero;

use shared::wire::GameState;

use crate::combat_feedback::HitSource;

use crate::balance::NEUTRAL_KILL_HEAL_FRACTION;

use crate::balance::AIM_HEIGHT;

pub(crate) fn apply_neutral_damage(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    neutrals: &mut HashMap<u64, Neutral>,
    team_buffs: &mut TeamBuffs,
    target_id: u64,
    damage: f32,
    attacker_player_id: u64,
    now: Instant,
) -> Option<CombatEvent> {
    if !damage.is_finite() || damage <= 0.0 {
        return None;
    }
    let neutral = neutrals.get_mut(&target_id)?;
    if neutral.dead_until.is_some() || neutral.state.hp <= 0.0 {
        return None;
    }
    // Forest Tracker: the attacker's class scales damage against monsters.
    let damage = players
        .values()
        .find(|player| player.hero.identity.id == attacker_player_id)
        .map_or(damage, |player| {
            damage
                * shared::jungle::neutral_damage_multiplier(
                    player.hero.identity.hero_class,
                    neutral.state.camp_type.is_boss(),
                )
        });
    let before = neutral.state.hp;
    neutral.state.hp = (before - damage).max(0.0);
    if players.values().any(|player| {
        player.joined && player.hero.identity.id == attacker_player_id && player.hero.hp > 0.0
    }) {
        neutral.target_player_id = Some(attacker_player_id);
        neutral.state.ai_state = NeutralAiState::Aggro;
    }
    if neutral.state.hp <= 0.0 {
        let camp_type = neutral.state.camp_type;
        award_neutral_kill_to_player(players, attacker_player_id, camp_type);
        // Boss kill: the killer's whole team gains the boss buff (refresh on
        // re-kill). Unresolvable killer (already gone) grants no buff.
        if let Some(kind) = camp_type.team_buff_kind() {
            let killer_team = players
                .values()
                .find(|player| player.hero.identity.id == attacker_player_id)
                .map(|player| player.hero.identity.team);
            if let Some(team) = killer_team {
                team_buffs.grant(team, kind, now);
                println!(
                    "Boss {camp_type:?} slain by player {attacker_player_id}; team {team:?} gains {kind:?} for {}s",
                    kind.duration().as_secs()
                );
            }
        }
        neutral.dead_until = Some(now + neutral_respawn_cooldown(camp_type));
        neutral.target_player_id = None;
        neutral.last_attack_at = None;
        neutral.state.ai_state = NeutralAiState::Idle;
    }
    damage_receipt(
        CombatEntityKind::Neutral,
        target_id,
        before,
        neutral.state.hp,
        Vec3f::new(neutral.state.x, neutral.state.y, neutral.state.z),
    )
}

pub(crate) fn award_neutral_kill_to_player(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    killer_id: u64,
    camp_type: NeutralCampType,
) {
    let rewards = neutral_template(camp_type);
    for player in players.values_mut() {
        if player.hero.identity.id == killer_id {
            let (gold, xp) = shared::jungle::neutral_kill_rewards(
                player.hero.identity.hero_class,
                camp_type.is_boss(),
                rewards.kill_gold,
                rewards.kill_xp,
            );
            award_gold(player, gold);
            if player.modifiers.grant_xp {
                grant_player_xp(&mut player.hero, xp);
            }
            if !camp_type.is_boss() && player.hero.hp > 0.0 {
                player.hero.hp = (player.hero.hp + player.hero.max_hp * NEUTRAL_KILL_HEAL_FRACTION)
                    .min(player.hero.max_hp);
            }
            break;
        }
    }
}

pub(crate) fn neutral_horizontal_distance_sq_from_anchor(anchor: Vec3f, player: &Hero) -> f32 {
    let dx = anchor.x - player.x;
    let dz = anchor.z - player.z;
    dx * dx + dz * dz
}

pub(crate) fn simulate_neutrals(world: &mut GameWorld, tick: TickCtx) -> Vec<CombatEvent> {
    let TickCtx { now, dt } = tick;
    if !matches!(world.game_state, GameState::Running) {
        return Vec::new();
    }
    let GameWorld {
        players, neutrals, ..
    } = world;

    let mut player_damage_events: Vec<(u64, f32, HitSource)> = Vec::new();

    for neutral in neutrals.values_mut() {
        if let Some(dead_until) = neutral.dead_until {
            if now >= dead_until {
                let template = neutral_template(neutral.state.camp_type);
                neutral.dead_until = None;
                neutral.state.max_hp = template.max_hp;
                reset_neutral_at_anchor(neutral);
            } else {
                continue;
            }
        }

        if neutral.state.hp <= 0.0 {
            continue;
        }

        let template = neutral_template(neutral.state.camp_type);
        let (aggro_radius, leash_distance) = neutral_aggro_and_leash(neutral.state.camp_type);
        let aggro_sq = aggro_radius * aggro_radius;
        let leash_sq = leash_distance * leash_distance;
        let anchor = neutral.anchor;
        let neutral_pos = Vec3f::new(neutral.state.x, neutral.state.y, neutral.state.z);

        if neutral.state.ai_state == NeutralAiState::Idle && neutral.target_player_id.is_none() {
            let best = players
                .values()
                .filter(|player| player.joined && player.hero.hp > 0.0)
                .map(|player| {
                    let hit = Vec3f::new(player.hero.x, player.hero.y + AIM_HEIGHT, player.hero.z);
                    (player.hero.identity.id, neutral_pos.distance_squared(hit))
                })
                .filter(|(_, dist_sq)| *dist_sq <= aggro_sq)
                .min_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
            if let Some((player_id, _)) = best {
                neutral.target_player_id = Some(player_id);
                neutral.state.ai_state = NeutralAiState::Aggro;
            }
        }

        let Some(target_id) = neutral.target_player_id else {
            // A projectile can outlive its owner and land after the previous
            // target-loss reset. With no living proximity target, discard that
            // orphan chip damage instead of leaving an abandoned wounded camp.
            reset_neutral_at_anchor(neutral);
            continue;
        };

        let Some(target_player) = players.values().find(|player| {
            player.joined && player.hero.identity.id == target_id && player.hero.hp > 0.0
        }) else {
            reset_neutral_at_anchor(neutral);
            continue;
        };

        if neutral_horizontal_distance_sq_from_anchor(anchor, &target_player.hero) > leash_sq {
            reset_neutral_at_anchor(neutral);
            continue;
        }

        let target_hit = Vec3f::new(
            target_player.hero.x,
            target_player.hero.y + AIM_HEIGHT,
            target_player.hero.z,
        );
        let dist = neutral_pos.distance(target_hit);

        if dist <= template.attack_range {
            let can_attack = neutral
                .last_attack_at
                .is_none_or(|last| now.duration_since(last) >= NEUTRAL_ATTACK_COOLDOWN);
            if can_attack {
                neutral.last_attack_at = Some(now);
                player_damage_events.push((
                    target_id,
                    template.attack_damage,
                    HitSource::new(
                        CombatEntityKind::Neutral,
                        neutral.state.id,
                        ProjectileStyle::Standard,
                    ),
                ));
            }
            let dir_x = target_hit.x - neutral.state.x;
            let dir_z = target_hit.z - neutral.state.z;
            if dir_x * dir_x + dir_z * dir_z > 0.0001 {
                neutral.state.yaw = shared::math::unit_yaw_towards(dir_x, dir_z);
            }
        } else {
            neutral.state.ai_state = NeutralAiState::Aggro;
            chase_neutral(
                neutral,
                [target_hit.x, target_hit.z],
                dt,
                shared::navigation::world_navigation(),
            );
        }
    }

    player_damage_events
        .into_iter()
        .filter_map(|(id, damage, source)| {
            apply_player_damage(players, id, damage, now).map(|event| source.annotate(event))
        })
        .collect()
}
