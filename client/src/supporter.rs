//! Server-authorized cosmetics. Preview has a separate camera and never equips a
//! world actor; all live aura components come only from the server snapshot.
use crate::{
    career::{CareerClient, CareerModal},
    combat::CombatStats,
    input_context::InputContextSet,
    net::{NetworkCommand, RemotePlayer},
    player::Player,
    sprite::PlayerVisualMode,
    ui::{
        Activated, ModalId, ModalRoot, Pressable, ScrollArea, TestId, UiAction, UiActionAppExt,
        UiSet, theme as ui,
    },
};
use bevy::{
    asset::RenderAssetUsages,
    camera::{RenderTarget, visibility::RenderLayers},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use shared::{
    career::CareerRequest,
    supporter::{AuraStyle, SupporterStatus},
};
use std::collections::HashMap;

const PREVIEW_LAYER: usize = 29;
const AURA_RADIUS: f32 = 0.76;
const ORBIT_COUNT: usize = 3;
const TRAIL_SAMPLES: usize = 8;
const GLINT_COUNT: usize = 4;
const TRAIL_STEP: f64 = 0.075;

#[derive(Clone, Copy)]
enum AuraParticle {
    Core,
    Halo,
    Trail(usize),
    Glint,
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NetworkSupporterAura(pub Option<AuraStyle>);
#[derive(Resource)]
pub(crate) struct SupporterUiState {
    pub open: bool,
    pub selected: AuraStyle,
}
impl Default for SupporterUiState {
    fn default() -> Self {
        Self {
            // `OMOBA_QA_SUPPORTER=1` opens the panel for `qa::supporter` captures.
            open: cfg!(feature = "qa")
                && std::env::var("OMOBA_QA_SUPPORTER").is_ok_and(|value| value == "1"),
            selected: AuraStyle::Solar,
        }
    }
}
#[derive(Message, Clone, Copy, Debug)]
pub(crate) enum SupporterPlatformAction {
    Purchase,
    Restore,
}
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub(crate) struct SupporterPlatformState {
    pub available: bool,
    pub busy: bool,
    pub price_label: Option<String>,
    pub message: Option<String>,
}
#[derive(Resource, Default)]
struct AuraRegistry(HashMap<Entity, (Entity, AuraStyle, PlayerVisualMode)>);
#[derive(Component)]
struct AuraOwner(Entity);
#[derive(Component)]
struct AuraAnchor;
#[derive(Component)]
struct AuraRing;
#[derive(Component)]
struct Orbit {
    index: f32,
    flat: bool,
    particle: AuraParticle,
    preview: bool,
}
#[derive(Component)]
struct PreviewCamera;
#[derive(Component)]
struct PreviewObject;
#[derive(Component)]
struct PreviewAura;
#[derive(Component)]
struct SupporterRoot;
#[derive(Component)]
struct SupporterScroll;
#[derive(Component)]
struct PreviewImage;
/// Supporter panel presses. The buttons keep their flat look (no hover
/// colour, as before): they carry `UiAction<Action>` but no `ButtonStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Close,
    Select(AuraStyle),
    Equip,
    Disable,
    Refresh,
    Purchase,
    Restore,
}
#[derive(Resource)]
struct AuraAssets {
    ring: Handle<Mesh>,
    orb: Handle<Mesh>,
    flat_orb: Handle<Mesh>,
    glow_quad: Handle<Mesh>,
    material: [Handle<StandardMaterial>; 3],
    flat: [Handle<ColorMaterial>; 3],
    cores: [Handle<StandardMaterial>; 3],
    halos: [Handle<StandardMaterial>; 3],
    trails: [Handle<StandardMaterial>; 3],
    flat_cores: [Handle<ColorMaterial>; 3],
    flat_halos: [Handle<ColorMaterial>; 3],
    flat_trails: [Handle<ColorMaterial>; 3],
    preview: Handle<Image>,
}
fn style_index(style: AuraStyle) -> usize {
    match style {
        AuraStyle::Solar => 0,
        AuraStyle::Lunar => 1,
        AuraStyle::Verdant => 2,
    }
}
fn color(style: AuraStyle) -> Color {
    match style {
        AuraStyle::Solar => Color::srgb(1., 0.67, 0.24),
        AuraStyle::Lunar => Color::srgb(0.43, 0.69, 1.),
        AuraStyle::Verdant => Color::srgb(0.25, 0.96, 0.65),
    }
}

// Shared materials and analytic paths keep the per-hero budget fixed. No emitter
// churn, dynamic lights, or bloom dependency on mobile renderers.
fn particle_material(style: AuraStyle, alpha: f32, white: f32) -> StandardMaterial {
    let tint = color(style).mix(&Color::WHITE, white);
    StandardMaterial {
        base_color: tint.with_alpha(alpha),
        emissive: LinearRgba::from(tint) * 2.2,
        unlit: true,
        alpha_mode: AlphaMode::Add,
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

fn glow_texture(star: bool) -> Image {
    const SIZE: u32 = 64;
    let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let u = (x as f32 + 0.5) / SIZE as f32 * 2. - 1.;
            let v = (y as f32 + 0.5) / SIZE as f32 * 2. - 1.;
            let radius = (u * u + v * v).sqrt();
            let edge = (1. - radius).max(0.);
            let mut alpha = edge.powi(3);
            if star {
                let rays = (-u.abs() * 45.).exp() * (1. - v.abs()).powi(2)
                    + (-v.abs() * 45.).exp() * (1. - u.abs()).powi(2);
                alpha = (alpha * 0.6 + rays * edge).min(1.);
            }
            data.extend_from_slice(&[255, 255, 255, (alpha * 255.) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

fn orbital_point(index: f32, seconds: f64) -> Vec3 {
    let phase = (seconds * (1.30 + index as f64 * 0.13)
        + index as f64 * std::f64::consts::TAU / ORBIT_COUNT as f64)
        .rem_euclid(std::f64::consts::TAU) as f32;
    let (s, c) = phase.sin_cos();
    let tilt =
        Quat::from_rotation_y(index * std::f32::consts::TAU / 3.) * Quat::from_rotation_z(0.34);
    Vec3::Y * 0.82 + tilt * Vec3::new(c * AURA_RADIUS, s * 0.68, s * 0.18)
}

fn project_particle(point: Vec3, flat: bool) -> Vec3 {
    if flat {
        Vec3::new(point.x, point.y * 0.68 + point.z * 0.35, 0.)
    } else {
        point
    }
}

fn particle_pose(orbit: &Orbit, seconds: f64) -> Transform {
    let head = orbital_point(orbit.index, seconds);
    let (position, scale, rotation) = match orbit.particle {
        AuraParticle::Core | AuraParticle::Halo => {
            let pulse = 1. + 0.12 * (seconds as f32 * 3. + orbit.index).sin();
            let size = if matches!(orbit.particle, AuraParticle::Halo) {
                5.0
            } else {
                2.0
            };
            (head, Vec3::splat(size * pulse), Quat::IDENTITY)
        }
        AuraParticle::Trail(sample) => {
            let start = orbital_point(orbit.index, seconds - sample as f64 * TRAIL_STEP);
            let end = orbital_point(orbit.index, seconds - (sample + 1) as f64 * TRAIL_STEP);
            let delta = project_particle(start, orbit.flat) - project_particle(end, orbit.flat);
            let taper = 1. - sample as f32 / TRAIL_SAMPLES as f32;
            let width = 0.48 * taper * taper + 0.025;
            (
                (start + end) * 0.5,
                Vec3::new(delta.length() / 0.13 + width, width, width),
                Quat::from_rotation_arc(Vec3::X, delta.normalize_or(Vec3::X)),
            )
        }
        AuraParticle::Glint => {
            let age = (seconds * 0.45 + orbit.index as f64 / GLINT_COUNT as f64).fract() as f32;
            let phase = orbit.index * 2.399 + age * 1.2;
            let radius = AURA_RADIUS * (0.5 + 0.3 * age);
            let point = Vec3::new(
                phase.cos() * radius,
                0.15 + age * 1.55,
                phase.sin() * radius,
            );
            let sparkle = (std::f32::consts::PI * age).sin().powi(3) * 1.5;
            (point, Vec3::splat(sparkle), Quat::IDENTITY)
        }
    };
    let mut projected = project_particle(position, orbit.flat);
    if orbit.flat {
        // Orbit behind the sprite on the far half and in front on the near half.
        projected.z = if position.z >= 0. {
            crate::world2d::layer::VFX - crate::world2d::layer::MARKER
        } else {
            0.02
        };
    }
    Transform::from_translation(projected)
        .with_scale(scale)
        .with_rotation(rotation)
}

pub(crate) struct SupporterPlugin;
impl Plugin for SupporterPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SupporterUiState>()
            .init_resource::<SupporterPlatformState>()
            .init_resource::<AuraRegistry>()
            .add_message::<SupporterPlatformAction>()
            .add_ui_action::<Action>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (actions, keyboard_close)
                    .chain()
                    .in_set(InputContextSet::Modal)
                    .after(UiSet::Dispatch)
                    .before(crate::pause_menu::toggle_pause_menu),
            )
            .add_systems(
                Update,
                (render_panel, sync_preview)
                    .chain()
                    .after(InputContextSet::Modal),
            )
            .add_systems(
                Update,
                sync_auras.after(crate::net::ClientNetPipeline::ApplySnapshot),
            )
            .add_systems(
                PostUpdate,
                (follow_actors, animate_orbits)
                    .chain()
                    .after(crate::net::NetworkGroundingSet)
                    .after(bevy::transform::TransformSystems::Propagate)
                    .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate)
                    .before(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
            );
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut flat: ResMut<Assets<ColorMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let image = images.add(Image::new_target_texture(
        320,
        224,
        TextureFormat::Rgba8Unorm,
        Some(TextureFormat::Rgba8UnormSrgb),
    ));
    let glow = images.add(glow_texture(false));
    let star = images.add(glow_texture(true));
    let aura = AuraAssets {
        ring: meshes.add(Annulus::new(AURA_RADIUS - 0.025, AURA_RADIUS)),
        orb: meshes.add(Sphere::new(0.065)),
        flat_orb: meshes.add(Circle::new(0.065)),
        glow_quad: meshes.add(Rectangle::new(0.13, 0.13)),
        material: AuraStyle::ALL.map(|s| {
            materials.add(StandardMaterial {
                base_color: color(s).with_alpha(0.22),
                emissive: LinearRgba::from(color(s)) * 0.25,
                unlit: true,
                alpha_mode: AlphaMode::Add,
                double_sided: true,
                cull_mode: None,
                ..default()
            })
        }),
        flat: AuraStyle::ALL.map(|s| flat.add(color(s).with_alpha(0.24))),
        cores: AuraStyle::ALL.map(|s| {
            materials.add(StandardMaterial {
                base_color_texture: Some(star.clone()),
                ..particle_material(s, 1.0, 0.85)
            })
        }),
        halos: AuraStyle::ALL.map(|s| {
            materials.add(StandardMaterial {
                base_color_texture: Some(glow.clone()),
                ..particle_material(s, 0.85, 0.1)
            })
        }),
        trails: AuraStyle::ALL.map(|s| materials.add(particle_material(s, 0.65, 0.30))),
        flat_cores: AuraStyle::ALL.map(|s| {
            flat.add(ColorMaterial {
                color: color(s).mix(&Color::WHITE, 0.85),
                texture: Some(star.clone()),
                ..default()
            })
        }),
        flat_halos: AuraStyle::ALL.map(|s| {
            flat.add(ColorMaterial {
                color: color(s).with_alpha(0.7),
                texture: Some(glow.clone()),
                ..default()
            })
        }),
        flat_trails: AuraStyle::ALL.map(|s| flat.add(color(s).with_alpha(0.65))),
        preview: image.clone(),
    };
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -1,
            is_active: false,
            clear_color: Color::srgb(0.025, 0.06, 0.065).into(),
            ..default()
        },
        RenderTarget::Image(image.into()),
        Transform::from_xyz(1.7, 1.6, 2.5).looking_at(Vec3::Y * 0.65, Vec3::Y),
        RenderLayers::layer(PREVIEW_LAYER),
        PreviewCamera,
    ));
    // Neutral preview mannequin; no real player, account, combat or network ID.
    let body = materials.add(StandardMaterial {
        base_color: Color::srgb(0.58, 0.67, 0.64),
        unlit: true,
        ..default()
    });
    let limb = meshes.add(Capsule3d::new(0.065, 0.38));
    let leg = meshes.add(Capsule3d::new(0.08, 0.28));
    for (mesh, position, tilt) in [
        (meshes.add(Capsule3d::new(0.20, 0.40)), Vec3::Y * 0.84, 0.0),
        (meshes.add(Sphere::new(0.18)), Vec3::Y * 1.44, 0.0),
        (limb.clone(), Vec3::new(-0.29, 0.90, 0.0), -0.15),
        (limb, Vec3::new(0.29, 0.90, 0.0), 0.15),
        (leg.clone(), Vec3::new(-0.10, 0.23, 0.0), 0.0),
        (leg, Vec3::new(0.10, 0.23, 0.0), 0.0),
    ] {
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(body.clone()),
            Transform::from_translation(position).with_rotation(Quat::from_rotation_z(tilt)),
            Visibility::Inherited,
            RenderLayers::layer(PREVIEW_LAYER),
            PreviewObject,
        ));
    }
    commands.insert_resource(aura);
}

fn spawn_aura(
    commands: &mut Commands,
    assets: &AuraAssets,
    style: AuraStyle,
    flat: bool,
    layer: usize,
) -> Entity {
    let index = style_index(style);
    let root = commands
        .spawn((
            AuraAnchor,
            Transform::default(),
            Visibility::Hidden,
            Name::new("Supporter cosmetic aura"),
        ))
        .id();
    let ring_transform = if flat {
        Transform::default()
    } else {
        Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
    };
    let ring = commands
        .spawn((
            AuraRing,
            ring_transform,
            Visibility::Inherited,
            RenderLayers::layer(layer),
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
        ))
        .id();
    if flat {
        commands.entity(ring).insert((
            Mesh2d(assets.ring.clone()),
            MeshMaterial2d(assets.flat[index].clone()),
        ));
    } else {
        commands.entity(ring).insert((
            Mesh3d(assets.ring.clone()),
            MeshMaterial3d(assets.material[index].clone()),
        ));
    }
    commands.entity(root).add_child(ring);
    let particles = (0..ORBIT_COUNT)
        .flat_map(|i| {
            [AuraParticle::Core, AuraParticle::Halo]
                .into_iter()
                .chain((0..TRAIL_SAMPLES).map(AuraParticle::Trail))
                .map(move |kind| (i, kind))
        })
        .chain((0..GLINT_COUNT).map(|i| (i, AuraParticle::Glint)));
    for (index_orbit, particle) in particles {
        let orbit = Orbit {
            index: index_orbit as f32,
            flat,
            particle,
            preview: layer == PREVIEW_LAYER,
        };
        let (material, flat_material) = match particle {
            AuraParticle::Core | AuraParticle::Glint => {
                (&assets.cores[index], &assets.flat_cores[index])
            }
            AuraParticle::Halo => (&assets.halos[index], &assets.flat_halos[index]),
            AuraParticle::Trail(_) => (&assets.trails[index], &assets.flat_trails[index]),
        };
        let orb = commands
            .spawn((
                particle_pose(&orbit, 0.),
                orbit,
                Visibility::Inherited,
                RenderLayers::layer(layer),
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
            ))
            .id();
        let billboard = !matches!(particle, AuraParticle::Trail(_));
        if flat {
            commands.entity(orb).insert((
                Mesh2d(if billboard {
                    assets.glow_quad.clone()
                } else {
                    assets.flat_orb.clone()
                }),
                MeshMaterial2d(flat_material.clone()),
            ));
        } else {
            commands.entity(orb).insert((
                Mesh3d(if billboard {
                    assets.glow_quad.clone()
                } else {
                    assets.orb.clone()
                }),
                MeshMaterial3d(material.clone()),
            ));
        }
        commands.entity(root).add_child(orb);
    }
    root
}

fn sync_auras(
    mut commands: Commands,
    assets: Res<AuraAssets>,
    mode: Res<PlayerVisualMode>,
    actors: Query<(Entity, &NetworkSupporterAura), Or<(With<Player>, With<RemotePlayer>)>>,
    mut registry: ResMut<AuraRegistry>,
) {
    registry.0.retain(|owner, (root, style, old_mode)| {
        if actors
            .get(*owner)
            .is_ok_and(|(_, aura)| aura.0 == Some(*style))
            && *old_mode == *mode
        {
            true
        } else {
            commands.entity(*root).despawn();
            false
        }
    });
    for (actor, aura) in &actors {
        if registry.0.contains_key(&actor) {
            continue;
        }
        if let Some(style) = aura.0 {
            let root = spawn_aura(
                &mut commands,
                &assets,
                style,
                *mode == PlayerVisualMode::Sprite2d,
                0,
            );
            commands.entity(root).insert(AuraOwner(actor));
            registry.0.insert(actor, (root, style, *mode));
        }
    }
}

fn actor_visible(
    stats: &CombatStats,
    visibility: &Visibility,
    inherited: &InheritedVisibility,
) -> bool {
    stats.is_alive() && *visibility != Visibility::Hidden && inherited.get()
}
fn follow_actors(
    actors: Query<
        (
            &GlobalTransform,
            &CombatStats,
            &Visibility,
            &InheritedVisibility,
        ),
        Without<AuraOwner>,
    >,
    mut roots: Query<
        (
            &AuraOwner,
            &mut Transform,
            &mut GlobalTransform,
            &mut Visibility,
            &mut InheritedVisibility,
        ),
        Without<CombatStats>,
    >,
    mode: Res<PlayerVisualMode>,
    map: Res<crate::maps::MapLayout>,
) {
    for (owner, mut transform, mut global, mut visibility, mut inherited_aura) in &mut roots {
        let Ok((pose, stats, own_visibility, inherited)) = actors.get(owner.0) else {
            *visibility = Visibility::Hidden;
            *inherited_aura = InheritedVisibility::HIDDEN;
            continue;
        };
        let position = pose.translation();
        *visibility = if actor_visible(stats, own_visibility, inherited) && position.is_finite() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        *inherited_aura = if *visibility == Visibility::Hidden {
            InheritedVisibility::HIDDEN
        } else {
            InheritedVisibility::VISIBLE
        };
        *transform = if *mode == PlayerVisualMode::Sprite2d {
            Transform::from_translation(
                crate::world2d::simulation_xz_to_render_xy(position)
                    .extend(crate::world2d::layer::MARKER - 0.01),
            )
        } else {
            Transform::from_xyz(
                position.x,
                map.terrain_height_3d(position.x, position.z) + 0.10,
                position.z,
            )
        };
        // Follow the interpolated ground pose in the same frame. Child poses are
        // updated below after propagation; no animated skeleton/scale is copied.
        *global = GlobalTransform::from(*transform);
    }
}

fn animate_orbits(
    time: Res<Time>,
    roots: Query<(&GlobalTransform, &InheritedVisibility), With<AuraAnchor>>,
    cameras: Query<
        (&Camera, &GlobalTransform, Option<&PreviewCamera>),
        (
            With<Camera3d>,
            Without<Orbit>,
            Without<AuraRing>,
            Without<AuraAnchor>,
        ),
    >,
    mut orbits: Query<
        (
            &Orbit,
            &ChildOf,
            &mut Transform,
            &mut GlobalTransform,
            &mut InheritedVisibility,
        ),
        Without<AuraAnchor>,
    >,
    mut rings: Query<
        (
            &ChildOf,
            &Transform,
            &mut GlobalTransform,
            &mut InheritedVisibility,
        ),
        (With<AuraRing>, Without<Orbit>, Without<AuraAnchor>),
    >,
) {
    for (orbit, parent, mut transform, mut global, mut inherited) in &mut orbits {
        *transform = particle_pose(orbit, time.elapsed_secs_f64());
        if !orbit.flat && !matches!(orbit.particle, AuraParticle::Trail(_)) {
            if let Some((_, pose, _)) = cameras
                .iter()
                .find(|(camera, _, preview)| camera.is_active && preview.is_some() == orbit.preview)
            {
                transform.rotation = pose.to_scale_rotation_translation().1;
            }
        }
        if let Ok((parent, visible)) = roots.get(parent.parent()) {
            *global = parent.mul_transform(*transform);
            *inherited = *visible;
        }
    }
    for (parent, transform, mut global, mut inherited) in &mut rings {
        if let Ok((parent, visible)) = roots.get(parent.parent()) {
            *global = parent.mul_transform(*transform);
            *inherited = *visible;
        }
    }
}

fn sync_preview(
    mut commands: Commands,
    state: Res<SupporterUiState>,
    assets: Res<AuraAssets>,
    mut cameras: Query<&mut Camera, With<PreviewCamera>>,
    roots: Query<Entity, With<PreviewAura>>,
    mut previous: Local<Option<(bool, AuraStyle)>>,
) {
    for mut camera in &mut cameras {
        camera.is_active = state.open;
    }
    let next = (state.open, state.selected);
    if *previous == Some(next) {
        return;
    }
    *previous = Some(next);
    for entity in &roots {
        commands.entity(entity).despawn();
    }
    if state.open {
        let entity = spawn_aura(&mut commands, &assets, state.selected, false, PREVIEW_LAYER);
        commands.entity(entity).insert((
            PreviewAura,
            Visibility::Visible,
            Transform::from_xyz(0., 0.1, 0.),
        ));
    }
}
fn keyboard_close(mut keys: ResMut<ButtonInput<KeyCode>>, mut state: ResMut<SupporterUiState>) {
    if state.open && keys.just_pressed(KeyCode::Escape) {
        state.open = false;
        keys.clear_just_pressed(KeyCode::Escape);
    }
}
fn actions(
    mut activated: MessageReader<Activated<Action>>,
    mut state: ResMut<SupporterUiState>,
    mut career: ResMut<CareerClient>,
    platform: Res<SupporterPlatformState>,
    mut platform_actions: MessageWriter<SupporterPlatformAction>,
    mut requests: MessageWriter<NetworkCommand>,
    mut sequence: Local<u64>,
) {
    for Activated { action, .. } in activated.read() {
        *sequence = sequence.saturating_add(1);
        let id = 0x5000_0000_0000_0000u64.saturating_add(*sequence);
        match action {
            Action::Close => state.open = false,
            Action::Select(style) => state.selected = *style,
            Action::Refresh => {
                requests.write(NetworkCommand::Career(CareerRequest::SupporterStatus {
                    request_id: id,
                }));
            }
            Action::Equip => {
                if career
                    .view
                    .supporter
                    .as_ref()
                    .is_some_and(|status| status.active)
                {
                    requests.write(NetworkCommand::Career(CareerRequest::EquipSupporterAura {
                        request_id: id,
                        aura: Some(state.selected),
                    }));
                }
            }
            Action::Disable => {
                requests.write(NetworkCommand::Career(CareerRequest::EquipSupporterAura {
                    request_id: id,
                    aura: None,
                }));
            }
            Action::Purchase
                if platform.available
                    && platform.price_label.is_some()
                    && !platform.busy
                    && purchase_allowed(&career) =>
            {
                platform_actions.write(SupporterPlatformAction::Purchase);
            }
            Action::Restore if platform.available && !platform.busy => {
                platform_actions.write(SupporterPlatformAction::Restore);
            }
            _ => {}
        }
        // Only one modal owns input. The account is still the source of truth.
        career.modal = CareerModal::Closed;
    }
}

fn purchase_allowed(career: &CareerClient) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_secs().min(i64::MAX as u64) as i64);
    purchase_allowed_at(career, now)
}
fn purchase_allowed_at(career: &CareerClient, now: i64) -> bool {
    career.view.storage_enabled
        && career.view.auth_nonce.is_some()
        && career.view.profile.is_some()
        && career.view.error.is_none()
        && career.view.supporter.as_ref().is_some_and(|s| {
            !s.active
                && !s
                    .grants
                    .iter()
                    .any(|grant| !grant.revoked && grant.valid_until > now)
        })
}

fn status_label(status: Option<&SupporterStatus>) -> String {
    let Some(status) = status else {
        return "Connect your saved account to view membership.".into();
    };
    if !status.active {
        return "No active membership. Preview is free and only visible here.".into();
    }
    let source = status
        .grants
        .iter()
        .filter(|g| !g.revoked && Some(g.valid_until) == status.active_until)
        .map(|g| g.provider.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Supporter active · {}\nAccess until {} UTC",
        source,
        status
            .active_until
            .map(utc_timestamp)
            .unwrap_or_else(|| "—".into())
    )
}
/// Gregorian civil date from an epoch day; avoids a platform locale dependency.
fn utc_timestamp(seconds: i64) -> String {
    let days = seconds.div_euclid(86400);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        seconds.rem_euclid(86400) / 3600,
        seconds.rem_euclid(3600) / 60
    )
}

fn render_panel(
    mut commands: Commands,
    state: Res<SupporterUiState>,
    career: Res<CareerClient>,
    platform: Res<SupporterPlatformState>,
    assets: Res<AuraAssets>,
    roots: Query<Entity, With<SupporterRoot>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut prior: Local<String>,
) {
    let compact = windows.single().is_ok_and(|w| w.height() < 540.);
    let summary = status_label(career.view.supporter.as_ref());
    let key = format!(
        "{}|{:?}|{}|{:?}|{}|{:?}|{}",
        state.open, state.selected, summary, *platform, career.nickname, career.view.error, compact
    );
    if *prior == key {
        return;
    }
    *prior = key;
    for root in &roots {
        commands.entity(root).despawn();
    }
    if !state.open {
        return;
    }
    commands.spawn((SupporterRoot,ModalRoot(ModalId::Supporter),Node{position_type:PositionType::Absolute,width:Val::Percent(100.),height:Val::Percent(100.),align_items:AlignItems::Center,justify_content:JustifyContent::Center,padding:UiRect::all(Val::Px(10.)),..default()},BackgroundColor(Color::srgba(0.,0.,0.,0.72)),GlobalZIndex(1300))).with_children(|root| {
        root.spawn((Node{position_type:PositionType::Absolute,top:Val::Px(8.),right:Val::Px(8.),..default()},GlobalZIndex(1301))).with_children(|corner|{button(corner,"Close".into(),Action::Close,true);});
        root.spawn((SupporterScroll,panel_scroll(),Node{width:Val::Px(660.),max_width:Val::Percent(100.),max_height:Val::Percent(96.),flex_direction:FlexDirection::Column,row_gap:Val::Px(if compact{5.}else{9.}),padding:UiRect::all(Val::Px(if compact{8.}else{12.})),overflow:Overflow::scroll_y(),..ui::panel_node()},BackgroundColor(ui::PANEL.with_alpha(1.0)),BorderColor::all(ui::EDGE))).with_children(|panel| {
            panel.spawn((Text::new(format!("Open Moba Supporter · {}",career.nickname)),ui::text(if compact{16.}else{20.}),TextColor(ui::GOLD)));
            panel.spawn((Text::new(summary),ui::text(if compact{12.}else{14.}),TextColor(ui::IVORY)));
            panel.spawn((Node{flex_direction:FlexDirection::Row,column_gap:Val::Px(12.),flex_wrap:FlexWrap::Wrap,..default()},)).with_children(|row| {
                row.spawn((ImageNode::new(assets.preview.clone()),PreviewImage,Node{width:Val::Px(if compact{170.}else{240.}),height:Val::Px(if compact{119.}else{168.}),..default()}));
                row.spawn((Node{width:Val::Px(330.),flex_direction:FlexDirection::Row,flex_wrap:FlexWrap::Wrap,align_content:AlignContent::Center,column_gap:Val::Px(6.),row_gap:Val::Px(7.),..default()},)).with_children(|choices| {
                    for style in AuraStyle::ALL {button(choices,if state.selected==style{format!("{} · Preview",style.label())}else{style.label().into()},Action::Select(style),true);}
                    let active=career.view.supporter.as_ref().is_some_and(|s|s.active);
                    button(choices,"Equip selected aura".into(),Action::Equip,active);
                    button(choices,"Hide my aura".into(),Action::Disable,active);
                });
            });
            panel.spawn((Text::new("LOCAL PREVIEW · Cosmetic only. No combat or progression benefits.\nBefore purchasing, link another device or save a recovery code in Account devices."),ui::text(if compact{10.}else{12.}),TextColor(ui::MUTED)));
            if let Some(error)=platform.message.as_ref().or(career.view.error.as_ref()) {panel.spawn((Text::new(error),ui::text(13.),TextColor(ui::GOLD)));}
            panel.spawn((Node{flex_direction:FlexDirection::Row,flex_wrap:FlexWrap::Wrap,column_gap:Val::Px(8.),row_gap:Val::Px(6.),..default()},)).with_children(|row| {
                if platform.available {
                    button(row,platform.price_label.clone().map(|p|format!("Support · {p}")).unwrap_or_else(||"Support with Apple".into()),Action::Purchase,platform.price_label.is_some()&&!platform.busy&&purchase_allowed(&career));
                    button(row,"Restore purchases".into(),Action::Restore,!platform.busy);
                } else {row.spawn((Text::new("Purchases are not configured on this client."),ui::text(if compact{10.}else{12.}),TextColor(ui::MUTED)));}
                button(row,"Refresh".into(),Action::Refresh,true);
            });
        });
    });
}
/// The supporter panel body: 24 px per wheel notch on every build, touch drag
/// from the first pixel.
fn panel_scroll() -> ScrollArea {
    ScrollArea::wheel(24.0).touch_drag(0.0)
}

fn button(parent: &mut ChildSpawnerCommands, label: String, action: Action, enabled: bool) {
    parent
        .spawn((
            Button,
            Node {
                min_height: Val::Px(38.),
                padding: UiRect::axes(Val::Px(12.), Val::Px(7.)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border_radius: BorderRadius::all(Val::Px(6.)),
                ..default()
            },
            BackgroundColor(if enabled { ui::TILE } else { ui::PANEL }),
            UiAction(action),
            // A disabled button is never hit or pressed (it used to carry no
            // action at all).
            Pressable {
                disabled: !enabled,
                ..default()
            },
            TestId::new(action.test_id()),
        ))
        .with_child((
            Text::new(label),
            ui::text(13.),
            TextColor(if enabled { ui::IVORY } else { ui::MUTED }),
        ));
}

impl Action {
    fn test_id(self) -> String {
        match self {
            Self::Close => "SupporterClose".into(),
            Self::Select(style) => format!("SupporterAura-{}", style.id()),
            Self::Equip => "SupporterEquip".into(),
            Self::Disable => "SupporterDisable".into(),
            Self::Refresh => "SupporterRefresh".into(),
            Self::Purchase => "SupporterPurchase".into(),
            Self::Restore => "SupporterRestore".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supporter_panel_scrolls_by_wheel_and_drag_while_it_is_the_top_modal() {
        use crate::platform::UiProfile;
        use crate::ui::scroll::harness;
        for profile in [UiProfile::Desktop, UiProfile::Mobile] {
            let mut app = App::new();
            let window = harness::install(&mut app, profile);
            let mut stack = crate::ui::ModalStack::default();
            stack.push(ModalId::Supporter);
            app.insert_resource(stack);
            let root = app
                .world_mut()
                .spawn((SupporterRoot, ModalRoot(ModalId::Supporter)))
                .id();
            let panel = app
                .world_mut()
                .spawn((SupporterScroll, panel_scroll(), ChildOf(root)))
                .id();
            let center = Vec2::new(400.0, 300.0);
            harness::measure(&mut app, panel, center);
            harness::wheel_lines(&mut app, window, -1.0);
            assert_eq!(harness::offset(&app, panel), 24.0, "{profile:?}");
            harness::drag(&mut app, window, 1, center, 30.0);
            assert_eq!(harness::offset(&app, panel), 54.0, "{profile:?}");
            // Covered by another modal: the panel keeps its offset.
            app.world_mut()
                .resource_mut::<crate::ui::ModalStack>()
                .pop(ModalId::Supporter);
            app.world_mut()
                .resource_mut::<crate::ui::ModalStack>()
                .push(ModalId::Pause);
            harness::wheel_lines(&mut app, window, -1.0);
            assert_eq!(harness::offset(&app, panel), 54.0, "{profile:?}");
        }
    }
    #[test]
    fn orbital_paths_are_continuous_bounded_and_time_based() {
        for i in 0..ORBIT_COUNT {
            for tick in 0..1200 {
                let t = tick as f64 / 60.;
                let p = orbital_point(i as f32, t);
                assert!(p.is_finite());
                assert!(p.y > 0.08 && p.y < 1.6);
                assert!(p.x.abs() <= 1. && p.z.abs() <= 1.);
                assert!(p.distance(orbital_point(i as f32, t + 1. / 60.)) < 0.025);
            }
        }
        // Three distinct planes, not three lights on the same horizontal ring.
        assert!(orbital_point(0., 0.).distance(orbital_point(1., 0.)) > 0.4);
        let n = |i: f32| {
            let speed = 1.30 + i as f64 * 0.13;
            let a = orbital_point(i, 0.) - Vec3::Y * 0.82;
            let b = orbital_point(i, std::f64::consts::FRAC_PI_2 / speed) - Vec3::Y * 0.82;
            a.cross(b).normalize()
        };
        assert!(n(0.).dot(n(1.)).abs() < 0.9);
    }

    #[test]
    fn trails_taper_and_flat_particles_cross_in_front_of_and_behind_actor() {
        for flat in [false, true] {
            let mut front = false;
            let mut back = false;
            for tick in 0..120 {
                let t = tick as f64 / 30.;
                for particle in [
                    AuraParticle::Core,
                    AuraParticle::Halo,
                    AuraParticle::Trail(0),
                    AuraParticle::Trail(TRAIL_SAMPLES - 1),
                    AuraParticle::Glint,
                ] {
                    let pose = particle_pose(
                        &Orbit {
                            index: 0.,
                            flat,
                            particle,
                            preview: false,
                        },
                        t,
                    );
                    assert!(
                        pose.translation.is_finite()
                            && pose.scale.is_finite()
                            && pose.rotation.is_finite()
                    );
                    assert!(pose.scale.min_element() >= 0.);
                    if flat {
                        front |= pose.translation.z > 10.;
                        back |= pose.translation.z < 1.;
                    }
                }
            }
            if flat {
                assert!(front && back);
            }
            let first = particle_pose(
                &Orbit {
                    index: 0.,
                    flat,
                    particle: AuraParticle::Trail(0),
                    preview: false,
                },
                2.,
            );
            let last = particle_pose(
                &Orbit {
                    index: 0.,
                    flat,
                    particle: AuraParticle::Trail(TRAIL_SAMPLES - 1),
                    preview: false,
                },
                2.,
            );
            assert!(last.scale.y < first.scale.y * 0.2);
        }
    }

    #[test]
    fn hidden_and_dead_actors_never_have_visible_aura() {
        let mut stats = CombatStats::default();
        assert!(actor_visible(
            &stats,
            &Visibility::Inherited,
            &InheritedVisibility::VISIBLE
        ));
        assert!(!actor_visible(
            &stats,
            &Visibility::Hidden,
            &InheritedVisibility::VISIBLE
        ));
        assert!(!actor_visible(
            &stats,
            &Visibility::Inherited,
            &InheritedVisibility::HIDDEN
        ));
        stats.hp = 0.;
        assert!(!actor_visible(
            &stats,
            &Visibility::Visible,
            &InheritedVisibility::VISIBLE
        ));
    }
    #[test]
    fn aura_lifecycle_tracks_server_style_and_despawns_with_owner_in_both_modes() {
        for (mode, remote) in [PlayerVisualMode::Models3d, PlayerVisualMode::Sprite2d]
            .into_iter()
            .flat_map(|mode| [(mode, false), (mode, true)])
        {
            let mut app = App::new();
            app.insert_resource(mode)
                .init_resource::<AuraRegistry>()
                .insert_resource(AuraAssets {
                    ring: default(),
                    orb: default(),
                    flat_orb: default(),
                    glow_quad: default(),
                    material: default(),
                    flat: default(),
                    cores: default(),
                    halos: default(),
                    trails: default(),
                    flat_cores: default(),
                    flat_halos: default(),
                    flat_trails: default(),
                    preview: default(),
                })
                .add_systems(Update, sync_auras);
            let actor = app
                .world_mut()
                .spawn((Player, NetworkSupporterAura(None)))
                .id();
            if remote {
                app.world_mut()
                    .entity_mut(actor)
                    .remove::<Player>()
                    .insert(RemotePlayer);
            }
            app.update();
            assert!(app.world().resource::<AuraRegistry>().0.is_empty());
            app.world_mut()
                .get_mut::<NetworkSupporterAura>(actor)
                .unwrap()
                .0 = Some(AuraStyle::Solar);
            app.update();
            let first = app.world().resource::<AuraRegistry>().0[&actor].0;
            let children = app
                .world()
                .get::<Children>(first)
                .unwrap()
                .iter()
                .collect::<Vec<_>>();
            assert_eq!(
                children.len(),
                1 + ORBIT_COUNT * (2 + TRAIL_SAMPLES) + GLINT_COUNT
            );
            app.world_mut()
                .get_mut::<NetworkSupporterAura>(actor)
                .unwrap()
                .0 = Some(AuraStyle::Lunar);
            app.update();
            assert!(app.world().get_entity(first).is_err());
            let second = app.world().resource::<AuraRegistry>().0[&actor].0;
            app.world_mut().despawn(actor);
            app.update();
            assert!(app.world().get_entity(second).is_err());
            assert!(app.world().resource::<AuraRegistry>().0.is_empty());
            for child in children {
                assert!(app.world().get_entity(child).is_err());
            }
        }
    }
    #[test]
    fn live_aura_follows_current_ground_pose_and_hides_children_without_a_frame_leak() {
        for (mode, remote) in [PlayerVisualMode::Models3d, PlayerVisualMode::Sprite2d]
            .into_iter()
            .flat_map(|mode| [(mode, false), (mode, true)])
        {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, TransformPlugin))
                .insert_resource(mode)
                .init_resource::<AuraRegistry>()
                .init_resource::<crate::maps::MapLayout>()
                .insert_resource(AuraAssets {
                    ring: default(),
                    orb: default(),
                    flat_orb: default(),
                    glow_quad: default(),
                    material: default(),
                    flat: default(),
                    cores: default(),
                    halos: default(),
                    trails: default(),
                    flat_cores: default(),
                    flat_halos: default(),
                    flat_trails: default(),
                    preview: default(),
                })
                .add_systems(Update, sync_auras)
                .add_systems(
                    PostUpdate,
                    (follow_actors, animate_orbits)
                        .chain()
                        .after(bevy::transform::TransformSystems::Propagate),
                );
            let actor = app
                .world_mut()
                .spawn((
                    Player,
                    NetworkSupporterAura(Some(AuraStyle::Solar)),
                    CombatStats::default(),
                    Transform::from_xyz(3., 0., 4.),
                    Visibility::Inherited,
                    InheritedVisibility::VISIBLE,
                ))
                .id();
            if remote {
                app.world_mut()
                    .entity_mut(actor)
                    .remove::<Player>()
                    .insert(RemotePlayer);
            }
            app.update();
            let aura = app.world().resource::<AuraRegistry>().0[&actor].0;
            assert!(app.world().get::<InheritedVisibility>(aura).unwrap().get());
            let before = app
                .world()
                .get::<GlobalTransform>(aura)
                .unwrap()
                .translation();
            let mut pose = app.world_mut().get_mut::<Transform>(actor).unwrap();
            pose.translation.x += 2.;
            pose.rotation = Quat::from_rotation_y(2.);
            app.update();
            let after = app
                .world()
                .get::<GlobalTransform>(aura)
                .unwrap()
                .translation();
            assert!((after.x - before.x - 2.).abs() < 0.0001);
            assert_eq!(
                app.world().get::<Transform>(aura).unwrap().rotation,
                Quat::IDENTITY
            );
            *app.world_mut()
                .get_mut::<InheritedVisibility>(actor)
                .unwrap() = InheritedVisibility::HIDDEN;
            app.update();
            assert!(!app.world().get::<InheritedVisibility>(aura).unwrap().get());
            for child in app.world().get::<Children>(aura).unwrap().iter() {
                assert!(!app.world().get::<InheritedVisibility>(child).unwrap().get());
            }
            *app.world_mut()
                .get_mut::<InheritedVisibility>(actor)
                .unwrap() = InheritedVisibility::VISIBLE;
            app.world_mut().get_mut::<CombatStats>(actor).unwrap().hp = 0.;
            app.update();
            assert!(!app.world().get::<InheritedVisibility>(aura).unwrap().get());
        }
    }
    #[test]
    fn purchase_requires_known_inactive_authenticated_profile_and_never_stacks_memberships() {
        let mut career = CareerClient::default();
        assert!(!purchase_allowed(&career));
        career.view.profile = Some(shared::career::ProfileSummary::new(
            "a".repeat(64),
            "Tester".into(),
        ));
        career.view.auth_nonce = Some("b".repeat(64));
        career.view.storage_enabled = true;
        assert!(!purchase_allowed(&career));
        career.view.supporter = Some(SupporterStatus::default());
        assert!(purchase_allowed(&career));
        career.view.supporter.as_mut().unwrap().grants.push(
            shared::supporter::SupporterGrantSummary {
                provider: "solana".into(),
                valid_from: 200,
                valid_until: 300,
                revoked: false,
                renewal_enabled: Some(false),
            },
        );
        assert!(!purchase_allowed_at(&career, 100));
        assert!(purchase_allowed_at(&career, 300));
        career.view.supporter.as_mut().unwrap().grants.clear();
        career.view.supporter.as_mut().unwrap().active = true;
        assert!(!purchase_allowed(&career));
        career.view.supporter.as_mut().unwrap().active = false;
        career.view.error = Some("Unavailable".into());
        assert!(!purchase_allowed(&career));
    }
    #[test]
    fn timestamp_and_empty_status_are_explicit() {
        assert_eq!(utc_timestamp(0), "1970-01-01 00:00");
        assert_eq!(utc_timestamp(1789516800), "2026-09-16 00:00");
        assert!(status_label(Some(&SupporterStatus::default())).contains("only visible here"));
    }
    #[test]
    fn preview_cannot_grant_aura_or_change_combat_stats() {
        let mut app = App::new();
        app.init_resource::<SupporterUiState>();
        let actor = app
            .world_mut()
            .spawn((CombatStats::default(), NetworkSupporterAura(None)))
            .id();
        app.world_mut().resource_mut::<SupporterUiState>().open = true;
        app.world_mut().resource_mut::<SupporterUiState>().selected = AuraStyle::Verdant;
        assert_eq!(
            app.world().get::<NetworkSupporterAura>(actor).unwrap().0,
            None
        );
        assert_eq!(
            app.world().get::<CombatStats>(actor).unwrap().hp,
            CombatStats::default().hp
        );
    }

    #[test]
    fn supporter_presses_apply_once_and_disabled_buttons_do_nothing() {
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        app.init_resource::<SupporterUiState>()
            .init_resource::<SupporterPlatformState>()
            .init_resource::<CareerClient>()
            .add_message::<SupporterPlatformAction>()
            .add_message::<NetworkCommand>()
            .add_ui_action::<Action>()
            .add_systems(Update, actions.after(UiSet::Dispatch));
        harness::spawn_ui(app.world_mut(), |row| {
            button(row, "Refresh".into(), Action::Refresh, true);
            button(row, "Hide my aura".into(), Action::Disable, false);
            button(row, "Close".into(), Action::Close, true);
        });
        app.world_mut().resource_mut::<SupporterUiState>().open = true;
        app.update();
        let requests = |app: &mut App| {
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .count()
        };
        harness::press(app.world_mut(), "SupporterRefresh");
        app.update();
        assert_eq!(requests(&mut app), 1);
        app.update();
        assert_eq!(requests(&mut app), 0, "a press fires once");
        harness::drain_actions::<Action>(app.world_mut());
        // Rendered disabled (inactive membership): never pressed.
        harness::press(app.world_mut(), "SupporterDisable");
        app.update();
        assert_eq!(requests(&mut app), 0);
        assert!(harness::drain_actions::<Action>(app.world_mut()).is_empty());
        harness::press(app.world_mut(), "SupporterClose");
        app.update();
        assert!(!app.world().resource::<SupporterUiState>().open);
    }
}
