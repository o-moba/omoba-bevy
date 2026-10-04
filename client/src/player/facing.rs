//! Accepted attacks own visual yaw briefly; movement still owns position and trajectory.
use crate::{
    combat::CombatStats,
    net::{
        GameStateSnapshot, NetworkPlayerId, PlayerActionFacing, PlayerCosmeticAction, PlayerLoadout,
    },
};
use bevy::prelude::*;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct FacingState {
    sequence: Option<u64>,
    yaw: Option<f32>,
    remaining: f32,
}
impl FacingState {
    fn update(
        &mut self,
        action: PlayerCosmeticAction,
        facing: PlayerActionFacing,
        alive: bool,
        delta: f32,
        preparing: bool,
    ) -> Option<f32> {
        self.remaining = (self.remaining - delta.max(0.0)).max(0.0);
        if self
            .sequence
            .is_some_and(|previous| action.sequence > previous)
        {
            self.yaw = (facing.sequence == action.sequence)
                .then_some(facing.yaw)
                .flatten()
                .filter(|v| v.is_finite());
            self.remaining = if action.slot == shared::BASIC_ATTACK_ACTION_SLOT {
                0.45
            } else {
                0.7
            };
        }
        self.sequence = Some(self.sequence.unwrap_or(0).max(action.sequence));
        if !alive {
            self.yaw = None;
            self.remaining = 0.0;
        }
        if preparing && alive && self.yaw.is_some() {
            self.remaining = self.remaining.max(0.35);
        }
        (self.remaining > 0.0).then_some(self.yaw).flatten()
    }
}

pub(super) fn face_confirmed_actions(
    time: Res<Time>,
    game: Option<Res<GameStateSnapshot>>,
    mode: Option<Res<crate::sprite::PlayerVisualMode>>,
    mut clock: Local<super::motion::SandboxVisualClock>,
    mut state: Local<(Option<(u64, u64)>, HashMap<Entity, FacingState>)>,
    mut actors: Query<(
        Entity,
        &mut Transform,
        &CombatStats,
        &PlayerCosmeticAction,
        &PlayerActionFacing,
        Option<&NetworkPlayerId>,
        Option<&PlayerLoadout>,
        Option<&crate::net::AuthoritativePlayerYaw>,
    )>,
) {
    if mode.is_some_and(|m| *m == crate::sprite::PlayerVisualMode::Sprite2d) {
        state.1.clear();
        return;
    }
    let round = game
        .as_ref()
        .map(|g| (g.meta.server_epoch, g.meta.match_id));
    if state.0 != round {
        state.0 = round;
        state.1.clear();
    }
    state.1.retain(|entity, _| actors.contains(*entity));
    let delta = clock.delta(&time, game.as_deref());
    for (entity, mut pose, stats, action, facing, id, loadout, authoritative_yaw) in &mut actors {
        if loadout
            .and_then(|loadout| loadout.0.as_ref())
            .is_some_and(|loadout| loadout.stun_remaining_secs > 0.0)
        {
            // Bluff turns a victim without granting them a new attack sequence.
            // Clear the old action lock so it cannot turn them back after the stun.
            state.1.remove(&entity);
            if let Some(yaw) = authoritative_yaw
                .map(|yaw| yaw.0)
                .filter(|yaw| yaw.is_finite())
            {
                pose.rotation = Quat::from_rotation_y(yaw);
            }
            continue;
        }
        let skill = crate::skill_presentation::equipped_skill(
            loadout.and_then(|l| l.0.as_ref()),
            action.slot,
        );
        let preparing = game.as_ref().is_some_and(|g| {
            g.skill_effects.iter().any(|e| {
                Some(e.owner_id) == id.map(|id| id.0)
                    && Some(e.skill) == skill
                    && e.kind == shared::loadout::EffectVisualKind::BeamWarning
                    && matches!(
                        e.skill,
                        shared::loadout::SkillId::DawnRay | shared::loadout::SkillId::HorizonWave
                    )
            })
        });
        if let Some(yaw) = state.1.entry(entity).or_default().update(
            *action,
            *facing,
            stats.is_alive(),
            delta,
            preparing,
        ) {
            // Hero -Z forward is shared with path following and semantic VRM alignment.
            pose.rotation = Quat::from_rotation_y(yaw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stun_uses_replicated_victim_yaw_and_clears_old_attack_facing() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<GameStateSnapshot>()
            .add_systems(PostUpdate, face_confirmed_actions);
        let actor = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                CombatStats::default(),
                NetworkPlayerId(7),
                PlayerCosmeticAction::default(),
                PlayerActionFacing::default(),
                PlayerLoadout(Some(shared::loadout::LoadoutState::default())),
                crate::net::AuthoritativePlayerYaw(1.5),
            ))
            .id();
        app.update();
        app.world_mut().entity_mut(actor).insert((
            PlayerCosmeticAction {
                sequence: 1,
                slot: 0,
                kind: shared::PlayerActionKind::Attack,
            },
            PlayerActionFacing {
                sequence: 1,
                yaw: Some(-1.0),
            },
        ));
        app.update();
        // The last attack points east; movement has since turned north. A
        // normal stun must retain current authoritative yaw, just like Bluff.
        app.world_mut()
            .get_mut::<PlayerLoadout>(actor)
            .unwrap()
            .0
            .as_mut()
            .unwrap()
            .stun_remaining_secs = 1.0;
        app.update();
        assert!(
            app.world()
                .get::<Transform>(actor)
                .unwrap()
                .rotation
                .angle_between(Quat::from_rotation_y(1.5))
                < 0.001
        );
        app.world_mut()
            .get_mut::<PlayerLoadout>(actor)
            .unwrap()
            .0
            .as_mut()
            .unwrap()
            .stun_remaining_secs = 0.0;
        app.update();
        assert!(
            app.world()
                .get::<Transform>(actor)
                .unwrap()
                .rotation
                .angle_between(Quat::from_rotation_y(1.5))
                < 0.001
        );
    }
    #[test]
    fn local_and_remote_actor_turn_after_locomotion_without_moving_their_positions() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<GameStateSnapshot>()
            .add_systems(PostUpdate, face_confirmed_actions);
        let actors: Vec<_> = [1.0, -1.0]
            .into_iter()
            .map(|direction| {
                let entity = app
                    .world_mut()
                    .spawn((
                        Transform::from_xyz(direction * 4.0, 0.0, 2.0),
                        CombatStats::default(),
                        PlayerCosmeticAction::default(),
                        PlayerActionFacing::default(),
                    ))
                    .id();
                (entity, direction)
            })
            .collect();
        app.world_mut()
            .entity_mut(actors[0].0)
            .insert(crate::player::Player);
        app.world_mut()
            .entity_mut(actors[1].0)
            .insert(crate::net::RemotePlayer);
        app.update();
        for (entity, direction) in &actors {
            app.world_mut().entity_mut(*entity).insert((
                PlayerCosmeticAction {
                    sequence: 1,
                    slot: 0,
                    kind: shared::PlayerActionKind::Attack,
                },
                PlayerActionFacing {
                    sequence: 1,
                    yaw: Some(shared::math::hero_yaw_towards(*direction, 0.0)),
                },
            ));
        }
        for _ in 0..3 {
            // Simulate both local steering and network interpolation writing the old movement yaw.
            for (entity, _) in &actors {
                app.world_mut()
                    .get_mut::<Transform>(*entity)
                    .unwrap()
                    .rotation = Quat::IDENTITY;
            }
            app.update();
            for (entity, direction) in &actors {
                let pose = app.world().get::<Transform>(*entity).unwrap();
                assert!((pose.rotation * Vec3::NEG_Z).dot(Vec3::X * *direction) > 0.999);
                assert_eq!(pose.translation, Vec3::new(direction * 4.0, 0.0, 2.0));
            }
        }
    }
    #[test]
    fn action_facing_survives_movement_but_not_death_or_duplicate_replay() {
        let mut state = FacingState::default();
        let mut action = PlayerCosmeticAction {
            sequence: 10,
            slot: 0,
            kind: shared::PlayerActionKind::Attack,
        };
        let mut facing = PlayerActionFacing {
            sequence: 10,
            yaw: Some(1.2),
        };
        assert_eq!(
            state.update(action, facing, true, 0.01, false),
            None,
            "first snapshot is history"
        );
        action.sequence = 11;
        facing.sequence = 11;
        assert_eq!(state.update(action, facing, true, 0.01, false), Some(1.2));
        assert_eq!(state.update(action, facing, true, 0.2, false), Some(1.2));
        assert_eq!(
            state.update(action, facing, true, 1.0, false),
            None,
            "duplicate never restarts hold"
        );
        action.sequence = 12;
        facing.sequence = 12;
        assert_eq!(state.update(action, facing, true, 0.01, false), Some(1.2));
        assert_eq!(state.update(action, facing, false, 0.01, true), None);
        assert_eq!(
            state.update(action, facing, true, 0.01, false),
            None,
            "respawn does not replay"
        );
    }
    #[test]
    fn windup_holds_facing_and_self_cast_or_bad_data_drops_old_direction() {
        let mut state = FacingState {
            sequence: Some(1),
            ..default()
        };
        let mut action = PlayerCosmeticAction {
            sequence: 2,
            slot: 3,
            kind: shared::PlayerActionKind::Cast,
        };
        let mut facing = PlayerActionFacing {
            sequence: 2,
            yaw: Some(-0.7),
        };
        assert_eq!(state.update(action, facing, true, 0.01, true), Some(-0.7));
        assert_eq!(state.update(action, facing, true, 1.0, true), Some(-0.7));
        action.sequence = 3;
        facing.sequence = 3;
        facing.yaw = None;
        assert_eq!(state.update(action, facing, true, 0.01, false), None);
        action.sequence = 4;
        facing.sequence = 4;
        facing.yaw = Some(f32::NAN);
        assert_eq!(state.update(action, facing, true, 0.01, false), None);
    }
}
