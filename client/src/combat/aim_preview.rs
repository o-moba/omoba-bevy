//! The aim preview of a held skill: the rule the server would apply to the cast, drawn on
//! the ground from `geometry::preview_shape`. Presentation only; nothing here decides a hit.
// i18n-strict
use bevy::ecs::system::SystemParam;
use bevy::gizmos::config::GizmoConfigGroup;
use bevy::prelude::*;
use shared::loadout::{LoadoutState, SkillDefinition};
use shared::navigation::Disc;

use super::selection::TargetCandidates;
use super::standard::point;
use crate::net::{GameStateSnapshot, StructureKind, TargetKind};
use crate::skill_presentation::geometry::{
    self, GeoShape, PickCandidate, PickRule, Preview, PreviewContext, PreviewMark,
};
use crate::skill_presentation::vocab::PreviewShape;
use crate::sprite::PlayerVisualMode;

/// What a line of the preview is painted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ink {
    /// The boundary of an area the server tests.
    Area,
    /// A path or a push: a reading aid that claims no area.
    Aid,
    /// The landing point and the unit the server would pick.
    Mark,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Stroke {
    pub points: Vec<Vec2>,
    pub ink: Ink,
}

/// A lane this long also shows on the minimap.
const MINIMAP_LANE: f32 = 35.0;
/// The arrowhead of a lane stands at most this far from the caster.
const HEAD_REACH: f32 = 14.0;
/// Length of the arrowhead of a lane and of a push arrow.
const HEAD: f32 = 1.5;
/// Half-size of the landing mark.
const LANDING: f32 = 0.45;
/// Clearance of the two highlight rings around a picked unit.
const PICK_RINGS: [f32; 2] = [0.25, 0.5];

/// The lane a preview shows on the minimap: its two ends and its half-width.
pub(super) fn minimap_vector(preview: &Preview) -> Option<(Vec2, Vec2, f32)> {
    if !matches!(
        preview.shape,
        PreviewShape::Lane | PreviewShape::LaneCapsule
    ) {
        return None;
    }
    preview.areas.iter().find_map(|area| match *area {
        GeoShape::Lane {
            from,
            to,
            half_width: radius,
        }
        | GeoShape::Capsule { from, to, radius }
            if from.distance(to) >= MINIMAP_LANE =>
        {
            Some((from, to, radius))
        }
        _ => None,
    })
}

/// The two wings of an arrowhead whose tip is `tip`.
fn head(tip: Vec2, along: Vec2, half_width: f32, ink: Ink) -> [Stroke; 2] {
    [-1.0, 1.0].map(|side| Stroke {
        points: vec![tip - along * HEAD + along.perp() * half_width * side, tip],
        ink,
    })
}

/// The ground lines of a preview, in simulation coordinates. Every area is outlined
/// exactly as `geometry` derived it; a lane from the caster keeps its two open rails and
/// its arrowhead.
pub(super) fn strokes(preview: &Preview) -> Vec<Stroke> {
    let mut strokes = Vec::new();
    for area in &preview.areas {
        match *area {
            GeoShape::Lane {
                from,
                to,
                half_width,
            } if preview.shape == PreviewShape::Lane => {
                let along = (to - from).normalize_or_zero();
                let side = along.perp() * half_width;
                strokes.extend([-1.0, 1.0].map(|sign| Stroke {
                    points: vec![from + side * sign, to + side * sign],
                    ink: Ink::Area,
                }));
                let tip = from + along * from.distance(to).min(HEAD_REACH);
                strokes.extend(head(tip, along, half_width.max(0.5), Ink::Area));
            }
            GeoShape::Capsule { from, to, radius } => {
                strokes.extend(area.outline().into_iter().map(|points| Stroke {
                    points,
                    ink: Ink::Area,
                }));
                let along = (to - from).normalize_or_zero();
                let tip = from + along * from.distance(to).min(HEAD_REACH);
                strokes.extend(head(tip, along, radius.max(0.5), Ink::Area));
            }
            _ => strokes.extend(area.outline().into_iter().map(|points| Stroke {
                points,
                ink: Ink::Area,
            })),
        }
    }
    for mark in &preview.marks {
        match *mark {
            PreviewMark::Path { from, to } => strokes.push(Stroke {
                points: vec![from, to],
                ink: Ink::Aid,
            }),
            PreviewMark::Landing(at) => {
                let corners = [Vec2::X, Vec2::Y, Vec2::NEG_X, Vec2::NEG_Y, Vec2::X];
                strokes.push(Stroke {
                    points: corners.map(|corner| at + corner * LANDING).to_vec(),
                    ink: Ink::Mark,
                });
                strokes.extend([Vec2::X, Vec2::Y].map(|axis| Stroke {
                    points: vec![at - axis * LANDING, at + axis * LANDING],
                    ink: Ink::Mark,
                }));
            }
            PreviewMark::Picked { at, radius } => {
                strokes.extend(PICK_RINGS.map(|clearance| {
                    Stroke {
                        points: GeoShape::Ring {
                            center: at,
                            radius: radius + clearance,
                        }
                        .outline()
                        .remove(0),
                        ink: Ink::Mark,
                    }
                }));
            }
            PreviewMark::Push { from, to } => {
                strokes.push(Stroke {
                    points: vec![from, to],
                    ink: Ink::Aid,
                });
                let along = (to - from).normalize_or_zero();
                strokes.extend(head(to, along, 0.5, Ink::Aid));
            }
        }
    }
    strokes
}

/// The colour of a line: the aim colour, or the refusal colour when the server would drop
/// the cast.
pub(super) fn color(ink: Ink, refused: bool) -> Color {
    let base = if refused {
        Color::linear_rgb(5.0, 0.16, 0.015)
    } else if ink == Ink::Mark {
        Color::linear_rgb(2.5, 4.5, 5.0)
    } else {
        Color::linear_rgb(0.015, 0.8, 5.0)
    };
    if ink == Ink::Aid {
        base.with_alpha(0.55)
    } else {
        base
    }
}

pub(super) fn draw<G: GizmoConfigGroup>(
    gizmos: &mut Gizmos<G>,
    preview: &Preview,
    mode: PlayerVisualMode,
    map: Option<&crate::maps::MapLayout>,
) {
    for stroke in strokes(preview) {
        // Long edges are split so that they follow the ground in 3D.
        let points = stroke.points.windows(2).flat_map(|edge| {
            let steps = (edge[0].distance(edge[1]) / 1.5).ceil().clamp(1.0, 192.0) as usize;
            (0..steps).map(move |i| edge[0].lerp(edge[1], i as f32 / steps as f32))
        });
        gizmos.linestrip(
            points
                .chain(stroke.points.last().copied())
                .map(|at| point(at, mode, map)),
            color(stroke.ink, preview.refused),
        );
    }
}

/// The living units the client sees as the server's pick rules read them, the caster first.
pub(crate) fn pick_candidates(
    caster: Vec2,
    caster_id: u64,
    team: crate::team::Team,
    units: &TargetCandidates,
    visible: &Query<&InheritedVisibility>,
) -> Vec<PickCandidate> {
    // A unit the client hides is one its team does not see.
    let seen = |entity: Entity| visible.get(entity).map_or(true, |shown| shown.get());
    let mut candidates = vec![PickCandidate {
        kind: TargetKind::Player,
        id: caster_id,
        position: caster,
        radius: shared::PLAYER_TARGET_RADIUS,
        ally: true,
    }];
    candidates.extend(
        units
            .players
            .iter()
            .filter(|(entity, _, _, stats, _)| stats.is_alive() && seen(*entity))
            .map(|(_, pose, id, _, other)| PickCandidate {
                kind: TargetKind::Player,
                id: id.0,
                position: pose.translation.xz(),
                radius: shared::PLAYER_TARGET_RADIUS,
                ally: *other == team,
            }),
    );
    candidates.extend(
        units
            .minions
            .iter()
            .filter(|(entity, _, _, stats, _)| stats.is_alive() && seen(*entity))
            .map(|(_, pose, id, _, other)| PickCandidate {
                kind: TargetKind::Minion,
                id: id.0,
                position: pose.translation.xz(),
                radius: shared::MINION_TARGET_RADIUS,
                ally: *other == team,
            }),
    );
    candidates.extend(
        units
            .structures
            .iter()
            .filter(|(entity, _, _, stats, ..)| stats.is_alive() && seen(*entity))
            .map(|(_, pose, id, _, other, kind)| PickCandidate {
                kind: TargetKind::Structure,
                id: id.0,
                position: pose.translation.xz(),
                radius: match kind {
                    StructureKind::Tower => shared::TOWER_TARGET_RADIUS,
                    StructureKind::BaseTower => shared::BASE_TOWER_TARGET_RADIUS,
                },
                ally: *other == team,
            }),
    );
    // A neutral belongs to no team.
    candidates.extend(
        units
            .neutrals
            .iter()
            .filter(|(entity, _, _, stats)| stats.is_alive() && seen(*entity))
            .map(|(_, pose, id, _)| PickCandidate {
                kind: TargetKind::Neutral,
                id: id.0,
                position: pose.translation.xz(),
                radius: shared::NEUTRAL_TARGET_RADIUS,
                ally: false,
            }),
    );
    candidates
}

/// The aim to send for a cast that the server drops unless it finds an ally at the aim: the
/// place of the unit it takes at `aim`, or `None` when it finds none. A hero-only pick
/// leaves every other kind out before it looks for the nearest unit
/// (`common/src/skills/advanced.rs:154-197`, `:490-514`). The picked unit itself is sent, so
/// that the server takes the same unit from its own positions. A unit is picked up to its
/// radius beyond the cast range, where a point aim is dropped (`:465-467`): the aim stays as
/// it is then.
pub(crate) fn ally_aim(
    rule: PickRule,
    candidates: &[PickCandidate],
    aim: Vec2,
    origin: Vec2,
    range: f32,
) -> Option<Vec2> {
    let eligible: Vec<PickCandidate> = candidates
        .iter()
        .filter(|unit| !rule.hero_only || unit.kind == TargetKind::Player)
        .copied()
        .collect();
    let picked = eligible[geometry::server_pick(&eligible, aim, origin, range, rule.ally)?];
    Some(if origin.distance(picked.position) <= range {
        picked.position
    } else {
        aim
    })
}

/// Where a ground move from one point toward another ends: the static map, then the
/// living structures, then the armed pillars the client sees, in the order of the server
/// (`common/src/skills/advanced.rs:277-279`).
pub(crate) fn movement_clip(structures: &[Disc], terrain: &[Disc], from: Vec2, to: Vec2) -> Vec2 {
    let start = from.to_array();
    let end = shared::navigation::world_navigation().clip_movement(start, to.to_array());
    let end = shared::navigation::clip_discs(start, end, structures);
    Vec2::from_array(shared::navigation::clip_discs(start, end, terrain))
}

/// The hero whose key is held.
pub(super) struct Caster<'a> {
    pub position: Vec2,
    pub id: Option<u64>,
    pub team: crate::team::Team,
    pub flags: Option<&'a LoadoutState>,
    pub slot: usize,
}

/// What the preview reads of the world besides the hero and its aim.
#[derive(SystemParam)]
pub(crate) struct AimWorld<'w, 's> {
    game: Option<Res<'w, GameStateSnapshot>>,
    visible: Query<'w, 's, &'static InheritedVisibility>,
}

impl AimWorld<'_, '_> {
    /// The discs of the structures that stand and of the armed pillars the client sees.
    fn solids(&self, units: &TargetCandidates) -> (Vec<Disc>, Vec<Disc>) {
        let structures = units
            .structures
            .iter()
            .filter(|(_, _, _, stats, ..)| stats.is_alive())
            .map(|(_, pose, _, _, _, kind)| Disc {
                center: pose.translation.xz().to_array(),
                radius: crate::navigation::structure_collision_radius(*kind)
                    - shared::navigation::HERO_RADIUS,
            })
            .collect();
        (
            structures,
            crate::navigation::skill_terrain(self.game.as_deref()),
        )
    }

    /// Whether the server would let a blink land on `point`, from what the client sees
    /// now. The preview and the cast ask this one question, so a landing painted as
    /// refused is a cast that is not sent.
    pub(super) fn blink_legal(&self, point: Vec2, units: &TargetCandidates) -> bool {
        let (structures, terrain) = self.solids(units);
        crate::navigation::blink_point_legal(point, &structures, &terrain)
    }

    /// The aim to send for a cast that needs an ally under `rule`, from what the client
    /// sees now; `None` when the server would drop the cast as it is aimed.
    pub(super) fn ally_aim(
        &self,
        rule: PickRule,
        caster: &Caster,
        range: f32,
        aim: Vec2,
        units: &TargetCandidates,
    ) -> Option<Vec2> {
        let game = self.game.as_deref();
        let hero = caster.id.or(game.map(|game| game.your_id)).unwrap_or(0);
        let candidates = pick_candidates(caster.position, hero, caster.team, units, &self.visible);
        ally_aim(rule, &candidates, aim, caster.position, range)
    }

    /// The preview of `def` for a hero that aims at `aim`, from what the client sees now.
    pub(super) fn preview(
        &self,
        def: &SkillDefinition,
        caster: &Caster,
        aim: Vec2,
        units: &TargetCandidates,
    ) -> Preview {
        let game = self.game.as_deref();
        let hero = caster.id.or(game.map(|game| game.your_id)).unwrap_or(0);
        let candidates = pick_candidates(caster.position, hero, caster.team, units, &self.visible);
        let (structures, terrain) = self.solids(units);
        let clip = |from: Vec2, to: Vec2| movement_clip(&structures, &terrain, from, to);
        let mut preview = geometry::preview_shape(
            def,
            &PreviewContext {
                origin: caster.position,
                aim,
                recast: caster
                    .flags
                    .is_some_and(|flags| flags.slots[caster.slot].can_recast),
                orb: caster
                    .flags
                    .and_then(|flags| flags.orb_position)
                    .map(Vec2::from_array),
                hero,
                effects: game.map_or(&[], |game| &game.skill_effects),
                candidates: &candidates,
                clip: &clip,
            },
        );
        // A blink is not clipped to a legal point: the server drops the cast instead.
        if preview.shape == PreviewShape::BlinkLanding
            && !crate::navigation::blink_point_legal(aim, &structures, &terrain)
        {
            preview.refused = true;
        }
        preview
    }
}

/// Evidence of the preview drawn this frame: the slot whose key is held and its preview.
#[cfg(feature = "qa")]
#[derive(Resource, Default)]
pub(crate) struct AimPreviewShown(pub Option<(usize, Preview)>);

#[cfg(test)]
mod tests;
