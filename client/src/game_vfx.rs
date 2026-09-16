//! Bounded, cosmetic-only particles. Hits arrive only from the accepted server
//! event cursor; ambient butterflies never become simulation entities.
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

const PARTICLE_BUDGET: usize = 128;
const BUTTERFLY_BUDGET: usize = 20;

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
    anchor: Vec3,
    phase: f32,
    side: f32,
    material: Handle<StandardMaterial>,
    flat: Handle<ColorMaterial>,
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
        app.add_message::<ImpactBurst>()
            .add_message::<ClearCombatVfx>()
            .add_systems(Startup, setup)
            .add_systems(
                PostUpdate,
                (animate_particles, animate_butterflies)
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
                let alpha = (1. - r).max(0.).powi(3);
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
    let colors = [Color::srgb(1., 0.65, 0.18), Color::srgb(0.3, 0.72, 1.)];
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
    let anchors = map.decorative_jungle_block_centers();
    for (i, anchor) in anchors
        .iter()
        .cycle()
        .take(if anchors.is_empty() {
            0
        } else {
            BUTTERFLY_BUDGET
        })
        .enumerate()
    {
        let phase = i as f32 * 2.399;
        let offset = Vec2::new(phase.cos(), phase.sin()) * 3.0;
        let anchor = *anchor + offset;
        let base = Vec3::new(
            anchor.x,
            map.terrain_height_3d(anchor.x, anchor.y),
            anchor.y,
        );
        for side in [-1., 1.] {
            let mut e = commands.spawn((
                Name::new("Forest butterfly wing"),
                ButterflyWing {
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
            size: size * 1.4,
            lifetime: lifetime * 0.65,
            ..first.clone()
        },
    ];
    let count = match burst.kind {
        BurstKind::Magic => 6,
        BurstKind::Melee => 4,
        BurstKind::Ranged => 2,
    };
    for i in 0..count {
        let phase =
            (burst.seed % 997) as f32 * 0.03 + i as f32 * std::f32::consts::TAU / count as f32;
        let speed = if burst.kind == BurstKind::Melee {
            3.8
        } else {
            2.2
        };
        particles.push(Particle {
            velocity: Vec3::new(
                phase.cos() * speed,
                0.4 + (i % 3) as f32 * 0.6,
                phase.sin() * speed,
            ),
            size: size * 0.36,
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
    for burst in bursts.read().take(96) {
        for p in burst_particles(burst) {
            let Some((entity, mut slot, _, _, _, _)) =
                slots.iter_mut().find(|(_, slot, ..)| slot.active.is_none())
            else {
                break;
            };
            let mesh = assets.mesh(p.shape);
            // Ring/slash meshes need a solid tint; particles use the radial texture.
            let texture = matches!(p.shape, Shape::Glow).then(|| assets.glow_texture.clone());
            if let Some(mut m) = materials.get_mut(&slot.material) {
                m.base_color = p.color;
                m.base_color_texture = texture.clone();
                m.alpha_mode = if matches!(p.shape, Shape::Glow) {
                    AlphaMode::Add
                } else {
                    AlphaMode::Blend
                };
            }
            if let Some(mut m) = flats.get_mut(&slot.flat) {
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
        if let Some(mut m) = materials.get_mut(&slot.material) {
            m.base_color = p.color.with_alpha(opacity);
        }
        if let Some(mut m) = flats.get_mut(&slot.flat) {
            m.color = p.color.with_alpha(opacity);
        }
    }
}
fn butterfly_position(anchor: Vec3, phase: f32, seconds: f64) -> Vec3 {
    let t = (seconds * 0.7).rem_euclid(std::f64::consts::TAU * 10.) as f32;
    anchor
        + Vec3::new(
            (t + phase).sin() * 0.9,
            0.7 + (t * 1.8 + phase).sin() * 0.23,
            (t * 0.8 + phase).cos() * 0.75,
        )
}
fn animate_butterflies(
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
        let point = butterfly_position(wing.anchor, wing.phase, time.elapsed_secs_f64());
        let position = if flat {
            simulation_xz_to_render_xy(point).extend(layer::VFX - 1.)
        } else {
            point
        };
        let visible = camera.is_some_and(|(cam, pose)| {
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
        *transform = Transform::from_translation(position).with_scale(Vec3::new(wing.side, 1., 1.));
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
#[cfg(test)]
mod tests {
    use super::*;
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
            8
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
            assert!(particles.len() <= 8);
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
                assert!(p.is_finite() && p.y > 0.4 && p.y < 1.);
                assert!(p.length() < 1.7);
                assert!(p.distance(butterfly_position(Vec3::ZERO, i as f32, t + 1. / 60.)) < 0.03);
            }
        }
    }
}
