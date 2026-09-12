//! Presentation consumes accepted server hits, never HP deltas or projectile disappearance.
use bevy::prelude::*;
use bevy::ui::FocusPolicy;
use shared::combat::{CombatEntityKind, CombatEvent};

use crate::{
    camera::MainCamera,
    combat_visuals::CombatVisualRegistry,
    net::{
        GameStateSnapshot, NetworkAvatar, NetworkHeroClass, NetworkPlayerId, NetworkSpriteCharacter,
    },
    player::Player,
    sprite::PlayerVisualMode,
    world2d::{layer, simulation_xz_to_render_xy},
};

const MAX_HITS: usize = 96;
const MAX_NUMBERS: usize = 48;
const NUMBER_LIFETIME: f32 = 0.95;

pub struct CombatFeedbackPlugin;
impl Plugin for CombatFeedbackPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CombatFeedback>()
            .add_systems(
                Update,
                collect_hits.after(crate::net::ClientNetPipeline::ApplySnapshot),
            )
            .add_systems(
                PostUpdate,
                (place_numbers, draw_impacts)
                    .after(bevy::camera::CameraUpdateSystems)
                    .before(bevy::ui::UiSystems::Layout),
            );
    }
}

#[derive(Default)]
struct HitCursor {
    round: Option<(u64, u64)>,
    high_water: u64,
}
impl HitCursor {
    fn accept(&mut self, round: (u64, u64), events: &[CombatEvent]) -> (bool, Vec<CombatEvent>) {
        let changed = self.round != Some(round);
        let latest = events.iter().map(|event| event.id).max().unwrap_or(0);
        if changed {
            self.round = Some(round);
            self.high_water = latest;
            // A new connection/round gets a baseline, not a replay of retained history.
            return (true, Vec::new());
        }
        let previous = self.high_water;
        self.high_water = self.high_water.max(latest);
        let mut accepted = Vec::new();
        for event in events.iter().take(MAX_HITS) {
            if event.id <= previous
                || accepted
                    .iter()
                    .any(|seen: &CombatEvent| seen.id == event.id)
            {
                continue;
            }
            if !event.amount.is_finite()
                || event.amount <= 0.0
                || !Vec3::new(event.x, event.y, event.z).is_finite()
                || event.target.kind == CombatEntityKind::Unknown
            {
                continue;
            }
            accepted.push(event.clone());
        }
        (false, accepted)
    }
}

struct Impact {
    position: Vec3,
    age: f32,
    lifetime: f32,
    scale: f32,
    color: Color,
}
#[derive(Resource, Default)]
struct CombatFeedback {
    cursor: HitCursor,
    impacts: Vec<Impact>,
}
#[derive(Component)]
pub(crate) struct DamageNumber {
    pub(crate) event_id: u64,
    position: Vec3,
    age: f32,
    lane: f32,
    color: Color,
}

fn collect_hits(
    mut commands: Commands,
    time: Res<Time>,
    snapshot: Res<GameStateSnapshot>,
    registry: Res<CombatVisualRegistry>,
    mut feedback: ResMut<CombatFeedback>,
    numbers: Query<Entity, With<DamageNumber>>,
    heroes: Query<(
        &NetworkPlayerId,
        &NetworkHeroClass,
        Option<&NetworkAvatar>,
        Option<&NetworkSpriteCharacter>,
    )>,
    local: Query<&NetworkPlayerId, With<Player>>,
    cameras: Query<(&Camera, &Transform), With<MainCamera>>,
    mode: Res<PlayerVisualMode>,
) {
    feedback.impacts.retain_mut(|impact| {
        impact.age += time.delta_secs();
        impact.age < impact.lifetime
    });
    let (changed, events) = feedback.cursor.accept(
        (snapshot.meta.server_epoch, snapshot.meta.match_id),
        &snapshot.combat_events,
    );
    let mut number_count = numbers.iter().count();
    if changed {
        feedback.impacts.clear();
        for entity in &numbers {
            commands.entity(entity).despawn();
        }
        number_count = 0;
    }
    let Ok(local_id) = local.single() else {
        return;
    };
    for event in events {
        let position = Vec3::new(event.x, event.y, event.z);
        // Cull against the viewed battle, including free-camera/minimap focus.
        let render = if *mode == PlayerVisualMode::Models3d {
            position + Vec3::Y * 2.0
        } else {
            simulation_xz_to_render_xy(position).extend(layer::OVERHEAD)
        };
        if !cameras.single().is_ok_and(|(camera, transform)| {
            let Some(size) = camera.logical_viewport_size() else {
                return false;
            };
            camera
                .world_to_viewport(&GlobalTransform::from(*transform), render)
                .is_ok_and(|p| p.x >= 0.0 && p.y >= 0.0 && p.x <= size.x && p.y <= size.y)
        }) {
            continue;
        }
        let owner = (event.source.kind == CombatEntityKind::Player)
            .then(|| heroes.iter().find(|(id, _, _, _)| id.0 == event.source.id))
            .flatten();
        let profile = registry.resolve(
            owner.map(|(_, class, _, _)| class.0),
            event.style,
            event.action_slot,
            owner.and_then(|(_, _, avatar, _)| avatar.and_then(|v| v.0.as_deref())),
            owner.and_then(|(_, _, _, sprite)| sprite.and_then(|v| v.0.as_deref())),
        );
        if feedback.impacts.len() == MAX_HITS {
            feedback.impacts.remove(0);
        }
        feedback.impacts.push(Impact {
            position,
            age: 0.0,
            lifetime: profile.impact.lifetime.clamp(0.08, 1.2),
            scale: profile.impact.scale.clamp(0.1, 2.0),
            color: profile.impact.color(),
        });
        if number_count >= MAX_NUMBERS {
            continue;
        }
        let incoming =
            event.target.kind == CombatEntityKind::Player && event.target.id == local_id.0;
        let outgoing =
            event.source.kind == CombatEntityKind::Player && event.source.id == local_id.0;
        let color = if incoming {
            Color::srgb(1.0, 0.28, 0.23)
        } else if outgoing {
            Color::srgb(1.0, 0.9, 0.45)
        } else {
            Color::srgb(0.85, 0.9, 1.0)
        };
        let text = if event.amount < 1.0 {
            format!("{:.1}", event.amount)
        } else {
            format!("{:.0}", event.amount)
        };
        commands.spawn((
            Name::new("ConfirmedDamageNumber"),
            DamageNumber {
                event_id: event.id,
                position,
                age: 0.0,
                lane: (event.id % 5) as f32 - 2.0,
                color,
            },
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                ..default()
            },
            Text::new(text),
            TextFont {
                font_size: if outgoing || incoming { 23.0 } else { 16.0 },
                ..default()
            },
            TextColor(color),
            TextShadow {
                offset: Vec2::splat(1.0),
                color: Color::srgba(0.0, 0.0, 0.0, 0.8),
            },
            FocusPolicy::Pass,
            ZIndex(45),
        ));
        number_count += 1;
    }
}

fn place_numbers(
    mut commands: Commands,
    time: Res<Time>,
    mode: Res<PlayerVisualMode>,
    camera: Query<(&Camera, &Transform), With<MainCamera>>,
    mut numbers: Query<(
        Entity,
        &mut DamageNumber,
        &mut Node,
        &mut TextColor,
        &mut TextShadow,
    )>,
) {
    let camera = camera.single().ok();
    for (entity, mut number, mut node, mut color, mut shadow) in &mut numbers {
        number.age += time.delta_secs();
        if number.age >= NUMBER_LIFETIME {
            commands.entity(entity).despawn();
            continue;
        }
        let progress = number.age / NUMBER_LIFETIME;
        let position = if *mode == PlayerVisualMode::Models3d {
            number.position + Vec3::Y * 2.0
        } else {
            simulation_xz_to_render_xy(number.position).extend(layer::OVERHEAD)
        };
        let screen = camera.and_then(|(camera, transform)| {
            // MainCamera is a root entity. Use its current local transform before UI
            // layout; Bevy propagates GlobalTransform only after that layout.
            let screen = camera
                .world_to_viewport(&GlobalTransform::from(*transform), position)
                .ok()?;
            let size = camera.logical_viewport_size()?;
            (screen.x > 8.0
                && screen.y > 8.0
                && screen.x < size.x - 30.0
                && screen.y < size.y - 30.0)
                .then_some(screen)
        });
        if let Some(screen) = screen {
            node.display = Display::Flex;
            node.left = Val::Px(screen.x - 12.0 + number.lane * (5.0 + 8.0 * progress));
            node.top = Val::Px(screen.y - 25.0 - 42.0 * progress);
            let opacity = ((1.0 - progress) * 3.0).min(1.0);
            color.0 = number.color.with_alpha(opacity);
            shadow.color = Color::srgba(0.0, 0.0, 0.0, 0.8 * opacity);
        } else {
            node.display = Display::None;
        }
    }
}

fn draw_impacts(mut gizmos: Gizmos, mode: Res<PlayerVisualMode>, feedback: Res<CombatFeedback>) {
    for impact in &feedback.impacts {
        let progress = impact.age / impact.lifetime;
        let color = impact.color.with_alpha(1.0 - progress);
        let radius = (0.18 + progress * 0.75) * impact.scale;
        if *mode == PlayerVisualMode::Models3d {
            let center = impact.position + Vec3::Y * 0.75;
            for direction in [
                Vec3::X,
                Vec3::Y,
                Vec3::Z,
                Vec3::new(0.7, 0.7, 0.0),
                Vec3::new(-0.7, 0.7, 0.0),
            ] {
                gizmos.line(
                    center + direction * radius * 0.4,
                    center + direction * radius,
                    color,
                );
                gizmos.line(
                    center - direction * radius * 0.4,
                    center - direction * radius,
                    color,
                );
            }
        } else {
            let center = simulation_xz_to_render_xy(impact.position).extend(layer::VFX);
            gizmos.circle(Isometry3d::from_translation(center), radius, color);
            for direction in [Vec3::X, Vec3::Y] {
                gizmos.line(
                    center - direction * radius * 1.4,
                    center + direction * radius * 1.4,
                    color,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hit(id: u64, amount: f32) -> CombatEvent {
        CombatEvent {
            id,
            amount,
            target: shared::combat::CombatEntity {
                kind: CombatEntityKind::Minion,
                id: 7,
            },
            ..default()
        }
    }
    #[test]
    fn retained_history_is_not_replayed_and_duplicates_are_suppressed() {
        let mut cursor = HitCursor::default();
        assert!(cursor.accept((1, 1), &[hit(1, 10.0)]).1.is_empty());
        assert_eq!(
            cursor
                .accept(
                    (1, 1),
                    &[hit(1, 10.0), hit(3, 8.0), hit(2, 6.0), hit(3, 8.0)]
                )
                .1
                .len(),
            2
        );
        assert!(
            cursor
                .accept((1, 1), &[hit(2, 6.0), hit(3, 8.0)])
                .1
                .is_empty()
        );
        assert!(cursor.accept((1, 2), &[hit(1, 4.0)]).1.is_empty());
        assert_eq!(cursor.accept((1, 2), &[hit(2, 5.0)]).1.len(), 1);
        assert!(cursor.accept((2, 1), &[hit(1, 6.0)]).1.is_empty());
    }
    #[test]
    fn invalid_feedback_cannot_create_labels_and_feed_is_bounded() {
        let mut cursor = HitCursor::default();
        cursor.accept((1, 1), &[]);
        assert!(
            cursor
                .accept((1, 1), &[hit(1, f32::NAN), hit(2, -1.0), hit(3, 0.0)])
                .1
                .is_empty()
        );
        let mut bad = hit(4, 12.0);
        bad.x = f32::INFINITY;
        assert!(cursor.accept((1, 1), &[bad]).1.is_empty());
        let events = (5..205).map(|id| hit(id, 7.0)).collect::<Vec<_>>();
        assert_eq!(cursor.accept((1, 1), &events).1.len(), MAX_HITS);
        assert!(cursor.accept((1, 1), &events).1.is_empty());
    }
}
