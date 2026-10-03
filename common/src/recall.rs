//! Server-owned return-to-base channel, shared by online and offline matches.
use std::time::{Duration, Instant};

use crate::{balance::PLAYER_GROUND_Y, entities::ConnectedPlayer, game_world::GameWorld};
use shared::{utility::RECALL_CHANNEL_SECS, wire::GameState};

/// Above numerical drift, below a visible step. Client movement intent also
/// sends CancelRecall so a stick held against a wall interrupts immediately.
pub const MOVEMENT_CANCEL_DISTANCE: f32 = 0.025;

#[derive(Debug, Clone, Copy)]
pub struct RecallChannel {
    completes_at: Instant,
    origin: [f32; 2],
}

pub fn start(player: &mut ConnectedPlayer, now: Instant) {
    // Duplicate fresh requests may neither accelerate nor restart a channel.
    if player.timers.recall.is_none() {
        player.timers.recall = Some(RecallChannel {
            completes_at: now + Duration::from_secs_f32(RECALL_CHANNEL_SECS),
            origin: [player.hero.x, player.hero.z],
        });
    }
}

pub fn cancel(player: &mut ConnectedPlayer) {
    player.timers.recall = None;
}

pub fn remaining(player: &ConnectedPlayer, now: Instant) -> f32 {
    if player.hero.hp <= 0.0 {
        return 0.0;
    }
    player.timers.recall.map_or(0.0, |channel| {
        channel
            .completes_at
            .saturating_duration_since(now)
            .as_secs_f32()
    })
}

pub fn tick(world: &mut GameWorld, now: Instant) {
    for player in world.players.values_mut() {
        let Some(channel) = player.timers.recall else {
            continue;
        };
        let moved = (player.hero.x - channel.origin[0]).hypot(player.hero.z - channel.origin[1]);
        if !matches!(world.game_state, GameState::Running)
            || !player.joined
            || player.hero.hp <= 0.0
            || moved > MOVEMENT_CANCEL_DISTANCE
            || player.hero.skills.control.movement(now) == 0.0
        {
            cancel(player);
            continue;
        }
        if now < channel.completes_at {
            continue;
        }
        let spawn = crate::world::spawn_position_for_team_from_base(
            &world.structures,
            &world.map_layout,
            player.hero.identity.team,
        );
        player.hero.x = spawn.x;
        player.hero.y = PLAYER_GROUND_Y;
        player.hero.z = spawn.z;
        player.timers.last_movement_at = now;
        player.timers.movement_slack = 0.0;
        // The normal relocation barrier rejects delayed pre-recall transforms.
        // Health, mana, cooldowns and skill ownership are deliberately retained.
        player.hero.utility.dash_sequence = player.hero.utility.dash_sequence.saturating_add(1);
        player.hero.utility.recall_sequence = player.hero.utility.recall_sequence.saturating_add(1);
        cancel(player);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command;
    use shared::{
        HeroClass,
        map::Team,
        utility::UtilityAction,
        wire::{CharacterChoice, ClientPacket, TargetId, TargetKind},
    };
    use std::net::SocketAddr;

    fn address() -> SocketAddr {
        "127.0.0.1:56399".parse().unwrap()
    }
    fn fixture(team: Team) -> (GameWorld, Instant) {
        let now = Instant::now();
        let mut world = GameWorld::empty();
        world.ensure_connected(address(), now);
        crate::session::handle_join_request(
            world.players.get_mut(&address()).unwrap(),
            team,
            CharacterChoice::Ipfs,
            HeroClass::Dawnweaver,
            None,
            &world.map_layout,
            now,
        );
        let p = world.players.get_mut(&address()).unwrap();
        p.hero.x = 0.0;
        p.hero.z = 0.0;
        p.hero.hp = 70.0;
        p.hero.mana = 20.0;
        world.game_state = GameState::Running;
        (world, now)
    }
    fn utility(world: &mut GameWorld, action: UtilityAction, id: u64, now: Instant) {
        command::apply(
            world,
            address(),
            &ClientPacket::Utility {
                action,
                direction: [1.0, 0.0],
                server_epoch: 1,
                match_id: 1,
                request_id: id,
            },
            1,
            1,
            now,
        );
    }
    #[test]
    fn recall_completes_at_seven_seconds_at_each_teams_base_and_rejects_old_transforms() {
        for team in [Team::Green, Team::Blue] {
            let (mut w, now) = fixture(team);
            w.structures = crate::world::build_configured_structures(&w.map_config);
            utility(&mut w, UtilityAction::Recall, 1, now);
            let view = w.players[&address()].owner_view(now, &w.map_layout, &w.game_state);
            assert_eq!(view.utility.recall_remaining_secs, 7.0);
            tick(&mut w, now + Duration::from_millis(6999));
            assert_eq!(w.players[&address()].hero.x, 0.0);
            utility(
                &mut w,
                UtilityAction::Recall,
                2,
                now + Duration::from_secs(4),
            );
            tick(&mut w, now + Duration::from_secs(7));
            let expected =
                crate::world::spawn_position_for_team_from_base(&w.structures, &w.map_layout, team);
            let p = &w.players[&address()];
            assert_eq!(
                [p.hero.x, p.hero.y, p.hero.z],
                [expected.x, PLAYER_GROUND_Y, expected.z]
            );
            assert_eq!((p.hero.hp, p.hero.mana), (70.0, 20.0));
            assert_eq!(
                (p.hero.utility.dash_sequence, p.hero.utility.recall_sequence),
                (1, 1)
            );
            assert_eq!(remaining(p, now + Duration::from_secs(7)), 0.0);
            command::apply(
                &mut w,
                address(),
                &ClientPacket::Transform {
                    x: 0.0,
                    y: 99.0,
                    z: 0.0,
                    yaw: 0.0,
                    dash_sequence: 0,
                },
                1,
                1,
                now + Duration::from_secs(8),
            );
            assert_eq!(w.players[&address()].hero.x, expected.x);
        }
    }
    #[test]
    fn recall_heartbeat_does_not_cancel_but_movement_intent_damage_and_death_do() {
        let (mut w, now) = fixture(Team::Green);
        utility(&mut w, UtilityAction::Recall, 1, now);
        command::apply(
            &mut w,
            address(),
            &ClientPacket::Transform {
                x: 0.0,
                y: 0.5,
                z: 0.0,
                yaw: 1.0,
                dash_sequence: 0,
            },
            1,
            1,
            now + Duration::from_secs(1),
        );
        assert_eq!(
            remaining(&w.players[&address()], now + Duration::from_secs(1)),
            6.0
        );
        command::apply(
            &mut w,
            address(),
            &ClientPacket::Transform {
                x: 1.0,
                y: 0.5,
                z: 0.0,
                yaw: 1.0,
                dash_sequence: 0,
            },
            1,
            1,
            now + Duration::from_secs(2),
        );
        assert!(w.players[&address()].timers.recall.is_none());
        utility(
            &mut w,
            UtilityAction::Recall,
            2,
            now + Duration::from_secs(2),
        );
        let id = w.players[&address()].hero.identity.id;
        crate::combat_feedback::apply_player_damage(
            &mut w.players,
            id,
            1.0,
            now + Duration::from_secs(3),
        );
        assert!(w.players[&address()].timers.recall.is_none());
        utility(
            &mut w,
            UtilityAction::Recall,
            3,
            now + Duration::from_secs(3),
        );
        w.players.get_mut(&address()).unwrap().hero.hp = 0.0;
        tick(&mut w, now + Duration::from_secs(10));
        assert!(w.players[&address()].timers.recall.is_none());
        assert_eq!(w.players[&address()].hero.utility.recall_sequence, 0);
    }
    #[test]
    fn recall_cancels_on_attack_cast_utility_or_explicit_cancel_and_replays_cannot_restart_it() {
        let (mut w, now) = fixture(Team::Blue);
        let actions = [
            ClientPacket::BasicAttack {
                target: TargetId {
                    kind: TargetKind::Player,
                    id: 999,
                },
                request_id: 1,
                server_epoch: 1,
                match_id: 1,
            },
            ClientPacket::CastSkill {
                slot: 0,
                aim: [10.0, 0.0],
                request_id: 1,
                server_epoch: 1,
                match_id: 1,
            },
            ClientPacket::Cast {
                target: TargetId {
                    kind: TargetKind::Player,
                    id: 999,
                },
                slot: 1,
            },
        ];
        for (index, action) in actions.iter().enumerate() {
            utility(&mut w, UtilityAction::Recall, index as u64 + 1, now);
            assert!(w.players[&address()].timers.recall.is_some());
            command::apply(&mut w, address(), action, 1, 1, now);
            assert!(w.players[&address()].timers.recall.is_none());
        }
        for (index, action) in [
            UtilityAction::Dash,
            UtilityAction::Haste,
            UtilityAction::CancelRecall,
        ]
        .iter()
        .enumerate()
        {
            let request = 10 + index as u64 * 2;
            utility(&mut w, UtilityAction::Recall, request, now);
            utility(&mut w, *action, request + 1, now);
            assert!(w.players[&address()].timers.recall.is_none());
            utility(&mut w, UtilityAction::Recall, request, now);
            assert!(w.players[&address()].timers.recall.is_none());
        }
    }
    #[test]
    fn recall_rejects_wrong_round_prejoin_dead_and_finished_matches() {
        let (mut w, now) = fixture(Team::Green);
        let request = ClientPacket::Utility {
            action: UtilityAction::Recall,
            direction: [0.0; 2],
            server_epoch: 2,
            match_id: 1,
            request_id: 1,
        };
        command::apply(&mut w, address(), &request, 1, 1, now);
        assert_eq!(w.players[&address()].hero.utility.last_request_id, 0);
        for state in 0..3 {
            let p = w.players.get_mut(&address()).unwrap();
            p.joined = state != 0;
            p.hero.hp = if state == 1 { 0.0 } else { 100.0 };
            w.game_state = if state == 2 {
                GameState::Victory {
                    winner: Team::Green,
                }
            } else {
                GameState::Running
            };
            utility(&mut w, UtilityAction::Recall, state + 1, now);
            assert!(w.players[&address()].timers.recall.is_none());
        }
        w.game_state = GameState::Running;
        utility(&mut w, UtilityAction::Recall, 10, now);
        w.game_state = GameState::Lobby;
        tick(&mut w, now + Duration::from_secs(8));
        assert!(w.players[&address()].timers.recall.is_none());
        assert_eq!(w.players[&address()].hero.utility.recall_sequence, 0);
    }
}
