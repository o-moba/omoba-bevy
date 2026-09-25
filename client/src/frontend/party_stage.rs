//! 3D line-up of the party for the lobby screen: every member's avatar on a
//! pedestal, side by side, playing its idle clip.
//!
//! Like the collection preview this owns its own camera, lights and render
//! layer and draws into an image the lobby UI displays. It never touches the
//! match world and only renders while the lobby is on screen.
use bevy::camera::{RenderTarget, visibility::RenderLayers};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;

use super::AppScreen;
use crate::model_scale::{ModelScaleSource, NormalizeModelScale, model_scale_key};
use crate::world::PlayerModelResolver;

/// Render layer of the line-up (the avatar preview owns 28, the supporter
/// aura preview 29).
pub const STAGE_LAYER: usize = 27;
pub const STAGE_WIDTH: u32 = 1200;
pub const STAGE_HEIGHT: u32 = 520;
/// Far below the arena and away from the avatar preview rig.
const STAGE_ORIGIN: Vec3 = Vec3::new(40.0, -2000.0, 0.0);
const SLOT_SPACING: f32 = 1.6;

/// One member on stage: the avatar slug (`None` = the default model) and
/// whether this member leads the party (gold pedestal).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageMember {
    pub avatar: Option<String>,
    pub leader: bool,
}

struct Slot {
    root: Entity,
    model: Entity,
    gltf: Option<Handle<Gltf>>,
    bound: bool,
}

#[derive(Resource)]
pub struct PartyStage {
    pub image: Handle<Image>,
    /// What the lobby wants shown, in order.
    pub members: Vec<StageMember>,
    spawned: Option<Vec<StageMember>>,
    slots: Vec<Slot>,
}

impl FromWorld for PartyStage {
    fn from_world(world: &mut World) -> Self {
        let image = world
            .resource_mut::<Assets<Image>>()
            .add(Image::new_target_texture(
                STAGE_WIDTH,
                STAGE_HEIGHT,
                TextureFormat::Rgba8Unorm,
                Some(TextureFormat::Rgba8UnormSrgb),
            ));
        Self {
            image,
            members: Vec::new(),
            spawned: None,
            slots: Vec::new(),
        }
    }
}

pub struct PartyStagePlugin;

impl Plugin for PartyStagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PartyStage>()
            .add_systems(Startup, setup_stage)
            .add_systems(OnExit(AppScreen::Lobby), clear_stage)
            .add_systems(
                Update,
                (
                    sync_stage,
                    tag_stage_layers,
                    play_idle,
                    frame_stage,
                    toggle_stage_camera,
                )
                    .chain(),
            );
    }
}

#[derive(Component)]
pub struct StageCamera;

fn setup_stage(mut commands: Commands, stage: Res<PartyStage>) {
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -3,
            is_active: false,
            clear_color: Color::srgb(0.035, 0.085, 0.092).into(),
            ..default()
        },
        RenderTarget::Image(stage.image.clone().into()),
        camera_transform(1),
        RenderLayers::layer(STAGE_LAYER),
        StageCamera,
        Name::new("PartyStageCamera"),
    ));
    for (illuminance, from) in [
        (9_000.0, Vec3::new(2.4, 3.4, 3.2)),
        (3_200.0, Vec3::new(-2.8, 1.8, -1.6)),
    ] {
        commands.spawn((
            DirectionalLight {
                illuminance,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_translation(from).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
            RenderLayers::layer(STAGE_LAYER),
            Name::new("PartyStageLight"),
        ));
    }
}

/// Pulls back as the party grows so every member stays in frame.
fn camera_transform(members: usize) -> Transform {
    let distance = 2.9 + 0.55 * members.max(1) as f32;
    Transform::from_translation(STAGE_ORIGIN + Vec3::new(0.0, 1.15, distance))
        .looking_at(STAGE_ORIGIN + Vec3::new(0.0, 0.9, 0.0), Vec3::Y)
}

/// X offset of slot `index` in a line-up of `count`, centred on the origin.
pub fn slot_offset(index: usize, count: usize) -> f32 {
    (index as f32 - (count.saturating_sub(1)) as f32 / 2.0) * SLOT_SPACING
}

fn clear_stage(mut stage: ResMut<PartyStage>) {
    stage.members.clear();
}

fn sync_stage(
    mut commands: Commands,
    mut stage: ResMut<PartyStage>,
    mut models: PlayerModelResolver,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut pedestal: Local<
        Option<(
            Handle<Mesh>,
            Handle<StandardMaterial>,
            Handle<StandardMaterial>,
        )>,
    >,
) {
    if stage.spawned.as_ref() == Some(&stage.members) {
        return;
    }
    for slot in stage.slots.drain(..) {
        commands
            .entity(slot.root)
            .despawn_related::<Children>()
            .despawn();
    }
    let members = stage.members.clone();
    let (mesh, plain, gold) = pedestal
        .get_or_insert_with(|| {
            let material = |color: Color| StandardMaterial {
                base_color: color,
                perceptual_roughness: 0.85,
                ..default()
            };
            (
                meshes.add(Cylinder::new(0.55, 0.06)),
                materials.add(material(Color::srgb(0.10, 0.20, 0.21))),
                materials.add(material(Color::srgb(0.55, 0.45, 0.22))),
            )
        })
        .clone();
    for (index, member) in members.iter().enumerate() {
        let root = commands
            .spawn((
                Transform::from_translation(
                    STAGE_ORIGIN + Vec3::new(slot_offset(index, members.len()), 0.0, 0.0),
                )
                // Roster models are authored facing -Z; the camera sits on +Z.
                .with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
                Visibility::Visible,
                RenderLayers::layer(STAGE_LAYER),
                Name::new(format!("PartyStageSlot{index}")),
            ))
            .id();
        commands.entity(root).with_child((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(if member.leader {
                gold.clone()
            } else {
                plain.clone()
            }),
            Transform::from_xyz(0.0, -0.03, 0.0),
            RenderLayers::layer(STAGE_LAYER),
        ));
        let (scene, gltf) = models.resolve(
            crate::team::CharacterChoice::default(),
            member.avatar.as_deref(),
        );
        let Some(scene) = scene else {
            stage.slots.push(Slot {
                root,
                model: root,
                gltf: None,
                bound: true,
            });
            continue;
        };
        let model = commands
            .spawn((
                SceneRoot(scene),
                Transform::default(),
                Visibility::Visible,
                RenderLayers::layer(STAGE_LAYER),
                NormalizeModelScale::for_player_model(),
                Name::new("PartyStageModel"),
            ))
            .id();
        if let Some(gltf) = gltf.clone() {
            commands.entity(model).insert(ModelScaleSource {
                gltf,
                key: model_scale_key(
                    crate::team::CharacterChoice::default(),
                    member.avatar.as_deref(),
                ),
            });
        }
        commands.entity(root).add_child(model);
        stage.slots.push(Slot {
            root,
            model,
            gltf,
            bound: false,
        });
    }
    stage.spawned = Some(members);
}

/// glTF children spawn over several frames and do not inherit render layers.
/// The line-up holds a handful of models, so every frame on screen is cheap.
fn tag_stage_layers(
    mut commands: Commands,
    stage: Res<PartyStage>,
    screen: Res<State<AppScreen>>,
    children: Query<&Children>,
    tagged: Query<&RenderLayers>,
) {
    if *screen.get() != AppScreen::Lobby {
        return;
    }
    let mut stack: Vec<Entity> = stage.slots.iter().map(|s| s.root).collect();
    while let Some(entity) = stack.pop() {
        if tagged.get(entity).is_err() {
            commands
                .entity(entity)
                .insert(RenderLayers::layer(STAGE_LAYER));
        }
        if let Ok(children) = children.get(entity) {
            stack.extend(children.iter());
        }
    }
}

/// Idle when the avatar has one, else its first clip.
fn play_idle(
    mut commands: Commands,
    mut stage: ResMut<PartyStage>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    gltfs: Res<Assets<Gltf>>,
    children: Query<&Children>,
    mut players: Query<&mut AnimationPlayer>,
) {
    for slot in stage.slots.iter_mut().filter(|s| !s.bound) {
        let Some(gltf) = slot.gltf.as_ref().and_then(|h| gltfs.get(h)) else {
            continue;
        };
        let mut stack = vec![slot.model];
        let mut target = None;
        while let Some(entity) = stack.pop() {
            if players.get(entity).is_ok() {
                target = Some(entity);
                break;
            }
            if let Ok(children) = children.get(entity) {
                stack.extend(children.iter());
            }
        }
        let Some(target) = target else {
            continue;
        };
        slot.bound = true;
        let clip = gltf
            .named_animations
            .iter()
            .find(|(name, _)| name.to_ascii_lowercase().contains("idle"))
            .or_else(|| gltf.named_animations.iter().next())
            .map(|(_, clip)| clip.clone());
        let Some(clip) = clip else {
            continue;
        };
        let (graph, node) = AnimationGraph::from_clip(clip);
        commands
            .entity(target)
            .insert(AnimationGraphHandle(graphs.add(graph)));
        if let Ok(mut player) = players.get_mut(target) {
            player.stop_all();
            player.play(node).repeat();
        }
    }
}

fn frame_stage(
    stage: Res<PartyStage>,
    mut cameras: Query<&mut Transform, With<StageCamera>>,
    mut framed: Local<usize>,
) {
    let count = stage.members.len();
    if *framed == count {
        return;
    }
    *framed = count;
    for mut transform in &mut cameras {
        *transform = camera_transform(count);
    }
}

fn toggle_stage_camera(
    screen: Res<State<AppScreen>>,
    stage: Res<PartyStage>,
    mut cameras: Query<&mut Camera, With<StageCamera>>,
) {
    let wanted = *screen.get() == AppScreen::Lobby && !stage.members.is_empty();
    for mut camera in &mut cameras {
        if camera.is_active != wanted {
            camera.is_active = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_up_is_centred_and_evenly_spaced() {
        assert_eq!(slot_offset(0, 1), 0.0);
        assert_eq!(slot_offset(0, 2), -slot_offset(1, 2));
        assert_eq!(slot_offset(2, 5), 0.0);
        assert!((slot_offset(4, 5) - slot_offset(3, 5) - SLOT_SPACING).abs() < 1e-6);
    }

    #[test]
    fn the_stage_layer_is_its_own() {
        assert_ne!(STAGE_LAYER, super::super::preview::PREVIEW_LAYER);
        assert_ne!(STAGE_LAYER, 29);
    }

    #[test]
    fn the_camera_pulls_back_for_bigger_parties() {
        let near = camera_transform(1).translation.z;
        let far = camera_transform(5).translation.z;
        assert!(far > near);
    }
}
