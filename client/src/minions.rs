//! Original procedural lane sentinels with state-driven walk/attack motion.

use bevy::prelude::*;

use crate::creatures3d::{
    CreatureAssets, ProceduralCreature, animate_creatures, setup_creature_assets, spawn_creature,
};
use crate::net::NetworkMinion;
use crate::sprite::PlayerVisualMode;
use crate::team::Team;

pub struct MinionVisualsPlugin;

impl Plugin for MinionVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            setup_creature_assets.after(crate::persistence::load_persistent_client_settings),
        )
        .add_systems(Update, (attach_minion_models, animate_creatures));
    }
}

fn attach_minion_models(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    assets: Option<Res<CreatureAssets>>,
    minions: Query<(Entity, &Team), (With<NetworkMinion>, Without<ProceduralCreature>)>,
) {
    if *mode != PlayerVisualMode::Models3d {
        return;
    }
    let Some(assets) = assets else { return };
    for (entity, team) in &minions {
        spawn_creature(
            &mut commands,
            entity,
            ProceduralCreature::Minion(*team),
            &assets,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::CombatStats;
    use crate::creatures3d::test_app;
    use crate::model_scale::{DEFAULT_MODEL_TARGET_HEIGHT, ModelScaleSource, NormalizeModelScale};
    use crate::net::{MinionBrainState, NetworkMinionBrainState};

    fn spawn_minion(app: &mut App, team: Team) -> Entity {
        let mode = *app.world().resource::<PlayerVisualMode>();
        let mut root = app.world_mut().spawn((
            NetworkMinion,
            team,
            NetworkMinionBrainState(MinionBrainState::Marching),
            Transform::default(),
            Visibility::default(),
            CombatStats {
                hp: 17.0,
                max_hp: 25.0,
                ..default()
            },
        ));
        if mode == PlayerVisualMode::Models3d {
            root.insert(NormalizeModelScale::scaled_by(0.6));
        }
        root.id()
    }

    #[test]
    fn procedural_minions_keep_team_silhouettes_scale_motion_and_bounded_wave_resources() {
        let mut app = test_app(PlayerVisualMode::Models3d);
        for _wave in 0..3 {
            let green = spawn_minion(&mut app, Team::Green);
            let blue = spawn_minion(&mut app, Team::Blue);
            for _ in 0..6 {
                app.update();
            }
            assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 3);
            assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 5);
            let mut counts = Vec::new();
            for (entity, crown_name, crown_count) in
                [(green, "jade-crown", 1), (blue, "azure-twin-crown", 2)]
            {
                let root = app.world().entity(entity);
                assert!(root.get::<ModelScaleSource>().is_none());
                assert_eq!(root.get::<CombatStats>().unwrap().hp, 17.0);
                let scale = root.get::<NormalizeModelScale>().unwrap();
                assert!(
                    (scale.head_local_y.unwrap() - DEFAULT_MODEL_TARGET_HEIGHT * 0.6).abs() < 0.001
                );
                assert!(scale.foot_local_y().unwrap().abs() < 0.001);
                let children = root.get::<Children>().unwrap();
                counts.push(children.len());
                assert_eq!(
                    children
                        .iter()
                        .filter(|child| app
                            .world()
                            .entity(*child)
                            .get::<Name>()
                            .unwrap()
                            .as_str()
                            .ends_with(crown_name))
                        .count(),
                    crown_count
                );
                for child in children.iter() {
                    let part = app.world().entity(child);
                    assert!(part.get::<Mesh3d>().is_some());
                    assert_ne!(part.get::<Visibility>(), Some(&Visibility::Hidden));
                }
            }
            let green_arm = app
                .world()
                .entity(green)
                .get::<Children>()
                .unwrap()
                .iter()
                .find(|child| {
                    app.world()
                        .entity(*child)
                        .get::<Name>()
                        .unwrap()
                        .as_str()
                        .ends_with("-arm")
                })
                .unwrap();
            let before = *app.world().entity(green_arm).get::<Transform>().unwrap();
            for frame in 0..100 {
                let state = if frame % 2 == 0 {
                    MinionBrainState::Attacking
                } else {
                    MinionBrainState::Chasing
                };
                app.world_mut()
                    .entity_mut(green)
                    .insert(NetworkMinionBrainState(state));
                app.update();
            }
            assert_ne!(
                *app.world().entity(green_arm).get::<Transform>().unwrap(),
                before
            );
            for (i, entity) in [green, blue].into_iter().enumerate() {
                assert_eq!(
                    app.world().entity(entity).get::<Children>().unwrap().len(),
                    counts[i]
                );
            }
            assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 3);
            assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 5);
            app.world_mut().entity_mut(green).despawn();
            app.world_mut().entity_mut(blue).despawn();
            app.update();
            assert_eq!(
                app.world_mut().query::<&Mesh3d>().iter(app.world()).count(),
                0
            );
            assert_eq!(
                app.world_mut()
                    .query::<&ProceduralCreature>()
                    .iter(app.world())
                    .count(),
                0
            );
        }
    }

    #[test]
    fn sprite2d_minions_do_not_allocate_or_attach_procedural_3d_assets() {
        let mut app = test_app(PlayerVisualMode::Sprite2d);
        spawn_minion(&mut app, Team::Green);
        spawn_minion(&mut app, Team::Blue);
        app.update();
        app.update();
        assert!(!app.world().contains_resource::<CreatureAssets>());
        assert_eq!(
            app.world_mut()
                .query::<&NormalizeModelScale>()
                .iter(app.world())
                .count(),
            0
        );
        assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 0);
        assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 0);
        assert_eq!(
            app.world_mut().query::<&Mesh3d>().iter(app.world()).count(),
            0
        );
        assert_eq!(
            app.world_mut()
                .query::<&ProceduralCreature>()
                .iter(app.world())
                .count(),
            0
        );
    }
}
