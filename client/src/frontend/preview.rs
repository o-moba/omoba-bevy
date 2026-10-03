//! Live 3D avatar preview used by the collection screen.
//!
//! The preview owns its own camera, its own lights and its own render layer,
//! and draws into an image the UI displays. It never touches the match world:
//! nothing here is a player, and the gameplay camera cannot see these
//! entities.
//!
//! No dictionary text: clip labels come from the model's animation names.
// i18n-strict

use bevy::camera::{RenderTarget, visibility::RenderLayers};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;

use super::AppScreen;
use crate::model_scale::{ModelScaleSource, NormalizeModelScale, model_scale_key};
use crate::world::PlayerModelResolver;

pub(crate) use super::preview_interaction::InteractivePreview;

/// Render layer reserved for the avatar preview (the supporter aura preview
/// owns 29).
pub const PREVIEW_LAYER: usize = 28;
const PREVIEW_WIDTH: u32 = 460;
const PREVIEW_HEIGHT: u32 = 620;
/// The painted Home platform meets the preview at this fraction of its height.
/// Camera fitting must preserve this ground line when a wide model zooms out.
pub(super) const PREVIEW_GROUND_ANCHOR: f32 = 0.87;
/// Frames the layer tagging keeps running *after* the scene reported ready, to
/// catch entities inserted by post-load systems. Before that it runs for as
/// long as the scene takes, however slow the load is.
const TAG_FRAMES_AFTER_READY: u8 = 30;
/// Where the preview rig lives. Far below the arena, so that even an entity
/// that somehow missed its render layer can never stand on the match map.
const PREVIEW_ORIGIN: Vec3 = Vec3::new(0.0, -2000.0, 0.0);

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PreviewClip {
    /// Clip name as authored in the glTF.
    pub name: String,
    pub node: AnimationNodeIndex,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PreviewStatus {
    /// No avatar selected yet.
    Empty,
    /// The model (or its animations) is still loading.
    Loading,
    Ready,
    /// The selected SDK model could not be downloaded or validated.
    Unavailable,
    /// The model rendered but carries no animation clips.
    NoAnimations,
}

/// The preview's whole state. Screens set [`AvatarPreview::slug`] and read
/// [`AvatarPreview::clips`]; everything else is driven by this module.
#[derive(Resource)]
pub struct AvatarPreview {
    /// Render target the UI draws with an `ImageNode`.
    pub image: Handle<Image>,
    /// Avatar currently shown.
    pub slug: Option<String>,
    /// Turntable angle in radians.
    pub yaw: f32,
    pub auto_spin: bool,
    /// Clips discovered on the current model, in a stable order.
    pub clips: Vec<PreviewClip>,
    /// Index into [`AvatarPreview::clips`].
    pub selected: usize,
    pub status: PreviewStatus,
    spawned_slug: Option<String>,
    spawned_store_state: Option<omoba_passport::store::ModelState>,
    pivot: Option<Entity>,
    model: Option<Entity>,
    player: Option<Entity>,
    gltf: Option<Handle<Gltf>>,
    graph: Option<Handle<AnimationGraph>>,
    bound: bool,
    /// Set by the `SceneInstanceReady` observer of the current model.
    scene_ready: bool,
    tag_frames: u8,
    gesture: Option<usize>,
    gesture_serial: u64,
}

impl AvatarPreview {
    /// Shows `slug` standing still and facing the camera.
    pub fn show_portrait(&mut self, slug: &str) {
        self.show(slug);
        self.auto_spin = false;
        // Roster models are authored facing -Z; the preview camera sits on +Z.
        self.yaw = std::f32::consts::PI;
    }

    pub fn show(&mut self, slug: &str) {
        if self.slug.as_deref() == Some(slug) {
            return;
        }
        self.slug = Some(slug.to_owned());
        self.yaw = std::f32::consts::PI;
        self.auto_spin = false;
    }

    pub fn selected_clip(&self) -> Option<&PreviewClip> {
        self.clips.get(self.selected)
    }

    /// Use expressive clips actually shipped with this model. VRM alone does
    /// not guarantee a wave/cheer animation; never substitute a death pose.
    pub(super) fn greet(&mut self, seed: u64) {
        let choices: Vec<_> = self
            .clips
            .iter()
            .enumerate()
            .filter(|(_, clip)| {
                let name = clip.name.to_ascii_lowercase();
                [
                    "wave", "hello", "greet", "cheer", "victory", "taunt", "attack", "cast",
                    "dance",
                ]
                .iter()
                .any(|word| name.contains(word))
            })
            .map(|(index, _)| index)
            .collect();
        if !choices.is_empty() {
            let mixed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(self.gesture_serial);
            self.gesture = Some(choices[(mixed as usize) % choices.len()]);
            self.gesture_serial = self.gesture_serial.wrapping_add(1);
        }
    }

    /// Human-readable clip label ("Walk Cycle" from "walkcycle").
    pub fn clip_label(name: &str) -> String {
        let cleaned = name.replace(['_', '-', '.'], " ");
        let mut label = String::with_capacity(cleaned.len());
        for word in cleaned.split_whitespace() {
            if !label.is_empty() {
                label.push(' ');
            }
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                label.extend(first.to_uppercase());
                label.push_str(chars.as_str());
            }
        }
        if label.is_empty() {
            name.to_owned()
        } else {
            label
        }
    }
}

/// The resource exists from the first frame: screens are entered during the
/// startup state transition, before `Startup` systems have run.
impl FromWorld for AvatarPreview {
    fn from_world(world: &mut World) -> Self {
        let image = world
            .resource_mut::<Assets<Image>>()
            .add(Image::new_target_texture(
                PREVIEW_WIDTH,
                PREVIEW_HEIGHT,
                TextureFormat::Rgba8Unorm,
                Some(TextureFormat::Rgba8UnormSrgb),
            ));
        Self {
            image,
            slug: None,
            yaw: std::f32::consts::PI,
            auto_spin: false,
            clips: Vec::new(),
            selected: 0,
            status: PreviewStatus::Empty,
            spawned_slug: None,
            spawned_store_state: None,
            pivot: None,
            model: None,
            player: None,
            gltf: None,
            graph: None,
            bound: false,
            scene_ready: false,
            tag_frames: 0,
            gesture: None,
            gesture_serial: 0,
        }
    }
}

pub struct AvatarPreviewPlugin;

impl Plugin for AvatarPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AvatarPreview>()
            .init_resource::<crate::humanoid::HumanoidRuntimeLibrary>()
            .add_systems(Startup, setup_preview)
            .add_systems(
                Update,
                (
                    release_preview_off_screen,
                    sync_preview_model,
                    sync_preview_equipment,
                    tag_preview_layers,
                    bind_preview_animations,
                    super::preview_interaction::interact,
                    apply_clip_selection,
                    spin_preview,
                    toggle_preview_camera,
                    frame_preview_camera,
                )
                    .chain(),
            );
    }
}

/// The preview's own camera; the QA harness reports whether it is rendering.
#[derive(Component)]
pub struct PreviewCamera;

pub(super) fn setup_preview(mut commands: Commands, preview: Res<AvatarPreview>) {
    let image = preview.image.clone();
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -2,
            is_active: false,
            clear_color: super::PREVIEW_CLEAR,
            ..default()
        },
        RenderTarget::Image(image.into()),
        Transform::from_translation(PREVIEW_ORIGIN + Vec3::new(0.0, 1.05, 3.4))
            .looking_at(PREVIEW_ORIGIN + Vec3::new(0.0, 0.92, 0.0), Vec3::Y),
        RenderLayers::layer(PREVIEW_LAYER),
        PreviewCamera,
        Name::new("AvatarPreviewCamera"),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 9_000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(2.4, 3.2, 2.6).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
        RenderLayers::layer(PREVIEW_LAYER),
        Name::new("AvatarPreviewKeyLight"),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 3_200.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(-2.6, 1.8, -1.8).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
        RenderLayers::layer(PREVIEW_LAYER),
        Name::new("AvatarPreviewFillLight"),
    ));
}

/// Spawns the selected avatar under a pivot the turntable rotates. The model
/// root keeps its own transform so height normalization stays in charge of it.
fn sync_preview_model(
    mut commands: Commands,
    mut preview: ResMut<AvatarPreview>,
    mut models: PlayerModelResolver,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut pedestal: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>)>>,
) {
    // Poll independently: draining take_changed here would steal updates from
    // the in-match renderer. A Ready transition replaces the loading preview.
    let store_state = preview.slug.as_deref().and_then(|slug| {
        if omoba_passport::store::knows(slug) {
            Some(omoba_passport::store::model_state(slug))
        } else if preview.slug == preview.spawned_slug && preview.spawned_store_state.is_some() {
            // A successful catalogue refresh removed this SDK entry. Never
            // relabel a built-in fallback as the selected downloaded model.
            Some(omoba_passport::store::ModelState::Unavailable)
        } else {
            None
        }
    });
    if !preview_model_changed(
        &preview.slug,
        &preview.spawned_slug,
        &store_state,
        &preview.spawned_store_state,
    ) {
        return;
    }
    preview.spawned_store_state = store_state.clone();
    if let Some(pivot) = preview.pivot.take() {
        commands
            .entity(pivot)
            .despawn_related::<Children>()
            .despawn();
    }
    preview.model = None;
    preview.player = None;
    preview.gltf = None;
    preview.graph = None;
    preview.clips.clear();
    preview.gesture = None;
    preview.selected = 0;
    preview.bound = false;
    preview.scene_ready = false;
    preview.tag_frames = 0;
    preview.spawned_slug = preview.slug.clone();
    let Some(slug) = preview.slug.clone() else {
        preview.status = PreviewStatus::Empty;
        return;
    };
    match store_state {
        Some(omoba_passport::store::ModelState::Pending) => {
            preview.status = PreviewStatus::Loading;
            return;
        }
        Some(omoba_passport::store::ModelState::Unavailable) => {
            preview.status = PreviewStatus::Unavailable;
            return;
        }
        _ => {}
    }
    let (scene, gltf) = models.resolve(crate::team::CharacterChoice::default(), Some(&slug));
    let Some(scene) = scene else {
        preview.status = PreviewStatus::Empty;
        warn!("No preview model available for avatar '{slug}'");
        return;
    };
    let (mesh, material) = pedestal
        .get_or_insert_with(|| {
            (
                meshes.add(Cylinder::new(0.72, 0.06)),
                materials.add(StandardMaterial {
                    base_color: Color::srgb(0.10, 0.20, 0.21),
                    perceptual_roughness: 0.85,
                    ..default()
                }),
            )
        })
        .clone();
    let pivot = commands
        .spawn((
            Transform::from_translation(PREVIEW_ORIGIN),
            Visibility::Visible,
            RenderLayers::layer(PREVIEW_LAYER),
            Name::new("AvatarPreviewPivot"),
        ))
        .id();
    let model = commands
        .spawn((
            SceneRoot(scene),
            Transform::default(),
            Visibility::Visible,
            RenderLayers::layer(PREVIEW_LAYER),
            NormalizeModelScale::for_player_model(),
            Name::new(format!("AvatarPreviewModel-{slug}")),
        ))
        .id();
    if let Some(gltf) = gltf.clone() {
        commands.entity(model).insert(ModelScaleSource {
            gltf,
            key: model_scale_key(crate::team::CharacterChoice::default(), Some(&slug)),
        });
    }
    let base = commands
        .spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_xyz(0.0, -0.03, 0.0),
            RenderLayers::layer(PREVIEW_LAYER),
            Name::new("AvatarPreviewPedestal"),
            PreviewPedestal,
        ))
        .id();
    // The scene reports when every one of its entities exists. Until then the
    // tagging pass keeps running, however long the load takes.
    commands.entity(model).observe(
        |ready: On<bevy::scene::SceneInstanceReady>, mut preview: ResMut<AvatarPreview>| {
            if preview.model == Some(ready.entity) {
                preview.scene_ready = true;
                preview.tag_frames = TAG_FRAMES_AFTER_READY;
            }
        },
    );
    commands.entity(pivot).add_children(&[model, base]);
    preview.pivot = Some(pivot);
    preview.model = Some(model);
    preview.gltf = gltf;
    preview.status = PreviewStatus::Loading;
}

fn sync_preview_equipment(
    mut commands: Commands,
    preview: Res<AvatarPreview>,
    selection: Res<crate::team::TeamSelection>,
) {
    if let Some(model) = preview.model {
        commands.entity(model).insert((
            crate::net::NetworkHeroClass(selection.hero_class),
            crate::net::PlayerHandheld(selection.handheld.clone()),
        ));
    }
}

fn preview_model_changed(
    selected: &Option<String>,
    spawned: &Option<String>,
    state: &Option<omoba_passport::store::ModelState>,
    spawned_state: &Option<omoba_passport::store::ModelState>,
) -> bool {
    selected != spawned || state != spawned_state
}

/// glTF scene children spawn over several frames and do not inherit render
/// layers, so every descendant is tagged as it appears.
fn tag_preview_layers(
    mut commands: Commands,
    mut preview: ResMut<AvatarPreview>,
    children: Query<&Children>,
    tagged: Query<&RenderLayers>,
) {
    let Some(model) = preview.model else {
        return;
    };
    // Equipment may finish downloading long after the avatar scene loaded.
    // Tag late descendants too, so a new prop stays on the preview camera.
    if preview.scene_ready {
        preview.tag_frames = preview.tag_frames.saturating_sub(1);
    }
    let mut stack = vec![model];
    while let Some(entity) = stack.pop() {
        if tagged.get(entity).is_err() {
            commands
                .entity(entity)
                .insert(RenderLayers::layer(PREVIEW_LAYER));
        }
        if let Ok(children) = children.get(entity) {
            stack.extend(children.iter());
        }
    }
}

/// Humanoids share the match runtime; supported extra authored clips are
/// remapped into that namespace. Other models retain their embedded graph.
fn bind_preview_animations(
    mut commands: Commands,
    mut preview: ResMut<AvatarPreview>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    gltfs: Res<Assets<Gltf>>,
    asset_server: Res<AssetServer>,
    children: Query<&Children>,
    mut players: Query<(&mut AnimationPlayer, Option<&AnimationGraphHandle>)>,
    mut runtime: ResMut<crate::humanoid::HumanoidRuntimeLibrary>,
    mut clips: ResMut<Assets<AnimationClip>>,
    runtime_players: Query<&crate::humanoid::RuntimeHumanoidPlayer>,
) {
    if preview.model.is_none() {
        return;
    }
    if preview.bound {
        if let (Some(player), Some(graph)) = (preview.player, preview.graph.as_ref()) {
            if let Ok((mut active, current)) = players.get_mut(player) {
                if current.is_none_or(|current| current.0 != *graph) {
                    commands
                        .entity(player)
                        .insert(AnimationGraphHandle(graph.clone()));
                }
                if let Some(clip) = preview.selected_clip() {
                    if preview.gesture.is_none() && !active.is_playing_animation(clip.node) {
                        active.stop_all();
                        active.play(clip.node).repeat();
                    }
                }
                return;
            }
            preview.bound = false;
            preview.player = None;
            preview.graph = None;
        } else {
            return;
        }
    }
    if preview.gltf.as_ref().is_some_and(|handle| {
        matches!(
            asset_server.get_load_state(handle.id()),
            Some(bevy::asset::LoadState::Failed(_))
        )
    }) {
        preview.status = PreviewStatus::Unavailable;
        return;
    }
    let Some(gltf) = preview.gltf.clone().and_then(|handle| gltfs.get(&handle)) else {
        return;
    };
    let Some(model) = preview.model else {
        return;
    };
    // Humanoid preview uses the exact semantic motion/binding path used in
    // matches. Embedded clips remain a fallback for non-humanoid models.
    let runtime_clips = preview
        .gltf
        .as_ref()
        .and_then(|handle| runtime.ensure(handle, gltf, &mut clips).ok());
    if runtime_clips.is_some() {
        commands
            .entity(model)
            .insert(crate::humanoid::RuntimeHumanoidRequest {
                model: preview.gltf.as_ref().unwrap().clone(),
            });
        if runtime_players.get(model).is_err() {
            return;
        }
    }
    let mut stack = vec![model];
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
        // No `AnimationPlayer` in a finished scene: the model has no clips.
        if preview.scene_ready {
            preview.status = PreviewStatus::NoAnimations;
            preview.bound = true;
        }
        return;
    };
    let mut names: Vec<(String, Handle<AnimationClip>)> = if let Some(motion) = runtime_clips {
        let mut names: Vec<(String, Handle<AnimationClip>)> = vec![
            ("idle".into(), motion.idle),
            ("walk".into(), motion.walk),
            ("run".into(), motion.run),
            ("attack".into(), motion.attack),
            ("cast".into(), motion.cast),
            ("death".into(), motion.death),
        ];
        for (name, handle) in &gltf.named_animations {
            if names
                .iter()
                .any(|(known, _)| known.eq_ignore_ascii_case(name))
            {
                continue;
            }
            if let Some(clip) = clips.get(handle).cloned() {
                if let Ok(remapped) =
                    runtime.remap_embedded_clip(preview.gltf.as_ref().unwrap(), gltf, &clip)
                {
                    names.push((name.to_string(), clips.add(remapped)));
                }
            }
        }
        names
    } else {
        gltf.named_animations
            .iter()
            .map(|(name, handle)| (name.to_string(), handle.clone()))
            .collect()
    };
    if names.is_empty() {
        preview.status = PreviewStatus::NoAnimations;
        preview.bound = true;
        return;
    }
    // Idle first when the model has one: it is the pose players expect.
    names.sort_by(|left, right| {
        let rank = |name: &str| u8::from(!name.to_ascii_lowercase().contains("idle"));
        rank(&left.0).cmp(&rank(&right.0)).then_with(|| {
            left.0
                .to_ascii_lowercase()
                .cmp(&right.0.to_ascii_lowercase())
        })
    });
    let (graph, nodes) = AnimationGraph::from_clips(names.iter().map(|(_, handle)| handle.clone()));
    preview.clips = names
        .into_iter()
        .zip(nodes)
        .map(|((name, _), node)| PreviewClip { name, node })
        .collect();
    preview.selected = 0;
    let handle = graphs.add(graph);
    commands
        .entity(target)
        .insert(AnimationGraphHandle(handle.clone()));
    if let Ok((mut player, _)) = players.get_mut(target)
        && let Some(clip) = preview.clips.first()
    {
        player.stop_all();
        player.play(clip.node).repeat();
    }
    preview.graph = Some(handle);
    preview.player = Some(target);
    preview.bound = true;
    preview.status = PreviewStatus::Ready;
}

fn apply_clip_selection(
    mut preview: ResMut<AvatarPreview>,
    mut players: Query<&mut AnimationPlayer>,
    mut playing: Local<Option<(Entity, AnimationNodeIndex, u64)>>,
) {
    let Some(entity) = preview.player else {
        *playing = None;
        return;
    };
    let Ok(mut player) = players.get_mut(entity) else {
        return;
    };
    if let Some(index) = preview.gesture {
        if let Some(clip) = preview.clips.get(index) {
            if *playing == Some((entity, clip.node, preview.gesture_serial))
                && player
                    .animation(clip.node)
                    .is_some_and(|animation| animation.is_finished())
            {
                preview.gesture = None;
                if let Some(idle) = preview
                    .clips
                    .iter()
                    .position(|clip| clip.name.eq_ignore_ascii_case("idle"))
                {
                    preview.selected = idle;
                }
                *playing = None;
            }
        }
    }
    let Some(clip) = preview
        .gesture
        .and_then(|i| preview.clips.get(i))
        .or_else(|| preview.selected_clip())
    else {
        *playing = None;
        return;
    };
    let key = (entity, clip.node, preview.gesture_serial);
    if *playing == Some(key) {
        return;
    }
    player.stop_all();
    let animation = player.play(clip.node);
    if preview.gesture.is_none() {
        animation.repeat();
    }
    *playing = Some(key);
}

fn spin_preview(
    time: Res<Time>,
    mut preview: ResMut<AvatarPreview>,
    mut pivots: Query<&mut Transform>,
) {
    let Some(pivot) = preview.pivot else {
        return;
    };
    if preview.auto_spin {
        preview.yaw += time.delta_secs() * 0.5;
    }
    let yaw = preview.yaw;
    if let Ok(mut transform) = pivots.get_mut(pivot) {
        transform.rotation = Quat::from_rotation_y(yaw);
    }
}

/// The preview camera only renders while a screen is actually showing it.
fn toggle_preview_camera(
    screen: Res<State<AppScreen>>,
    preview: Res<AvatarPreview>,
    mut cameras: Query<&mut Camera, With<PreviewCamera>>,
    mut pedestals: Query<&mut Visibility, With<PreviewPedestal>>,
) {
    for mut visibility in &mut pedestals {
        *visibility = if *screen.get() == AppScreen::Home {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
    // Home, the collection and the picker all show the live model.
    let wanted = preview.slug.is_some()
        && matches!(
            *screen.get(),
            AppScreen::Home | AppScreen::Collection | AppScreen::HeroSelect
        );
    for mut camera in &mut cameras {
        if camera.is_active != wanted {
            camera.is_active = wanted;
        }
    }
}

const PREVIEW_FRAME_LIMIT: f32 = 0.9;

/// Exact rotational envelope of the authored vertices. A wing's radius must
/// not also become the radius of an imaginary foot at the same AABB corner.
struct PreviewFit {
    tan_y: f32,
    horizontal_factor: f32,
    distance: f32,
    vertices: usize,
}

impl PreviewFit {
    fn new(fov: f32, aspect: f32) -> Self {
        let tan_y = (fov * 0.5).tan();
        Self {
            tan_y,
            // max over yaw of z + |x| / horizontal_tangent, for a vertex
            // orbit of radius r, is r * sqrt(1 + 1 / tangent^2).
            horizontal_factor: (1.0 + (1.0 / (PREVIEW_FRAME_LIMIT * tan_y * aspect)).powi(2))
                .sqrt(),
            distance: 0.1,
            vertices: 0,
        }
    }

    /// The normalized model is grounded first, so y=0 is the platform plane.
    fn include(&mut self, point: Vec3) {
        if !point.is_finite() {
            return;
        }
        let ground_ndc = 2.0 * PREVIEW_GROUND_ANCHOR - 1.0;
        let radius = point.xz().length();
        // Solve the upper/lower frustum inequalities at this vertex's actual
        // height, for its nearest possible depth during a complete turn.
        self.distance = self
            .distance
            .max(radius * self.horizontal_factor)
            .max(
                (point.y / self.tan_y + PREVIEW_FRAME_LIMIT * radius)
                    / (PREVIEW_FRAME_LIMIT + ground_ndc),
            )
            .max(
                (PREVIEW_FRAME_LIMIT * radius - point.y / self.tan_y)
                    / (PREVIEW_FRAME_LIMIT - ground_ndc),
            );
        self.vertices += 1;
    }

    fn frame(&self) -> (Vec3, f32) {
        let distance = self.distance * 1.001;
        let camera_y = (2.0 * PREVIEW_GROUND_ANCHOR - 1.0) * distance * self.tan_y;
        (Vec3::new(0.0, camera_y, 0.0), distance)
    }
}

#[derive(PartialEq)]
struct PreviewFrameKey {
    model: Entity,
    asset: AssetId<Gltf>,
    transform: Transform,
    fov: f32,
    aspect: f32,
}

struct PreviewFrameCache {
    key: PreviewFrameKey,
    frame: (Vec3, f32),
}

fn frame_preview_camera(
    preview: Res<AvatarPreview>,
    gltfs: Res<Assets<Gltf>>,
    nodes: Res<Assets<bevy::gltf::GltfNode>>,
    gltf_meshes: Res<Assets<bevy::gltf::GltfMesh>>,
    meshes: Res<Assets<Mesh>>,
    mut models: Query<(&mut Transform, &NormalizeModelScale), Without<PreviewCamera>>,
    mut cameras: Query<(&mut Transform, &Projection), With<PreviewCamera>>,
    mut cached: Local<Option<PreviewFrameCache>>,
) {
    let (Some(model), Some(handle)) = (preview.model, preview.gltf.as_ref()) else {
        return;
    };
    let Ok((mut model_transform, normalization)) = models.get_mut(model) else {
        return;
    };
    let Some(foot_y) = normalization.foot_local_y() else {
        return;
    };
    // Imported models can have a centered or offset origin. Ground them at the
    // preview pivot, just as the match renderer does, without changing scale or
    // applying an avatar-specific correction. This also aligns the 3D pedestal.
    if model_transform.translation.y != -foot_y {
        model_transform.translation.y = -foot_y;
    }
    for (mut transform, projection) in &mut cameras {
        let Projection::Perspective(projection) = projection else {
            continue;
        };
        let key = PreviewFrameKey {
            model,
            asset: handle.id(),
            transform: *model_transform,
            fov: projection.fov,
            aspect: PREVIEW_WIDTH as f32 / PREVIEW_HEIGHT as f32,
        };
        if cached.as_ref().is_none_or(|cache| cache.key != key) {
            let Some(gltf) = gltfs.get(handle) else {
                continue;
            };
            let mut fit = PreviewFit::new(key.fov, key.aspect);
            if crate::model_scale::visit_gltf_mesh_vertices(
                gltf,
                &nodes,
                &gltf_meshes,
                &meshes,
                |point| fit.include(key.transform.transform_point(point)),
            )
            .is_none()
                || fit.vertices == 0
            {
                continue;
            }
            *cached = Some(PreviewFrameCache {
                key,
                frame: fit.frame(),
            });
        }
        let (center, distance) = cached.as_ref().unwrap().frame;
        let center = PREVIEW_ORIGIN + center;
        *transform =
            Transform::from_translation(center + Vec3::Z * distance).looking_at(center, Vec3::Y);
    }
}

#[derive(Component)]
struct PreviewPedestal;

/// The model only exists while a screen shows it. A match never carries a
/// preview avatar along, and the card editor does not pay for one either.
fn release_preview_off_screen(screen: Res<State<AppScreen>>, mut preview: ResMut<AvatarPreview>) {
    if !screen.is_changed() {
        return;
    }
    let shown = matches!(
        *screen.get(),
        AppScreen::Home | AppScreen::Collection | AppScreen::HeroSelect
    );
    if !shown && preview.slug.is_some() {
        preview.slug = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_vertices(min: Vec3, max: Vec3) -> Vec<Vec3> {
        let mut vertices = Vec::new();
        for x in [min.x, max.x] {
            for y in [min.y, max.y] {
                for z in [min.z, max.z] {
                    vertices.push(Vec3::new(x, y - min.y, z));
                }
            }
        }
        vertices
    }

    fn fit_vertices(vertices: &[Vec3], fov: f32, aspect: f32) -> (Vec3, f32) {
        let mut fit = PreviewFit::new(fov, aspect);
        for point in vertices {
            fit.include(*point);
        }
        fit.frame()
    }

    fn assert_full_rotation_fits(vertices: &[Vec3], fov: f32, aspect: f32) {
        let (center, distance) = fit_vertices(vertices, fov, aspect);
        let ground_y = -center.y / (distance * (fov * 0.5).tan());
        assert!(((1.0 - ground_y) * 0.5 - PREVIEW_GROUND_ANCHOR).abs() < 1e-6);
        for degrees in 0..360 {
            let rotation = Quat::from_rotation_y((degrees as f32).to_radians());
            for vertex in vertices {
                let point = rotation * *vertex - center;
                let depth = distance - point.z;
                assert!(depth > 0.0);
                assert!(point.x.abs() / (depth * (fov * 0.5).tan() * aspect) < 0.9);
                assert!(point.y.abs() / (depth * (fov * 0.5).tan()) < 0.9);
            }
        }
    }

    #[test]
    fn framing_contains_tall_wide_and_offset_avatars_at_every_rotation() {
        let fov = std::f32::consts::FRAC_PI_4;
        let aspect = PREVIEW_WIDTH as f32 / PREVIEW_HEIGHT as f32;
        for (min, max) in [
            (Vec3::new(-0.4, 0.0, -0.2), Vec3::new(0.4, 3.6, 0.2)),
            (Vec3::new(-2.0, 0.0, -0.5), Vec3::new(2.0, 2.1, 0.5)),
            (Vec3::new(-0.3, -1.2, -0.6), Vec3::new(1.4, 1.5, 0.4)),
            (Vec3::new(-3.0, 0.0, -1.5), Vec3::new(3.0, 1.2, 1.5)),
        ] {
            assert_full_rotation_fits(&box_vertices(min, max), fov, aspect);
        }
    }

    #[test]
    fn broad_upper_silhouette_with_narrow_feet_stays_large_and_grounded() {
        // Wings belong to the upper body, not to a box stretched down to the
        // feet. This catches the overly distant full-cylinder lower constraint.
        let vertices = [
            Vec3::new(-0.1, 0.0, -0.09),
            Vec3::new(0.1, 0.0, 0.09),
            Vec3::new(-1.075, 1.5, 0.1),
            Vec3::new(1.075, 1.5, 0.1),
            Vec3::new(0.0, 1.7, 0.793),
            Vec3::new(0.0, 2.1, 0.0),
        ];
        for fov in [35_f32, 45.0, 60.0].map(f32::to_radians) {
            for aspect in [PREVIEW_WIDTH as f32 / PREVIEW_HEIGHT as f32, 1.0] {
                assert_full_rotation_fits(&vertices, fov, aspect);
            }
        }
        let (camera, distance) = fit_vertices(
            &vertices,
            std::f32::consts::FRAC_PI_4,
            PREVIEW_WIDTH as f32 / PREVIEW_HEIGHT as f32,
        );
        assert!(distance < 4.3, "upper wings must not invent wide feet");
        let projected_height = 2.1 / (distance * (std::f32::consts::PI / 8.0).tan()) * 125.0;
        assert!(
            projected_height > 145.0,
            "readable silhouette in the 250px Home panel"
        );
        assert!((camera.y / distance / (std::f32::consts::PI / 8.0).tan() - 0.74).abs() < 1e-6);
    }

    #[test]
    fn fitted_feet_stay_on_home_platform_on_phone_and_desktop() {
        let fov = std::f32::consts::FRAC_PI_4;
        let aspect = PREVIEW_WIDTH as f32 / PREVIEW_HEIGHT as f32;
        // Home centers the image inside its authored panel; the fractional
        // image dimensions introduce less than one logical pixel of slack.
        for (panel_height, image_width) in [(250.0, 185.0), (440.0, 326.0)] {
            let image_height = image_width / 0.742;
            let stage_y = 500.0;
            let panel_top = stage_y - panel_height * PREVIEW_GROUND_ANCHOR;
            let image_top = panel_top + (panel_height - image_height) * 0.5;
            for (height, width, depth, bottom) in [
                (2.3, 0.8, 0.4, 0.0),
                (2.3, 4.0, 1.0, 0.0),
                (4.6, 1.0, 0.8, -1.2),
            ] {
                let min = Vec3::new(-width * 0.5, bottom, -depth * 0.5);
                let max = Vec3::new(width * 0.5, bottom + height, depth * 0.5);
                let (camera, distance) = fit_vertices(&box_vertices(min, max), fov, aspect);
                let foot_ndc = -camera.y / (distance * (fov * 0.5).tan());
                let foot_y = image_top + (1.0 - foot_ndc) * 0.5 * image_height;
                assert!(
                    (foot_y - stage_y).abs() < 0.5,
                    "feet must meet the platform after fitting {width}x{height}: {foot_y} vs {stage_y}"
                );
            }
        }
    }

    #[test]
    fn greeting_selects_real_expressive_clips_and_can_replay() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let mut preview = AvatarPreview::from_world(&mut world);
        preview.clips = ["idle", "walk", "death", "attack", "cast"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| PreviewClip {
                name: name.into(),
                node: AnimationNodeIndex::new(index),
            })
            .collect();
        let mut selected = std::collections::HashSet::new();
        for _ in 0..16 {
            preview.greet(123);
            let index = preview.gesture.unwrap();
            assert!(index == 3 || index == 4);
            selected.insert(index);
        }
        assert_eq!(selected.len(), 2);
        assert_eq!(preview.gesture_serial, 16);
        preview.clips.truncate(3);
        preview.gesture = None;
        preview.greet(123);
        assert_eq!(preview.gesture, None);
    }

    #[test]
    fn sdk_preview_refreshes_without_consuming_match_change_queue() {
        use omoba_passport::store::ModelState;
        let slug = Some("approved-sdk-avatar".to_string());
        assert!(preview_model_changed(
            &slug,
            &slug,
            &Some(ModelState::Ready),
            &Some(ModelState::Pending)
        ));
        assert!(!preview_model_changed(
            &slug,
            &slug,
            &Some(ModelState::Ready),
            &Some(ModelState::Ready)
        ));
        assert!(preview_model_changed(
            &slug,
            &slug,
            &Some(ModelState::Ready),
            &None
        ));
    }

    #[test]
    fn avatar_selection_starts_stationary_and_facing_camera() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let mut preview = AvatarPreview::from_world(&mut world);
        preview.show("one");
        assert_eq!(preview.yaw, std::f32::consts::PI);
        assert!(!preview.auto_spin);
        preview.yaw = 0.5;
        preview.auto_spin = true;
        preview.show("two");
        assert_eq!(preview.yaw, std::f32::consts::PI);
        assert!(!preview.auto_spin);
    }

    #[test]
    fn clip_labels_are_readable() {
        assert_eq!(AvatarPreview::clip_label("walk_cycle"), "Walk Cycle");
        assert_eq!(AvatarPreview::clip_label("idle"), "Idle");
        assert_eq!(AvatarPreview::clip_label(""), "");
    }

    #[test]
    fn the_preview_layer_is_not_the_supporter_layer() {
        assert_ne!(PREVIEW_LAYER, 29);
    }
}
