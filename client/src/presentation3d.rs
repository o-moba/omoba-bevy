//! Offline, bounded Models3d combat feedback and allegiance markers.
//!
//! Gizmos are rebuilt each frame: effects own no render entities or assets.
//! Local = double circle, ally = square, enemy = triangle, in addition to color.

use bevy::prelude::*;
use shared::PlayerActionKind;
use std::collections::HashMap;

use crate::combat::CombatStats;
use crate::maps::MapLayout;
use crate::net::{GameStateSnapshot, PlayerCosmeticAction, RemotePlayer};
use crate::player::Player;
use crate::sprite::PlayerVisualMode;
use crate::team::{Team, TeamSelection};

const MAX_EFFECTS: usize = 192;

pub struct Presentation3dPlugin;

impl Plugin for Presentation3dPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CombatPresentation>()
            .add_systems(PostUpdate, (collect_feedback, draw_feedback).chain());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Allegiance {
    Local,
    Friendly,
    Enemy,
}

fn allegiance(local: bool, team: Team, selected: Option<Team>) -> Allegiance {
    if local {
        Allegiance::Local
    } else if selected == Some(team) {
        Allegiance::Friendly
    } else {
        Allegiance::Enemy
    }
}

impl Allegiance {
    fn color(self) -> Color {
        match self {
            Self::Local => Color::srgb(1.0, 0.90, 0.30),
            Self::Friendly => Color::srgb(0.20, 0.90, 1.0),
            Self::Enemy => Color::srgb(1.0, 0.24, 0.15),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EffectKind {
    Attack,
    Cast,
    Impact,
    Heal,
    Death,
}

impl EffectKind {
    fn lifetime(self) -> f32 {
        match self {
            Self::Attack => 0.24,
            Self::Cast => 0.55,
            Self::Impact => 0.30,
            Self::Heal => 0.75,
            Self::Death => 1.1,
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Attack => Color::srgb(1.0, 0.9, 0.55),
            Self::Cast => Color::srgb(0.65, 0.45, 1.0),
            Self::Impact => Color::srgb(1.0, 0.3, 0.12),
            Self::Heal => Color::srgb(0.25, 1.0, 0.45),
            Self::Death => Color::srgb(0.9, 0.9, 1.0),
        }
    }
}

struct Effect {
    position: Vec3,
    kind: EffectKind,
    age: f32,
}

#[derive(Clone, Copy)]
struct Observation {
    hp: f32,
    action_sequence: u64,
}

#[derive(Resource, Default)]
struct CombatPresentation {
    previous: HashMap<Entity, Observation>,
    effects: Vec<Effect>,
    round: Option<(u64, u64)>,
}

impl CombatPresentation {
    fn observe_round(&mut self, round: (u64, u64)) -> bool {
        let changed = self.round.is_some_and(|previous| previous != round);
        if changed {
            self.previous.clear();
            self.effects.clear();
        }
        self.round = Some(round);
        changed
    }

    fn emit(&mut self, position: Vec3, kind: EffectKind) {
        if self.effects.len() == MAX_EFFECTS {
            self.effects.remove(0);
        }
        self.effects.push(Effect {
            position,
            kind,
            age: 0.0,
        });
    }

    fn advance(&mut self, delta: f32) {
        self.effects.retain_mut(|effect| {
            effect.age += delta;
            effect.age < effect.kind.lifetime()
        });
    }

    fn observe(&mut self, entity: Entity, position: Vec3, hp: f32, action: PlayerCosmeticAction) {
        let new = Observation {
            hp,
            action_sequence: action.sequence,
        };
        let Some(old) = self.previous.insert(entity, new) else {
            // Admission/reconnect starts with a baseline, not a replay of history.
            return;
        };
        if old.hp > 0.0 && hp <= 0.0 {
            self.emit(position, EffectKind::Death);
        } else if hp > 0.0 && hp < old.hp {
            self.emit(position, EffectKind::Impact);
        } else if old.hp > 0.0 && hp > old.hp + 0.01 {
            self.emit(position, EffectKind::Heal);
        }
        if hp > 0.0 && old.hp > 0.0 && action.sequence > old.action_sequence {
            match action.kind {
                PlayerActionKind::Attack => self.emit(position, EffectKind::Attack),
                PlayerActionKind::Cast => self.emit(position, EffectKind::Cast),
                PlayerActionKind::None => {}
            }
        }
    }
}

fn collect_feedback(
    time: Res<Time>,
    game_state: Option<Res<GameStateSnapshot>>,
    mode: Res<PlayerVisualMode>,
    mut feedback: ResMut<CombatPresentation>,
    actors: Query<(
        Entity,
        &Transform,
        &CombatStats,
        Option<&PlayerCosmeticAction>,
    )>,
) {
    feedback.advance(time.delta_secs());
    if *mode != PlayerVisualMode::Models3d {
        feedback.previous.clear();
        feedback.effects.clear();
        feedback.round = None;
        return;
    }
    let round_changed = game_state.as_ref().is_some_and(|state| {
        state.meta.server_epoch != 0
            && state.meta.match_id != 0
            && feedback.observe_round((state.meta.server_epoch, state.meta.match_id))
    });
    feedback
        .previous
        .retain(|entity, _| actors.contains(*entity));
    for (entity, transform, stats, action) in &actors {
        if round_changed {
            // The first received packet may already carry action 1: do not
            // require the initial action-0 snapshot to have survived UDP loss.
            feedback.previous.insert(
                entity,
                Observation {
                    hp: stats.hp,
                    action_sequence: 0,
                },
            );
        }
        feedback.observe(
            entity,
            transform.translation,
            stats.hp,
            action.copied().unwrap_or_default(),
        );
    }
}

fn ground_circle(gizmos: &mut Gizmos, center: Vec3, radius: f32, color: Color) {
    gizmos.circle(
        Isometry3d::new(center, Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        radius,
        color,
    );
}

fn draw_feedback(
    mut gizmos: Gizmos,
    mode: Res<PlayerVisualMode>,
    layout: Res<MapLayout>,
    selection: Res<TeamSelection>,
    feedback: Res<CombatPresentation>,
    heroes: Query<(&Transform, &Team, Has<Player>), Or<(With<Player>, With<RemotePlayer>)>>,
) {
    if *mode != PlayerVisualMode::Models3d {
        return;
    }
    for (transform, team, local) in &heroes {
        let mut center = transform.translation;
        center.y = layout.terrain_height(center.x, center.z) + 0.09;
        let cue = allegiance(local, *team, selection.team);
        let color = cue.color();
        let radius = 0.85;
        ground_circle(&mut gizmos, center, radius, color);
        match cue {
            Allegiance::Local => ground_circle(&mut gizmos, center, radius + 0.18, color),
            Allegiance::Friendly => {
                gizmos.linestrip(
                    [
                        center + Vec3::new(-radius, 0.0, -radius),
                        center + Vec3::new(radius, 0.0, -radius),
                        center + Vec3::new(radius, 0.0, radius),
                        center + Vec3::new(-radius, 0.0, radius),
                        center + Vec3::new(-radius, 0.0, -radius),
                    ],
                    color,
                );
            }
            Allegiance::Enemy => {
                gizmos.linestrip(
                    [
                        center + Vec3::new(0.0, 0.0, -1.2),
                        center + Vec3::new(1.05, 0.0, 0.7),
                        center + Vec3::new(-1.05, 0.0, 0.7),
                        center + Vec3::new(0.0, 0.0, -1.2),
                    ],
                    color,
                );
            }
        }
    }
    for effect in &feedback.effects {
        let progress = effect.age / effect.kind.lifetime();
        let color = effect.kind.color().with_alpha(1.0 - progress);
        let center = effect.position + Vec3::Y * 0.8;
        match effect.kind {
            EffectKind::Attack | EffectKind::Impact => {
                let radius = 0.25 + progress * 0.8;
                for direction in [Vec3::X, Vec3::Y, Vec3::Z] {
                    gizmos.line(
                        center - direction * radius,
                        center + direction * radius,
                        color,
                    );
                }
            }
            EffectKind::Cast => ground_circle(&mut gizmos, center, 0.4 + 1.6 * progress, color),
            EffectKind::Heal => {
                let center = center + Vec3::Y * progress;
                gizmos.line(center - Vec3::X * 0.35, center + Vec3::X * 0.35, color);
                gizmos.line(center - Vec3::Y * 0.35, center + Vec3::Y * 0.35, color);
                ground_circle(&mut gizmos, center, 0.5 + 0.3 * progress, color);
            }
            EffectKind::Death => {
                let center = center + Vec3::Y * progress * 1.5;
                ground_circle(&mut gizmos, center, 0.9 * (1.0 - progress), color);
                gizmos.line(center, center + Vec3::Y * 0.7, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allegiance_is_relative_and_local_does_not_depend_on_avatar_or_color() {
        assert_eq!(
            allegiance(true, Team::Blue, Some(Team::Blue)),
            Allegiance::Local
        );
        assert_eq!(
            allegiance(false, Team::Blue, Some(Team::Blue)),
            Allegiance::Friendly
        );
        assert_eq!(
            allegiance(false, Team::Blue, Some(Team::Green)),
            Allegiance::Enemy
        );
        assert_eq!(
            allegiance(false, Team::Green, Some(Team::Blue)),
            Allegiance::Enemy
        );
    }

    #[test]
    fn replicated_combat_feedback_is_deduplicated_and_death_respawn_is_bounded() {
        let mut world = World::new();
        let entity = world.spawn_empty().id();
        let mut feedback = CombatPresentation::default();
        let action = PlayerCosmeticAction {
            sequence: 1,
            kind: PlayerActionKind::Cast,
            slot: 0,
        };
        feedback.observe(entity, Vec3::ZERO, 100.0, PlayerCosmeticAction::default());
        feedback.observe(entity, Vec3::ZERO, 100.0, action);
        feedback.observe(entity, Vec3::ZERO, 100.0, action);
        assert_eq!(feedback.effects.len(), 1);
        assert_eq!(feedback.effects[0].kind, EffectKind::Cast);
        feedback.observe(entity, Vec3::ZERO, 80.0, action);
        feedback.observe(entity, Vec3::ZERO, 90.0, action);
        feedback.observe(entity, Vec3::ZERO, 0.0, action);
        feedback.observe(entity, Vec3::ZERO, 0.0, action);
        feedback.observe(entity, Vec3::ZERO, 100.0, action);
        assert_eq!(
            feedback
                .effects
                .iter()
                .map(|effect| effect.kind)
                .collect::<Vec<_>>(),
            vec![
                EffectKind::Cast,
                EffectKind::Impact,
                EffectKind::Heal,
                EffectKind::Death
            ]
        );
        feedback.advance(2.0);
        assert!(feedback.effects.is_empty());
    }

    #[test]
    fn round_change_clears_old_effects_without_waiting_for_sequence_zero() {
        let mut world = World::new();
        let entity = world.spawn_empty().id();
        let mut feedback = CombatPresentation::default();
        feedback.observe_round((100, 1));
        feedback.observe(
            entity,
            Vec3::ZERO,
            100.0,
            PlayerCosmeticAction {
                sequence: 42,
                ..default()
            },
        );
        feedback.emit(Vec3::ZERO, EffectKind::Death);
        assert!(feedback.observe_round((100, 2)));
        assert!(feedback.effects.is_empty());
        assert!(feedback.previous.is_empty());
        feedback.previous.insert(
            entity,
            Observation {
                hp: 100.0,
                action_sequence: 0,
            },
        );
        feedback.observe(
            entity,
            Vec3::ZERO,
            100.0,
            PlayerCosmeticAction {
                sequence: 1,
                kind: PlayerActionKind::Cast,
                slot: 0,
            },
        );
        assert_eq!(feedback.effects.len(), 1);
        assert_eq!(feedback.effects[0].kind, EffectKind::Cast);
        assert!(!feedback.observe_round((100, 2)));
    }

    #[test]
    fn effect_storage_is_capped_even_during_a_large_burst() {
        let mut feedback = CombatPresentation::default();
        for _ in 0..MAX_EFFECTS * 3 {
            feedback.emit(Vec3::ZERO, EffectKind::Cast);
        }
        assert_eq!(feedback.effects.len(), MAX_EFFECTS);
        feedback.advance(0.6);
        assert!(feedback.effects.is_empty());
    }

    #[test]
    fn observers_are_removed_with_despawned_actors_and_sprite_mode_clears_state() {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .insert_resource(PlayerVisualMode::Models3d)
            .init_resource::<CombatPresentation>()
            .add_systems(Update, collect_feedback);
        let entity = app
            .world_mut()
            .spawn((Transform::default(), CombatStats::default()))
            .id();
        app.update();
        assert_eq!(
            app.world().resource::<CombatPresentation>().previous.len(),
            1
        );
        app.world_mut().despawn(entity);
        app.update();
        assert!(
            app.world()
                .resource::<CombatPresentation>()
                .previous
                .is_empty()
        );
        app.world_mut()
            .resource_mut::<CombatPresentation>()
            .emit(Vec3::ZERO, EffectKind::Heal);
        app.insert_resource(PlayerVisualMode::Sprite2d);
        app.update();
        assert!(
            app.world()
                .resource::<CombatPresentation>()
                .effects
                .is_empty()
        );
    }
}
