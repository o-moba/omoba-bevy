//! Verdant3d owns the imported foliage scene and reuses `DecorRoot` for F4.
//! Historical scatter fixtures remain test-only to retain layout regressions;
//! the unchanged World2d renderer owns its separate tile/prop presentation.
use crate::sprite::PlayerVisualMode;
use bevy::prelude::*;

pub struct DecorPlugin;

impl Plugin for DecorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, toggle_decor_visibility);
    }
}

#[derive(Component)]
pub struct DecorRoot;

fn toggle_decor_visibility(
    keyboard: Res<ButtonInput<KeyCode>>,
    mode: Option<Res<PlayerVisualMode>>,
    mut roots: Query<&mut Visibility, With<DecorRoot>>,
) {
    if mode.as_deref() == Some(&PlayerVisualMode::Sprite2d) || !keyboard.just_pressed(KeyCode::F4) {
        return;
    }
    for mut visibility in &mut roots {
        *visibility = if *visibility == Visibility::Hidden {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Legacy deterministic layout regression fixture. Its old primitive renderer
/// is retired; the runtime Verdant geometry has its own artifact validator.
#[cfg(test)]
mod layout {
    use bevy::math::Vec2;
    use std::f32::consts::TAU;

    use crate::maps::{BASE_PAD_SIZE, LANE_WIDTH, MapLayout, RIVER_WIDTH};

    /// Fixed compile-time seed for the shipped layout.
    pub(crate) const DECOR_SEED: u64 = 0x00DE_C018_0000_5EED;
    /// Legacy scatter placement regression ceiling.
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
    use super::{DecorRoot, toggle_decor_visibility};

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
    fn legacy_scatter_stays_bounded_and_dense() {
        let placements = generate_layout(DECOR_SEED);
        // Retain the scatter density regression without resurrecting retired
        // primitive parts. Current 3D ownership is checked in verdant3d tests.
        let total_entities = placements.len() + 1;
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
        // Preserve the density of the legacy fixture.
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
