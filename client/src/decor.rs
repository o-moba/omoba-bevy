//! Environment decoration (TASK-18): purely cosmetic vegetation and props
//! assembled from Bevy mesh primitives (no external art assets).
//!
//! A deterministic, seeded layout ([`layout::generate_layout`]) scatters trees,
//! bushes, grass tufts, flowers, and rocks across the arena while excluding
//! every gameplay zone (lanes, base pads, towers, camp clearings, river,
//! jungle blocks) derived from the same [`MapLayout`](crate::maps::MapLayout)
//! math the map renderer uses. All props spawn once at `Startup` as children
//! of a single `DecorRoot` entity; F4 flips the root's [`Visibility`].
//!
//! Performance: every prop part reuses one of [`MESH_VARIANTS`] shared mesh
//! handles and [`MATERIAL_VARIANTS`] shared material handles (stored in
//! [`DecorAssets`]), so Bevy can batch instances. Decor has no collision and
//! no per-frame system beyond the F4 toggle.

use bevy::prelude::*;

use crate::sprite::PlayerVisualMode;
use layout::PropKind;

/// Number of distinct shared decor meshes (one per primitive shape used).
pub const MESH_VARIANTS: usize = 5;
/// Number of distinct shared decor materials (one per palette color).
pub const MATERIAL_VARIANTS: usize = 12;

pub struct DecorPlugin;

impl Plugin for DecorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_decor)
            .add_systems(Update, (sync_decor_visual_mode, toggle_decor_visibility));
    }
}

/// Marker for the single parent entity all decor props hang off.
#[derive(Component)]
pub struct DecorRoot;

/// Shared mesh/material handles reused by every decor prop instance.
#[derive(Resource)]
pub struct DecorAssets {
    meshes: [Handle<Mesh>; MESH_VARIANTS],
    materials: [Handle<StandardMaterial>; MATERIAL_VARIANTS],
}

/// Index into [`DecorAssets::meshes`]; each variant is a unit-sized primitive
/// scaled per part via its `Transform`.
#[derive(Clone, Copy)]
enum PartMesh {
    Cylinder,
    Sphere,
    Cone,
    Cuboid,
    Capsule,
}

/// Index into [`DecorAssets::materials`]: a stylized palette that harmonizes
/// with the dark-green terrain (rgb 0.08, 0.22, 0.10).
#[derive(Clone, Copy)]
enum PartMaterial {
    TrunkBrown,
    TrunkPale,
    CanopyBright,
    CanopyDeep,
    BushGreen,
    GrassGreen,
    StemGreen,
    FlowerWhite,
    FlowerYellow,
    FlowerRed,
    FlowerViolet,
    RockGray,
}

impl DecorAssets {
    fn build(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Self {
        let mut matte = |color: Color| {
            materials.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: 0.9,
                ..default()
            })
        };

        Self {
            meshes: [
                meshes.add(Cylinder::new(0.5, 1.0)),
                meshes.add(Sphere::new(0.5)),
                meshes.add(Cone {
                    radius: 0.5,
                    height: 1.0,
                }),
                meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
                meshes.add(Capsule3d::new(0.5, 1.0)),
            ],
            materials: [
                matte(Color::srgb(0.30, 0.20, 0.12)),
                matte(Color::srgb(0.55, 0.52, 0.45)),
                matte(Color::srgb(0.20, 0.42, 0.18)),
                matte(Color::srgb(0.13, 0.34, 0.15)),
                matte(Color::srgb(0.18, 0.38, 0.16)),
                matte(Color::srgb(0.25, 0.45, 0.18)),
                matte(Color::srgb(0.16, 0.35, 0.14)),
                matte(Color::srgb(0.92, 0.92, 0.88)),
                matte(Color::srgb(0.85, 0.72, 0.20)),
                matte(Color::srgb(0.75, 0.20, 0.18)),
                matte(Color::srgb(0.55, 0.30, 0.70)),
                matte(Color::srgb(0.42, 0.42, 0.44)),
            ],
        }
    }

    fn mesh(&self, mesh: PartMesh) -> Handle<Mesh> {
        self.meshes[mesh as usize].clone()
    }

    fn material(&self, material: PartMaterial) -> Handle<StandardMaterial> {
        self.materials[material as usize].clone()
    }
}

/// One primitive part of a composed prop, relative to the prop origin.
struct PartSpec {
    mesh: PartMesh,
    material: PartMaterial,
    offset: Vec3,
    rotation: Quat,
    scale: Vec3,
}

impl PartSpec {
    fn new(mesh: PartMesh, material: PartMaterial, offset: Vec3, scale: Vec3) -> Self {
        Self {
            mesh,
            material,
            offset,
            rotation: Quat::IDENTITY,
            scale,
        }
    }

    fn rotated(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }
}

/// Primitive-part assembly for each prop variant. Pure data: unit meshes
/// positioned/scaled relative to the prop's ground-level origin.
fn prop_parts(kind: PropKind) -> Vec<PartSpec> {
    use PartMaterial as M;
    use PartMesh as P;

    match kind {
        // Round tree: trunk plus a clustered three-sphere canopy.
        PropKind::TreeOak => vec![
            PartSpec::new(
                P::Cylinder,
                M::TrunkBrown,
                Vec3::new(0.0, 1.2, 0.0),
                Vec3::new(0.7, 2.4, 0.7),
            ),
            PartSpec::new(
                P::Sphere,
                M::CanopyBright,
                Vec3::new(0.0, 3.3, 0.0),
                Vec3::splat(3.2),
            ),
            PartSpec::new(
                P::Sphere,
                M::CanopyBright,
                Vec3::new(1.0, 2.8, 0.4),
                Vec3::splat(2.2),
            ),
            PartSpec::new(
                P::Sphere,
                M::CanopyDeep,
                Vec3::new(-0.9, 2.9, -0.4),
                Vec3::splat(2.0),
            ),
        ],
        // Conifer: trunk plus two stacked cones.
        PropKind::TreePine => vec![
            PartSpec::new(
                P::Cylinder,
                M::TrunkBrown,
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.55, 2.0, 0.55),
            ),
            PartSpec::new(
                P::Cone,
                M::CanopyDeep,
                Vec3::new(0.0, 3.0, 0.0),
                Vec3::new(3.2, 2.6, 3.2),
            ),
            PartSpec::new(
                P::Cone,
                M::CanopyDeep,
                Vec3::new(0.0, 4.6, 0.0),
                Vec3::new(2.2, 2.2, 2.2),
            ),
        ],
        // Slender pale-trunk tree with a tall two-sphere canopy.
        PropKind::TreeBirch => vec![
            PartSpec::new(
                P::Cylinder,
                M::TrunkPale,
                Vec3::new(0.0, 1.5, 0.0),
                Vec3::new(0.42, 3.0, 0.42),
            ),
            PartSpec::new(
                P::Sphere,
                M::CanopyBright,
                Vec3::new(0.0, 3.9, 0.0),
                Vec3::new(2.0, 2.6, 2.0),
            ),
            PartSpec::new(
                P::Sphere,
                M::CanopyBright,
                Vec3::new(0.5, 3.0, 0.3),
                Vec3::splat(1.4),
            ),
        ],
        // Two overlapping flattened spheres.
        PropKind::BushRound => vec![
            PartSpec::new(
                P::Sphere,
                M::BushGreen,
                Vec3::new(0.0, 0.55, 0.0),
                Vec3::new(1.8, 1.3, 1.8),
            ),
            PartSpec::new(
                P::Sphere,
                M::BushGreen,
                Vec3::new(0.7, 0.45, 0.2),
                Vec3::new(1.2, 0.9, 1.2),
            ),
        ],
        // Low hedge: a capsule lying on its side.
        PropKind::BushLow => vec![
            PartSpec::new(
                P::Capsule,
                M::BushGreen,
                Vec3::new(0.0, 0.5, 0.0),
                Vec3::new(1.2, 1.2, 1.4),
            )
            .rotated(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
        ],
        // Three thin, slightly splayed cones as grass blades.
        PropKind::GrassTuft => vec![
            PartSpec::new(
                P::Cone,
                M::GrassGreen,
                Vec3::new(0.0, 0.45, 0.0),
                Vec3::new(0.25, 0.9, 0.25),
            ),
            PartSpec::new(
                P::Cone,
                M::GrassGreen,
                Vec3::new(0.18, 0.38, 0.10),
                Vec3::new(0.20, 0.75, 0.20),
            )
            .rotated(Quat::from_rotation_z(-0.25)),
            PartSpec::new(
                P::Cone,
                M::GrassGreen,
                Vec3::new(-0.15, 0.38, 0.12),
                Vec3::new(0.20, 0.7, 0.20),
            )
            .rotated(Quat::from_rotation_x(0.22)),
        ],
        // Stem plus a round white head.
        PropKind::FlowerDaisy => vec![
            PartSpec::new(
                P::Cylinder,
                M::StemGreen,
                Vec3::new(0.0, 0.35, 0.0),
                Vec3::new(0.08, 0.7, 0.08),
            ),
            PartSpec::new(
                P::Sphere,
                M::FlowerWhite,
                Vec3::new(0.0, 0.78, 0.0),
                Vec3::splat(0.36),
            ),
        ],
        // Stem plus a larger yellow head.
        PropKind::FlowerSun => vec![
            PartSpec::new(
                P::Cylinder,
                M::StemGreen,
                Vec3::new(0.0, 0.4, 0.0),
                Vec3::new(0.08, 0.8, 0.08),
            ),
            PartSpec::new(
                P::Sphere,
                M::FlowerYellow,
                Vec3::new(0.0, 0.88, 0.0),
                Vec3::splat(0.42),
            ),
        ],
        // Stem plus a red cone bud.
        PropKind::FlowerTulip => vec![
            PartSpec::new(
                P::Cylinder,
                M::StemGreen,
                Vec3::new(0.0, 0.35, 0.0),
                Vec3::new(0.08, 0.7, 0.08),
            ),
            PartSpec::new(
                P::Cone,
                M::FlowerRed,
                Vec3::new(0.0, 0.85, 0.0),
                Vec3::new(0.30, 0.35, 0.30),
            ),
        ],
        // Stem plus a drooping violet bell (elongated sphere).
        PropKind::FlowerBell => vec![
            PartSpec::new(
                P::Cylinder,
                M::StemGreen,
                Vec3::new(0.0, 0.32, 0.0),
                Vec3::new(0.08, 0.64, 0.08),
            ),
            PartSpec::new(
                P::Sphere,
                M::FlowerViolet,
                Vec3::new(0.0, 0.72, 0.0),
                Vec3::new(0.30, 0.40, 0.30),
            ),
        ],
        // Single tilted cuboid, partially sunk into the ground.
        PropKind::RockSmall => vec![
            PartSpec::new(
                P::Cuboid,
                M::RockGray,
                Vec3::new(0.0, 0.25, 0.0),
                Vec3::new(0.9, 0.6, 0.7),
            )
            .rotated(Quat::from_rotation_z(0.2)),
        ],
        // Two overlapping cuboids forming a larger boulder.
        PropKind::RockBoulder => vec![
            PartSpec::new(
                P::Cuboid,
                M::RockGray,
                Vec3::new(0.0, 0.45, 0.0),
                Vec3::new(1.6, 1.1, 1.3),
            )
            .rotated(Quat::from_rotation_x(0.12)),
            PartSpec::new(
                P::Cuboid,
                M::RockGray,
                Vec3::new(0.7, 0.35, 0.4),
                Vec3::new(1.0, 0.8, 0.9),
            )
            .rotated(Quat::from_rotation_y(0.6)),
        ],
    }
}

/// Spawns the whole decoration layer once: shared assets, `DecorRoot`, and
/// every prop part as a child entity reusing the shared handles.
fn spawn_decor(
    mut commands: Commands,
    visual_mode: Res<PlayerVisualMode>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // `World2dPlugin` owns the sprite-mode prop layer.  Do not instantiate
    // hidden Mesh3d decoration in a genuine 2D session.
    if *visual_mode == PlayerVisualMode::Sprite2d {
        return;
    }
    let assets = DecorAssets::build(&mut meshes, &mut materials);
    let placements = layout::generate_layout(layout::DECOR_SEED);

    let mut parts_spawned: usize = 0;
    commands
        .spawn((
            DecorRoot,
            Transform::default(),
            if *visual_mode == PlayerVisualMode::Sprite2d {
                Visibility::Hidden
            } else {
                Visibility::Visible
            },
            Name::new("DecorRoot"),
        ))
        .with_children(|root| {
            for placement in &placements {
                let yaw = Quat::from_rotation_y(placement.yaw);
                let base = Vec3::new(placement.position.x, 0.0, placement.position.y);
                for part in prop_parts(placement.kind) {
                    root.spawn((
                        Mesh3d(assets.mesh(part.mesh)),
                        MeshMaterial3d(assets.material(part.material)),
                        Transform {
                            translation: base + yaw * (part.offset * placement.scale),
                            rotation: yaw * part.rotation,
                            scale: part.scale * placement.scale,
                        },
                    ));
                    parts_spawned += 1;
                }
            }
        });

    let total_entities = parts_spawned + 1;
    if total_entities > layout::MAX_DECOR_ENTITIES {
        warn!(
            "decor: {} entities exceed the documented budget of {}",
            total_entities,
            layout::MAX_DECOR_ENTITIES
        );
    }
    info!(
        "decor: spawned {} decor entities ({} props + 1 root, budget {}) reusing {} shared meshes and {} shared materials; F4 toggles visibility",
        total_entities,
        placements.len(),
        layout::MAX_DECOR_ENTITIES,
        MESH_VARIANTS,
        MATERIAL_VARIANTS,
    );
}

fn sync_decor_visual_mode(
    mode: Res<PlayerVisualMode>,
    mut roots: Query<&mut Visibility, With<DecorRoot>>,
) {
    if !mode.is_changed() {
        return;
    }
    for mut visibility in &mut roots {
        *visibility = if *mode == PlayerVisualMode::Sprite2d {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
}

/// Client-local debug toggle (F4): flips `Visibility` on the `DecorRoot`, so
/// the whole decoration layer hides/shows at once. No network message.
fn toggle_decor_visibility(
    keyboard: Res<ButtonInput<KeyCode>>,
    mode: Option<Res<PlayerVisualMode>>,
    mut roots: Query<&mut Visibility, With<DecorRoot>>,
) {
    if mode.as_deref() == Some(&PlayerVisualMode::Sprite2d) || !keyboard.just_pressed(KeyCode::F4) {
        return;
    }
    for mut visibility in &mut roots {
        let next = if matches!(*visibility, Visibility::Hidden) {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        *visibility = next;
        info!(
            "[debug] decor_visible -> {}",
            matches!(next, Visibility::Visible)
        );
    }
}

/// Pure, render-independent layout generation: a seeded PRNG scatters prop
/// placements across the arena, rejecting any candidate inside a gameplay
/// exclusion zone. Everything here is unit-testable without a window.
pub(crate) mod layout {
    use bevy::math::Vec2;
    use std::f32::consts::TAU;

    use crate::maps::{BASE_PAD_SIZE, LANE_WIDTH, MapLayout, RIVER_WIDTH};

    /// Fixed compile-time seed for the shipped layout.
    pub(crate) const DECOR_SEED: u64 = 0x00DE_C018_0000_5EED;
    /// Hard ceiling for spawned decor entities (prop parts + the root).
    pub(crate) const MAX_DECOR_ENTITIES: usize = 1200;

    /// Keep-out distance from lane center lines (half width + margin).
    pub(crate) const LANE_CLEAR: f32 = LANE_WIDTH * 0.5 + 1.5;
    /// Keep-out distance from the river center line; the river stays fully
    /// clear of decor per the frozen spec policy.
    pub(crate) const RIVER_CLEAR: f32 = RIVER_WIDTH * 0.5 + 1.0;
    /// Chebyshev keep-out from base pad centers (half pad + margin).
    pub(crate) const BASE_PAD_CLEAR: f32 = BASE_PAD_SIZE * 0.5 + 2.0;
    /// Clear radius around each lane tower (~2.3x TOWER_SIZE = 2.6).
    pub(crate) const TOWER_CLEAR: f32 = 6.0;
    /// Clear radius around each neutral camp center, keeping camps readable.
    pub(crate) const CAMP_CLEAR: f32 = 8.0;
    /// Chebyshev keep-out from jungle block centers (half block 6.0 + margin).
    pub(crate) const JUNGLE_BLOCK_CLEAR: f32 = 7.0;
    /// Inset from the arena bounds so props do not clip the boundary walls.
    const EDGE_INSET: f32 = 1.5;
    /// Depth of the decorative forest belt hugging the arena edges.
    const BELT_DEPTH: f32 = 9.0;
    /// Radius of the tree/rock scatter ring around each jungle block.
    const JUNGLE_RING_RADIUS: f32 = 16.0;

    /// Lane towers sit at these fractions along each lane polyline
    /// (mirrors `build_structures` in server/src/world.rs).
    const TOWER_LANE_FRACTIONS: [f32; 2] = [0.30, 0.70];

    // Scatter targets per category (upper bounds; rejection sampling may
    // place slightly fewer in heavily excluded regions).
    const EDGE_TREE_TARGET: usize = 60;
    const JUNGLE_TREE_TARGET: usize = 30;
    const JUNGLE_ROCK_TARGET: usize = 22;
    const BUSH_TARGET: usize = 60;
    const GRASS_TARGET: usize = 110;
    const FLOWER_TARGET: usize = 90;
    const MEADOW_ROCK_TARGET: usize = 24;

    /// Every decor prop variant. Trees/bushes/flowers/rocks count as distinct
    /// assemblies (different part composition or palette), not scale jitter.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub(crate) enum PropKind {
        TreeOak,
        TreePine,
        TreeBirch,
        BushRound,
        BushLow,
        GrassTuft,
        FlowerDaisy,
        FlowerSun,
        FlowerTulip,
        FlowerBell,
        RockSmall,
        RockBoulder,
    }

    /// Test support: every variant, used to assert layout coverage.
    #[cfg(test)]
    pub(crate) const ALL_PROP_KINDS: [PropKind; 12] = [
        PropKind::TreeOak,
        PropKind::TreePine,
        PropKind::TreeBirch,
        PropKind::BushRound,
        PropKind::BushLow,
        PropKind::GrassTuft,
        PropKind::FlowerDaisy,
        PropKind::FlowerSun,
        PropKind::FlowerTulip,
        PropKind::FlowerBell,
        PropKind::RockSmall,
        PropKind::RockBoulder,
    ];

    const TREE_VARIANTS: [PropKind; 3] =
        [PropKind::TreeOak, PropKind::TreePine, PropKind::TreeBirch];
    const BUSH_VARIANTS: [PropKind; 2] = [PropKind::BushRound, PropKind::BushLow];
    const FLOWER_VARIANTS: [PropKind; 4] = [
        PropKind::FlowerDaisy,
        PropKind::FlowerSun,
        PropKind::FlowerTulip,
        PropKind::FlowerBell,
    ];
    const ROCK_VARIANTS: [PropKind; 2] = [PropKind::RockBoulder, PropKind::RockSmall];

    /// One prop instance: kind, XZ position, yaw, and uniform scale jitter.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(crate) struct Placement {
        pub kind: PropKind,
        pub position: Vec2,
        pub yaw: f32,
        pub scale: f32,
    }

    /// Minimal deterministic PRNG (splitmix64); no external `rand` crate.
    pub(crate) struct DecorRng(u64);

    impl DecorRng {
        pub(crate) fn new(seed: u64) -> Self {
            Self(seed)
        }

        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        /// Uniform float in [0, 1).
        fn next_f32(&mut self) -> f32 {
            (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
        }

        fn range(&mut self, min: f32, max: f32) -> f32 {
            min + (max - min) * self.next_f32()
        }

        fn index(&mut self, len: usize) -> usize {
            (self.next_u64() % len as u64) as usize
        }
    }

    /// Gameplay keep-out geometry, all derived from [`MapLayout`] — the same
    /// constants the map renderer and (mirrored) server logic use.
    pub(crate) struct ExclusionZones {
        pub min: Vec2,
        pub max: Vec2,
        pub lanes: [Vec<Vec2>; 3],
        pub river: [Vec2; 2],
        pub base_pads: [Vec2; 2],
        pub towers: Vec<Vec2>,
        pub camps: [Vec2; 3],
        pub jungle_blocks: [Vec2; 10],
    }

    impl ExclusionZones {
        pub(crate) fn from_map() -> Self {
            let layout = MapLayout::default();
            let lanes = layout.lane_polylines();
            let mut towers = Vec::new();
            for lane in &lanes {
                for fraction in TOWER_LANE_FRACTIONS {
                    towers.push(sample_polyline(lane, fraction));
                }
            }
            // Base towers stand at the spawn centers, inside the base pads.
            let base_pads = [
                Vec2::new(layout.home_spawn.x, layout.home_spawn.z),
                Vec2::new(layout.away_spawn.x, layout.away_spawn.z),
            ];
            towers.extend_from_slice(&base_pads);

            Self {
                min: layout.min,
                max: layout.max,
                lanes,
                river: layout.river_polyline(),
                base_pads,
                towers,
                camps: layout.camp_centers(),
                jungle_blocks: layout.jungle_block_centers(),
            }
        }

        pub(crate) fn in_bounds(&self, p: Vec2) -> bool {
            p.x >= self.min.x + EDGE_INSET
                && p.x <= self.max.x - EDGE_INSET
                && p.y >= self.min.y + EDGE_INSET
                && p.y <= self.max.y - EDGE_INSET
        }

        /// True when `p` falls inside any gameplay keep-out zone.
        pub(crate) fn blocked(&self, p: Vec2) -> bool {
            self.lanes
                .iter()
                .any(|lane| polyline_distance(p, lane) < LANE_CLEAR)
                || point_segment_distance(p, self.river[0], self.river[1]) < RIVER_CLEAR
                || self
                    .base_pads
                    .iter()
                    .any(|pad| chebyshev(p, *pad) < BASE_PAD_CLEAR)
                || self.towers.iter().any(|t| p.distance(*t) < TOWER_CLEAR)
                || self.camps.iter().any(|c| p.distance(*c) < CAMP_CLEAR)
                || self
                    .jungle_blocks
                    .iter()
                    .any(|b| chebyshev(p, *b) < JUNGLE_BLOCK_CLEAR)
        }
    }

    pub(crate) fn chebyshev(p: Vec2, center: Vec2) -> f32 {
        let d = (p - center).abs();
        d.x.max(d.y)
    }

    pub(crate) fn point_segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
        let ab = b - a;
        let len_sq = ab.length_squared();
        if len_sq <= f32::EPSILON {
            return p.distance(a);
        }
        let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
        p.distance(a + ab * t)
    }

    pub(crate) fn polyline_distance(p: Vec2, points: &[Vec2]) -> f32 {
        points
            .windows(2)
            .map(|pair| point_segment_distance(p, pair[0], pair[1]))
            .fold(f32::INFINITY, f32::min)
    }

    /// Position at fraction `t` of a polyline's total length (mirrors the
    /// server's `sample_polyline_position`).
    fn sample_polyline(points: &[Vec2], t: f32) -> Vec2 {
        let lengths: Vec<f32> = points
            .windows(2)
            .map(|pair| pair[0].distance(pair[1]))
            .collect();
        let total: f32 = lengths.iter().sum();
        if total <= 0.0001 {
            return points[0];
        }
        let mut remaining = total * t.clamp(0.0, 1.0);
        for (index, length) in lengths.into_iter().enumerate() {
            if remaining <= length {
                let local_t = if length <= 0.0001 {
                    0.0
                } else {
                    remaining / length
                };
                return points[index].lerp(points[index + 1], local_t);
            }
            remaining -= length;
        }
        *points.last().expect("polyline has points")
    }

    /// Deterministic scatter: same seed, same placements, same order.
    pub(crate) fn generate_layout(seed: u64) -> Vec<Placement> {
        let zones = ExclusionZones::from_map();
        let mut rng = DecorRng::new(seed);
        let mut placements = Vec::new();

        // Forest belts hugging the arena edges.
        scatter(
            &mut rng,
            &zones,
            &mut placements,
            EDGE_TREE_TARGET,
            &TREE_VARIANTS,
            (0.85, 1.25),
            sample_edge_belt,
        );
        // Trees and boulders ringing the jungle blocks in each quadrant.
        scatter(
            &mut rng,
            &zones,
            &mut placements,
            JUNGLE_TREE_TARGET,
            &TREE_VARIANTS,
            (0.8, 1.2),
            sample_jungle_ring,
        );
        scatter(
            &mut rng,
            &zones,
            &mut placements,
            JUNGLE_ROCK_TARGET,
            &ROCK_VARIANTS,
            (0.7, 1.6),
            sample_jungle_ring,
        );
        // Bushes, grass, flowers, and small rocks across the open meadow.
        scatter(
            &mut rng,
            &zones,
            &mut placements,
            BUSH_TARGET,
            &BUSH_VARIANTS,
            (0.8, 1.3),
            sample_uniform,
        );
        scatter(
            &mut rng,
            &zones,
            &mut placements,
            GRASS_TARGET,
            &[PropKind::GrassTuft],
            (0.8, 1.4),
            sample_uniform,
        );
        scatter(
            &mut rng,
            &zones,
            &mut placements,
            FLOWER_TARGET,
            &FLOWER_VARIANTS,
            (0.9, 1.3),
            sample_uniform,
        );
        scatter(
            &mut rng,
            &zones,
            &mut placements,
            MEADOW_ROCK_TARGET,
            &ROCK_VARIANTS,
            (0.7, 1.4),
            sample_uniform,
        );

        placements
    }

    /// Rejection-samples up to `target` placements from `sample`, cycling
    /// through `kinds` so every variant is guaranteed to appear.
    fn scatter(
        rng: &mut DecorRng,
        zones: &ExclusionZones,
        out: &mut Vec<Placement>,
        target: usize,
        kinds: &[PropKind],
        scale_range: (f32, f32),
        sample: fn(&mut DecorRng, &ExclusionZones) -> Vec2,
    ) {
        let mut placed = 0;
        let max_attempts = target * 30;
        for _ in 0..max_attempts {
            if placed >= target {
                break;
            }
            let position = sample(rng, zones);
            if !zones.in_bounds(position) || zones.blocked(position) {
                continue;
            }
            out.push(Placement {
                kind: kinds[placed % kinds.len()],
                position,
                yaw: rng.range(0.0, TAU),
                scale: rng.range(scale_range.0, scale_range.1),
            });
            placed += 1;
        }
    }

    fn sample_uniform(rng: &mut DecorRng, zones: &ExclusionZones) -> Vec2 {
        Vec2::new(
            rng.range(zones.min.x + EDGE_INSET, zones.max.x - EDGE_INSET),
            rng.range(zones.min.y + EDGE_INSET, zones.max.y - EDGE_INSET),
        )
    }

    /// A point inside the belt band along one of the four arena edges.
    fn sample_edge_belt(rng: &mut DecorRng, zones: &ExclusionZones) -> Vec2 {
        let along = rng.range(zones.min.x + EDGE_INSET, zones.max.x - EDGE_INSET);
        let depth = rng.range(EDGE_INSET, BELT_DEPTH);
        match rng.index(4) {
            0 => Vec2::new(along, zones.max.y - depth),
            1 => Vec2::new(along, zones.min.y + depth),
            2 => Vec2::new(zones.min.x + depth, along),
            _ => Vec2::new(zones.max.x - depth, along),
        }
    }

    /// A point in an annulus around a random jungle block.
    fn sample_jungle_ring(rng: &mut DecorRng, zones: &ExclusionZones) -> Vec2 {
        let block = zones.jungle_blocks[rng.index(zones.jungle_blocks.len())];
        let angle = rng.range(0.0, TAU);
        let radius = rng.range(JUNGLE_BLOCK_CLEAR + 0.5, JUNGLE_RING_RADIUS);
        block + Vec2::new(angle.cos(), angle.sin()) * radius
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use bevy::prelude::*;

    use super::layout::{
        ALL_PROP_KINDS, BASE_PAD_CLEAR, CAMP_CLEAR, DECOR_SEED, ExclusionZones, JUNGLE_BLOCK_CLEAR,
        LANE_CLEAR, MAX_DECOR_ENTITIES, PropKind, RIVER_CLEAR, TOWER_CLEAR, chebyshev,
        generate_layout, point_segment_distance, polyline_distance,
    };
    use super::{DecorRoot, prop_parts, toggle_decor_visibility};

    #[test]
    fn layout_is_deterministic_for_the_same_seed() {
        assert_eq!(generate_layout(DECOR_SEED), generate_layout(DECOR_SEED));
    }

    #[test]
    fn layout_covers_every_prop_kind() {
        let kinds: HashSet<PropKind> = generate_layout(DECOR_SEED)
            .iter()
            .map(|placement| placement.kind)
            .collect();
        for kind in ALL_PROP_KINDS {
            assert!(kinds.contains(&kind), "missing prop kind {kind:?}");
        }
    }

    #[test]
    fn placements_stay_in_bounds_and_outside_gameplay_zones() {
        let zones = ExclusionZones::from_map();
        for placement in generate_layout(DECOR_SEED) {
            let p = placement.position;
            assert!(
                p.x >= zones.min.x
                    && p.x <= zones.max.x
                    && p.y >= zones.min.y
                    && p.y <= zones.max.y,
                "{p:?} outside arena bounds"
            );
            for lane in &zones.lanes {
                assert!(
                    polyline_distance(p, lane) >= LANE_CLEAR,
                    "{p:?} inside lane clearance"
                );
            }
            assert!(
                point_segment_distance(p, zones.river[0], zones.river[1]) >= RIVER_CLEAR,
                "{p:?} inside river clearance"
            );
            for pad in &zones.base_pads {
                assert!(
                    chebyshev(p, *pad) >= BASE_PAD_CLEAR,
                    "{p:?} inside base pad clearance"
                );
            }
            for tower in &zones.towers {
                assert!(
                    p.distance(*tower) >= TOWER_CLEAR,
                    "{p:?} inside tower clearance"
                );
            }
            for camp in &zones.camps {
                assert!(
                    p.distance(*camp) >= CAMP_CLEAR,
                    "{p:?} inside camp clearance"
                );
            }
            for block in &zones.jungle_blocks {
                assert!(
                    chebyshev(p, *block) >= JUNGLE_BLOCK_CLEAR,
                    "{p:?} inside jungle block clearance"
                );
            }
        }
    }

    #[test]
    fn layout_fits_entity_budget_and_stays_dense() {
        let placements = generate_layout(DECOR_SEED);
        let parts: usize = placements
            .iter()
            .map(|placement| prop_parts(placement.kind).len())
            .sum();
        let total_entities = parts + 1; // + DecorRoot
        println!(
            "decor layout: {} props, {} entities (budget {})",
            placements.len(),
            total_entities,
            MAX_DECOR_ENTITIES
        );
        assert!(
            total_entities <= MAX_DECOR_ENTITIES,
            "{total_entities} decor entities exceed the {MAX_DECOR_ENTITIES} budget"
        );
        // The arena should actually look decorated, not sparse.
        let count = |kinds: &[PropKind]| {
            placements
                .iter()
                .filter(|placement| kinds.contains(&placement.kind))
                .count()
        };
        assert!(
            count(&[PropKind::TreeOak, PropKind::TreePine, PropKind::TreeBirch]) >= 40,
            "too few trees"
        );
        assert!(count(&[PropKind::GrassTuft]) >= 60, "too little grass");
        assert!(
            count(&[
                PropKind::FlowerDaisy,
                PropKind::FlowerSun,
                PropKind::FlowerTulip,
                PropKind::FlowerBell,
            ]) >= 40,
            "too few flowers"
        );
    }

    #[test]
    fn f4_toggles_decor_root_visibility() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.add_systems(Update, toggle_decor_visibility);
        let root = app.world_mut().spawn((DecorRoot, Visibility::Visible)).id();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::F4);
        app.update();
        assert_eq!(
            *app.world().entity(root).get::<Visibility>().unwrap(),
            Visibility::Hidden
        );

        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release(KeyCode::F4);
        input.clear();
        input.press(KeyCode::F4);
        app.update();
        assert_eq!(
            *app.world().entity(root).get::<Visibility>().unwrap(),
            Visibility::Visible
        );
    }
}
