//! Server-owned sight; no client visibility claims participate in combat or replication.
use std::time::Instant;

use shared::combat::CombatEntityKind;
use shared::map::Team;
use shared::vision::*;
use shared::wire::{
    GameState, MinionTargetKind, ServerPacket, StructureKind, TargetId, TargetKind,
};
use shared::{SkillSlot, TargetingMode, ability_for_class_slot};

use crate::entities::ConnectedPlayer;
use crate::game_world::GameWorld;

pub(crate) fn sources(team: Team, world: &GameWorld) -> Vec<VisionSource> {
    let GameWorld {
        players,
        minions,
        structures,
        ..
    } = world;
    let mut result: Vec<_> = players
        .values()
        .filter(|p| p.joined && p.hero.hp > 0.0 && p.hero.identity.team == team)
        .map(|p| VisionSource {
            position: [p.hero.x, p.hero.z],
            radius: HERO_SIGHT_RADIUS,
        })
        .collect();
    result.extend(
        minions
            .values()
            .filter(|p| p.state.hp > 0.0 && p.state.team == team)
            .map(|p| VisionSource {
                position: [p.state.x, p.state.z],
                radius: MINION_SIGHT_RADIUS,
            }),
    );
    result.extend(
        structures
            .values()
            .filter(|p| p.state.hp > 0.0 && p.state.team == team)
            .map(|p| VisionSource {
                position: [p.state.x, p.state.z],
                radius: match p.state.kind {
                    StructureKind::Tower => TOWER_SIGHT_RADIUS,
                    StructureKind::BaseTower => BASE_SIGHT_RADIUS,
                },
            }),
    );
    result.sort_by(|a, b| {
        a.position[0]
            .total_cmp(&b.position[0])
            .then_with(|| a.position[1].total_cmp(&b.position[1]))
            .then_with(|| a.radius.total_cmp(&b.radius))
    });
    result
}

pub(crate) fn revealed(player: &ConnectedPlayer, now: Instant) -> bool {
    let recent =
        |at: Instant| now.saturating_duration_since(at).as_secs_f32() < HOSTILE_REVEAL_SECS;
    player.timers.last_basic_attack_at.is_some_and(recent)
        || SkillSlot::ALL.into_iter().any(|slot| {
            ability_for_class_slot(player.hero.identity.hero_class, slot).targeting
                == TargetingMode::UnitTarget
                && player.timers.last_cast_at[slot.index()].is_some_and(recent)
        })
}
pub(crate) fn player_visible(
    sight: &[VisionSource],
    player: &ConnectedPlayer,
    now: Instant,
) -> bool {
    player.joined
        && point_visible(
            sight,
            [player.hero.x, player.hero.z],
            !revealed(player, now),
        )
}
pub(crate) fn target_visible(
    team: Team,
    target: TargetId,
    world: &GameWorld,
    now: Instant,
) -> bool {
    let GameWorld {
        players,
        minions,
        structures,
        neutrals,
        ..
    } = world;
    let sight = sources(team, world);
    match target.kind {
        TargetKind::Player => players
            .values()
            .find(|p| p.hero.identity.id == target.id)
            .is_some_and(|p| p.hero.identity.team == team || player_visible(&sight, p, now)),
        TargetKind::Minion => minions.get(&target.id).is_some_and(|p| {
            p.state.team == team || point_visible(&sight, [p.state.x, p.state.z], false)
        }),
        TargetKind::Structure => structures.get(&target.id).is_some_and(|p| {
            p.state.team == team || point_visible(&sight, [p.state.x, p.state.z], false)
        }),
        TargetKind::Neutral => neutrals
            .get(&target.id)
            .is_some_and(|p| point_visible(&sight, [p.state.x, p.state.z], false)),
    }
}

pub(crate) fn filter_snapshot(
    packet: &mut ServerPacket,
    viewer: &ConnectedPlayer,
    world: &GameWorld,
    now: Instant,
) {
    let GameWorld {
        players: live_players,
        minions: live_minions,
        structures: live_structures,
        neutrals: live_neutrals,
        projectiles: live_projectiles,
        ..
    } = world;
    let ServerPacket::Snapshot {
        vision,
        players,
        structures,
        minions,
        neutrals,
        projectiles,
        combat_events,
        forest_pickups,
        game_state,
        ..
    } = packet
    else {
        return;
    };
    // Public lobbies remain public. Completed rounds retain fog until reset.
    if matches!(
        game_state,
        GameState::Forming { .. } | GameState::Starting { .. }
    ) {
        return;
    }
    let sight = if viewer.joined {
        sources(viewer.hero.identity.team, world)
    } else {
        Vec::new()
    };
    let local_brush = (viewer.joined && viewer.hero.hp > 0.0)
        .then(|| brush_at([viewer.hero.x, viewer.hero.z]))
        .flatten();
    let enemy_sight = sources(
        match viewer.hero.identity.team {
            Team::Green => Team::Blue,
            Team::Blue => Team::Green,
        },
        world,
    );
    *vision = Some(TeamVision {
        local_brush,
        local_hidden: local_brush.is_some()
            && viewer.hero.hp > 0.0
            && !player_visible(&enemy_sight, viewer, now),
        sources: sight.clone(),
    });
    players.retain(|p| {
        viewer.joined
            && (p.team == viewer.hero.identity.team
                || live_players
                    .values()
                    .find(|live| live.hero.identity.id == p.id)
                    .is_some_and(|live| player_visible(&sight, live, now)))
    });
    structures.retain(|p| {
        viewer.joined
            && (p.team == viewer.hero.identity.team || point_visible(&sight, [p.x, p.z], false))
    });
    minions.retain(|p| {
        viewer.joined
            && (p.team == viewer.hero.identity.team || point_visible(&sight, [p.x, p.z], false))
    });
    neutrals.retain(|p| point_visible(&sight, [p.x, p.z], false));
    let visible = |kind: CombatEntityKind, id: u64| match kind {
        CombatEntityKind::Player => players.iter().any(|p| p.id == id),
        CombatEntityKind::Minion => minions.iter().any(|p| p.id == id),
        CombatEntityKind::Structure => structures.iter().any(|p| p.id == id),
        CombatEntityKind::Neutral => neutrals.iter().any(|p| p.id == id),
        CombatEntityKind::Unknown => false,
    };
    projectiles.retain(|p| {
        point_visible(&sight, [p.x, p.z], false)
            && visible(p.source_kind, p.owner_id)
            && live_projectiles.get(&p.id).is_some_and(|live| {
                visible(
                    match live.target.kind {
                        TargetKind::Player => CombatEntityKind::Player,
                        TargetKind::Minion => CombatEntityKind::Minion,
                        TargetKind::Structure => CombatEntityKind::Structure,
                        TargetKind::Neutral => CombatEntityKind::Neutral,
                    },
                    live.target.id,
                )
            })
    });
    combat_events.retain(|event| {
        // Lethal simulation removes dynamic victims before the snapshot is built.
        // Preserve the final visible impact without reintroducing a living hidden
        // target or bypassing hero brush concealment.
        let removed_victim = event.killed
            && match event.target.kind {
                CombatEntityKind::Minion => live_minions
                    .get(&event.target.id)
                    .is_none_or(|p| p.state.hp <= 0.0),
                CombatEntityKind::Neutral => live_neutrals
                    .get(&event.target.id)
                    .is_none_or(|p| p.state.hp <= 0.0),
                CombatEntityKind::Structure => live_structures
                    .get(&event.target.id)
                    .is_none_or(|p| p.state.hp <= 0.0),
                _ => false,
            };
        visible(event.source.kind, event.source.id)
            && (visible(event.target.kind, event.target.id) || removed_victim)
            && point_visible(
                &sight,
                [event.x, event.z],
                event.target.kind == CombatEntityKind::Player
                    && live_players
                        .values()
                        .find(|p| p.hero.identity.id == event.target.id)
                        .is_none_or(|p| !revealed(p, now)),
            )
    });
    forest_pickups.retain(|p| point_visible(&sight, p.position, false));
    for pickup in forest_pickups {
        if pickup
            .last_collector_id
            .is_some_and(|id| !visible(CombatEntityKind::Player, id))
        {
            pickup.last_collector_id = None;
            pickup.healed_amount = 0.0;
            pickup.collection_sequence = 0;
        }
    }
    // AI target identifiers are private even when the attacking minion is allied.
    let visible_players: Vec<_> = players.iter().map(|p| p.id).collect();
    let visible_minions: Vec<_> = minions.iter().map(|p| p.id).collect();
    let visible_structures: Vec<_> = structures.iter().map(|p| p.id).collect();
    for minion in minions {
        let valid = match (minion.target_kind, minion.target_id) {
            (Some(MinionTargetKind::Player), Some(id)) => visible_players.contains(&id),
            (Some(MinionTargetKind::Minion), Some(id)) => visible_minions.contains(&id),
            (Some(MinionTargetKind::Structure), Some(id)) => visible_structures.contains(&id),
            _ => false,
        };
        if !valid {
            minion.target_kind = None;
            minion.target_id = None;
        }
    }
}

#[cfg(test)]
mod tests;
