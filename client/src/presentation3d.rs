//! Offline, bounded Models3d combat feedback and allegiance markers.
//!
//! Gizmos are rebuilt each frame: effects own no render entities or assets.
//! A terrain-anchored double circle identifies only the locally controlled hero.
// i18n-strict

use bevy::prelude::*;
use shared::PlayerActionKind;
use std::collections::HashMap;

use crate::combat::CombatStats;
use crate::maps::MapLayout;
use crate::net::{GameStateSnapshot, PlayerCosmeticAction};
use crate::player::Player;
use crate::sprite::{PlayerVisualMode, in_models3d};

const MAX_EFFECTS: usize = 192;

pub struct Presentation3dPlugin;

impl Plugin for Presentation3dPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CombatPresentation>().add_systems(
            PostUpdate,
            (collect_feedback, draw_feedback)
                .chain()
                .run_if(in_models3d()),
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EffectKind {
    Attack,
    Cast,
    Death,
}

impl EffectKind {
    fn lifetime(self) -> f32 {
        match self {
            Self::Attack => 0.24,
            Self::Cast => 0.55,
            Self::Death => 1.1,
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Attack => Color::srgb(1.0, 0.9, 0.55),
            Self::Cast => Color::srgb(0.65, 0.45, 1.0),
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

fn ground_marker_center(position: Vec3, layout: MapLayout) -> Vec3 {
    Vec3::new(
        position.x,
        layout.terrain_height_3d(position.x, position.z) + 0.09,
        position.z,
    )
}

fn draw_feedback(
    mut gizmos: Gizmos,
    mode: Res<PlayerVisualMode>,
    layout: Res<MapLayout>,
    feedback: Res<CombatPresentation>,
    heroes: Query<&Transform, With<Player>>,
) {
    if *mode != PlayerVisualMode::Models3d {
        return;
    }
    for transform in &heroes {
        // Selection belongs only to the controlled actor and follows terrain,
        // never the animated model's vertical offset.
        let center = ground_marker_center(transform.translation, *layout);
        let color = Color::srgb(1.0, 0.90, 0.30);
        ground_circle(&mut gizmos, center, 0.85, color);
        ground_circle(&mut gizmos, center, 1.03, color);
    }

    for effect in &feedback.effects {
        let progress = effect.age / effect.kind.lifetime();
        let color = effect.kind.color().with_alpha(1.0 - progress);
        let center = effect.position + Vec3::Y * 0.8;
        match effect.kind {
            EffectKind::Attack => {
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
    fn local_marker_is_grounded_independent_of_animated_height() {
        let layout = MapLayout::default();
        for point in [Vec3::new(0.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 12.0)] {
            let grounded = ground_marker_center(point, layout);
            assert_eq!(grounded.xz(), point.xz());
            assert_eq!(
                grounded.y,
                layout.terrain_height_3d(point.x, point.z) + 0.09
            );
            for height in [-5.0, 1.0, 9.0] {
                assert_eq!(
                    ground_marker_center(Vec3::new(point.x, height, point.z), layout),
                    grounded
                );
            }
        }
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
            vec![EffectKind::Cast, EffectKind::Death]
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
            .emit(Vec3::ZERO, EffectKind::Cast);
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
