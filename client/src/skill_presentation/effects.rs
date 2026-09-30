//! Persistent, bounded drawables for authoritative skill objects. Despawn is silent.
use super::{EffectStyle, SkillPresentation};
use crate::game_vfx::hdr_tint;
use crate::{net::GameStateSnapshot, sprite::PlayerVisualMode};
use bevy::prelude::*;
use shared::loadout::{EffectVisualKind, SkillEffectState};
use std::collections::{HashMap, HashSet};

pub(super) struct SkillEffectsPlugin;
impl Plugin for SkillEffectsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Instances>()
            .add_systems(Startup, setup)
            .add_systems(
                PostUpdate,
                sync.before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

#[derive(Resource)]
struct Geometry {
    ball: Handle<Mesh>,
    cone: Handle<Mesh>,
    block: Handle<Mesh>,
    ring: Handle<Mesh>,
    disc: Handle<Mesh>,
    white: Handle<StandardMaterial>,
    friendly: Handle<StandardMaterial>,
    hostile: Handle<StandardMaterial>,
    colors: HashMap<String, Handle<StandardMaterial>>,
    fills: HashMap<String, Handle<StandardMaterial>>,
    rocket: Handle<Scene>,
    trap: Handle<Scene>,
}

fn material(color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        unlit: true,
        fog_enabled: false,
        double_sided: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Blend,
        ..default()
    }
}
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    server: Res<AssetServer>,
) {
    commands.insert_resource(Geometry {
        ball: meshes.add(Sphere::new(1.0).mesh().ico(1).unwrap()),
        cone: meshes.add(Cone::new(1.0, 1.0)),
        block: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        ring: meshes.add(Annulus::new(0.94, 1.0)),
        disc: meshes.add(Circle::new(1.0)),
        white: materials.add(material(hdr_tint(Color::srgb(1.0, 0.97, 0.82), 5.0))),
        friendly: materials.add(material(hdr_tint(Color::srgb(0.12, 0.95, 0.60), 2.0))),
        hostile: materials.add(material(hdr_tint(Color::srgb(1.0, 0.12, 0.08), 2.0))),
        colors: HashMap::new(),
        fills: HashMap::new(),
        rocket: server.load("cosmetics/standard/rocket.glb#Scene0"),
        trap: server.load("cosmetics/standard/trap.glb#Scene0"),
    });
}

#[derive(Clone, Copy)]
enum Part {
    Core,
    Boundary,
    Inner,
    Satellite(u8),
    Streak(u8),
    Model,
}
struct Instance {
    root: Entity,
    parts: Vec<(Entity, Part)>,
    skill: shared::loadout::SkillId,
    kind: EffectVisualKind,
    style: EffectStyle,
    friendly: bool,
    model: Option<Handle<Scene>>,
}
#[derive(Resource, Default)]
struct Instances {
    round: Option<(u64, u64)>,
    objects: HashMap<u64, Instance>,
}

pub(crate) fn valid_effect(e: &SkillEffectState) -> bool {
    e.position.into_iter().chain(e.end).all(f32::is_finite)
        && e.radius.is_finite()
        && (0.0..=256.0).contains(&e.radius)
        && e.remaining_secs.is_finite()
        && e.remaining_secs >= 0.0
}

fn spawn_instance(
    commands: &mut Commands,
    e: &SkillEffectState,
    style: EffectStyle,
    friendly: bool,
    geometry: &Geometry,
    color: Handle<StandardMaterial>,
    fill: Handle<StandardMaterial>,
) -> Instance {
    let root = commands
        .spawn((
            Transform::default(),
            Visibility::Hidden,
            Name::new(format!("SkillVfx-{}-{}", e.skill.id(), e.id)),
        ))
        .id();
    let mut parts = Vec::new();
    let boundary = if friendly {
        &geometry.friendly
    } else {
        &geometry.hostile
    };
    let mut part = |role, mesh: &Handle<Mesh>, mat: &Handle<StandardMaterial>| {
        let entity = commands
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::default(),
                Visibility::default(),
                ChildOf(root),
            ))
            .id();
        parts.push((entity, role));
    };
    match style {
        EffectStyle::Field => {
            part(Part::Core, &geometry.disc, &fill);
            part(Part::Boundary, &geometry.ring, boundary);
            part(Part::Inner, &geometry.ring, &color);
            for i in 0..6 {
                part(Part::Satellite(i), &geometry.ball, &color);
            }
        }
        EffectStyle::Beam => {
            part(Part::Core, &geometry.block, &fill);
            for i in 0..2 {
                part(Part::Streak(i), &geometry.block, boundary);
            }
            part(Part::Streak(2), &geometry.block, &geometry.white);
            part(Part::Inner, &geometry.ring, &color);
        }
        EffectStyle::Trap => {
            part(Part::Core, &geometry.ball, &color);
            part(Part::Boundary, &geometry.ring, boundary);
            for i in 0..6 {
                part(Part::Satellite(i), &geometry.cone, &color);
            }
        }
        EffectStyle::Aegis => {
            part(Part::Core, &geometry.ball, &geometry.white);
            part(Part::Boundary, &geometry.ring, boundary);
            part(Part::Inner, &geometry.ring, &color);
        }
        _ => {
            part(Part::Core, &geometry.ball, &color);
            part(Part::Boundary, &geometry.ring, boundary);
            for i in 0..3 {
                part(
                    Part::Streak(i),
                    &geometry.block,
                    if i == 0 { &geometry.white } else { &color },
                );
            }
        }
    }
    let model = match style {
        EffectStyle::Rocket => Some(geometry.rocket.clone()),
        EffectStyle::Trap => Some(geometry.trap.clone()),
        _ => None,
    };
    if let Some(scene) = &model {
        let entity = commands
            .spawn((
                SceneRoot(scene.clone()),
                Transform::default(),
                Visibility::Hidden,
                ChildOf(root),
            ))
            .id();
        parts.push((entity, Part::Model));
    }
    Instance {
        root,
        parts,
        skill: e.skill,
        kind: e.kind,
        style,
        friendly,
        model,
    }
}

/// Uses received (possibly fog-clipped) geometry, never reconstructs a hidden origin.
fn sync(
    mut commands: Commands,
    game: Option<Res<GameStateSnapshot>>,
    mode: Res<PlayerVisualMode>,
    profiles: Res<SkillPresentation>,
    time: Res<Time>,
    map: Option<Res<crate::maps::MapLayout>>,
    local: Query<&crate::team::Team, With<crate::player::Player>>,
    mut instances: ResMut<Instances>,
    mut geometry: ResMut<Geometry>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    server: Res<AssetServer>,
    mut poses: Query<(&mut Transform, &mut Visibility)>,
    scene_instances: Query<&bevy::scene::SceneInstance>,
    scene_spawner: Option<Res<bevy::scene::SceneSpawner>>,
) {
    let round = game
        .as_ref()
        .map(|g| (g.meta.server_epoch, g.meta.match_id));
    if round != instances.round || *mode != PlayerVisualMode::Models3d {
        for (_, instance) in instances.objects.drain() {
            commands.entity(instance.root).despawn();
        }
        instances.round = round;
    }
    let Some(game) = game.filter(|_| *mode == PlayerVisualMode::Models3d) else {
        return;
    };
    let now = game
        .sandbox
        .as_ref()
        .map_or(time.elapsed_secs(), |s| s.simulation_secs as f32);
    let mut live = HashSet::new();
    for e in game
        .skill_effects
        .iter()
        .take(shared::loadout::MAX_ACTIVE_EFFECTS)
    {
        if !valid_effect(e) {
            continue;
        }
        let Some(profile) = profiles.profile(e.skill) else {
            continue;
        };
        let friendly = local.single().is_ok_and(|t| *t == e.owner_team);
        if instances.objects.get(&e.id).is_some_and(|i| {
            i.skill != e.skill
                || i.kind != e.kind
                || i.style != profile.effect
                || i.friendly != friendly
        }) {
            let previous = instances.objects.remove(&e.id).unwrap();
            commands.entity(previous.root).despawn();
        }
        live.insert(e.id);
        let color_key = format!("{}-{:?}-{}", e.skill.id(), profile.color, profile.hdr_gain);
        let color = geometry
            .colors
            .entry(color_key.clone())
            .or_insert_with(|| {
                materials.add(material(hdr_tint(
                    Color::srgb_from_array(profile.color),
                    profile.hdr_gain,
                )))
            })
            .clone();
        let fill = geometry
            .fills
            .entry(color_key)
            .or_insert_with(|| {
                materials.add(material(Color::srgba(
                    profile.color[0],
                    profile.color[1],
                    profile.color[2],
                    0.12,
                )))
            })
            .clone();
        let instance = instances.objects.entry(e.id).or_insert_with(|| {
            spawn_instance(
                &mut commands,
                e,
                profile.effect,
                friendly,
                &geometry,
                color,
                fill,
            )
        });
        let p = Vec2::from_array(e.position);
        let end = Vec2::from_array(e.end);
        let direction = (end - p).normalize_or(Vec2::Y);
        let rotation = Quat::from_rotation_y(direction.x.atan2(direction.y));
        let ground = map.as_ref().map_or(0.0, |m| m.terrain_height_3d(p.x, p.y));
        let planar = matches!(
            profile.effect,
            EffectStyle::Field | EffectStyle::Trap | EffectStyle::Beam
        );
        let root_pose = Transform::from_xyz(p.x, ground + if planar { 0.15 } else { 0.95 }, p.y)
            .with_rotation(rotation);
        if let Ok((mut pose, mut visible)) = poses.get_mut(instance.root) {
            *pose = root_pose;
            *visible = Visibility::Inherited;
        } else {
            commands
                .entity(instance.root)
                .insert((root_pose, Visibility::Inherited));
        }
        let radius = e.radius.max(0.05);
        let model_ready = instance.model.as_ref().is_some_and(|s| {
            matches!(
                server.get_recursive_dependency_load_state(s.id()),
                Some(bevy::asset::RecursiveDependencyLoadState::Loaded)
            )
        }) && instance.parts.iter().any(|(entity, role)| {
            matches!(role, Part::Model)
                && scene_instances.get(*entity).is_ok_and(|scene| {
                    scene_spawner
                        .as_ref()
                        .is_some_and(|spawner| spawner.instance_is_ready(**scene))
                })
        });
        let ground_ring = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        #[cfg(feature = "qa")]
        commands
            .entity(instance.root)
            .insert(super::SkillEffectVisual {
                id: e.id,
                model_ready,
            });
        for &(entity, role) in &instance.parts {
            let mut visibility = Visibility::Inherited;
            let mut t = Transform::default();
            match (profile.effect, role) {
                (_, Part::Model) => {
                    visibility = if model_ready {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    };
                    t.scale = Vec3::splat(if profile.effect == EffectStyle::Trap {
                        radius * 1.6
                    } else {
                        1.15
                    });
                    if profile.effect == EffectStyle::Trap {
                        t.rotation = Quat::from_rotation_y(now * if e.armed { 0.0 } else { 3.0 });
                    }
                }
                (EffectStyle::Field, Part::Core) => {
                    t.scale = Vec3::splat(radius);
                    t.rotation = ground_ring;
                }
                (EffectStyle::Field, Part::Boundary) => {
                    t.scale = Vec3::splat(radius);
                    t.rotation = ground_ring;
                }
                (EffectStyle::Field, Part::Inner) => {
                    t.scale = Vec3::splat(radius * (0.65 + 0.03 * (now * 3.0).sin()));
                    t.rotation = ground_ring;
                    t.translation.y = 0.03;
                }
                (EffectStyle::Field, Part::Satellite(i)) => {
                    let angle = now * 0.5 + i as f32 * std::f32::consts::TAU / 6.0;
                    t.translation = Vec3::new(
                        angle.cos() * radius * 0.82,
                        0.25 + (now * 2.0 + angle).sin() * 0.1,
                        angle.sin() * radius * 0.82,
                    );
                    t.scale = Vec3::splat(0.1);
                }
                (EffectStyle::Beam, Part::Core) => {
                    let length = p.distance(end);
                    t.translation.z = length * 0.5;
                    t.scale = Vec3::new(
                        radius * 2.0,
                        if e.kind == EffectVisualKind::BeamWarning {
                            0.025
                        } else {
                            0.25
                        },
                        length.max(0.01),
                    );
                }
                (EffectStyle::Beam, Part::Streak(i)) => {
                    let length = p.distance(end);
                    if i == 2 {
                        t.translation = Vec3::new(0.0, 0.16, length * 0.5);
                        t.scale = Vec3::new(radius * 0.55, 0.16, length.max(0.01));
                        if e.kind == EffectVisualKind::BeamWarning {
                            visibility = Visibility::Hidden;
                        }
                    } else {
                        t.translation =
                            Vec3::new(if i == 0 { -radius } else { radius }, 0.07, length * 0.5);
                        t.scale = Vec3::new(0.055, 0.07, length.max(0.01));
                    }
                }
                (EffectStyle::Beam, Part::Inner) => {
                    t.rotation = ground_ring;
                    t.scale = Vec3::splat(radius.max(0.5) * (1.0 + 0.1 * (now * 9.0).sin()));
                }
                (EffectStyle::Trap, Part::Boundary) => {
                    t.rotation = ground_ring;
                    t.scale = Vec3::splat(radius);
                }
                (EffectStyle::Trap, Part::Satellite(i)) => {
                    let a = i as f32 * std::f32::consts::TAU / 6.0;
                    t.translation = Vec3::new(a.cos() * radius * 0.6, 0.1, a.sin() * radius * 0.6);
                    t.scale = Vec3::new(0.13, if e.armed { 0.4 } else { 0.1 }, 0.13);
                }
                (EffectStyle::Trap, Part::Core) => {
                    t.translation.y = 0.15;
                    t.scale = Vec3::splat(if e.armed { 0.16 } else { 0.08 });
                }
                (EffectStyle::Aegis, Part::Core) => {
                    t.scale = Vec3::splat(radius * 0.35);
                }
                (EffectStyle::Aegis, Part::Boundary) => {
                    t.scale = Vec3::splat(radius);
                    t.rotation = Quat::from_rotation_z(now * 3.0);
                }
                (EffectStyle::Aegis, Part::Inner) => {
                    t.scale = Vec3::splat(radius * 0.7);
                    t.rotation = Quat::from_rotation_y(now * 4.0);
                }
                (_, Part::Core) => {
                    t.scale = Vec3::new(radius * 0.75, radius * 0.75, radius * 2.0);
                    if model_ready {
                        visibility = Visibility::Hidden;
                    }
                }
                (_, Part::Boundary) => {
                    t.scale = Vec3::splat(radius);
                    t.rotation = ground_ring;
                    t.translation.y = -0.75;
                }
                (_, Part::Streak(i)) => {
                    t.translation.z = -0.5 - i as f32 * 0.55;
                    let width = (0.11 - i as f32 * 0.025).max(0.025);
                    t.scale = Vec3::new(width, width, 0.8);
                }
                _ => {}
            }
            if model_ready
                && profile.effect == EffectStyle::Trap
                && matches!(role, Part::Core | Part::Satellite(_))
            {
                visibility = Visibility::Hidden;
            }
            if let Ok((mut pose, mut visible)) = poses.get_mut(entity) {
                *pose = t;
                *visible = visibility;
            } else {
                // Deferred spawns receive their final pose before transform propagation.
                // A one-snapshot beam must render on this frame, not one frame later.
                commands.entity(entity).insert((t, visibility));
            }
        }
    }
    instances.objects.retain(|id, instance| {
        if live.contains(id) {
            true
        } else {
            commands.entity(instance.root).despawn();
            false
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_objects_reuse_identity_and_clear_on_fog_round_or_2d_switch() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<Scene>()
            .insert_resource(PlayerVisualMode::Models3d)
            .init_resource::<GameStateSnapshot>()
            .insert_resource(
                SkillPresentation::parse(include_str!("../../assets/config/skills.skillfx"))
                    .unwrap(),
            )
            .add_plugins(SkillEffectsPlugin);
        let mut e = SkillEffectState {
            id: 9,
            owner_id: 7,
            owner_team: shared::map::Team::Green,
            skill: shared::loadout::SkillId::WildRocket,
            kind: EffectVisualKind::Rocket,
            position: [4.0, 5.0],
            end: [10.0, 5.0],
            radius: 0.8,
            remaining_secs: 1.0,
            armed: true,
            consumed_segments: 0,
        };
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .skill_effects
            .push(e.clone());
        app.update();
        let root = app.world().resource::<Instances>().objects[&9].root;
        assert_eq!(
            *app.world().get::<Visibility>(root).unwrap(),
            Visibility::Inherited
        );
        assert_eq!(
            app.world().get::<Transform>(root).unwrap().translation.x,
            4.0
        );
        e.position[0] = 6.0;
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .skill_effects[0] = e.clone();
        app.update();
        assert_eq!(app.world().resource::<Instances>().objects[&9].root, root);
        assert_eq!(
            app.world().get::<Transform>(root).unwrap().translation.x,
            6.0
        );
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .skill_effects
            .clear();
        app.update();
        assert!(
            app.world().get_entity(root).is_err(),
            "fog omission removes visual and children"
        );
        assert!(app.world().resource::<Instances>().objects.is_empty());
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .skill_effects
            .push(e);
        app.update();
        let old = app.world().resource::<Instances>().objects[&9].root;
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .meta
            .match_id += 1;
        app.update();
        assert!(
            app.world().get_entity(old).is_err(),
            "reused IDs do not survive round changes"
        );
        *app.world_mut().resource_mut::<PlayerVisualMode>() = PlayerVisualMode::Sprite2d;
        app.update();
        assert!(app.world().resource::<Instances>().objects.is_empty());
    }
}
