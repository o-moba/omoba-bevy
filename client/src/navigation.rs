//! Bevy coordinate adapter for the shared forest/structure navigation module.

use bevy::prelude::*;
#[cfg(test)]
use shared::navigation::NavigationMap;
use shared::navigation::{Bounds, Disc, HERO_RADIUS, world_navigation};

use crate::maps::MapLayout;
use crate::net::StructureKind;

/// Physical hero-center clearance used by the existing structure collision.
pub(crate) fn structure_collision_radius(kind: StructureKind) -> f32 {
    HERO_RADIUS
        + match kind {
            StructureKind::Tower => shared::TOWER_TARGET_RADIUS,
            StructureKind::BaseTower => shared::navigation::BASE_COLLISION_RADIUS,
        }
}

/// Plan once per order; waypoints exclude the start and retain its height.
/// Custom test arenas get their own empty static map instead of global trees.
#[cfg(test)]
pub(crate) fn plan_route(
    layout: &MapLayout,
    start: Vec3,
    destination: Vec3,
    structures: &[(Vec3, StructureKind)],
) -> Option<Vec<Vec3>> {
    plan_route_with_terrain(layout, start, destination, structures, &[])
}
pub(crate) fn plan_route_with_terrain(
    layout: &MapLayout,
    start: Vec3,
    destination: Vec3,
    structures: &[(Vec3, StructureKind)],
    terrain: &[Disc],
) -> Option<Vec<Vec3>> {
    if !start.is_finite()
        || !destination.is_finite()
        || !layout.min.is_finite()
        || !layout.max.is_finite()
        || structures.iter().any(|(position, _)| !position.is_finite())
    {
        return None;
    }
    let bounds = Bounds {
        min: layout.min.to_array(),
        max: layout.max.to_array(),
    };
    let world = world_navigation();
    let world_bounds = world.bounds();
    #[cfg(test)]
    let custom;
    let map = if (0..2).all(|axis| {
        (bounds.min[axis] - world_bounds.min[axis]).abs() < 0.001
            && (bounds.max[axis] - world_bounds.max[axis]).abs() < 0.001
    }) {
        world
    } else {
        // Synthetic empty arenas are fixtures only. Production must never
        // silently discard forest collision for an unsupported map geometry.
        #[cfg(test)]
        {
            custom = NavigationMap::new(bounds, Vec::new()).ok()?;
            &custom
        }
        #[cfg(not(test))]
        {
            return None;
        }
    };
    let mut dynamic: Vec<_> = structures
        .iter()
        .map(|&(position, kind)| Disc {
            center: [position.x, position.z],
            radius: structure_collision_radius(kind) - HERO_RADIUS,
        })
        .collect();
    dynamic.extend_from_slice(terrain);
    map.plan_route([start.x, start.z], [destination.x, destination.z], &dynamic)
        .map(|route| {
            route
                .into_iter()
                .map(|point| Vec3::new(point[0], start.y, point[1]))
                .collect()
        })
}

/// Temporary terrain uses the same discs for prediction, route planning and
/// authoritative movement. Only armed, visible server objects participate.
pub(crate) fn skill_terrain(game: Option<&crate::net::GameStateSnapshot>) -> Vec<Disc> {
    game.into_iter()
        .flat_map(|g| g.skill_effects.iter())
        .filter(|e| {
            e.armed
                && e.remaining_secs > 0.0
                && matches!(
                    shared::loadout::skill(e.skill).effect,
                    shared::loadout::SkillEffect::Technique {
                        action: shared::loadout::Technique::TerrainLine,
                        ..
                    }
                )
        })
        .map(|e| Disc {
            center: e.position,
            radius: e.radius,
        })
        .collect()
}
pub(crate) fn clip_skill_terrain(
    from: Vec3,
    to: Vec3,
    game: Option<&crate::net::GameStateSnapshot>,
) -> Vec3 {
    let p = shared::navigation::clip_discs(
        from.xz().to_array(),
        to.xz().to_array(),
        &skill_terrain(game),
    );
    Vec3::new(p[0], to.y, p[1])
}

/// Whether the server lets a blink put the hero's centre on `point` (`legal_landing`,
/// `common/src/skills/advanced.rs`): the static map is clear there, the point is outside
/// the disc of every standing structure, and it is a hero's radius away from every armed
/// pillar. `structures` are the discs a ground move is clipped against. The server keeps a
/// landing out of a structure's disc only, which is nearer than a walking hero gets; the
/// client must not be stricter than that, or it would refuse casts the server accepts.
pub(crate) fn blink_point_legal(point: Vec2, structures: &[Disc], terrain: &[Disc]) -> bool {
    let apart = |disc: &Disc| point.distance(Vec2::from_array(disc.center));
    world_navigation().point_clear(point.to_array())
        && structures.iter().all(|disc| apart(disc) > disc.radius)
        && terrain
            .iter()
            .all(|disc| apart(disc) > disc.radius + HERO_RADIUS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::navigation::{MAX_DYNAMIC_DISCS as MAX_OBSTACLES, PLANNING_CLEARANCE as CLEARANCE};

    fn in_bounds(layout: &MapLayout, point: Vec2) -> bool {
        point.cmpge(layout.min).all() && point.cmple(layout.max).all()
    }

    fn arena(half_size: f32) -> MapLayout {
        MapLayout {
            min: Vec2::splat(-half_size),
            max: Vec2::splat(half_size),
            ..default()
        }
    }

    // Independently sample the traveled polyline densely against the physical
    // collision discs, including every corner and the final destination.
    fn assert_safe(
        layout: &MapLayout,
        start: Vec3,
        route: &[Vec3],
        structures: &[(Vec3, StructureKind)],
    ) {
        let mut previous = start;
        for &waypoint in route {
            assert_eq!(waypoint.y, start.y);
            let steps = (previous.distance(waypoint) / 0.01).ceil().max(1.0) as usize;
            for step in 0..=steps {
                let point = previous.lerp(waypoint, step as f32 / steps as f32);
                assert!(in_bounds(layout, Vec2::new(point.x, point.z)));
                for &(center, kind) in structures {
                    let distance = Vec2::new(point.x - center.x, point.z - center.z).length();
                    assert!(
                        distance >= structure_collision_radius(kind),
                        "segment hit {kind:?}"
                    );
                }
            }
            previous = waypoint;
        }
    }

    #[test]
    fn direct_route_preserves_height_clamps_destination_and_completes() {
        let layout = arena(10.0);
        let start = Vec3::new(-5.0, 0.7, 0.0);
        let route = plan_route(&layout, start, Vec3::new(30.0, 99.0, 4.0), &[]).unwrap();
        assert_eq!(route, [Vec3::new(10.0, 0.7, 4.0)]);
        assert_safe(&layout, start, &route, &[]);
        assert!(
            plan_route(&layout, route[0], route[0], &[])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn tower_and_base_routes_clear_every_segment() {
        for kind in [StructureKind::Tower, StructureKind::BaseTower] {
            let layout = arena(20.0);
            let structures = [(Vec3::ZERO, kind)];
            let start = Vec3::new(-10.0, 0.5, 0.0);
            let destination = Vec3::new(10.0, 0.5, 0.0);
            let route = plan_route(&layout, start, destination, &structures).unwrap();
            assert!(route.len() >= 2);
            assert_eq!(route.last(), Some(&destination));
            assert_safe(&layout, start, &route, &structures);
        }
    }

    #[test]
    fn real_map_base_detours_stay_short_on_both_sides_and_after_translation() {
        // Native minimap QA found a 150 m detour for this 14 m order. Preserve
        // the real map's two bases and six lane towers, including their large
        // absolute coordinates, instead of testing only a circle at the origin.
        let layout = arena(108.549_515);
        let structures = [
            (
                Vec3::new(-79.549_515, 0.0, -79.549_515),
                StructureKind::BaseTower,
            ),
            (
                Vec3::new(79.549_515, 0.0, 79.549_515),
                StructureKind::BaseTower,
            ),
            (
                Vec3::new(-96.549_515, 0.0, 29.509_907),
                StructureKind::Tower,
            ),
            (Vec3::new(4.490_105, 0.0, 96.549_515), StructureKind::Tower),
            (
                Vec3::new(-31.819_803, 0.0, -31.819_803),
                StructureKind::Tower,
            ),
            (Vec3::new(31.819_803, 0.0, 31.819_803), StructureKind::Tower),
            (
                Vec3::new(-4.490_089, 0.0, -96.549_515),
                StructureKind::Tower,
            ),
            (
                Vec3::new(96.549_515, 0.0, -29.509_897),
                StructureKind::Tower,
            ),
        ];
        for side in [-1.0, 1.0] {
            let start = Vec3::new(side * 74.599_77, 0.5, side * 74.599_77);
            let destination = Vec3::new(side * 84.499_26, 0.5, side * 84.499_31);
            let mut baseline_length: Option<f32> = None;
            for translation in [
                Vec3::ZERO,
                Vec3::new(-side * 79.549_515, 0.0, -side * 79.549_515),
                Vec3::new(150.25, 0.0, -150.25),
            ] {
                let translated_layout = MapLayout {
                    min: layout.min + translation.xz(),
                    max: layout.max + translation.xz(),
                    ..default()
                };
                let translated_structures: Vec<_> = structures
                    .iter()
                    .map(|&(position, kind)| (position + translation, kind))
                    .collect();
                let start = start + translation;
                let destination = destination + translation;
                let route = plan_route(
                    &translated_layout,
                    start,
                    destination,
                    &translated_structures,
                )
                .expect("base crossing must have a route");
                assert_safe(&translated_layout, start, &route, &translated_structures);
                assert_eq!(route.last(), Some(&destination));
                let mut previous = start;
                let length: f32 = route
                    .iter()
                    .map(|&point| {
                        let length = previous.distance(point);
                        previous = point;
                        length
                    })
                    .sum();
                // Two 7 m approaches around a 3.85 m padded disc have an
                // approximately 16.2 m shortest path; 20 m allows polygonal
                // approximation while rejecting a cross-map excursion.
                assert!(length < 20.0, "base route is {length} m: {route:?}");
                if let Some(baseline_length) = baseline_length {
                    assert!((length - baseline_length).abs() < 0.01);
                } else {
                    baseline_length = Some(length);
                }
            }
        }
    }

    #[test]
    fn multiple_obstacles_are_deterministic_and_traversable_without_oscillation() {
        let layout = arena(20.0);
        let mut structures = vec![
            (Vec3::new(-5.0, 0.0, 0.0), StructureKind::Tower),
            (Vec3::new(0.0, 0.0, -2.0), StructureKind::BaseTower),
            (Vec3::new(5.0, 0.0, 1.0), StructureKind::Tower),
        ];
        let start = Vec3::new(-15.0, 0.5, 0.0);
        let destination = Vec3::new(15.0, 0.5, 0.0);
        let route = plan_route(&layout, start, destination, &structures).unwrap();
        assert_safe(&layout, start, &route, &structures);
        structures.reverse();
        assert_eq!(
            plan_route(&layout, start, destination, &structures).unwrap(),
            route
        );
        let mut position = start;
        let mut steps = 0;
        for target in route {
            while position.distance(target) > 0.0001 {
                let before = position.distance(target);
                position = position.move_towards(target, 5.0 / 60.0);
                assert!(position.distance(target) < before);
                steps += 1;
                assert!(
                    steps < 1000,
                    "route failed to complete at normal movement speed"
                );
            }
        }
        assert!(position.distance(destination) < 0.0001);
    }

    #[test]
    fn obstacle_near_boundary_takes_the_open_side() {
        let layout = arena(10.0);
        let structures = [(Vec3::new(0.0, 0.0, 9.0), StructureKind::BaseTower)];
        let start = Vec3::new(-8.0, 0.5, 9.0);
        let destination = Vec3::new(8.0, 0.5, 9.0);
        let route = plan_route(&layout, start, destination, &structures).unwrap();
        assert!(route.iter().any(|point| point.z < 6.0));
        assert_safe(&layout, start, &route, &structures);
    }

    #[test]
    fn blocked_destination_projects_to_near_reachable_boundary() {
        let layout = arena(10.0);
        let structures = [(Vec3::ZERO, StructureKind::Tower)];
        let start = Vec3::new(-8.0, 0.5, 0.0);
        let route = plan_route(&layout, start, Vec3::ZERO, &structures).unwrap();
        let end = *route.last().unwrap();
        assert!(end.x < 0.0);
        assert!(end.z.abs() < 0.01);
        assert!((end.x.abs() - (1.8 + CLEARANCE)).abs() < 0.01);
        assert_safe(&layout, start, &route, &structures);
    }

    #[test]
    fn overlapping_discs_and_boundary_cannot_receive_an_unsafe_destination() {
        let layout = arena(10.0);
        let structures = [
            (Vec3::new(9.0, 0.0, 0.0), StructureKind::BaseTower),
            (Vec3::new(7.0, 0.0, 0.0), StructureKind::Tower),
        ];
        let start = Vec3::new(-8.0, 0.5, 0.0);
        let route = plan_route(&layout, start, Vec3::new(10.0, 0.0, 0.0), &structures).unwrap();
        assert_safe(&layout, start, &route, &structures);
        assert!(!route.is_empty());
    }

    #[test]
    fn overlapped_start_escapes_outward_before_detouring() {
        let layout = arena(10.0);
        let structures = [(Vec3::ZERO, StructureKind::Tower)];
        let start = Vec3::new(1.0, 0.5, 0.0);
        let destination = Vec3::new(-8.0, 0.5, 0.0);
        let route = plan_route(&layout, start, destination, &structures).unwrap();
        assert!(route[0].x >= start.x);
        assert!(route[0].xz().length() >= 1.8 + CLEARANCE);
        assert_safe(&layout, route[0], &route[1..], &structures);
        assert_eq!(route.last(), Some(&destination));
    }

    #[test]
    fn wall_of_discs_and_fully_covered_map_return_no_route() {
        let layout = arena(5.0);
        let wall: Vec<_> = [-4.0, -2.0, 0.0, 2.0, 4.0]
            .into_iter()
            .map(|z| (Vec3::new(0.0, 0.0, z), StructureKind::Tower))
            .collect();
        assert!(plan_route(&layout, Vec3::new(-4.0, 0.5, 0.0), Vec3::X * 4.0, &wall).is_none());
        assert!(
            plan_route(
                &arena(1.0),
                Vec3::ZERO,
                Vec3::X,
                &[(Vec3::ZERO, StructureKind::BaseTower)],
            )
            .is_none()
        );
    }

    #[test]
    fn invalid_inputs_and_excessive_obstacle_count_fail_closed() {
        let layout = arena(10.0);
        assert!(plan_route(&layout, Vec3::NAN, Vec3::X, &[]).is_none());
        assert!(plan_route(&layout, Vec3::ZERO, Vec3::INFINITY, &[]).is_none());
        assert!(plan_route(&arena(0.0), Vec3::ZERO, Vec3::X, &[]).is_none());
        assert!(plan_route(&layout, Vec3::X * 20.0, Vec3::ZERO, &[]).is_none());
        assert!(
            plan_route(
                &layout,
                Vec3::ZERO,
                Vec3::X,
                &[(Vec3::NAN, StructureKind::Tower)],
            )
            .is_none()
        );
        let excessive = vec![(Vec3::ZERO, StructureKind::Tower); MAX_OBSTACLES + 1];
        assert!(plan_route(&layout, Vec3::ZERO, Vec3::X, &excessive).is_none());
    }

    /// The disc a ground move is clipped against for a structure at `center`.
    fn structure(center: Vec2, kind: StructureKind) -> Disc {
        Disc {
            center: center.to_array(),
            radius: structure_collision_radius(kind) - HERO_RADIUS,
        }
    }

    #[test]
    fn a_blink_lands_outside_structure_discs_and_a_hero_radius_from_pillars() {
        let map = world_navigation();
        let middle = Vec2::ZERO;
        let way = Vec2::new(0.6, 0.8);
        assert!((0..=8).all(|step| map.point_clear((middle + way * step as f32).to_array())));
        assert!(blink_point_legal(middle, &[], &[]));

        // The server keeps a landing out of the disc of a structure: 1.3 for a tower and
        // 3.2 for a base, not the 1.8 and 3.7 a walking hero is held at.
        for (kind, disc) in [(StructureKind::Tower, 1.3), (StructureKind::BaseTower, 3.2)] {
            let solid = [structure(middle, kind)];
            assert!(!blink_point_legal(
                middle + way * (disc - 0.01),
                &solid,
                &[]
            ));
            assert!(blink_point_legal(middle + way * (disc + 0.01), &solid, &[]));
            assert!(!blink_point_legal(middle, &solid, &[]));
            // A structure that fell is not in the list any more.
            assert!(blink_point_legal(middle + way * (disc - 0.01), &[], &[]));
        }

        // An armed pillar is a disc of radius 1.0 and keeps a hero's radius more.
        let pillar = [Disc {
            center: middle.to_array(),
            radius: 1.0,
        }];
        assert!(!blink_point_legal(middle + way * 1.4, &[], &pillar));
        assert!(blink_point_legal(middle + way * 1.6, &[], &pillar));
        // Every solid has to allow the landing.
        let tower = [structure(middle + way * 3.0, StructureKind::Tower)];
        assert!(blink_point_legal(middle + way * 1.6, &tower, &pillar));
        assert!(!blink_point_legal(middle + way * 1.75, &tower, &pillar));
        assert!(blink_point_legal(middle + way * 4.35, &tower, &pillar));

        // The static map: a point in the forest, one off the map and no point at all.
        let forest = (1..80)
            .map(|step| Vec2::X * step as f32)
            .find(|at| !map.point_clear(at.to_array()))
            .expect("the forest is somewhere along +X");
        assert!(!blink_point_legal(forest, &[], &[]));
        assert!(!blink_point_legal(Vec2::splat(10_000.0), &[], &[]));
        assert!(!blink_point_legal(Vec2::new(f32::NAN, 0.0), &[], &[]));
    }

    /// A practice authority with one hero that has Fault Line on Q and Rift Step on E,
    /// alone on the real map with its towers.
    struct Authority {
        session: common::offline::PracticeSession,
        request: u64,
    }

    impl Authority {
        const FAULT_LINE: u8 = 0;
        const RIFT_STEP: u8 = 2;

        fn new() -> Self {
            use shared::loadout::{CoreId, SkillId};
            let mut session = common::offline::PracticeSession::new(std::time::Instant::now());
            session.command(shared::wire::ClientPacket::Join {
                handheld: Default::default(),
                prematch: false,
                team: shared::map::Team::Green,
                character: shared::wire::CharacterChoice::Ipfs,
                hero_class: shared::HeroClass::Riftshot,
                avatar: None,
                sprite_character: None,
                session_id: None,
                passport_ticket: None,
            });
            session.command(shared::wire::ClientPacket::Practice {
                command: shared::practice::PracticeCommand::ClearBots,
            });
            session.bots = Default::default();
            session.world.minions.clear();
            session.world.neutrals.clear();
            let mut recipe = CoreId::Riftshot.preset();
            assert_eq!(
                recipe.skills[usize::from(Self::RIFT_STEP)],
                SkillId::RiftStep
            );
            recipe.skills[usize::from(Self::FAULT_LINE)] = SkillId::FaultLine;
            session
                .world
                .players
                .get_mut(&common::offline::LOCAL_ADDR)
                .unwrap()
                .hero
                .skills
                .loadout = Some(shared::loadout::resolve(&recipe).expect("a legal mixed kit"));
            Self {
                session,
                request: 0,
            }
        }

        fn hero(&self) -> Vec2 {
            let hero = &self.session.world.players[&common::offline::LOCAL_ADDR].hero;
            Vec2::new(hero.x, hero.z)
        }

        fn stand(&mut self, at: Vec2) {
            let hero = &mut self
                .session
                .world
                .players
                .get_mut(&common::offline::LOCAL_ADDR)
                .unwrap()
                .hero;
            (hero.x, hero.z) = (at.x, at.y);
        }

        fn cast(&mut self, slot: u8, aim: Vec2) {
            self.request += 1;
            self.session.command(shared::wire::ClientPacket::CastSkill {
                slot,
                aim: aim.to_array(),
                server_epoch: common::offline::EPOCH,
                match_id: self.session.match_id,
                request_id: self.request,
            });
        }

        fn advance(&mut self, ticks: u32) {
            for _ in 0..ticks {
                self.session.advance(0.05);
            }
        }

        /// Whether a Rift Step cast from `from` puts the hero on `aim`.
        fn lands(&mut self, from: Vec2, aim: Vec2) -> bool {
            self.stand(from);
            self.cast(Self::RIFT_STEP, aim);
            self.advance(1);
            let blinked = self.hero().distance(aim) < 1e-3;
            assert!(blinked || self.hero().distance(from) < 1e-3);
            blinked
        }

        /// The standing structure of `kind` with the lowest id: its id, centre and disc.
        fn structure(&self, kind: StructureKind) -> (u64, Vec2, Disc) {
            let wire = match kind {
                StructureKind::Tower => shared::wire::StructureKind::Tower,
                StructureKind::BaseTower => shared::wire::StructureKind::BaseTower,
            };
            let (id, found) = self
                .session
                .world
                .structures
                .iter()
                .filter(|(_, structure)| structure.state.kind == wire && structure.state.hp > 0.0)
                .min_by_key(|(id, _)| **id)
                .expect("the map has a structure of each kind");
            let center = Vec2::new(found.state.x, found.state.z);
            (*id, center, structure(center, kind))
        }

        /// The armed pillars as the client derives them from the replicated effects.
        fn pillars(&mut self) -> Vec<Disc> {
            let shared::wire::ServerPacket::Snapshot { skill_effects, .. } =
                self.session.snapshot()
            else {
                panic!("practice publishes a snapshot");
            };
            skill_terrain(Some(&crate::net::GameStateSnapshot {
                skill_effects,
                ..default()
            }))
        }
    }

    /// A direction in which the static map is clear from `near` to `far` units of `center`.
    fn open_way(center: Vec2, near: f32, far: f32) -> Vec2 {
        let map = world_navigation();
        (0..16)
            .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 16.0))
            .find(|way| {
                (0..=20).all(|step| {
                    let at = center + *way * (near + (far - near) * step as f32 / 20.0);
                    map.point_clear(at.to_array())
                })
            })
            .expect("some side of the structure is open ground")
    }

    /// A step that is clearly inside or outside a boundary and far smaller than a hero.
    const MARGIN: f32 = 0.03;

    /// Parity with the in-process authority: Rift Step lands exactly where
    /// `blink_point_legal` says it may, at the edge of a tower, of a base, of a tower that
    /// fell and of an armed pillar, and in the forest.
    #[test]
    fn a_blink_is_legal_exactly_where_the_authority_lets_rift_step_land() {
        for (kind, radius) in [(StructureKind::Tower, 1.3), (StructureKind::BaseTower, 3.2)] {
            for (gap, expected) in [(-MARGIN, false), (MARGIN, true)] {
                let mut authority = Authority::new();
                let (_, center, disc) = authority.structure(kind);
                assert_eq!(disc.radius, radius);
                let way = open_way(center, radius - MARGIN, radius + 5.0);
                let aim = center + way * (radius + gap);
                let landed = authority.lands(center + way * (radius + 5.0), aim);
                assert_eq!(landed, expected, "{kind:?} {gap}");
                assert_eq!(
                    blink_point_legal(aim, &[disc], &[]),
                    landed,
                    "{kind:?} {gap}"
                );
            }
        }

        // A tower that fell blocks nothing, and the client does not list it.
        let mut authority = Authority::new();
        let (id, center, disc) = authority.structure(StructureKind::Tower);
        let way = open_way(center, 0.5, 6.0);
        let aim = center + way * 0.6;
        assert!(!blink_point_legal(aim, &[disc], &[]));
        authority
            .session
            .world
            .structures
            .get_mut(&id)
            .unwrap()
            .state
            .hp = 0.0;
        assert!(authority.lands(center + way * 6.0, aim));
        assert!(blink_point_legal(aim, &[], &[]));

        // An armed pillar, as the client reads it from the replicated effect.
        for (gap, expected) in [(-MARGIN, false), (MARGIN, true)] {
            let mut authority = Authority::new();
            let start = Vec2::ZERO;
            authority.stand(start);
            assert!(authority.pillars().is_empty());
            authority.cast(Authority::FAULT_LINE, start + Vec2::new(0.0, 10.0));
            authority.advance(30);
            let pillars = authority.pillars();
            let [pillar] = pillars.as_slice() else {
                panic!("one armed pillar: {pillars:?}");
            };
            let center = Vec2::from_array(pillar.center);
            let reach = pillar.radius + HERO_RADIUS;
            assert_eq!(reach, 1.5);
            let way = open_way(center, reach - MARGIN, reach + 5.0);
            let aim = center + way * (reach + gap);
            let landed = authority.lands(center + way * (reach + 5.0), aim);
            assert_eq!(landed, expected, "pillar {gap}");
            assert_eq!(
                blink_point_legal(aim, &[], &pillars),
                landed,
                "pillar {gap}"
            );
        }

        // The forest: the first blocked point along +X from the middle of the map.
        let map = world_navigation();
        let forest = (1..80)
            .map(|step| Vec2::X * step as f32)
            .find(|at| !map.point_clear(at.to_array()))
            .expect("the forest is somewhere along +X");
        let open = forest - Vec2::X * 1.5;
        assert!(map.point_clear(open.to_array()));
        let mut authority = Authority::new();
        assert!(!authority.lands(forest - Vec2::X * 5.0, forest));
        assert!(!blink_point_legal(forest, &[], &[]));
        let mut authority = Authority::new();
        assert!(authority.lands(forest - Vec2::X * 5.0, open));
        assert!(blink_point_legal(open, &[], &[]));
    }
}
