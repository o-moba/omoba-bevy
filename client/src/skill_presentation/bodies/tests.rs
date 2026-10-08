use super::super::SkillPresentation;
use super::super::schema::Satellites;
use super::super::stage::{EffectKey, EffectMemory, Frame};
use super::super::vocab::ExpireKind;
use super::*;
use shared::loadout::{EffectVisualKind, SkillId};
use std::collections::BTreeSet;

use EffectVisualKind as K;

const AT: Vec2 = Vec2::new(3.5, -2.25);

/// The heading of every test effect that has one.
const HEADING_OF: Vec2 = Vec2::new(0.6, 0.8);

fn effect(skill: SkillId, kind: EffectVisualKind) -> SkillEffectState {
    use geometry::GeoClass;
    // What the server sends as `end`: one unit ahead for a kind with a heading, the far
    // end of a strip or a cone at full range, the half-length of a wall ahead, and the
    // place itself for every other.
    let ahead = match geometry::boundary_class(skill, kind) {
        _ if category::heading_only(kind) => 1.0,
        GeoClass::Capsule | GeoClass::Lane | GeoClass::Sector => {
            shared::loadout::skill(skill).ability.cast_range.min(45.0)
        }
        GeoClass::Bar => geometry::replicated_radius(skill, kind),
        GeoClass::Ring | GeoClass::Pentagon | GeoClass::None => 0.0,
    };
    SkillEffectState {
        id: 41,
        owner_id: 7,
        owner_team: shared::map::Team::Green,
        skill,
        kind,
        position: AT.to_array(),
        end: (AT + HEADING_OF * ahead).to_array(),
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
        trail_scale: None,
        fill: None,
        fill_strength: None,
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
        PartMesh::Wedge => wedge_mesh(),
        PartMesh::Cap => cap_mesh(),
        PartMesh::Band => band_mesh(),
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
fn the_library_has_nineteen_small_meshes_inside_a_unit_cube() {
    let meshes: Vec<PartMesh> = Silhouette::ALL
        .iter()
        .map(|mesh| PartMesh::Silhouette(*mesh))
        .chain([
            PartMesh::Disc,
            PartMesh::Wedge,
            PartMesh::Cap,
            PartMesh::Band,
        ])
        .collect();
    assert_eq!(meshes.len(), 19);
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
    // The boundary rings, the cap of a strip and the fill disc end exactly on the unit
    // circle.
    for mesh in [
        PartMesh::Silhouette(Silhouette::Ring),
        PartMesh::Disc,
        PartMesh::Cap,
        PartMesh::Band,
    ] {
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
        .chain(
            [
                PartMesh::Disc,
                PartMesh::Wedge,
                PartMesh::Cap,
                PartMesh::Band,
            ]
            .map(|mesh| library.handle(mesh).id()),
        )
        .collect();
    assert_eq!(handles.len(), 19);
    assert_eq!(assets.len(), 19);
    // The layer over the inside of a cone is the kite point for point, and nothing shades
    // it: a fan or a plate silhouette is lit in its middle and deep on its outline, the
    // bands that bound a strip and every solid are of one colour.
    assert_eq!(
        vertices(PartMesh::Wedge),
        vertices(PartMesh::Silhouette(Silhouette::Kite))
    );
    for mesh in [
        PartMesh::Disc,
        PartMesh::Wedge,
        PartMesh::Cap,
        PartMesh::Band,
    ] {
        assert!(mesh_of(mesh).attribute(Mesh::ATTRIBUTE_COLOR).is_none());
    }
    for mesh in Silhouette::ALL {
        let shades: Vec<f32> = match silhouette_mesh(*mesh).attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) => {
                assert!(
                    colors
                        .iter()
                        .all(|c| c[0] == c[1] && c[1] == c[2] && c[3] == 1.0)
                );
                colors.iter().map(|c| c[0]).collect()
            }
            Some(other) => panic!("{mesh:?}: {other:?}"),
            None => Vec::new(),
        };
        let shaded = matches!(
            mesh,
            Silhouette::Kite
                | Silhouette::Star
                | Silhouette::Chevron
                | Silhouette::Diamond
                | Silhouette::Drop
                | Silhouette::Cross
                | Silhouette::Crescent
        );
        assert_eq!(!shades.is_empty(), shaded, "{mesh:?}");
        if shaded {
            let rim = crate::game_vfx::RIM_SHADE;
            assert!(shades.iter().all(|shade| *shade == 1.0 || *shade == rim));
            assert!(shades.contains(&1.0) && shades.contains(&rim), "{mesh:?}");
            // Every vertex on the unit circle is shaded: the tips of a shape are its edge.
            for (point, shade) in vertices(PartMesh::Silhouette(*mesh)).iter().zip(&shades) {
                if point.xy().length() > UNIT_RADIUS - 1e-3 {
                    assert_eq!(*shade, rim, "{mesh:?} at {point}");
                }
            }
        }
    }
    assert_eq!(
        library.particle(ParticleShape::Kite),
        Some(library.handle(PartMesh::Silhouette(Silhouette::Kite)))
    );
    assert_eq!(library.particle(ParticleShape::Glow), None);
}

/// The engine outlines a strip and fills a cone with two meshes whose measures it mirrors:
/// the band of the cap and the corners of the kite.
#[test]
fn the_mirrored_measures_of_the_cap_and_the_kite_are_those_of_their_meshes() {
    // The ring under a small body in flight is heavier than the ring of a zone.
    let ring = vertices(PartMesh::Band);
    let (inner, outer) = span(ring.iter().map(|point| point.xy().length()));
    assert!((outer - UNIT_RADIUS).abs() < 1e-6);
    assert!((inner - UNIT_RADIUS * (1.0 - FLIGHT_BAND)).abs() < 1e-6);
    assert!(inner < UNIT_RADIUS * MARKER_REACH);
    // A half ring that bulges toward +X and ends on the Y axis, thinner than the arc a row
    // may name.
    let named = vertices(PartMesh::Silhouette(Silhouette::Arc));
    let (named_inner, _) = span(named.iter().map(|point| point.xy().length()));
    let arc = vertices(PartMesh::Cap);
    let (inner, outer) = span(arc.iter().map(|point| point.xy().length()));
    assert!((outer - UNIT_RADIUS).abs() < 1e-6);
    assert!((inner - UNIT_RADIUS * (1.0 - ARC_BAND)).abs() < 1e-6);
    assert!(inner > named_inner);
    assert!(arc.iter().all(|point| point.x >= -1e-6));
    let (low, high) = span(arc.iter().map(|point| point.y));
    assert!((low + UNIT_RADIUS).abs() < 1e-6 && (high - UNIT_RADIUS).abs() < 1e-6);
    // A kite: its point on -X, its widest corners and its far corners.
    let kite = vertices(PartMesh::Silhouette(Silhouette::Kite));
    let point = kite
        .iter()
        .copied()
        .min_by(|a, b| a.x.total_cmp(&b.x))
        .unwrap();
    assert!(point.xy().distance(Vec2::new(-UNIT_RADIUS, 0.0)) < 1e-6);
    let from_point = |corner: &Vec3| Vec2::new(corner.x - point.x, corner.y.abs());
    let widest = kite
        .iter()
        .map(from_point)
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .unwrap();
    let farthest = kite
        .iter()
        .map(from_point)
        .max_by(|a, b| a.x.total_cmp(&b.x))
        .unwrap();
    assert!(widest.distance(KITE_SHOULDER) < 1e-6, "{widest}");
    assert!(farthest.distance(KITE_TOP) < 1e-6, "{farthest}");
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
    for (skill, kind, body) in bodies {
        let e = effect(skill, kind);
        let seen = first(&e, 0.0);
        let parts = part_list(&body, &seen.shape);
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
        // Rule E-10: the soul has no ground ring; every other body has the boundary of
        // its archetype.
        let boundary = match body.archetype {
            Archetype::Lane => 4,
            Archetype::Sector => 8,
            Archetype::Cage => 10,
            _ => usize::from(kind != K::Soul),
        };
        assert_eq!(
            count(|role| matches!(role, Role::Boundary(_))),
            boundary,
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

/// The boundary parts of a body in the world, one entry for each: the vertices of a part
/// that is drawn, `None` for a hidden one.
fn boundary_in_world(body: &Body, seen: &Seen) -> Vec<Option<Vec<Vec3>>> {
    let root = root_pose(seen);
    slots(body, seen, |role| matches!(role, Role::Boundary(_)))
        .iter()
        .map(|slot| {
            drawn(slot, seen).map(|points| {
                points
                    .into_iter()
                    .map(|point| root.transform_point(point))
                    .collect()
            })
        })
        .collect()
}

/// A ground point in the frame of a segment: to its side, and along it from `from`.
fn beside(point: Vec2, from: Vec2, to: Vec2) -> Vec2 {
    let axis = (to - from).normalize();
    let offset = point - from;
    Vec2::new(offset.perp_dot(axis), offset.dot(axis))
}

/// Distance of a ground point from a segment.
fn off_segment(point: Vec2, from: Vec2, to: Vec2) -> f32 {
    let Some(axis) = (to - from).try_normalize() else {
        return point.distance(from);
    };
    let along = (point - from).dot(axis).clamp(0.0, from.distance(to));
    point.distance(from + axis * along)
}

/// The largest and the smallest of some numbers.
fn span(values: impl Iterator<Item = f32>) -> (f32, f32) {
    values.fold((f32::MAX, f32::MIN), |(least, most), value| {
        (least.min(value), most.max(value))
    })
}

/// A body whose authored look is another one: nothing of it may reach the boundary.
fn altered(body: &Body) -> Body {
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
    altered.fill = Some(false);
    altered
}

/// What a client could have observed of an instance without any of it moving a boundary.
fn observed_later<'a>(seen: &Seen<'a>, history: &'a [Vec2]) -> Seen<'a> {
    Seen {
        history,
        renewed: true,
        resting: true,
        owner: Some(Vec3::new(9.0, 1.0, 9.0)),
        peak_remaining_secs: 9.0,
        now: 91.7,
        ..*seen
    }
}

/// AC "drawn boundaries equal replicated geometry": the engine parts of every archetype lie
/// exactly on the shape `geometry.rs` derives from the received fields, for every radius
/// and length, and nothing a row authors or the client observed can move or scale them.
#[test]
fn boundary_equals_replicated_geometry() {
    let history = [AT + Vec2::new(-1.0, 0.5), AT + Vec2::new(-2.0, 1.0)];
    let mut archetypes = BTreeSet::new();
    let mut checked = 0;
    for (skill, kind, body) in target_bodies() {
        if !matches!(
            first(&effect(skill, kind), 0.0).shape,
            GeoShape::Ring { .. }
        ) {
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
                // A line, not an area: the ring of a zone, and the heavier one under a small
                // body in flight, both inside the replicated radius.
                let band = if moving(body.archetype) && radius <= FLIGHT_BAND_RADIUS {
                    FLIGHT_BAND
                } else {
                    1.0 - MARKER_REACH
                };
                assert!(
                    (inner - radius * (1.0 - band)).abs() <= radius * 1e-4,
                    "{} at {radius}: the line starts at {inner}",
                    skill.id()
                );
                assert_eq!(
                    boundary[0].mesh,
                    Some(if band == FLIGHT_BAND {
                        PartMesh::Band
                    } else {
                        PartMesh::Silhouette(Silhouette::Ring)
                    })
                );

                // Authored sizes, the layout, the trail, the marker and the model do not
                // reach it, and neither do the clock or what was observed of the instance.
                let pose = part_pose(&boundary[0], &seen).0;
                let later = observed_later(&seen, &history);
                let same = slots(&altered(&body), &later, |role| {
                    matches!(role, Role::Boundary(_))
                });
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

    the_outline_of_a_strip_is_the_replicated_capsule(&history);
    the_outline_of_a_cone_is_the_replicated_sector(&history);
    the_bar_of_a_wall_is_the_replicated_plane(&history);
    the_bars_of_a_cage_are_the_replicated_pentagon(&history);
}

/// `lane`: two edge bars at the replicated half-width and a half ring around each end,
/// which together outline every point within `radius` of the received segment.
fn the_outline_of_a_strip_is_the_replicated_capsule(history: &[Vec2]) {
    let rows = [
        (SkillId::DawnRay, K::BeamWarning),
        (SkillId::DawnRay, K::Beam),
        (SkillId::HorizonWave, K::BeamWarning),
        (SkillId::WinterDivide, K::BeamWarning),
    ];
    for (skill, kind) in rows {
        let body = target_body(skill, kind);
        assert_eq!(body.archetype, Archetype::Lane);
        for radius in [0.3, 0.8, 2.0, 4.5] {
            for length in [0.5, 3.0, 20.0, 45.0] {
                for heading in [Vec2::X, HEADING_OF, Vec2::new(-0.8, 0.6)] {
                    let (from, to) = (AT, AT + heading * length);
                    let e = with(&effect(skill, kind), |e| {
                        e.radius = radius;
                        e.end = to.to_array();
                    });
                    let seen = Seen::of(&e, None, None, 1.75, 7.3);
                    assert_eq!(seen.shape, GeoShape::Capsule { from, to, radius });
                    let root = root_pose(&seen);
                    assert_eq!(root.translation, Vec3::new(AT.x, 1.75, AT.y));
                    assert!((root.rotation * Vec3::Z).xz().distance(heading) < 1e-5);
                    let what = format!("{} {radius} x {length}", skill.id());
                    let slack = 1e-4 * (1.0 + radius + length);
                    let parts = boundary_in_world(&body, &seen);
                    assert_eq!(parts.len(), 4, "{what}");
                    for (index, points) in parts.iter().enumerate() {
                        let points = points.as_ref().expect(&what);
                        for point in points {
                            // Nothing of the outline lies outside the capsule, and it is
                            // a line along its rim.
                            let off = off_segment(point.xz(), from, to);
                            assert!(off <= radius + slack, "{what}: part {index} at {off}");
                            assert!(off >= radius * (1.0 - ARC_BAND) - slack, "{what}");
                            assert!((point.y - 1.75 - BOUNDARY_LIFT).abs() <= LINE_HEIGHT);
                        }
                        let frame: Vec<Vec2> = points
                            .iter()
                            .map(|point| beside(point.xz(), from, to))
                            .collect();
                        let (_, side) = span(frame.iter().map(|at| at.x.abs()));
                        let (near, far) = span(frame.iter().map(|at| at.y));
                        // Every part reaches the replicated half-width.
                        assert!((side - radius).abs() <= slack, "{what}: {side}");
                        if index < 2 {
                            // An edge bar: on one side, from one end to the other.
                            assert!(near.abs() <= slack && (far - length).abs() <= slack);
                            let (left, right) = span(frame.iter().map(|at| at.x));
                            assert!(left * right > 0.0, "{what}: an edge is on one side");
                        } else {
                            // A cap: a half ring around its end and beyond it, out to the
                            // tip of the capsule.
                            let (end, tip) = if index == 2 {
                                assert!(far <= slack, "{what}");
                                (from, -near)
                            } else {
                                assert!(near >= length - slack, "{what}");
                                (to, far - length)
                            };
                            assert!((tip - radius).abs() <= slack, "{what}: {tip}");
                            let (_, rim) = span(points.iter().map(|p| p.xz().distance(end)));
                            assert!((rim - radius).abs() <= slack, "{what}");
                        }
                    }
                    // The two edges are the two sides.
                    let side_of =
                        |index: usize| beside(parts[index].as_ref().unwrap()[0].xz(), from, to).x;
                    assert!(side_of(0) * side_of(1) < 0.0);
                    // The whole outline is the team's, and no row or observation moves it.
                    let boundary = slots(&body, &seen, |role| matches!(role, Role::Boundary(_)));
                    let later = observed_later(&seen, history);
                    let same = slots(&altered(&body), &later, |role| {
                        matches!(role, Role::Boundary(_))
                    });
                    for (slot, other) in boundary.iter().zip(&same) {
                        assert_eq!(part_paint(slot, &seen), Some(Paint::Team));
                        assert_eq!(part_pose(slot, &seen), part_pose(other, &later), "{what}");
                    }
                }
            }
        }
    }
    // A strip without a length is the circle around its point: no edges, two half rings.
    let body = target_body(SkillId::WinterDivide, K::BeamWarning);
    let e = with(&effect(SkillId::WinterDivide, K::BeamWarning), |e| {
        e.end = e.position
    });
    let seen = first(&e, 0.0);
    let parts = boundary_in_world(&body, &seen);
    assert!(parts[0].is_none() && parts[1].is_none());
    for cap in &parts[2..] {
        let (inner, outer) = span(cap.as_ref().unwrap().iter().map(|p| p.xz().distance(AT)));
        assert!((outer - e.radius).abs() < 1e-4 && inner >= e.radius * (1.0 - ARC_BAND) - 1e-4);
    }
    // A strip with flat ends has its two edges and nothing around the ends.
    let to = AT + HEADING_OF * 12.0;
    let e = with(&e, |e| e.end = to.to_array());
    let flat = Seen {
        shape: GeoShape::Lane {
            from: AT,
            to,
            half_width: e.radius,
        },
        ..first(&e, 0.0)
    };
    let parts = boundary_in_world(&body, &flat);
    assert_eq!(parts.len(), 2);
    for points in parts {
        let frame: Vec<Vec2> = points
            .unwrap()
            .iter()
            .map(|point| beside(point.xz(), AT, to))
            .collect();
        let (near, far) = span(frame.iter().map(|at| at.y));
        let (_, side) = span(frame.iter().map(|at| at.x.abs()));
        assert!(near.abs() < 1e-3 && (far - 12.0).abs() < 1e-3 && (side - e.radius).abs() < 1e-3);
    }
}

/// `sector`: two edge bars and six chords of the arc, at the received length and the
/// half-angle of the skill. Rule F: a received axis shorter than the cast range was cut by
/// the fog and is one plain bar on the received segment.
fn the_outline_of_a_cone_is_the_replicated_sector(history: &[Vec2]) {
    let (skill, kind) = (SkillId::FurnaceBreath, K::BeamWarning);
    let body = target_body(skill, kind);
    assert_eq!(body.archetype, Archetype::Sector);
    let range = shared::loadout::skill(skill).ability.cast_range;
    let half_angle = category::cone_half_angle(skill).unwrap();
    assert!((half_angle.cos() - geometry::FURNACE_CONE_COS).abs() < 1e-6);
    for length in [range - 0.04, range, range + 2.5] {
        for radius in [1.0, 3.0, 6.0] {
            for heading in [Vec2::X, HEADING_OF, Vec2::new(-0.8, 0.6)] {
                let e = with(&effect(skill, kind), |e| {
                    e.radius = radius;
                    e.end = (AT + heading * length).to_array();
                });
                let seen = Seen::of(&e, None, None, 1.75, 7.3);
                let GeoShape::Sector {
                    apex,
                    axis,
                    radius: reach,
                    half_angle: drawn_angle,
                } = seen.shape
                else {
                    panic!("a full axis is a cone");
                };
                assert_eq!((apex, drawn_angle), (AT, half_angle));
                assert!(axis.distance(heading) < 1e-5 && (reach - length).abs() < 1e-5);
                // The apex is the replicated position; the replicated `radius` is no size
                // of the cone.
                assert_eq!(root_pose(&seen).translation, Vec3::new(AT.x, 1.75, AT.y));
                let parts = boundary_in_world(&body, &seen);
                assert_eq!(parts.len(), 8);
                // Where a point is in the cone: its distance from the apex and its angle
                // from the axis, positive toward the lateral axis of the body.
                let polar = |point: Vec3| {
                    let offset = point.xz() - AT;
                    let side = offset.dot(Vec2::new(heading.y, -heading.x));
                    (offset.length(), side.atan2(offset.dot(heading)))
                };
                let step = 2.0 * half_angle / f32::from(SECTOR_CHORDS);
                for (index, points) in parts.iter().enumerate() {
                    let points = points.as_ref().unwrap();
                    let polar: Vec<(f32, f32)> = points.iter().map(|p| polar(*p)).collect();
                    for (distance, angle) in &polar {
                        // Nothing of the outline lies outside the cone.
                        assert!(*distance <= length + 1e-3, "part {index}: {distance}");
                        assert!(
                            *distance < 1e-3 || angle.abs() <= half_angle + 1e-4,
                            "part {index}: {angle} at {distance}"
                        );
                    }
                    let (_, far) = span(polar.iter().map(|at| at.0));
                    if index < 2 {
                        // An edge: its outer side on the edge of the cone, from the apex
                        // to the arc.
                        let edge = if index == 0 { -half_angle } else { half_angle };
                        let on_edge: Vec<f32> = polar
                            .iter()
                            .filter(|(distance, angle)| {
                                *distance > 1e-3 && (angle - edge).abs() < 1e-4
                            })
                            .map(|at| at.0)
                            .collect();
                        let (_, reaches) = span(on_edge.iter().copied());
                        assert!((reaches - length).abs() < 1e-2, "edge {index}: {reaches}");
                        assert!(polar.iter().any(|(distance, _)| *distance < 1e-3));
                    } else {
                        // A chord between two points of the arc, a sixth of it apart:
                        // its outer corners are those points, except where it meets an
                        // edge and ends on it.
                        let chord = (index - 2) as f32;
                        let ends = [
                            -half_angle + step * chord,
                            -half_angle + step * (chord + 1.0),
                        ];
                        assert!((far - length).abs() < 1e-3, "chord {index}: {far}");
                        let on_arc: Vec<f32> = polar
                            .iter()
                            .filter(|(distance, _)| (distance - length).abs() < 1e-3)
                            .map(|at| at.1)
                            .collect();
                        for end in ends {
                            let at_an_edge = (end.abs() - half_angle).abs() < 1e-4;
                            assert!(
                                at_an_edge || on_arc.iter().any(|angle| (angle - end).abs() < 1e-3),
                                "chord {index} misses {end}"
                            );
                        }
                        for angle in on_arc {
                            assert!(ends.iter().any(|end| (angle - end).abs() < 1e-3));
                        }
                        // It is a line near the arc, not an area.
                        let (near, _) = span(polar.iter().map(|at| at.0));
                        assert!(near >= length * (step * 0.5).cos() - LINE_MAX - 1e-3);
                    }
                }
                let boundary = slots(&body, &seen, |role| matches!(role, Role::Boundary(_)));
                let later = observed_later(&seen, history);
                let same = slots(&altered(&body), &later, |role| {
                    matches!(role, Role::Boundary(_))
                });
                for (slot, other) in boundary.iter().zip(&same) {
                    assert_eq!(part_paint(slot, &seen), Some(Paint::Team));
                    assert_eq!(part_pose(slot, &seen), part_pose(other, &later));
                }
            }
        }
    }
    // Rule F, with and without a visible owner: the received segment and nothing else.
    for owner in [7, 0] {
        for length in [0.6, 4.0, range - 0.06] {
            let to = AT + HEADING_OF * length;
            let e = with(&effect(skill, kind), |e| {
                e.owner_id = owner;
                e.end = to.to_array();
                e.remaining_secs = 0.5;
            });
            let seen = first(&e, 0.4);
            assert_eq!(seen.shape, GeoShape::Segment { from: AT, to });
            let parts = boundary_in_world(&body, &seen);
            assert_eq!(parts.len(), 1);
            let frame: Vec<Vec2> = parts[0]
                .as_ref()
                .unwrap()
                .iter()
                .map(|point| beside(point.xz(), AT, to))
                .collect();
            let (near, far) = span(frame.iter().map(|at| at.y));
            let (left, right) = span(frame.iter().map(|at| at.x));
            assert!(near.abs() < 1e-4 && (far - length).abs() < 1e-4, "{length}");
            // Constant width, both ends the same: no apex, no taper.
            assert!((left + right).abs() < 1e-5 && right <= LINE_MAX * 0.5 + 1e-5);
            // No fill, no growing read-out and no flame of a cone that is not known whole.
            let all = part_list(&body, &seen.shape);
            assert_eq!(all.len(), part_total(&body, &seen.shape));
            let visible: Vec<Role> = all
                .iter()
                .filter(|slot| part_pose(slot, &seen).1 != Visibility::Hidden)
                .map(|slot| slot.role)
                .collect();
            assert_eq!(visible, [Role::Boundary(0)], "owner {owner}");
            assert_eq!(part_paint(&all[0], &seen), Some(Paint::Team));
        }
    }
}

/// `wall`: one bar across the heading, one unit ahead, half as long to each side as the
/// replicated radius.
fn the_bar_of_a_wall_is_the_replicated_plane(history: &[Vec2]) {
    let (skill, kind) = (SkillId::Northwall, K::ShieldWall);
    let body = target_body(skill, kind);
    assert_eq!(body.archetype, Archetype::Wall);
    for radius in [0.8, 2.5, 6.0] {
        for heading in [Vec2::X, HEADING_OF, Vec2::new(-0.8, 0.6)] {
            let e = with(&effect(skill, kind), |e| {
                e.radius = radius;
                e.end = (AT + heading * radius).to_array();
            });
            let seen = Seen::of(&e, None, None, 1.75, 7.3);
            let root = root_pose(&seen);
            assert_eq!(root.translation, Vec3::new(AT.x, 1.75, AT.y));
            assert!((root.rotation * Vec3::Z).xz().distance(heading) < 1e-5);
            let parts = boundary_in_world(&body, &seen);
            assert_eq!(parts.len(), 1);
            // Along the heading from the hero, and across it.
            let frame: Vec<Vec2> = parts[0]
                .as_ref()
                .unwrap()
                .iter()
                .map(|point| beside(point.xz(), AT, AT + heading))
                .collect();
            let (left, right) = span(frame.iter().map(|at| at.x));
            let (near, far) = span(frame.iter().map(|at| at.y));
            assert!((left + radius).abs() < 1e-4 && (right - radius).abs() < 1e-4);
            assert!(((near + far) * 0.5 - geometry::WALL_AHEAD).abs() < 1e-4);
            assert!(far - near <= LINE_MAX + 1e-5, "the plane is a line");
            let slot = slots(&body, &seen, |role| matches!(role, Role::Boundary(_)))[0];
            assert_eq!(part_paint(&slot, &seen), Some(Paint::Team));
            let later = observed_later(&seen, history);
            let same = slots(&altered(&body), &later, |role| {
                matches!(role, Role::Boundary(_))
            })[0];
            assert_eq!(part_pose(&slot, &seen), part_pose(&same, &later));
        }
    }
    // A wall without a heading has no plane, and nothing is stood on one.
    let e = with(&effect(skill, kind), |e| e.end = e.position);
    let seen = first(&e, 0.0);
    assert_eq!(seen.shape, GeoShape::None);
    assert!(shown(&body, &seen, |_| true).is_empty());
}

/// `cage`: a low bar and a rail on each side of the pentagon whose corners stand at the
/// replicated radius, the first one toward world +X (rule E-9).
fn the_bars_of_a_cage_are_the_replicated_pentagon(history: &[Vec2]) {
    let (skill, kind) = (SkillId::IronBoundary, K::Cage);
    let body = target_body(skill, kind);
    assert_eq!(body.archetype, Archetype::Cage);
    for radius in [2.0, 5.0, 8.5] {
        // The heading field of a cage means nothing and turns nothing.
        for end in [AT, AT + HEADING_OF * 3.0] {
            let e = with(&effect(skill, kind), |e| {
                e.radius = radius;
                e.end = end.to_array();
            });
            let seen = Seen::of(&e, None, None, 1.75, 7.3);
            assert_eq!(root_pose(&seen).translation, Vec3::new(AT.x, 1.75, AT.y));
            let parts = boundary_in_world(&body, &seen);
            assert_eq!(parts.len(), 10);
            let boundary = slots(&body, &seen, |role| matches!(role, Role::Boundary(_)));
            let corner = |index: usize| AT + Vec2::from_angle(TAU * index as f32 / 5.0) * radius;
            assert!(corner(0).distance(AT + Vec2::X * radius) < 1e-5);
            for (index, points) in parts.iter().enumerate() {
                let points = points.as_ref().unwrap();
                let (from, to) = (corner(index % 5), corner(index % 5 + 1));
                let frame: Vec<Vec2> = points
                    .iter()
                    .map(|point| beside(point.xz(), from, to))
                    .collect();
                // From corner to corner, the same band to both sides of the side.
                let (near, far) = span(frame.iter().map(|at| at.y));
                assert!(near.abs() < 1e-3 && (far - from.distance(to)).abs() < 1e-3);
                let (left, right) = span(frame.iter().map(|at| at.x));
                let (low, high) = span(points.iter().map(|point| point.y - 1.75));
                if index < 5 {
                    // The low bar is the band the server tests around the side.
                    assert!((right - geometry::CAGE_BAR_HALF_WIDTH).abs() < 1e-4);
                    assert!((left + geometry::CAGE_BAR_HALF_WIDTH).abs() < 1e-4);
                    assert!(low > 0.0 && high < 0.1);
                    assert_eq!(part_paint(&boundary[index], &seen), Some(Paint::Team));
                } else {
                    assert!((left + right).abs() < 1e-4 && right <= CAGE_RAIL);
                    assert!(((low + high) * 0.5 - CAGE_RAIL_HEIGHT).abs() < 1e-4);
                    assert_eq!(
                        part_paint(&boundary[index], &seen),
                        Some(Paint::Slot(PaletteSlot::Primary))
                    );
                }
                // The centre line of each bar ends on the circle of the corners.
                for point in points {
                    assert!(point.xz().distance(AT) <= radius + geometry::CAGE_BAR_HALF_WIDTH);
                }
            }
            let later = observed_later(&seen, history);
            let same = slots(&altered(&body), &later, |role| {
                matches!(role, Role::Boundary(_))
            });
            for (slot, other) in boundary.iter().zip(&same) {
                assert_eq!(part_pose(slot, &seen), part_pose(other, &later));
            }
        }
    }
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
    // A `core` and a `shell` in the middle of a circle may be larger than that: as large
    // as the measure of the parser allows, which is taken on their own meshes. Each shape
    // is scaled until that measure is exactly at its bound.
    let shapes = [
        [2.0, 3.0, 2.0],
        [2.0, 1.0, 0.02],
        [0.02, 1.0, 2.0],
        [0.3, 0.05, 2.0],
        [1.0, 0.02, 1.0],
        [1.0, 1.0, 1.0],
    ];
    for (archetype, skill) in [
        (Archetype::Zone, SkillId::DawnField),
        (Archetype::Prop, SkillId::WildTraps),
        // Rule E-6: the plate of a stance stands half a radius ahead of the centre.
        (Archetype::Prop, SkillId::MirrorGuard),
    ] {
        let kind = category::own_kinds(skill)[0];
        for mesh in Silhouette::ALL {
            for behave in Behaviour::ALL {
                if matches!(behave, Behaviour::Gyro | Behaviour::OnlyAfterRenew) {
                    continue;
                }
                for shape in shapes {
                    for altitude in [Altitude::Ground, Altitude::Chest] {
                        let lead = plate_lead(archetype, kind, *mesh);
                        let measure = |size: [f32; 3]| {
                            centred_reach(
                                &part(*mesh, size, *behave),
                                altitude != Altitude::Ground,
                                lead,
                            )
                        };
                        // The measure is the lead of the plate plus a term that grows with
                        // the size, and the lead alone is inside the bound.
                        let (mut low, mut high) = (0.0_f32, 4.0_f32);
                        for _ in 0..40 {
                            let middle = 0.5 * (low + high);
                            if measure(shape.map(|extent| extent * middle)) <= MARKER_REACH {
                                low = middle;
                            } else {
                                high = middle;
                            }
                        }
                        let size = shape.map(|extent| extent * low);
                        assert!(low > 0.1 && measure(size) > MARKER_REACH - 1e-3);
                        let mut body = bare(archetype);
                        body.altitude = Some(altitude);
                        body.fill = Some(true);
                        body.core = Some(part(*mesh, size, *behave));
                        body.shell = Some(part(*mesh, size, *behave));
                        for radius in [0.3, 0.4, 3.0] {
                            let e = with(&effect(skill, kind), |e| e.radius = radius);
                            for now in [0.0, 0.31, 1.7, 4.4] {
                                for armed in [false, true] {
                                    let state = with(&e, |e| e.armed = armed);
                                    inside_the_boundary(
                                        &body,
                                        &first(&state, now),
                                        &format!("{mesh:?} {behave:?} {size:?} {altitude:?}"),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // A ring that lies in the circle spans it up to the inner edge of its line, and a
    // row that keeps every extent within the radius never needed the measure.
    let lying = |mesh, size: f32| {
        centred_reach(
            &part(mesh, [size, 0.02, size], Behaviour::Steady),
            false,
            0.0,
        )
    };
    assert!((lying(Silhouette::Ring, 2.0 * MARKER_REACH) - MARKER_REACH).abs() < 1e-5);
    assert!(lying(Silhouette::Block, 1.0) < MARKER_REACH);
    // A prop wider than a small boundary is scaled down to it.
    let mut lantern = bare(Archetype::Zone);
    lantern.model = Some(Model::Lantern);
    let small = with(&effect(SkillId::GuidingLantern, K::Lantern), |e| {
        e.radius = 0.2
    });
    inside_the_boundary(&lantern, &first(&small, 0.0), "a small lantern");
}

/// Every drawn part of a body in the world, with its role.
fn parts_in_world(body: &Body, seen: &Seen) -> Vec<(Role, Vec<Vec3>)> {
    let root = root_pose(seen);
    part_list(body, &seen.shape)
        .iter()
        .filter(|slot| slot.role != Role::Model)
        .filter_map(|slot| drawn(slot, seen).map(|points| (slot.role, points)))
        .map(|(role, points)| {
            let points = points
                .into_iter()
                .map(|point| root.transform_point(point))
                .collect();
            (role, points)
        })
        .collect()
}

/// Asserts that everything a strip, a cone, a wall or a cage shows beside its boundary
/// lies inside the replicated shape: within the radius of the segment, inside the cone,
/// between the two ends of the wall bar, inside the circle of the corners of the cage.
/// `grounded` also asks that nothing dips under the ground.
fn inside_the_shape(body: &Body, seen: &Seen, grounded: bool, what: &str) {
    let e = seen.effect;
    let slack = 1e-4 * (1.0 + e.radius) + 1e-3;
    for (role, points) in parts_in_world(body, seen) {
        if matches!(role, Role::Boundary(_)) {
            continue;
        }
        for point in points {
            let at = point.xz();
            let inside = match seen.shape {
                GeoShape::Capsule { from, to, radius } => {
                    off_segment(at, from, to) <= radius + slack
                }
                GeoShape::Sector {
                    apex,
                    axis,
                    radius,
                    half_angle,
                } => {
                    let offset = at - apex;
                    offset.length() <= radius + slack
                        && (offset.length() < slack
                            || axis.angle_to(offset).abs() <= half_angle + 1e-3)
                }
                GeoShape::Segment { from, to } => {
                    let along = beside(at, from, to);
                    (-slack..=from.distance(to) + slack).contains(&along.y)
                        && along.x.abs() <= e.radius + slack
                }
                GeoShape::Pentagon { center, radius } => at.distance(center) <= radius + slack,
                other => panic!("{what}: {other:?} is no shape of these bodies"),
            };
            assert!(inside, "{what}: {role:?} reaches {at} at {}", seen.now);
            assert!(
                !grounded || point.y >= seen.ground,
                "{what}: {role:?} is under the ground"
            );
        }
    }
}

#[test]
fn no_authored_part_leaves_the_boundary_of_a_strip_a_cone_a_wall_or_a_cage() {
    // The final rows, through their stages, over sizes a server could send.
    let mut rows = 0;
    for (skill, kind, body) in target_bodies() {
        if matches!(
            body.archetype,
            Archetype::Traveller | Archetype::Orbiter | Archetype::Zone | Archetype::Prop
        ) {
            continue;
        }
        rows += 1;
        for radius in [0.4, 1.0, 2.5, 5.0] {
            for length in [0.4, 7.0, 30.0] {
                let e = with(&effect(skill, kind), |e| {
                    e.radius = radius;
                    // A cone is whole at its range only; a wall sends its half-length.
                    if body.archetype == Archetype::Lane {
                        e.end = (AT + HEADING_OF * length).to_array();
                    } else if body.archetype == Archetype::Wall {
                        e.end = (AT + HEADING_OF * radius).to_array();
                    }
                });
                for (state, now) in area_states(&e) {
                    // The whole telegraph, the firing moment and the time after it.
                    for remaining in [state.remaining_secs, 0.6, 0.2] {
                        let state = with(&state, |e| e.remaining_secs = remaining);
                        let seen = Seen {
                            peak_remaining_secs: 5.0,
                            ..first(&state, now)
                        };
                        assert!(!matches!(
                            seen.shape,
                            GeoShape::None | GeoShape::Ring { .. }
                        ));
                        inside_the_shape(&body, &seen, true, skill.id());
                    }
                }
            }
        }
    }
    // The ray as a warning and as a beam, the wave warning, the fissure, the cone, the
    // wall and the cage.
    assert_eq!(rows, 7);

    // Everything a row could author: every silhouette at the largest sizes the parser
    // lets through, with every behaviour, as core, shell and in every layout of the
    // archetype.
    let sizes = [
        [1.0, 3.0, 1.0],
        [1.0, 1.0, 0.02],
        [0.3, 0.05, 1.0],
        [0.14, 0.5, 0.14],
        [1.0, 1.0, 1.0],
    ];
    let cases: [(Archetype, SkillId, EffectVisualKind, &[SatelliteLayout]); 5] = [
        (
            Archetype::Lane,
            SkillId::DawnRay,
            K::BeamWarning,
            &[SatelliteLayout::Line, SatelliteLayout::Stagger],
        ),
        (
            Archetype::Lane,
            SkillId::WinterDivide,
            K::BeamWarning,
            &[SatelliteLayout::Line, SatelliteLayout::Stagger],
        ),
        (
            Archetype::Sector,
            SkillId::FurnaceBreath,
            K::BeamWarning,
            &[SatelliteLayout::Fan],
        ),
        (
            Archetype::Wall,
            SkillId::Northwall,
            K::ShieldWall,
            &[SatelliteLayout::Line],
        ),
        (
            Archetype::Cage,
            SkillId::IronBoundary,
            K::Cage,
            &[SatelliteLayout::Rim],
        ),
    ];
    let mut shown_parts = 0;
    for (archetype, skill, kind, layouts) in cases {
        let telegraphed = category::telegraph_secs(skill).is_some();
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
                    if telegraphed {
                        body.marker = Marker::FillToEdge;
                    }
                    body.core = Some(part(*mesh, size, *behave));
                    body.shell = Some(part(*mesh, size, *behave));
                    for layout in layouts {
                        for count in [1, 2, 5, 8] {
                            body.satellites = Some(Satellites {
                                mesh: *mesh,
                                layout: *layout,
                                count,
                                size,
                                slot: PaletteSlot::Accent,
                                behave: *behave,
                            });
                            for (radius, length) in [(0.4, 0.3), (2.0, 6.0), (1.2, 30.0)] {
                                let e = with(&effect(skill, kind), |e| {
                                    e.radius = radius;
                                    if archetype == Archetype::Lane {
                                        e.end = (AT + HEADING_OF * length).to_array();
                                    } else if archetype == Archetype::Wall {
                                        e.end = (AT + HEADING_OF * radius).to_array();
                                    }
                                    e.remaining_secs = 0.45;
                                });
                                for now in [0.0, 0.31, 1.7] {
                                    let seen = first(&e, now);
                                    // A part that turns end over end may dip into the
                                    // ground it stands on.
                                    inside_the_shape(
                                        &body,
                                        &seen,
                                        *behave != Behaviour::Tumble,
                                        &format!("{mesh:?} {behave:?} {layout:?} x{count}"),
                                    );
                                    shown_parts += parts_in_world(&body, &seen).len();
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // The sweep is not vacuous: parts were drawn in it.
    assert!(shown_parts > 100_000, "{shown_parts}");
}

/// AC10: an enemy who does not see the caster gets the fog-cut segment of a strip, and
/// nothing drawn on it may say at which end the caster stands. With `owner_id == 0` the
/// whole body is the same when the two received ends change places.
#[test]
fn hidden_owner_lane_is_direction_blind() {
    let lanes = [
        (SkillId::HorizonWave, K::BeamWarning),
        (SkillId::DawnRay, K::BeamWarning),
        (SkillId::DawnRay, K::Beam),
        (SkillId::WinterDivide, K::BeamWarning),
    ];
    let every_pose = |body: &Body, seen: &Seen| -> Vec<_> {
        part_list(body, &seen.shape)
            .iter()
            .map(|slot| {
                let (pose, visibility) = part_pose(slot, seen);
                (slot.role, pose, visibility, part_paint(slot, seen))
            })
            .collect()
    };
    for (skill, kind) in lanes {
        let body = target_body(skill, kind);
        // Segments in every direction, so that either end can be the one that sorts first.
        for toward in [
            Vec2::new(9.0, 0.0),
            Vec2::new(-9.0, 0.0),
            Vec2::new(0.0, 5.0),
            Vec2::new(0.0, -5.0),
            Vec2::new(-6.0, 8.0),
            Vec2::new(6.0, -8.0),
        ] {
            let telegraph = category::telegraph_secs(skill).unwrap_or(1.0);
            let tail = category::tail_secs(skill).unwrap_or(0.0);
            let hidden = with(&effect(skill, kind), |e| {
                e.owner_id = 0;
                e.end = (AT + toward).to_array();
                e.remaining_secs = tail + telegraph * 0.4;
            });
            let swapped = with(&hidden, |e| std::mem::swap(&mut e.position, &mut e.end));
            for now in [0.0, 0.37, 2.9] {
                let (one, other) = (first(&hidden, now), first(&swapped, now));
                assert_eq!(root_pose(&one), root_pose(&other), "{}", skill.id());
                let poses = every_pose(&body, &one);
                assert_eq!(poses, every_pose(&body, &other), "{}", skill.id());
                assert!(
                    poses
                        .iter()
                        .filter(|pose| pose.2 != Visibility::Hidden)
                        .count()
                        >= 6
                );
                // The ground under the root is asked for at the same end, too.
                assert_eq!(root_at(&hidden), root_at(&swapped));
            }

            // A viewer who sees the caster gets the strip from its origin: the frame
            // stands on the replicated position and heads for the replicated end.
            let known = with(&hidden, |e| e.owner_id = 7);
            let back = with(&swapped, |e| e.owner_id = 7);
            let (one, other) = (first(&known, 0.0), first(&back, 0.0));
            assert_eq!(root_at(&known), AT);
            assert_eq!(root_at(&back), AT + toward);
            let ahead = |seen: &Seen| (root_pose(seen).rotation * Vec3::Z).xz();
            assert!(ahead(&one).distance(toward.normalize()) < 1e-5);
            assert!(ahead(&other).distance(-toward.normalize()) < 1e-5);
            // Rule P-2 for every viewer: the copies of a row stand on the same places
            // whichever end the strip is laid out from.
            let places = |seen: &Seen| {
                let root = root_pose(seen);
                let mut places: Vec<Vec2> =
                    shown(&body, seen, |role| matches!(role, Role::Satellite(_)))
                        .iter()
                        .map(|pose| root.transform_point(pose.translation).xz())
                        .collect();
                // Along the strip from its replicated origin.
                places.sort_by(|a, b| a.distance(AT).total_cmp(&b.distance(AT)));
                places
            };
            let (here, there) = (places(&one), places(&other));
            assert_eq!(here.len(), there.len());
            for (a, b) in here.iter().zip(&there) {
                assert!(a.distance(*b) < 1e-3, "{}: {a} and {b}", skill.id());
            }
        }
    }

    // A part that has a facing shows it only to a viewer who may know the direction.
    let mut arrows = bare(Archetype::Lane);
    arrows.satellites = Some(Satellites {
        mesh: Silhouette::Chevron,
        layout: SatelliteLayout::Line,
        count: 6,
        size: [0.7, 0.02, 0.7],
        slot: PaletteSlot::Primary,
        behave: Behaviour::Steady,
    });
    let toward = Vec2::new(-6.0, 8.0);
    let facing = |owner: u64, swap: bool| -> Vec<Vec2> {
        let e = with(&effect(SkillId::HorizonWave, K::BeamWarning), |e| {
            e.owner_id = owner;
            e.end = (AT + toward).to_array();
            if swap {
                std::mem::swap(&mut e.position, &mut e.end);
            }
        });
        let seen = first(&e, 0.0);
        let root = root_pose(&seen);
        shown(&arrows, &seen, |role| matches!(role, Role::Satellite(_)))
            .iter()
            // A flat mesh points along its own +X.
            .map(|pose| (root.rotation * pose.rotation * Vec3::X).xz())
            .collect()
    };
    for point in facing(7, false) {
        assert!(point.distance(toward.normalize()) < 1e-5);
    }
    for point in facing(7, true) {
        assert!(point.distance(-toward.normalize()) < 1e-5);
    }
    assert_eq!(facing(0, false), facing(0, true));
    assert_eq!(facing(0, false).len(), 6);

    // What the fog left of a cone is one bar with two equal ends, for either order.
    let cone = target_body(SkillId::FurnaceBreath, K::BeamWarning);
    let cut = with(&effect(SkillId::FurnaceBreath, K::BeamWarning), |e| {
        e.owner_id = 0;
        e.end = (AT + toward * 0.4).to_array();
    });
    let swapped = with(&cut, |e| std::mem::swap(&mut e.position, &mut e.end));
    let bar_of = |e: &SkillEffectState| {
        let seen = first(e, 0.0);
        let mut points: Vec<[i32; 3]> = boundary_in_world(&cone, &seen)[0]
            .as_ref()
            .unwrap()
            .iter()
            .map(|point| point.to_array().map(|value| (value * 1e3).round() as i32))
            .collect();
        points.sort_unstable();
        points.dedup();
        (points, shown(&cone, &seen, |_| true).len())
    };
    assert_eq!(bar_of(&cut), bar_of(&swapped));
    assert_eq!(bar_of(&cut).1, 1);
}

/// The growing read-out of a telegraph reaches the boundary exactly when the server fires:
/// progress is 1 when `remaining_secs` reads the tail, also for an effect first seen in
/// the middle of its telegraph, because nothing but the newest snapshot is read.
#[test]
fn fill_completes_when_the_server_fires() {
    let mut rows = Vec::new();
    for (skill, kind, body) in target_bodies() {
        if body.marker != Marker::FillToEdge || kind == K::Beam {
            continue;
        }
        rows.push(skill.id());
        let (telegraph, tail) = category::telegraph_secs(skill)
            .zip(category::tail_secs(skill))
            .unwrap();
        let e = effect(skill, kind);
        // How far the read-out reaches at a remaining time, as a share of where the inner
        // side of the boundary line is; `None` while it is not drawn.
        let reach = |remaining: f32| -> Option<f32> {
            let state = with(&e, |e| e.remaining_secs = remaining);
            let seen = first(&state, 0.0);
            assert_eq!(seen.view.stage, Stage::Telegraph);
            let slot = slots(&body, &seen, |role| matches!(role, Role::Marker(_)))[0];
            let points = drawn(&slot, &seen)?;
            Some(match seen.shape {
                GeoShape::Ring { radius, .. } => {
                    assert_eq!(
                        part_paint(&slot, &seen),
                        Some(Paint::Slot(PaletteSlot::Primary))
                    );
                    span(points.iter().map(|point| point.xz().length())).1 / (radius * MARKER_REACH)
                }
                GeoShape::Capsule { from, to, radius } => {
                    assert_eq!(part_paint(&slot, &seen), Some(Paint::FillDim));
                    // Rule P-2: it spreads across the width and always has the whole length.
                    let (near, far) = span(points.iter().map(|point| point.z));
                    assert!(near.abs() < 1e-3 && (far - from.distance(to)).abs() < 1e-3);
                    let (left, right) = span(points.iter().map(|point| point.x));
                    assert!((left + right).abs() < 1e-4, "it grows from the centre line");
                    right / (radius * (1.0 - ARC_BAND))
                }
                GeoShape::Sector {
                    radius, half_angle, ..
                } => {
                    assert_eq!(part_paint(&slot, &seen), Some(Paint::FillDim));
                    // It grows from the apex, between the two edges.
                    for point in &points {
                        let angle = point.x.atan2(point.z).abs();
                        assert!(point.xz().length() < 1e-3 || angle <= half_angle + 1e-4);
                    }
                    assert!(points.iter().any(|point| point.xz().length() < 1e-3));
                    span(points.iter().map(|point| point.xz().length())).1 / radius
                }
                other => panic!("{other:?}"),
            })
        };
        let near = |reach: Option<f32>, share: f32| {
            reach.is_some_and(|reach| (reach - share).abs() < 1e-3)
        };
        // Nothing at the first moment, half of it half-way, and all of it at the firing
        // tick, when the effect still has its tail to live.
        assert!(
            reach(tail + telegraph).unwrap_or(0.0) < 1e-4,
            "{}",
            skill.id()
        );
        assert!(near(reach(tail + telegraph * 0.5), 0.5), "{}", skill.id());
        assert!(near(reach(tail + telegraph * 0.25), 0.75), "{}", skill.id());
        assert!(near(reach(tail), 1.0), "{}", skill.id());
        assert!(near(reach((tail - 0.1).max(0.0)), 1.0), "{}", skill.id());
        // It never runs back.
        let mut last = 0.0;
        for step in 0..=40 {
            let now = reach(tail + telegraph * (1.0 - step as f32 / 40.0)).unwrap_or(0.0);
            assert!(now >= last - 1e-5, "{}", skill.id());
            last = now;
        }
        // The dim fill under it is whole from the first frame.
        if fills(&body) {
            let state = with(&e, |e| e.remaining_secs = tail + telegraph);
            let seen = first(&state, 0.0);
            let fill = slots(&body, &seen, |role| role == Role::Fill)[0];
            assert_eq!(part_paint(&fill, &seen), Some(Paint::FillDim));
            assert!(drawn(&fill, &seen).is_some(), "{}", skill.id());
        }
    }
    rows.sort_unstable();
    assert_eq!(
        rows,
        [
            "dawn_ray",
            "furnace_breath",
            "horizon_wave",
            "mirror_guard",
            "orbital_collapse"
        ]
    );
    // The ray that fired has no read-out left and its fill at full strength.
    let ray = target_body(SkillId::DawnRay, K::Beam);
    let e = with(&effect(SkillId::DawnRay, K::Beam), |e| {
        e.remaining_secs = 0.1
    });
    let seen = first(&e, 0.0);
    assert_eq!(seen.view.stage, Stage::Active);
    assert!(shown(&ray, &seen, |role| matches!(role, Role::Marker(_))).is_empty());
    let fill = slots(&ray, &seen, |role| role == Role::Fill)[0];
    assert_eq!(part_paint(&fill, &seen), Some(Paint::Fill));
    // The fill of a strip covers the received segment at the replicated width, and the
    // fill of a cone lies inside it with its far corners on the arc.
    let sheet = drawn(&fill, &seen).unwrap();
    let (left, right) = span(sheet.iter().map(|point| point.x));
    let (near, far) = span(sheet.iter().map(|point| point.z));
    assert!((right - e.radius).abs() < 1e-4 && (left + e.radius).abs() < 1e-4);
    assert!(near.abs() < 1e-3 && (far - 45.0).abs() < 1e-3);
    let cone = target_body(SkillId::FurnaceBreath, K::BeamWarning);
    let e = effect(SkillId::FurnaceBreath, K::BeamWarning);
    let seen = first(&e, 0.0);
    let fill = slots(&cone, &seen, |role| role == Role::Fill)[0];
    let kite = drawn(&fill, &seen).unwrap();
    let half_angle = category::cone_half_angle(e.skill).unwrap();
    let (_, far) = span(kite.iter().map(|point| point.xz().length()));
    let (_, widest) = span(kite.iter().map(|point| point.x.atan2(point.z).abs()));
    assert!((far - 7.0).abs() < 1e-3, "{far}");
    assert!((widest - half_angle).abs() < 1e-3, "{widest}");
}

/// `rise_on_spawn` reads the age of an instance from its remaining time alone, so an
/// instance first seen later than its first moments stands at full height at once.
#[test]
fn rise_on_spawn_is_full_height_when_first_seen_late() {
    for (skill, kind, tall) in [
        (SkillId::Northwall, K::ShieldWall, 0.8),
        (SkillId::WinterDivide, K::BeamWarning, 1.2),
    ] {
        let body = target_body(skill, kind);
        let copies = body.satellites.as_ref().unwrap();
        assert_eq!(copies.behave, Behaviour::RiseOnSpawn);
        assert_eq!(copies.size[1], tall);
        let e = effect(skill, kind);
        let spawn = category::spawn_lifetime_secs(skill).unwrap();
        // The height of every copy at an age of the instance. No memory is given: this is
        // what a client draws that sees the instance for the first time at that age.
        let heights = |age: f32| -> Vec<f32> {
            let state = with(&e, |e| e.remaining_secs = spawn - age);
            let seen = first(&state, 0.3);
            slots(&body, &seen, |role| matches!(role, Role::Satellite(_)))
                .iter()
                .map(|slot| {
                    let points = drawn(slot, &seen).unwrap();
                    let (low, high) = span(points.iter().map(|point| point.y));
                    assert!((low - GROUND_LIFT).abs() < 1e-4, "it stands on the ground");
                    high - low
                })
                .collect()
        };
        // A grown copy is as tall as its mesh makes the authored height.
        let full = heights(spawn * 0.5)[0];
        assert!(full <= tall * e.radius + 1e-4 && full >= tall * e.radius * 0.8);
        let all = |age: f32, share: f32| {
            let heights = heights(age);
            assert_eq!(heights.len(), usize::from(copies.count));
            heights
                .iter()
                .all(|height| (height - full * share).abs() < 1e-3)
        };
        // They rise together over the first 0.2 s of the instance.
        assert!(all(0.0, FLAT_SHARE), "{}", skill.id());
        assert!(all(0.1, 0.5), "{}", skill.id());
        assert!(all(RISE_SECS, 1.0), "{}", skill.id());
        // First seen late, at any later age: at full height in that very frame.
        for age in [0.25, 1.4, spawn - 0.05] {
            assert!(all(age, 1.0), "{} at {age}", skill.id());
        }
        // More time left than the lifetime: the age is not known, nothing is replayed.
        assert!(all(-0.4, 1.0), "{}", skill.id());
        // The boundary, and the fill of the fissure, are whole in the first frame: the
        // effect acts from its first tick.
        let born = with(&e, |e| e.remaining_secs = spawn);
        let seen = first(&born, 0.0);
        let engine = shown(&body, &seen, |role| {
            matches!(role, Role::Boundary(_) | Role::Fill)
        })
        .len();
        assert_eq!(
            engine,
            usize::from(engine_parts(body.archetype, &seen.shape)) + usize::from(fills(&body))
        );
    }
}

/// The pinned assertions of the legacy cage, now against the body: a consumed side is
/// gone, every other side stands, and everything stays inside the replicated radius.
#[test]
fn cage_sides_are_hidden_per_consumed_bit() {
    let body = target_body(SkillId::IronBoundary, K::Cage);
    let e = effect(SkillId::IronBoundary, K::Cage);
    assert_eq!(e.radius, 5.0);
    for consumed in 0..32u8 {
        let state = with(&e, |e| e.consumed_segments = consumed);
        let seen = first(&state, 0.0);
        let bars = boundary_in_world(&body, &seen);
        assert_eq!(bars.len(), 10);
        for (index, bar) in bars.iter().enumerate() {
            let side = index % 5;
            assert_eq!(
                bar.is_none(),
                consumed & (1 << side) != 0,
                "side {side} of {consumed:#07b}"
            );
        }
        // The collars on the corners and the link in the middle stay.
        assert_eq!(
            shown(&body, &seen, |role| role.authored()).len(),
            6,
            "{consumed:#07b}"
        );
    }
    // The case the legacy test pins.
    let state = with(&e, |e| e.consumed_segments = 0b00101);
    let seen = first(&state, 0.0);
    for (index, slot) in slots(&body, &seen, |role| matches!(role, Role::Boundary(_)))
        .iter()
        .enumerate()
    {
        let (pose, visibility) = part_pose(slot, &seen);
        let side = index % 5;
        assert_eq!(visibility == Visibility::Hidden, side == 0 || side == 2);
        assert!(pose.translation.xz().length() <= e.radius);
    }
    // Rule E-9: the low bars are as wide as the band the server tests, and the collars
    // stand in the corners, the first one toward world +X.
    let low = slots(&body, &seen, |role| role == Role::Boundary(1))[0];
    assert_eq!(
        part_pose(&low, &seen).0.scale.x,
        2.0 * geometry::CAGE_BAR_HALF_WIDTH
    );
    assert_eq!(geometry::CAGE_BAR_HALF_WIDTH * 2.0, 0.8);
    let root = root_pose(&seen);
    let collars = shown(&body, &seen, |role| matches!(role, Role::Satellite(_)));
    assert_eq!(collars.len(), 5);
    let across = body.satellites.as_ref().unwrap().size[0] * e.radius;
    for (corner, collar) in collars.iter().enumerate() {
        let at = root.transform_point(collar.translation).xz() - AT;
        let angle = TAU * corner as f32 / 5.0;
        assert!(at.normalize().distance(Vec2::from_angle(angle)) < 1e-4);
        // Inset by its own half-extent, so it ends on the corner.
        assert!(at.length() <= e.radius && at.length() >= e.radius - across);
        // It faces the centre.
        let facing = (root.rotation * collar.rotation * Vec3::Z).xz();
        assert!(facing.dot(-at.normalize()).abs() > 0.99);
    }
    assert!((collars[0].translation.y - GROUND_LIFT).abs() < 1.0);
}

/// Rules E-5 and E-8: the plates of a wall stand upright on its bar, across the heading,
/// in equal slots, and the keystone has the middle one.
#[test]
fn the_plates_of_a_wall_stand_in_equal_slots_on_its_bar() {
    let body = target_body(SkillId::Northwall, K::ShieldWall);
    let e = effect(SkillId::Northwall, K::ShieldWall);
    assert_eq!(e.radius, 2.5);
    let seen = first(&e, 0.0);
    // Where a part stands: across the bar from its middle, and ahead of the hero.
    let stands = |body: &Body, seen: &Seen, role: Role| -> Vec<Vec2> {
        shown(body, seen, move |r| match (r, role) {
            (Role::Satellite(_), Role::Satellite(_)) => true,
            _ => r == role,
        })
        .iter()
        .map(|pose| pose.translation.xz())
        .collect()
    };
    let keystone = stands(&body, &seen, Role::Core);
    assert_eq!(keystone, [Vec2::new(0.0, geometry::WALL_AHEAD)]);
    let mut flanks: Vec<f32> = stands(&body, &seen, Role::Satellite(0))
        .iter()
        .map(|at| {
            assert!((at.y - geometry::WALL_AHEAD).abs() < 1e-5, "on the bar");
            at.x
        })
        .collect();
    flanks.sort_by(f32::total_cmp);
    // Five slots of one unit across a bar of five: the plates stand apart.
    for (flank, expected) in flanks.iter().zip([-2.0, -1.0, 1.0, 2.0]) {
        assert!((flank - expected).abs() < 1e-4, "{flanks:?}");
    }
    // Upright, facing along the heading, standing on the ground, inside the bar.
    for (role, points) in parts_in_world(&body, &seen) {
        if !role.authored() {
            continue;
        }
        let frame: Vec<Vec2> = points
            .iter()
            .map(|point| beside(point.xz(), AT, AT + HEADING_OF))
            .collect();
        let (near, far) = span(frame.iter().map(|at| at.y));
        assert!(far - near < 1e-4, "{role:?} is a plate across the heading");
        let (left, right) = span(frame.iter().map(|at| at.x));
        assert!(left >= -e.radius && right <= e.radius);
        let (low, high) = span(points.iter().map(|point| point.y));
        assert!((low - GROUND_LIFT).abs() < 1e-4 && high > 1.5, "{role:?}");
    }
    // Without a keystone the copies share the bar among themselves.
    let mut rank = body.clone();
    rank.core = None;
    let mut alone: Vec<f32> = stands(&rank, &seen, Role::Satellite(0))
        .iter()
        .map(|at| at.x)
        .collect();
    alone.sort_by(f32::total_cmp);
    for (plate, expected) in alone.iter().zip([-1.875, -0.625, 0.625, 1.875]) {
        assert!((plate - expected).abs() < 1e-4, "{alone:?}");
    }
    // An odd row has no middle slot to give: the keystone takes the one before it, and
    // no two parts share a slot.
    let mut odd = body.clone();
    odd.satellites.as_mut().unwrap().count = 3;
    let mut all: Vec<f32> = stands(&odd, &seen, Role::Satellite(0))
        .iter()
        .chain(&stands(&odd, &seen, Role::Core))
        .map(|at| at.x)
        .collect();
    all.sort_by(f32::total_cmp);
    for pair in all.windows(2) {
        assert!((pair[1] - pair[0] - 1.25).abs() < 1e-4, "{all:?}");
    }
    // A plate wider than its slot is kept inside the ends of the bar.
    let mut wide = body.clone();
    wide.satellites.as_mut().unwrap().size = [0.9, 0.5, 0.03];
    for at in stands(&wide, &seen, Role::Satellite(0)) {
        assert!(at.x.abs() + 0.45 * e.radius <= e.radius + 1e-4);
    }
    // Rule E-5: where the sizes of a flat part do not name its normal, it stands in a
    // wall and lies on the ground of a zone.
    let equal = part(Silhouette::Kite, [0.3, 0.3, 0.3], Behaviour::Steady);
    let mut plate = bare(Archetype::Wall);
    plate.core = Some(equal.clone());
    let standing = drawn(&slots(&plate, &seen, |role| role == Role::Core)[0], &seen).unwrap();
    let (near, far) = span(standing.iter().map(|point| point.z));
    assert!(far - near < 1e-5 && reach(&standing).y > 0.5);
    let mut tile = bare(Archetype::Zone);
    tile.core = Some(equal);
    let zone = effect(SkillId::DawnField, K::Field);
    let seen = first(&zone, 0.0);
    let lying = drawn(&slots(&tile, &seen, |role| role == Role::Core)[0], &seen).unwrap();
    let (low, high) = span(lying.iter().map(|point| point.y));
    assert!(high - low < 1e-5);
}

/// Rule E-7 and the two strip layouts: the core and the shell of a lane run the whole
/// received segment whatever its length, and the copies stand evenly along it.
#[test]
fn a_strip_spans_its_core_and_spaces_its_copies_along_the_segment() {
    let ray = target_body(SkillId::DawnRay, K::Beam);
    let fissure = target_body(SkillId::WinterDivide, K::BeamWarning);
    for length in [0.8, 6.0, 20.0, 45.0] {
        for (skill, kind, body) in [
            (SkillId::DawnRay, K::Beam, &ray),
            (SkillId::WinterDivide, K::BeamWarning, &fissure),
        ] {
            let e = with(&effect(skill, kind), |e| {
                e.end = (AT + HEADING_OF * length).to_array();
            });
            for now in [0.0, 0.23, 0.61, 1.9] {
                let seen = first(&e, now);
                for role in [Role::Core, Role::Shell] {
                    let slot = slots(body, &seen, |r| r == role)[0];
                    let points = drawn(&slot, &seen).unwrap();
                    // From the root to the far end, whatever the part breathes.
                    let (near, far) = span(points.iter().map(|point| point.z));
                    assert!(near.abs() < 1e-3 && (far - length).abs() < 1e-3, "{role:?}");
                    // As wide as authored at most, in multiples of the radius, around the
                    // centre line.
                    let (left, right) = span(points.iter().map(|point| point.x));
                    let half = slot.size.x * e.radius * 0.5;
                    assert!(left >= -half - 1e-4 && right <= half + 1e-4, "{role:?}");
                    assert!(right - left >= half * 1.4, "{role:?}");
                }
            }
        }
    }
    // `stagger`: eight copies an eighth of the strip apart, left and right in turn,
    // against the inside of the outline.
    let e = effect(SkillId::WinterDivide, K::BeamWarning);
    assert_eq!(
        (e.radius, Vec2::from_array(e.end).distance(AT)),
        (2.0, 20.0)
    );
    let seen = first(&e, 0.0);
    let fangs = shown(&fissure, &seen, |role| matches!(role, Role::Satellite(_)));
    assert_eq!(fangs.len(), 8);
    let inside = e.radius * (1.0 - ARC_BAND);
    for (index, fang) in fangs.iter().enumerate() {
        let at = fang.translation.xz();
        assert!((at.y - (index as f32 + 0.5) * 2.5).abs() < 1e-4);
        let side = if index % 2 == 0 { 1.0 } else { -1.0 };
        // A fang is 0.6 units across.
        assert!((at.x - side * (inside - 0.3)).abs() < 1e-4, "{at}");
    }
    for (index, count, side) in [
        (0, 8, 1.0),
        (7, 8, -1.0),
        (0, 5, 1.0),
        (1, 5, -1.0),
        (2, 5, 0.0),
        (3, 5, 1.0),
        (4, 5, -1.0),
        (0, 1, 0.0),
        (0, 2, 1.0),
        (1, 2, -1.0),
    ] {
        assert_eq!(stagger_side(index, count), side, "{index} of {count}");
    }
    // `line`: on the centre line.
    let warning = target_body(SkillId::HorizonWave, K::BeamWarning);
    let e = effect(SkillId::HorizonWave, K::BeamWarning);
    let seen = first(&e, 0.0);
    let marks = shown(&warning, &seen, |role| matches!(role, Role::Satellite(_)));
    assert_eq!(marks.len(), 6);
    for (index, mark) in marks.iter().enumerate() {
        let at = mark.translation.xz();
        assert!(at.x.abs() < 1e-5 && (at.y - (index as f32 + 0.5) * 7.5).abs() < 1e-3);
    }
    // In a strip shorter than its copies they stand in its middle, not beyond its ends.
    let short = with(&e, |e| e.end = (AT + HEADING_OF * 0.2).to_array());
    let seen = first(&short, 0.0);
    for mark in shown(&warning, &seen, |role| matches!(role, Role::Satellite(_))) {
        assert!((mark.translation.z - 0.1).abs() < 1e-5);
    }
}

/// Rule E-6: the flat core of a stance is posed half a radius ahead of its replicated
/// position along its replicated heading, upright, and still inside its ring.
#[test]
fn the_plate_of_a_stance_stands_half_a_radius_ahead() {
    let body = target_body(SkillId::MirrorGuard, K::Barrier);
    assert_eq!(
        (body.archetype, altitude(&body)),
        (Archetype::Prop, Altitude::Chest)
    );
    for radius in [0.8, 1.6] {
        let e = with(&effect(SkillId::MirrorGuard, K::Barrier), |e| {
            e.radius = radius
        });
        let seen = first(&e, 0.0);
        let plate = shown(&body, &seen, |role| role == Role::Core)[0];
        assert_eq!(plate.translation, Vec3::new(0.0, CHEST, radius * 0.5));
        let root = root_pose(&seen);
        let at = root.transform_point(plate.translation).xz();
        assert!(at.distance(AT + HEADING_OF * radius * 0.5) < 1e-5);
        // It faces along the heading and its corners stay inside the ring.
        let slot = slots(&body, &seen, |role| role == Role::Core)[0];
        let points = drawn(&slot, &seen).unwrap();
        let (near, far) = span(points.iter().map(|point| point.z));
        assert!(far - near < 1e-5);
        for point in points {
            assert!(point.xz().length() <= radius);
        }
    }
    // Only a flat part is a plate, and only a stance has one: a solid stays on the cast
    // position, and so does a flat part of any other prop.
    let e = effect(SkillId::MirrorGuard, K::Barrier);
    let seen = first(&e, 0.0);
    let mut solid = bare(Archetype::Prop);
    solid.core = Some(part(Silhouette::Ball, [0.4, 0.4, 0.4], Behaviour::Steady));
    assert_eq!(
        shown(&solid, &seen, |role| role == Role::Core)[0]
            .translation
            .xz(),
        Vec2::ZERO
    );
    let trap = effect(SkillId::WildTraps, K::Trap);
    let seen = first(&trap, 0.0);
    let mut flat = bare(Archetype::Prop);
    flat.core = Some(part(Silhouette::Kite, [0.5, 0.02, 0.5], Behaviour::Steady));
    assert_eq!(
        shown(&flat, &seen, |role| role == Role::Core)[0]
            .translation
            .xz(),
        Vec2::ZERO
    );
}

/// The copies of a `fan` stand across a cone, each turned along its own ray.
#[test]
fn a_fan_spreads_its_copies_across_the_cone() {
    let body = target_body(SkillId::FurnaceBreath, K::BeamWarning);
    let e = effect(SkillId::FurnaceBreath, K::BeamWarning);
    let half_angle = category::cone_half_angle(e.skill).unwrap();
    let seen = first(&e, 0.0);
    let tongues = shown(&body, &seen, |role| matches!(role, Role::Satellite(_)));
    assert_eq!(tongues.len(), 2);
    let angles: Vec<f32> = tongues
        .iter()
        .map(|pose| {
            let at = pose.translation.xz();
            assert!((at.length() - 7.0 * FAN_DISTANCE).abs() < 1e-4);
            let angle = at.x.atan2(at.y);
            // A flat part points along its own +X: outward, along its ray.
            let points = (pose.rotation * Vec3::X).xz();
            assert!(points.distance(at.normalize()) < 1e-4);
            angle
        })
        .collect();
    assert!((angles[0] + angles[1]).abs() < 1e-5, "one to each side");
    assert!((angles[1] - half_angle * FAN_SPREAD).abs() < 1e-4);
    // More copies fill the fan between the two; one copy stands on the axis.
    let mut many = body.clone();
    many.satellites.as_mut().unwrap().count = 5;
    let spread: Vec<f32> = shown(&many, &seen, |role| matches!(role, Role::Satellite(_)))
        .iter()
        .map(|pose| pose.translation.x.atan2(pose.translation.z))
        .collect();
    for pair in spread.windows(2) {
        assert!((pair[1] - pair[0] - half_angle * FAN_SPREAD * 0.5).abs() < 1e-4);
    }
    many.satellites.as_mut().unwrap().count = 1;
    let alone = shown(&many, &seen, |role| matches!(role, Role::Satellite(_)));
    assert!(alone[0].translation.x.abs() < 1e-6);
    // A copy too large for the cone is not drawn across its edge.
    let narrow = with(&e, |e| e.radius = 9.0);
    many.satellites.as_mut().unwrap().size = [1.0, 0.2, 1.0];
    assert!(
        shown(&many, &first(&narrow, 0.0), |role| matches!(
            role,
            Role::Satellite(_)
        ))
        .is_empty()
    );
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
    let flat = |size: [f32; 3], altitude| {
        normal_axis(Vec3::from_array(size), altitude != Altitude::Ground)
    };
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
            // `trail_scale` changes the size of a part across the path and nothing else:
            // not its place, and not the length of a ribbon between two sightings.
            let mut wide = body.clone();
            wide.trail_scale = Some(2.0);
            let scaled = shown(&wide, &seen, |role| matches!(role, Role::Trail(_)));
            assert_eq!(scaled.len(), poses.len());
            for (index, (pose, scaled)) in poses.iter().zip(&scaled).enumerate() {
                assert_eq!(pose.translation, scaled.translation);
                assert_eq!(pose.rotation, scaled.rotation);
                assert!((scaled.scale.x - 2.0 * pose.scale.x).abs() < 1e-5);
                let width = trail_width(trail, e.radius, 1.0);
                let fade = if trail == Trail::Links {
                    1.0
                } else {
                    1.0 - index as f32 * 0.22
                };
                assert!((pose.scale.x - width * fade).abs() < 1e-5, "{trail:?}");
                if trail == Trail::Ribbon {
                    assert_eq!(pose.scale.z, scaled.scale.z);
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
    // The engine's sizes for a radius, which a row scales: a ribbon of 0.45 at most.
    assert_eq!(trail_width(Trail::Ribbon, 2.0, 1.0), 0.45);
    assert_eq!(trail_width(Trail::Ribbon, 2.0, 2.0), 0.9);
    assert_eq!(trail_width(Trail::None, 2.0, 2.0), 0.0);
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
    assert_eq!(over_budget(&loads, 51), [] as [u8; 0]);
    // One part too many: the farthest body that has authored parts gives all of its own.
    assert_eq!(over_budget(&loads, 50), [2]);
    assert_eq!(over_budget(&loads, 41), [2]);
    assert_eq!(over_budget(&loads, 40), [2, 5]);
    assert_eq!(over_budget(&loads, 37), [2, 5]);
    assert_eq!(over_budget(&loads, 36), [2, 5, 3]);
    // Engine parts survive: when they alone are over the budget every authored part goes
    // and nothing else does.
    assert_eq!(over_budget(&loads, 3), [2, 5, 3, 1]);
    assert_eq!(over_budget(&loads, 0), [2, 5, 3, 1]);
    assert!(over_budget::<u8>(&[], 3).is_empty());

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
    let hidden = over_budget(&field, PART_BUDGET);
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
    assert_eq!(rows, 12);
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
                trail_scale: 1.0,
                marker: Marker::None,
                has_core: true,
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
                    trail_scale: 1.0,
                    marker: Marker::None,
                    has_core: true,
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
