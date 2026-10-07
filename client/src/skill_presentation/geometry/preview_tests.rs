//! The aim preview table, the pick rule and their parity with the in-process authority.
use super::*;
use crate::combat::aim_preview::movement_clip;
use crate::combat::standard::bounded_aim;
use common::balance::PLAYER_HIT_RADIUS;
use common::offline::{EPOCH, LOCAL_ADDR, PracticeSession};
use shared::HeroClass;
use shared::loadout::LoadoutState;
use shared::practice::PracticeCommand;
use shared::wire::{CharacterChoice, ClientPacket, ServerPacket};
use std::collections::BTreeMap;

/// The caster of the table and the way it aims.
const O: Vec2 = Vec2::new(2.0, 3.0);
const D: Vec2 = Vec2::new(0.6, 0.8);
/// How far the aim point of the table is from the caster, before the client bounds it.
const AIM: f32 = 5.0;

fn unclipped(_: Vec2, to: Vec2) -> Vec2 {
    to
}

/// A context in which the hero at `O` aims `AIM` units along `D`, as the client would
/// send it for the targeting of the skill.
fn context<'a>(
    def: &SkillDefinition,
    candidates: &'a [PickCandidate],
    effects: &'a [SkillEffectState],
) -> PreviewContext<'a> {
    PreviewContext {
        origin: O,
        aim: bounded_aim(
            O,
            O + D * AIM,
            def.ability.targeting,
            def.ability.cast_range,
        ),
        recast: false,
        orb: None,
        hero: 7,
        effects,
        candidates,
        clip: &unclipped,
    }
}

fn first_cast(id: SkillId) -> Preview {
    let def = skill(id);
    preview_shape(def, &context(def, &[], &[]))
}

fn recast(id: SkillId, effects: &[SkillEffectState]) -> Preview {
    let def = skill(id);
    preview_shape(
        def,
        &PreviewContext {
            recast: true,
            ..context(def, &[], effects)
        },
    )
}

fn near(a: Vec2, b: Vec2) -> bool {
    a.distance(b) < 1e-4
}

fn same_shape(a: &GeoShape, b: &GeoShape) -> bool {
    use GeoShape as G;
    let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
    match (*a, *b) {
        (
            G::Ring { center, radius },
            G::Ring {
                center: c,
                radius: r,
            },
        ) => near(center, c) && close(radius, r),
        (
            G::Capsule { from, to, radius },
            G::Capsule {
                from: f,
                to: t,
                radius: r,
            },
        ) => near(from, f) && near(to, t) && close(radius, r),
        (
            G::Lane {
                from,
                to,
                half_width,
            },
            G::Lane {
                from: f,
                to: t,
                half_width: h,
            },
        ) => near(from, f) && near(to, t) && close(half_width, h),
        (
            G::Sector {
                apex,
                axis,
                radius,
                half_angle,
            },
            G::Sector {
                apex: p,
                axis: x,
                radius: r,
                half_angle: h,
            },
        ) => near(apex, p) && near(axis, x) && close(radius, r) && close(half_angle, h),
        (
            G::Pentagon { center, radius },
            G::Pentagon {
                center: c,
                radius: r,
            },
        ) => near(center, c) && close(radius, r),
        (G::Segment { from, to }, G::Segment { from: f, to: t }) => near(from, f) && near(to, t),
        (G::None, G::None) => true,
        _ => false,
    }
}

fn same_mark(a: &PreviewMark, b: &PreviewMark) -> bool {
    use PreviewMark as M;
    match (*a, *b) {
        (M::Path { from, to }, M::Path { from: f, to: t })
        | (M::Push { from, to }, M::Push { from: f, to: t }) => near(from, f) && near(to, t),
        (M::Landing(at), M::Landing(other)) => near(at, other),
        (
            M::Picked { at, radius },
            M::Picked {
                at: other,
                radius: r,
            },
        ) => near(at, other) && (radius - r).abs() < 1e-4,
        _ => false,
    }
}

#[track_caller]
fn assert_areas(preview: &Preview, areas: &[GeoShape]) {
    assert!(
        preview.areas.len() == areas.len()
            && preview
                .areas
                .iter()
                .zip(areas)
                .all(|(drawn, expected)| same_shape(drawn, expected)),
        "{:?} is not {areas:?}",
        preview.areas
    );
}

#[track_caller]
fn assert_marks(preview: &Preview, marks: &[PreviewMark]) {
    assert!(
        preview.marks.len() == marks.len()
            && preview
                .marks
                .iter()
                .zip(marks)
                .all(|(drawn, expected)| same_mark(drawn, expected)),
        "{:?} is not {marks:?}",
        preview.marks
    );
}

#[track_caller]
fn assert_preview(
    preview: &Preview,
    shape: PreviewShape,
    areas: &[GeoShape],
    marks: &[PreviewMark],
    refused: bool,
) {
    assert_eq!(preview.shape, shape);
    assert_eq!(preview.refused, refused, "{preview:?}");
    assert_areas(preview, areas);
    assert_marks(preview, marks);
}

/// The shape and the numbers of the preview of every modular skill at its first cast, for
/// a hero at `O` that aims five units along `D`. The numbers are written out: a change of
/// the catalog or of a server literal has to be made here on purpose.
#[test]
fn the_preview_table_draws_the_server_rule_of_all_48_modular_skills() {
    use PreviewShape as P;
    let side = D.perp();
    // The bounded aim of a point skill of the given range.
    let at = |range: f32| O + D * AIM.min(range);
    let aim = at(AIM);
    let lane = |length: f32, half_width: f32| GeoShape::Lane {
        from: O,
        to: O + D * length,
        half_width,
    };
    let capsule = |length: f32, radius: f32| GeoShape::Capsule {
        from: O,
        to: O + D * length,
        radius,
    };
    let ring = |center: Vec2, radius: f32| GeoShape::Ring { center, radius };
    let sector = |radius: f32, cosine: f32| GeoShape::Sector {
        apex: O,
        axis: D,
        radius,
        half_angle: cosine.acos(),
    };
    let pick = ring(aim, 2.0);
    type Row = (&'static str, P, Vec<GeoShape>, Vec<PreviewMark>, bool);
    let table: Vec<Row> = vec![
        ("dawn_bind", P::Lane, vec![lane(18.0, 0.35)], vec![], false),
        (
            "dawn_barrier",
            P::Lane,
            vec![lane(15.0, 0.7)],
            vec![],
            false,
        ),
        (
            "dawn_field",
            P::PointRing,
            vec![ring(aim, 3.0)],
            vec![],
            false,
        ),
        (
            "dawn_ray",
            P::LaneCapsule,
            vec![capsule(45.0, 0.8)],
            vec![],
            false,
        ),
        ("wild_switch", P::None, vec![], vec![], false),
        ("wild_zap", P::Lane, vec![lane(24.0, 0.3)], vec![], false),
        (
            "wild_traps",
            P::TrapRow,
            vec![
                ring(aim - side * 1.5, 0.8),
                ring(aim, 0.8),
                ring(aim + side * 1.5, 0.8),
            ],
            vec![],
            false,
        ),
        (
            "wild_rocket",
            P::Lane,
            vec![lane(256.0, 1.05)],
            vec![],
            false,
        ),
        ("fault_line", P::Lane, vec![lane(15.0, 1.0)], vec![], false),
        (
            "furnace_breath",
            P::Sector,
            vec![sector(7.0, 0.6)],
            vec![],
            false,
        ),
        (
            "anvil_charge",
            P::DashLanding,
            vec![ring(O + D * 10.0, 3.0)],
            vec![PreviewMark::Path {
                from: O,
                to: O + D * 10.0,
            }],
            false,
        ),
        (
            "mountain_echo",
            P::Lane,
            vec![lane(28.0, 2.0)],
            vec![],
            false,
        ),
        (
            "edge_lunge",
            P::DashLanding,
            vec![ring(aim, 2.0)],
            vec![PreviewMark::Path { from: O, to: aim }],
            false,
        ),
        (
            "mirror_guard",
            P::Lane,
            vec![lane(10.0, 0.8)],
            vec![],
            false,
        ),
        ("twin_tempo", P::None, vec![], vec![], false),
        // No unit stands at the aim: a pick that needs one is refused.
        ("fourfold_duel", P::UnitPick, vec![pick], vec![], true),
        ("echo_strike", P::Lane, vec![lane(18.0, 0.5)], vec![], false),
        (
            "anchor_step",
            P::UnitPick,
            vec![pick],
            vec![PreviewMark::Landing(aim)],
            false,
        ),
        (
            "thunder_pulse",
            P::SelfRing,
            vec![ring(O, 5.0)],
            vec![],
            false,
        ),
        ("thunder_kick", P::PickThenLane, vec![pick], vec![], true),
        (
            "thorn_volley",
            P::Lane,
            vec![lane(13.0, 0.45)],
            vec![],
            false,
        ),
        ("patient_curse", P::UnitPick, vec![pick], vec![], true),
        ("shadow_lash", P::UnitPick, vec![pick], vec![], true),
        (
            "nightfall",
            P::Sector,
            vec![sector(8.0, 0.2)],
            vec![PreviewMark::Landing(O - D * 7.0)],
            false,
        ),
        (
            "wandering_ember",
            P::Lane,
            vec![lane(17.0, 0.6)],
            vec![],
            false,
        ),
        (
            "kindled_wisps",
            P::SelfRing,
            vec![ring(O, 9.0)],
            vec![],
            false,
        ),
        (
            "heart_tether",
            P::Lane,
            vec![lane(18.0, 0.5)],
            vec![],
            false,
        ),
        (
            "flame_dance",
            P::DashLanding,
            vec![ring(aim, 9.0)],
            vec![PreviewMark::Path { from: O, to: aim }],
            false,
        ),
        // Without a replicated orb the server puts it on the caster.
        (
            "orbital_command",
            P::EffectOriginLane,
            vec![GeoShape::Lane {
                from: O,
                to: aim,
                half_width: 0.65,
            }],
            vec![],
            false,
        ),
        (
            "orbital_field",
            P::PointRing,
            vec![ring(O, 4.0)],
            vec![],
            false,
        ),
        ("orbital_guard", P::UnitPick, vec![pick], vec![], true),
        (
            "orbital_collapse",
            P::PointRing,
            vec![ring(O, 5.0)],
            vec![],
            false,
        ),
        ("rift_needle", P::Lane, vec![lane(22.0, 0.4)], vec![], false),
        ("rift_seal", P::Lane, vec![lane(20.0, 0.65)], vec![], false),
        (
            "rift_step",
            P::BlinkLanding,
            vec![ring(aim, 9.0)],
            vec![PreviewMark::Landing(aim)],
            false,
        ),
        (
            "horizon_wave",
            P::LaneCapsule,
            vec![capsule(256.0, 1.2)],
            vec![],
            false,
        ),
        ("iron_hook", P::Lane, vec![lane(18.0, 0.5)], vec![], false),
        (
            "guiding_lantern",
            P::PointRing,
            vec![ring(aim, 2.0)],
            vec![],
            false,
        ),
        (
            "chain_sweep",
            P::SelfRing,
            vec![ring(O, 6.0)],
            vec![PreviewMark::Push {
                from: O,
                to: O + D * 3.0,
            }],
            false,
        ),
        (
            "iron_boundary",
            P::SelfPentagon,
            vec![GeoShape::Pentagon {
                center: O,
                radius: 5.0,
            }],
            vec![],
            false,
        ),
        (
            "winter_shard",
            P::Lane,
            vec![lane(18.0, 0.6)],
            vec![],
            false,
        ),
        ("sheltering_leap", P::UnitPick, vec![pick], vec![], true),
        (
            "northwall",
            P::WallAhead,
            vec![GeoShape::Segment {
                from: O + D - side * 2.5,
                to: O + D + side * 2.5,
            }],
            vec![],
            false,
        ),
        (
            "winter_divide",
            P::LaneCapsule,
            vec![capsule(20.0, 2.0)],
            vec![],
            false,
        ),
        // The corridor ends at the bounded aim and is 0.55 wide to each side, not 1.4.
        (
            "dagger_deadly_blow",
            P::LaneToPoint,
            vec![GeoShape::Lane {
                from: O,
                to: at(2.6),
                half_width: 0.55,
            }],
            vec![],
            false,
        ),
        (
            "dagger_bluff",
            P::LaneToPoint,
            vec![GeoShape::Lane {
                from: O,
                to: at(2.8),
                half_width: 0.55,
            }],
            vec![],
            false,
        ),
        (
            "dagger_backstab",
            P::LaneToPoint,
            vec![GeoShape::Lane {
                from: O,
                to: at(2.6),
                half_width: 0.55,
            }],
            vec![],
            false,
        ),
        (
            "dagger_lethal_blow",
            P::LaneToPoint,
            vec![GeoShape::Lane {
                from: O,
                to: at(2.6),
                half_width: 0.55,
            }],
            vec![],
            false,
        ),
    ];
    // One row for every skill of the catalog, in its order.
    assert_eq!(
        table.iter().map(|row| row.0).collect::<Vec<_>>(),
        SkillId::ALL.map(SkillId::id)
    );
    let mut uses: BTreeMap<P, usize> = BTreeMap::new();
    for (id, (name, shape, areas, marks, refused)) in SkillId::ALL.into_iter().zip(&table) {
        let preview = first_cast(id);
        // The row is named before its numbers are compared.
        assert_eq!((*name, preview.shape), (*name, *shape));
        assert_eq!((*name, preview.refused), (*name, *refused));
        assert!(
            preview.areas.len() == areas.len()
                && preview
                    .areas
                    .iter()
                    .zip(areas)
                    .all(|(a, b)| same_shape(a, b)),
            "{name}: {:?} is not {areas:?}",
            preview.areas
        );
        assert!(
            preview.marks.len() == marks.len()
                && preview
                    .marks
                    .iter()
                    .zip(marks)
                    .all(|(a, b)| same_mark(a, b)),
            "{name}: {:?} is not {marks:?}",
            preview.marks
        );
        assert_eq!(preview.pick, None, "{name}");
        assert_eq!(preview_kind(id, false), *shape, "{name}");
        *uses.entry(*shape).or_default() += 1;
    }
    // The first casts use every shape but the range ring, as often as the vocabulary says.
    assert_eq!(
        uses.into_iter().collect::<Vec<_>>(),
        [
            (P::None, 2),
            (P::Lane, 15),
            (P::LaneCapsule, 3),
            (P::LaneToPoint, 4),
            (P::PointRing, 4),
            (P::TrapRow, 1),
            (P::Sector, 2),
            (P::SelfRing, 3),
            (P::SelfPentagon, 1),
            (P::DashLanding, 3),
            (P::BlinkLanding, 1),
            (P::UnitPick, 6),
            (P::PickThenLane, 1),
            (P::EffectOriginLane, 1),
            (P::WallAhead, 1),
        ]
    );
}

fn own(id: SkillId, at: Vec2) -> SkillEffectState {
    SkillEffectState {
        id: 40,
        owner_id: 7,
        owner_team: shared::map::Team::Green,
        skill: id,
        kind: EffectVisualKind::Bolt,
        position: at.to_array(),
        end: (at + Vec2::X).to_array(),
        radius: 2.0,
        remaining_secs: 3.0,
        armed: true,
        consumed_segments: 0,
    }
}

/// While the replicated slot offers a recast the preview is the rule of that press.
#[test]
fn a_recast_window_previews_the_rule_of_the_recast() {
    use PreviewShape as P;
    // A press that ignores the aim previews nothing: the follow-up to the bound unit,
    // the sustain, the slow of the stored victims and the detonation of the field.
    for id in [
        SkillId::EchoStrike,
        SkillId::IronHook,
        SkillId::AnchorStep,
        SkillId::ThunderPulse,
        SkillId::DawnField,
    ] {
        assert_preview(&recast(id, &[]), P::None, &[], &[], false);
        assert_ne!(preview_kind(id, false), P::None, "{}", id.id());
    }
    // The spike recast strikes the nearest unit around the caster, wherever it aims.
    assert_preview(
        &recast(SkillId::ThornVolley, &[]),
        P::RangeRing,
        &[GeoShape::Ring {
            center: O,
            radius: 13.0,
        }],
        &[],
        false,
    );
    // The second dash of Flame Dance is the same dash.
    assert_eq!(
        recast(SkillId::FlameDance, &[]),
        first_cast(SkillId::FlameDance)
    );

    // The colossus is redirected from where it stands, along the hero's aim, and only
    // while the hero is inside the gate.
    let echo = SkillId::MountainEcho;
    let inside = O + Vec2::new(-2.4, 3.2);
    assert!((inside.distance(O) - RECAST_GATE_MOUNTAIN_ECHO).abs() < 1e-4);
    assert_preview(
        &recast(echo, &[own(echo, inside)]),
        P::EffectOriginLane,
        &[GeoShape::Lane {
            from: inside,
            to: inside + D * 28.0,
            half_width: 2.0,
        }],
        &[],
        false,
    );
    let outside = O + Vec2::new(-2.4, 3.2) * 1.01;
    let none = |effects: &[SkillEffectState]| {
        assert_preview(&recast(echo, effects), P::None, &[], &[], false);
    };
    none(&[own(echo, outside)]);
    none(&[]);
    // Another hero's colossus, an effect of another skill and an effect of a hidden
    // owner open nothing.
    none(&[SkillEffectState {
        owner_id: 8,
        ..own(echo, inside)
    }]);
    none(&[own(SkillId::FaultLine, inside)]);
    let unknown = skill(echo);
    let hidden = [SkillEffectState {
        owner_id: 0,
        ..own(echo, inside)
    }];
    let preview = preview_shape(
        unknown,
        &PreviewContext {
            recast: true,
            hero: 0,
            ..context(unknown, &[], &hidden)
        },
    );
    assert_eq!(preview.shape, P::None);
    // The server redirects its first effect of the skill, whichever opened the gate.
    let far = SkillEffectState {
        id: 12,
        ..own(echo, O + D * 20.0)
    };
    let preview = recast(echo, &[own(echo, inside), far.clone()]);
    assert!(same_shape(
        &preview.areas[0],
        &GeoShape::Lane {
            from: Vec2::from_array(far.position),
            to: Vec2::from_array(far.position) + D * 28.0,
            half_width: 2.0,
        }
    ));

    // Only these skills can report a recast; for the others the flag changes nothing.
    let mut shapes: Vec<P> = Vec::new();
    for id in SkillId::ALL {
        if super::super::category::has_recast(id) {
            shapes.push(preview_kind(id, true));
        } else {
            assert_eq!(
                preview_kind(id, true),
                preview_kind(id, false),
                "{}",
                id.id()
            );
        }
        shapes.push(preview_kind(id, false));
    }
    // Every preview shape of the vocabulary is the preview of some press.
    shapes.sort();
    shapes.dedup();
    assert_eq!(shapes, P::ALL);
}

/// A directional cast without a direction is dropped by the server: nothing is drawn.
#[test]
fn a_directional_cast_without_a_direction_previews_nothing() {
    for id in SkillId::ALL {
        let def = skill(id);
        let preview = preview_shape(
            def,
            &PreviewContext {
                aim: O,
                ..context(def, &[], &[])
            },
        );
        if def.ability.targeting == TargetingMode::Direction {
            assert!(
                preview.areas.is_empty() && preview.marks.is_empty(),
                "{}",
                id.id()
            );
            assert!(preview.refused, "{}", id.id());
        } else {
            // A point or self cast at the caster is legal and falls back to +Z.
            assert_eq!(preview.shape, preview_kind(id, false), "{}", id.id());
            for area in &preview.areas {
                assert!(
                    area.outline().iter().flatten().all(|at| at.is_finite()),
                    "{}",
                    id.id()
                );
            }
        }
    }
    // The three traps of a cast at the caster's feet lie across +Z, as the server lays them.
    let traps = skill(SkillId::WildTraps);
    let preview = preview_shape(
        traps,
        &PreviewContext {
            aim: O,
            ..context(traps, &[], &[])
        },
    );
    let centers: Vec<Vec2> = preview
        .areas
        .iter()
        .map(|area| match area {
            GeoShape::Ring { center, .. } => *center,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        centers,
        [O + Vec2::new(1.5, 0.0), O, O - Vec2::new(1.5, 0.0)]
    );
}

/// The field, the collapse and both orb orders are drawn from the replicated orb.
#[test]
fn orb_skills_are_previewed_from_the_replicated_orb() {
    let orb = Vec2::new(-4.0, 9.0);
    let with_orb = |id: SkillId, candidates: &[PickCandidate]| {
        let def = skill(id);
        preview_shape(
            def,
            &PreviewContext {
                orb: Some(orb),
                ..context(def, candidates, &[])
            },
        )
    };
    let aim = O + D * AIM;
    assert!(same_shape(
        &with_orb(SkillId::OrbitalCommand, &[]).areas[0],
        &GeoShape::Lane {
            from: orb,
            to: aim,
            half_width: 0.65
        }
    ));
    for (id, radius) in [
        (SkillId::OrbitalField, 4.0),
        (SkillId::OrbitalCollapse, 5.0),
    ] {
        assert_areas(
            &with_orb(id, &[]),
            &[GeoShape::Ring {
                center: orb,
                radius,
            }],
        );
    }
    // Orbital Guard: the pick ring at the aim and the path of the orb to the picked hero.
    let ally = candidate(TargetKind::Player, 9, aim + Vec2::new(0.5, 0.0), true);
    let preview = with_orb(SkillId::OrbitalGuard, &[ally]);
    assert_eq!((preview.pick, preview.refused), (Some(0), false));
    assert_areas(
        &preview,
        &[
            GeoShape::Ring {
                center: aim,
                radius: PICK_RADIUS,
            },
            GeoShape::Lane {
                from: orb,
                to: ally.position,
                half_width: ORB_RADIUS,
            },
        ],
    );
    // An allied minion nearer the aim is what the server picks, and it then refuses the
    // cast: the minion is marked and no path is drawn.
    let minion = candidate(TargetKind::Minion, 3, aim, true);
    let preview = with_orb(SkillId::OrbitalGuard, &[ally, minion]);
    assert_eq!((preview.pick, preview.refused), (Some(1), true));
    assert_eq!(preview.areas.len(), 1);
    assert_marks(
        &preview,
        &[PreviewMark::Picked {
            at: aim,
            radius: shared::MINION_TARGET_RADIUS,
        }],
    );
}

fn candidate(kind: TargetKind, id: u64, position: Vec2, ally: bool) -> PickCandidate {
    PickCandidate {
        kind,
        id,
        position,
        radius: match kind {
            TargetKind::Player => shared::PLAYER_TARGET_RADIUS,
            TargetKind::Minion => shared::MINION_TARGET_RADIUS,
            TargetKind::Neutral => shared::NEUTRAL_TARGET_RADIUS,
            TargetKind::Structure => shared::TOWER_TARGET_RADIUS,
        },
        ally,
    }
}

#[test]
fn server_pick_takes_the_nearest_legal_unit_within_both_reaches() {
    use TargetKind as K;
    let aim = Vec2::new(4.0, 0.0);
    let pick = |candidates: &[PickCandidate], ally: bool| {
        server_pick(candidates, aim, Vec2::ZERO, 5.0, ally)
    };
    let hero = candidate(K::Player, 20, aim + Vec2::new(1.0, 0.0), false);
    let minion = candidate(K::Minion, 5, aim + Vec2::new(0.0, 0.5), false);
    // The nearest to the aim wins, whatever its kind and whatever the order of the list.
    assert_eq!(pick(&[hero, minion], false), Some(1));
    assert_eq!(pick(&[minion, hero], false), Some(0));
    // A tie goes to the hero, then the minion, the structure and the neutral, then to
    // the lower id.
    let tie = |kind: K, id: u64| candidate(kind, id, aim + Vec2::new(0.0, 1.0), false);
    let tied = [
        tie(K::Neutral, 1),
        tie(K::Structure, 1),
        tie(K::Minion, 9),
        tie(K::Minion, 4),
        tie(K::Player, 30),
        tie(K::Player, 12),
    ];
    let mut order = Vec::new();
    let mut left = tied.to_vec();
    while let Some(index) = pick(&left, false) {
        order.push((left[index].kind, left[index].id));
        left.remove(index);
    }
    assert_eq!(
        order,
        [
            (K::Player, 12),
            (K::Player, 30),
            (K::Minion, 4),
            (K::Minion, 9),
            (K::Structure, 1),
            (K::Neutral, 1)
        ]
    );
    // The pick radius is measured to the unit's edge: 2.0 plus its target radius.
    let edge = |kind: K, gap: f32| {
        let unit = candidate(kind, 1, Vec2::ZERO, false);
        let unit = PickCandidate {
            position: aim - Vec2::X * (PICK_RADIUS + unit.radius + gap),
            ..unit
        };
        pick(&[unit], false)
    };
    for kind in [K::Player, K::Minion, K::Structure, K::Neutral] {
        assert_eq!(edge(kind, -0.01), Some(0), "{kind:?}");
        assert_eq!(edge(kind, 0.01), None, "{kind:?}");
    }
    // So is the cast range, from the caster.
    let reach = |gap: f32| {
        let unit = candidate(K::Player, 1, Vec2::X * (5.0 + 0.62 + gap), false);
        server_pick(&[unit], Vec2::X * 5.0, Vec2::ZERO, 5.0, false)
    };
    assert_eq!((reach(-0.01), reach(0.01)), (Some(0), None));

    // A hostile pick takes any unit that is not on the caster's team; an ally pick takes
    // heroes and minions of the team, the caster included, and never a structure.
    let units = [
        candidate(K::Structure, 2, aim, true),
        candidate(K::Player, 7, aim + Vec2::new(0.9, 0.0), true),
        candidate(K::Minion, 8, aim + Vec2::new(0.4, 0.0), true),
        candidate(K::Neutral, 3, aim + Vec2::new(0.2, 0.0), false),
        candidate(K::Structure, 4, aim + Vec2::new(0.0, 0.3), false),
    ];
    assert_eq!(pick(&units, true), Some(2));
    assert_eq!(pick(&units[..2], true), Some(1));
    assert_eq!(pick(&units[..1], true), None);
    assert_eq!(pick(&units, false), Some(3));
    assert_eq!(pick(&units[..3], false), None);
    assert_eq!(pick(&[], true), None);
}

#[test]
fn pick_rules_follow_the_server_gates() {
    let rule = |ally, hero_only, pick_optional| {
        Some(PickRule {
            ally,
            hero_only,
            pick_optional,
        })
    };
    for (id, expected) in [
        (SkillId::FourfoldDuel, rule(false, true, false)),
        (SkillId::PatientCurse, rule(false, true, false)),
        (SkillId::ShadowLash, rule(false, false, false)),
        (SkillId::ThunderKick, rule(false, false, false)),
        (SkillId::AnchorStep, rule(true, false, true)),
        (SkillId::ShelteringLeap, rule(true, false, false)),
        (SkillId::OrbitalGuard, rule(true, true, false)),
    ] {
        assert_eq!(pick_rule(id), expected, "{}", id.id());
    }
    // Exactly the skills whose preview is a pick have a rule.
    for id in SkillId::ALL {
        assert_eq!(
            pick_rule(id).is_some(),
            matches!(
                preview_kind(id, false),
                PreviewShape::UnitPick | PreviewShape::PickThenLane
            ),
            "{}",
            id.id()
        );
    }
}

/// The pick previews mark the unit the server rule takes and tell a refusal apart.
#[test]
fn a_pick_preview_marks_the_unit_and_says_when_the_cast_is_refused() {
    use TargetKind as K;
    let aim = O + D * AIM;
    let with = |id: SkillId, candidates: &[PickCandidate]| {
        let def = skill(id);
        preview_shape(def, &context(def, candidates, &[]))
    };
    let caster = candidate(K::Player, 7, O, true);
    let foe = candidate(K::Player, 21, aim + Vec2::new(0.8, 0.0), false);
    let creep = candidate(K::Minion, 4, aim + Vec2::new(0.0, 0.3), false);
    let marked = |unit: PickCandidate| PreviewMark::Picked {
        at: unit.position,
        radius: unit.radius,
    };

    // A duel and a curse need a hero: the nearer minion is what the server picks, so the
    // cast is refused and the minion is the unit that is marked.
    for id in [SkillId::FourfoldDuel, SkillId::PatientCurse] {
        let preview = with(id, &[caster, foe]);
        assert_eq!(
            (preview.pick, preview.refused),
            (Some(1), false),
            "{}",
            id.id()
        );
        assert_marks(&preview, &[marked(foe)]);
        let preview = with(id, &[caster, foe, creep]);
        assert_eq!(
            (preview.pick, preview.refused),
            (Some(2), true),
            "{}",
            id.id()
        );
        assert_marks(&preview, &[marked(creep)]);
    }
    // The lash takes any hostile unit, a structure included.
    let tower = candidate(K::Structure, 2, aim, false);
    let preview = with(SkillId::ShadowLash, &[caster, foe, creep, tower]);
    assert_eq!((preview.pick, preview.refused), (Some(3), false));
    // The caster's own team is never a hostile pick.
    let friend = candidate(K::Player, 9, aim, true);
    let preview = with(SkillId::ShadowLash, &[caster, friend]);
    assert_eq!((preview.pick, preview.refused), (None, true));

    // The kick carries the picked unit ten units away from the caster and strikes what
    // it passes with the catalog radius.
    let preview = with(SkillId::ThunderKick, &[caster, foe]);
    let away = (foe.position - O).normalize();
    assert_eq!(
        (preview.shape, preview.pick),
        (PreviewShape::PickThenLane, Some(1))
    );
    assert!(same_shape(
        &preview.areas[1],
        &GeoShape::Lane {
            from: foe.position,
            to: foe.position + away * KICK_LENGTH,
            half_width: 1.2
        }
    ));
    // A structure is hit and not moved: no lane.
    let preview = with(SkillId::ThunderKick, &[caster, tower]);
    assert_eq!((preview.pick, preview.refused), (Some(1), false));
    assert_eq!(preview.areas.len(), 1);

    // The leap of the Frostguard needs an allied hero or minion; he is one himself.
    let leap = skill(SkillId::ShelteringLeap);
    let at_self = PreviewContext {
        aim: O + Vec2::new(1.0, 0.0),
        ..context(leap, std::slice::from_ref(&caster), &[])
    };
    let preview = preview_shape(leap, &at_self);
    assert_eq!((preview.pick, preview.refused), (Some(0), false));
    assert_marks(&preview, &[marked(caster)]);
    let preview = with(SkillId::ShelteringLeap, &[caster, foe]);
    assert_eq!((preview.pick, preview.refused), (None, true));
    assert!(preview.marks.is_empty());

    // Anchor Step leaps to the aim point without a pick and to the picked ally with one;
    // it is never refused for its pick.
    let ward = candidate(K::Minion, 5, aim + Vec2::new(0.5, 0.5), true);
    let preview = with(SkillId::AnchorStep, &[caster, foe]);
    assert_eq!((preview.pick, preview.refused), (None, false));
    assert_marks(&preview, &[PreviewMark::Landing(aim)]);
    let preview = with(SkillId::AnchorStep, &[caster, ward]);
    assert_eq!((preview.pick, preview.refused), (Some(1), false));
    assert_marks(
        &preview,
        &[marked(ward), PreviewMark::Landing(ward.position)],
    );
}

/// Every ground move of a preview ends where the movement clip ends it.
#[test]
fn landings_and_pushed_lanes_end_where_the_move_is_clipped() {
    // A wall three units from wherever the move starts.
    let wall = |from: Vec2, to: Vec2| from + (to - from).clamp_length_max(3.0);
    let with = |id: SkillId, candidates: &[PickCandidate]| {
        let def = skill(id);
        preview_shape(
            def,
            &PreviewContext {
                clip: &wall,
                ..context(def, candidates, &[])
            },
        )
    };
    let stop = O + D * 3.0;
    for (id, radius) in [
        (SkillId::AnvilCharge, 3.0),
        (SkillId::EdgeLunge, 2.0),
        (SkillId::FlameDance, 9.0),
    ] {
        let preview = with(id, &[]);
        assert_marks(&preview, &[PreviewMark::Path { from: O, to: stop }]);
        assert_areas(
            &preview,
            &[GeoShape::Ring {
                center: stop,
                radius,
            }],
        );
    }
    assert_marks(
        &with(SkillId::Nightfall, &[]),
        &[PreviewMark::Landing(O - D * 3.0)],
    );
    assert_marks(
        &with(SkillId::AnchorStep, &[]),
        &[PreviewMark::Landing(stop)],
    );
    let foe = candidate(TargetKind::Player, 21, O + D * AIM, false);
    let preview = with(SkillId::ThunderKick, &[foe]);
    assert!(same_shape(
        &preview.areas[1],
        &GeoShape::Lane {
            from: foe.position,
            to: foe.position + D * 3.0,
            half_width: 1.2
        }
    ));
    // A blink is not clipped: the server drops the cast instead of shortening it.
    assert_marks(
        &with(SkillId::RiftStep, &[]),
        &[PreviewMark::Landing(O + D * AIM)],
    );
}

/// One hero of a class in the in-process authority and one enemy hero that stands
/// still. Nothing else lives, and the hero's team sees the whole map: the client offers
/// the rule only the units it sees.
struct Duel {
    session: PracticeSession,
    started: std::time::Instant,
    ticks: u32,
    slot: u8,
    request: u64,
    hero: u64,
    target: u64,
}

impl Duel {
    /// The hero stands at `origin` and the enemy `offset` away from it.
    fn new(class: HeroClass, id: SkillId, origin: Vec2, offset: Vec2) -> Self {
        let slot = shared::loadout::preset_for_class(class)
            .unwrap()
            .skills()
            .iter()
            .position(|skill| *skill == id)
            .unwrap() as u8;
        let started = std::time::Instant::now();
        let mut session = PracticeSession::new(started);
        session.command(ClientPacket::Join {
            handheld: Default::default(),
            prematch: false,
            team: shared::map::Team::Green,
            character: CharacterChoice::Ipfs,
            hero_class: class,
            avatar: None,
            sprite_character: None,
            session_id: None,
            passport_ticket: None,
        });
        for command in [PracticeCommand::ClearBots, PracticeCommand::SpawnDummy] {
            session.command(ClientPacket::Practice { command });
        }
        session.bots = Default::default();
        session.world.structures.clear();
        session.world.minions.clear();
        session.world.neutrals.clear();
        let caster = session.world.players.get_mut(&LOCAL_ADDR).unwrap();
        caster.modifiers.bypass_vision = true;
        caster.hero.x = origin.x;
        caster.hero.z = origin.y;
        let hero = caster.hero.identity.id;
        let target = session
            .world
            .players
            .values_mut()
            .find(|player| player.hero.identity.is_bot)
            .unwrap();
        target.hero.x = origin.x + offset.x;
        target.hero.z = origin.y + offset.y;
        let target = target.hero.identity.id;
        Self {
            session,
            started,
            ticks: 0,
            slot,
            request: 0,
            hero,
            target,
        }
    }

    /// The clock of the authority.
    fn now(&self) -> std::time::Instant {
        self.started + std::time::Duration::from_secs_f32(0.05) * self.ticks
    }

    fn position(&self, id: u64) -> Vec2 {
        let hero = &self
            .session
            .world
            .players
            .values()
            .find(|player| player.hero.identity.id == id)
            .unwrap()
            .hero;
        Vec2::new(hero.x, hero.z)
    }

    fn origin(&self) -> Vec2 {
        self.position(self.hero)
    }

    /// Casts the skill at `aim`, a point of the world.
    fn cast(&mut self, aim: Vec2) {
        self.request += 1;
        self.session.command(ClientPacket::CastSkill {
            slot: self.slot,
            aim: aim.to_array(),
            server_epoch: EPOCH,
            match_id: 1,
            request_id: self.request,
        });
    }

    fn advance(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.session.advance(0.05);
        }
        self.ticks += ticks;
    }

    fn hp(&self, id: u64) -> f32 {
        self.session
            .world
            .players
            .values()
            .find(|player| player.hero.identity.id == id)
            .unwrap()
            .hero
            .hp
    }

    /// What the authority replicates to the hero: its flags and the skill effects.
    fn replicated(&mut self) -> (LoadoutState, Vec<SkillEffectState>) {
        let ServerPacket::Snapshot {
            players,
            skill_effects,
            ..
        } = self.session.snapshot()
        else {
            panic!("practice publishes a snapshot");
        };
        let flags = players
            .into_iter()
            .find(|player| player.id == self.hero)
            .and_then(|player| player.loadout)
            .unwrap();
        (flags, skill_effects)
    }

    /// The two heroes as the client offers them to the pick rule.
    fn candidates(&self) -> [PickCandidate; 2] {
        [
            candidate(TargetKind::Player, self.hero, self.origin(), true),
            candidate(
                TargetKind::Player,
                self.target,
                self.position(self.target),
                false,
            ),
        ]
    }

    /// The preview of the skill for the hero as it stands, aimed at `aim`.
    fn preview(&mut self, id: SkillId, aim: Vec2) -> Preview {
        let (flags, effects) = self.replicated();
        let def = skill(id);
        let origin = self.origin();
        let clip = |from: Vec2, to: Vec2| movement_clip(&[], &[], from, to);
        preview_shape(
            def,
            &PreviewContext {
                origin,
                aim: bounded_aim(origin, aim, def.ability.targeting, def.ability.cast_range),
                recast: flags.slots[usize::from(self.slot)].can_recast,
                orb: flags.orb_position.map(Vec2::from_array),
                hero: self.hero,
                effects: &effects,
                candidates: &self.candidates(),
                clip: &clip,
            },
        )
    }
}

/// Whether the authority damages an enemy hero that stands `offset` from a hero of
/// `class` at the middle of the map when `id` is cast `aim` away from that hero.
fn strikes(class: HeroClass, id: SkillId, aim: Vec2, offset: Vec2) -> bool {
    let mut duel = Duel::new(class, id, Vec2::ZERO, offset);
    let full = duel.hp(duel.target);
    duel.cast(aim);
    duel.advance(30);
    duel.hp(duel.target) < full
}

/// A step that is clearly inside or outside a boundary and far smaller than a hero.
const MARGIN: f32 = 0.03;

/// Parity with the in-process authority for the pick ring: Shadow Lash strikes the hero
/// the preview marks and is dropped when the preview says so, at the edge of the pick
/// radius and at the edge of the cast range.
#[test]
fn the_pick_ring_takes_the_unit_the_authority_takes() {
    let id = SkillId::ShadowLash;
    let reach = PICK_RADIUS + PLAYER_HIT_RADIUS;
    let range = skill(id).ability.cast_range;
    assert_eq!((reach, range), (2.62, 5.0));
    for (offset, aim, hit) in [
        // The aim point is the pick radius plus the target's own radius from the hero.
        (Vec2::new(3.0, 0.0), Vec2::new(3.0, reach - MARGIN), true),
        (Vec2::new(3.0, 0.0), Vec2::new(3.0, reach + MARGIN), false),
        (Vec2::new(3.0, 0.0), Vec2::new(3.0, -reach + MARGIN), true),
        (Vec2::new(3.0, 0.0), Vec2::new(3.0, -reach - MARGIN), false),
        // The unit is the cast range plus its own radius from the caster.
        (
            Vec2::new(range + PLAYER_HIT_RADIUS - MARGIN, 0.0),
            Vec2::new(range, 0.0),
            true,
        ),
        (
            Vec2::new(range + PLAYER_HIT_RADIUS + MARGIN, 0.0),
            Vec2::new(range, 0.0),
            false,
        ),
    ] {
        assert_eq!(
            strikes(HeroClass::Veilstalker, id, aim, offset),
            hit,
            "{offset} {aim}"
        );
        let mut duel = Duel::new(HeroClass::Veilstalker, id, Vec2::ZERO, offset);
        let preview = duel.preview(id, aim);
        assert_eq!(preview.pick.is_some(), hit, "{offset} {aim}");
        assert_eq!(preview.refused, !hit, "{offset} {aim}");
        assert_areas(
            &preview,
            &[GeoShape::Ring {
                center: aim,
                radius: PICK_RADIUS,
            }],
        );
    }
}

/// Parity with the in-process authority for the ally pick: the caster is a unit of his
/// own team. Anchor Step stays where it is when its aim picks the caster and leaps to the
/// aim point when it picks nobody; Sheltering Leap is dropped without a pick.
#[test]
fn the_ally_pick_takes_the_caster_as_the_authority_does() {
    let reach = PICK_RADIUS + PLAYER_HIT_RADIUS;
    let far = Vec2::new(-12.0, 0.0);
    for (distance, picked) in [(reach - MARGIN, true), (reach + MARGIN, false)] {
        let aim = Vec2::new(0.0, distance);

        let id = SkillId::AnchorStep;
        let mut duel = Duel::new(HeroClass::Stormfist, id, Vec2::ZERO, far);
        let preview = duel.preview(id, aim);
        assert_eq!(preview.pick, picked.then_some(0), "{distance}");
        assert!(!preview.refused);
        let Some(PreviewMark::Landing(landing)) = preview.marks.last().copied() else {
            panic!("the leap has a landing: {preview:?}");
        };
        duel.cast(aim);
        duel.advance(1);
        // He lands where the preview said: on himself, or on the aim point.
        assert!(duel.origin().distance(landing) < 1e-3, "{distance}");
        assert!(near(landing, if picked { Vec2::ZERO } else { aim }));
        let (flags, effects) = duel.replicated();
        assert!(flags.shield_hp > 0.0);
        // The anchor is left only by a leap without a pick.
        assert_eq!(
            effects
                .iter()
                .any(|effect| effect.kind == EffectVisualKind::Anchor),
            !picked
        );

        let id = SkillId::ShelteringLeap;
        let mut duel = Duel::new(HeroClass::Frostguard, id, Vec2::ZERO, far);
        let preview = duel.preview(id, aim);
        assert_eq!(preview.pick, picked.then_some(0), "{distance}");
        assert_eq!(preview.refused, !picked, "{distance}");
        duel.cast(aim);
        duel.advance(1);
        assert_eq!(duel.replicated().0.shield_hp > 0.0, picked, "{distance}");
    }
}

/// Parity with the in-process authority for a hero-only pick: the duel is accepted on the
/// enemy hero at the edge of the pick ring and dropped just outside it.
#[test]
fn the_duel_challenges_the_hero_the_pick_ring_takes() {
    let id = SkillId::FourfoldDuel;
    let reach = PICK_RADIUS + PLAYER_HIT_RADIUS;
    let offset = Vec2::new(4.0, 0.0);
    for (aim, picked) in [
        (offset + Vec2::new(0.0, reach - MARGIN), true),
        (offset + Vec2::new(0.0, reach + MARGIN), false),
    ] {
        let mut duel = Duel::new(HeroClass::Edgeweaver, id, Vec2::ZERO, offset);
        let preview = duel.preview(id, aim);
        assert_eq!(
            (preview.pick, preview.refused),
            (picked.then_some(1), !picked)
        );
        duel.cast(aim);
        duel.advance(1);
        assert_eq!(
            duel.replicated().0.challenge_target,
            picked.then_some(duel.target)
        );
    }
}

/// Parity with the in-process authority for the sweep: Chain Sweep strikes a hero whose
/// edge is inside the previewed ring in every direction, whatever the aim, and moves it
/// the previewed push along the aim.
#[test]
fn the_sweep_ring_is_the_disc_the_authority_strikes() {
    let id = SkillId::ChainSweep;
    let aim = Vec2::new(4.0, 0.0);
    let mut duel = Duel::new(HeroClass::Chainkeeper, id, Vec2::ZERO, Vec2::new(3.0, 0.0));
    let preview = duel.preview(id, aim);
    let [GeoShape::Ring { center, radius }] = preview.areas[..] else {
        panic!("the sweep is one ring: {preview:?}");
    };
    assert!(near(center, Vec2::ZERO) && radius == 6.0);
    for degrees in [0.0_f32, 90.0, 180.0, 250.0] {
        let toward = Vec2::from_angle(degrees.to_radians());
        let edge = radius + PLAYER_HIT_RADIUS;
        assert!(
            strikes(HeroClass::Chainkeeper, id, aim, toward * (edge - MARGIN)),
            "{degrees}"
        );
        assert!(
            !strikes(HeroClass::Chainkeeper, id, aim, toward * (edge + MARGIN)),
            "{degrees}"
        );
    }
    // The push: three units along the aim, from wherever the unit stood.
    let [PreviewMark::Push { from, to }] = preview.marks[..] else {
        panic!("the sweep has a push arrow: {preview:?}");
    };
    assert!(near(to - from, Vec2::new(SWEEP_PUSH, 0.0)));
    let before = duel.position(duel.target);
    duel.cast(aim);
    duel.advance(1);
    assert!((duel.position(duel.target) - before).distance(to - from) < 1e-3);
}

/// Parity with the in-process authority for the retreat strike: Nightfall strikes a hero
/// a degree inside either edge of the previewed sector and at its outer edge, misses one
/// a degree outside, and carries the caster to the previewed landing.
#[test]
fn the_retreat_sector_and_landing_are_what_the_authority_applies() {
    let id = SkillId::Nightfall;
    let aim = Vec2::new(5.0, 0.0);
    let mut duel = Duel::new(HeroClass::Veilstalker, id, Vec2::ZERO, Vec2::new(4.0, 0.0));
    let preview = duel.preview(id, aim);
    let [
        GeoShape::Sector {
            apex,
            axis,
            radius,
            half_angle,
        },
    ] = preview.areas[..]
    else {
        panic!("the strike is one sector: {preview:?}");
    };
    assert!(near(apex, Vec2::ZERO) && near(axis, Vec2::X) && radius == 8.0);
    assert!((half_angle.cos() - NIGHTFALL_SECTOR_COS).abs() < 1e-6);
    let hits = |offset: Vec2| strikes(HeroClass::Veilstalker, id, aim, offset);
    let degree = 1.0_f32.to_radians();
    for side in [-1.0, 1.0] {
        assert!(hits(Vec2::from_angle(side * (half_angle - degree)) * 4.0));
        assert!(!hits(Vec2::from_angle(side * (half_angle + degree)) * 4.0));
    }
    let edge = radius + PLAYER_HIT_RADIUS;
    assert!(hits(Vec2::X * (edge - MARGIN)));
    assert!(!hits(Vec2::X * (edge + MARGIN)));
    assert!(!hits(Vec2::NEG_X * 2.0));

    let [PreviewMark::Landing(landing)] = preview.marks[..] else {
        panic!("the retreat has a landing: {preview:?}");
    };
    assert!(near(landing, Vec2::NEG_X * NIGHTFALL_RETREAT));
    duel.cast(aim);
    duel.advance(1);
    assert!(duel.origin().distance(landing) < 1e-3);
}

/// Parity with the in-process authority for the dagger corridor: a hero whose edge is
/// inside the previewed lane is struck, one just beside it is not, and the lane is the
/// clamp wide, not the catalog radius.
#[test]
fn the_dagger_corridor_is_the_clamp_the_authority_sweeps() {
    let id = SkillId::DaggerDeadlyBlow;
    let aim = Vec2::new(2.6, 0.0);
    let mut duel = Duel::new(HeroClass::Adventurer, id, Vec2::ZERO, Vec2::new(1.3, 0.0));
    let preview = duel.preview(id, aim);
    let [
        GeoShape::Lane {
            from,
            to,
            half_width,
        },
    ] = preview.areas[..]
    else {
        panic!("the corridor is one lane: {preview:?}");
    };
    assert!(near(from, Vec2::ZERO) && near(to, aim) && half_width == DAGGER_LANE_CLAMP);
    assert!(half_width < catalog_radius(id));
    let hits = |offset: Vec2| strikes(HeroClass::Adventurer, id, aim, offset);
    let beside = half_width + PLAYER_HIT_RADIUS;
    for side in [-1.0, 1.0] {
        assert!(hits(Vec2::new(1.3, side * (beside - MARGIN))), "{side}");
        assert!(!hits(Vec2::new(1.3, side * (beside + MARGIN))), "{side}");
    }
    // A shorter drag sweeps a shorter corridor: the hero beyond its end is out of reach.
    let short = Vec2::new(0.6, 0.0);
    let end = 0.6 + beside;
    assert!(strikes(
        HeroClass::Adventurer,
        id,
        short,
        Vec2::X * (end - MARGIN)
    ));
    assert!(!strikes(
        HeroClass::Adventurer,
        id,
        short,
        Vec2::X * (end + MARGIN)
    ));
}

/// Parity with the in-process authority for the kick: Thunder Kick carries the hero it
/// picks to the far end of the previewed lane.
#[test]
fn the_kick_lane_ends_where_the_authority_carries_the_unit() {
    let id = SkillId::ThunderKick;
    let offset = Vec2::new(1.8, 2.4);
    let mut duel = Duel::new(HeroClass::Stormfist, id, Vec2::ZERO, offset);
    let preview = duel.preview(id, offset);
    assert_eq!(preview.pick, Some(1));
    let GeoShape::Lane {
        from,
        to,
        half_width,
    } = preview.areas[1]
    else {
        panic!("the kick has a lane: {preview:?}");
    };
    assert!(near(from, offset) && half_width == 1.2);
    assert!((from.distance(to) - KICK_LENGTH).abs() < 1e-4);
    // Three units out, ten more along the same line.
    assert!(near(to, Vec2::new(7.8, 10.4)));
    duel.cast(offset);
    duel.advance(1);
    assert!(duel.position(duel.target).distance(to) < 1e-3);
}

/// Parity with the in-process authority for a dash landing: Anvil Charge stops where the
/// previewed path stops, also when the forest cuts the charge short.
#[test]
fn the_dash_landing_is_where_the_authority_stops_the_charge() {
    let id = SkillId::AnvilCharge;
    let map = shared::navigation::world_navigation();
    // Walk out from the middle of the map until the forest is within reach of a charge.
    let origin = (1..80)
        .map(|step| Vec2::X * step as f32)
        .find(|at| {
            map.point_clear(at.to_array()) && !map.point_clear_with_radius(at.to_array(), 6.0)
        })
        .expect("the forest is somewhere along +X");
    let mut clipped = 0;
    for step in 0..16 {
        let toward = Vec2::from_angle(step as f32 * std::f32::consts::TAU / 16.0);
        let mut duel = Duel::new(HeroClass::Cinderforge, id, origin, Vec2::new(0.0, 40.0));
        let preview = duel.preview(id, origin + toward * 5.0);
        let [PreviewMark::Path { from, to }] = preview.marks[..] else {
            panic!("the charge has a path: {preview:?}");
        };
        assert!(near(from, origin));
        assert_areas(
            &preview,
            &[GeoShape::Ring {
                center: to,
                radius: 3.0,
            }],
        );
        duel.cast(origin + toward * 5.0);
        duel.advance(1);
        assert!(duel.origin().distance(to) < 1e-3, "{step}: {to}");
        clipped += usize::from(from.distance(to) < 9.9);
    }
    // Both cases were seen: a full charge and one the forest stopped.
    assert!((1..16).contains(&clipped), "{clipped}");
}

/// Parity with the in-process authority for the recast gate: Mountain Echo is redirected
/// when the hero is within the gate of the colossus and dropped just outside it, and the
/// previewed lane starts at the colossus.
#[test]
fn the_colossus_recast_is_previewed_only_inside_the_gate_the_authority_keeps() {
    let id = SkillId::MountainEcho;
    for (gap, accepted) in [(-MARGIN, true), (MARGIN, false)] {
        let mut duel = Duel::new(
            HeroClass::Cinderforge,
            id,
            Vec2::ZERO,
            Vec2::new(0.0, -40.0),
        );
        duel.cast(Vec2::new(5.0, 0.0));
        duel.advance(10);
        let (flags, effects) = duel.replicated();
        assert!(flags.slots[usize::from(duel.slot)].can_recast);
        let body = effects
            .iter()
            .find(|effect| effect.skill == id)
            .expect("the colossus is replicated");
        let body = Vec2::from_array(body.position);
        // The hero stands the gate from the colossus, a step inside or outside it.
        let stand = body + Vec2::new(0.0, RECAST_GATE_MOUNTAIN_ECHO + gap);
        let hero = duel.session.world.players.get_mut(&LOCAL_ADDR).unwrap();
        (hero.hero.x, hero.hero.z) = (stand.x, stand.y);
        let aim = stand + Vec2::new(0.0, -6.0);
        assert_eq!(
            super::super::status::recast_in_reach(id, duel.hero, stand, &effects),
            accepted
        );
        let preview = duel.preview(id, aim);
        if accepted {
            assert_eq!(preview.shape, PreviewShape::EffectOriginLane);
            assert!(same_shape(
                &preview.areas[0],
                &GeoShape::Lane {
                    from: body,
                    to: body + Vec2::NEG_Y * 28.0,
                    half_width: 2.0
                }
            ));
        } else {
            assert_eq!(preview.shape, PreviewShape::None);
            assert!(preview.areas.is_empty());
        }
        duel.cast(aim);
        // An accepted recast spends its one use.
        let (flags, _) = duel.replicated();
        assert_eq!(!flags.slots[usize::from(duel.slot)].can_recast, accepted);
        // And the colossus leaves along the hero's aim instead of coming home along -X.
        duel.advance(4);
        let (_, effects) = duel.replicated();
        let moved = Vec2::from_array(
            effects
                .iter()
                .find(|effect| effect.skill == id)
                .expect("the colossus is still on its way")
                .position,
        ) - body;
        if accepted {
            assert!(moved.x.abs() < 1e-3 && moved.y < -2.0, "{moved}");
        } else {
            assert!(moved.y.abs() < 1e-3 && moved.x < -2.0, "{moved}");
        }
    }
}

/// The spike recast: the preview is the ring the authority searches, around the caster.
#[test]
fn the_spike_recast_strikes_inside_the_range_ring_whatever_the_aim() {
    let id = SkillId::ThornVolley;
    for (gap, hit) in [(-MARGIN, true), (MARGIN, false)] {
        // The enemy stands behind the hero, away from both aims.
        let offset = Vec2::new(-(13.0 + PLAYER_HIT_RADIUS + gap), 0.0);
        let mut duel = Duel::new(HeroClass::Veilstalker, id, Vec2::ZERO, offset);
        let aim = Vec2::new(0.0, 6.0);
        assert_eq!(duel.preview(id, aim).shape, PreviewShape::Lane);
        duel.cast(aim);
        duel.advance(4);
        let preview = duel.preview(id, aim);
        assert_eq!(preview.shape, PreviewShape::RangeRing);
        assert_areas(
            &preview,
            &[GeoShape::Ring {
                center: Vec2::ZERO,
                radius: 13.0,
            }],
        );
        let full = duel.hp(duel.target);
        duel.cast(aim);
        duel.advance(1);
        assert_eq!(duel.hp(duel.target) < full, hit, "{gap}");
    }
}

/// Parity with the in-process authority for the breath: the previewed sector is the cone
/// the replicated warning draws, and the authority strikes a degree inside either edge
/// of it and at its outer edge.
#[test]
fn the_breath_sector_is_the_cone_the_authority_fires() {
    let id = SkillId::FurnaceBreath;
    let aim = Vec2::new(5.0, 0.0);
    let mut duel = Duel::new(HeroClass::Cinderforge, id, Vec2::ZERO, Vec2::new(4.0, 0.0));
    let preview = duel.preview(id, aim);
    let [
        GeoShape::Sector {
            apex,
            axis,
            radius,
            half_angle,
        },
    ] = preview.areas[..]
    else {
        panic!("the breath is one sector: {preview:?}");
    };
    assert!(near(apex, Vec2::ZERO) && near(axis, Vec2::X) && radius == 7.0);
    assert!((half_angle.cos() - FURNACE_CONE_COS).abs() < 1e-6);
    duel.cast(aim);
    duel.advance(1);
    let (_, effects) = duel.replicated();
    let warning = effects
        .iter()
        .find(|effect| effect.skill == id)
        .expect("the warning is replicated");
    assert!(same_shape(
        &boundary_shape(id, warning.kind, warning),
        &preview.areas[0]
    ));
    let hits = |offset: Vec2| strikes(HeroClass::Cinderforge, id, aim, offset);
    let degree = 1.0_f32.to_radians();
    for side in [-1.0, 1.0] {
        assert!(hits(Vec2::from_angle(side * (half_angle - degree)) * 4.0));
        assert!(!hits(Vec2::from_angle(side * (half_angle + degree)) * 4.0));
    }
    let edge = radius + PLAYER_HIT_RADIUS;
    assert!(hits(Vec2::X * (edge - MARGIN)));
    assert!(!hits(Vec2::X * (edge + MARGIN)));
}

/// Parity with the in-process authority for the wall: Northwall takes a hostile
/// projectile that reaches the previewed bar, one unit ahead of its owner, within the
/// half-length of the bar.
#[test]
fn the_wall_bar_is_where_the_authority_intercepts() {
    let id = SkillId::Northwall;
    let aim = Vec2::new(6.0, 0.0);
    let mut duel = Duel::new(HeroClass::Frostguard, id, Vec2::ZERO, Vec2::new(0.0, 30.0));
    let preview = duel.preview(id, aim);
    let [GeoShape::Segment { from, to }] = preview.areas[..] else {
        panic!("the wall is one bar: {preview:?}");
    };
    assert!(near(from, Vec2::new(WALL_AHEAD, -2.5)) && near(to, Vec2::new(WALL_AHEAD, 2.5)));
    duel.cast(aim);
    duel.advance(1);
    let now = duel.now();
    // A projectile of the other team, as wide as a bolt, on its way from `start` to `end`.
    let width = 0.3;
    let mut taken = |start: Vec2, end: Vec2| {
        common::skills::advanced::intercept_players(
            &mut duel.session.world.players,
            shared::map::Team::Blue,
            start.to_array(),
            end.to_array(),
            width,
            now,
        )
        .is_some()
    };
    // It is taken when its edge reaches the bar, not before.
    let ahead = from.x + width;
    assert!(taken(Vec2::new(5.0, 0.0), Vec2::new(ahead - MARGIN, 0.0)));
    assert!(!taken(Vec2::new(5.0, 0.0), Vec2::new(ahead + MARGIN, 0.0)));
    // And only beside the bar: its half-length plus the projectile's own radius.
    for end in [from, to] {
        let beside = end.y.signum() * (end.y.abs() + width);
        let pass = |y: f32| (Vec2::new(5.0, y), Vec2::new(0.5, y));
        let (start, stop) = pass(beside - end.y.signum() * MARGIN);
        assert!(taken(start, stop), "{end}");
        let (start, stop) = pass(beside + end.y.signum() * MARGIN);
        assert!(!taken(start, stop), "{end}");
    }
    // Nothing is taken from behind the wall or on its way out.
    assert!(!taken(Vec2::new(-5.0, 0.0), Vec2::new(-0.5, 0.0)));
    assert!(!taken(Vec2::new(0.5, 0.0), Vec2::new(5.0, 0.0)));
}
