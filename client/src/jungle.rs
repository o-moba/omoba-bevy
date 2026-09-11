//! Ordinary jungle presentation; authoritative entities own all health and life cycles.
use bevy::prelude::*;

use crate::{
    creatures3d::{CreatureAssets, ProceduralCreature, spawn_creature},
    maps::MapLayout,
    model_scale::NormalizeModelScale,
    net::{NetworkNeutral, NetworkNeutralCampType, NeutralCampType},
    sprite::PlayerVisualMode,
};

pub struct JungleVisualsPlugin;
impl Plugin for JungleVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, attach_jungle_creatures)
            .add_systems(
                PostUpdate,
                ground_jungle_creatures.before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

fn creature_height_scale(kind: NeutralCampType) -> f32 {
    match kind {
        NeutralCampType::Skirmisher => 1.7,
        NeutralCampType::Bruiser => 2.2,
        _ => 1.6,
    }
}

fn attach_jungle_creatures(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    assets: Option<Res<CreatureAssets>>,
    roots: Query<
        (Entity, &NetworkNeutralCampType),
        (With<NetworkNeutral>, Without<ProceduralCreature>),
    >,
) {
    if *mode != PlayerVisualMode::Models3d {
        return;
    }
    let Some(assets) = assets else {
        return;
    };
    for (entity, kind) in &roots {
        if kind.0.is_boss() {
            continue;
        }
        commands
            .entity(entity)
            .insert(NormalizeModelScale::scaled_by(creature_height_scale(
                kind.0,
            )));
        spawn_creature(
            &mut commands,
            entity,
            ProceduralCreature::Jungle(kind.0),
            &assets,
        );
    }
}

pub(crate) fn ground_jungle_creatures(
    layout: Res<MapLayout>,
    mode: Res<PlayerVisualMode>,
    mut roots: Query<
        (
            &mut Transform,
            &NormalizeModelScale,
            &NetworkNeutralCampType,
        ),
        With<NetworkNeutral>,
    >,
) {
    if *mode != PlayerVisualMode::Models3d {
        return;
    }
    for (mut transform, scale, kind) in &mut roots {
        if kind.0.is_boss() {
            continue;
        }
        transform.translation.y = layout
            .terrain_height_3d(transform.translation.x, transform.translation.z)
            - scale.foot_local_y().unwrap_or(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        combat::CombatStats,
        creatures3d::test_app,
        model_scale::CREATURE_MODEL_TARGET_HEIGHT,
        net::{NetworkNeutralId, NeutralAiState, NeutralAiStateTag},
    };

    fn spawn(app: &mut App, kind: NeutralCampType) -> Entity {
        app.world_mut()
            .spawn((
                NetworkNeutral,
                NetworkNeutralId(50),
                NetworkNeutralCampType(kind),
                NeutralAiStateTag(NeutralAiState::Idle),
                Transform::from_xyz(0.0, 0.5, 0.0),
                Visibility::default(),
                CombatStats {
                    hp: 72.0,
                    max_hp: 72.0,
                    ..default()
                },
            ))
            .id()
    }

    #[test]
    fn jungle_respawns_keep_distinct_silhouettes_grounded_feet_and_bounded_resources() {
        let mut app = test_app(PlayerVisualMode::Models3d);
        app.init_resource::<MapLayout>()
            .add_plugins(JungleVisualsPlugin);
        for _ in 0..3 {
            let roots = [
                NeutralCampType::Skirmisher,
                NeutralCampType::Bruiser,
                NeutralCampType::Spitter,
            ]
            .map(|kind| (spawn(&mut app, kind), kind));
            for _ in 0..8 {
                app.update();
            }
            assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 3);
            assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 5);
            for (entity, kind) in roots {
                let root = app.world().entity(entity);
                assert!(root.get::<Mesh3d>().is_none());
                assert_eq!(root.get::<CombatStats>().unwrap().hp, 72.0);
                let scale = root.get::<NormalizeModelScale>().unwrap();
                let transform = root.get::<Transform>().unwrap();
                assert!((transform.translation.y + scale.foot_local_y().unwrap()).abs() < 0.001);
                assert!(
                    (scale.head_local_y.unwrap()
                        - scale.foot_local_y().unwrap()
                        - CREATURE_MODEL_TARGET_HEIGHT * creature_height_scale(kind))
                    .abs()
                        < 0.001
                );
                let expected = match kind {
                    NeutralCampType::Skirmisher => "leaf-crest",
                    NeutralCampType::Bruiser => "boulder-fist",
                    _ => "spitter-muzzle",
                };
                let children = root.get::<Children>().unwrap();
                assert!(children.iter().any(|child| {
                    app.world()
                        .entity(child)
                        .get::<Name>()
                        .unwrap()
                        .as_str()
                        .ends_with(expected)
                }));
                assert!(
                    children.iter().all(|child| app
                        .world()
                        .entity(child)
                        .get::<Mesh3d>()
                        .is_some())
                );
                app.world_mut()
                    .entity_mut(entity)
                    .insert(NeutralAiStateTag(NeutralAiState::Aggro));
            }
            app.update();
            for (entity, _) in roots {
                app.world_mut().despawn(entity);
            }
            app.update();
            assert_eq!(
                app.world_mut()
                    .query::<&crate::creatures3d::CreaturePart>()
                    .iter(app.world())
                    .count(),
                0
            );
        }
    }

    #[test]
    fn sprite_jungle_keeps_authoritative_roots_without_allocating_3d_assets() {
        let mut app = test_app(PlayerVisualMode::Sprite2d);
        app.init_resource::<MapLayout>()
            .add_plugins(JungleVisualsPlugin);
        let root = spawn(&mut app, NeutralCampType::Bruiser);
        app.update();
        assert!(app.world().entity(root).get::<NetworkNeutral>().is_some());
        assert!(
            app.world()
                .entity(root)
                .get::<ProceduralCreature>()
                .is_none()
        );
        assert!(!app.world().contains_resource::<CreatureAssets>());
        assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 0);
    }
}
