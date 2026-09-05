//! The authored Verdant scene is presentation only. Network entities remain
//! the owners of live structures; the static GLBs exclude all eight copies.
use bevy::prelude::*;

use crate::combat::CombatStats;
use crate::decor::DecorRoot;
use crate::maps::MapLayout;
use crate::net::{NetworkStructure, StructureKind};
use crate::sprite::PlayerVisualMode;
use crate::team::Team;

pub struct Verdant3dPlugin;

impl Plugin for Verdant3dPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (load_assets, spawn_environment).chain())
            .add_systems(
                PostUpdate,
                reconcile_structures.before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

/// One persistent world root, independent of round/network teardown.
#[derive(Component)]
pub struct VerdantEnvironment;

/// The separately owned foliage layer shares the existing F4 debug toggle.
#[derive(Component)]
pub struct VerdantFoliage;

/// Exactly one scene child belongs to each authoritative structure root.
#[derive(Component)]
pub struct VerdantStructureVisual {
    pub owner: Entity,
}

#[derive(Component)]
struct AttachedStructure(Entity);

#[derive(Resource, Clone)]
struct VerdantAssets {
    environment: Handle<Scene>,
    foliage: Handle<Scene>,
    watchtower_green: Handle<Scene>,
    watchtower_blue: Handle<Scene>,
    sanctuary_green: Handle<Scene>,
    sanctuary_blue: Handle<Scene>,
}

impl VerdantAssets {
    fn structure(&self, kind: StructureKind, team: Team) -> Handle<Scene> {
        match (kind, team) {
            (StructureKind::Tower, Team::Green) => self.watchtower_green.clone(),
            (StructureKind::Tower, Team::Blue) => self.watchtower_blue.clone(),
            (StructureKind::BaseTower, Team::Green) => self.sanctuary_green.clone(),
            (StructureKind::BaseTower, Team::Blue) => self.sanctuary_blue.clone(),
        }
    }
}

fn load_assets(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    server: Option<Res<AssetServer>>,
) {
    if *mode != PlayerVisualMode::Models3d {
        return;
    }
    let Some(server) = server else {
        return;
    };
    commands.insert_resource(VerdantAssets {
        environment: server.load("verdant/environment.glb#Scene0"),
        foliage: server.load("verdant/foliage.glb#Scene0"),
        watchtower_green: server.load("verdant/watchtower_green.glb#Scene0"),
        watchtower_blue: server.load("verdant/watchtower_blue.glb#Scene0"),
        sanctuary_green: server.load("verdant/sanctuary_green.glb#Scene0"),
        sanctuary_blue: server.load("verdant/sanctuary_blue.glb#Scene0"),
    });
}

fn spawn_environment(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    layout: Res<MapLayout>,
    assets: Option<Res<VerdantAssets>>,
    existing: Query<Entity, With<VerdantEnvironment>>,
) {
    if *mode != PlayerVisualMode::Models3d || !existing.is_empty() {
        return;
    }
    let Some(assets) = assets else { return };
    // Source export is already Y-up, one meter per unit. Do not reorient it.
    commands.spawn((
        VerdantEnvironment,
        SceneRoot(assets.environment.clone()),
        Transform::IDENTITY,
        Name::new("Verdant / environment"),
    ));
    commands.spawn((
        VerdantFoliage,
        DecorRoot,
        SceneRoot(assets.foliage.clone()),
        Transform::IDENTITY,
        Name::new("Verdant / foliage (F4)"),
    ));
    info!(
        "Verdant: shared environment, foliage and four live structure scenes (Y-up meters); authoritative center lane {:.1} m",
        layout.center_lane_distance()
    );
}

fn structure_transform(layout: &MapLayout, root: &Transform, kind: StructureKind) -> Transform {
    let ground = layout.terrain_height_3d(root.translation.x, root.translation.z);
    // The authoritative root's Y is the legacy box center. Keep that root
    // intact and move only its presentation child to the exported ground pivot.
    let foundation = if kind == StructureKind::Tower {
        0.02
    } else {
        0.0
    };
    let rotation = if kind == StructureKind::BaseTower {
        // Open the diagonal spawn approach: authored diagonal ribs otherwise
        // coincide with the player's unchanged seven-meter spawn offset.
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_4)
    } else {
        Quat::IDENTITY
    };
    Transform::from_xyz(0.0, ground + foundation - root.translation.y, 0.0).with_rotation(rotation)
}

fn reconcile_structures(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    layout: Res<MapLayout>,
    assets: Option<Res<VerdantAssets>>,
    roots: Query<
        (
            Entity,
            &Transform,
            &StructureKind,
            &Team,
            &CombatStats,
            Option<&AttachedStructure>,
        ),
        With<NetworkStructure>,
    >,
    mut visuals: Query<
        (
            &mut SceneRoot,
            &mut Transform,
            &mut Visibility,
            &VerdantStructureVisual,
        ),
        Without<NetworkStructure>,
    >,
) {
    if *mode != PlayerVisualMode::Models3d {
        return;
    }
    let Some(assets) = assets else { return };
    for (owner, root, kind, team, stats, attached) in &roots {
        let transform = structure_transform(&layout, root, *kind);
        let visibility = if stats.hp > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        let scene = assets.structure(*kind, *team);
        if let Some(attached) = attached
            && let Ok((mut current_scene, mut current_transform, mut current_visibility, visual)) =
                visuals.get_mut(attached.0)
            && visual.owner == owner
        {
            if current_scene.0 != scene {
                current_scene.0 = scene;
            }
            *current_transform = transform;
            *current_visibility = visibility;
            continue;
        }
        let child = commands
            .spawn((
                SceneRoot(scene),
                transform,
                visibility,
                VerdantStructureVisual { owner },
                Name::new(format!("Verdant / {team:?} {kind:?}")),
            ))
            .id();
        commands
            .entity(owner)
            .add_child(child)
            .insert(AttachedStructure(child));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(mode: PlayerVisualMode) -> App {
        let mut app = App::new();
        app.insert_resource(mode).init_resource::<MapLayout>();
        let mut scenes = Assets::<Scene>::default();
        let mut scene = || scenes.add(Scene::new(World::new()));
        app.insert_resource(VerdantAssets {
            environment: scene(),
            foliage: scene(),
            watchtower_green: scene(),
            watchtower_blue: scene(),
            sanctuary_green: scene(),
            sanctuary_blue: scene(),
        });
        app.insert_resource(scenes)
            .add_systems(Update, (spawn_environment, reconcile_structures).chain());
        app
    }

    fn structure(app: &mut App, kind: StructureKind, team: Team) -> Entity {
        let layout = *app.world().resource::<MapLayout>();
        let location = match kind {
            StructureKind::Tower => Vec3::new(20.0, 2.0, 20.0),
            StructureKind::BaseTower => match team {
                Team::Green => layout.home_spawn,
                Team::Blue => layout.away_spawn,
            },
        };
        app.world_mut()
            .spawn((
                NetworkStructure,
                kind,
                team,
                Transform::from_translation(location),
                CombatStats {
                    hp: 100.0,
                    max_hp: 100.0,
                    ..default()
                },
            ))
            .id()
    }

    #[test]
    fn sprite2d_never_loads_or_spawns_the_verdant_scene() {
        let mut app = App::new();
        app.insert_resource(PlayerVisualMode::Sprite2d)
            .init_resource::<MapLayout>()
            .add_plugins(Verdant3dPlugin);
        app.update();
        assert!(!app.world().contains_resource::<VerdantAssets>());
        assert_eq!(
            app.world_mut()
                .query::<&SceneRoot>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn repeated_snapshots_keep_one_environment_and_eight_shared_structure_children() {
        let mut app = fixture(PlayerVisualMode::Models3d);
        let mut owners = Vec::new();
        for team in [Team::Green, Team::Blue] {
            owners.push(structure(&mut app, StructureKind::BaseTower, team));
            for _ in 0..3 {
                owners.push(structure(&mut app, StructureKind::Tower, team));
            }
        }
        for _ in 0..20 {
            app.update();
        }
        assert_eq!(
            app.world_mut()
                .query::<&VerdantEnvironment>()
                .iter(app.world())
                .count(),
            1
        );
        assert_eq!(
            app.world_mut()
                .query::<&VerdantFoliage>()
                .iter(app.world())
                .count(),
            1
        );
        assert_eq!(
            app.world_mut()
                .query::<&VerdantStructureVisual>()
                .iter(app.world())
                .count(),
            8
        );
        assert_eq!(app.world().resource::<Assets<Scene>>().len(), 6);
        let assets = app.world().resource::<VerdantAssets>();
        for owner in &owners {
            let entity = app.world().entity(*owner);
            let child = entity.get::<AttachedStructure>().unwrap().0;
            assert_eq!(entity.get::<Children>().unwrap().len(), 1);
            let visual = app.world().entity(child);
            assert_eq!(
                visual.get::<VerdantStructureVisual>().unwrap().owner,
                *owner
            );
            assert_eq!(visual.get::<ChildOf>().unwrap().parent(), *owner);
            assert_eq!(
                visual.get::<SceneRoot>().unwrap().0,
                assets.structure(
                    *entity.get::<StructureKind>().unwrap(),
                    *entity.get::<Team>().unwrap()
                )
            );
            assert!(entity.get::<Mesh3d>().is_none());
        }
    }

    #[test]
    fn damage_death_and_rematch_preserve_hp_ownership_without_scene_accumulation() {
        let mut app = fixture(PlayerVisualMode::Models3d);
        let owner = structure(&mut app, StructureKind::Tower, Team::Green);
        app.update();
        let child = app.world().get::<AttachedStructure>(owner).unwrap().0;
        let scene = app.world().get::<SceneRoot>(child).unwrap().0.clone();
        app.world_mut().get_mut::<CombatStats>(owner).unwrap().hp = 37.0;
        app.update();
        assert_eq!(app.world().get::<CombatStats>(owner).unwrap().hp, 37.0);
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Inherited
        );
        app.world_mut().get_mut::<CombatStats>(owner).unwrap().hp = 0.0;
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Hidden
        );
        // A reset using the same server ID/entity restores the existing scene.
        app.world_mut().get_mut::<CombatStats>(owner).unwrap().hp = 100.0;
        app.update();
        assert_eq!(
            app.world().get::<AttachedStructure>(owner).unwrap().0,
            child
        );
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Inherited
        );
        // Disconnect/full rematch teardown recursively removes the owned child.
        app.world_mut().despawn(owner);
        assert!(app.world().get_entity(child).is_err());
        let restored = structure(&mut app, StructureKind::Tower, Team::Green);
        app.update();
        let restored_child = app.world().get::<AttachedStructure>(restored).unwrap().0;
        assert_eq!(
            app.world().get::<SceneRoot>(restored_child).unwrap().0,
            scene
        );
        assert_eq!(
            app.world_mut()
                .query::<&VerdantStructureVisual>()
                .iter(app.world())
                .count(),
            1
        );
        assert_eq!(
            app.world_mut()
                .query::<&VerdantEnvironment>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[test]
    fn structure_offsets_cancel_authoritative_box_centers_and_open_spawn_approach() {
        let layout = MapLayout::default();
        let tower = Transform::from_xyz(20.0, 2.0, 20.0);
        let visual = structure_transform(&layout, &tower, StructureKind::Tower);
        assert!((tower.translation.y + visual.translation.y - 0.02).abs() < 0.00001);
        let base = Transform::from_translation(layout.home_spawn);
        let visual = structure_transform(&layout, &base, StructureKind::BaseTower);
        assert!((base.translation.y + visual.translation.y - 0.7).abs() < 0.00001);
        let spawn = layout.team_spawn(Team::Green) - layout.home_spawn;
        for i in 0..4 {
            let angle = std::f32::consts::FRAC_PI_4 + i as f32 * std::f32::consts::FRAC_PI_2;
            let foot = visual.rotation * Vec3::new(angle.cos() * 7.15, 0.0, angle.sin() * 7.15);
            assert!(
                foot.distance(spawn) > 4.0,
                "sanctuary rib blocks unchanged player spawn"
            );
        }
    }
}
