use super::*;
use crate::combat::standard::bounded_aim;
use crate::skill_presentation::geometry::{ORB_RADIUS, PICK_RADIUS, preview_shape};
use shared::loadout::{SkillEffectState, SkillId, skill};

const HERO: Vec2 = Vec2::new(3.0, -2.0);
const WAY: Vec2 = Vec2::new(0.0, 1.0);

fn unclipped(_: Vec2, to: Vec2) -> Vec2 {
    to
}

/// What the hero at `HERO` knows when it aims four units along `WAY`.
fn context<'a>(id: SkillId, candidates: &'a [PickCandidate]) -> PreviewContext<'a> {
    let def = skill(id);
    PreviewContext {
        origin: HERO,
        aim: bounded_aim(
            HERO,
            HERO + WAY * 4.0,
            def.ability.targeting,
            def.ability.cast_range,
        ),
        recast: false,
        orb: None,
        hero: 7,
        effects: &[],
        candidates,
        clip: &unclipped,
    }
}

fn hero(id: u64, position: Vec2, ally: bool) -> PickCandidate {
    PickCandidate {
        kind: TargetKind::Player,
        id,
        position,
        radius: shared::PLAYER_TARGET_RADIUS,
        ally,
    }
}

/// The strokes of one ink.
fn lines(strokes: &[Stroke], ink: Ink) -> Vec<&Vec<Vec2>> {
    strokes
        .iter()
        .filter(|stroke| stroke.ink == ink)
        .map(|stroke| &stroke.points)
        .collect()
}

/// A closed line whose points are all `radius` from `center`.
fn is_ring(points: &[Vec2], center: Vec2, radius: f32) -> bool {
    points.len() > 8
        && points[0].distance(*points.last().unwrap()) < 1e-4
        && points
            .iter()
            .all(|at| (at.distance(center) - radius).abs() < 1e-4)
}

#[test]
fn thorn_recast_preview_is_the_caster_range_ring() {
    let id = SkillId::ThornVolley;
    // The first cast is a spike along the aim: two rails of the real half-width and an
    // arrowhead.
    let first = strokes(&preview_shape(skill(id), &context(id, &[])));
    assert_eq!(first.len(), 4);
    for (rail, side) in first[..2].iter().zip([-1.0, 1.0]) {
        let beside = HERO + WAY.perp() * 0.45 * side;
        assert_eq!(rail.ink, Ink::Area);
        assert!(rail.points[0].distance(beside) < 1e-4);
        assert!(rail.points[1].distance(beside + WAY * 13.0) < 1e-4);
    }
    // While the slot offers its recast the next press strikes the nearest unit around
    // the caster, wherever it is aimed: the ring of the cast range and nothing along
    // the aim.
    for aim in [
        HERO + WAY * 4.0,
        HERO - WAY * 9.0,
        HERO + Vec2::new(30.0, 2.0),
    ] {
        let preview = preview_shape(
            skill(id),
            &PreviewContext {
                recast: true,
                aim,
                ..context(id, &[])
            },
        );
        assert_eq!(preview.shape, PreviewShape::RangeRing);
        let recast = strokes(&preview);
        assert_eq!(recast.len(), 1);
        assert_eq!(recast[0].ink, Ink::Area);
        assert!(is_ring(&recast[0].points, HERO, 13.0));
        assert_eq!(color(Ink::Area, preview.refused), color(Ink::Area, false));
    }
}

#[test]
fn orbital_guard_preview_starts_at_the_orb() {
    let id = SkillId::OrbitalGuard;
    let orb = Vec2::new(-5.0, 4.0);
    let aim = HERO + WAY * 4.0;
    let ally = hero(9, aim + Vec2::new(0.6, 0.0), true);
    let caster = hero(7, HERO, true);
    let units = [caster, ally];
    let preview = preview_shape(
        skill(id),
        &PreviewContext {
            orb: Some(orb),
            ..context(id, &units)
        },
    );
    assert!(!preview.refused);
    let drawn = strokes(&preview);
    let areas = lines(&drawn, Ink::Area);
    assert_eq!(areas.len(), 2);
    // The pick ring stands at the aim point.
    assert!(is_ring(areas[0], aim, PICK_RADIUS));
    // The path of the orb: a closed strip of the orb's radius from the orb to the
    // picked hero. Nothing starts at the caster.
    let along = (ally.position - orb).normalize();
    let side = along.perp() * ORB_RADIUS;
    let corners = [
        orb + side,
        ally.position + side,
        ally.position - side,
        orb - side,
        orb + side,
    ];
    assert_eq!(areas[1].len(), 5);
    for (drawn, corner) in areas[1].iter().zip(corners) {
        assert!(drawn.distance(corner) < 1e-4, "{drawn} {corner}");
    }
    assert!(areas[1].iter().all(|at| at.distance(HERO) > 2.0));
    // The picked hero is ringed twice, clear of its body.
    let marks = lines(&drawn, Ink::Mark);
    assert_eq!(marks.len(), 2);
    for (mark, clearance) in marks.iter().zip(PICK_RINGS) {
        assert!(is_ring(
            mark,
            ally.position,
            shared::PLAYER_TARGET_RADIUS + clearance
        ));
    }

    // Without a hero to guard the server drops the cast: the ring alone, in the colour
    // of a refusal, and no path.
    let preview = preview_shape(
        skill(id),
        &PreviewContext {
            orb: Some(orb),
            ..context(id, std::slice::from_ref(&caster))
        },
    );
    assert!(preview.refused);
    let drawn = strokes(&preview);
    assert_eq!(drawn.len(), 1);
    assert!(is_ring(&drawn[0].points, aim, PICK_RADIUS));
    assert_ne!(color(Ink::Area, true), color(Ink::Area, false));

    // The order of the orb to a point is drawn from the orb as well.
    let command = strokes(&preview_shape(
        skill(SkillId::OrbitalCommand),
        &PreviewContext {
            orb: Some(orb),
            ..context(SkillId::OrbitalCommand, &units)
        },
    ));
    assert_eq!(command.len(), 1);
    assert!(command[0].points[0].distance(orb) < ORB_RADIUS + 1e-4);
    assert!(command[0].points[1].distance(aim) < ORB_RADIUS + 1e-4);
}

/// A lane from the caster looks as it did: two open rails and an arrowhead that stays in
/// view however long the lane is.
#[test]
fn a_lane_keeps_its_open_rails_and_its_arrowhead() {
    for (id, length, half_width) in [
        (SkillId::DawnBind, 18.0, 0.35),
        (SkillId::WildRocket, 256.0, 1.05),
        (SkillId::MirrorGuard, 10.0, 0.8),
    ] {
        let preview = preview_shape(skill(id), &context(id, &[]));
        let side = WAY.perp() * half_width;
        let tip = HERO + WAY * f32::min(length, 14.0);
        let wing = WAY.perp() * half_width.max(0.5);
        let expected = [
            vec![HERO - side, HERO + WAY * length - side],
            vec![HERO + side, HERO + WAY * length + side],
            vec![tip - WAY * 1.5 - wing, tip],
            vec![tip - WAY * 1.5 + wing, tip],
        ];
        let drawn = strokes(&preview);
        assert_eq!(drawn.len(), expected.len(), "{}", id.id());
        for (stroke, expected) in drawn.iter().zip(&expected) {
            assert_eq!(stroke.ink, Ink::Area);
            assert_eq!(stroke.points.len(), 2);
            for (at, expected) in stroke.points.iter().zip(expected) {
                assert!(at.distance(*expected) < 1e-3, "{} {at} {expected}", id.id());
            }
        }
    }
    // A capsule is closed around both ends and keeps the arrowhead.
    let preview = preview_shape(
        skill(SkillId::WinterDivide),
        &context(SkillId::WinterDivide, &[]),
    );
    let drawn = strokes(&preview);
    assert_eq!(drawn.len(), 3);
    let outline = &drawn[0].points;
    assert!(outline[0].distance(*outline.last().unwrap()) < 1e-4);
    for tip in [HERO - WAY * 2.0, HERO + WAY * 22.0] {
        assert!(outline.iter().any(|at| at.distance(tip) < 1e-3));
    }
}

/// The areas of a preview are outlined exactly as the table derived them, and the aids
/// stay where they belong.
#[test]
fn every_preview_is_drawn_from_its_areas_and_marks_alone() {
    let caster = hero(7, HERO, true);
    let foe = hero(21, HERO + WAY * 4.0 + Vec2::new(0.5, 0.0), false);
    let units = [caster, foe];
    let echo = SkillEffectState {
        id: 40,
        owner_id: 7,
        owner_team: shared::map::Team::Green,
        skill: SkillId::MountainEcho,
        kind: shared::loadout::EffectVisualKind::Bolt,
        position: (HERO + Vec2::new(2.0, 1.0)).to_array(),
        end: HERO.to_array(),
        radius: 2.0,
        remaining_secs: 3.0,
        armed: true,
        consumed_segments: 0,
    };
    let mut seen = std::collections::BTreeSet::new();
    for id in SkillId::ALL {
        for recast in [false, true] {
            let preview = preview_shape(
                skill(id),
                &PreviewContext {
                    recast,
                    effects: std::slice::from_ref(&echo),
                    ..context(id, &units)
                },
            );
            seen.insert(preview.shape);
            let drawn = strokes(&preview);
            for stroke in &drawn {
                assert!(stroke.points.len() >= 2, "{}", id.id());
                assert!(stroke.points.iter().all(|at| at.is_finite()), "{}", id.id());
            }
            assert_eq!(drawn.is_empty(), preview.areas.is_empty(), "{}", id.id());
            // Every outline of every area is drawn, point for point; a lane or a
            // capsule from the caster adds only its arrowhead.
            let areas = lines(&drawn, Ink::Area);
            let heads = match preview.shape {
                PreviewShape::Lane | PreviewShape::LaneCapsule => 2,
                _ => 0,
            };
            let outlines: Vec<Vec<Vec2>> = preview
                .areas
                .iter()
                .flat_map(|area| match (preview.shape, area) {
                    (PreviewShape::Lane, GeoShape::Lane { .. }) => {
                        let closed = area.outline().remove(0);
                        vec![vec![closed[3], closed[2]], vec![closed[0], closed[1]]]
                    }
                    _ => area.outline(),
                })
                .collect();
            assert_eq!(areas.len(), outlines.len() + heads, "{}", id.id());
            for (drawn, outline) in areas.iter().zip(&outlines) {
                assert_eq!(*drawn, outline, "{}", id.id());
            }
            // A landing is a small mark around its point; a path and a push are single
            // lines between their two points, the push with an arrowhead.
            let aids = lines(&drawn, Ink::Aid);
            let marks = lines(&drawn, Ink::Mark);
            let (mut aid_lines, mut mark_lines) = (0, 0);
            for mark in &preview.marks {
                match *mark {
                    PreviewMark::Path { from, to } => {
                        assert!(aids.contains(&&vec![from, to]));
                        aid_lines += 1;
                    }
                    PreviewMark::Push { from, to } => {
                        assert!(aids.contains(&&vec![from, to]));
                        assert!((from.distance(to) - geometry::SWEEP_PUSH).abs() < 1e-4);
                        aid_lines += 3;
                    }
                    PreviewMark::Landing(at) => {
                        let near = marks
                            .iter()
                            .filter(|line| line.iter().all(|p| p.distance(at) <= LANDING + 1e-4))
                            .count();
                        assert_eq!(near, 3, "{}", id.id());
                        mark_lines += 3;
                    }
                    PreviewMark::Picked { at, radius } => {
                        for clearance in PICK_RINGS {
                            assert!(
                                marks
                                    .iter()
                                    .any(|line| is_ring(line, at, radius + clearance))
                            );
                        }
                        mark_lines += 2;
                    }
                }
            }
            assert_eq!(
                (aids.len(), marks.len()),
                (aid_lines, mark_lines),
                "{}",
                id.id()
            );
        }
    }
    // The loop drew every shape of the vocabulary.
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), PreviewShape::ALL);
}

#[test]
fn a_refused_cast_is_painted_in_the_refusal_colour_and_aids_are_faint() {
    for ink in [Ink::Area, Ink::Aid, Ink::Mark] {
        let (open, refused) = (color(ink, false), color(ink, true));
        assert_ne!(open, refused);
        // Blue leads the aim colour and red the refusal.
        let (open, refused) = (open.to_linear(), refused.to_linear());
        assert!(
            open.blue > open.red && refused.red > refused.blue,
            "{ink:?}"
        );
        assert_eq!(open.alpha < 1.0, ink == Ink::Aid);
        assert_eq!(refused.alpha < 1.0, ink == Ink::Aid);
    }
    assert_ne!(color(Ink::Area, false), color(Ink::Mark, false));
}

#[test]
fn only_a_long_lane_from_the_caster_reaches_the_minimap() {
    let vector = |id: SkillId| minimap_vector(&preview_shape(skill(id), &context(id, &[])));
    for (id, length, radius) in [
        (SkillId::DawnRay, 45.0, 0.8),
        (SkillId::WildRocket, 256.0, 1.05),
        (SkillId::HorizonWave, 256.0, 1.2),
    ] {
        let (from, to, width) = vector(id).unwrap();
        assert!(from.distance(HERO) < 1e-4, "{}", id.id());
        assert!(to.distance(HERO + WAY * length) < 1e-3, "{}", id.id());
        assert_eq!(width, radius);
    }
    // As before, a lane shorter than 35 units stays off the map, and so does every
    // other shape.
    for id in SkillId::ALL {
        let long = matches!(
            id,
            SkillId::DawnRay | SkillId::WildRocket | SkillId::HorizonWave
        );
        assert_eq!(vector(id).is_some(), long, "{}", id.id());
    }
}

/// The movement clip of the client is the server's: the static map first, then the
/// discs of living structures and armed pillars, all measured from the start.
#[test]
fn the_movement_clip_stops_at_structures_and_pillars() {
    // Open ground at the middle of the map.
    let from = Vec2::ZERO;
    let to = Vec2::new(10.0, 0.0);
    assert!(movement_clip(&[], &[], from, to).distance(to) < 1e-4);
    let disc = |x: f32, radius: f32| Disc {
        center: [x, 0.0],
        radius,
    };
    let tower = [disc(6.0, 1.3)];
    let pillar = [disc(4.0, 1.0)];
    let at_tower = movement_clip(&tower, &[], from, to);
    let at_pillar = movement_clip(&[], &pillar, from, to);
    // A hero is half a unit wide: it stops that far short of either disc.
    let hero = shared::navigation::HERO_RADIUS;
    assert!((at_tower.x - (6.0 - 1.3 - hero)).abs() < 0.01, "{at_tower}");
    assert!(
        (at_pillar.x - (4.0 - 1.0 - hero)).abs() < 0.01,
        "{at_pillar}"
    );
    // The nearer of the two stops the move, whichever list it is in.
    for both in [
        movement_clip(&tower, &pillar, from, to),
        movement_clip(&pillar, &tower, from, to),
    ] {
        assert!(both.distance(at_pillar) < 1e-4, "{both}");
    }
    // The forest stops it as well.
    let map = shared::navigation::world_navigation();
    let edge = (1..80)
        .map(|step| Vec2::X * step as f32)
        .find(|at| !map.point_clear(at.to_array()))
        .expect("the forest is somewhere along +X");
    let stopped = movement_clip(&[], &[], from, edge);
    assert!(
        stopped.x < edge.x - 1e-3 && stopped.x > edge.x - 1.0,
        "{stopped} {edge}"
    );
    assert!(map.point_clear(stopped.to_array()));
}

/// The held key of a hero in a world: what `draw_aim` hands to the table.
#[cfg(feature = "qa")]
mod held {
    use super::*;
    use crate::combat::standard::{SkillAimGizmos, SkillAimVector, draw_aim};
    use crate::combat::{CombatStats, TargetState};
    use crate::net::{
        NetworkHeroClass, NetworkMinion, NetworkMinionId, NetworkPlayerId, PlayerLoadout,
        PlayerProgression, RemotePlayer,
    };
    use crate::player::Player;
    use crate::team::Team;
    use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
    use bevy::window::PrimaryWindow;
    use shared::loadout::CoreId;

    /// A flat view ten units to each side of the origin, the hero of `core` at `HERO`.
    fn stage(core: CoreId, flags: LoadoutState) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<bevy::gizmos::GizmoAsset>()
            .init_gizmo_group::<SkillAimGizmos>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<crate::input_context::GameplayInputContext>()
            .init_resource::<TargetState>()
            .init_resource::<crate::targeting::BasicAttackState>()
            .init_resource::<SkillAimVector>()
            .init_resource::<AimPreviewShown>()
            .insert_resource(PlayerVisualMode::Sprite2d)
            .add_systems(Update, draw_aim);
        let window = Window {
            resolution: bevy::window::WindowResolution::new(800, 800),
            ..default()
        };
        let window = app.world_mut().spawn((window, PrimaryWindow)).id();
        app.world_mut().spawn((
            crate::camera::MainCamera,
            GlobalTransform::IDENTITY,
            Camera {
                computed: ComputedCameraValues {
                    clip_from_view: Mat4::from_scale(Vec3::new(0.1, 0.1, 1.0)),
                    target_info: Some(RenderTargetInfo {
                        physical_size: UVec2::new(800, 800),
                        scale_factor: 1.0,
                    }),
                    ..default()
                },
                ..default()
            },
        ));
        app.world_mut().spawn((
            Player,
            Transform::from_xyz(HERO.x, 0.0, HERO.y),
            NetworkHeroClass(core.class()),
            PlayerProgression::default(),
            CombatStats::default(),
            Team::Green,
            NetworkPlayerId(7),
            PlayerLoadout(Some(LoadoutState {
                recipe: Some(core.preset()),
                ..flags
            })),
        ));
        (app, window)
    }

    /// Puts the cursor on a point of the world.
    fn point_at(app: &mut App, window: Entity, at: Vec2) {
        let pixel = Vec2::new(400.0 + at.x * 40.0, 400.0 - at.y * 40.0);
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(pixel));
    }

    fn hold(app: &mut App, key: KeyCode) -> Option<(usize, Preview)> {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.reset_all();
        keys.press(key);
        app.update();
        app.world().resource::<AimPreviewShown>().0.clone()
    }

    #[test]
    fn a_held_self_cast_is_previewed_without_a_cursor() {
        // Thunder Pulse is the E of the Stormfist: before, a held self cast drew nothing.
        let (mut app, _) = stage(CoreId::Stormfist, default());
        let (slot, preview) = hold(&mut app, KeyCode::KeyE).expect("the pulse is previewed");
        assert_eq!((slot, preview.shape), (2, PreviewShape::SelfRing));
        assert_eq!(
            preview.areas,
            [GeoShape::Ring {
                center: HERO,
                radius: 5.0
            }]
        );
        // In its recast window the press slows the stored victims: no ring.
        let (mut app, _) = app_with_recast(CoreId::Stormfist, 2);
        let (_, preview) = hold(&mut app, KeyCode::KeyE).unwrap();
        assert_eq!(preview.shape, PreviewShape::None);
        assert!(preview.areas.is_empty());
        // A skill that needs an aim still waits for the cursor.
        assert!(hold(&mut app, KeyCode::KeyQ).is_none());
        // No key, no preview; a dead hero previews nothing.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.update();
        assert!(app.world().resource::<AimPreviewShown>().0.is_none());
        let (mut app, _) = stage(CoreId::Stormfist, default());
        let mut heroes = app
            .world_mut()
            .query_filtered::<&mut CombatStats, With<Player>>();
        heroes.single_mut(app.world_mut()).unwrap().hp = 0.0;
        assert!(hold(&mut app, KeyCode::KeyE).is_none());
    }

    fn app_with_recast(core: CoreId, slot: usize) -> (App, Entity) {
        let mut flags = LoadoutState::default();
        flags.slots[slot].can_recast = true;
        stage(core, flags)
    }

    #[test]
    fn the_held_key_reads_the_orb_the_recast_flag_and_the_units_the_client_sees() {
        let orb = Vec2::new(-5.0, 4.0);
        let (mut app, window) = stage(
            CoreId::Orbitwright,
            LoadoutState {
                orb_position: Some(orb.to_array()),
                ..default()
            },
        );
        let aim = Vec2::new(5.0, 2.0);
        point_at(&mut app, window, aim);
        let ally = app
            .world_mut()
            .spawn((
                RemotePlayer,
                Transform::from_xyz(aim.x + 0.5, 0.0, aim.y),
                Team::Green,
                NetworkPlayerId(9),
                CombatStats::default(),
                InheritedVisibility::VISIBLE,
            ))
            .id();
        // The field and the collapse stand on the orb.
        for (key, radius) in [(KeyCode::KeyW, 4.0), (KeyCode::KeyR, 5.0)] {
            let (_, preview) = hold(&mut app, key).unwrap();
            assert_eq!(
                preview.areas,
                [GeoShape::Ring {
                    center: orb,
                    radius
                }]
            );
        }
        // The guard picks the allied hero at the cursor and draws the orb's way to it.
        let (slot, preview) = hold(&mut app, KeyCode::KeyE).unwrap();
        assert_eq!((slot, preview.shape), (2, PreviewShape::UnitPick));
        assert!(!preview.refused);
        let GeoShape::Lane { from, to, .. } = preview.areas[1] else {
            panic!("the orb has a way: {preview:?}");
        };
        assert!(from.distance(orb) < 1e-4 && to.distance(aim + Vec2::new(0.5, 0.0)) < 1e-4);
        // The guard takes heroes only. An enemy hero at the cursor is no ally, and an
        // allied minion nearer the cursor than the allied hero does not shadow him: the
        // orb still has its way to the hero.
        app.world_mut().spawn((
            RemotePlayer,
            Transform::from_xyz(aim.x, 0.0, aim.y),
            Team::Blue,
            NetworkPlayerId(21),
            CombatStats::default(),
            InheritedVisibility::VISIBLE,
        ));
        let (_, legal) = hold(&mut app, KeyCode::KeyE).unwrap();
        assert!(!legal.refused);
        app.world_mut().spawn((
            NetworkMinion,
            Transform::from_xyz(aim.x + 0.2, 0.0, aim.y),
            Team::Green,
            NetworkMinionId(3),
            CombatStats::default(),
            InheritedVisibility::VISIBLE,
        ));
        let (_, preview) = hold(&mut app, KeyCode::KeyE).unwrap();
        assert_eq!(preview.areas, legal.areas);
        assert_eq!((preview.marks, preview.refused), (legal.marks, false));
        // A unit the client hides, and a dead one, are not offered to the rule: without
        // the hero the minion is no pick.
        app.world_mut()
            .entity_mut(ally)
            .insert(InheritedVisibility::HIDDEN);
        let (_, preview) = hold(&mut app, KeyCode::KeyE).unwrap();
        assert!(preview.refused && preview.pick.is_none() && preview.areas.len() == 1);
        app.world_mut()
            .entity_mut(ally)
            .insert(InheritedVisibility::VISIBLE);
        assert!(!hold(&mut app, KeyCode::KeyE).unwrap().1.refused);
        app.world_mut().entity_mut(ally).insert(CombatStats {
            hp: 0.0,
            ..default()
        });
        let (_, preview) = hold(&mut app, KeyCode::KeyE).unwrap();
        assert!(preview.refused && preview.pick.is_none());
        // The caster is a hero of his own team: aimed at himself, the guard is legal.
        point_at(&mut app, window, HERO + Vec2::new(0.5, 0.0));
        let (_, preview) = hold(&mut app, KeyCode::KeyE).unwrap();
        assert_eq!((preview.pick, preview.refused), (Some(0), false));
        assert_eq!(
            preview.marks,
            [PreviewMark::Picked {
                at: HERO,
                radius: shared::PLAYER_TARGET_RADIUS
            }]
        );
    }

    /// What `draw_aim` shows for a controller that holds the button of `slot`, with its
    /// right stick at `stick`.
    fn held_on_the_pad(app: &mut App, slot: usize, stick: Option<Vec2>) -> Preview {
        let mut pad = crate::gamepad::GamepadControls::default();
        pad.active = true;
        pad.aiming_slot = Some(slot);
        pad.aim = stick;
        app.insert_resource(pad);
        app.update();
        let (shown, preview) = app.world().resource::<AimPreviewShown>().0.clone().unwrap();
        assert_eq!(shown, slot);
        preview
    }

    /// The preview of a press without the stick is drawn at the aim the cast is sent with:
    /// for a skill cast on an ally that is an ally (`mobile::ally_quick_cast_target`), not
    /// the enemy the other skills are aimed at.
    #[test]
    fn a_press_without_the_stick_previews_the_ally_the_cast_is_sent_to() {
        let hero = |app: &mut App, id: u64, at: Vec2, team: Team, health: f32| {
            let mut stats = CombatStats::default();
            stats.hp *= health;
            app.world_mut().spawn((
                RemotePlayer,
                Transform::from_xyz(at.x, 0.0, at.y),
                team,
                NetworkPlayerId(id),
                stats,
                InheritedVisibility::VISIBLE,
            ));
        };
        let enemy = HERO + Vec2::new(1.5, 0.0);
        let near = HERO + Vec2::new(0.0, 6.0);
        let hurt = HERO + Vec2::new(-9.0, 0.0);
        let picked = |at: Vec2| {
            [PreviewMark::Picked {
                at,
                radius: shared::PLAYER_TARGET_RADIUS,
            }]
        };
        // Where the pick ring of a preview stands.
        let ring = |preview: &Preview| {
            let GeoShape::Ring { center, radius } = preview.areas[0] else {
                panic!("a pick has its ring: {preview:?}");
            };
            assert_eq!(radius, PICK_RADIUS);
            center
        };
        for (core, slot, ally) in [
            // Orbital Guard goes to the nearest allied hero.
            (CoreId::Orbitwright, 2, near),
            // Sheltering Leap goes to the allied hero lowest on health.
            (CoreId::Frostguard, 1, hurt),
        ] {
            let (mut app, _) = stage(core, default());
            // Alone, the cast is aimed at the caster himself and is legal.
            let preview = held_on_the_pad(&mut app, slot, None);
            assert_eq!(preview.shape, PreviewShape::UnitPick);
            assert_eq!((preview.pick, preview.refused), (Some(0), false));
            assert!(ring(&preview).distance(HERO) < 1e-4);
            assert_eq!(preview.marks, picked(HERO));
            // An enemy next to the hero does not draw the aim to itself.
            hero(&mut app, 21, enemy, Team::Blue, 1.0);
            let preview = held_on_the_pad(&mut app, slot, None);
            assert!(ring(&preview).distance(HERO) < 1e-4);
            assert!(!preview.refused);
            // With allied heroes around, the pick ring stands on the one the cast goes to.
            hero(&mut app, 9, near, Team::Green, 1.0);
            hero(&mut app, 10, hurt, Team::Green, 0.3);
            let preview = held_on_the_pad(&mut app, slot, None);
            assert!(ring(&preview).distance(ally) < 1e-4, "{core:?}");
            assert_eq!(preview.marks, picked(ally), "{core:?}");
            assert!(!preview.refused);
            // The stick is the player's own aim: the ring follows it and finds nobody.
            let way = crate::player::mobile_screen_direction(
                Vec2::X,
                &GlobalTransform::IDENTITY,
                PlayerVisualMode::Sprite2d,
            )
            .xz();
            let kit = shared::loadout::preset_for_class(core.class()).unwrap();
            let range = skill(kit.skills()[slot]).ability.cast_range;
            let preview = held_on_the_pad(&mut app, slot, Some(Vec2::X));
            assert!(ring(&preview).distance(HERO + way * range) < 1e-3);
            assert!(preview.refused && preview.marks.is_empty() && preview.areas.len() == 1);
        }
        // A skill that is not cast on an ally is still aimed at the enemy.
        let (mut app, _) = stage(CoreId::Frostguard, default());
        hero(&mut app, 21, enemy, Team::Blue, 1.0);
        hero(&mut app, 9, near, Team::Green, 1.0);
        let preview = held_on_the_pad(&mut app, 0, None);
        let GeoShape::Lane { from, to, .. } = preview.areas[0] else {
            panic!("Winter Shard is a lane: {preview:?}");
        };
        assert!((to - from).normalize().distance(Vec2::X) < 1e-4);
    }

    #[test]
    fn a_long_lane_is_handed_to_the_minimap_and_a_ring_is_not() {
        let (mut app, window) = stage(CoreId::Dawnweaver, default());
        point_at(&mut app, window, HERO + Vec2::new(0.0, 6.0));
        let (_, preview) = hold(&mut app, KeyCode::KeyR).unwrap();
        assert_eq!(preview.shape, PreviewShape::LaneCapsule);
        let (from, to, radius) = app.world().resource::<SkillAimVector>().0.unwrap();
        assert!(from.distance(HERO) < 1e-4 && to.distance(HERO + Vec2::new(0.0, 45.0)) < 1e-3);
        assert_eq!(radius, 0.8);
        let (_, preview) = hold(&mut app, KeyCode::KeyE).unwrap();
        assert_eq!(preview.shape, PreviewShape::PointRing);
        assert!(app.world().resource::<SkillAimVector>().0.is_none());
        // In the recast window of the field its key detonates it where it stands: the
        // aim point gets no ring.
        let (mut app, window) = app_with_recast(CoreId::Dawnweaver, 2);
        point_at(&mut app, window, HERO + Vec2::new(0.0, 6.0));
        let (_, preview) = hold(&mut app, KeyCode::KeyE).unwrap();
        assert_eq!(preview.shape, PreviewShape::None);
        assert!(preview.areas.is_empty());
    }

    #[test]
    fn a_blink_onto_a_landing_the_server_refuses_is_painted_as_refused() {
        use crate::net::{NetworkStructure, NetworkStructureId, StructureKind};

        let (mut app, window) = stage(CoreId::Riftshot, default());
        let aim = HERO + Vec2::new(4.0, 0.0);
        let map = shared::navigation::world_navigation();
        assert!(
            [HERO, aim, aim + Vec2::X]
                .into_iter()
                .all(|at| map.point_clear(at.to_array()))
        );
        point_at(&mut app, window, aim);
        // Rift Step is the E of the Riftshot. On open ground its landing is legal.
        let (slot, preview) = hold(&mut app, KeyCode::KeyE).unwrap();
        assert_eq!((slot, preview.shape), (2, PreviewShape::BlinkLanding));
        let [PreviewMark::Landing(landing)] = preview.marks[..] else {
            panic!("a blink has one landing: {preview:?}");
        };
        assert!(landing.distance(aim) < 1e-3, "{landing}");
        assert!(!preview.refused);
        let open = strokes(&preview);

        // A tower whose disc holds the landing: the same lines, in the refusal colour.
        let tower = app
            .world_mut()
            .spawn((
                NetworkStructure,
                NetworkStructureId(4),
                Transform::from_xyz(aim.x + 1.29, 0.0, aim.y),
                CombatStats::default(),
                Team::Blue,
                StructureKind::Tower,
            ))
            .id();
        let (_, preview) = hold(&mut app, KeyCode::KeyE).unwrap();
        assert!(preview.refused);
        assert_eq!(strokes(&preview), open);
        assert_eq!(color(Ink::Mark, preview.refused), color(Ink::Area, true));
        // Only the blink is judged by its landing: a lane aimed into the tower is not.
        assert!(!hold(&mut app, KeyCode::KeyQ).unwrap().1.refused);
        // A step further from the tower, and at a tower that fell, the landing is legal.
        app.world_mut()
            .entity_mut(tower)
            .insert(Transform::from_xyz(aim.x + 1.31, 0.0, aim.y));
        assert!(!hold(&mut app, KeyCode::KeyE).unwrap().1.refused);
        app.world_mut().entity_mut(tower).insert((
            Transform::from_xyz(aim.x + 1.0, 0.0, aim.y),
            CombatStats {
                hp: 0.0,
                ..default()
            },
        ));
        assert!(!hold(&mut app, KeyCode::KeyE).unwrap().1.refused);

        // An armed pillar the client sees keeps a hero's radius more than its own.
        let pillar = |armed: bool, apart: f32| SkillEffectState {
            id: 5,
            owner_id: 9,
            owner_team: shared::map::Team::Blue,
            skill: SkillId::FaultLine,
            kind: shared::loadout::EffectVisualKind::Trap,
            position: [aim.x, aim.y + apart],
            end: [aim.x, aim.y + apart],
            radius: 1.0,
            remaining_secs: 3.0,
            armed,
            consumed_segments: 0,
        };
        for (armed, apart, refused) in [(true, 1.4, true), (true, 1.6, false), (false, 1.4, false)]
        {
            app.insert_resource(GameStateSnapshot {
                your_id: 7,
                skill_effects: vec![pillar(armed, apart)],
                ..default()
            });
            let (_, preview) = hold(&mut app, KeyCode::KeyE).unwrap();
            assert_eq!(preview.refused, refused, "{armed} {apart}");
        }
    }
}
