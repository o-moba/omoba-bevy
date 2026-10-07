use super::super::SkillPresentation;
use super::super::schema::Satellites;
use super::super::stage::{EffectKey, EffectMemory, Frame};
use super::super::vocab::ExpireKind;
use super::*;
use shared::loadout::{EffectVisualKind, SkillId};
use std::collections::BTreeSet;

use EffectVisualKind as K;

const AT: Vec2 = Vec2::new(3.5, -2.25);

fn effect(skill: SkillId, kind: EffectVisualKind) -> SkillEffectState {
    SkillEffectState {
        id: 41,
        owner_id: 7,
        owner_team: shared::map::Team::Green,
        skill,
        kind,
        position: AT.to_array(),
        // One unit ahead for a kind with a heading, the place itself for every other.
        end: if category::heading_only(kind) {
            [AT.x + 0.6, AT.y + 0.8]
        } else {
            AT.to_array()
        },
        radius: geometry::replicated_radius(skill, kind),
        remaining_secs: 2.0,
        armed: true,
        consumed_segments: 0,
    }
}

fn with(effect: &SkillEffectState, change: impl FnOnce(&mut SkillEffectState)) -> SkillEffectState {
    let mut next = effect.clone();
    change(&mut next);
    next
}

/// The instance as a client sees it for the first time.
fn first(effect: &SkillEffectState, now: f32) -> Seen<'_> {
    Seen::of(effect, None, None, 0.0, now)
}

/// Every body of the final rows with the skill and one kind it is bound to.
fn target_bodies() -> Vec<(SkillId, EffectVisualKind, Body)> {
    let registry = SkillPresentation::target();
    let mut bodies = Vec::new();
    for (id, profile) in registry.rows() {
        let Some(skill) = SkillId::from_id(id) else {
            continue;
        };
        if let Some(body) = &profile.body {
            bodies.extend(
                category::own_kinds(skill)
                    .iter()
                    .map(|kind| (skill, *kind, body.clone())),
            );
        }
        for (name, body) in &profile.aux {
            let kind = category::aux_kinds(skill)
                .iter()
                .find(|kind| category::kind_id(**kind) == name)
                .unwrap();
            bodies.push((skill, *kind, body.clone()));
        }
    }
    bodies
}

fn target_body(skill: SkillId, kind: EffectVisualKind) -> Body {
    target_bodies()
        .into_iter()
        .find(|(id, bound, _)| (*id, *bound) == (skill, kind))
        .map(|(.., body)| body)
        .unwrap()
}

fn bare(archetype: Archetype) -> Body {
    Body {
        archetype,
        core: None,
        shell: None,
        satellites: None,
        trail: Trail::None,
        fill: None,
        marker: Marker::None,
        model: None,
        altitude: None,
        expire: ExpireKind::None,
    }
}

fn part(mesh: Silhouette, size: [f32; 3], behave: Behaviour) -> Part {
    Part {
        mesh,
        slot: PaletteSlot::Primary,
        size,
        behave,
    }
}

fn mesh_of(mesh: PartMesh) -> Mesh {
    match mesh {
        PartMesh::Silhouette(mesh) => silhouette_mesh(mesh),
        PartMesh::Disc => disc_mesh(),
    }
}

fn vertices(mesh: PartMesh) -> Vec<Vec3> {
    mesh_of(mesh)
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .and_then(|values| values.as_float3())
        .unwrap()
        .iter()
        .map(|point| Vec3::from_array(*point))
        .collect()
}

/// The vertices of a visible part in the frame of its body; `None` when it is hidden.
fn drawn(slot: &PartSlot, seen: &Seen) -> Option<Vec<Vec3>> {
    let (pose, visibility) = part_pose(slot, seen);
    (visibility != Visibility::Hidden).then(|| {
        vertices(slot.mesh.unwrap())
            .into_iter()
            .map(|vertex| pose.transform_point(vertex))
            .collect()
    })
}

fn slots(body: &Body, seen: &Seen, wanted: impl Fn(Role) -> bool) -> Vec<PartSlot> {
    part_list(body, &seen.shape)
        .into_iter()
        .filter(|slot| wanted(slot.role))
        .collect()
}

fn shown(body: &Body, seen: &Seen, wanted: impl Fn(Role) -> bool) -> Vec<Transform> {
    slots(body, seen, wanted)
        .iter()
        .map(|slot| part_pose(slot, seen))
        .filter(|(_, visibility)| *visibility != Visibility::Hidden)
        .map(|(pose, _)| pose)
        .collect()
}

/// Half extents of a set of points around the origin of their frame, per axis.
fn reach(points: &[Vec3]) -> Vec3 {
    points
        .iter()
        .fold(Vec3::ZERO, |most, point| most.max(point.abs()))
}

#[test]
fn the_library_has_sixteen_small_meshes_inside_a_unit_cube() {
    let meshes: Vec<PartMesh> = Silhouette::ALL
        .iter()
        .map(|mesh| PartMesh::Silhouette(*mesh))
        .chain([PartMesh::Disc])
        .collect();
    assert_eq!(meshes.len(), 16);
    for mesh in meshes {
        let built = mesh_of(mesh);
        let points = vertices(mesh);
        let triangles = built
            .indices()
            .map_or(points.len() / 3, |indices| indices.len() / 3);
        assert!((1..=200).contains(&triangles), "{mesh:?}: {triangles}");
        let most = reach(&points);
        assert!(most.max_element() <= UNIT_RADIUS + 1e-5, "{mesh:?}: {most}");
        // A flat mesh lies in its own XY plane; only the torus has a tube.
        if planar(mesh) {
            let thick = if mesh == PartMesh::Silhouette(Silhouette::Torus) {
                TORUS_TUBE * 0.5
            } else {
                0.0
            };
            assert!((most.z - thick).abs() < 1e-5, "{mesh:?}: {}", most.z);
            // It reaches the rim of the unit circle, so a size is a real extent.
            let rim = points
                .iter()
                .map(|point| point.xy().length())
                .fold(0.0, f32::max);
            assert!((rim - UNIT_RADIUS).abs() < 2e-2, "{mesh:?}: {rim}");
        }
    }
    // The boundary ring and the fill disc end exactly on the unit circle.
    for mesh in [PartMesh::Silhouette(Silhouette::Ring), PartMesh::Disc] {
        let rim = vertices(mesh)
            .iter()
            .map(|point| point.xy().length())
            .fold(0.0, f32::max);
        assert!((rim - UNIT_RADIUS).abs() < 1e-6, "{mesh:?}");
    }
    // The flat silhouettes a particle can have are the particle meshes, point for point.
    let mut shared = 0;
    for mesh in Silhouette::ALL {
        let Some(shape) = flat_shape(*mesh) else {
            assert!(
                !planar(PartMesh::Silhouette(*mesh))
                    || matches!(mesh, Silhouette::Ring | Silhouette::Torus),
                "{mesh:?}"
            );
            continue;
        };
        shared += 1;
        assert_eq!(shape.id(), mesh.id());
        let particle = crate::game_vfx::shape_mesh(shape);
        assert_eq!(
            particle
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|values| values.as_float3()),
            silhouette_mesh(*mesh)
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|values| values.as_float3()),
            "{mesh:?}"
        );
    }
    assert_eq!(shared, 9);
    // The handles follow the order of the vocabulary.
    let mut assets = Assets::<Mesh>::default();
    let library = VfxMeshes::new(&mut assets);
    let handles: BTreeSet<_> = Silhouette::ALL
        .iter()
        .map(|mesh| library.handle(PartMesh::Silhouette(*mesh)).id())
        .chain([library.handle(PartMesh::Disc).id()])
        .collect();
    assert_eq!(handles.len(), 16);
    assert_eq!(assets.len(), 16);
    assert_eq!(
        library.particle(ParticleShape::Kite),
        Some(library.handle(PartMesh::Silhouette(Silhouette::Kite)))
    );
    assert_eq!(library.particle(ParticleShape::Glow), None);
}

#[test]
fn the_archetype_table_counts_the_engine_parts_of_every_shape() {
    let ring = GeoShape::Ring {
        center: AT,
        radius: 2.0,
    };
    let capsule = GeoShape::Capsule {
        from: AT,
        to: AT + Vec2::X,
        radius: 1.0,
    };
    let lane = GeoShape::Lane {
        from: AT,
        to: AT + Vec2::X,
        half_width: 1.0,
    };
    let sector = GeoShape::Sector {
        apex: AT,
        axis: Vec2::X,
        radius: 7.0,
        half_angle: 0.9,
    };
    let segment = GeoShape::Segment {
        from: AT,
        to: AT + Vec2::X,
    };
    let pentagon = GeoShape::Pentagon {
        center: AT,
        radius: 5.0,
    };
    for (archetype, shape, parts) in [
        (Archetype::Traveller, ring, 1),
        (Archetype::Orbiter, ring, 1),
        (Archetype::Zone, ring, 1),
        (Archetype::Prop, ring, 1),
        (Archetype::Lane, capsule, 4),
        (Archetype::Lane, lane, 2),
        (Archetype::Sector, sector, 8),
        // Rule F: a fog-cut cone is one plain segment.
        (Archetype::Sector, segment, 1),
        (Archetype::Wall, segment, 1),
        (Archetype::Cage, pentagon, 10),
    ] {
        assert_eq!(engine_parts(archetype, &shape), parts, "{archetype:?}");
    }
    // Rule E-10: a kind without a boundary gets no engine part, whatever draws it.
    for archetype in Archetype::ALL {
        assert_eq!(engine_parts(*archetype, &GeoShape::None), 0);
    }
    let drawn_here: Vec<_> = Archetype::ALL
        .iter()
        .filter(|archetype| staged(**archetype))
        .map(|archetype| archetype.id())
        .collect();
    assert_eq!(drawn_here, ["traveller", "orbiter", "zone", "prop"]);
    // The defaults a row gets when it names neither a height band nor a fill.
    for archetype in Archetype::ALL {
        let body = bare(*archetype);
        assert_eq!(
            altitude(&body) == Altitude::Chest,
            moving(*archetype),
            "{archetype:?}"
        );
        assert_eq!(
            fills(&body),
            matches!(
                archetype,
                Archetype::Zone | Archetype::Lane | Archetype::Sector
            )
        );
    }
    let mut flying = bare(Archetype::Traveller);
    flying.fill = Some(true);
    assert!(!fills(&flying), "a body in flight has no interior");
    assert_eq!(burst_lift(&flying), CHEST);
    flying.altitude = Some(Altitude::High);
    assert_eq!(burst_lift(&flying), HIGH);
    assert_eq!(burst_lift(&bare(Archetype::Prop)), 0.0);
}

#[test]
fn every_target_body_lists_the_parts_its_row_counts() {
    let bodies = target_bodies();
    // 26 rows with a body (two kinds for the ray) and six auxiliary bodies.
    assert_eq!(bodies.len(), 33);
    let mut staged_bodies = 0;
    for (skill, kind, body) in bodies {
        let e = effect(skill, kind);
        let seen = first(&e, 0.0);
        let parts = part_list(&body, &seen.shape);
        if !staged(body.archetype) {
            assert!(parts.is_empty(), "{}", skill.id());
            continue;
        }
        staged_bodies += 1;
        assert_eq!(
            parts.len(),
            part_total(&body, &seen.shape),
            "{} {kind:?}",
            skill.id()
        );
        let count = |wanted: fn(Role) -> bool| parts.iter().filter(|p| wanted(p.role)).count();
        assert_eq!(
            count(|role| matches!(role, Role::Boundary(_))),
            usize::from(engine_parts(body.archetype, &seen.shape))
        );
        // Rule E-10: the soul has no ground ring; every other staged body has one.
        assert_eq!(
            count(|role| matches!(role, Role::Boundary(_))),
            usize::from(kind != K::Soul),
            "{}",
            skill.id()
        );
        assert_eq!(
            count(|role| role == Role::Fill),
            usize::from(body.fill == Some(true))
        );
        assert_eq!(
            count(|role| role == Role::Model),
            usize::from(body.model.is_some())
        );
        assert_eq!(
            count(|role| matches!(role, Role::Satellite(_))),
            body.satellites.as_ref().map_or(0, |s| usize::from(s.count))
        );
        // Only the model has no mesh of the library.
        for slot in &parts {
            assert_eq!(slot.mesh.is_none(), slot.role == Role::Model);
        }
    }
    assert_eq!(staged_bodies, 26);
    // Spot checks against the designs' own accounts.
    let total = |skill, kind| {
        let e = effect(skill, kind);
        part_total(&target_body(skill, kind), &first(&e, 0.0).shape)
    };
    assert_eq!(total(SkillId::DawnBind, K::Bolt), 5);
    assert_eq!(total(SkillId::DawnField, K::Field), 12);
    assert_eq!(total(SkillId::WildTraps, K::Trap), 11);
    assert_eq!(total(SkillId::IronHook, K::Bolt), 10);
    assert_eq!(total(SkillId::IronHook, K::Soul), 1);
    assert_eq!(total(SkillId::OrbitalCommand, K::Orb), 9);
    assert_eq!(total(SkillId::GuidingLantern, K::Lantern), 7);
    assert_eq!(total(SkillId::IronBoundary, K::Cage), 16);
    assert_eq!(total(SkillId::WinterDivide, K::BeamWarning), 15);
}

/// AC "drawn boundaries equal replicated geometry" for the four archetypes laid out here:
/// the engine ring ends exactly at the received radius around the received position, for
/// every radius, and nothing a row authors or the client observed can move or scale it.
#[test]
fn boundary_equals_replicated_geometry() {
    let mut archetypes = BTreeSet::new();
    let mut checked = 0;
    for (skill, kind, body) in target_bodies() {
        if !staged(body.archetype) || kind == K::Soul {
            continue;
        }
        archetypes.insert(body.archetype);
        for radius in [0.05, 0.35, 0.8, 2.0, 5.0, 12.5] {
            for ground in [0.0, 1.75] {
                let e = with(&effect(skill, kind), |e| e.radius = radius);
                let seen = Seen::of(&e, None, None, ground, 7.3);
                // The root stands on the replicated position.
                let root = root_pose(&seen);
                assert_eq!(root.translation, Vec3::new(AT.x, ground, AT.y));
                let boundary = slots(&body, &seen, |role| matches!(role, Role::Boundary(_)));
                assert_eq!(boundary.len(), 1, "{}", skill.id());
                assert_eq!(part_paint(&boundary[0], &seen), Some(Paint::Team));
                let rim: Vec<f32> = drawn(&boundary[0], &seen)
                    .unwrap()
                    .into_iter()
                    .map(|vertex| root.transform_point(vertex))
                    .inspect(|point| assert!((point.y - ground - BOUNDARY_LIFT).abs() < 1e-4))
                    .map(|point| point.xz().distance(AT))
                    .collect();
                let outer = rim.iter().copied().fold(0.0, f32::max);
                let inner = rim.iter().copied().fold(f32::MAX, f32::min);
                assert!(
                    (outer - radius).abs() <= radius * 1e-5,
                    "{} at {radius}: the ring ends at {outer}",
                    skill.id()
                );
                assert!(inner >= radius * 0.9, "the boundary is a line, not an area");

                // Authored sizes, the layout, the trail, the marker and the model do not
                // reach it, and neither do the clock or what was observed of the instance.
                let pose = part_pose(&boundary[0], &seen).0;
                let mut altered = body.clone();
                for part in [&mut altered.core, &mut altered.shell]
                    .into_iter()
                    .flatten()
                {
                    part.size = part.size.map(|extent| extent * 0.37);
                    part.behave = Behaviour::Flicker;
                }
                altered.satellites = None;
                altered.model = None;
                altered.marker = Marker::None;
                altered.trail = Trail::None;
                let history = [AT + Vec2::new(-1.0, 0.5), AT + Vec2::new(-2.0, 1.0)];
                let later = Seen {
                    history: &history,
                    renewed: true,
                    resting: true,
                    owner: Some(Vec3::new(9.0, 1.0, 9.0)),
                    peak_remaining_secs: 9.0,
                    now: 91.7,
                    ..seen
                };
                let same = slots(&altered, &later, |role| matches!(role, Role::Boundary(_)));
                assert_eq!(part_pose(&same[0], &later).0, pose, "{}", skill.id());
                checked += 1;
            }
        }
    }
    assert_eq!(
        archetypes.into_iter().collect::<Vec<_>>(),
        [
            Archetype::Traveller,
            Archetype::Orbiter,
            Archetype::Zone,
            Archetype::Prop
        ]
    );
    assert_eq!(checked, 25 * 12);
    // A radius of nothing draws no ring rather than a degenerate one.
    let e = with(&effect(SkillId::DawnField, K::Field), |e| e.radius = 0.0);
    let seen = first(&e, 0.0);
    assert!(shown(&target_body(SkillId::DawnField, K::Field), &seen, |_| true).is_empty());
}

/// Whether every vertex of every visible authored part, fill and marker lies inside the
/// boundary circle, and the packaged prop with it.
fn inside_the_boundary(body: &Body, seen: &Seen, what: &str) {
    let radius = seen.effect.radius;
    for slot in part_list(body, &seen.shape) {
        if matches!(slot.role, Role::Boundary(_)) {
            continue;
        }
        if slot.role == Role::Model {
            let (pose, _) = part_pose(&slot, seen);
            let (scale, _) = model_fit(body.model.unwrap(), &slot, radius);
            assert_eq!(pose.scale, Vec3::splat(scale));
            let half_width = match body.model.unwrap() {
                Model::Trap => 0.46,
                Model::Lantern => 0.42,
                other => panic!("{other:?} is no standing prop"),
            };
            assert!(half_width * scale <= radius + 1e-4, "{what}: the prop");
            continue;
        }
        for vertex in drawn(&slot, seen).unwrap_or_default() {
            let from_centre = vertex.xz().length();
            assert!(
                from_centre <= radius * (1.0 + 1e-3) + 1e-4,
                "{what}: {:?} reaches {from_centre} of {radius} at {}",
                slot.role,
                seen.now
            );
        }
    }
}

/// States an instance of an area body goes through, with the clock running.
fn area_states(e: &SkillEffectState) -> Vec<(SkillEffectState, f32)> {
    let mut states = Vec::new();
    for step in 0..24 {
        let now = step as f32 * 0.173;
        for (armed, remaining) in [(false, 2.0), (true, 2.0), (true, 0.45), (true, 0.05)] {
            states.push((
                with(e, |e| {
                    e.armed = armed;
                    e.remaining_secs = remaining;
                }),
                now,
            ));
        }
    }
    states
}

#[test]
fn no_authored_part_leaves_the_boundary_of_a_zone_or_a_prop() {
    // The final rows, over radii a server could send.
    let mut rows = 0;
    for (skill, kind, body) in target_bodies() {
        if !matches!(body.archetype, Archetype::Zone | Archetype::Prop) {
            continue;
        }
        rows += 1;
        for radius in [0.3, 1.0, 2.5, 5.0, 9.0] {
            let e = with(&effect(skill, kind), |e| e.radius = radius);
            for (state, now) in area_states(&e) {
                let seen = Seen {
                    peak_remaining_secs: 5.0,
                    ..first(&state, now)
                };
                inside_the_boundary(&body, &seen, skill.id());
            }
        }
    }
    assert_eq!(rows, 9);

    // Everything a row could author: every silhouette at the largest sizes the parser
    // lets through, with every behaviour, as core, shell and in every layout of an area.
    let sizes = [
        [1.0, 3.0, 1.0],
        [1.0, 1.0, 0.02],
        [0.3, 0.05, 1.0],
        [0.14, 0.5, 0.14],
        [1.0, 1.0, 1.0],
    ];
    for archetype in [Archetype::Zone, Archetype::Prop] {
        let (skill, kind) = match archetype {
            Archetype::Zone => (SkillId::DawnField, K::Field),
            _ => (SkillId::WildTraps, K::Trap),
        };
        for mesh in Silhouette::ALL {
            for behave in Behaviour::ALL {
                // The orb's ring and the renewal mark belong to bodies in flight.
                if matches!(behave, Behaviour::Gyro | Behaviour::OnlyAfterRenew) {
                    continue;
                }
                for size in sizes {
                    // A part that turns end over end may not be taller than the radius.
                    if *behave == Behaviour::Tumble && size[1] > 1.0 {
                        continue;
                    }
                    let mut body = bare(archetype);
                    body.fill = Some(true);
                    body.marker = Marker::RemainingRing;
                    body.core = Some(part(*mesh, size, *behave));
                    body.shell = Some(part(*mesh, size, *behave));
                    for layout in [
                        SatelliteLayout::Orbit,
                        SatelliteLayout::Column,
                        SatelliteLayout::QuadX,
                        SatelliteLayout::Rim,
                    ] {
                        for count in [1, 5, 8] {
                            body.satellites = Some(Satellites {
                                mesh: *mesh,
                                layout,
                                count,
                                size,
                                slot: PaletteSlot::Accent,
                                behave: *behave,
                            });
                            for radius in [0.4, 3.0] {
                                let e = with(&effect(skill, kind), |e| e.radius = radius);
                                for now in [0.0, 0.31, 1.7, 4.4] {
                                    for armed in [false, true] {
                                        let state = with(&e, |e| e.armed = armed);
                                        inside_the_boundary(
                                            &body,
                                            &first(&state, now),
                                            &format!("{mesh:?} {behave:?} {layout:?}"),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // A prop wider than a small boundary is scaled down to it.
    let mut lantern = bare(Archetype::Zone);
    lantern.model = Some(Model::Lantern);
    let small = with(&effect(SkillId::GuidingLantern, K::Lantern), |e| {
        e.radius = 0.2
    });
    inside_the_boundary(&lantern, &first(&small, 0.0), "a small lantern");
}

/// A part is never larger than authored: its vertices stay inside the box of its sizes
/// around its centre, whatever it is made of and however it is laid.
#[test]
fn a_part_never_exceeds_its_authored_size() {
    let e = effect(SkillId::WinterShard, K::Bolt);
    for mesh in Silhouette::ALL {
        for size in [
            [0.45, 0.45, 1.6],
            [1.0, 1.7, 0.05],
            [0.5, 0.06, 0.5],
            [0.05, 0.6, 0.9],
            [0.8, 0.8, 0.8],
            [0.16, 0.16, 2.8],
            [0.85, 2.0, 0.85],
        ] {
            for behave in [
                Behaviour::Steady,
                Behaviour::Pulse,
                Behaviour::Flicker,
                Behaviour::RiseOnArm,
                Behaviour::BlinkLast,
            ] {
                for altitude in Altitude::ALL {
                    let mut body = bare(Archetype::Traveller);
                    body.altitude = Some(*altitude);
                    body.core = Some(part(*mesh, size, behave));
                    for now in [0.0, 0.4, 2.9] {
                        let seen = first(&e, now);
                        let core = &slots(&body, &seen, |role| role == Role::Core)[0];
                        let (pose, _) = part_pose(core, &seen);
                        let points: Vec<Vec3> = vertices(core.mesh.unwrap())
                            .into_iter()
                            .map(|vertex| pose.transform_point(vertex) - pose.translation)
                            .collect();
                        let half = Vec3::from_array(size) * 0.5;
                        let most = reach(&points);
                        assert!(
                            most.cmple(half + Vec3::splat(1e-4)).all(),
                            "{mesh:?} {size:?} {altitude:?} {behave:?}: {most} of {half}"
                        );
                    }
                }
            }
        }
    }
}

/// Rule E-5 and its reading of the sizes: a flat mesh faces its thinnest authored extent;
/// where the sizes do not decide it stands across the heading in the air and lies on the
/// ground; a cone points along its longest extent.
#[test]
fn flat_parts_face_their_thinnest_extent_and_e5_decides_ties() {
    let flat = |size: [f32; 3], altitude| normal_axis(Vec3::from_array(size), altitude);
    // The mirror plate, the Aegis and every part with equal sizes stand across the heading.
    assert_eq!(flat([1.0, 1.7, 0.05], Altitude::Chest), HEADING);
    assert_eq!(flat([0.85, 0.85, 0.05], Altitude::Chest), HEADING);
    assert_eq!(flat([0.5, 0.5, 0.5], Altitude::Chest), HEADING);
    assert_eq!(flat([0.72, 0.72, 0.72], Altitude::High), HEADING);
    // On the ground the same sizes lie flat.
    assert_eq!(flat([0.9, 0.9, 0.9], Altitude::Ground), VERTICAL);
    assert_eq!(flat([0.5, 0.02, 0.5], Altitude::Ground), VERTICAL);
    // A thin height lies flat in the air as well: the wave front, the needle's fin.
    assert_eq!(flat([0.5, 0.06, 0.5], Altitude::Chest), VERTICAL);
    assert_eq!(flat([0.44, 0.04, 1.4], Altitude::Chest), VERTICAL);
    // A thin depth stands on the ground: the wave's upright arc.
    assert_eq!(flat([1.8, 1.0, 0.06], Altitude::Ground), HEADING);
    // Long along the heading with equal width and height: flat, seen from above.
    assert_eq!(flat([0.45, 0.45, 1.6], Altitude::Chest), VERTICAL);
    // Tall with equal width and depth: upright, across the heading, also on the ground.
    assert_eq!(flat([0.18, 0.42, 0.18], Altitude::Chest), HEADING);
    assert_eq!(flat([0.06, 0.14, 0.06], Altitude::Ground), HEADING);
    // Thin sideways: on edge along the heading.
    assert_eq!(flat([0.05, 0.6, 0.9], Altitude::Chest), LATERAL);

    let e = effect(SkillId::WinterShard, K::Bolt);
    let seen = first(&e, 0.0);
    let laid = |mesh, size: [f32; 3], altitude| {
        let mut body = bare(Archetype::Traveller);
        body.altitude = Some(altitude);
        body.core = Some(part(mesh, size, Behaviour::Steady));
        let core = slots(&body, &seen, |role| role == Role::Core)[0];
        let pose = part_pose(&core, &seen).0;
        vertices(core.mesh.unwrap())
            .into_iter()
            .map(|vertex| pose.transform_point(vertex) - pose.translation)
            .collect::<Vec<_>>()
    };
    // The kite stands with its flat top up and its point down, 1.0 wide and 1.7 tall.
    let kite = laid(Silhouette::Kite, [1.0, 1.7, 0.05], Altitude::Chest);
    assert!(reach(&kite).z < 1e-5);
    let top = kite.iter().map(|p| p.y).fold(f32::MIN, f32::max);
    let bottom = kite
        .iter()
        .copied()
        .min_by(|a, b| a.y.total_cmp(&b.y))
        .unwrap();
    assert!(
        (bottom.y + 0.85).abs() < 1e-4 && bottom.x.abs() < 1e-4,
        "the point is down"
    );
    assert!(top > 0.5 && kite.iter().filter(|p| (p.y - top).abs() < 0.06).count() >= 2);
    // The drop of a soul points up.
    let drop = laid(Silhouette::Drop, [0.18, 0.42, 0.18], Altitude::Chest);
    let tip = drop
        .iter()
        .copied()
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .unwrap();
    assert!((tip.y - 0.21).abs() < 1e-4 && tip.x.abs() < 1e-4);
    // The wave front lies flat with its convex side toward the heading.
    let arc = laid(Silhouette::Arc, [0.5, 0.06, 0.5], Altitude::Chest);
    assert!(reach(&arc).y < 1e-5);
    let front = arc
        .iter()
        .copied()
        .max_by(|a, b| a.z.total_cmp(&b.z))
        .unwrap();
    assert!((front.z - 0.25).abs() < 1e-4 && front.x.abs() < 1e-3);
    assert!(arc.iter().all(|p| p.z > -1e-4), "the arc opens backward");
    // The shuttle is long on the heading.
    let diamond = laid(Silhouette::Diamond, [0.45, 0.45, 1.6], Altitude::Chest);
    assert!(
        (reach(&diamond) - Vec3::new(0.135, 0.0, 0.8))
            .abs()
            .max_element()
            < 1e-4
    );
    // A cone points along its longest extent: forward as a needle, up as a spike.
    let needle = laid(Silhouette::Cone, [0.16, 0.16, 2.8], Altitude::Chest);
    let apex = needle
        .iter()
        .copied()
        .max_by(|a, b| a.z.total_cmp(&b.z))
        .unwrap();
    assert!((apex - Vec3::new(0.0, 0.0, 1.4)).length() < 1e-4);
    let spike = laid(Silhouette::Cone, [0.85, 2.8, 0.85], Altitude::Ground);
    let apex = spike
        .iter()
        .copied()
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .unwrap();
    assert!((apex - Vec3::new(0.0, 1.4, 0.0)).length() < 1e-4);
    let equal = laid(Silhouette::Cone, [0.5, 0.5, 0.5], Altitude::Chest);
    let apex = equal
        .iter()
        .copied()
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .unwrap();
    assert!((apex - Vec3::new(0.0, 0.25, 0.0)).length() < 1e-4);
    // A torus keeps its tube: the gyro ring is not a pipe, the seal is a thin washer.
    let ring = reach(&laid(Silhouette::Torus, [0.72, 0.72, 0.72], Altitude::High));
    assert!(
        (ring - Vec3::new(0.36, 0.36, 0.72 * TORUS_TUBE * 0.5))
            .abs()
            .max_element()
            < 1e-4
    );
    let seal = reach(&laid(Silhouette::Torus, [0.95, 0.08, 0.95], Altitude::High));
    assert!((seal - Vec3::new(0.475, 0.04, 0.475)).abs().max_element() < 1e-4);
}

/// A flat rim around a flat plate: the same kind of material lies just behind it; light
/// over matter shares its plane and is drawn over it from both sides.
#[test]
fn a_flat_shell_frames_its_core() {
    let e = effect(SkillId::DawnBarrier, K::Barrier);
    let seen = first(&e, 0.0);
    let plate = |core: PaletteSlot, shell: PaletteSlot| {
        let mut body = bare(Archetype::Orbiter);
        body.core = Some(Part {
            slot: core,
            ..part(Silhouette::Kite, [0.85, 0.85, 0.05], Behaviour::Steady)
        });
        body.shell = Some(Part {
            slot: shell,
            ..part(Silhouette::Kite, [1.05, 1.05, 0.05], Behaviour::Steady)
        });
        let depth = |role| shown(&body, &seen, move |r| r == role)[0].translation;
        depth(Role::Shell) - depth(Role::Core)
    };
    // The pearl plate on its rose rim: one plane.
    assert_eq!(
        plate(PaletteSlot::Primary, PaletteSlot::Secondary),
        Vec3::ZERO
    );
    assert_eq!(
        plate(PaletteSlot::Secondary, PaletteSlot::White),
        Vec3::ZERO
    );
    // Two lights, or two pieces of matter: the rim steps back along the heading.
    for (core, shell) in [
        (PaletteSlot::Primary, PaletteSlot::Accent),
        (PaletteSlot::Secondary, PaletteSlot::Secondary),
    ] {
        assert_eq!(plate(core, shell), Vec3::new(0.0, 0.0, -SHELL_NUDGE));
    }
    // The final Aegis is such a plate, upright and across the heading.
    let aegis = target_body(SkillId::DawnBarrier, K::Barrier);
    for role in [Role::Core, Role::Shell] {
        let slot = slots(&aegis, &seen, |r| r == role)[0];
        let points = drawn(&slot, &seen).unwrap();
        let spread = reach(
            &points
                .iter()
                .map(|point| *point - Vec3::Y * CHEST)
                .collect::<Vec<_>>(),
        );
        assert!(
            spread.z < 1e-5 && spread.y > 0.3 && spread.x > 0.3,
            "{role:?}"
        );
    }
}

/// The part stands on the ground at `ground` altitude and at one height in the air.
#[test]
fn altitude_places_the_body_and_ground_parts_stand_on_the_ground() {
    let e = effect(SkillId::FaultLine, K::Trap);
    let seen = first(&e, 0.0);
    let mut body = bare(Archetype::Prop);
    body.core = Some(part(
        Silhouette::Cone,
        [0.85, 2.8, 0.85],
        Behaviour::RiseOnArm,
    ));
    let core = slots(&body, &seen, |role| role == Role::Core)[0];
    let lowest = |seen: &Seen| {
        drawn(&core, seen)
            .unwrap()
            .iter()
            .map(|p| p.y)
            .fold(f32::MAX, f32::min)
    };
    let tallest = |seen: &Seen| {
        drawn(&core, seen)
            .unwrap()
            .iter()
            .map(|p| p.y)
            .fold(0.0, f32::max)
    };
    // Sizes of an area body are multiples of the replicated radius (1.0 here).
    assert!((lowest(&seen) - GROUND_LIFT).abs() < 1e-4);
    assert!((tallest(&seen) - GROUND_LIFT - 2.8).abs() < 1e-4);
    // `rise_on_arm`: flat until the replicated flag is set, and still on the ground.
    let unarmed = with(&e, |e| e.armed = false);
    let flat = first(&unarmed, 0.0);
    assert!((lowest(&flat) - GROUND_LIFT).abs() < 1e-4);
    assert!((tallest(&flat) - GROUND_LIFT - 2.8 * FLAT_SHARE).abs() < 1e-4);
    // A radius twice as large makes the part twice as large.
    let wide = with(&e, |e| e.radius = 2.0);
    assert!((tallest(&first(&wide, 0.0)) - GROUND_LIFT - 5.6).abs() < 1e-4);

    let shard = effect(SkillId::WinterShard, K::Bolt);
    let seen = first(&shard, 0.0);
    for (altitude, stands) in [(Altitude::Chest, CHEST), (Altitude::High, HIGH)] {
        let mut body = bare(Archetype::Traveller);
        body.altitude = Some(altitude);
        body.core = Some(part(Silhouette::Shard, [0.8, 0.8, 1.5], Behaviour::Steady));
        let pose = shown(&body, &seen, |role| role == Role::Core)[0];
        assert_eq!(pose.translation, Vec3::new(0.0, stands, 0.0));
        // Sizes of a body in flight are metres, whatever its radius.
        assert_eq!(pose.scale, Vec3::new(0.8, 0.8, 1.5));
    }
    // The root turns the body along the replicated heading (0.6, 0.8).
    let forward = root_pose(&seen).rotation * Vec3::Z;
    assert!((forward - Vec3::new(0.6, 0.0, 0.8)).length() < 1e-5);
    // A kind without a heading faces the side the camera looks from, whatever `end` says.
    let zone = with(&effect(SkillId::DawnField, K::Field), |e| {
        e.end = [9.0, 9.0]
    });
    let forward = root_pose(&first(&zone, 0.0)).rotation * Vec3::Z;
    assert!((forward - Vec3::NEG_X).length() < 1e-5);
}

fn trail_body(trail: Trail) -> Body {
    let mut body = bare(Archetype::Traveller);
    body.core = Some(part(Silhouette::Ball, [0.4, 0.4, 0.4], Behaviour::Steady));
    body.trail = trail;
    body
}

#[test]
fn trail_parts_lie_only_on_observed_positions() {
    let e = effect(SkillId::WanderingEmber, K::Bolt);
    let heading = Vec2::new(0.6, 0.8);
    // Six earlier sightings, one snapshot of travel apart, newest first.
    let observed: Vec<Vec2> = (1..=6)
        .map(|step| AT - heading * 1.1 * step as f32)
        .collect();
    for trail in [Trail::Ribbon, Trail::Motes, Trail::Chevrons, Trail::Links] {
        let body = trail_body(trail);
        let parts = usize::from(trail_parts(trail));
        // The first frame has none.
        let seen = first(&e, 0.3);
        assert_eq!(
            slots(&body, &seen, |role| matches!(role, Role::Trail(_))).len(),
            parts
        );
        assert!(shown(&body, &seen, |role| matches!(role, Role::Trail(_))).is_empty());
        for known in 1..=observed.len() {
            let seen = Seen {
                history: &observed[..known],
                ..first(&e, 0.3)
            };
            let root = root_pose(&seen);
            let poses = shown(&body, &seen, |role| matches!(role, Role::Trail(_)));
            // One part for each observed position, as far as the trail has parts.
            assert_eq!(poses.len(), parts.min(known), "{trail:?} with {known}");
            for (index, pose) in poses.iter().enumerate() {
                let at = root.transform_point(pose.translation);
                assert!((at.y - CHEST).abs() < 1e-4);
                if trail == Trail::Ribbon {
                    // A bar from one observed position to the one before it.
                    let newer = if index == 0 { AT } else { observed[index - 1] };
                    let older = observed[index];
                    assert!(at.xz().distance((newer + older) * 0.5) < 1e-4);
                    assert!((pose.scale.z - newer.distance(older)).abs() < 1e-4);
                    let along = root.rotation * pose.rotation * Vec3::Z;
                    assert!((along.xz() - (older - newer).normalize()).length() < 1e-4);
                } else {
                    assert!(
                        at.xz().distance(observed[index]) < 1e-4,
                        "{trail:?} part {index} is at {at}"
                    );
                }
            }
            // A body at rest shows none of them.
            let resting = Seen {
                resting: true,
                ..seen
            };
            assert!(shown(&body, &resting, |role| matches!(role, Role::Trail(_))).is_empty());
        }
    }
    assert_eq!(
        shown(&trail_body(Trail::None), &first(&e, 0.0), |_| true).len(),
        2
    );

    // Through the tracker: a turn and a relocation leave no trail behind them.
    let registry = SkillPresentation::target();
    let mut memory = EffectMemory::default();
    let mut now = 10.0;
    let mut take = |memory: &mut EffectMemory, effect: &SkillEffectState| {
        now += 0.05;
        memory.take(&Frame {
            round: Some((1, 1)),
            now,
            running: true,
            effects: std::slice::from_ref(effect),
            heroes: &[],
            casts: &[],
            registry: Some(&registry),
        });
    };
    let key = EffectKey::Runtime(41);
    let body = trail_body(Trail::Motes);
    let visible = |memory: &EffectMemory, effect: &SkillEffectState| {
        let seen = Seen::of(effect, memory.get(key), None, 0.0, 0.0);
        shown(&body, &seen, |role| matches!(role, Role::Trail(_))).len()
    };
    let flying = |step: f32| {
        with(&e, |e| {
            let at = AT + heading * step;
            e.position = at.to_array();
            e.end = (at + heading).to_array();
        })
    };
    take(&mut memory, &flying(0.0));
    assert_eq!(visible(&memory, &flying(0.0)), 0);
    for step in 1..=4 {
        take(&mut memory, &flying(step as f32));
        assert_eq!(visible(&memory, &flying(step as f32)), step.min(3));
    }
    // One repeated position is no rest. After three it rests: no trail. It moves on: the
    // trail is back.
    take(&mut memory, &flying(4.0));
    assert_eq!(visible(&memory, &flying(4.0)), 3);
    take(&mut memory, &flying(4.0));
    take(&mut memory, &flying(4.0));
    assert_eq!(visible(&memory, &flying(4.0)), 0);
    take(&mut memory, &flying(5.0));
    assert_eq!(visible(&memory, &flying(5.0)), 3);
    // It turns back.
    let back = with(&flying(4.5), |e| {
        e.end = (Vec2::from_array(e.position) - heading).to_array();
    });
    take(&mut memory, &back);
    assert_eq!(visible(&memory, &back), 0);
    // It is relocated farther than a body travels in one snapshot.
    let home = with(&back, |e| {
        let at = Vec2::from_array(e.position) - heading;
        e.position = at.to_array();
        e.end = (at - heading).to_array();
    });
    take(&mut memory, &home);
    assert_eq!(visible(&memory, &home), 1);
    let far = with(&home, |e| {
        let at = Vec2::from_array(e.position) - heading * (stage::JUMP_UNITS + 0.5);
        e.position = at.to_array();
        e.end = (at - heading).to_array();
    });
    take(&mut memory, &far);
    assert_eq!(visible(&memory, &far), 0);
}

/// A positional key can pass from one soul to the next, so what it remembers is not the
/// path of one object. Only the orb, of which a hero has one, is followed by its key.
#[test]
fn only_the_orb_is_followed_among_the_auxiliary_objects() {
    let registry = SkillPresentation::target();
    let seen_after_a_flight = |skill: SkillId, kind: EffectVisualKind| {
        let mut memory = EffectMemory::default();
        let e = effect(skill, kind);
        let key = stage::keyed(std::slice::from_ref(&e))[0].0;
        let mut last = e.clone();
        for step in 0..4 {
            last = with(&e, |e| {
                e.position = [AT.x + step as f32, AT.y];
                e.end = e.position;
            });
            memory.take(&Frame {
                round: Some((1, 1)),
                now: 5.0 + step as f64 * 0.05,
                running: true,
                effects: std::slice::from_ref(&last),
                heroes: &[],
                casts: &[],
                registry: Some(&registry),
            });
        }
        let held = memory.get(key).unwrap();
        assert_eq!(held.trail().len(), 3, "the tracker itself remembers");
        Seen::of(&last, Some(held), None, 0.0, 0.0).history.len()
    };
    assert_eq!(seen_after_a_flight(SkillId::OrbitalCommand, K::Orb), 3);
    assert_eq!(seen_after_a_flight(SkillId::WinterShard, K::Bolt), 3);
    for (skill, kind) in [
        (SkillId::IronHook, K::Soul),
        (SkillId::AnchorStep, K::Anchor),
        (SkillId::FourfoldDuel, K::Healing),
    ] {
        assert_eq!(seen_after_a_flight(skill, kind), 0, "{kind:?}");
    }
}

#[test]
fn only_after_renew_is_hidden_until_this_instance_renews() {
    let body = target_body(SkillId::WanderingEmber, K::Bolt);
    assert_eq!(
        body.shell.as_ref().unwrap().behave,
        Behaviour::OnlyAfterRenew
    );
    let e = effect(SkillId::WanderingEmber, K::Bolt);
    let shell = |seen: &Seen| shown(&body, seen, |role| role == Role::Shell).len();
    assert_eq!(shell(&first(&e, 0.0)), 0);
    assert_eq!(
        shell(&Seen {
            renewed: true,
            ..first(&e, 0.0)
        }),
        1
    );

    // Through the tracker: the ember flies out, turns and is renewed, and shows its flame
    // from then on. A client that first sees it on its way back never saw it renew.
    let registry = SkillPresentation::target();
    let frame = |effects: &[SkillEffectState], now: f64| {
        let mut memory = EffectMemory::default();
        let mut shells = Vec::new();
        for (step, effect) in effects.iter().enumerate() {
            memory.take(&Frame {
                round: Some((1, 1)),
                now: now + step as f64 * 0.05,
                running: true,
                effects: std::slice::from_ref(effect),
                heroes: &[],
                casts: &[],
                registry: Some(&registry),
            });
            let seen = Seen::of(effect, memory.get(EffectKey::Runtime(41)), None, 0.0, 0.0);
            shells.push(shell(&seen));
        }
        shells
    };
    let out = with(&e, |e| e.remaining_secs = 0.4);
    let returning = with(&e, |e| {
        e.end = [AT.x - 0.6, AT.y - 0.8];
        e.remaining_secs = 3.0;
    });
    let later = with(&returning, |e| e.remaining_secs = 2.95);
    assert_eq!(
        frame(&[out.clone(), out, returning.clone(), later.clone()], 5.0),
        [0, 0, 1, 1]
    );
    assert_eq!(frame(&[returning, later], 5.0), [0, 0]);
}

#[test]
fn the_part_budget_hides_authored_parts_of_the_farthest_bodies_first() {
    let load = |key: u8, distance: f32, kept, authored| Load {
        key,
        distance,
        kept,
        authored,
    };
    let loads = [
        load(1, 4.0, 2, 10),
        load(2, 30.0, 2, 10),
        load(3, 12.0, 2, 10),
        load(4, 55.0, 10, 0),
        load(5, 21.0, 1, 4),
    ];
    // 17 kept and 34 authored parts.
    assert_eq!(over_budget(&loads, 0, 51), [] as [u8; 0]);
    // One part too many: the farthest body that has authored parts gives all of its own.
    assert_eq!(over_budget(&loads, 0, 50), [2]);
    assert_eq!(over_budget(&loads, 0, 41), [2]);
    assert_eq!(over_budget(&loads, 0, 40), [2, 5]);
    assert_eq!(over_budget(&loads, 0, 37), [2, 5]);
    assert_eq!(over_budget(&loads, 0, 36), [2, 5, 3]);
    // Parts no body can give up count against the same budget.
    assert_eq!(over_budget(&loads, 1, 51), [2]);
    // Engine parts survive: when they alone are over the budget every authored part goes
    // and nothing else does.
    assert_eq!(over_budget(&loads, 0, 3), [2, 5, 3, 1]);
    assert_eq!(over_budget(&loads, 400, 3), [2, 5, 3, 1]);
    assert!(over_budget::<u8>(&[], 9, 3).is_empty());

    // The ceiling of the server: 128 zones of twelve parts each. The budget holds, every
    // boundary, fill and marker stays, and the nearest bodies keep their look.
    let body = target_body(SkillId::DawnField, K::Field);
    let e = effect(SkillId::DawnField, K::Field);
    let seen = first(&e, 0.0);
    let parts = part_list(&body, &seen.shape);
    let authored = parts.iter().filter(|slot| slot.role.authored()).count();
    let kept = parts.len() - authored;
    assert_eq!((kept, authored), (3, 9));
    let field: Vec<_> = (0..128u8)
        .map(|key| load(key, f32::from(key) * 1.5, kept, authored))
        .collect();
    let hidden = over_budget(&field, 0, PART_BUDGET);
    let visible = 128 * kept + (128 - hidden.len()) * authored;
    assert!(visible <= PART_BUDGET && visible + authored > PART_BUDGET);
    assert_eq!(hidden.len(), 127);
    assert!(!hidden.contains(&0), "the nearest body keeps its parts");
    assert_eq!(hidden[0], 127);
    // Which roles a budget may take.
    for (role, authored) in [
        (Role::Boundary(0), false),
        (Role::Fill, false),
        (Role::Marker(0), false),
        (Role::Core, true),
        (Role::Shell, true),
        (Role::Satellite(3), true),
        (Role::Trail(1), true),
        (Role::Model, true),
    ] {
        assert_eq!(role.authored(), authored, "{role:?}");
    }
}

#[test]
fn satellites_of_a_body_in_flight_stay_inside_its_radius() {
    let mut rows = 0;
    for (skill, kind, body) in target_bodies() {
        let Some(satellites) = body.satellites.as_ref().filter(|_| moving(body.archetype)) else {
            continue;
        };
        rows += 1;
        let e = effect(skill, kind);
        for step in 0..40 {
            let seen = first(&e, step as f32 * 0.071);
            let poses = shown(&body, &seen, |role| matches!(role, Role::Satellite(_)));
            assert_eq!(poses.len(), usize::from(satellites.count), "{}", skill.id());
            for pose in poses {
                let side = pose.translation.x.abs() + satellites.size[0] * 0.5;
                assert!(
                    side <= e.radius + 1e-4,
                    "{}: a copy reaches {side} of {}",
                    skill.id(),
                    e.radius
                );
                assert!(pose.translation.y > 0.0, "{}: below the ground", skill.id());
            }
        }
    }
    assert_eq!(rows, 11);
    // Layouts: copies of one ring are spread evenly and differ from one another.
    let e = effect(SkillId::RiftSeal, K::Bolt);
    let body = target_body(SkillId::RiftSeal, K::Bolt);
    let quad = shown(&body, &first(&e, 0.0), |role| {
        matches!(role, Role::Satellite(_))
    });
    assert_eq!(quad.len(), 4);
    for (index, pose) in quad.iter().enumerate() {
        let at = pose.translation.xz();
        let next = quad[(index + 1) % 4].translation.xz();
        assert!((at.length() - next.length()).abs() < 1e-5);
        assert!(at.dot(next).abs() < 1e-4, "an X has right angles");
        assert!(
            (at.x.abs() - at.y.abs()).abs() < 1e-4,
            "an X stands on the diagonals"
        );
    }
    // A strip layout has no place on a circle.
    let mut stray = bare(Archetype::Zone);
    stray.satellites = Some(Satellites {
        mesh: Silhouette::Block,
        layout: SatelliteLayout::Stagger,
        count: 4,
        size: [0.1, 0.1, 0.1],
        slot: PaletteSlot::Primary,
        behave: Behaviour::Steady,
    });
    let zone = effect(SkillId::DawnField, K::Field);
    assert!(
        shown(&stray, &first(&zone, 0.0), |role| matches!(
            role,
            Role::Satellite(_)
        ))
        .is_empty()
    );
}

#[test]
fn markers_read_replicated_state() {
    // The remaining ring shrinks from the boundary with the share of the longest
    // remaining time the client saw of this instance.
    let field = target_body(SkillId::DawnField, K::Field);
    assert_eq!(field.marker, Marker::RemainingRing);
    let e = effect(SkillId::DawnField, K::Field);
    let ring = |remaining: f32, peak: f32| {
        let state = with(&e, |e| e.remaining_secs = remaining);
        let seen = Seen {
            peak_remaining_secs: peak,
            ..first(&state, 0.0)
        };
        shown(&field, &seen, |role| matches!(role, Role::Marker(_)))
            .first()
            .map(|pose| pose.scale.x * UNIT_RADIUS)
    };
    let near =
        |ring: Option<f32>, radius: f32| ring.is_some_and(|ring| (ring - radius).abs() < 1e-5);
    // The ring ends at the inner edge of the boundary line, so the team colour stays seen.
    let reach = 3.0 * MARKER_REACH;
    assert!(near(ring(5.0, 5.0), reach));
    assert!(near(ring(2.5, 5.0), reach * 0.5));
    assert!(near(ring(0.5, 5.0), reach * 0.1));
    assert_eq!(ring(0.0, 5.0), None);
    // First seen in mid-life: full at that moment, and gone when the time is.
    assert!(near(ring(2.0, 2.0), reach));

    // The fill of a fuse reaches the boundary exactly when the server fires.
    let collapse = target_body(SkillId::OrbitalCollapse, K::BeamWarning);
    assert_eq!(collapse.marker, Marker::FillToEdge);
    let e = effect(SkillId::OrbitalCollapse, K::BeamWarning);
    let (telegraph, tail) = category::telegraph_secs(e.skill)
        .zip(category::tail_secs(e.skill))
        .unwrap();
    let grown = |remaining: f32| {
        let state = with(&e, |e| e.remaining_secs = remaining);
        shown(&collapse, &first(&state, 0.0), |role| {
            matches!(role, Role::Marker(_))
        })
        .first()
        .map(|pose| pose.scale.x * UNIT_RADIUS)
    };
    let edge = e.radius * MARKER_REACH;
    assert_eq!(grown(tail + telegraph), None);
    assert!((grown(tail + telegraph * 0.5).unwrap() - edge * 0.5).abs() < 1e-4);
    assert!(near(grown(tail), edge));
    assert!(near(grown(0.0), edge));

    // The pips of a trap are dark and small until the replicated flag is set.
    let trap = target_body(SkillId::WildTraps, K::Trap);
    assert_eq!(trap.marker, Marker::ArmingPips);
    let armed = effect(SkillId::WildTraps, K::Trap);
    let unarmed = with(&armed, |e| e.armed = false);
    let pips = |e: &SkillEffectState| {
        let seen = first(e, 0.0);
        let slots = slots(&trap, &seen, |role| matches!(role, Role::Marker(_)));
        assert_eq!(slots.len(), 3);
        (
            part_paint(&slots[0], &seen),
            part_pose(&slots[0], &seen).0.scale.x,
        )
    };
    let (dark, small) = pips(&unarmed);
    let (lit, large) = pips(&armed);
    assert_eq!(dark, Some(Paint::Slot(PaletteSlot::Secondary)));
    assert_eq!(lit, Some(Paint::Slot(PaletteSlot::Primary)));
    assert!(small < large);
    // The fill is the dim one while the effect is a telegraph.
    let fill = |body: &Body, e: &SkillEffectState| {
        let seen = first(e, 0.0);
        part_paint(&slots(body, &seen, |role| role == Role::Fill)[0], &seen)
    };
    assert_eq!(fill(&trap, &unarmed), Some(Paint::FillDim));
    assert_eq!(fill(&trap, &armed), Some(Paint::Fill));
    assert_eq!(fill(&collapse, &e), Some(Paint::FillDim));
    assert_eq!(
        fill(&field, &effect(SkillId::DawnField, K::Field)),
        Some(Paint::Fill)
    );
}

/// Honesty rule "nothing reveals a fogged hero": the tether is drawn only to an owner the
/// client draws, and never for an effect without one.
#[test]
fn the_tether_needs_a_visible_owner() {
    let hook = target_body(SkillId::IronHook, K::Bolt);
    assert_eq!(hook.marker, Marker::OwnerTether);
    let e = effect(SkillId::IronHook, K::Bolt);
    let tether = |seen: &Seen| shown(&hook, seen, |role| matches!(role, Role::Marker(_)));
    assert!(tether(&first(&e, 0.0)).is_empty());
    let chest = Vec3::new(AT.x - 6.0, 2.5, AT.y - 8.0);
    let seen = Seen::of(&e, None, Some(chest), 1.5, 0.0);
    let bar = tether(&seen)[0];
    let root = root_pose(&seen);
    let (from, to) = (
        root.transform_point(bar.transform_point(Vec3::Z * -0.5)),
        root.transform_point(bar.transform_point(Vec3::Z * 0.5)),
    );
    assert!(
        from.distance(Vec3::new(AT.x, 1.5 + CHEST, AT.y)) < 1e-4,
        "{from}"
    );
    assert!(to.distance(chest) < 1e-4, "{to}");
    // An effect of a hidden caster carries no owner id; no position can attach it.
    let orphan = with(&e, |e| e.owner_id = 0);
    assert!(tether(&Seen::of(&orphan, None, Some(chest), 1.5, 0.0)).is_empty());
    // Rule E-11: the chain and the tether are iron, not light.
    let seen = Seen {
        history: &[AT + Vec2::X],
        ..seen
    };
    for slot in slots(&hook, &seen, |role| {
        matches!(role, Role::Marker(_) | Role::Trail(_))
    }) {
        assert_eq!(
            part_paint(&slot, &seen),
            Some(Paint::Slot(PaletteSlot::Secondary))
        );
    }
    let ribbon = trail_body(Trail::Ribbon);
    let slot = slots(&ribbon, &seen, |role| matches!(role, Role::Trail(_)))[0];
    assert_eq!(
        part_paint(&slot, &seen),
        Some(Paint::Slot(PaletteSlot::Primary))
    );
    // The model brings its own materials.
    let model = slots(&hook, &seen, |role| role == Role::Model)[0];
    assert_eq!(part_paint(&model, &seen), None);
}

/// Rule E-11: above a packaged prop a column starts at its top.
#[test]
fn a_column_starts_at_the_top_of_the_prop() {
    let lantern = target_body(SkillId::GuidingLantern, K::Lantern);
    let e = effect(SkillId::GuidingLantern, K::Lantern);
    let seen = first(&e, 0.0);
    let model = slots(&lantern, &seen, |role| role == Role::Model)[0];
    let top = model_fit(Model::Lantern, &model, e.radius).1;
    assert!((top - 1.2 * 1.15).abs() < 1e-5);
    let wisps = shown(&lantern, &seen, |role| matches!(role, Role::Satellite(_)));
    assert_eq!(wisps.len(), 3);
    let mut heights: Vec<f32> = wisps.iter().map(|pose| pose.translation.y).collect();
    assert!(heights.iter().all(|height| *height > top));
    heights.dedup_by(|a, b| (*a - *b).abs() < 0.05);
    assert_eq!(heights.len(), 3, "the copies are stacked");
    assert!(
        wisps
            .iter()
            .all(|pose| pose.translation.xz().length() < 1e-5)
    );
    // Without a prop it stands on the ground.
    let mut plain = lantern.clone();
    plain.model = None;
    let lowest = shown(&plain, &seen, |role| matches!(role, Role::Satellite(_)))
        .iter()
        .map(|pose| pose.translation.y)
        .fold(f32::MAX, f32::min);
    assert!(lowest < 0.5);
}

#[test]
fn a_gyro_ring_is_level_at_rest_and_tilts_along_its_travel() {
    let orb = target_body(SkillId::OrbitalCommand, K::Orb);
    assert_eq!(orb.shell.as_ref().unwrap().behave, Behaviour::Gyro);
    let e = effect(SkillId::OrbitalCommand, K::Orb);
    let normal = |seen: &Seen| {
        let pose = shown(&orb, seen, |role| role == Role::Shell)[0];
        (root_pose(seen).rotation * pose.rotation * Vec3::Z).normalize()
    };
    // First seen, and at rest after a flight: level.
    assert!((normal(&first(&e, 0.0)) - Vec3::Y).length() < 1e-5);
    let came_from = [AT - Vec2::new(0.0, 2.0)];
    let resting = Seen {
        history: &came_from,
        resting: true,
        ..first(&e, 0.0)
    };
    assert!((normal(&resting) - Vec3::Y).length() < 1e-5);
    // In flight toward +Z of the simulation: tilted toward that direction.
    let flying = Seen {
        resting: false,
        ..resting
    };
    let tilted = normal(&flying);
    assert!((tilted - Vec3::new(0.0, GYRO_TILT.cos(), GYRO_TILT.sin())).length() < 1e-4);
    // The orb is laid out at the height that clears a hero, with its ribbon only in flight.
    let model = shown(&orb, &flying, |role| role == Role::Model)[0];
    assert_eq!(model.translation, Vec3::new(0.0, HIGH, 0.0));
    assert_eq!(
        shown(&orb, &flying, |role| matches!(role, Role::Trail(_))).len(),
        1
    );
    assert!(shown(&orb, &resting, |role| matches!(role, Role::Trail(_))).is_empty());
}

#[test]
fn behaviours_follow_the_stage_and_the_remaining_time() {
    let e = effect(SkillId::OrbitalCollapse, K::BeamWarning);
    let core = |behave, e: &SkillEffectState| {
        let mut body = bare(Archetype::Zone);
        body.core = Some(part(Silhouette::Block, [0.2, 0.2, 0.2], behave));
        shown(&body, &first(e, 0.0), |role| role == Role::Core).len()
    };
    // A fuse is a telegraph for its whole observed life; a field is live at once.
    let live = effect(SkillId::DawnField, K::Field);
    assert_eq!(core(Behaviour::HideInTelegraph, &e), 0);
    assert_eq!(core(Behaviour::OnlyInTelegraph, &e), 1);
    assert_eq!(core(Behaviour::HideInTelegraph, &live), 1);
    assert_eq!(core(Behaviour::OnlyInTelegraph, &live), 0);
    // `blink_last` shows the part until the last half second, then in steps.
    let blink = |remaining: f32| {
        core(
            Behaviour::BlinkLast,
            &with(&live, |e| e.remaining_secs = remaining),
        )
    };
    assert_eq!(
        [3.0, 0.51, 0.45, 0.35, 0.25, 0.15, 0.05].map(blink),
        [1, 1, 1, 0, 1, 0, 1]
    );
    // `rise_on_spawn` grows a part of a body with a known lifetime over its first moments.
    let wall = effect(SkillId::Northwall, K::ShieldWall);
    let spawn = category::spawn_lifetime_secs(SkillId::Northwall).unwrap();
    let share = |age: f32| {
        let state = with(&wall, |e| e.remaining_secs = spawn - age);
        let slot = PartSlot {
            role: Role::Core,
            mesh: Some(PartMesh::Silhouette(Silhouette::Block)),
            size: Vec3::ONE,
            slot: PaletteSlot::Primary,
            behave: Behaviour::RiseOnSpawn,
            plan: Plan {
                archetype: Archetype::Wall,
                altitude: Altitude::Ground,
                model: None,
                layout: None,
                trail: Trail::None,
                marker: Marker::None,
                shell_like_core: false,
            },
        };
        motion(&slot, &first(&state, 0.0)).unwrap().share.y
    };
    assert_eq!(share(0.0), FLAT_SHARE);
    assert!((share(0.1) - 0.5).abs() < 1e-4);
    assert!((share(0.2) - 1.0).abs() < 1e-4);
    // First seen later: at full height at once.
    assert_eq!(share(1.4), 1.0);
    // No behaviour makes a part larger than authored.
    for behave in Behaviour::ALL {
        for step in 0..200 {
            let slot = PartSlot {
                role: Role::Core,
                mesh: Some(PartMesh::Silhouette(Silhouette::Ball)),
                size: Vec3::ONE,
                slot: PaletteSlot::Primary,
                behave: *behave,
                plan: Plan {
                    archetype: Archetype::Zone,
                    altitude: Altitude::Ground,
                    model: None,
                    layout: None,
                    trail: Trail::None,
                    marker: Marker::None,
                    shell_like_core: false,
                },
            };
            if let Some(motion) = motion(&slot, &first(&live, step as f32 * 0.037)) {
                assert!(motion.share.max_element() <= 1.0, "{behave:?}");
                assert!(motion.share.min_element() > 0.0, "{behave:?}");
                assert!(motion.lift >= 0.0, "{behave:?} sinks into the ground");
            }
        }
    }
}

#[test]
fn every_pose_of_every_target_body_is_finite() {
    for (skill, kind, body) in target_bodies() {
        if !staged(body.archetype) {
            continue;
        }
        for radius in [0.0, 0.05, 1.0, 256.0] {
            for (history, resting) in [
                (vec![], false),
                (vec![AT], false),
                (vec![AT + Vec2::X, AT + Vec2::X], false),
                (vec![AT - Vec2::Y * 3.0, AT - Vec2::Y * 6.0], true),
            ] {
                for remaining in [0.0, 0.2, 4.0] {
                    let e = with(&effect(skill, kind), |e| {
                        e.radius = radius;
                        e.remaining_secs = remaining;
                        e.armed = remaining > 1.0;
                    });
                    let seen = Seen {
                        history: &history,
                        resting,
                        peak_remaining_secs: 0.0,
                        owner: Some(Vec3::new(AT.x, 1.0, AT.y)),
                        ..first(&e, 3.3)
                    };
                    assert!(root_pose(&seen).is_finite());
                    for slot in part_list(&body, &seen.shape) {
                        let (pose, visibility) = part_pose(&slot, &seen);
                        assert!(pose.is_finite(), "{} {:?}", skill.id(), slot.role);
                        if visibility != Visibility::Hidden {
                            assert!(pose.scale.min_element() > 0.0, "{:?}", slot.role);
                        }
                    }
                }
            }
        }
    }
}
