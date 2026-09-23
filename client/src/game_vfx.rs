//! Bounded combat particles and presentation of authoritative healing butterflies.
//! Neither trails nor pickup presentation can award damage or healing.
use crate::{
    camera::MainCamera,
    maps::MapLayout,
    sprite::PlayerVisualMode,
    world2d::{layer, simulation_xz_to_render_xy},
};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use shared::combat::ProjectileStyle;

const PARTICLE_BUDGET: usize = 256;
const BUTTERFLY_BUDGET: usize = shared::forest_pickups::FOREST_PICKUP_COUNT * 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BurstKind {
    Melee,
    Magic,
    Ranged,
}
impl BurstKind {
    pub(crate) fn for_style(style: ProjectileStyle) -> Self {
        match style {
            ProjectileStyle::Arcane | ProjectileStyle::Holy | ProjectileStyle::CasterBolt => {
                Self::Magic
            }
            ProjectileStyle::Crescent | ProjectileStyle::Standard => Self::Melee,
            _ => Self::Ranged,
        }
    }
}
#[derive(Message, Clone)]
pub(crate) struct ImpactBurst {
    pub position: Vec3,
    pub direction: Vec2,
    pub color: Color,
    pub scale: f32,
    pub lifetime: f32,
    pub kind: BurstKind,
    pub seed: u64,
}
#[derive(Message)]
pub(crate) struct ClearCombatVfx;
#[derive(Message)]
struct FlightParticles(Vec<Particle>);
#[derive(Clone, Copy)]
enum Shape {
    Glow,
    Ring,
    Slash,
}
#[derive(Clone)]
struct Particle {
    event_id: u64,
    origin: Vec3,
    velocity: Vec3,
    age: f32,
    lifetime: f32,
    size: f32,
    angle: f32,
    color: Color,
    shape: Shape,
}
impl Particle {
    fn pose(&self, flat: bool, facing: Quat) -> Transform {
        let t = (self.age / self.lifetime).clamp(0., 1.);
        let p = self.origin + self.velocity * self.age;
        let position = if flat {
            simulation_xz_to_render_xy(p).extend(layer::VFX + 0.1)
        } else {
            p
        };
        let size = self.size
            * match self.shape {
                Shape::Glow => (1. - t).max(0.01),
                Shape::Ring => 0.4 + t * 1.6,
                Shape::Slash => 0.75 + t * 0.5,
            };
        let sweep = if matches!(self.shape, Shape::Slash) {
            t * 1.1
        } else {
            0.
        };
        let angle = if flat {
            self.angle
        } else {
            let projected = facing.inverse() * Vec3::new(self.angle.cos(), 0., self.angle.sin());
            projected.y.atan2(projected.x)
        };
        Transform::from_translation(position)
            .with_scale(Vec3::splat(size))
            .with_rotation(if flat {
                Quat::from_rotation_z(angle + sweep)
            } else {
                facing * Quat::from_rotation_z(angle + sweep)
            })
    }
}
#[derive(Component)]
pub(crate) struct ParticleSlot {
    active: Option<Particle>,
    material: Handle<StandardMaterial>,
    flat: Handle<ColorMaterial>,
}
impl ParticleSlot {
    pub(crate) fn sample(&self) -> Option<(u64, f32)> {
        self.active.as_ref().map(|p| (p.event_id, p.age))
    }
}
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct VfxPresentation;
#[derive(Component)]
pub(crate) struct ButterflyWing {
    pub(crate) pickup_id: u64,
    anchor: Vec3,
    phase: f32,
    side: f32,
    material: Handle<StandardMaterial>,
    flat: Handle<ColorMaterial>,
}
#[derive(Component)]
struct ButterflyGlow {
    pickup_id: u64,
    anchor: Vec3,
    phase: f32,
    material: Handle<StandardMaterial>,
    flat: Handle<ColorMaterial>,
}
#[derive(Resource, Default)]
struct PickupReceipts {
    identity: Option<(u64, u64)>,
    sequences: std::collections::HashMap<u64, u64>,
}

#[derive(Resource)]
struct VfxAssets {
    glow_texture: Handle<Image>,
    glow: Handle<Mesh>,
    ring: Handle<Mesh>,
    slash: Handle<Mesh>,
    wing: Handle<Mesh>,
}
impl VfxAssets {
    fn mesh(&self, shape: Shape) -> Handle<Mesh> {
        match shape {
            Shape::Glow => &self.glow,
            Shape::Ring => &self.ring,
            Shape::Slash => &self.slash,
        }
        .clone()
    }
}
pub(crate) struct GameVfxPlugin;
impl Plugin for GameVfxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PickupReceipts>()
            .add_message::<ImpactBurst>()
            .add_message::<ClearCombatVfx>()
            .add_message::<FlightParticles>()
            .add_systems(Startup, setup)
            .add_systems(
                PostUpdate,
                (
                    (
                        pickup_feedback,
                        emit_projectile_particles,
                        animate_particles,
                    )
                        .chain(),
                    animate_butterflies,
                    animate_butterfly_glows,
                )
                    .in_set(VfxPresentation)
                    .after(bevy::transform::TransformSystems::Propagate)
                    .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate)
                    .before(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
            );
    }
}
fn texture(wing: bool) -> Image {
    let mut pixels = Vec::with_capacity(64 * 64 * 4);
    for y in 0..64 {
        for x in 0..64 {
            let u = (x as f32 + 0.5) / 64.;
            let v = (y as f32 + 0.5) / 64.;
            if wing {
                let edge = u.min(1. - u).min(v.min(1. - v));
                let vein = ((v - 0.5).atan2(u + 0.15) * 7.).sin().abs() < 0.075;
                let spot = ((u - 0.7).powi(2) + (v - 0.32).powi(2)).sqrt();
                let value = if edge < 0.08 || vein || (0.05..0.095).contains(&spot) {
                    35
                } else if spot < 0.05 {
                    255
                } else {
                    200
                };
                pixels.extend_from_slice(&[value, value, value, 255]);
            } else {
                let r = Vec2::new(u * 2. - 1., v * 2. - 1.).length();
                let alpha = (1. - r).max(0.).powf(1.6);
                pixels.extend_from_slice(&[255, 255, 255, (alpha * 255.) as u8]);
            }
        }
    }
    Image::new(
        Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}
fn slash_mesh() -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for i in 0..=20 {
        let t = i as f32 / 20.;
        let a = -1.2 + t * 2.4;
        let width = (std::f32::consts::PI * t).sin() * 0.16;
        for radius in [0.7 - width, 0.7] {
            positions.push([a.cos() * radius, a.sin() * radius, 0.]);
        }
        if i < 20 {
            let j = i * 2;
            indices.extend_from_slice(&[j, j + 1, j + 2, j + 1, j + 3, j + 2]);
        }
    }
    let count = positions.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., 1.]; count])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.5, 0.5]; count])
    .with_inserted_indices(Indices::U32(indices))
}
fn wing_mesh() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [0., 0., 0.],
            [0.24, 0.17, 0.],
            [0.32, 0.05, 0.],
            [0.19, -0.14, 0.],
            [0.06, -0.11, 0.],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., 1.]; 5])
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0., 0.5], [0.75, 0.], [1., 0.4], [0.6, 1.], [0.2, 0.9]],
    )
    .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3, 0, 3, 4]))
}
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut flat_materials: ResMut<Assets<ColorMaterial>>,
    mode: Res<PlayerVisualMode>,
    map: Res<MapLayout>,
) {
    let glow = images.add(texture(false));
    let wing_texture = images.add(texture(true));
    let assets = VfxAssets {
        glow_texture: glow.clone(),
        glow: meshes.add(Rectangle::new(1., 1.)),
        ring: meshes.add(Annulus::new(0.43, 0.5)),
        slash: meshes.add(slash_mesh()),
        wing: meshes.add(wing_mesh()),
    };
    for _ in 0..PARTICLE_BUDGET {
        let material = materials.add(StandardMaterial {
            unlit: true,
            alpha_mode: AlphaMode::Add,
            base_color_texture: Some(glow.clone()),
            cull_mode: None,
            double_sided: true,
            ..default()
        });
        let flat = flat_materials.add(ColorMaterial {
            texture: Some(glow.clone()),
            ..default()
        });
        let mut e = commands.spawn((
            Name::new("Pooled cosmetic particle"),
            ParticleSlot {
                active: None,
                material: material.clone(),
                flat: flat.clone(),
            },
            Transform::default(),
            Visibility::Hidden,
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
        ));
        if *mode == PlayerVisualMode::Sprite2d {
            e.insert((Mesh2d(assets.glow.clone()), MeshMaterial2d(flat)));
        } else {
            e.insert((Mesh3d(assets.glow.clone()), MeshMaterial3d(material)));
        }
    }
    let colors = [Color::srgb(0.55, 1., 0.78), Color::srgb(1., 0.88, 0.42)];
    let wing_materials = colors.map(|color| {
        materials.add(StandardMaterial {
            base_color: color,
            base_color_texture: Some(wing_texture.clone()),
            unlit: true,
            cull_mode: None,
            double_sided: true,
            ..default()
        })
    });
    let wing_flat = colors.map(|color| {
        flat_materials.add(ColorMaterial {
            color,
            texture: Some(wing_texture.clone()),
            ..default()
        })
    });
    let anchors = shared::forest_pickups::pickup_layout();
    let glow_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.22, 1.0, 0.60, 0.42),
        base_color_texture: Some(glow.clone()),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        ..default()
    });
    let glow_flat = flat_materials.add(ColorMaterial {
        color: Color::srgba(0.22, 1.0, 0.60, 0.35),
        texture: Some(glow.clone()),
        ..default()
    });
    for i in 0..BUTTERFLY_BUDGET {
        let pickup_id = (i / 3 + 1) as u64;
        let anchor = Vec2::from_array(anchors[i / 3]);
        let phase = i as f32 * 2.399;
        let base = Vec3::new(
            anchor.x,
            map.terrain_height_3d(anchor.x, anchor.y),
            anchor.y,
        );
        let mut halo = commands.spawn((
            Name::new("Healing butterfly glow"),
            ButterflyGlow {
                pickup_id,
                anchor: base,
                phase,
                material: glow_material.clone(),
                flat: glow_flat.clone(),
            },
            Transform::default(),
            Visibility::Hidden,
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
        ));
        if *mode == PlayerVisualMode::Sprite2d {
            halo.insert((
                Mesh2d(assets.glow.clone()),
                MeshMaterial2d(glow_flat.clone()),
            ));
        } else {
            halo.insert((
                Mesh3d(assets.glow.clone()),
                MeshMaterial3d(glow_material.clone()),
            ));
        }
        for side in [-1., 1.] {
            let mut e = commands.spawn((
                Name::new("Forest butterfly wing"),
                ButterflyWing {
                    pickup_id,
                    anchor: base,
                    phase,
                    side,
                    material: wing_materials[i % 2].clone(),
                    flat: wing_flat[i % 2].clone(),
                },
                Transform::default(),
                Visibility::Hidden,
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
            ));
            if *mode == PlayerVisualMode::Sprite2d {
                e.insert((
                    Mesh2d(assets.wing.clone()),
                    MeshMaterial2d(wing_flat[i % 2].clone()),
                ));
            } else {
                e.insert((
                    Mesh3d(assets.wing.clone()),
                    MeshMaterial3d(wing_materials[i % 2].clone()),
                ));
            }
        }
    }
    commands.insert_resource(assets);
}
fn burst_particles(burst: &ImpactBurst) -> Vec<Particle> {
    if !burst.position.is_finite()
        || !burst.scale.is_finite()
        || !burst.lifetime.is_finite()
        || !burst.direction.is_finite()
    {
        return Vec::new();
    }
    let size = burst.scale.clamp(0.1, 2.);
    let lifetime = burst.lifetime.clamp(0.08, 1.2);
    let origin = burst.position + Vec3::Y * 0.75;
    let angle = burst.direction.y.atan2(burst.direction.x);
    let first = Particle {
        event_id: burst.seed,
        origin,
        velocity: Vec3::ZERO,
        age: 0.,
        lifetime,
        size: size * 2.4,
        angle,
        color: burst.color,
        shape: match burst.kind {
            BurstKind::Magic => Shape::Ring,
            BurstKind::Melee => Shape::Slash,
            BurstKind::Ranged => Shape::Glow,
        },
    };
    let mut particles = vec![
        first.clone(),
        Particle {
            shape: Shape::Glow,
            size: size * 2.0,
            lifetime: lifetime * 0.65,
            ..first.clone()
        },
    ];
    let count = match burst.kind {
        BurstKind::Magic => 10,
        BurstKind::Melee => 4,
        BurstKind::Ranged => 6,
    };
    for i in 0..count {
        let phase =
            (burst.seed % 997) as f32 * 0.03 + i as f32 * std::f32::consts::TAU / count as f32;
        let speed = if burst.kind == BurstKind::Melee {
            3.8
        } else {
            3.6
        };
        particles.push(Particle {
            velocity: Vec3::new(
                phase.cos() * speed,
                0.4 + (i % 3) as f32 * 0.6,
                phase.sin() * speed,
            ),
            size: size * 0.85,
            shape: Shape::Glow,
            lifetime: lifetime * 1.4,
            ..first.clone()
        });
    }
    particles
}
fn animate_particles(
    mut commands: Commands,
    time: Res<Time>,
    mode: Res<PlayerVisualMode>,
    assets: Res<VfxAssets>,
    mut bursts: MessageReader<ImpactBurst>,
    mut flight: MessageReader<FlightParticles>,
    mut resets: MessageReader<ClearCombatVfx>,
    cameras: Query<&GlobalTransform, (With<MainCamera>, Without<ParticleSlot>)>,
    mut slots: Query<(
        Entity,
        &mut ParticleSlot,
        &mut Transform,
        &mut GlobalTransform,
        &mut Visibility,
        &mut InheritedVisibility,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut flats: ResMut<Assets<ColorMaterial>>,
) {
    let flat = *mode == PlayerVisualMode::Sprite2d;
    let facing = cameras
        .single()
        .map(|p| p.to_scale_rotation_translation().1)
        .unwrap_or(Quat::IDENTITY);
    let clear = resets.read().next().is_some();
    for (_, mut slot, _, _, _, _) in &mut slots {
        if clear || mode.is_changed() {
            slot.active = None;
        }
        if let Some(p) = &mut slot.active {
            p.age += time.delta_secs();
            if p.age >= p.lifetime {
                slot.active = None;
            }
        }
    }
    // Confirmed impacts get first access; decorative flight particles cannot starve hits.
    let mut impacts = Vec::new();
    for burst in bursts.read() {
        if impacts.len() < 96 * 12 {
            impacts.extend(burst_particles(burst));
        }
    }
    let mut trails = Vec::new();
    for batch in flight.read() {
        trails.extend(
            batch
                .0
                .iter()
                .take(48usize.saturating_sub(trails.len()))
                .cloned(),
        );
    }
    let mut trail_count = slots
        .iter()
        .filter(|(_, slot, ..)| slot.active.as_ref().is_some_and(|p| p.event_id == 0))
        .count();
    for p in impacts.into_iter().chain(trails) {
        // Reserve half the pool for combat confirmations, even during sustained fire.
        if p.event_id == 0 {
            if trail_count >= PARTICLE_BUDGET / 2 {
                break;
            }
            trail_count += 1;
        }
        let Some((entity, mut slot, _, _, _, _)) =
            slots.iter_mut().find(|(_, slot, ..)| slot.active.is_none())
        else {
            break;
        };
        let mesh = assets.mesh(p.shape);
        // Ring/slash meshes need a solid tint; particles use the radial texture.
        let texture = matches!(p.shape, Shape::Glow).then(|| assets.glow_texture.clone());
        if let Some(m) = materials.get_mut(&slot.material) {
            m.base_color = p.color;
            m.base_color_texture = texture.clone();
            m.alpha_mode = AlphaMode::Blend;
        }
        if let Some(m) = flats.get_mut(&slot.flat) {
            m.color = p.color;
            m.texture = texture;
        }
        if flat {
            commands
                .entity(entity)
                .remove::<(Mesh3d, MeshMaterial3d<StandardMaterial>)>()
                .insert((Mesh2d(mesh), MeshMaterial2d(slot.flat.clone())));
        } else {
            commands
                .entity(entity)
                .remove::<(Mesh2d, MeshMaterial2d<ColorMaterial>)>()
                .insert((Mesh3d(mesh), MeshMaterial3d(slot.material.clone())));
        }
        slot.active = Some(p);
    }
    for (_, slot, mut transform, mut global, mut visibility, mut inherited) in &mut slots {
        let Some(p) = &slot.active else {
            *visibility = Visibility::Hidden;
            *inherited = InheritedVisibility::HIDDEN;
            continue;
        };
        *transform = p.pose(flat, facing);
        *global = GlobalTransform::from(*transform);
        *visibility = Visibility::Visible;
        *inherited = InheritedVisibility::VISIBLE;
        let opacity = (1. - p.age / p.lifetime).max(0.);
        if let Some(m) = materials.get_mut(&slot.material) {
            m.base_color = p.color.with_alpha(p.color.alpha() * opacity);
        }
        if let Some(m) = flats.get_mut(&slot.flat) {
            m.color = p.color.with_alpha(p.color.alpha() * opacity);
        }
    }
}
fn butterfly_position(anchor: Vec3, phase: f32, seconds: f64) -> Vec3 {
    let t = (seconds * 0.7).rem_euclid(std::f64::consts::TAU * 10.) as f32;
    anchor
        + Vec3::new(
            (t + phase).sin() * 0.68,
            1.35 + (t * 1.8 + phase).sin() * 0.30,
            (t * 0.8 + phase).cos() * 0.62,
        )
}
fn animate_butterflies(
    game: Option<Res<crate::net::GameStateSnapshot>>,
    mut commands: Commands,
    assets: Res<VfxAssets>,
    time: Res<Time>,
    mode: Res<PlayerVisualMode>,
    cameras: Query<(&Camera, &GlobalTransform), (With<MainCamera>, Without<ButterflyWing>)>,
    mut wings: Query<(
        Entity,
        &ButterflyWing,
        &mut Transform,
        &mut GlobalTransform,
        &mut Visibility,
        &mut InheritedVisibility,
    )>,
) {
    let camera = cameras.single().ok();
    let flat = *mode == PlayerVisualMode::Sprite2d;
    for (entity, wing, mut transform, mut global, mut visibility, mut inherited) in &mut wings {
        if mode.is_changed() {
            if flat {
                commands
                    .entity(entity)
                    .remove::<(Mesh3d, MeshMaterial3d<StandardMaterial>)>()
                    .insert((
                        Mesh2d(assets.wing.clone()),
                        MeshMaterial2d(wing.flat.clone()),
                    ));
            } else {
                commands
                    .entity(entity)
                    .remove::<(Mesh2d, MeshMaterial2d<ColorMaterial>)>()
                    .insert((
                        Mesh3d(assets.wing.clone()),
                        MeshMaterial3d(wing.material.clone()),
                    ));
            }
        }
        let anchor = pickup_anchor(game.as_deref(), wing.pickup_id, wing.anchor);
        let point = butterfly_position(anchor, wing.phase, time.elapsed_secs_f64());
        let position = if flat {
            simulation_xz_to_render_xy(point).extend(layer::VFX - 1.)
        } else {
            point
        };
        let available = pickup_available(game.as_deref(), wing.pickup_id);
        let visible = available
            && camera.is_some_and(|(cam, pose)| {
                cam.world_to_viewport(pose, position).is_ok_and(|p| {
                    cam.logical_viewport_size().is_some_and(|s| {
                        p.x >= -40. && p.y >= -40. && p.x <= s.x + 40. && p.y <= s.y + 40.
                    })
                })
            });
        *visibility = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        *inherited = if visible {
            InheritedVisibility::VISIBLE
        } else {
            InheritedVisibility::HIDDEN
        };
        if !visible {
            continue;
        }
        let flap = (time.elapsed_secs() * 12. + wing.phase).sin() * 0.9;
        *transform =
            Transform::from_translation(position).with_scale(Vec3::new(wing.side * 2.2, 2.2, 2.2));
        if flat {
            transform.scale.x *= 0.2 + flap.cos() * 0.8;
            transform.rotation = Quat::from_rotation_z(wing.phase * 0.3);
        } else {
            transform.rotation = Quat::from_rotation_y(wing.phase * 0.3)
                * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
                * Quat::from_rotation_y(flap * wing.side);
        }
        *global = GlobalTransform::from(*transform);
    }
}
/// Short tails sample authoritative positions; no stationary projectile invents a hit.
fn emit_projectile_particles(
    time: Res<Time>,
    mode: Res<PlayerVisualMode>,
    game: Option<Res<crate::net::GameStateSnapshot>>,
    cameras: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    registry: Option<Res<crate::combat_visuals::CombatVisualRegistry>>,
    owners: Query<(
        &crate::net::NetworkPlayerId,
        Option<&crate::net::NetworkHeroClass>,
        Option<&crate::net::NetworkAvatar>,
        Option<&crate::net::NetworkSpriteCharacter>,
    )>,
    projectiles: Query<(
        &Transform,
        &crate::net::NetworkProjectile,
        &InheritedVisibility,
    )>,
    mut clock: Local<(Option<(u64, u64)>, f32)>,
    mut output: MessageWriter<FlightParticles>,
) {
    if game
        .as_ref()
        .is_some_and(|g| !matches!(g.state, crate::net::GameState::Running))
    {
        return;
    }
    let identity = game
        .as_ref()
        .map(|g| (g.meta.server_epoch, g.meta.match_id));
    if clock.0 != identity {
        clock.0 = identity;
        clock.1 = 0.0;
    }
    clock.1 += time.delta_secs().min(0.1);
    if clock.1 < 0.045 {
        return;
    }
    clock.1 = 0.0;
    let Ok((camera, pose)) = cameras.single() else {
        return;
    };
    let Some(viewport) = camera.logical_viewport_size() else {
        return;
    };
    let flat = *mode == PlayerVisualMode::Sprite2d;
    let mut particles = Vec::new();
    for (transform, projectile, inherited) in &projectiles {
        if !inherited.get() || !transform.translation.is_finite() {
            continue;
        }
        let p = transform.translation;
        let render = if flat {
            simulation_xz_to_render_xy(p).extend(layer::VFX)
        } else {
            p
        };
        if !camera.world_to_viewport(pose, render).is_ok_and(|p| {
            p.x >= -10.0 && p.y >= -10.0 && p.x <= viewport.x + 10.0 && p.y <= viewport.y + 10.0
        }) {
            continue;
        }
        let magic = matches!(
            projectile.style,
            ProjectileStyle::Arcane | ProjectileStyle::Holy
        );
        let owner = matches!(
            projectile.source_kind,
            shared::combat::CombatEntityKind::Player | shared::combat::CombatEntityKind::Unknown
        )
        .then(|| owners.iter().find(|(id, ..)| id.0 == projectile.owner_id))
        .flatten();
        let profile = registry.as_ref().map(|r| {
            r.resolve(
                owner.and_then(|(_, c, _, _)| c.map(|c| c.0)),
                projectile.style,
                projectile.action_slot,
                owner.and_then(|(_, _, a, _)| a.and_then(|a| a.0.as_deref())),
                owner.and_then(|(_, _, _, s)| s.and_then(|s| s.0.as_deref())),
            )
        });
        let color = profile
            .map_or(Color::srgb(0.65, 0.8, 1.0), |p| p.color())
            .with_alpha(0.85);
        let scale = profile.map_or(1.0, |p| p.scale).clamp(0.2, 2.0);
        let direction = projectile.direction.normalize_or_zero();
        let base = Particle {
            event_id: 0,
            origin: p,
            velocity: -direction * 1.0,
            age: 0.0,
            lifetime: 0.26,
            size: (if magic { 1.5 } else { 0.65 }) * scale,
            angle: 0.0,
            color,
            shape: Shape::Glow,
        };
        particles.push(base.clone());
        if magic {
            let phase = time.elapsed_secs() * 9.0 + projectile.id as f32 % 100.0;
            for angle in [phase, phase + std::f32::consts::PI] {
                particles.push(Particle {
                    origin: p + if flat {
                        Vec3::new(angle.cos() * 0.50, 0.0, angle.sin() * 0.50)
                    } else {
                        Vec3::new(angle.cos() * 0.50, angle.sin() * 0.42, 0.0)
                    },
                    size: 0.45,
                    lifetime: 0.16,
                    color: Color::srgba(0.90, 0.85, 1.0, 0.9),
                    ..base.clone()
                });
            }
        }
        if particles.len() >= 45 {
            break;
        }
    }
    if !particles.is_empty() {
        output.write(FlightParticles(particles));
    }
}

fn pickup_available(game: Option<&crate::net::GameStateSnapshot>, id: u64) -> bool {
    game.is_some_and(|g| {
        matches!(g.state, crate::net::GameState::Running)
            && g.forest_pickups.iter().any(|p| p.id == id && p.available)
    })
}
fn pickup_anchor(game: Option<&crate::net::GameStateSnapshot>, id: u64, fallback: Vec3) -> Vec3 {
    game.and_then(|g| g.forest_pickups.iter().find(|p| p.id == id))
        .filter(|p| p.position.iter().all(|v| v.is_finite()))
        .map_or(fallback, |p| {
            Vec3::new(p.position[0], fallback.y, p.position[1])
        })
}
fn animate_butterfly_glows(
    mut commands: Commands,
    game: Option<Res<crate::net::GameStateSnapshot>>,
    time: Res<Time>,
    mode: Res<PlayerVisualMode>,
    assets: Res<VfxAssets>,
    cameras: Query<(&Camera, &GlobalTransform), (With<MainCamera>, Without<ButterflyGlow>)>,
    mut glows: Query<(
        Entity,
        &ButterflyGlow,
        &mut Transform,
        &mut GlobalTransform,
        &mut Visibility,
        &mut InheritedVisibility,
    )>,
) {
    let flat = *mode == PlayerVisualMode::Sprite2d;
    let camera = cameras.single().ok();
    for (entity, glow, mut pose, mut global, mut visible, mut inherited) in &mut glows {
        if mode.is_changed() {
            if flat {
                commands
                    .entity(entity)
                    .remove::<(Mesh3d, MeshMaterial3d<StandardMaterial>)>()
                    .insert((
                        Mesh2d(assets.glow.clone()),
                        MeshMaterial2d(glow.flat.clone()),
                    ));
            } else {
                commands
                    .entity(entity)
                    .remove::<(Mesh2d, MeshMaterial2d<ColorMaterial>)>()
                    .insert((
                        Mesh3d(assets.glow.clone()),
                        MeshMaterial3d(glow.material.clone()),
                    ));
            }
        }
        let anchor = pickup_anchor(game.as_deref(), glow.pickup_id, glow.anchor);
        let point = butterfly_position(anchor, glow.phase, time.elapsed_secs_f64());
        let position = if flat {
            simulation_xz_to_render_xy(point).extend(layer::VFX - 1.2)
        } else {
            point - Vec3::Y * 0.04
        };
        let on_screen = camera.is_some_and(|(cam, transform)| {
            cam.world_to_viewport(transform, position).is_ok_and(|p| {
                cam.logical_viewport_size().is_some_and(|s| {
                    p.x >= -40.0 && p.y >= -40.0 && p.x <= s.x + 40.0 && p.y <= s.y + 40.0
                })
            })
        });
        let show = pickup_available(game.as_deref(), glow.pickup_id) && on_screen;
        *visible = if show {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        *inherited = if show {
            InheritedVisibility::VISIBLE
        } else {
            InheritedVisibility::HIDDEN
        };
        let facing = if flat {
            Quat::IDENTITY
        } else {
            camera.map_or(Quat::IDENTITY, |(_, p)| p.to_scale_rotation_translation().1)
        };
        *pose = Transform::from_translation(position)
            .with_rotation(facing)
            .with_scale(Vec3::splat(
                2.0 + 0.20 * (time.elapsed_secs() * 3.0 + glow.phase).sin(),
            ));
        *global = GlobalTransform::from(*pose);
    }
}
fn pickup_feedback(
    game: Option<Res<crate::net::GameStateSnapshot>>,
    mut receipts: ResMut<PickupReceipts>,
    actors: Query<(
        &crate::net::NetworkPlayerId,
        &Transform,
        Option<&InheritedVisibility>,
        &crate::combat::CombatStats,
    )>,
    mut bursts: MessageWriter<ImpactBurst>,
    mut feedback: Option<ResMut<crate::combat::ActionFeedback>>,
) {
    let Some(game) = game.filter(|g| matches!(g.state, crate::net::GameState::Running)) else {
        *receipts = PickupReceipts::default();
        return;
    };
    let identity = (game.meta.server_epoch, game.meta.match_id);
    let fresh = receipts.identity != Some(identity);
    if fresh {
        receipts.identity = Some(identity);
        receipts.sequences.clear();
    }
    for pickup in game
        .forest_pickups
        .iter()
        .filter(|p| (1..=shared::forest_pickups::FOREST_PICKUP_COUNT as u64).contains(&p.id))
    {
        let previous = receipts.sequences.get(&pickup.id).copied();
        receipts.sequences.insert(
            pickup.id,
            previous.unwrap_or(0).max(pickup.collection_sequence),
        );
        if fresh
            || previous.is_none_or(|old| pickup.collection_sequence <= old)
            || !pickup.healed_amount.is_finite()
            || pickup.healed_amount <= 0.0
        {
            continue;
        }
        let Some((_, pose, visibility, stats)) = actors
            .iter()
            .find(|(id, _, _, _)| Some(id.0) == pickup.last_collector_id)
        else {
            continue;
        };
        if !stats.is_alive() || visibility.is_some_and(|v| !v.get()) {
            continue;
        }
        bursts.write(ImpactBurst {
            position: pose.translation,
            direction: Vec2::Y,
            color: Color::srgb(0.35, 1.0, 0.65),
            scale: 1.3,
            lifetime: 0.65,
            kind: BurstKind::Magic,
            seed: u64::MAX - pickup.id,
        });
        if pickup.last_collector_id == Some(game.your_id) {
            if let Some(feedback) = feedback.as_deref_mut() {
                feedback.push_line(format!(
                    "Forest butterfly · +{:.0} HP",
                    pickup.healed_amount
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pickup_receipts_seed_once_ignore_rollbacks_and_respect_hidden_collectors() {
        use crate::net::{GameState, GameStateSnapshot, NetworkPlayerId};
        let mut app = App::new();
        app.init_resource::<PickupReceipts>()
            .add_message::<ImpactBurst>()
            .add_systems(Update, pickup_feedback);
        let actor = app
            .world_mut()
            .spawn((
                NetworkPlayerId(7),
                Transform::default(),
                crate::combat::CombatStats::default(),
                InheritedVisibility::VISIBLE,
            ))
            .id();
        let mut game = GameStateSnapshot::default();
        game.state = GameState::Running;
        game.your_id = 7;
        game.forest_pickups
            .push(shared::forest_pickups::ForestPickupState {
                id: 1,
                position: [-66.0, 45.0],
                available: false,
                collection_sequence: 8,
                last_collector_id: Some(7),
                healed_amount: 40.0,
            });
        app.insert_resource(game);
        app.update(); // Connecting after a collection must not replay it.
        assert_eq!(app.world().resource::<Messages<ImpactBurst>>().len(), 0);
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .forest_pickups[0]
            .collection_sequence = 9;
        app.update();
        assert_eq!(app.world().resource::<Messages<ImpactBurst>>().len(), 1);
        app.world_mut()
            .resource_mut::<Messages<ImpactBurst>>()
            .clear();
        for sequence in [9, 8, 9] {
            app.world_mut()
                .resource_mut::<GameStateSnapshot>()
                .forest_pickups[0]
                .collection_sequence = sequence;
            app.update();
            assert_eq!(app.world().resource::<Messages<ImpactBurst>>().len(), 0);
        }
        app.world_mut()
            .entity_mut(actor)
            .insert(InheritedVisibility::HIDDEN);
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .forest_pickups[0]
            .collection_sequence = 10;
        app.update();
        assert_eq!(app.world().resource::<Messages<ImpactBurst>>().len(), 0);
        app.world_mut()
            .entity_mut(actor)
            .insert(InheritedVisibility::VISIBLE);
        app.update(); // Becoming visible cannot replay a hidden receipt.
        assert_eq!(app.world().resource::<Messages<ImpactBurst>>().len(), 0);
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .meta
            .match_id += 1;
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .forest_pickups[0]
            .collection_sequence = 11;
        app.update();
        assert_eq!(app.world().resource::<Messages<ImpactBurst>>().len(), 0);
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Lobby;
        app.update();
        assert!(
            app.world()
                .resource::<PickupReceipts>()
                .sequences
                .is_empty()
        );
    }
    #[test]
    fn pickup_visibility_follows_authority_and_drift_stays_collectible() {
        let mut game = crate::net::GameStateSnapshot::default();
        game.state = crate::net::GameState::Running;
        game.forest_pickups
            .push(shared::forest_pickups::ForestPickupState {
                id: 1,
                position: [-66.0, 45.0],
                available: true,
                collection_sequence: 0,
                last_collector_id: None,
                healed_amount: 0.0,
            });
        assert!(pickup_available(Some(&game), 1));
        assert!(!pickup_available(Some(&game), 2));
        assert!(!pickup_available(None, 1));
        let anchor = pickup_anchor(Some(&game), 1, Vec3::Y);
        assert_eq!(anchor, Vec3::new(-66.0, 1.0, 45.0));
        for frame in 0..100 {
            let point = butterfly_position(anchor, 0.7, frame as f64 * 0.1);
            assert!(point.xz().distance(anchor.xz()) < shared::forest_pickups::PICKUP_RADIUS);
            assert!(point.y > anchor.y + 1.0);
        }
        game.forest_pickups[0].available = false;
        assert!(!pickup_available(Some(&game), 1));
        game.forest_pickups[0].available = true;
        game.state = crate::net::GameState::Lobby;
        assert!(!pickup_available(Some(&game), 1));
    }
    fn test_burst() -> ImpactBurst {
        ImpactBurst {
            position: Vec3::ZERO,
            direction: Vec2::X,
            color: Color::WHITE,
            scale: 1.,
            lifetime: 1.,
            kind: BurstKind::Magic,
            seed: 1,
        }
    }
    #[test]
    fn pool_saturates_resets_and_reuses_entities_across_render_modes() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<ColorMaterial>>()
            .init_resource::<PlayerVisualMode>()
            .init_resource::<MapLayout>()
            .add_plugins(GameVfxPlugin);
        app.update();
        let count = app.world().entities().len();
        for _ in 0..96 {
            app.world_mut().write_message(test_burst());
        }
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ParticleSlot>()
                .iter(app.world())
                .filter(|s| s.active.is_some())
                .count(),
            PARTICLE_BUDGET
        );
        assert_eq!(app.world().entities().len(), count);
        app.world_mut().write_message(ClearCombatVfx);
        app.update();
        assert!(
            app.world_mut()
                .query::<(&ParticleSlot, &Visibility)>()
                .iter(app.world())
                .all(|(s, v)| s.active.is_none() && *v == Visibility::Hidden)
        );
        app.world_mut().insert_resource(PlayerVisualMode::Sprite2d);
        app.world_mut().write_message(test_burst());
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ParticleSlot>()
                .iter(app.world())
                .filter(|s| s.active.is_some())
                .count(),
            12
        );
        assert_eq!(
            app.world_mut()
                .query_filtered::<&Mesh2d, With<ButterflyWing>>()
                .iter(app.world())
                .count(),
            BUTTERFLY_BUDGET * 2
        );
        assert_eq!(app.world().entities().len(), count);
        // Expired particles free slots instead of growing the entity set.
        for mut slot in app
            .world_mut()
            .query::<&mut ParticleSlot>()
            .iter_mut(app.world_mut())
        {
            if let Some(p) = &mut slot.active {
                p.age = 10.;
            }
        }
        app.update();
        assert!(
            app.world_mut()
                .query::<&ParticleSlot>()
                .iter(app.world())
                .all(|s| s.active.is_none())
        );
    }
    #[test]
    fn sustained_trails_leave_reserved_capacity_and_never_replay_dropped_bursts() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<ColorMaterial>>()
            .init_resource::<PlayerVisualMode>()
            .init_resource::<MapLayout>()
            .add_plugins(GameVfxPlugin);
        app.update();
        let mut trail = burst_particles(&test_burst())[1].clone();
        trail.event_id = 0;
        trail.lifetime = 100.0;
        for _ in 0..10 {
            app.world_mut()
                .write_message(FlightParticles(vec![trail.clone(); 100]));
            app.update();
        }
        let count = app
            .world_mut()
            .query::<&ParticleSlot>()
            .iter(app.world())
            .filter(|s| s.active.as_ref().is_some_and(|p| p.event_id == 0))
            .count();
        assert_eq!(count, PARTICLE_BUDGET / 2);
        for _ in 0..200 {
            app.world_mut().write_message(test_burst());
        }
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ParticleSlot>()
                .iter(app.world())
                .filter(|s| s.active.as_ref().is_some_and(|p| p.event_id == 1))
                .count(),
            PARTICLE_BUDGET / 2
        );
        app.world_mut().write_message(ClearCombatVfx);
        app.update();
        app.update();
        assert!(
            app.world_mut()
                .query::<&ParticleSlot>()
                .iter(app.world())
                .all(|s| s.active.is_none())
        );
    }
    #[test]
    fn malformed_bursts_do_not_produce_particles() {
        let mut burst = test_burst();
        burst.direction.x = f32::NAN;
        assert!(burst_particles(&burst).is_empty());
        burst = test_burst();
        burst.position.y = f32::INFINITY;
        assert!(burst_particles(&burst).is_empty());
    }
    #[test]
    fn styles_dispatch_and_effect_lifetimes_are_bounded() {
        assert_eq!(
            BurstKind::for_style(ProjectileStyle::Arcane),
            BurstKind::Magic
        );
        assert_eq!(
            BurstKind::for_style(ProjectileStyle::Crescent),
            BurstKind::Melee
        );
        assert_eq!(
            BurstKind::for_style(ProjectileStyle::Arrow),
            BurstKind::Ranged
        );
        for kind in [BurstKind::Melee, BurstKind::Magic, BurstKind::Ranged] {
            let burst = ImpactBurst {
                position: Vec3::ZERO,
                direction: Vec2::X,
                color: Color::WHITE,
                scale: 100.,
                lifetime: 100.,
                kind,
                seed: 42,
            };
            let particles = burst_particles(&burst);
            assert!(particles.len() <= 12);
            for p in particles {
                assert!(p.lifetime <= 1.7);
                for flat in [false, true] {
                    let pose = p.pose(flat, Quat::IDENTITY);
                    assert!(pose.translation.is_finite() && pose.scale.is_finite());
                }
            }
        }
    }
    #[test]
    fn butterfly_paths_stay_near_forest_and_are_continuous() {
        for i in 0..BUTTERFLY_BUDGET {
            for frame in 0..1000 {
                let t = frame as f64 / 60.;
                let p = butterfly_position(Vec3::ZERO, i as f32, t);
                assert!(p.is_finite() && p.y >= 1.05 && p.y <= 1.66);
                assert!(p.xz().length() < shared::forest_pickups::PICKUP_RADIUS);
                assert!(p.distance(butterfly_position(Vec3::ZERO, i as f32, t + 1. / 60.)) < 0.03);
            }
        }
    }
}
