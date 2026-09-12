//! Original procedural lane sentinels with authoritative release poses.

use bevy::prelude::*;

use crate::creatures3d::{
    CreatureAssets, MinionAttackPulse, ProceduralCreature, animate_creatures,
    setup_creature_assets, spawn_creature, update_minion_attack_pulses,
};
use crate::net::{NetworkMinion, NetworkMinionAction, NetworkMinionKind};
use crate::sprite::PlayerVisualMode;
use crate::team::Team;

pub struct MinionVisualsPlugin;

impl Plugin for MinionVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            setup_creature_assets.after(crate::persistence::load_persistent_client_settings),
        )
        .add_systems(
            Update,
            (
                attach_minion_models,
                update_minion_attack_pulses,
                animate_creatures,
            )
                .chain(),
        );
    }
}

#[allow(clippy::type_complexity)]
fn attach_minion_models(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    assets: Option<Res<CreatureAssets>>,
    minions: Query<
        (
            Entity,
            &Team,
            Option<&NetworkMinionKind>,
            Option<&NetworkMinionAction>,
        ),
        (With<NetworkMinion>, Without<ProceduralCreature>),
    >,
) {
    if *mode != PlayerVisualMode::Models3d {
        return;
    }
    let Some(assets) = assets else { return };
    for (entity, team, kind, action) in &minions {
        // Network snapshots always supply the role. Old local visual fixtures
        // may omit it; their original sentinel remains a melee unit.
        let role = kind.map(|kind| kind.0).unwrap_or_default();
        commands
            .entity(entity)
            .insert(MinionAttackPulse::at_sequence(
                action.map_or(0, |action| action.0),
            ));
        spawn_creature(
            &mut commands,
            entity,
            ProceduralCreature::Minion(*team, role),
            &assets,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::CombatStats;
    use crate::creatures3d::test_app;
    use crate::model_scale::{CREATURE_MODEL_TARGET_HEIGHT, ModelScaleSource, NormalizeModelScale};
    use crate::net::{MinionBrainState, NetworkMinionBrainState};
    use shared::combat::MinionKind;

    const TEST_HEIGHT_SCALE: f32 = 0.9;

    fn spawn_minion(app: &mut App, team: Team, kind: MinionKind) -> Entity {
        let mode = *app.world().resource::<PlayerVisualMode>();
        let mut root = app.world_mut().spawn((
            NetworkMinion,
            NetworkMinionKind(kind),
            NetworkMinionAction(0),
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
            root.insert(NormalizeModelScale::scaled_by(TEST_HEIGHT_SCALE));
        }
        root.id()
    }

    fn named_part(app: &App, owner: Entity, suffix: &str) -> Entity {
        app.world()
            .entity(owner)
            .get::<Children>()
            .unwrap()
            .iter()
            .find(|child| {
                app.world()
                    .entity(*child)
                    .get::<Name>()
                    .is_some_and(|name| name.as_str().ends_with(suffix))
            })
            .unwrap()
    }

    fn pose(app: &App, part: Entity) -> Transform {
        *app.world().entity(part).get::<Transform>().unwrap()
    }

    fn tick(app: &mut App, frames: usize) {
        for _ in 0..frames {
            app.update();
        }
    }

    #[test]
    fn mixed_minions_keep_team_role_silhouettes_scale_and_bounded_wave_resources() {
        let mut app = test_app(PlayerVisualMode::Models3d);
        for _wave in 0..3 {
            let mut roots = Vec::new();
            for team in [Team::Green, Team::Blue] {
                for kind in [MinionKind::Melee, MinionKind::Caster] {
                    roots.push((spawn_minion(&mut app, team, kind), team, kind));
                }
            }
            tick(&mut app, 6);
            assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 3);
            assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 5);
            let mut counts = Vec::new();
            for (entity, team, kind) in &roots {
                let root = app.world().entity(*entity);
                assert!(root.get::<ModelScaleSource>().is_none());
                assert_eq!(root.get::<CombatStats>().unwrap().hp, 17.0);
                assert_eq!(
                    root.get::<ProceduralCreature>(),
                    Some(&ProceduralCreature::Minion(*team, *kind))
                );
                let scale = root.get::<NormalizeModelScale>().unwrap();
                assert!(
                    (scale.head_local_y.unwrap()
                        - CREATURE_MODEL_TARGET_HEIGHT * TEST_HEIGHT_SCALE)
                        .abs()
                        < 0.001
                );
                assert!(scale.foot_local_y().unwrap().abs() < 0.001);
                let children = root.get::<Children>().unwrap();
                counts.push(children.len());
                let names = children
                    .iter()
                    .map(|child| {
                        app.world()
                            .entity(child)
                            .get::<Name>()
                            .unwrap()
                            .as_str()
                            .to_owned()
                    })
                    .collect::<Vec<_>>();
                let (crown_name, crown_count) = match team {
                    Team::Green => ("jade-crown", 1),
                    Team::Blue => ("azure-twin-crown", 2),
                };
                assert_eq!(
                    names
                        .iter()
                        .filter(|name| name.ends_with(crown_name))
                        .count(),
                    crown_count
                );
                assert_eq!(
                    names.iter().any(|name| name.ends_with("melee-shield")),
                    *kind == MinionKind::Melee
                );
                assert_eq!(
                    names.iter().any(|name| name.ends_with("melee-blade")),
                    *kind == MinionKind::Melee
                );
                assert_eq!(
                    names.iter().any(|name| name.ends_with("caster-staff")),
                    *kind == MinionKind::Caster
                );
                assert_eq!(
                    names.iter().any(|name| name.ends_with("caster-focus")),
                    *kind == MinionKind::Caster
                );
                for child in children.iter() {
                    let part = app.world().entity(child);
                    assert!(part.get::<Mesh3d>().is_some());
                    assert_ne!(part.get::<Visibility>(), Some(&Visibility::Hidden));
                }
            }
            let melee_torso = pose(&app, named_part(&app, roots[0].0, "-torso"));
            let caster_torso = pose(&app, named_part(&app, roots[1].0, "-torso"));
            assert!(melee_torso.scale.x > caster_torso.scale.x * 1.4);
            let leg = named_part(&app, roots[0].0, "-leg");
            let before_walk = pose(&app, leg);
            tick(&mut app, 10);
            assert_ne!(pose(&app, leg), before_walk);
            for action in 1..=5 {
                for (entity, _, _) in &roots {
                    app.world_mut().entity_mut(*entity).insert((
                        NetworkMinionBrainState(MinionBrainState::Attacking),
                        NetworkMinionAction(action),
                    ));
                }
                tick(&mut app, 30);
            }
            for (i, (entity, _, _)) in roots.iter().enumerate() {
                assert_eq!(
                    app.world().entity(*entity).get::<Children>().unwrap().len(),
                    counts[i]
                );
            }
            assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 3);
            assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 5);
            for (entity, _, _) in roots {
                app.world_mut().entity_mut(entity).despawn();
            }
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
            assert_eq!(
                app.world_mut()
                    .query::<&MinionAttackPulse>()
                    .iter(app.world())
                    .count(),
                0
            );
        }
    }

    #[test]
    fn release_poses_require_new_server_sequence_and_recover_without_replaying_history() {
        let mut app = test_app(PlayerVisualMode::Models3d);
        let melee = spawn_minion(&mut app, Team::Green, MinionKind::Melee);
        let caster = spawn_minion(&mut app, Team::Blue, MinionKind::Caster);
        for entity in [melee, caster] {
            app.world_mut().entity_mut(entity).insert((
                NetworkMinionBrainState(MinionBrainState::Attacking),
                NetworkMinionAction(7),
            ));
        }
        tick(&mut app, 6);
        let blade = named_part(&app, melee, "melee-blade");
        let staff = named_part(&app, caster, "caster-staff");
        let rest_blade = pose(&app, blade);
        let rest_staff = pose(&app, staff);
        // Initial authoritative history and continuous Attacking do not release.
        tick(&mut app, 60);
        assert_eq!(pose(&app, blade), rest_blade);
        assert_eq!(pose(&app, staff), rest_staff);
        for entity in [melee, caster] {
            app.world_mut()
                .entity_mut(entity)
                .insert(NetworkMinionAction(8));
        }
        app.update();
        let striking_blade = pose(&app, blade);
        let casting_staff = pose(&app, staff);
        assert_ne!(striking_blade, rest_blade);
        assert_ne!(casting_staff, rest_staff);
        assert_ne!(striking_blade.rotation, casting_staff.rotation);
        tick(&mut app, 30);
        assert_eq!(pose(&app, blade), rest_blade);
        assert_eq!(pose(&app, staff), rest_staff);
        for sequence in [8, 6, 8] {
            for entity in [melee, caster] {
                app.world_mut()
                    .entity_mut(entity)
                    .insert(NetworkMinionAction(sequence));
            }
            app.update();
            assert_eq!(pose(&app, blade), rest_blade);
            assert_eq!(pose(&app, staff), rest_staff);
        }
        // A skipped snapshot collapses to one latest release, never a queued burst.
        for entity in [melee, caster] {
            app.world_mut()
                .entity_mut(entity)
                .insert(NetworkMinionAction(12));
        }
        app.update();
        assert_ne!(pose(&app, blade), rest_blade);
        tick(&mut app, 60);
        assert_eq!(pose(&app, blade), rest_blade);
        assert_eq!(pose(&app, staff), rest_staff);
        assert_eq!(
            app.world().entity(melee).get::<CombatStats>().unwrap().hp,
            17.0
        );
    }

    #[test]
    fn sprite2d_minions_do_not_allocate_or_attach_procedural_3d_assets() {
        let mut app = test_app(PlayerVisualMode::Sprite2d);
        for team in [Team::Green, Team::Blue] {
            for kind in [MinionKind::Melee, MinionKind::Caster] {
                spawn_minion(&mut app, team, kind);
            }
        }
        tick(&mut app, 2);
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
        assert_eq!(
            app.world_mut()
                .query::<&MinionAttackPulse>()
                .iter(app.world())
                .count(),
            0
        );
    }
}
