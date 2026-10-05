//! Shared live 3D formation for the party lobby and authoritative pre-match team.
//!
//! Callers put the viewer first and supply the actual members and their selected
//! models. The image is transparent, with isolated lighting and render layers;
//! dragging a hero turns only that hero without moving anyone out of formation.
//! No player-facing text: screens label the team and its interaction.
// i18n-strict
use bevy::camera::{RenderTarget, visibility::RenderLayers};
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::window::PrimaryWindow;
use omoba_passport::store::ModelState;

use super::AppScreen;
use crate::model_scale::{ModelScaleSource, NormalizeModelScale, model_scale_key};
use crate::team::CharacterChoice;
use crate::world::PlayerModelResolver;

/// The avatar preview owns layer 28; the supporter aura preview owns 29.
pub const STAGE_LAYER: usize = 27;
pub const STAGE_WIDTH: u32 = 1280;
pub const STAGE_HEIGHT: u32 = 720;
const STAGE_ORIGIN: Vec3 = Vec3::new(40.0, -2000.0, 0.0);
const PLINTH_TOP: f32 = 0.18;
const DRAG_RADIANS_PER_PIXEL: f32 = 0.012;
const STAGE_FOV: f32 = std::f32::consts::FRAC_PI_4;

/// One real member. Index zero is always the viewer, independently of who leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageMember {
    pub hero_class: shared::HeroClass,
    pub handheld: shared::handheld::HandheldSelection,
    pub avatar: Option<String>,
    pub character: CharacterChoice,
    pub leader: bool,
    /// Lobby members and the viewer's own draft preview are revealed. Other
    /// draft members reveal their accepted hero when they lock their choice.
    pub revealed: bool,
}

/// Attach to the displayed stage `ImageNode` to enable mouse/touch rotation.
#[derive(Component)]
pub struct StageSurface;

struct Slot {
    root: Entity,
    pivot: Entity,
    model: Entity,
    gltf: Option<Handle<Gltf>>,
    bound: bool,
    idle: Option<IdleBinding>,
}

struct IdleBinding {
    player: Entity,
    graph: Handle<AnimationGraph>,
    node: AnimationNodeIndex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StageSnapshot {
    members: Vec<StageMember>,
    store_states: Vec<Option<ModelState>>,
}

impl StageSnapshot {
    /// Revealing a confirmed hero changes visibility, not scene identity:
    /// keep the already-loaded model and its live animation player intact.
    fn same_models(&self, other: &Self) -> bool {
        self.store_states == other.store_states
            && self.members.len() == other.members.len()
            && self.members.iter().zip(&other.members).all(|(a, b)| {
                a.avatar == b.avatar && a.character == b.character && a.leader == b.leader
            })
    }
}

#[derive(Resource)]
pub struct PartyStage {
    pub image: Handle<Image>,
    /// Caller-owned identity order: viewer, inner left/right, outer left/right.
    pub members: Vec<StageMember>,
    /// Independent turntables in the caller-owned identity order.
    pub yaws: [f32; shared::party::MAX_PARTY_SIZE],
    spawned: Option<StageSnapshot>,
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
            yaws: [std::f32::consts::PI; shared::party::MAX_PARTY_SIZE],
            spawned: None,
            slots: Vec::new(),
        }
    }
}

pub struct PartyStagePlugin;

impl Plugin for PartyStagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PartyStage>()
            .init_resource::<crate::humanoid::HumanoidRuntimeLibrary>()
            .add_systems(Startup, setup_stage)
            .add_systems(
                Update,
                (
                    release_stage_off_screen,
                    sync_stage,
                    sync_stage_equipment,
                    sync_model_visibility,
                    tag_stage_layers,
                    interact,
                    turn_and_ground_heroes,
                    frame_stage,
                    toggle_stage_camera,
                )
                    .chain(),
            )
            .add_systems(
                // WorldInstanceSpawner may replace imported players after Update.
                // Validate the binding after those writes, before pose evaluation.
                PostUpdate,
                play_idle
                    .after(crate::humanoid::bind_runtime_humanoids)
                    .before(bevy::app::AnimationSystems),
            );
    }
}

#[derive(Component)]
pub struct StageCamera;

pub(super) fn setup_stage(mut commands: Commands, stage: Res<PartyStage>) {
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: STAGE_FOV,
            ..default()
        }),
        Camera {
            order: -3,
            is_active: false,
            clear_color: super::PREVIEW_CLEAR,
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
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_translation(from).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
            RenderLayers::layer(STAGE_LAYER),
            Name::new("PartyStageLight"),
        ));
    }
}

fn stage_screen(screen: AppScreen) -> bool {
    matches!(
        screen,
        AppScreen::Lobby | AppScreen::Draft | AppScreen::Loading
    )
}

/// A shallow chevron keeps bodies separate in perspective, including all five
/// supported seats. The viewer never shifts when teammates arrive or leave.
pub fn slot_position(index: usize, _count: usize) -> Vec3 {
    if index == 0 {
        return Vec3::new(0.0, 0.0, 0.40);
    }
    let rank = index.div_ceil(2) as f32;
    let side = if index % 2 == 1 { -1.0 } else { 1.0 };
    Vec3::new(side * (1.30 + (rank - 1.0) * 1.45), 0.0, -0.75 * rank)
}

/// Screen-space hero feet in the rendered image, with `(0, 0)` at its top
/// left. Place identity labels relative to these anchors so two/four-member
/// formations keep the viewer label centred and empty seats stay empty.
pub fn slot_anchor(index: usize, count: usize) -> Vec2 {
    let view = camera_transform(count).to_matrix().inverse();
    let projection = Mat4::perspective_rh(
        STAGE_FOV,
        STAGE_WIDTH as f32 / STAGE_HEIGHT as f32,
        0.1,
        100.0,
    );
    project_slot(index, count, 0.0, view, projection)
}

fn project_slot(index: usize, count: usize, height: f32, view: Mat4, projection: Mat4) -> Vec2 {
    let feet = STAGE_ORIGIN + slot_position(index, count) + Vec3::Y * (PLINTH_TOP + height);
    let projected = projection.project_point3(view.transform_point3(feet));
    Vec2::new(0.5 + projected.x * 0.5, 0.5 - projected.y * 0.5)
}

/// Select the closest revealed body under the pointer. Width is limited by
/// neighbouring projected feet so an adjacent hero never rotates by accident.
fn touched_slot(position: Vec2, rect: Rect, members: &[StageMember]) -> Option<usize> {
    if !rect.contains(position) || rect.size().min_element() <= 0.0 {
        return None;
    }
    let point = (position - rect.min) / rect.size();
    let count = members.len();
    let view = camera_transform(count).to_matrix().inverse();
    let projection = Mat4::perspective_rh(
        STAGE_FOV,
        STAGE_WIDTH as f32 / STAGE_HEIGHT as f32,
        0.1,
        100.0,
    );
    members
        .iter()
        .enumerate()
        .filter(|(_, member)| member.revealed)
        .filter_map(|(index, _)| {
            let feet = slot_anchor(index, count);
            let head = project_slot(
                index,
                count,
                crate::model_scale::DEFAULT_MODEL_TARGET_HEIGHT,
                view,
                projection,
            );
            let half_width = (0..count)
                .filter(|other| *other != index)
                .map(|other| (slot_anchor(other, count).x - feet.x).abs() * 0.5)
                .fold(0.13_f32, f32::min);
            (point.x >= feet.x - half_width
                && point.x <= feet.x + half_width
                && point.y >= head.y - 0.03
                && point.y <= feet.y + 0.03)
                .then_some((index, (point.x - feet.x).abs()))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

/// A close, gently elevated camera gives the main hero prominence; the wider
/// outer pair needs only a small retreat, rather than shrinking the whole team.
fn camera_transform(members: usize) -> Transform {
    let distance = if members > 3 { 4.25 } else { 3.85 };
    Transform::from_translation(STAGE_ORIGIN + Vec3::new(0.0, 1.75, distance))
        .looking_at(STAGE_ORIGIN + Vec3::new(0.0, 1.08, 0.0), Vec3::Y)
}

fn release_stage_off_screen(screen: Res<State<AppScreen>>, mut stage: ResMut<PartyStage>) {
    if !stage_screen(*screen.get()) && !stage.members.is_empty() {
        stage.members.clear();
        stage.yaws.fill(std::f32::consts::PI);
    }
}

/// Track model installation independently. Consuming the store change queue
/// here would steal updates from the in-match renderer and collection preview.
fn desired_snapshot(stage: &PartyStage) -> StageSnapshot {
    let store_states = stage
        .members
        .iter()
        .map(|member| {
            let slug = member.avatar.as_deref()?;
            if omoba_passport::store::knows(slug) {
                Some(omoba_passport::store::model_state(slug))
            } else if stage.spawned.as_ref().is_some_and(|previous| {
                previous
                    .members
                    .iter()
                    .zip(&previous.store_states)
                    .any(|(old, state)| old.avatar == member.avatar && state.is_some())
            }) {
                Some(ModelState::Unavailable)
            } else {
                None
            }
        })
        .collect();
    StageSnapshot {
        members: stage.members.clone(),
        store_states,
    }
}

struct PlinthAssets {
    base: Handle<Mesh>,
    trim: Handle<Mesh>,
    crown: Handle<Mesh>,
    stone: Handle<StandardMaterial>,
    top: Handle<StandardMaterial>,
    jade: Handle<StandardMaterial>,
    gold: Handle<StandardMaterial>,
}

impl PlinthAssets {
    fn new(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Self {
        let trim_material = |color| StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * 0.35,
            metallic: 0.55,
            perceptual_roughness: 0.35,
            ..default()
        };
        Self {
            base: meshes.add(Cylinder::new(0.66, 0.08).mesh().resolution(8)),
            trim: meshes.add(Cylinder::new(0.61, 0.035).mesh().resolution(8)),
            crown: meshes.add(Cylinder::new(0.56, 0.065).mesh().resolution(8)),
            stone: materials.add(StandardMaterial {
                base_color: Color::srgb(0.055, 0.115, 0.12),
                metallic: 0.30,
                perceptual_roughness: 0.6,
                ..default()
            }),
            top: materials.add(StandardMaterial {
                base_color: Color::srgb(0.12, 0.23, 0.23),
                metallic: 0.18,
                perceptual_roughness: 0.72,
                ..default()
            }),
            jade: materials.add(trim_material(Color::srgb(0.22, 0.66, 0.51))),
            gold: materials.add(trim_material(Color::srgb(0.76, 0.58, 0.28))),
        }
    }
}

fn sync_stage(
    mut commands: Commands,
    mut stage: ResMut<PartyStage>,
    mut models: PlayerModelResolver,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut plinth: Local<Option<PlinthAssets>>,
) {
    let snapshot = desired_snapshot(&stage);
    if stage
        .spawned
        .as_ref()
        .is_some_and(|previous| previous.same_models(&snapshot))
    {
        // A legacy catalogue can finish loading after the screen opens. An
        // empty pedestal must retry that missing scene without rebuilding the
        // formation every frame while the catalogue is still unavailable.
        let scene_arrived = stage
            .slots
            .iter()
            .zip(&snapshot.members)
            .any(|(slot, member)| {
                slot.model == slot.pivot
                    && models
                        .resolve(member.character, member.avatar.as_deref())
                        .0
                        .is_some()
            });
        if !scene_arrived {
            if stage.spawned.as_ref() != Some(&snapshot) {
                stage.spawned = Some(snapshot);
            }
            return;
        }
    }
    for slot in stage.slots.drain(..) {
        commands
            .entity(slot.root)
            .despawn_related::<Children>()
            .despawn();
    }
    let pedestal = plinth.get_or_insert_with(|| PlinthAssets::new(&mut meshes, &mut materials));
    for (index, member) in snapshot.members.iter().enumerate() {
        let root = commands
            .spawn((
                Transform::from_translation(
                    STAGE_ORIGIN + slot_position(index, snapshot.members.len()),
                ),
                Visibility::Visible,
                RenderLayers::layer(STAGE_LAYER),
                Name::new(format!("PartyStageSlot{index}")),
            ))
            .id();
        for (mesh, material, y) in [
            (&pedestal.base, &pedestal.stone, 0.04),
            (
                &pedestal.trim,
                if member.leader {
                    &pedestal.gold
                } else {
                    &pedestal.jade
                },
                0.0975,
            ),
            (&pedestal.crown, &pedestal.top, 0.1475),
        ] {
            commands.entity(root).with_child((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(0.0, y, 0.0),
                RenderLayers::layer(STAGE_LAYER),
            ));
        }
        let pivot = commands
            .spawn((
                Transform::from_xyz(0.0, PLINTH_TOP, 0.0)
                    .with_rotation(Quat::from_rotation_y(stage.yaws[index])),
                Visibility::Visible,
                RenderLayers::layer(STAGE_LAYER),
                Name::new(format!("PartyStageHeroPivot{index}")),
            ))
            .id();
        commands.entity(root).add_child(pivot);
        let (scene, gltf) = models.resolve(member.character, member.avatar.as_deref());
        let Some(scene) = scene else {
            stage.slots.push(Slot {
                root,
                pivot,
                model: pivot,
                gltf: None,
                bound: true,
                idle: None,
            });
            continue;
        };
        let model = commands
            .spawn((
                WorldAssetRoot(scene),
                Transform::default(),
                if member.revealed {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                },
                RenderLayers::layer(STAGE_LAYER),
                NormalizeModelScale::for_player_model(),
                Name::new(format!("PartyStageModel{index}")),
            ))
            .id();
        if let Some(gltf) = gltf.clone() {
            commands.entity(model).insert(ModelScaleSource {
                gltf,
                key: model_scale_key(member.character, member.avatar.as_deref()),
            });
        }
        commands.entity(pivot).add_child(model);
        stage.slots.push(Slot {
            root,
            pivot,
            model,
            gltf,
            bound: false,
            idle: None,
        });
    }
    stage.spawned = Some(snapshot);
}

/// Equipment changes reuse the existing rig instead of restarting its idle.
fn sync_stage_equipment(
    mut commands: Commands,
    stage: Res<PartyStage>,
    screen: Res<State<AppScreen>>,
    selection: Res<crate::team::TeamSelection>,
) {
    for (index, (slot, member)) in stage.slots.iter().zip(&stage.members).enumerate() {
        if slot.model == slot.pivot {
            continue;
        }
        let (class, handheld) = if *screen.get() == AppScreen::Lobby && index == 0 {
            (selection.hero_class, &selection.handheld)
        } else {
            (member.hero_class, &member.handheld)
        };
        commands.entity(slot.model).insert((
            crate::net::NetworkHeroClass(class),
            crate::net::PlayerHandheld(handheld.clone()),
        ));
    }
}

/// Conceal only unconfirmed heroes. Plinths remain visible and hidden scenes
/// continue loading/animating, so lock-in reveals the same warmed-up instance.
fn sync_model_visibility(stage: Res<PartyStage>, mut visibility: Query<&mut Visibility>) {
    for (slot, member) in stage.slots.iter().zip(&stage.members) {
        if slot.model == slot.pivot {
            continue; // This slot is still waiting for its first scene.
        }
        if let Ok(mut current) = visibility.get_mut(slot.model) {
            let wanted = if member.revealed {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
            if *current != wanted {
                *current = wanted;
            }
        }
    }
}

/// glTF children spawn over several frames and do not inherit render layers.
fn tag_stage_layers(
    mut commands: Commands,
    stage: Res<PartyStage>,
    screen: Res<State<AppScreen>>,
    children: Query<&Children>,
    tagged: Query<&RenderLayers>,
) {
    if !stage_screen(*screen.get()) {
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

/// Idle when available, else a stable first clip. Every model owns its player.
fn play_idle(
    mut commands: Commands,
    mut stage: ResMut<PartyStage>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    gltfs: Res<Assets<Gltf>>,
    children: Query<&Children>,
    mut players: Query<(&mut AnimationPlayer, Option<&AnimationGraphHandle>)>,
    mut runtime: ResMut<crate::humanoid::HumanoidRuntimeLibrary>,
    mut clips: ResMut<Assets<AnimationClip>>,
    runtime_players: Query<&crate::humanoid::RuntimeHumanoidPlayer>,
) {
    for slot in &mut stage.slots {
        if slot.bound {
            let Some(idle) = &slot.idle else {
                continue; // A fully loaded model with no animation clips.
            };
            if let Ok((mut player, graph)) = players.get_mut(idle.player) {
                if graph.is_none_or(|graph| graph.0 != idle.graph) {
                    commands
                        .entity(idle.player)
                        .insert(AnimationGraphHandle(idle.graph.clone()));
                }
                if !player.is_playing_animation(idle.node) {
                    player.stop_all();
                    player.play(idle.node).repeat();
                }
                continue;
            }
            // A scene refresh replaced the player entity. Search the current
            // descendants instead of permanently trusting the old bound flag.
            slot.bound = false;
            slot.idle = None;
        }
        let Some(gltf) = slot.gltf.as_ref().and_then(|h| gltfs.get(h)) else {
            continue;
        };
        let runtime_idle = slot
            .gltf
            .as_ref()
            .and_then(|handle| runtime.ensure(handle, gltf, &mut clips).ok())
            .map(|motion| motion.idle);
        if runtime_idle.is_some() {
            commands
                .entity(slot.model)
                .insert(crate::humanoid::RuntimeHumanoidRequest {
                    model: slot.gltf.as_ref().unwrap().clone(),
                });
            if runtime_players.get(slot.model).is_err() {
                continue;
            }
        } else if gltf.animations.is_empty() {
            slot.bound = true;
            continue;
        }
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
        let clip = runtime_idle.or_else(|| {
            gltf.named_animations
                .iter()
                .min_by_key(|(name, _)| {
                    let lower = name.to_ascii_lowercase();
                    (!lower.contains("idle"), lower)
                })
                .map(|(_, clip)| clip.clone())
                .or_else(|| gltf.animations.first().cloned())
        });
        let Some(clip) = clip else {
            continue;
        };
        let (graph, node) = AnimationGraph::from_clip(clip);
        let graph = graphs.add(graph);
        commands
            .entity(target)
            .insert(AnimationGraphHandle(graph.clone()));
        if let Ok((mut player, _)) = players.get_mut(target) {
            player.stop_all();
            player.play(node).repeat();
        }
        slot.idle = Some(IdleBinding {
            player: target,
            graph,
            node,
        });
    }
}

#[derive(Default)]
struct StageDrag {
    held: Option<(Option<u64>, Vec2)>,
    slot: Option<usize>,
    viewport: Option<Vec2>,
    screen: Option<AppScreen>,
}

impl StageDrag {
    fn begin(&mut self, pointer: Option<u64>, position: Vec2, slot: Option<usize>) {
        if self.held.is_none() && slot.is_some() && position.is_finite() {
            self.held = Some((pointer, position));
            self.slot = slot;
        }
    }

    fn moved(&mut self, pointer: Option<u64>, position: Vec2) -> f32 {
        let Some((owner, previous)) = self.held else {
            return 0.0;
        };
        if owner != pointer || !position.is_finite() {
            return 0.0;
        }
        self.held = Some((owner, position));
        (position.x - previous.x) * DRAG_RADIANS_PER_PIXEL
    }

    fn end(&mut self, pointer: Option<u64>) {
        if self.held.is_some_and(|(owner, _)| owner == pointer) {
            self.held = None;
            self.slot = None;
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn interact(
    mut stage: ResMut<PartyStage>,
    screen: Res<State<AppScreen>>,
    mut gesture: Local<StageDrag>,
    mut touches: MessageReader<TouchInput>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    surfaces: Query<
        (
            &ComputedNode,
            &UiGlobalTransform,
            Option<&bevy::ui::CalculatedClip>,
            Option<&InheritedVisibility>,
        ),
        With<StageSurface>,
    >,
    modals: Option<Res<crate::ui::ModalStack>>,
) {
    let Ok(window) = windows.single() else {
        touches.clear();
        gesture.held = None;
        return;
    };
    let viewport = Vec2::new(window.width(), window.height());
    if gesture.viewport != Some(viewport) || gesture.screen != Some(*screen.get()) {
        gesture.held = None;
        gesture.viewport = Some(viewport);
        gesture.screen = Some(*screen.get());
    }
    if !window.focused
        || !stage_screen(*screen.get())
        || surfaces.is_empty()
        || modals.as_ref().is_some_and(|stack| stack.is_open())
    {
        touches.clear();
        gesture.held = None;
        return;
    }
    let members = stage.members.clone();
    let slot_at = |position| {
        surfaces
            .iter()
            .find_map(|(node, transform, clip, visible)| {
                if visible.is_some_and(|visibility| !visibility.get()) {
                    return None;
                }
                let rect = crate::ui::gesture::logical_ui_rect(
                    node,
                    transform,
                    clip,
                    window.scale_factor(),
                );
                if !rect.contains(position) {
                    return None;
                }
                // Projection coordinates belong to the complete render image;
                // clipping controls admission, never its coordinate system.
                let image_rect = crate::ui::gesture::logical_ui_rect(
                    node,
                    transform,
                    None,
                    window.scale_factor(),
                );
                touched_slot(position, image_rect, &members)
            })
    };
    let mut touched = false;
    for event in touches.read() {
        touched = true;
        match event.phase {
            TouchPhase::Started => {
                gesture.begin(Some(event.id), event.position, slot_at(event.position))
            }
            TouchPhase::Moved => {
                let delta = gesture.moved(Some(event.id), event.position);
                if let Some(slot) = gesture.slot {
                    stage.yaws[slot] += delta;
                }
            }
            TouchPhase::Ended | TouchPhase::Canceled => gesture.end(Some(event.id)),
        }
    }
    if !touched && let Some(mouse) = mouse {
        if mouse.just_released(MouseButton::Left) {
            gesture.end(None);
        } else if let Some(position) = window.cursor_position() {
            if mouse.just_pressed(MouseButton::Left) {
                gesture.begin(None, position, slot_at(position));
            } else if mouse.pressed(MouseButton::Left) {
                let delta = gesture.moved(None, position);
                if let Some(slot) = gesture.slot {
                    stage.yaws[slot] += delta;
                }
            }
        } else {
            gesture.end(None);
        }
    }
    for yaw in &mut stage.yaws {
        *yaw = yaw.rem_euclid(std::f32::consts::TAU);
    }
}

fn turn_and_ground_heroes(
    stage: Res<PartyStage>,
    mut transforms: Query<&mut Transform>,
    normalized: Query<&NormalizeModelScale>,
) {
    for (index, slot) in stage.slots.iter().enumerate() {
        if let Ok(mut transform) = transforms.get_mut(slot.pivot) {
            transform.rotation = Quat::from_rotation_y(stage.yaws[index]);
        }
        if let Ok(normalized) = normalized.get(slot.model)
            && let Some(feet) = normalized.foot_local_y()
            && let Ok(mut transform) = transforms.get_mut(slot.model)
        {
            transform.translation.y = -feet;
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
    surfaces: Query<(), With<StageSurface>>,
    mut cameras: Query<&mut Camera, With<StageCamera>>,
) {
    let wanted = stage_screen(*screen.get()) && !stage.members.is_empty() && !surfaces.is_empty();
    for mut camera in &mut cameras {
        if camera.is_active != wanted {
            camera.is_active = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member() -> StageMember {
        StageMember {
            hero_class: Default::default(),
            handheld: shared::handheld::HandheldSelection::Unequipped,
            avatar: Some("agnes".into()),
            character: CharacterChoice::default(),
            leader: false,
            revealed: true,
        }
    }

    #[test]
    fn viewer_stays_centre_front_as_the_full_team_arrives() {
        let viewer = slot_position(0, 1);
        assert_eq!(viewer.x, 0.0);
        for count in 2..=5 {
            assert_eq!(viewer, slot_position(0, count));
            for index in 1..count {
                let position = slot_position(index, count);
                assert!(position.z < viewer.z);
                assert!(position.x.abs() >= 1.3);
            }
        }
        for (left, right) in [(1, 2), (3, 4)] {
            let left = slot_position(left, 5);
            let right = slot_position(right, 5);
            assert_eq!(left.x, -right.x);
            assert_eq!(left.z, right.z);
        }
        assert!(slot_position(3, 5).x < slot_position(1, 5).x);
        assert!(slot_position(3, 5).z < slot_position(1, 5).z);
    }

    #[test]
    fn projected_full_height_bodies_fit_without_overlap_for_one_to_five_members() {
        let projection = Mat4::perspective_rh(
            STAGE_FOV,
            STAGE_WIDTH as f32 / STAGE_HEIGHT as f32,
            0.1,
            100.0,
        );
        for count in 1..=5 {
            let view = camera_transform(count).to_matrix().inverse();
            let mut body_edges = Vec::new();
            for index in [3, 1, 0, 2, 4].into_iter().filter(|&index| index < count) {
                let origin = STAGE_ORIGIN + slot_position(index, count);
                let project = |offset: Vec3| {
                    projection.project_point3(view.transform_point3(origin + offset))
                };
                // A one-unit-wide body envelope, with headroom above the
                // normalized hero and the stepped platform beneath its feet.
                for x in [-0.5, 0.5] {
                    for y in [0.0, 2.4] {
                        let point = project(Vec3::new(x, y, 0.0));
                        assert!(
                            point.x.abs() < 0.95,
                            "count {count} slot {index}: {point:?}"
                        );
                        assert!(
                            point.y.abs() < 0.95,
                            "count {count} slot {index}: {point:?}"
                        );
                    }
                }
                body_edges.push((
                    project(Vec3::new(-0.5, 1.0, 0.0)).x,
                    project(Vec3::new(0.5, 1.0, 0.0)).x,
                ));
            }
            assert!(body_edges.windows(2).all(|pair| pair[0].1 < pair[1].0));
        }
    }

    #[test]
    fn identity_anchors_follow_hero_feet_including_asymmetric_parties() {
        for count in 1..=5 {
            let viewer = slot_anchor(0, count);
            assert!((viewer.x - 0.5).abs() < 0.0001);
            for index in 0..count {
                let anchor = slot_anchor(index, count);
                assert!(anchor.x > 0.0 && anchor.x < 1.0);
                assert!(anchor.y > 0.0 && anchor.y < 1.0);
                if index > 0 {
                    assert!(anchor.y < viewer.y);
                    assert_eq!(anchor.x < 0.5, index % 2 == 1);
                }
            }
            for (left, right) in [(1, 2), (3, 4)] {
                if right < count {
                    let left = slot_anchor(left, count);
                    let right = slot_anchor(right, count);
                    assert!((left.x + right.x - 1.0).abs() < 0.0001);
                    assert!((left.y - right.y).abs() < 0.0001);
                }
            }
        }
    }

    #[test]
    fn ready_store_models_invalidate_an_unchanged_member_list() {
        let pending = StageSnapshot {
            members: vec![member()],
            store_states: vec![Some(ModelState::Pending)],
        };
        let ready = StageSnapshot {
            members: pending.members.clone(),
            store_states: vec![Some(ModelState::Ready)],
        };
        assert_ne!(pending, ready);
        assert_eq!(pending.members, ready.members);
        let mut changed_character = ready.clone();
        changed_character.members[0].character = CharacterChoice::Cube;
        assert_ne!(ready, changed_character);
    }

    #[test]
    fn draft_equipment_changes_reuse_the_model_and_lobby_uses_local_choice() {
        use shared::handheld::HandheldSelection;
        let mut app = App::new();
        app.init_resource::<Assets<Image>>()
            .init_resource::<PartyStage>()
            .init_resource::<crate::team::TeamSelection>()
            .insert_resource(State::new(AppScreen::Draft))
            .add_systems(Update, sync_stage_equipment);
        let pivot = app.world_mut().spawn_empty().id();
        let model = app.world_mut().spawn(ChildOf(pivot)).id();
        let mut accepted = member();
        accepted.handheld = HandheldSelection::Item("forge-sword".into());
        let before = StageSnapshot {
            members: vec![accepted.clone()],
            store_states: vec![None],
        };
        {
            let mut stage = app.world_mut().resource_mut::<PartyStage>();
            stage.members = vec![accepted];
            stage.slots.push(Slot {
                root: pivot,
                pivot,
                model,
                gltf: None,
                bound: false,
                idle: None,
            });
        }
        app.update();
        assert_eq!(
            app.world()
                .get::<crate::net::PlayerHandheld>(model)
                .unwrap()
                .0,
            HandheldSelection::Item("forge-sword".into())
        );
        app.world_mut().resource_mut::<PartyStage>().members[0].handheld =
            HandheldSelection::Unequipped;
        app.update();
        let stage = app.world().resource::<PartyStage>();
        assert_eq!(stage.slots[0].model, model);
        let after = StageSnapshot {
            members: stage.members.clone(),
            store_states: vec![None],
        };
        assert!(
            before.same_models(&after),
            "cosmetic choice must not recreate the animated rig"
        );
        assert_eq!(
            app.world()
                .get::<crate::net::PlayerHandheld>(model)
                .unwrap()
                .0,
            HandheldSelection::Unequipped
        );
        app.insert_resource(State::new(AppScreen::Lobby));
        app.world_mut()
            .resource_mut::<crate::team::TeamSelection>()
            .handheld = HandheldSelection::Item("forge-hammer".into());
        app.update();
        assert_eq!(
            app.world()
                .get::<crate::net::PlayerHandheld>(model)
                .unwrap()
                .0,
            HandheldSelection::Item("forge-hammer".into())
        );
    }

    #[test]
    fn confirmation_reveals_only_the_hero_without_replacing_its_loaded_instance() {
        let mut app = App::new();
        app.init_resource::<Assets<Image>>()
            .init_resource::<PartyStage>()
            .add_systems(Update, sync_model_visibility);
        let root = app.world_mut().spawn(Visibility::Visible).id();
        let pivot = app
            .world_mut()
            .spawn((Visibility::Visible, ChildOf(root)))
            .id();
        let model = app
            .world_mut()
            .spawn((Visibility::Visible, ChildOf(pivot)))
            .id();
        let plinth = app
            .world_mut()
            .spawn((Visibility::Visible, ChildOf(root)))
            .id();
        let hidden = StageMember {
            revealed: false,
            ..member()
        };
        let pending = StageSnapshot {
            members: vec![hidden.clone()],
            store_states: vec![None],
        };
        let mut confirmed = pending.clone();
        confirmed.members[0].revealed = true;
        assert!(pending.same_models(&confirmed));
        confirmed.members[0].avatar = Some("crowley".into());
        assert!(
            !pending.same_models(&confirmed),
            "a new avatar still replaces its model"
        );
        {
            let mut stage = app.world_mut().resource_mut::<PartyStage>();
            stage.members.push(hidden);
            stage.slots.push(Slot {
                root,
                pivot,
                model,
                gltf: None,
                bound: false,
                idle: None,
            });
        }
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(model).unwrap(),
            Visibility::Hidden
        );
        assert_eq!(
            *app.world().get::<Visibility>(plinth).unwrap(),
            Visibility::Visible
        );
        assert_eq!(
            *app.world().get::<Visibility>(pivot).unwrap(),
            Visibility::Visible
        );
        app.world_mut().resource_mut::<PartyStage>().members[0].revealed = true;
        app.update();
        assert_eq!(app.world().resource::<PartyStage>().slots[0].model, model);
        assert_eq!(
            *app.world().get::<Visibility>(model).unwrap(),
            Visibility::Visible
        );
        assert_eq!(
            *app.world().get::<Visibility>(plinth).unwrap(),
            Visibility::Visible
        );
    }

    #[test]
    fn idle_binding_recovers_from_scene_player_reset_and_replacement() {
        let mut app = App::new();
        app.init_resource::<Assets<Image>>()
            .init_resource::<Assets<Gltf>>()
            .init_resource::<Assets<AnimationClip>>()
            .init_resource::<Assets<AnimationGraph>>()
            .init_resource::<PartyStage>()
            .init_resource::<crate::humanoid::HumanoidRuntimeLibrary>()
            .add_systems(PostUpdate, play_idle.before(bevy::app::AnimationSystems));
        let clip = app
            .world_mut()
            .resource_mut::<Assets<AnimationClip>>()
            .add(AnimationClip::default());
        let gltf = app.world_mut().resource_mut::<Assets<Gltf>>().add(Gltf {
            scenes: vec![],
            named_scenes: default(),
            meshes: vec![],
            named_meshes: default(),
            materials: vec![],
            named_materials: default(),
            nodes: vec![],
            named_nodes: default(),
            skins: vec![],
            named_skins: default(),
            default_scene: None,
            animations: vec![clip.clone()],
            named_animations: [("idle".into(), clip)].into(),
            source: None,
        });
        let mut roots = Vec::new();
        for _ in 0..2 {
            let model = app.world_mut().spawn_empty().id();
            let player = app
                .world_mut()
                .spawn((AnimationPlayer::default(), ChildOf(model)))
                .id();
            app.world_mut()
                .resource_mut::<PartyStage>()
                .slots
                .push(Slot {
                    root: model,
                    pivot: model,
                    model,
                    gltf: Some(gltf.clone()),
                    bound: false,
                    idle: None,
                });
            roots.push((model, player));
        }
        app.update();
        let bindings: Vec<_> = app
            .world()
            .resource::<PartyStage>()
            .slots
            .iter()
            .map(|slot| {
                let idle = slot.idle.as_ref().unwrap();
                (idle.player, idle.node, idle.graph.clone())
            })
            .collect();
        assert_ne!(bindings[0].2, bindings[1].2, "each instance owns its graph");
        for (player, node, _) in &bindings {
            app.world_mut()
                .get_mut::<AnimationPlayer>(*player)
                .unwrap()
                .animation_mut(*node)
                .unwrap()
                .set_seek_time(1.25);
        }
        app.update();
        assert_eq!(
            app.world()
                .get::<AnimationPlayer>(bindings[1].0)
                .unwrap()
                .animation(bindings[1].1)
                .unwrap()
                .seek_time(),
            1.25,
            "valid bindings must not restart the idle every frame"
        );

        // Simulate WorldInstanceSpawner's writes after Update in its actual schedule.
        #[derive(Resource)]
        struct ResetPlayer(Entity);
        app.add_systems(bevy::app::SpawnScene, |world: &mut World| {
            if let Some(reset) = world.remove_resource::<ResetPlayer>() {
                world
                    .entity_mut(reset.0)
                    .insert(AnimationPlayer::default())
                    .remove::<AnimationGraphHandle>();
            }
        });
        app.world_mut().insert_resource(ResetPlayer(bindings[0].0));
        app.update();
        assert!(
            app.world()
                .get::<AnimationPlayer>(bindings[0].0)
                .unwrap()
                .is_playing_animation(bindings[0].1)
        );
        assert_eq!(
            app.world()
                .get::<AnimationGraphHandle>(bindings[0].0)
                .unwrap()
                .0,
            bindings[0].2
        );
        assert_eq!(
            app.world()
                .get::<AnimationPlayer>(bindings[1].0)
                .unwrap()
                .animation(bindings[1].1)
                .unwrap()
                .seek_time(),
            1.25
        );

        // Asset refresh replaces scene entities altogether, retaining the
        // external WorldAssetRoot. That replacement needs a newly located player.
        app.world_mut().despawn(roots[0].1);
        let replacement = app
            .world_mut()
            .spawn((AnimationPlayer::default(), ChildOf(roots[0].0)))
            .id();
        app.update();
        let repaired = app.world().resource::<PartyStage>().slots[0]
            .idle
            .as_ref()
            .unwrap();
        assert_eq!(repaired.player, replacement);
        assert!(
            app.world()
                .get::<AnimationPlayer>(replacement)
                .unwrap()
                .is_playing_animation(repaired.node)
        );
        assert_ne!(repaired.graph, bindings[1].2);
    }

    #[test]
    fn touched_preview_selects_each_revealed_body_and_ignores_empty_space() {
        let rect = Rect::from_corners(Vec2::new(32.0, 60.0), Vec2::new(398.0, 265.875));
        let mut members = vec![member(); 5];
        for index in 0..5 {
            let anchor = slot_anchor(index, 5);
            let body = rect.min + (anchor - Vec2::Y * 0.15) * rect.size();
            assert_eq!(touched_slot(body, rect, &members), Some(index));
        }
        let hidden = rect.min + (slot_anchor(1, 5) - Vec2::Y * 0.15) * rect.size();
        members[1].revealed = false;
        assert_eq!(touched_slot(hidden, rect, &members), None);
        assert_eq!(touched_slot(rect.min, rect, &members), None);
        assert_eq!(touched_slot(rect.max + Vec2::ONE, rect, &members), None);
    }

    #[test]
    fn only_drag_started_on_surface_rotates_and_other_fingers_are_ignored() {
        let mut drag = StageDrag::default();
        drag.begin(Some(1), Vec2::ZERO, None);
        assert_eq!(drag.moved(Some(1), Vec2::new(50.0, 0.0)), 0.0);
        drag.begin(Some(1), Vec2::ZERO, Some(2));
        drag.begin(Some(2), Vec2::ZERO, Some(1));
        assert_eq!(drag.slot, Some(2));
        assert_eq!(drag.moved(Some(2), Vec2::new(50.0, 0.0)), 0.0);
        assert!((drag.moved(Some(1), Vec2::new(50.0, 0.0)) - 0.6).abs() < 1e-6);
        drag.end(Some(2));
        assert!(drag.held.is_some());
        drag.end(Some(1));
        assert_eq!(drag.moved(Some(1), Vec2::new(80.0, 0.0)), 0.0);
        drag.begin(None, Vec2::ZERO, Some(0));
        assert_eq!(drag.moved(None, Vec2::new(0.0, 70.0)), 0.0);
        drag.end(None);
        assert!(drag.held.is_none());
    }

    #[test]
    fn raw_touch_rotates_only_the_body_where_the_drag_began() {
        let mut app = App::new();
        app.init_resource::<Assets<Image>>()
            .init_resource::<PartyStage>()
            .insert_resource(State::new(AppScreen::Draft))
            .add_message::<TouchInput>()
            .add_systems(Update, interact);
        app.world_mut().resource_mut::<PartyStage>().members = vec![member(); 5];
        let mut window = Window {
            focused: true,
            ..default()
        };
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set(852.0, 393.0);
        let window = app.world_mut().spawn((window, PrimaryWindow)).id();
        let size = Vec2::new(366.0, 205.875);
        let rect = Rect::from_corners(Vec2::new(32.0, 60.0), Vec2::new(32.0, 60.0) + size);
        app.world_mut().spawn((
            StageSurface,
            ComputedNode {
                size,
                inverse_scale_factor: 1.0,
                ..default()
            },
            UiGlobalTransform::from_translation(rect.center()),
            InheritedVisibility::VISIBLE,
        ));
        let start = rect.min + (slot_anchor(1, 5) - Vec2::Y * 0.15) * size;
        for (id, phase, position) in [
            (1, TouchPhase::Started, start),
            (2, TouchPhase::Started, rect.center()),
            (2, TouchPhase::Moved, rect.center() + Vec2::X * 50.0),
            (1, TouchPhase::Moved, start + Vec2::X * 50.0),
            (1, TouchPhase::Ended, start + Vec2::X * 50.0),
        ] {
            app.world_mut().write_message(TouchInput {
                id,
                phase,
                position,
                window,
                force: None,
            });
            app.update();
        }
        for (index, yaw) in app.world().resource::<PartyStage>().yaws.iter().enumerate() {
            let expected = std::f32::consts::PI + if index == 1 { 0.6 } else { 0.0 };
            assert!(
                (yaw - expected).abs() < 1e-5,
                "slot {index} rotated by {yaw}"
            );
        }
    }

    #[test]
    fn turning_heroes_preserves_formation_positions() {
        let mut app = App::new();
        app.init_resource::<Assets<Image>>()
            .init_resource::<PartyStage>()
            .add_systems(Update, turn_and_ground_heroes);
        let mut positions = Vec::new();
        for index in 0..5 {
            let position = slot_position(index, 5);
            let root = app
                .world_mut()
                .spawn(Transform::from_translation(position))
                .id();
            let pivot = app
                .world_mut()
                .spawn(Transform::from_xyz(0.0, PLINTH_TOP, 0.0))
                .id();
            app.world_mut()
                .resource_mut::<PartyStage>()
                .slots
                .push(Slot {
                    root,
                    pivot,
                    model: pivot,
                    gltf: None,
                    bound: true,
                    idle: None,
                });
            positions.push((root, pivot, position));
        }
        app.world_mut().resource_mut::<PartyStage>().yaws[2] = 0.75;
        app.update();
        for (index, (root, pivot, original)) in positions.into_iter().enumerate() {
            assert_eq!(
                app.world().get::<Transform>(root).unwrap().translation,
                original
            );
            let transformed = app.world().get::<Transform>(pivot).unwrap();
            assert_eq!(transformed.translation, Vec3::Y * PLINTH_TOP);
            assert!(transformed.rotation.abs_diff_eq(
                Quat::from_rotation_y(if index == 2 {
                    0.75
                } else {
                    std::f32::consts::PI
                }),
                1e-6
            ));
        }
    }

    #[test]
    fn stage_is_isolated_and_only_available_on_its_three_screens() {
        assert_ne!(STAGE_LAYER, super::super::preview::PREVIEW_LAYER);
        assert_ne!(STAGE_LAYER, 29);
        for screen in [AppScreen::Lobby, AppScreen::Draft, AppScreen::Loading] {
            assert!(stage_screen(screen));
        }
        for screen in [
            AppScreen::Home,
            AppScreen::Collection,
            AppScreen::InMatch,
            AppScreen::HeroSelect,
            AppScreen::Searching,
        ] {
            assert!(!stage_screen(screen));
        }
    }
}
