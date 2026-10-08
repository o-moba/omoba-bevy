//! The pooled-particle language at the caster and at a replicated effect: cast accents, the
//! outline of a signed-off instant area, moves, links, stage one-shots and cues.
//! Every generator is a pure function of its arguments, so a row is validated by running
//! it, and nothing here can read a position the client did not observe. The systems at the
//! end hand the generators what the cast observer, the stage tracker and the receipt
//! collector reported.
use super::SkillPresentation;
use super::cast::{CastKey, MoveCause, MoveObserved, SkillCastObserved, ThemedDashes};
use super::category::{self, SkillKey, StrikeOrigin};
use super::geometry::{self, AreaContext, GeoShape};
pub(crate) use super::schema::{CastAccent, MoveSpec};
use super::schema::{SkillProfile, Theme};
use super::stage::{EndKind, OwnerSeen, StageChange, StageEvent, Transition};
use super::vocab::{AccentPattern, Archetype, ExpireKind, MovePattern, PaletteSlot, ParticleShape};
use crate::combat_feedback::ConfirmedHit;
use crate::game_vfx::{
    Curve, Orient, ParticleSource, ParticleSpec, SkillBurst, Tint, jitter, unit_radius,
};
use crate::net::GameStateSnapshot;
use crate::player::Player;
use bevy::prelude::*;
use shared::loadout::{EffectVisualKind, SkillEffectState, SkillId};
use std::collections::HashSet;
use std::f32::consts::{PI, TAU};

/// Budgets of one generated burst (particles, seconds until the last one is gone).
pub(crate) const ACCENT_MAX: usize = 8;
pub(crate) const ACCENT_SECS: f32 = 0.5;
pub(crate) const MOVE_MAX: usize = 8;
pub(crate) const MOVE_SECS: f32 = 0.6;
pub(crate) const LINK_MAX: usize = 3;
pub(crate) const LINK_SECS: f32 = 0.25;
pub(crate) const STAGE_MAX: usize = 8;
pub(crate) const STAGE_SECS: f32 = 0.6;
/// How long the flash of a released telegraph is seen: the payoff of a wait, so it stays
/// for most of what a stage one-shot may last.
const DISCHARGE_SECS: f32 = 0.5;
/// Wisps of a fade, and the pause between one pair of them and the next.
const FADE_WISPS: usize = 6;
const FADE_STEP_SECS: f32 = 0.03;
/// Budget of a cue of the receipt collector, the trap cue and the camp hit: particles
/// and seconds.
#[cfg(test)]
pub(crate) const CUE_MAX: usize = 6;
pub(crate) const CUE_SECS: f32 = 0.4;
/// A decorative accent stays within this distance of the caster on the ground plane.
pub(crate) const DECORATIVE_REACH: f32 = 2.0;
/// Half extent of a soft glint of an accent as a share of the reach of its pattern.
const GLINT: f32 = 0.14;

/// Half length of one skid mark of a forced displacement, at most.
const SKID: f32 = 0.6;

/// Heights above the ground a pattern is drawn at.
const FLOOR: f32 = 0.06;
const HAND: f32 = 0.9;
const CHEST: f32 = 1.0;
/// The ring of `rune_mark` lies on the ground around the caster, clear of the decals under
/// it: over the head it covered the health bars of the caster.
const RUNE: f32 = 0.14;
/// Half extent of a mark of `strike_line` in units per unit of the row scale, at most:
/// eight marks of scale 2 join into one crack along the 15 units of a fault line. And the
/// share of its life in which the crack runs from the caster to the other end.
const STRIKE_MARK: f32 = 0.5;
const STRIKE_RUN: f32 = 0.3;

/// The four colours a row may name, resolved for one skill or one class.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Palette {
    primary: Tint,
    secondary: Tint,
    accent: Tint,
    white: Tint,
}

impl Palette {
    /// A skill row: its own colour and the theme of its home class, unless the row
    /// overrides a theme colour. The matter colour is drawn without HDR gain.
    pub(crate) fn of(profile: &SkillProfile, theme: &Theme) -> Self {
        let lit = |color: [f32; 3]| Tint {
            color: Color::srgb_from_array(color),
            gain: profile.hdr_gain,
        };
        Self {
            primary: lit(profile.color),
            secondary: Tint {
                color: Color::srgb_from_array(profile.secondary.unwrap_or(theme.secondary)),
                gain: 1.0,
            },
            accent: lit(profile.accent.unwrap_or(theme.accent)),
            white: lit([1.0; 3]),
        }
    }

    /// A basic attack: the class theme at the pool's own gain. A basic row cannot name
    /// `primary`; the slot falls back to the spark colour.
    pub(crate) fn of_class(theme: &Theme) -> Self {
        let gain = ParticleSpec::BASE.color.gain;
        let accent = Tint {
            color: Color::srgb_from_array(theme.accent),
            gain,
        };
        Self {
            primary: accent,
            secondary: Tint {
                color: Color::srgb_from_array(theme.secondary),
                gain: 1.0,
            },
            accent,
            white: Tint {
                color: Color::WHITE,
                gain,
            },
        }
    }

    pub(crate) fn slot(&self, slot: PaletteSlot) -> Tint {
        match slot {
            PaletteSlot::Primary => self.primary,
            PaletteSlot::Secondary => self.secondary,
            PaletteSlot::Accent => self.accent,
            PaletteSlot::White => self.white,
        }
    }

    /// The lead colour and the companion colour of a block; `[primary, accent]` when the
    /// row names none.
    pub(super) fn pair(&self, slots: Option<[PaletteSlot; 2]>) -> [Tint; 2] {
        slots
            .unwrap_or([PaletteSlot::Primary, PaletteSlot::Accent])
            .map(|slot| self.slot(slot))
    }
}

/// Size at which the mesh of `shape` is `radius` units from its centre at the peak of
/// `curve`.
pub(super) fn sized(shape: ParticleShape, radius: f32, curve: Curve) -> f32 {
    radius / (unit_radius(shape) * curve.peak())
}

/// Starts the particle `share` of its life late and still ends it on time.
pub(super) fn late(mut spec: ParticleSpec, share: f32) -> ParticleSpec {
    spec.delay += spec.lifetime * share;
    spec.lifetime *= 1.0 - share;
    spec
}

/// Moves the particle `distance` along `direction` over its life at a steady pace.
pub(super) fn drift(mut spec: ParticleSpec, direction: Vec3, distance: f32) -> ParticleSpec {
    spec.velocity += direction * (distance / spec.lifetime);
    spec
}

/// Throws the particle so that it has covered `distance` along `direction` when it ends,
/// fast at first and nearly at rest at the end.
pub(super) fn fly(mut spec: ParticleSpec, direction: Vec3, distance: f32) -> ParticleSpec {
    const SLOWDOWN: f32 = 3.0;
    spec.drag = SLOWDOWN / spec.lifetime;
    spec.velocity += direction * (distance * spec.drag / (1.0 - (-SLOWDOWN).exp()));
    spec
}

/// Even positions in `-1..=1`; a single item sits in the middle.
pub(super) fn spread(index: usize, count: usize) -> f32 {
    if count < 2 {
        0.0
    } else {
        2.0 * index as f32 / (count - 1) as f32 - 1.0
    }
}

/// Even positions in `0..=1`; a single item sits at the start.
pub(super) fn share(index: usize, count: usize) -> f32 {
    if count < 2 {
        0.0
    } else {
        index as f32 / (count - 1) as f32
    }
}

/// The ground heading of a horizontal direction.
pub(super) fn heading(direction: Vec3) -> f32 {
    direction.z.atan2(direction.x)
}

/// The unit ground vector at `angle`.
pub(super) fn ground(angle: f32) -> Vec3 {
    Vec3::new(angle.cos(), 0.0, angle.sin())
}

pub(super) fn tagged(mut specs: Vec<ParticleSpec>, source: ParticleSource) -> Vec<ParticleSpec> {
    for spec in &mut specs {
        spec.source = source;
    }
    specs
}

/// What the client observed with one accepted cast.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CastContext {
    /// Where the pattern is anchored: the caster's observed feet position.
    pub origin: Vec3,
    /// Ground direction of the accepted cast.
    pub direction: Vec2,
    /// A recast edge draws `cast.recast`, and never an area (rules E-1 and E-2).
    pub recast: bool,
    /// The instant area of a signed-off first cast, from `geometry::instant_area`.
    pub area: Option<GeoShape>,
    /// The position of the skill's own new effect; `strike_line` ends there.
    pub strike_to: Option<Vec3>,
    /// The action sequence: the `event_id` of every particle and the seed of its jitter.
    pub sequence: u64,
}

impl CastContext {
    /// Direction of an accepted cast: the replicated yaw when the action carries one, else
    /// the way the hero faces.
    pub(crate) fn aim(yaw: Option<f32>, forward: Vec3) -> Vec2 {
        yaw.filter(|yaw| yaw.is_finite())
            .map(|yaw| Vec2::from_array(shared::math::hero_forward(yaw)))
            .or_else(|| forward.xz().try_normalize())
            .unwrap_or(Vec2::X)
    }
}

/// Particles a pattern draws when the row names no `count`.
const fn default_count(pattern: AccentPattern) -> u8 {
    use AccentPattern as P;
    match pattern {
        P::ShieldFlash | P::RuneMark => 1,
        P::DoubleArc | P::MuzzleFlash => 2,
        P::ArcSweep | P::RakeTriple | P::ThrustLine | P::MuzzleBurst | P::TossArc => 3,
        P::GroundRing => 4,
        P::FanSpray | P::GroundSlam | P::RisingMotes => 5,
        P::InwardGather | P::SpiralUp | P::StrikeLine => 6,
        P::None => 0,
    }
}

/// One pattern laid out for one cast: where it is anchored, how far it reaches and what it
/// is painted with.
struct Frame {
    origin: Vec3,
    forward: Vec3,
    side: Vec3,
    angle: f32,
    /// Ground reach of the pattern in units: its base extent times the row scale.
    reach: f32,
    scale: f32,
    life: f32,
    shape: ParticleShape,
    lead: Tint,
    companion: Tint,
    id: u64,
}

impl Frame {
    /// The point `forward` and `side` reaches from the caster, `up` units above the ground.
    fn at(&self, forward: f32, side: f32, up: f32) -> Vec3 {
        self.origin + (self.forward * forward + self.side * side) * self.reach + Vec3::Y * up
    }

    /// The ground direction `turn` radians from the cast direction.
    fn turned(&self, turn: f32) -> Vec3 {
        ground(self.angle + turn)
    }

    /// A particle of the lead shape whose half extent is `radius` units. It starts in the
    /// lead colour and ends in the companion's.
    fn piece(&self, radius: f32, curve: Curve) -> ParticleSpec {
        ParticleSpec {
            event_id: self.id,
            lifetime: self.life,
            size: sized(self.shape, radius, curve),
            angle: self.angle,
            color: self.lead,
            end_color: Some(self.companion),
            shape: self.shape,
            curve,
            ..ParticleSpec::BASE
        }
    }

    /// A lead-shaped particle whose half extent is `radius` reaches.
    fn lead(&self, radius: f32, curve: Curve) -> ParticleSpec {
        self.piece(radius * self.reach, curve)
    }

    /// A soft glint of `radius` reaches in the companion colour.
    fn glint(&self, radius: f32) -> ParticleSpec {
        ParticleSpec {
            event_id: self.id,
            lifetime: self.life,
            size: sized(ParticleShape::Glow, radius * self.reach, Curve::Shrink),
            angle: self.angle,
            color: self.companion,
            ..ParticleSpec::BASE
        }
    }

    /// A ring on the ground around the caster that grows to `radius` reaches.
    fn ring(&self, radius: f32) -> ParticleSpec {
        ParticleSpec {
            event_id: self.id,
            origin: self.origin + Vec3::Y * FLOOR,
            lifetime: self.life,
            size: sized(ParticleShape::Ringlet, radius * self.reach, Curve::Grow),
            angle: self.angle,
            color: self.lead,
            shape: ParticleShape::Ringlet,
            curve: Curve::Grow,
            orient: Orient::Ground,
            ..ParticleSpec::BASE
        }
    }

    fn pattern(
        &self,
        pattern: AccentPattern,
        n: usize,
        strike_to: Option<Vec3>,
    ) -> Vec<ParticleSpec> {
        use AccentPattern as P;
        let mut out = Vec::with_capacity(n);
        match pattern {
            P::ArcSweep => {
                let mut arc = self.lead(0.45, Curve::Pop);
                arc.origin = self.at(0.42, -0.25, CHEST);
                arc.orient = Orient::Ground;
                arc.angle = self.angle - 0.45;
                arc.spin = 0.9 / self.life;
                out.push(drift(arc, self.side, 0.5 * self.reach));
                for i in 1..n {
                    let along = self.turned(spread(i - 1, n - 1) * 0.7);
                    let mut glint = late(self.glint(GLINT), 0.2 * share(i - 1, n - 1));
                    glint.origin = self.origin + along * (0.62 * self.reach) + Vec3::Y * CHEST;
                    out.push(drift(glint, along, 0.16 * self.reach));
                }
            }
            P::DoubleArc => {
                for (i, turn) in [-0.6_f32, 0.6].into_iter().enumerate().take(n) {
                    let mut arc = self.lead(0.45, Curve::Pop);
                    arc.origin = self.at(0.42, 0.0, CHEST);
                    arc.orient = Orient::Ground;
                    arc.angle = self.angle + turn;
                    arc.spin = -2.0 * turn / self.life;
                    // Each arc keeps one colour, so that two cuts are told apart: the
                    // first in the lead colour, the second 80 ms later in the companion's.
                    arc.end_color = None;
                    if i == 1 {
                        arc.color = self.companion;
                        arc = late(arc, (0.08 / self.life).min(0.4));
                    }
                    out.push(arc);
                }
                for i in 2..n {
                    let along = self.turned(spread(i - 2, n - 2) * 0.8);
                    let mut glint = late(self.glint(GLINT), 0.25);
                    glint.origin = self.at(0.42, 0.0, CHEST);
                    out.push(fly(glint, along, 0.3 * self.reach));
                }
            }
            P::RakeTriple => {
                let rakes = n.min(3);
                for i in 0..rakes {
                    let mut rake = late(self.lead(0.26, Curve::Pop), 0.12 * i as f32);
                    rake.origin = self.at(0.3, 0.2 * spread(i, rakes), HAND);
                    rake.orient = Orient::Ground;
                    out.push(drift(rake, self.forward, 0.3 * self.reach));
                }
                for i in 3..n {
                    let mut glint = late(self.glint(0.12), 0.35);
                    glint.origin = self.at(0.6, 0.2 * spread(i - 3, n - 3), HAND);
                    out.push(drift(glint, self.forward, 0.1 * self.reach));
                }
            }
            P::ThrustLine => {
                let mut line = self.lead(0.42, Curve::Stretch);
                line.origin = self.at(0.5, 0.0, HAND);
                line.orient = Orient::Ground;
                out.push(line);
                for i in 1..n {
                    let along = share(i - 1, n - 1);
                    let mut copy = late(self.lead(0.14, Curve::Pop), 0.3 * along);
                    copy.origin = self.at(0.15 + 0.5 * along, 0.0, HAND);
                    copy.orient = Orient::Ground;
                    out.push(drift(copy, self.forward, 0.2 * self.reach));
                }
            }
            P::MuzzleFlash => {
                let mut flash = self.lead(0.4, Curve::Pop);
                flash.origin = self.at(0.45, 0.0, HAND);
                out.push(drift(flash, self.forward, 0.1 * self.reach));
                if n > 1 {
                    let mut glow = self.glint(0.3);
                    glow.origin = self.at(0.3, 0.0, HAND);
                    glow.lifetime *= 0.7;
                    out.push(glow);
                }
                for i in 2..n {
                    let mut spark = late(self.glint(GLINT), 0.15);
                    spark.origin = self.at(0.45, 0.0, HAND);
                    let along = self.turned(spread(i - 2, n - 2) * 0.5);
                    out.push(fly(spark, along, 0.3 * self.reach));
                }
            }
            P::MuzzleBurst => {
                for i in 0..n {
                    let radius = 0.26 * (1.0 - 0.1 * i as f32).max(0.5);
                    let mut shot = late(self.lead(radius, Curve::Pop), 0.45 * share(i, n));
                    shot.origin = self.at(0.25, 0.06 * jitter(self.id, i as u64, 11), HAND);
                    out.push(fly(shot, self.forward, 0.45 * self.reach));
                }
            }
            P::FanSpray => {
                for i in 0..n {
                    let mut piece = late(self.lead(0.2, Curve::Pop), 0.12 * (i % 2) as f32);
                    piece.origin = self.at(0.2, 0.0, HAND);
                    piece.orient = Orient::Velocity;
                    piece.gravity = 3.0;
                    piece.velocity = Vec3::Y;
                    out.push(fly(
                        piece,
                        self.turned(spread(i, n) * 0.5),
                        0.58 * self.reach,
                    ));
                }
            }
            P::GroundRing => {
                out.push(self.ring(0.9));
                for i in 1..n {
                    let outward = self.turned(TAU * (i - 1) as f32 / (n - 1) as f32);
                    let mut mote = self.lead(0.2, Curve::Pop);
                    mote.origin = self.origin + outward * (0.3 * self.reach) + Vec3::Y * FLOOR;
                    mote.orient = Orient::Ground;
                    mote.angle = heading(outward);
                    out.push(drift(mote, outward, 0.42 * self.reach));
                }
            }
            P::GroundSlam => {
                out.push(self.ring(0.8));
                for i in 1..n {
                    let outward = self.turned(TAU * (i - 1) as f32 / (n - 1) as f32 + 0.5);
                    let mut chip = self.lead(0.2, Curve::Pop);
                    chip.origin = self.origin + outward * (0.35 * self.reach) + Vec3::Y * 0.1;
                    chip.orient = Orient::Velocity;
                    chip.angle = heading(outward);
                    chip.gravity = 6.0;
                    chip.velocity = Vec3::Y * (3.4 + 0.5 * jitter(self.id, i as u64, 12));
                    out.push(drift(chip, outward, 0.3 * self.reach));
                }
            }
            P::RisingMotes => {
                let first = jitter(self.id, 0, 13) * PI;
                for i in 0..n {
                    let outward = self.turned(first + TAU * i as f32 / n as f32);
                    let mut mote = late(self.lead(0.2, Curve::Pop), 0.1 * (i % 3) as f32);
                    mote.origin = self.origin
                        + outward * (0.78 * self.reach)
                        + Vec3::Y * (0.15 + 0.12 * (i % 3) as f32);
                    mote.orient = Orient::Velocity;
                    mote.angle = heading(outward);
                    mote.velocity = Vec3::Y * (1.5 + 0.3 * jitter(self.id, i as u64, 14));
                    out.push(mote);
                }
            }
            P::InwardGather => {
                let first = jitter(self.id, 0, 15) * PI;
                for i in 0..n {
                    let outward = self.turned(first + TAU * i as f32 / n as f32);
                    let lift = 0.4 * ((i % 3) as f32 - 1.0);
                    let mut mote = late(self.lead(0.2, Curve::Hold), 0.1 * (i % 4) as f32);
                    mote.origin =
                        self.origin + outward * (0.8 * self.reach) + Vec3::Y * (CHEST + lift);
                    mote.orient = Orient::Velocity;
                    mote.angle = heading(-outward);
                    mote.velocity = Vec3::Y * (-lift / mote.lifetime);
                    out.push(drift(mote, -outward, 0.66 * self.reach));
                }
            }
            P::SpiralUp => {
                for i in 0..n {
                    // One and a quarter turns from the feet to above the head.
                    let rise = share(i, n);
                    let outward = self.turned(rise * TAU * 1.25);
                    let tangent = Vec3::new(-outward.z, 0.0, outward.x);
                    let mut mote = late(self.lead(0.26, Curve::Pop), 0.5 * rise);
                    mote.origin =
                        self.origin + outward * (0.55 * self.reach) + Vec3::Y * (0.2 + 1.7 * rise);
                    mote.orient = Orient::Velocity;
                    mote.angle = heading(tangent);
                    mote.velocity = tangent * (0.25 * self.reach / self.life) + Vec3::Y * 0.8;
                    out.push(mote);
                }
            }
            P::ShieldFlash => {
                let mut plate = self.lead(0.42, Curve::Pop);
                plate.origin = self.at(0.5, 0.0, CHEST);
                // A slow rise stands the plate upright on screen.
                plate.orient = Orient::Velocity;
                plate.velocity = Vec3::Y * 0.15;
                out.push(plate);
                for i in 1..n {
                    let side = if i % 2 == 1 { 1.0 } else { -1.0 };
                    let height = (CHEST + 0.3 - 0.3 * ((i - 1) / 2) as f32).max(0.1);
                    let mut glint = late(self.glint(GLINT), 0.15);
                    glint.origin = self.at(0.5, 0.3 * side, height);
                    out.push(drift(glint, self.side * side, 0.1 * self.reach));
                }
            }
            // Rule E-4: `count` copies evenly on the ring, each pointing outward.
            P::RuneMark if n == 1 => {
                let mut mark = self.lead(0.45, Curve::Pop);
                mark.origin = self.origin + Vec3::Y * RUNE;
                mark.orient = Orient::Ground;
                mark.spin = 2.5;
                out.push(mark);
            }
            P::RuneMark => {
                for i in 0..n {
                    let turn = TAU * i as f32 / n as f32;
                    let outward = self.turned(turn);
                    let tangent = Vec3::new(-outward.z, 0.0, outward.x);
                    let mut mark = self.lead(0.32, Curve::Pop);
                    mark.origin = self.origin + outward * (0.55 * self.reach) + Vec3::Y * RUNE;
                    mark.orient = Orient::Ground;
                    mark.angle = self.angle + turn;
                    // The ring turns: a mark slides along its tangent and keeps pointing out.
                    mark.spin = 0.25 / 0.55 / self.life;
                    out.push(drift(mark, tangent, 0.25 * self.reach));
                }
            }
            P::TossArc => {
                for i in 0..n {
                    let mut piece = late(self.lead(0.18, Curve::Pop), 0.5 * share(i, n));
                    // Alternating sides, so a staggered toss does not read as a slanted line.
                    let lane = i.div_ceil(2) as f32 * if i % 2 == 0 { 1.0 } else { -1.0 };
                    piece.origin = self.at(0.2, 0.07 * lane, HAND);
                    piece.orient = Orient::Velocity;
                    piece.gravity = 9.0;
                    piece.velocity = self.forward * (0.55 * self.reach / self.life) + Vec3::Y * 3.0;
                    out.push(piece);
                }
            }
            P::StrikeLine => {
                let Some(end) = strike_to.filter(|end| end.is_finite()) else {
                    return out;
                };
                let line = (end - self.origin).with_y(0.0);
                let Some(along) = line.try_normalize() else {
                    return out;
                };
                // The marks tile the segment and never reach past either end; on a line
                // longer than the row covers they keep their size and stand apart.
                let radius = (STRIKE_MARK * self.scale).min(0.5 * line.length() / n as f32);
                for i in 0..n {
                    // The crack runs out from the caster and is whole while it cools.
                    let mut mark = late(self.piece(radius, Curve::Pop), STRIKE_RUN * share(i, n));
                    mark.origin =
                        self.origin.lerp(end, (i as f32 + 0.5) / n as f32) + Vec3::Y * FLOOR;
                    mark.orient = Orient::Ground;
                    mark.angle = heading(along);
                    out.push(mark);
                }
            }
            P::None => {}
        }
        out
    }

    /// The instant area of a signed-off first cast (rule E-2): a ring held at the exact
    /// radius with the other particles on it, or the marks of a sector on its arc and
    /// edges. Outline marks keep their ground position and point along the cast direction.
    fn outline(
        &self,
        pattern: AccentPattern,
        n: usize,
        area: &GeoShape,
    ) -> Option<Vec<ParticleSpec>> {
        let mark = |point: Vec2| {
            let mut mark = self.piece(0.22 * self.scale, Curve::Pop);
            mark.origin = Vec3::new(point.x, self.origin.y + 0.1, point.y);
            mark.orient = Orient::Ground;
            if pattern == AccentPattern::GroundSlam {
                // Thrown straight up from the outline.
                mark.velocity = Vec3::Y * 2.0;
                mark.gravity = 6.0;
            }
            mark
        };
        match *area {
            GeoShape::Ring { center, radius } => {
                let mut out = vec![ParticleSpec {
                    event_id: self.id,
                    origin: Vec3::new(center.x, self.origin.y + FLOOR, center.y),
                    lifetime: self.life,
                    size: sized(ParticleShape::Ringlet, radius, Curve::Hold),
                    angle: self.angle,
                    color: self.lead,
                    shape: ParticleShape::Ringlet,
                    curve: Curve::Hold,
                    orient: Orient::Ground,
                    ..ParticleSpec::BASE
                }];
                out.extend((1..n).map(|i| {
                    let turn = self.angle + TAU * (i - 1) as f32 / (n - 1) as f32;
                    mark(center + Vec2::from_angle(turn) * radius)
                }));
                Some(out)
            }
            GeoShape::Sector {
                apex,
                axis,
                radius,
                half_angle,
            } => {
                let ray = |share: f32| Vec2::from_angle(share * half_angle).rotate(axis);
                let arc = [-1.0, -0.5, 0.0, 0.5, 1.0].map(|share| apex + ray(share) * radius);
                let edges = [
                    apex,
                    apex + ray(-1.0) * (0.5 * radius),
                    apex + ray(1.0) * (0.5 * radius),
                ];
                Some(arc.into_iter().chain(edges).take(n).map(mark).collect())
            }
            _ => None,
        }
    }
}

/// The accent of one accepted cast. A recast draws the row's `recast` pattern with that
/// pattern's own default lead, count and colours (rule E-1); without one it repeats the
/// first-cast accent. An area is outlined only on a first cast.
pub(crate) fn accent_particles(
    accent: &CastAccent,
    palette: &Palette,
    ctx: &CastContext,
) -> Vec<ParticleSpec> {
    if !(ctx.origin.is_finite() && ctx.direction.is_finite()) {
        return Vec::new();
    }
    let recast = accent.recast.filter(|_| ctx.recast).map(CastAccent::plain);
    let accent = recast.as_ref().unwrap_or(accent);
    let Some(shape) = accent.lead() else {
        return Vec::new();
    };
    let pattern = accent.pattern;
    let count = usize::from(accent.count.unwrap_or(default_count(pattern))).clamp(1, ACCENT_MAX);
    let [lead, companion] = palette.pair(accent.slots);
    let direction = ctx.direction.try_normalize().unwrap_or(Vec2::X);
    let scale = accent.scale.clamp(0.4, 2.0);
    let frame = Frame {
        origin: ctx.origin,
        forward: Vec3::new(direction.x, 0.0, direction.y),
        side: Vec3::new(-direction.y, 0.0, direction.x),
        angle: direction.to_angle(),
        reach: pattern.base_extent() * scale,
        scale,
        life: accent.lifetime.clamp(0.12, ACCENT_SECS),
        shape,
        lead,
        companion,
        id: ctx.sequence,
    };
    let outline = ctx
        .area
        .filter(|_| accent.area && !ctx.recast)
        .and_then(|area| frame.outline(pattern, count, &area));
    tagged(
        outline.unwrap_or_else(|| frame.pattern(pattern, count, ctx.strike_to)),
        ParticleSource::Accent,
    )
}

/// The veil of `veil_step`: how many pieces hang on the travelled line, how far each drifts
/// along it, and the half extent of a piece in units, on a short step and at most.
const VEIL_PIECES: usize = 3;
const VEIL_DRIFT: f32 = 0.3;
const VEIL_PIECE: f32 = 0.45;
const VEIL_PIECE_MAX: f32 = 0.9;

/// The lead shape of a move when the row names none.
const fn move_lead(pattern: MovePattern) -> ParticleShape {
    match pattern {
        MovePattern::Afterimage => ParticleShape::Streak,
        MovePattern::BlinkPair => ParticleShape::Star,
        MovePattern::LeapArc | MovePattern::ChargeDust => ParticleShape::Glow,
        MovePattern::WhirlStep | MovePattern::VeilStep => ParticleShape::Crescent,
    }
}

/// How a displacement is painted. `pattern` is `None` for a forced one.
struct Step {
    pattern: Option<MovePattern>,
    shape: ParticleShape,
    lead: Tint,
    companion: Tint,
    /// The matter colour: dust and dark pieces.
    matter: Tint,
    id: u64,
}

impl Step {
    /// A lead-shaped particle of half extent `radius` at `origin`.
    fn piece(&self, origin: Vec3, radius: f32, life: f32) -> ParticleSpec {
        ParticleSpec {
            event_id: self.id,
            origin,
            lifetime: life,
            size: sized(self.shape, radius, Curve::Pop),
            color: self.lead,
            end_color: Some(self.companion),
            shape: self.shape,
            curve: Curve::Pop,
            ..ParticleSpec::BASE
        }
    }

    fn glow(&self, origin: Vec3, radius: f32, life: f32, color: Tint) -> ParticleSpec {
        ParticleSpec {
            event_id: self.id,
            origin,
            lifetime: life,
            size: sized(ParticleShape::Glow, radius, Curve::Shrink),
            color,
            ..ParticleSpec::BASE
        }
    }

    /// A ring on the ground at `at`: it grows to `radius`, or closes from it.
    fn ring(&self, at: Vec3, radius: f32, life: f32, curve: Curve, color: Tint) -> ParticleSpec {
        ParticleSpec {
            event_id: self.id,
            origin: at + Vec3::Y * FLOOR,
            lifetime: life,
            size: sized(ParticleShape::Ringlet, radius, curve),
            color,
            shape: ParticleShape::Ringlet,
            curve,
            orient: Orient::Ground,
            ..ParticleSpec::BASE
        }
    }

    /// `count` directions evenly around a point. The first one comes from the seed alone,
    /// so the set says nothing about where the hero came from or went.
    fn around(&self, count: usize) -> impl Iterator<Item = Vec3> {
        let first = jitter(self.id, 0, 21) * PI;
        (0..count).map(move |i| ground(first + TAU * i as f32 / count as f32))
    }

    /// What is left where a hero vanished toward a place the client cannot see.
    fn departure(&self, from: Vec3) -> Vec<ParticleSpec> {
        let mut out = vec![self.ring(from, 0.5, 0.35, Curve::Grow, self.matter)];
        out.extend(self.around(3).map(|outward| {
            let mut mote = self.piece(from + outward * 0.3 + Vec3::Y * 0.4, 0.14, 0.35);
            mote.orient = Orient::Velocity;
            mote.angle = heading(outward);
            mote.velocity = Vec3::Y * 1.2;
            mote
        }));
        out
    }

    /// The half of the pattern at the observed destination. It starts `wait` seconds late.
    fn arrival(&self, to: Vec3, wait: f32) -> Vec<ParticleSpec> {
        use MovePattern as M;
        let chest = to + Vec3::Y * 0.8;
        let scatter = |count: usize, height: f32, distance: f32, life: f32| -> Vec<ParticleSpec> {
            self.around(count)
                .map(|outward| {
                    let mut piece = self.piece(to + Vec3::Y * height, 0.14, life);
                    piece.orient = Orient::Velocity;
                    piece.angle = heading(outward);
                    piece.velocity = Vec3::Y * 0.8;
                    fly(piece, outward, distance)
                })
                .collect()
        };
        let mut out = match self.pattern {
            Some(M::Afterimage) => vec![
                self.glow(chest, 0.6, 0.25, self.companion),
                self.ring(to, 0.7, 0.4, Curve::Grow, self.lead),
            ],
            Some(M::BlinkPair) => {
                let mut out = vec![
                    self.glow(chest, 0.7, 0.25, self.companion),
                    self.ring(to, 0.8, 0.4, Curve::Grow, self.lead),
                ];
                out.extend(scatter(3, 0.8, 0.8, 0.4));
                out
            }
            Some(M::LeapArc) => {
                let mut out = vec![self.ring(to, 0.9, 0.45, Curve::Grow, self.lead)];
                out.extend(scatter(5, 0.15, 0.9, 0.45));
                out
            }
            Some(M::ChargeDust) => {
                let mut out = vec![self.ring(to, 0.8, 0.4, Curve::Grow, self.lead)];
                out.extend(scatter(3, 0.15, 0.6, 0.4));
                out
            }
            Some(M::WhirlStep) => vec![
                self.glow(chest, 0.5, 0.3, self.companion),
                self.ring(to, 0.6, 0.35, Curve::Grow, self.lead),
            ],
            Some(M::VeilStep) => {
                // The body re-forms: pieces close in on the landing, dark at first and in
                // the colour of the skill when they meet. No ring is drawn around the
                // hero: a dark ring at its feet read as a mark on it.
                self.around(4)
                    .map(|outward| {
                        let mut piece = self.piece(chest + outward * 0.8, 0.3, 0.4);
                        piece.orient = Orient::Velocity;
                        piece.angle = heading(-outward);
                        piece.color = self.matter;
                        piece.end_color = Some(self.lead);
                        drift(piece, -outward, 0.5)
                    })
                    .collect()
            }
            None => self
                .around(2)
                .map(|outward| {
                    let puff = self.glow(to + Vec3::Y * 0.15, 0.2, 0.4, self.matter);
                    drift(puff, outward, 0.3)
                })
                .collect(),
        };
        for spec in &mut out {
            spec.delay += wait;
        }
        out
    }

    /// The half of the pattern at the origin and along the travelled line, and the wait
    /// before the arrival half starts.
    fn path(&self, from: Vec3, to: Vec3) -> (Vec<ParticleSpec>, f32) {
        use MovePattern as M;
        let line = (to - from).with_y(0.0);
        let length = line.length();
        let along = line.try_normalize().unwrap_or(Vec3::X);
        let side = Vec3::new(-along.z, 0.0, along.x);
        let steps = |spacing: f32, most: usize| ((length / spacing).ceil() as usize).clamp(2, most);
        let on_line = |i: usize, count: usize| from.lerp(to, (i as f32 + 0.5) / count as f32);
        match self.pattern {
            Some(M::Afterimage) => {
                let count = steps(1.5, 5);
                let out = (0..count)
                    .map(|i| {
                        let mut image = self.piece(on_line(i, count) + Vec3::Y * 0.85, 0.55, 0.38);
                        image.angle = heading(along);
                        image.curve = Curve::Stretch;
                        image.size = sized(self.shape, 0.55, Curve::Stretch);
                        image.delay = 0.03 * i as f32;
                        drift(image, along, 0.4)
                    })
                    .collect();
                (out, 0.03 * count as f32)
            }
            Some(M::BlinkPair) => {
                // Nothing is drawn between the two ends.
                let mut out = vec![self.ring(from, 0.8, 0.3, Curve::Shrink, self.lead)];
                out.extend([-1.0, 1.0].map(|way| {
                    let mut piece =
                        self.piece(from + side * (0.6 * way) + Vec3::Y * 0.8, 0.14, 0.25);
                    piece.orient = Orient::Velocity;
                    piece.angle = heading(side * -way);
                    drift(piece, side * -way, 0.5)
                }));
                (out, 0.05)
            }
            Some(M::LeapArc) => (
                vec![
                    self.ring(from, 0.6, 0.35, Curve::Grow, self.matter),
                    self.glow(from + Vec3::Y * 0.3, 0.35, 0.25, self.companion),
                ],
                0.08,
            ),
            Some(M::ChargeDust) => {
                let count = steps(2.0, 4);
                let out = (0..count)
                    .map(|i| {
                        let offset = side * if i % 2 == 0 { 0.25 } else { -0.25 };
                        let mut dust =
                            self.piece(on_line(i, count) + offset + Vec3::Y * 0.12, 0.16, 0.45);
                        dust.orient = Orient::Velocity;
                        dust.angle = heading(along);
                        dust.color = self.matter;
                        dust.velocity = Vec3::Y * 0.6;
                        dust.delay = 0.04 * i as f32;
                        dust
                    })
                    .collect();
                (out, 0.04 * count as f32)
            }
            Some(M::WhirlStep) => {
                let out = (0..6)
                    .map(|i| {
                        let turn = i as f32 * 2.1;
                        let mut piece = self.piece(
                            on_line(i, 6)
                                + side * (0.45 * turn.sin())
                                + Vec3::Y * (0.7 + 0.45 * turn.cos()),
                            0.16,
                            0.4,
                        );
                        piece.orient = Orient::Velocity;
                        piece.angle = heading(along);
                        piece.velocity = side * (0.8 * turn.cos()) + Vec3::Y * 0.5;
                        piece.delay = 0.035 * i as f32;
                        piece
                    })
                    .collect();
                (out, 0.2)
            }
            Some(M::VeilStep) => {
                // The body dissolves where it stood, and the veil it leaves hangs along
                // the line it travelled: pieces in the colour of the skill that darken,
                // the nearest to the origin first. On a long step they are as large as
                // their third of the line allows, so that the line is read at a glance;
                // none reaches past either end of it.
                let radius = (length / VEIL_PIECES as f32 * 0.5 - VEIL_DRIFT)
                    .clamp(VEIL_PIECE, VEIL_PIECE_MAX);
                let mut out = vec![self.glow(from + Vec3::Y * 0.8, 0.6, 0.4, self.matter)];
                out.extend((0..VEIL_PIECES).map(|i| {
                    let mut piece =
                        self.piece(on_line(i, VEIL_PIECES) + Vec3::Y * 0.8, radius, 0.45);
                    piece.orient = Orient::Velocity;
                    piece.angle = heading(along);
                    piece.curve = Curve::Shrink;
                    piece.size = sized(self.shape, radius, Curve::Shrink);
                    piece.end_color = Some(self.matter);
                    piece.velocity = Vec3::Y * 0.4;
                    piece.delay = 0.04 * i as f32;
                    drift(piece, along, VEIL_DRIFT)
                }));
                (out, 0.1)
            }
            None => {
                // Skid marks on the ground along the real displacement, end to end; a
                // displacement too short to mark leaves only the dust of the arrival. Each
                // mark is three furrows with hard edges whose points lead the way the hero
                // went: a soft streak is lost on pale stone. On a long displacement they
                // cover more than half of the line, so that where the hero was thrown from
                // and to is read at a glance.
                let count = if length < 0.05 { 0 } else { steps(1.0, 6) };
                let radius = (0.5 * length / count.max(1) as f32).min(SKID);
                let out = (0..count)
                    .map(|i| ParticleSpec {
                        event_id: self.id,
                        origin: on_line(i, count) + Vec3::Y * FLOOR,
                        lifetime: 0.55,
                        delay: 0.01 * i as f32,
                        size: sized(ParticleShape::Claw, radius, Curve::Hold),
                        angle: heading(along),
                        color: self.matter,
                        shape: ParticleShape::Claw,
                        curve: Curve::Hold,
                        orient: Orient::Ground,
                        ..ParticleSpec::BASE
                    })
                    .collect();
                (out, 0.01 * count as f32)
            }
        }
    }

    /// Both positions are observed ones. A hero that was not visible before the move gets
    /// the arrival half only; one that is no longer visible after it gets a puff that
    /// points nowhere.
    fn particles(&self, from: Option<Vec3>, to: Option<Vec3>) -> Vec<ParticleSpec> {
        let seen = |point: Option<Vec3>| point.filter(|point| point.is_finite());
        match (seen(from), seen(to)) {
            (None, None) => Vec::new(),
            (Some(from), None) => self.departure(from),
            (None, Some(to)) => self.arrival(to, 0.0),
            (Some(from), Some(to)) => {
                let (mut out, wait) = self.path(from, to);
                out.extend(self.arrival(to, wait));
                out
            }
        }
    }
}

/// Particles of a hero whose own cast moved it between two observed positions, in the
/// colours of the skill.
pub(crate) fn move_particles(
    spec: &MoveSpec,
    palette: &Palette,
    from: Option<Vec3>,
    to: Option<Vec3>,
    seed: u64,
) -> Vec<ParticleSpec> {
    let step = Step {
        pattern: Some(spec.pattern),
        shape: spec.shape.unwrap_or(move_lead(spec.pattern)),
        lead: palette.slot(PaletteSlot::Primary),
        companion: palette.slot(PaletteSlot::Accent),
        matter: palette.slot(PaletteSlot::Secondary),
        id: seed,
    };
    tagged(step.particles(from, to), ParticleSource::Move)
}

/// Neutral ground skid marks of a hero that something else displaced. No afterimage and no
/// colour of any skill: the client does not know what moved it.
pub(crate) fn drag_streak(from: Option<Vec3>, to: Option<Vec3>, seed: u64) -> Vec<ParticleSpec> {
    // Far darker than the pale stone it is drawn on, and still no colour of any skill.
    let dust = Tint {
        color: Color::srgba(0.25, 0.23, 0.2, 0.9),
        gain: 1.0,
    };
    let step = Step {
        pattern: None,
        shape: ParticleShape::Glow,
        lead: dust,
        companion: dust,
        matter: dust,
        id: seed,
    };
    tagged(step.particles(from, to), ParticleSource::Move)
}

/// Streaks from the caster to the position of one accepted receipt. They arrive exactly at
/// the receipt position when they end and never wait for it.
pub(crate) fn link_particles(
    shape: ParticleShape,
    palette: &Palette,
    from: Vec3,
    to: Vec3,
    seed: u64,
) -> Vec<ParticleSpec> {
    if !(from.is_finite() && to.is_finite()) {
        return Vec::new();
    }
    let start = from + Vec3::Y * HAND;
    let line = to + Vec3::Y * HAND - start;
    let side = Vec3::new(-line.z, 0.0, line.x).normalize_or_zero();
    let specs = (0..LINK_MAX)
        .map(|i| {
            let delay = 0.03 * i as f32;
            let lifetime = LINK_SECS - delay;
            ParticleSpec {
                event_id: seed,
                // The streaks leave side by side and meet at the receipt.
                origin: start + side * (0.12 * spread(i, LINK_MAX)),
                velocity: (line - side * (0.12 * spread(i, LINK_MAX))) / lifetime,
                lifetime,
                delay,
                size: sized(shape, 0.3 - 0.06 * i as f32, Curve::Hold),
                angle: heading(line),
                color: palette.slot(PaletteSlot::Primary),
                end_color: Some(palette.slot(PaletteSlot::Accent)),
                shape,
                curve: Curve::Hold,
                orient: Orient::Velocity,
                ..ParticleSpec::BASE
            }
        })
        .collect();
    tagged(specs, ParticleSource::Link)
}

/// One-shots of a classified transition or end of a replicated effect. None of them is an
/// impact recipe, and none is drawn on a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OneShot {
    /// The effect armed.
    ArmPop,
    /// The cage lost the side with this index.
    SegmentSnap(u8),
    /// A travelling body turned or was renewed.
    TurnSpark,
    Fade,
    Crumble,
    Discharge,
    Detonate,
}

impl OneShot {
    /// The one-shot a row's `expire` kind draws on a classified end.
    pub(crate) const fn of(expire: ExpireKind) -> Option<Self> {
        match expire {
            ExpireKind::None => None,
            ExpireKind::Fade => Some(Self::Fade),
            ExpireKind::Crumble => Some(Self::Crumble),
            ExpireKind::Discharge => Some(Self::Discharge),
            ExpireKind::Detonate => Some(Self::Detonate),
        }
    }

    /// The one-shot of a classified end: the row's `expire` kind when it names what was
    /// observed. A release is a discharge, a burst zone a detonation, and only an effect
    /// whose time ran out fades or crumbles; a row that names another kind draws nothing.
    pub(crate) const fn of_end(end: EndKind, expire: ExpireKind) -> Option<Self> {
        match (end, expire) {
            (EndKind::Released, ExpireKind::Discharge) => Some(Self::Discharge),
            (EndKind::Detonated, ExpireKind::Detonate) => Some(Self::Detonate),
            (EndKind::TrueExpiry, ExpireKind::Fade) => Some(Self::Fade),
            (EndKind::TrueExpiry, ExpireKind::Crumble) => Some(Self::Crumble),
            _ => None,
        }
    }
}

/// The largest circle that lies inside a boundary: its centre and radius.
fn inner_circle(geo: &GeoShape) -> Option<(Vec2, f32)> {
    match *geo {
        GeoShape::Ring { center, radius } => Some((center, radius)),
        GeoShape::Pentagon { center, radius } => Some((center, radius * (PI / 5.0).cos())),
        GeoShape::Capsule { from, to, radius } => Some((from.midpoint(to), radius)),
        GeoShape::Lane {
            from,
            to,
            half_width,
        } => Some((from.midpoint(to), half_width.min(0.5 * from.distance(to)))),
        GeoShape::Sector {
            apex,
            axis,
            radius,
            half_angle,
        } => {
            // The circle that touches both edges and the arc.
            let sine = half_angle.min(PI / 2.0).sin();
            let fit = radius * sine / (1.0 + sine);
            Some((apex + axis * (radius - fit), fit))
        }
        GeoShape::Segment { .. } | GeoShape::None => None,
    }
}

/// `count` points spread over the inside of a boundary, each at least `margin` from its edge.
fn inner_points(geo: &GeoShape, count: usize, margin: f32) -> Vec<Vec2> {
    let line = |from: Vec2, to: Vec2| -> Vec<Vec2> {
        (0..count)
            .map(|i| from.lerp(to, (i as f32 + 0.5) / count as f32))
            .collect()
    };
    match *geo {
        GeoShape::Capsule { from, to, .. }
        | GeoShape::Lane { from, to, .. }
        | GeoShape::Segment { from, to } => line(from, to),
        GeoShape::Sector {
            apex, axis, radius, ..
        } => line(
            apex + axis * margin.min(0.5 * radius),
            apex + axis * (radius - margin).max(0.5 * radius),
        ),
        GeoShape::Ring { .. } | GeoShape::Pentagon { .. } => {
            let Some((centre, radius)) = inner_circle(geo) else {
                return Vec::new();
            };
            let ring = (radius - margin).max(0.0) * 0.75;
            (0..count)
                .map(|i| centre + Vec2::from_angle(TAU * i as f32 / count as f32) * ring)
                .collect()
        }
        GeoShape::None => Vec::new(),
    }
}

/// The one-shot of one stage event, drawn inside the replicated geometry of the effect in
/// the colours of its skill. `height` is the ground level under the effect.
pub(crate) fn stage_oneshot(
    kind: OneShot,
    palette: &Palette,
    geo: &GeoShape,
    height: f32,
    seed: u64,
) -> Vec<ParticleSpec> {
    let lead = palette.slot(PaletteSlot::Primary);
    let spark = palette.slot(PaletteSlot::Accent);
    let matter = palette.slot(PaletteSlot::Secondary);
    let lift = |point: Vec2, up: f32| Vec3::new(point.x, height + up, point.y);
    let base = ParticleSpec {
        event_id: seed,
        ..ParticleSpec::BASE
    };
    let ring = |centre: Vec2, radius: f32, curve: Curve, lifetime: f32, color: Tint| ParticleSpec {
        origin: lift(centre, FLOOR),
        lifetime,
        size: sized(ParticleShape::Ringlet, radius, curve),
        color,
        shape: ParticleShape::Ringlet,
        curve,
        orient: Orient::Ground,
        ..base.clone()
    };
    let glow = |origin: Vec3, radius: f32, lifetime: f32, color: Tint| ParticleSpec {
        origin,
        lifetime,
        size: sized(ParticleShape::Glow, radius, Curve::Shrink),
        color,
        ..base.clone()
    };
    let bar = |from: Vec2, to: Vec2, lifetime: f32, color: Tint| ParticleSpec {
        origin: lift(from.midpoint(to), FLOOR),
        lifetime,
        size: sized(ParticleShape::Streak, 0.5 * from.distance(to), Curve::Hold),
        angle: (to - from).to_angle(),
        color,
        shape: ParticleShape::Streak,
        curve: Curve::Hold,
        orient: Orient::Ground,
        ..base.clone()
    };
    let specs = match kind {
        OneShot::ArmPop => {
            let Some((centre, radius)) = inner_circle(geo) else {
                return Vec::new();
            };
            let radius = radius.min(1.0);
            let mut out = vec![
                ring(centre, radius, Curve::Grow, 0.3, lead),
                glow(lift(centre, 0.4), 0.3 * radius, 0.2, spark),
            ];
            out.extend((0..4).map(|i| {
                let outward = Vec2::from_angle(TAU * i as f32 / 4.0 + jitter(seed, 0, 31));
                let mut mote = glow(
                    lift(centre + outward * (0.4 * radius), 0.2),
                    0.1 * radius,
                    0.35,
                    spark,
                );
                mote.velocity = Vec3::Y * 1.4;
                mote
            }));
            out
        }
        OneShot::TurnSpark => {
            let Some((centre, radius)) = inner_circle(geo) else {
                return Vec::new();
            };
            let radius = radius.min(0.6);
            let mut out = vec![ParticleSpec {
                origin: lift(centre, 0.0),
                lifetime: 0.25,
                size: sized(ParticleShape::Star, radius, Curve::Pop),
                color: spark,
                shape: ParticleShape::Star,
                curve: Curve::Pop,
                ..base.clone()
            }];
            out.extend((0..3).map(|i| {
                let outward = Vec2::from_angle(TAU * i as f32 / 3.0 + jitter(seed, 0, 32) * PI);
                let mote = glow(lift(centre, 0.0), 0.15 * radius, 0.3, lead);
                fly(mote, Vec3::new(outward.x, 0.0, outward.y), 0.8 * radius)
            }));
            out
        }
        OneShot::SegmentSnap(index) => {
            let GeoShape::Pentagon { center, radius } = *geo else {
                return Vec::new();
            };
            let corner = |i: u8| center + Vec2::from_angle(TAU * f32::from(i % 5) / 5.0) * radius;
            let (from, to) = (corner(index), corner(index % 5 + 1));
            let middle = from.midpoint(to);
            let inward = (center - middle).normalize_or_zero();
            // The two halves of the bar sink a little toward the inside.
            let nudge = inward * 0.12;
            let mut out: Vec<_> = [(from, middle), (middle, to)]
                .into_iter()
                .map(|(a, b)| {
                    let mut half = bar(a.lerp(b, 0.1) + nudge, a.lerp(b, 0.9) + nudge, 0.35, lead);
                    half.origin.y = height + 0.5;
                    half.gravity = 4.0;
                    half
                })
                .collect();
            out.extend((0..4).map(|i| {
                let way = (inward + inward.perp() * (0.5 * spread(i, 4))).normalize_or_zero();
                let mut chip = ParticleSpec {
                    origin: lift(middle + inward * 0.25, 0.6),
                    lifetime: 0.45,
                    size: sized(ParticleShape::Diamond, 0.12, Curve::Pop),
                    color: lead,
                    end_color: Some(matter),
                    shape: ParticleShape::Diamond,
                    curve: Curve::Pop,
                    orient: Orient::Velocity,
                    gravity: 6.0,
                    ..base.clone()
                };
                chip.velocity = Vec3::Y * 1.5;
                fly(chip, Vec3::new(way.x, 0.0, way.y), 0.3 * radius)
            }));
            out
        }
        OneShot::Fade => {
            // Soft and grey: a dissipation must not read as a hit.
            let mist = Tint {
                color: matter
                    .color
                    .mix(&Color::srgb(0.6, 0.6, 0.6), 0.5)
                    .with_alpha(0.7),
                gain: 1.0,
            };
            let radius = inner_circle(geo).map_or(0.2, |(_, radius)| (0.3 * radius).min(0.25));
            let points = inner_points(geo, FADE_WISPS, radius);
            points
                .iter()
                .enumerate()
                .map(|(i, point)| {
                    let mut wisp = glow(lift(*point, 0.3), radius, 0.5, mist);
                    wisp.velocity = Vec3::Y * 0.5;
                    // From both ends toward the middle: the order in which a strip
                    // dissolves must not say which of its ends was the origin.
                    wisp.delay = FADE_STEP_SECS * i.min(points.len() - 1 - i) as f32;
                    wisp
                })
                .collect()
        }
        OneShot::Crumble => inner_points(geo, 7, 0.15)
            .into_iter()
            .enumerate()
            .map(|(i, point)| ParticleSpec {
                origin: lift(point, 0.6 + 0.12 * (i % 3) as f32),
                velocity: Vec3::Y * 0.5,
                lifetime: 0.5,
                delay: 0.02 * (i % 4) as f32,
                size: sized(ParticleShape::Diamond, 0.13, Curve::Pop),
                angle: jitter(seed, i as u64, 33) * PI,
                color: lead,
                end_color: Some(matter),
                shape: ParticleShape::Diamond,
                gravity: 9.0,
                spin: 3.0,
                curve: Curve::Pop,
                ..base.clone()
            })
            .collect(),
        OneShot::Discharge => match *geo {
            GeoShape::Sector {
                apex,
                axis,
                radius,
                half_angle,
            } => {
                // Rays across the cone, from near its apex to just inside its arc.
                (0..7)
                    .map(|i| {
                        let ray = Vec2::from_angle(0.8 * half_angle * spread(i, 7)).rotate(axis);
                        let mut flash = bar(
                            apex + ray * (0.2 * radius),
                            apex + ray * (0.9 * radius),
                            DISCHARGE_SECS,
                            lead,
                        );
                        flash.end_color = Some(spark);
                        flash
                    })
                    .collect()
            }
            GeoShape::Segment { from, to }
            | GeoShape::Capsule { from, to, .. }
            | GeoShape::Lane { from, to, .. } => (0..5)
                .map(|i| {
                    let (a, b) = (i as f32 / 5.0, (i as f32 + 0.8) / 5.0);
                    let mut flash = bar(from.lerp(to, a), from.lerp(to, b), DISCHARGE_SECS, lead);
                    flash.end_color = Some(spark);
                    flash
                })
                .collect(),
            GeoShape::Ring { .. } | GeoShape::Pentagon { .. } => {
                let Some((centre, radius)) = inner_circle(geo) else {
                    return Vec::new();
                };
                // The boundary itself flashes, with sparks rising inside it.
                let mut out = vec![ring(centre, radius, Curve::Hold, DISCHARGE_SECS, lead)];
                out.extend(inner_points(geo, 6, 0.3 * radius).into_iter().map(|point| {
                    let mut mote = glow(
                        lift(point, 0.2),
                        (0.15 * radius).min(0.25),
                        DISCHARGE_SECS,
                        spark,
                    );
                    mote.velocity = Vec3::Y * 1.2;
                    mote
                }));
                out
            }
            GeoShape::None => Vec::new(),
        },
        OneShot::Detonate => {
            let GeoShape::Ring { center, radius } = *geo else {
                return Vec::new();
            };
            // The ring grows to the boundary and the sparks stop short of it.
            let piece = (0.1 * radius).min(0.15);
            let mut out = vec![ring(center, radius, Curve::Grow, 0.35, lead)];
            out.extend((0..7).map(|i| {
                let outward = Vec2::from_angle(TAU * i as f32 / 7.0 + jitter(seed, 0, 34) * PI);
                let mut mote = glow(
                    lift(center + outward * (0.25 * radius), 0.3),
                    piece,
                    0.45,
                    spark,
                );
                mote.velocity = Vec3::Y * 0.8;
                fly(
                    mote,
                    Vec3::new(outward.x, 0.0, outward.y),
                    (0.7 * radius - piece).max(0.0),
                )
            }));
            out
        }
    };
    tagged(specs, ParticleSource::Stage)
}

/// The snap of a trap that triggered on a receipt without damage. It is a cue, not an
/// impact: it closes inward at the receipt position and throws nothing out.
pub(crate) fn trap_cue(palette: &Palette, at: Vec3, seed: u64) -> Vec<ParticleSpec> {
    if !at.is_finite() {
        return Vec::new();
    }
    let lead = palette.slot(PaletteSlot::Primary);
    let base = ParticleSpec {
        event_id: seed,
        lifetime: 0.3,
        ..ParticleSpec::BASE
    };
    let mut out = vec![
        ParticleSpec {
            origin: at + Vec3::Y * FLOOR,
            size: sized(ParticleShape::Ringlet, 0.7, Curve::Shrink),
            color: lead,
            shape: ParticleShape::Ringlet,
            orient: Orient::Ground,
            ..base.clone()
        },
        ParticleSpec {
            origin: at + Vec3::Y * 0.3,
            lifetime: 0.2,
            size: sized(ParticleShape::Glow, 0.25, Curve::Shrink),
            color: palette.slot(PaletteSlot::Accent),
            ..base.clone()
        },
    ];
    let first = jitter(seed, 0, 41) * PI;
    out.extend((0..4).map(|i| {
        let outward = ground(first + TAU * i as f32 / 4.0);
        let jaw = ParticleSpec {
            origin: at + outward * 0.6 + Vec3::Y * 0.15,
            size: sized(ParticleShape::Chevron, 0.15, Curve::Pop),
            angle: heading(-outward),
            color: lead,
            end_color: Some(palette.slot(PaletteSlot::Secondary)),
            shape: ParticleShape::Chevron,
            curve: Curve::Pop,
            orient: Orient::Ground,
            ..base.clone()
        };
        fly(jaw, -outward, 0.45)
    }));
    tagged(out, ParticleSource::Cue)
}

/// Rising drops over a neutral camp that a hero with a camp bonus hit: two, or four on a
/// kill, in the spark colour of the hero's class.
pub(crate) fn camp_hit(color: Tint, at: Vec3, kill: bool, seed: u64) -> Vec<ParticleSpec> {
    if !at.is_finite() {
        return Vec::new();
    }
    let count = if kill { 4 } else { 2 };
    let specs = (0..count)
        .map(|i| ParticleSpec {
            event_id: seed,
            origin: at
                + Vec3::new(
                    0.3 * spread(i, count),
                    1.2,
                    0.15 * jitter(seed, i as u64, 51),
                ),
            velocity: Vec3::Y * 1.6,
            lifetime: CUE_SECS - 0.04 * i as f32,
            delay: 0.04 * i as f32,
            size: sized(ParticleShape::Drop, 0.28, Curve::Pop),
            color,
            shape: ParticleShape::Drop,
            drag: 1.0,
            curve: Curve::Pop,
            orient: Orient::Velocity,
            ..ParticleSpec::BASE
        })
        .collect();
    tagged(specs, ParticleSource::Cue)
}

/// A link is drawn for receipts that arrive with the snapshot of the cast or one of the two
/// after it, for at most three receipts of one cast.
const LINK_SNAPSHOTS: u64 = 2;
/// A thrown body counts as in flight while it was in one of this many newest snapshots: its
/// receipt may come with the snapshot that drops it.
const BODY_SNAPSHOTS: u64 = 2;
const LINKS_PER_CAST: usize = 3;
/// Casts that may wait for their receipts at one time.
const OPEN_LINKS: usize = 16;

/// The live effect of a skill that its owner's newest cast created.
fn own_effect(effects: &[SkillEffectState], owner: u64, id: SkillId) -> Option<&SkillEffectState> {
    effects
        .iter()
        .filter(|effect| {
            owner != 0
                && effect.owner_id == owner
                && effect.skill == id
                && category::own_kinds(id).contains(&effect.kind)
        })
        .max_by_key(|effect| effect.id)
}

/// The accent of one observed cast, from the row of its skill or basic attack. A row without
/// the block draws nothing here. The accent sits where the hit is resolved from: a skill
/// that moves first is anchored at the observed arrival, never at the place it left
/// (rule E-3). `appeared` are the effects that were not in the previous snapshot; a strike
/// line ends at the caster's own new effect and at nothing else.
pub(crate) fn cast_burst(
    registry: &SkillPresentation,
    cast: &SkillCastObserved,
    appeared: &[SkillEffectState],
) -> Vec<ParticleSpec> {
    let Some((accent, palette)) = registry
        .look(cast.key)
        .and_then(|look| look.accent.zip(Some(look.palette)))
    else {
        return Vec::new();
    };
    let skill = cast.key.skill().and_then(|key| key.modular());
    let anchor = match skill.map(|id| category::strike_origin(id, cast.recast)) {
        Some(StrikeOrigin::Arrival) => cast.position,
        // A skill that strikes from its effect is still cast, and charged, at the caster.
        Some(StrikeOrigin::Origin | StrikeOrigin::EffectPosition) | None => cast.origin,
    };
    let direction = CastContext::aim(cast.yaw, cast.forward);
    let area = skill
        .filter(|id| accent.area && !cast.recast && category::AREA_FLASH_SIGNED_OFF.contains(id))
        .and_then(|id| {
            geometry::instant_area(
                id,
                &AreaContext {
                    origin: cast.origin.xz(),
                    arrival: Some(cast.position.xz()),
                    direction,
                    recast: cast.recast,
                },
            )
        });
    let strike_to = skill
        .filter(|id| category::own_effect_strike(*id))
        .and_then(|id| own_effect(appeared, cast.actor_id, id))
        .map(|effect| Vec3::new(effect.position[0], anchor.y, effect.position[1]));
    accent_particles(
        accent,
        &palette,
        &CastContext {
            origin: anchor,
            direction,
            recast: cast.recast,
            area,
            strike_to,
            sequence: cast.sequence,
        },
    )
}

/// How an observed relocation is painted when it is not left to the utility dash: the move
/// pattern of the row whose cast moved the hero, or neutral skid marks for a displacement
/// by something else. `None` keeps the built-in look.
pub(crate) fn move_burst(
    registry: Option<&SkillPresentation>,
    moved: &MoveObserved,
) -> Option<Vec<ParticleSpec>> {
    match moved.cause {
        MoveCause::Recall | MoveCause::UtilityDash => None,
        MoveCause::Forced => Some(drag_streak(moved.from, moved.to, moved.seed)),
        MoveCause::SkillCast => {
            let look = registry?.look(CastKey::Skill(moved.skill?))?;
            let spec = look.accent?.movement.as_ref()?;
            Some(move_particles(
                spec,
                &look.palette,
                moved.from,
                moved.to,
                moved.seed,
            ))
        }
    }
}

/// The cue of a trap that snapped on a receipt without damage, in the colours of the row
/// that set it. A row without a `cast` block draws none.
pub(crate) fn trap_snap(
    registry: &SkillPresentation,
    key: CastKey,
    at: Vec3,
    receipt: u64,
) -> Vec<ParticleSpec> {
    match (key, registry.look(key)) {
        (CastKey::Skill(_), Some(look)) if look.accent.is_some() => {
            trap_cue(&look.palette, at, receipt)
        }
        _ => Vec::new(),
    }
}

/// The one-shot of one stage event. Only a row that gives the effect a body draws one. The
/// flip of a warning pops when it sets a travelling body off; the beam a warning becomes is
/// its own read.
pub(crate) fn stage_shot(registry: &SkillPresentation, event: &StageEvent) -> Option<OneShot> {
    let effect = &event.effect;
    let body = registry.body_for(effect)?;
    // Rule F: a cone the fog cut down to a line is not known whole, and says nothing when
    // it goes.
    let cut = body.archetype == Archetype::Sector
        && !matches!(
            geometry::boundary_shape(effect.skill, effect.kind, effect),
            GeoShape::Sector { .. }
        );
    match event.change {
        StageChange::Ended(_) if cut => None,
        StageChange::Transition(Transition::Armed) => Some(OneShot::ArmPop),
        StageChange::Transition(Transition::KindFlipped) => {
            (event.effect.kind == EffectVisualKind::Bolt).then_some(OneShot::ArmPop)
        }
        StageChange::Transition(Transition::SegmentBroken(side)) => {
            Some(OneShot::SegmentSnap(side))
        }
        StageChange::Transition(Transition::Turned | Transition::Renewed) => {
            Some(OneShot::TurnSpark)
        }
        StageChange::Ended(end) => OneShot::of_end(end, body.expire),
    }
}

/// The particles of one stage event: its one-shot inside the replicated geometry of the
/// effect, in the colours of the row its `skill` names. `height` is the level the effect
/// is drawn at: the ground under it, or where its body flies. Never an impact recipe.
pub(crate) fn stage_burst(
    registry: &SkillPresentation,
    event: &StageEvent,
    height: f32,
) -> Vec<ParticleSpec> {
    let effect = &event.effect;
    let Some((shot, look)) = stage_shot(registry, event)
        .zip(registry.look(CastKey::Skill(SkillKey::Modular(effect.skill))))
    else {
        return Vec::new();
    };
    stage_oneshot(
        shot,
        &look.palette,
        &geometry::boundary_shape(effect.skill, effect.kind, effect),
        height,
        effect.id,
    )
}

/// A cast whose row links it to its hits, waiting for their receipts.
struct OpenLink {
    actor_id: u64,
    slot: u8,
    start: Vec3,
    shape: ParticleShape,
    palette: Palette,
    left: usize,
    /// The count of snapshots when the cast was observed.
    opened: u64,
    local: bool,
    /// The skill whose recast opened this link although its first cast throws a body: no
    /// receipt is linked while that body may still be what struck.
    after_body: Option<SkillId>,
}

/// Casts that wait for the receipts of their own source and slot. Nothing here draws
/// without such a receipt.
#[derive(Default)]
pub(crate) struct LinkBook {
    round: Option<(u64, u64)>,
    tick: u64,
    snapshots: u64,
    open: Vec<OpenLink>,
    /// Bodies in flight of the skills whose recast hits at once: owner, skill and the count
    /// of snapshots when the body was last in one.
    flying: Vec<(u64, SkillId, u64)>,
}

impl LinkBook {
    /// Notes the snapshot of this frame and closes the links that waited too long. A new
    /// round closes them all.
    pub(crate) fn turn(&mut self, round: Option<(u64, u64)>, tick: u64) {
        if self.round != round {
            *self = Self {
                round,
                tick,
                ..default()
            };
        } else if self.tick != tick {
            self.tick = tick;
            self.snapshots += 1;
            let now = self.snapshots;
            self.open.retain(|link| now - link.opened <= LINK_SNAPSHOTS);
            self.flying.retain(|(.., seen)| now - seen < BODY_SNAPSHOTS);
        }
    }

    /// Notes the bodies in flight of the snapshot of this frame that a recast link has to
    /// wait for. Call it after `turn`.
    pub(crate) fn sight(&mut self, effects: &[SkillEffectState]) {
        let now = self.snapshots;
        for effect in effects {
            let thrown = matches!(
                effect.kind,
                EffectVisualKind::Bolt | EffectVisualKind::Rocket
            );
            if !thrown || !category::recast_instant_hit(effect.skill) {
                continue;
            }
            let body = (effect.owner_id, effect.skill);
            match self
                .flying
                .iter_mut()
                .find(|(owner, skill, _)| (*owner, *skill) == body)
            {
                Some((.., seen)) => *seen = now,
                None => self.flying.push((body.0, body.1, now)),
            }
        }
    }

    /// Whether the body of `skill` thrown by `owner` was in this snapshot or the one before.
    fn in_flight(&self, owner: u64, skill: SkillId) -> bool {
        self.flying.iter().any(|(by, thrown, seen)| {
            (*by, *thrown) == (owner, skill) && self.snapshots - seen < BODY_SNAPSHOTS
        })
    }

    /// Opens the link of a cast whose row names one. It starts where the hit is resolved
    /// from, and a newer cast of the same slot replaces the older one.
    pub(crate) fn open(&mut self, registry: &SkillPresentation, cast: &SkillCastObserved) {
        let CastKey::Skill(key) = cast.key else {
            return;
        };
        let Some((shape, palette)) = registry
            .look(cast.key)
            .and_then(|look| look.accent?.link.zip(Some(look.palette)))
        else {
            return;
        };
        // The hit of a travelling body comes long after its cast, and so does the hit of a
        // telegraph that fires on its own; a link from the cast would run ahead of either.
        // The one exception is a recast that throws nothing and hits at once: it is linked
        // unless the body of the first cast may still be what strikes.
        let mut after_body = None;
        if category::travelling_body(key) || key.modular().is_some_and(category::strikes_on_release)
        {
            let Some(id) = key.modular().filter(|id| category::recast_instant_hit(*id)) else {
                return;
            };
            // Whatever an earlier recast of the slot left open ends with this edge.
            self.open
                .retain(|open| (open.actor_id, open.slot) != (cast.actor_id, cast.slot));
            if !cast.recast || self.in_flight(cast.actor_id, id) {
                return;
            }
            after_body = Some(id);
        }
        let start = match key
            .modular()
            .map(|id| category::strike_origin(id, cast.recast))
        {
            Some(StrikeOrigin::Arrival) => cast.position,
            // Resolved from the effect when it releases, which no cast edge reports.
            Some(StrikeOrigin::EffectPosition) => return,
            Some(StrikeOrigin::Origin) | None => cast.origin,
        };
        self.wait(OpenLink {
            actor_id: cast.actor_id,
            slot: cast.slot,
            start,
            shape,
            palette,
            left: LINKS_PER_CAST,
            opened: self.snapshots,
            local: cast.local,
            after_body,
        });
    }

    /// Opens the link of a telegraph that was seen to release: its receipts come with the
    /// snapshot that dropped the effect or one of the two after it. The link starts where
    /// the server resolves the strike from, the effect at `ground` or the owner as that
    /// snapshot shows it.
    pub(crate) fn release(
        &mut self,
        registry: &SkillPresentation,
        effect: &SkillEffectState,
        owner: &OwnerSeen,
        ground: Vec3,
    ) {
        let Some(((shape, palette), slot)) = registry
            .look(CastKey::Skill(SkillKey::Modular(effect.skill)))
            .and_then(|look| look.accent?.link.zip(Some(look.palette)))
            .zip(owner.slot)
        else {
            return;
        };
        self.wait(OpenLink {
            actor_id: effect.owner_id,
            slot,
            start: match category::strike_origin(effect.skill, false) {
                StrikeOrigin::EffectPosition => ground,
                StrikeOrigin::Origin | StrikeOrigin::Arrival => owner.position,
            },
            shape,
            palette,
            left: LINKS_PER_CAST,
            opened: self.snapshots,
            local: owner.local,
            after_body: None,
        });
    }

    /// A newer link of the same hero and slot replaces the older one.
    fn wait(&mut self, link: OpenLink) {
        self.open
            .retain(|open| (open.actor_id, open.slot) != (link.actor_id, link.slot));
        if self.open.len() == OPEN_LINKS {
            self.open.remove(0);
        }
        self.open.push(link);
    }

    /// The streaks to one accepted receipt of an open cast and whether the caster is the
    /// local hero; `None` when no cast waits for this source and slot.
    pub(crate) fn link(&mut self, hit: &ConfirmedHit) -> Option<(Vec<ParticleSpec>, bool)> {
        let at = self.open.iter().position(|link| {
            link.actor_id == hit.source && link.slot == hit.slot && link.left > 0
        })?;
        // A receipt that the body of the first cast may have dealt is never drawn as the
        // lash of a recast; a missing lash is the lesser fault.
        if self.open[at]
            .after_body
            .is_some_and(|skill| self.in_flight(hit.source, skill))
        {
            return None;
        }
        let link = &mut self.open[at];
        link.left -= 1;
        Some((
            link_particles(
                link.shape,
                &link.palette,
                link.start,
                hit.position,
                hit.receipt,
            ),
            link.local,
        ))
    }
}

/// A decorative burst with its place in the admission order of the pool: the local hero's
/// first, then the nearest to the local hero.
fn ranked(mut specs: Vec<ParticleSpec>, local: bool, at: Vec3, viewer: Option<Vec3>) -> SkillBurst {
    let key = if local {
        0
    } else {
        1 + viewer.map_or(0.0, |viewer| viewer.distance(at)).min(1.0e4) as u32
    };
    for spec in &mut specs {
        spec.sort_key = key;
    }
    SkillBurst(specs)
}

/// Draws the accent of every cast the observer reported this frame.
pub(crate) fn emit_cast(
    registry: Option<Res<SkillPresentation>>,
    game: Option<Res<GameStateSnapshot>>,
    viewer: Query<&Transform, With<Player>>,
    mut known: Local<HashSet<u64>>,
    mut casts: MessageReader<SkillCastObserved>,
    mut out: MessageWriter<SkillBurst>,
) {
    let effects = game
        .as_deref()
        .map_or(&[][..], |game| game.skill_effects.as_slice());
    match registry {
        Some(registry) if !casts.is_empty() => {
            let appeared: Vec<_> = effects
                .iter()
                .filter(|effect| !known.contains(&effect.id))
                .cloned()
                .collect();
            let viewer = viewer.single().ok().map(|pose| pose.translation);
            for cast in casts.read() {
                let specs = cast_burst(&registry, cast, &appeared);
                if !specs.is_empty() {
                    out.write(ranked(specs, cast.local, cast.position, viewer));
                }
            }
        }
        _ => casts.clear(),
    }
    known.clear();
    known.extend(effects.iter().map(|effect| effect.id));
}

/// Draws every relocation the observer reported this frame that is not a utility dash and
/// notes its hero, so the pool leaves the generic dash out.
pub(crate) fn emit_moves(
    registry: Option<Res<SkillPresentation>>,
    viewer: Query<&Transform, With<Player>>,
    mut themed: ResMut<ThemedDashes>,
    mut moves: MessageReader<MoveObserved>,
    mut out: MessageWriter<SkillBurst>,
) {
    themed.0.clear();
    let viewer = viewer.single().ok().map(|pose| pose.translation);
    for moved in moves.read() {
        let Some(specs) = move_burst(registry.as_deref(), moved) else {
            continue;
        };
        themed.0.insert(moved.actor_id);
        let at = moved.to.or(moved.from).unwrap_or_default();
        out.write(ranked(specs, moved.local, at, viewer));
    }
}

/// The ground under a point of the simulation plane.
fn ground_at(map: Option<&crate::maps::MapLayout>, at: Vec2) -> Vec3 {
    Vec3::new(
        at.x,
        map.map_or(0.0, |map| map.terrain_height_3d(at.x, at.y)),
        at.y,
    )
}

/// Draws the one-shot of every transition and classified end the stage tracker reported
/// this frame. A turn and a renewal of one effect in one snapshot spark once.
pub(crate) fn emit_stage_oneshots(
    registry: Option<Res<SkillPresentation>>,
    map: Option<Res<crate::maps::MapLayout>>,
    viewer: Query<&Transform, With<Player>>,
    mut events: MessageReader<StageEvent>,
    mut out: MessageWriter<SkillBurst>,
) {
    let Some(registry) = registry else {
        events.clear();
        return;
    };
    let viewer = viewer.single().ok().map(|pose| pose.translation);
    let mut sparked = Vec::new();
    for event in events.read() {
        if stage_shot(&registry, event) == Some(OneShot::TurnSpark) {
            if sparked.contains(&event.effect.id) {
                continue;
            }
            sparked.push(event.effect.id);
        }
        // The ground the body of the effect stands on: for a strip whose owner is not seen
        // that may be either of its ends.
        let at = ground_at(map.as_deref(), super::bodies::root_at(&event.effect));
        // A body in flight sparks where it flies, not on the ground under it.
        let lift = registry
            .body_for(&event.effect)
            .map_or(0.0, super::bodies::burst_lift);
        let specs = stage_burst(&registry, event, at.y + lift);
        if !specs.is_empty() {
            let local = event.owner.is_some_and(|owner| owner.local);
            out.write(ranked(specs, local, at, viewer));
        }
    }
}

/// Draws a link from a cast, or from a telegraph that released, to each accepted receipt of
/// its own source and slot.
pub(crate) fn emit_links(
    registry: Option<Res<SkillPresentation>>,
    game: Option<Res<GameStateSnapshot>>,
    map: Option<Res<crate::maps::MapLayout>>,
    viewer: Query<&Transform, With<Player>>,
    mut book: Local<LinkBook>,
    mut casts: MessageReader<SkillCastObserved>,
    mut stages: MessageReader<StageEvent>,
    mut hits: MessageReader<ConfirmedHit>,
    mut out: MessageWriter<SkillBurst>,
) {
    let (Some(registry), Some(game)) = (registry, game) else {
        casts.clear();
        stages.clear();
        hits.clear();
        return;
    };
    book.turn(
        Some((game.meta.server_epoch, game.meta.match_id)),
        game.meta.snapshot_tick,
    );
    book.sight(&game.skill_effects);
    for cast in casts.read() {
        book.open(&registry, cast);
    }
    for event in stages.read() {
        if let (StageChange::Ended(EndKind::Released), Some(owner)) = (event.change, &event.owner) {
            let at = Vec2::from_array(event.effect.position);
            book.release(
                &registry,
                &event.effect,
                owner,
                ground_at(map.as_deref(), at),
            );
        }
    }
    let viewer = viewer.single().ok().map(|pose| pose.translation);
    for hit in hits.read() {
        if let Some((specs, local)) = book.link(hit) {
            out.write(ranked(specs, local, hit.position, viewer));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::geometry::{self, AreaContext};
    use super::super::tests::target::target;
    use super::*;
    use shared::HeroClass;
    use shared::loadout::SkillId;

    const ORIGIN: Vec3 = Vec3::new(4.0, 1.5, -3.0);

    fn tint(color: [f32; 3], gain: f32) -> Tint {
        Tint {
            color: Color::srgb_from_array(color),
            gain,
        }
    }

    fn palette() -> Palette {
        Palette {
            primary: tint([1.0, 0.0, 0.0], 3.0),
            secondary: tint([0.0, 1.0, 0.0], 1.0),
            accent: tint([0.0, 0.0, 1.0], 3.0),
            white: tint([1.0; 3], 3.0),
        }
    }

    fn cast(direction: Vec2) -> CastContext {
        CastContext {
            origin: ORIGIN,
            direction,
            recast: false,
            area: None,
            strike_to: Some(ORIGIN + Vec3::new(direction.x, 0.0, direction.y) * 9.0),
            sequence: 77,
        }
    }

    fn accent(pattern: AccentPattern, shape: ParticleShape, count: u8) -> CastAccent {
        CastAccent {
            shape: Some(shape),
            count: Some(count),
            ..CastAccent::plain(pattern)
        }
    }

    /// Where a particle is on the ground when it ends.
    fn landing(spec: &ParticleSpec) -> Vec2 {
        spec.pose_at(spec.lifetime, true, Quat::IDENTITY)
            .translation
            .truncate()
    }

    #[test]
    fn palette_resolves_the_row_the_theme_and_the_overrides() {
        let registry = target();
        let theme = |class: HeroClass| registry.theme(class).unwrap().clone();
        let strike = registry.row("rampage").unwrap();
        assert!(strike.secondary.is_none() && strike.accent.is_none());
        let palette = Palette::of(strike, &theme(HeroClass::Warrior));
        assert_eq!(
            palette.slot(PaletteSlot::Primary),
            tint(strike.color, strike.hdr_gain)
        );
        // The matter colour is drawn without HDR gain, the spark colour with the skill's.
        assert_eq!(
            palette.slot(PaletteSlot::Secondary),
            tint(theme(HeroClass::Warrior).secondary, 1.0)
        );
        assert_eq!(
            palette.slot(PaletteSlot::Accent),
            tint(theme(HeroClass::Warrior).accent, strike.hdr_gain)
        );
        assert_eq!(
            palette.slot(PaletteSlot::White),
            tint([1.0; 3], strike.hdr_gain)
        );
        let pulse = registry.profile(SkillId::ThunderKick).unwrap();
        assert!(pulse.secondary.is_some());
        assert_eq!(
            Palette::of(pulse, &theme(HeroClass::Stormfist)).slot(PaletteSlot::Secondary),
            tint(pulse.secondary.unwrap(), 1.0)
        );
        let step = registry.profile(SkillId::AnchorStep).unwrap();
        assert_eq!(
            Palette::of(step, &theme(HeroClass::Stormfist)).slot(PaletteSlot::Accent),
            tint(step.accent.unwrap(), step.hdr_gain)
        );
        // A basic attack has no colour of its own.
        let basic = Palette::of_class(&theme(HeroClass::Mage));
        assert_eq!(
            basic.slot(PaletteSlot::Primary),
            basic.slot(PaletteSlot::Accent)
        );
        assert_eq!(basic.slot(PaletteSlot::Secondary).gain, 1.0);
        assert_eq!(
            basic.pair(None),
            [
                basic.slot(PaletteSlot::Primary),
                basic.slot(PaletteSlot::Accent)
            ]
        );
    }

    #[test]
    fn accents_follow_the_accepted_direction_and_carry_the_action_sequence() {
        // The replicated yaw wins; without one the hero's facing is used.
        let yaw = shared::math::hero_yaw_towards(0.6, 0.8);
        assert!(CastContext::aim(Some(yaw), Vec3::X).distance(Vec2::new(0.6, 0.8)) < 1e-5);
        assert_eq!(
            CastContext::aim(None, Vec3::new(0.0, 0.3, -2.0)),
            Vec2::NEG_Y
        );
        assert_eq!(CastContext::aim(Some(f32::NAN), Vec3::NEG_X), Vec2::NEG_X);
        assert_eq!(CastContext::aim(None, Vec3::Y), Vec2::X);

        let direction = Vec2::new(0.6, 0.8);
        let turn = |v: Vec3| Vec3::new(v.x * 0.6 - v.z * 0.8, v.y, v.x * 0.8 + v.z * 0.6);
        for pattern in AccentPattern::ALL {
            let row = accent(*pattern, ParticleShape::Chevron, 7);
            let ahead = accent_particles(&row, &palette(), &cast(Vec2::X));
            let turned = accent_particles(&row, &palette(), &cast(direction));
            assert_eq!(ahead.len(), turned.len(), "{}", pattern.id());
            for (a, b) in ahead.iter().zip(&turned) {
                // The whole accent turns with the cast and nothing else changes.
                assert!(
                    turn(a.origin - ORIGIN).distance(b.origin - ORIGIN) < 1e-4,
                    "{}",
                    pattern.id()
                );
                assert!(
                    turn(a.velocity).distance(b.velocity) < 1e-4,
                    "{}",
                    pattern.id()
                );
                assert!(
                    Vec2::from_angle(a.angle)
                        .rotate(direction)
                        .distance(Vec2::from_angle(b.angle))
                        < 1e-4,
                    "{}",
                    pattern.id()
                );
                assert_eq!(
                    (a.lifetime, a.delay, a.size, a.shape),
                    (b.lifetime, b.delay, b.size, b.shape)
                );
                assert_eq!((b.event_id, b.source), (77, ParticleSource::Accent));
            }
        }
        // A thrust lies on the aim line, ahead of the caster.
        let thrust = accent_particles(
            &accent(AccentPattern::ThrustLine, ParticleShape::Streak, 5),
            &palette(),
            &cast(direction),
        );
        assert_eq!(thrust.len(), 5);
        for spec in &thrust {
            let offset = (spec.origin - ORIGIN).xz();
            assert!(offset.perp_dot(direction).abs() < 1e-4 && offset.dot(direction) > 0.0);
            assert!((spec.angle - direction.to_angle()).abs() < 1e-5);
            assert_eq!(spec.orient, Orient::Ground);
        }
        // Lead-shaped particles start in the first slot and end in the second; a companion
        // glint takes the second.
        let row = CastAccent {
            slots: Some([PaletteSlot::White, PaletteSlot::Secondary]),
            ..accent(AccentPattern::MuzzleFlash, ParticleShape::Kite, 3)
        };
        let flash = accent_particles(&row, &palette(), &cast(Vec2::X));
        let (white, matter) = (
            palette().slot(PaletteSlot::White),
            palette().slot(PaletteSlot::Secondary),
        );
        assert_eq!(
            (flash[0].shape, flash[0].color, flash[0].end_color),
            (ParticleShape::Kite, white, Some(matter))
        );
        assert!(
            flash[1..]
                .iter()
                .all(|spec| spec.shape == ParticleShape::Glow && spec.color == matter)
        );
        // One particle is the lead alone: no glow is added behind a row's back.
        let single = accent_particles(
            &accent(AccentPattern::MuzzleFlash, ParticleShape::Arc, 1),
            &palette(),
            &cast(Vec2::X),
        );
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].shape, ParticleShape::Arc);
        // Nothing is drawn for `none`, or from a position that is not a number.
        assert!(
            accent_particles(
                &CastAccent::plain(AccentPattern::None),
                &palette(),
                &cast(Vec2::X)
            )
            .is_empty()
        );
        let lost = CastContext {
            origin: Vec3::NAN,
            ..cast(Vec2::X)
        };
        assert!(accent_particles(&row, &palette(), &lost).is_empty());
        // A cast without a usable direction still draws, along +X.
        let aimless = CastContext {
            direction: Vec2::ZERO,
            ..cast(Vec2::X)
        };
        assert_eq!(
            accent_particles(&row, &palette(), &aimless),
            accent_particles(&row, &palette(), &cast(Vec2::X))
        );
    }

    #[test]
    fn recast_uses_pattern_default_lead() {
        // Anchor Step: a shield plate on the first cast, a spiral on the recast.
        let row = CastAccent {
            scale: 2.0,
            lifetime: 0.5,
            slots: Some([PaletteSlot::White, PaletteSlot::Secondary]),
            recast: Some(AccentPattern::SpiralUp),
            ..accent(AccentPattern::ShieldFlash, ParticleShape::Kite, 3)
        };
        let again = CastContext {
            recast: true,
            ..cast(Vec2::X)
        };
        let recast = accent_particles(&row, &palette(), &again);
        assert_eq!(recast.len(), 6);
        for spec in &recast {
            // The row's kite, colours, size and timing do not reach the recast (rule E-1).
            assert_eq!(spec.shape, ParticleShape::Star);
            assert_eq!(spec.color, palette().slot(PaletteSlot::Primary));
            assert_eq!(spec.end_color, Some(palette().slot(PaletteSlot::Accent)));
            assert!(spec.end_secs() <= 0.35 + 1e-6);
        }
        for pattern in AccentPattern::ALL {
            let row = CastAccent {
                recast: Some(*pattern),
                ..row.clone()
            };
            assert_eq!(
                accent_particles(&row, &palette(), &again),
                accent_particles(&CastAccent::plain(*pattern), &palette(), &again),
                "{}",
                pattern.id()
            );
            let leads: Vec<_> = accent_particles(&row, &palette(), &again)
                .into_iter()
                .filter(|spec| spec.end_color.is_some())
                .map(|spec| spec.shape)
                .collect();
            assert!(
                leads
                    .iter()
                    .all(|shape| Some(*shape) == pattern.default_lead()),
                "{}",
                pattern.id()
            );
        }
        // `none` silences the recast; the first cast is untouched by the recast field.
        let silent = CastAccent {
            recast: Some(AccentPattern::None),
            ..row.clone()
        };
        assert!(accent_particles(&silent, &palette(), &again).is_empty());
        let first = accent_particles(&row, &palette(), &cast(Vec2::X));
        assert_eq!(first.len(), 3);
        assert_eq!(first[0].shape, ParticleShape::Kite);
        // A row without a recast pattern repeats its accent on every edge.
        let same = CastAccent {
            recast: None,
            ..row
        };
        assert_eq!(accent_particles(&same, &palette(), &again), first);
    }

    #[test]
    fn area_outline_equals_instant_area() {
        let direction = Vec2::new(0.0, 1.0);
        let landing_point = Vec2::new(11.0, -3.0);
        let observed = AreaContext {
            origin: ORIGIN.xz(),
            arrival: Some(landing_point),
            direction,
            recast: false,
        };
        for (id, pattern, lead, centre, radius) in [
            (
                SkillId::ThunderPulse,
                AccentPattern::GroundRing,
                ParticleShape::Ringlet,
                ORIGIN.xz(),
                5.0,
            ),
            (
                SkillId::ChainSweep,
                AccentPattern::GroundRing,
                ParticleShape::Chevron,
                ORIGIN.xz(),
                6.0,
            ),
            (
                SkillId::AnvilCharge,
                AccentPattern::GroundSlam,
                ParticleShape::Diamond,
                landing_point,
                3.0,
            ),
        ] {
            let area = geometry::instant_area(id, &observed);
            assert_eq!(
                area,
                Some(GeoShape::Ring {
                    center: centre,
                    radius
                }),
                "{}",
                id.id()
            );
            let row = CastAccent {
                area: true,
                scale: 0.75,
                lifetime: 0.45,
                ..accent(pattern, lead, 8)
            };
            let ctx = CastContext {
                area,
                ..cast(direction)
            };
            let specs = accent_particles(&row, &palette(), &ctx);
            assert_eq!(specs.len(), 8, "{}", id.id());
            // One ring held at exactly the derived radius for the whole accent.
            let ring = &specs[0];
            assert_eq!(
                (
                    ring.shape,
                    ring.orient,
                    ring.curve,
                    ring.velocity,
                    ring.lifetime
                ),
                (
                    ParticleShape::Ringlet,
                    Orient::Ground,
                    Curve::Hold,
                    Vec3::ZERO,
                    0.45
                )
            );
            assert_eq!(ring.origin.xz(), centre);
            assert_eq!(unit_radius(ring.shape) * ring.size, radius);
            assert_eq!(ring.color, palette().slot(PaletteSlot::Primary));
            // The other seven sit on that outline, evenly, pointing along the cast.
            let mut turns: Vec<f32> = Vec::new();
            for mark in &specs[1..] {
                let offset = mark.origin.xz() - centre;
                assert!((offset.length() - radius).abs() < 1e-4, "{}", id.id());
                assert_eq!(mark.velocity.xz(), Vec2::ZERO);
                assert_eq!((mark.shape, mark.orient), (lead, Orient::Ground));
                assert!((mark.angle - direction.to_angle()).abs() < 1e-5);
                assert_eq!(landing(mark), mark.origin.xz());
                turns.push(offset.to_angle().rem_euclid(TAU));
            }
            turns.sort_by(f32::total_cmp);
            for pair in turns.windows(2) {
                assert!((pair[1] - pair[0] - TAU / 7.0).abs() < 1e-3, "{}", id.id());
            }
            // Only a slam throws its marks up.
            assert_eq!(
                specs[1..].iter().all(|mark| mark.velocity.y > 0.0),
                pattern == AccentPattern::GroundSlam
            );
            // `scale` sizes the marks and cannot move the ring.
            let larger = CastAccent {
                scale: 2.0,
                ..row.clone()
            };
            let scaled = accent_particles(&larger, &palette(), &ctx);
            assert_eq!(scaled[0], specs[0]);
            assert!(scaled[1].size > specs[1].size && scaled[1].origin == specs[1].origin);

            // No area without the observed geometry, without the row's flag, or on a recast
            // (rule E-2): the plain pattern stays at the caster.
            let plain = accent_particles(&row, &palette(), &cast(direction));
            let reach = pattern.base_extent() * 0.75;
            assert!(plain.iter().all(|spec| spec.reach(ORIGIN) <= reach + 1e-4));
            let unflagged = CastAccent {
                area: false,
                ..row.clone()
            };
            assert_eq!(accent_particles(&unflagged, &palette(), &ctx), plain);
            let again = CastContext {
                recast: true,
                ..ctx
            };
            assert_eq!(accent_particles(&row, &palette(), &again), plain);
            let silent = CastAccent {
                recast: Some(AccentPattern::None),
                ..row
            };
            assert!(accent_particles(&silent, &palette(), &again).is_empty());
        }
        // An unobserved landing gives no area, so nothing is drawn at the departure.
        let unseen = AreaContext {
            arrival: None,
            ..observed
        };
        assert_eq!(geometry::instant_area(SkillId::AnvilCharge, &unseen), None);

        // A sector is marked on its arc and edges, never as a full ring.
        let Some(sector) = geometry::instant_area(SkillId::Nightfall, &observed) else {
            panic!("the retreat strike is a sector");
        };
        let GeoShape::Sector {
            apex,
            axis,
            radius,
            half_angle,
        } = sector
        else {
            panic!("the retreat strike is a sector");
        };
        let row = CastAccent {
            area: true,
            ..accent(AccentPattern::FanSpray, ParticleShape::Streak, 8)
        };
        let ctx = CastContext {
            area: Some(sector),
            ..cast(direction)
        };
        let marks = accent_particles(&row, &palette(), &ctx);
        assert_eq!(marks.len(), 8);
        let along = |mark: &ParticleSpec| (mark.origin.xz() - apex).length();
        let off_axis = |mark: &ParticleSpec| (mark.origin.xz() - apex).angle_to(axis).abs();
        for mark in &marks[..5] {
            assert!((along(mark) - radius).abs() < 1e-4 && off_axis(mark) <= half_angle + 1e-4);
        }
        assert!(along(&marks[5]) < 1e-5);
        for mark in &marks[6..] {
            assert!((along(mark) - 0.5 * radius).abs() < 1e-4);
            assert!((off_axis(mark) - half_angle).abs() < 1e-4);
        }
        assert!(marks.iter().all(|mark| mark.shape == ParticleShape::Streak
            && mark.velocity.xz() == Vec2::ZERO));
        // Fewer particles keep the arc first.
        let fewer = CastAccent {
            count: Some(5),
            ..row
        };
        assert_eq!(
            accent_particles(&fewer, &palette(), &ctx),
            marks[..5].to_vec()
        );
    }

    /// The two cuts of a double arc are told apart: the first is the lead colour and the
    /// second, a moment later and turned the other way, the companion's. Neither fades
    /// into the other.
    #[test]
    fn the_arcs_of_a_double_arc_are_one_colour_each() {
        let palette = palette();
        let specs = accent_particles(
            &accent(AccentPattern::DoubleArc, ParticleShape::Claw, 4),
            &palette,
            &cast(Vec2::X),
        );
        assert_eq!(specs.len(), 4);
        let (first, second) = (&specs[0], &specs[1]);
        assert_eq!(
            (first.shape, second.shape),
            (ParticleShape::Claw, ParticleShape::Claw)
        );
        assert_eq!(
            (first.color, first.end_color),
            (palette.slot(PaletteSlot::Primary), None)
        );
        assert_eq!(
            (second.color, second.end_color),
            (palette.slot(PaletteSlot::Accent), None)
        );
        assert_eq!(first.delay, 0.0);
        assert!((second.delay - 0.08).abs() < 1e-6);
        assert!((first.end_secs() - second.end_secs()).abs() < 1e-6);
        // They cross: one starts turned to each side and sweeps toward the other.
        assert!(first.angle < 0.0 && second.angle > 0.0);
        assert!(first.spin > 0.0 && second.spin < 0.0);
    }

    #[test]
    fn rune_mark_lays_its_copies_on_one_ring_pointing_outward() {
        let row = CastAccent {
            scale: 1.4,
            ..accent(AccentPattern::RuneMark, ParticleShape::Chevron, 4)
        };
        let marks = accent_particles(&row, &palette(), &cast(Vec2::X));
        assert_eq!(marks.len(), 4);
        let ring = 0.55 * 0.8 * 1.4;
        for (i, mark) in marks.iter().enumerate() {
            let offset = mark.origin - ORIGIN;
            assert_eq!(mark.shape, ParticleShape::Chevron);
            assert_eq!(mark.orient, Orient::Ground);
            // On the ground around the caster and under the health bars, on the ring, a
            // quarter turn apart, pointing away from it.
            assert!(offset.y > 0.0 && offset.y < 0.3);
            assert!((offset.xz().length() - ring).abs() < 1e-4);
            assert!((offset.xz().to_angle().rem_euclid(TAU) - TAU * i as f32 / 4.0).abs() < 1e-4);
            assert!(Vec2::from_angle(mark.angle).distance(offset.xz().normalize()) < 1e-4);
            assert!(mark.spin > 0.0 && mark.end_color.is_some());
        }
        // One copy is the single mark under the caster.
        let single = CastAccent {
            count: Some(1),
            ..row
        };
        let mark = accent_particles(&single, &palette(), &cast(Vec2::X));
        assert_eq!(mark.len(), 1);
        assert_eq!((mark[0].origin - ORIGIN).xz(), Vec2::ZERO);
        assert!((mark[0].origin - ORIGIN).y < 0.3);
        assert_eq!(mark[0].velocity, Vec3::ZERO);
    }

    #[test]
    fn strike_line_needs_the_own_effect_and_stays_between_the_two_points() {
        let row = accent(AccentPattern::StrikeLine, ParticleShape::Diamond, 8);
        let nowhere = CastContext {
            strike_to: None,
            ..cast(Vec2::X)
        };
        assert!(accent_particles(&row, &palette(), &nowhere).is_empty());
        let at_feet = CastContext {
            strike_to: Some(ORIGIN),
            ..cast(Vec2::X)
        };
        assert!(accent_particles(&row, &palette(), &at_feet).is_empty());
        for length in [0.4, 3.0, 28.0] {
            // The effect may stand anywhere; the cast direction does not bend the line.
            let end = ORIGIN + Vec3::new(-0.6, 0.0, 0.8) * length + Vec3::Y * 0.5;
            let ctx = CastContext {
                strike_to: Some(end),
                ..cast(Vec2::X)
            };
            let marks = accent_particles(&row, &palette(), &ctx);
            assert_eq!(marks.len(), 8);
            let line = (end - ORIGIN).xz().normalize();
            for mark in &marks {
                let offset = (mark.origin - ORIGIN).xz();
                let half = unit_radius(mark.shape) * mark.size;
                assert!(offset.perp_dot(line).abs() < 1e-4);
                assert!(
                    offset.dot(line) - half >= -1e-4 && offset.dot(line) + half <= length + 1e-4
                );
                assert_eq!(mark.velocity, Vec3::ZERO);
                assert!((Vec2::from_angle(mark.angle) - line).length() < 1e-4);
            }
        }
    }

    /// A fault line is one crack, not a row of crumbs: over the 15 units of the skill the
    /// eight marks of the shipped row lie end to end from the caster to the pillar, the
    /// crack has run its length well before it fades, and it ends as one. A longer line
    /// than the row covers keeps the size of its marks and shows gaps instead.
    #[test]
    fn strike_line_marks_join_into_one_crack() {
        let registry = target();
        let row = registry
            .profile(SkillId::FaultLine)
            .and_then(|profile| profile.cast.clone())
            .unwrap();
        assert_eq!(row.pattern, AccentPattern::StrikeLine);
        let laid = |length: f32| {
            let ctx = CastContext {
                strike_to: Some(ORIGIN + Vec3::new(0.8, 0.0, -0.6) * length),
                ..cast(Vec2::X)
            };
            let marks = accent_particles(&row, &palette(), &ctx);
            let line = Vec2::new(0.8, -0.6);
            marks
                .into_iter()
                .map(|mark| {
                    let along = (mark.origin - ORIGIN).xz().dot(line);
                    let half = unit_radius(mark.shape) * mark.size * mark.curve.peak();
                    (along - half, along + half, mark.delay, mark.lifetime)
                })
                .collect::<Vec<_>>()
        };
        // The skill's own length (`common/src/skills/advanced.rs:639-662`) and a short one.
        for length in [15.0, 4.0] {
            let marks = laid(length);
            assert_eq!(marks.len(), 8);
            assert!(marks[0].0.abs() < 1e-4, "{length}");
            assert!((marks[7].1 - length).abs() < 1e-4, "{length}");
            for pair in marks.windows(2) {
                assert!((pair[0].1 - pair[1].0).abs() < 1e-4, "{length}: {pair:?}");
                // It runs away from the caster.
                assert!(pair[1].2 > pair[0].2);
            }
            let life = marks[0].3;
            for (_, _, delay, lifetime) in &marks {
                assert!((delay + lifetime - life).abs() < 1e-5);
                // Whole before a mark begins to fade at half its life.
                assert!(*delay <= STRIKE_RUN * life + 1e-5);
            }
            assert!(marks[7].2 < crate::game_vfx::HOLD_SHARE * life);
        }
        // Past the reach of the row the marks are no longer than its scale allows.
        let far = laid(28.0);
        let half = STRIKE_MARK * row.scale;
        for pair in far.windows(2) {
            assert!((pair[0].1 - pair[0].0 - 2.0 * half).abs() < 1e-4);
            assert!(pair[1].0 - pair[0].1 > 1.0);
        }
        assert!(far[0].0 >= -1e-4 && far[7].1 <= 28.0 + 1e-4);
    }

    #[test]
    fn moves_are_drawn_between_observed_positions_only() {
        let (from, to) = (ORIGIN, ORIGIN + Vec3::new(6.0, 0.0, 8.0));
        let near = |spec: &ParticleSpec, point: Vec3| (spec.origin - point).xz().length();
        for pattern in MovePattern::ALL.iter().copied().map(Some).chain([None]) {
            let draw = |from: Option<Vec3>, to: Option<Vec3>| match pattern {
                Some(pattern) => move_particles(
                    &MoveSpec {
                        pattern,
                        shape: Some(ParticleShape::Crescent),
                    },
                    &palette(),
                    from,
                    to,
                    9,
                ),
                None => drag_streak(from, to, 9),
            };
            let name = pattern.map_or("drag_streak", MovePattern::id);
            assert!(draw(None, None).is_empty(), "{name}");
            assert!(
                draw(Some(Vec3::NAN), Some(Vec3::INFINITY)).is_empty(),
                "{name}"
            );
            let full = draw(Some(from), Some(to));
            assert!(!full.is_empty(), "{name}");
            assert!(
                full.iter()
                    .all(|spec| spec.event_id == 9 && spec.source == ParticleSource::Move),
                "{name}"
            );

            // A hidden destination: a puff at the origin that points nowhere. It is the
            // same puff whatever the pattern does on a full move.
            let departure = draw(Some(from), None);
            assert_eq!(departure.len(), 4, "{name}");
            for spec in &departure {
                assert!(
                    near(spec, from) <= 0.31 && spec.velocity.xz() == Vec2::ZERO,
                    "{name}"
                );
            }
            assert_eq!(draw(Some(from), Some(Vec3::NAN)), departure, "{name}");

            // A hidden origin: the arrival half only, and it is the half a full move draws
            // there, whichever way the hero came.
            let arrival = draw(None, Some(to));
            assert!(!arrival.is_empty() && arrival.len() < full.len(), "{name}");
            assert!(arrival.iter().all(|spec| near(spec, to) <= 0.81), "{name}");
            for other in [from, to + Vec3::new(-20.0, 0.0, 3.0)] {
                let seen = draw(Some(other), Some(to));
                let tail = &seen[seen.len() - arrival.len()..];
                for (a, b) in arrival.iter().zip(tail) {
                    assert_eq!(
                        (a.origin, a.velocity, a.size, a.shape, a.lifetime),
                        (b.origin, b.velocity, b.size, b.shape, b.lifetime),
                        "{name}"
                    );
                }
            }
            // Everything else lies at the origin or along the travelled line.
            let line = (to - from).xz().normalize();
            for spec in &full[..full.len() - arrival.len()] {
                let offset = (spec.origin - from).xz();
                let on_line =
                    offset.perp_dot(line).abs() <= 0.61 && (0.0..=10.0).contains(&offset.dot(line));
                assert!(near(spec, from) <= 0.61 || on_line, "{name}");
            }
            // A move that went nowhere still draws without a direction to divide by.
            assert!(
                draw(Some(from), Some(from))
                    .iter()
                    .all(ParticleSpec::is_sound),
                "{name}"
            );
        }
        // A blink draws nothing between its two ends.
        let blink = move_particles(
            &MoveSpec {
                pattern: MovePattern::BlinkPair,
                shape: None,
            },
            &palette(),
            Some(from),
            Some(to),
            9,
        );
        assert!(
            blink
                .iter()
                .all(|spec| near(spec, from).min(near(spec, to)) <= 0.81)
        );
        assert!(blink.iter().any(|spec| spec.shape == ParticleShape::Star));
        // A forced displacement leaves neutral marks on the ground and no afterimage.
        let skid = drag_streak(Some(from), Some(to), 9);
        let marks: Vec<_> = skid
            .iter()
            .filter(|spec| spec.shape == ParticleShape::Claw)
            .collect();
        assert_eq!(marks.len(), 6);
        let way = (to - from).normalize();
        for mark in &marks {
            assert_eq!(mark.orient, Orient::Ground);
            assert!(mark.origin.y - ORIGIN.y < 0.1 && mark.velocity == Vec3::ZERO);
            // Furrows on the line of the displacement, between its two ends, with their
            // points toward where the hero went.
            let along = (mark.origin - from).dot(way);
            let reach = SKID + 1e-4;
            assert!(along >= reach && along <= (to - from).length() - reach);
            assert!((mark.angle - heading(way)).abs() < 1e-5);
            assert!((mark.size - sized(ParticleShape::Claw, SKID, Curve::Hold)).abs() < 1e-5);
        }
        assert!(
            skid.iter()
                .all(|spec| spec.color.gain == 1.0 && spec.end_color.is_none())
        );
        assert!(skid.iter().all(|spec| spec.color == skid[0].color));
    }

    /// The veil of `veil_step`: a dark dissolve where the hero stood, three pieces on the
    /// line it really travelled that are as large as their third of that line allows, and
    /// pieces that close on the landing. Nothing is a ring at the hero's feet and nothing
    /// lies past either observed end.
    #[test]
    fn the_veil_hangs_on_the_travelled_line() {
        let way = Vec3::new(0.6, 0.0, 0.8);
        let veil = |length: f32| {
            move_particles(
                &MoveSpec {
                    pattern: MovePattern::VeilStep,
                    shape: None,
                },
                &palette(),
                Some(ORIGIN),
                Some(ORIGIN + way * length),
                9,
            )
        };
        let half = |length: f32| {
            (length / VEIL_PIECES as f32 * 0.5 - VEIL_DRIFT).clamp(VEIL_PIECE, VEIL_PIECE_MAX)
        };
        let colours = palette();
        for length in [0.5, 2.0, 4.5, 7.0, 12.0, 40.0] {
            let all = veil(length);
            assert_eq!(all.len(), MOVE_MAX, "{length}");
            assert!(
                all.iter().all(|spec| {
                    spec.is_sound()
                        && spec.delay + spec.lifetime <= MOVE_SECS + 1e-4
                        && spec.shape != ParticleShape::Ringlet
                }),
                "{length}"
            );
            let (dissolve, rest) = all.split_first().unwrap();
            let (pieces, landing) = rest.split_at(VEIL_PIECES);
            // The dissolve is dark and stays where the hero stood.
            assert_eq!(dissolve.shape, ParticleShape::Glow);
            assert_eq!(dissolve.color, colours.secondary);
            assert!((dissolve.origin - ORIGIN).xz().length() < 1e-4);
            assert_eq!(dissolve.velocity.xz(), Vec2::ZERO);
            // The pieces: the lead shape in the colour of the skill, darkening, one after
            // another from the origin, each on its own third of the line.
            for (i, piece) in pieces.iter().enumerate() {
                assert_eq!(piece.shape, ParticleShape::Crescent, "{length}");
                assert_eq!(piece.color, colours.primary, "{length}");
                assert_eq!(piece.end_color, Some(colours.secondary), "{length}");
                assert!((piece.delay - 0.04 * i as f32).abs() < 1e-5, "{length}");
                assert!(
                    (piece.size - sized(ParticleShape::Crescent, half(length), Curve::Shrink))
                        .abs()
                        < 1e-5,
                    "{length}"
                );
                let offset = (piece.origin - ORIGIN).xz();
                assert!(offset.perp_dot(way.xz()).abs() < 1e-4, "{length}");
                let start = offset.dot(way.xz());
                assert!(
                    (start - length * (i as f32 + 0.5) / VEIL_PIECES as f32).abs() < 1e-4,
                    "{length}"
                );
                // From where it appears to where it has drifted, the piece stays between
                // the two observed ends once the step is long enough to hold three.
                let end = start + piece.velocity.xz().dot(way.xz()) * piece.lifetime;
                assert!((end - start - VEIL_DRIFT).abs() < 1e-4, "{length}");
                if length >= 2.0 * VEIL_PIECES as f32 * (VEIL_PIECE + VEIL_DRIFT) {
                    assert!(start - half(length) >= -1e-4, "{length}");
                    assert!(end + half(length) <= length + 1e-4, "{length}");
                }
            }
            // The landing: dark pieces that close on the hero and end in the skill colour.
            assert_eq!(landing.len(), 4, "{length}");
            for piece in landing {
                let from_landing = (piece.origin - (ORIGIN + way * length)).xz();
                assert!((from_landing.length() - 0.8).abs() < 1e-4, "{length}");
                assert!(piece.velocity.xz().dot(from_landing) < 0.0, "{length}");
                assert_eq!(piece.color, colours.secondary, "{length}");
                assert_eq!(piece.end_color, Some(colours.primary), "{length}");
            }
        }
        // A short step keeps the small pieces; a long one grows them up to the cap.
        assert_eq!(half(0.5), VEIL_PIECE);
        assert_eq!(half(4.5), VEIL_PIECE);
        assert!(half(7.0) > 1.8 * VEIL_PIECE && half(7.0) < VEIL_PIECE_MAX);
        assert_eq!(half(12.0), VEIL_PIECE_MAX);
        assert_eq!(half(40.0), VEIL_PIECE_MAX);
    }

    #[test]
    fn links_run_from_the_caster_to_the_receipt_and_end_there() {
        let to = ORIGIN + Vec3::new(-7.0, 0.5, 2.0);
        for shape in ParticleShape::ALL {
            let link = link_particles(*shape, &palette(), ORIGIN, to, 31);
            assert_eq!(link.len(), LINK_MAX);
            for spec in &link {
                assert_eq!(
                    (spec.shape, spec.event_id, spec.source),
                    (*shape, 31, ParticleSource::Link)
                );
                assert_eq!(spec.orient, Orient::Velocity);
                assert!((spec.end_secs() - LINK_SECS).abs() < 1e-6);
                let end = spec.origin + spec.velocity * spec.lifetime;
                assert!(end.distance(to + Vec3::Y * HAND) < 1e-3);
                assert!((spec.origin - ORIGIN).xz().length() <= 0.13);
            }
        }
        assert!(link_particles(ParticleShape::Streak, &palette(), ORIGIN, Vec3::NAN, 1).is_empty());
        // A receipt at the caster's own feet still gives finite streaks.
        assert!(
            link_particles(ParticleShape::Streak, &palette(), ORIGIN, ORIGIN, 1)
                .iter()
                .all(ParticleSpec::is_sound)
        );
    }

    /// Whether a ground point lies inside a boundary, with a small tolerance.
    fn inside(geo: &GeoShape, point: Vec2) -> bool {
        const SLACK: f32 = 1e-3;
        let on_segment = |from: Vec2, to: Vec2, reach: f32| {
            let along = (to - from).normalize_or_zero();
            let t = (point - from).dot(along).clamp(0.0, from.distance(to));
            point.distance(from + along * t) <= reach + SLACK
        };
        match *geo {
            GeoShape::Ring { center, radius } => point.distance(center) <= radius + SLACK,
            GeoShape::Pentagon { center, radius } => (0..5).all(|i| {
                let normal = Vec2::from_angle(TAU * (i as f32 + 0.5) / 5.0);
                (point - center).dot(normal) <= radius * (PI / 5.0).cos() + SLACK
            }),
            GeoShape::Capsule { from, to, radius } => on_segment(from, to, radius),
            GeoShape::Lane {
                from,
                to,
                half_width,
            } => {
                let along = (to - from).normalize();
                let offset = point - from;
                (-SLACK..=from.distance(to) + SLACK).contains(&offset.dot(along))
                    && offset.perp_dot(along).abs() <= half_width + SLACK
            }
            GeoShape::Sector {
                apex,
                axis,
                radius,
                half_angle,
            } => {
                let offset = point - apex;
                offset.length() <= radius + SLACK
                    && (offset.length() < SLACK
                        || offset.angle_to(axis).abs() <= half_angle + SLACK)
            }
            GeoShape::Segment { from, to } => on_segment(from, to, 0.0),
            GeoShape::None => false,
        }
    }

    #[test]
    fn stage_oneshots_stay_inside_the_replicated_geometry() {
        let centre = Vec2::new(ORIGIN.x, ORIGIN.z);
        let far = centre + Vec2::new(9.0, 12.0);
        let mut shapes: Vec<GeoShape> = [0.3, 1.0, 3.0, 8.0]
            .into_iter()
            .map(|radius| GeoShape::Ring {
                center: centre,
                radius,
            })
            .collect();
        shapes.extend([
            GeoShape::Pentagon {
                center: centre,
                radius: 6.0,
            },
            GeoShape::Capsule {
                from: centre,
                to: far,
                radius: 1.2,
            },
            GeoShape::Lane {
                from: centre,
                to: far,
                half_width: 0.8,
            },
            GeoShape::Sector {
                apex: centre,
                axis: Vec2::new(0.6, 0.8),
                radius: 9.0,
                half_angle: 0.6_f32.acos(),
            },
            GeoShape::Segment {
                from: centre,
                to: centre + Vec2::new(0.0, 5.0),
            },
        ]);
        let kinds = [
            OneShot::ArmPop,
            OneShot::TurnSpark,
            OneShot::Fade,
            OneShot::Crumble,
            OneShot::Discharge,
            OneShot::Detonate,
        ]
        .into_iter()
        .chain((0..5).map(OneShot::SegmentSnap));
        for kind in kinds {
            assert!(
                stage_oneshot(kind, &palette(), &GeoShape::None, 0.0, 5).is_empty(),
                "{kind:?}"
            );
            let mut drawn = 0;
            for geo in &shapes {
                let specs = stage_oneshot(kind, &palette(), geo, ORIGIN.y, 5);
                drawn += usize::from(!specs.is_empty());
                assert!(specs.len() <= STAGE_MAX, "{kind:?} {geo:?}");
                for spec in &specs {
                    assert!(
                        spec.is_sound() && spec.end_secs() <= STAGE_SECS,
                        "{kind:?} {geo:?}"
                    );
                    assert_eq!((spec.event_id, spec.source), (5, ParticleSource::Stage));
                    // A one-shot starts and ends inside the boundary and above the ground.
                    assert!(
                        inside(geo, spec.origin.xz()),
                        "{kind:?} {geo:?} starts outside"
                    );
                    assert!(inside(geo, landing(spec)), "{kind:?} {geo:?} ends outside");
                    assert!(spec.origin.y >= ORIGIN.y);
                }
            }
            // A one-shot is drawn only for a boundary it can follow.
            let expected = match kind {
                OneShot::Detonate => 4,
                OneShot::SegmentSnap(_) => 1,
                OneShot::ArmPop | OneShot::TurnSpark => 8,
                OneShot::Fade | OneShot::Crumble | OneShot::Discharge => 9,
            };
            assert_eq!(drawn, expected, "{kind:?}");
        }
        // The rings of a release and of a detonation are the boundary itself.
        for radius in [0.3, 3.0, 8.0] {
            let zone = GeoShape::Ring {
                center: centre,
                radius,
            };
            let flash = &stage_oneshot(OneShot::Discharge, &palette(), &zone, 0.0, 5)[0];
            assert_eq!(
                (flash.shape, flash.curve),
                (ParticleShape::Ringlet, Curve::Hold)
            );
            assert_eq!(unit_radius(flash.shape) * flash.size, radius);
            let burst = stage_oneshot(OneShot::Detonate, &palette(), &zone, 0.0, 5);
            assert_eq!(
                (burst[0].shape, burst[0].curve),
                (ParticleShape::Ringlet, Curve::Grow)
            );
            assert!((burst[0].reach(Vec3::new(centre.x, 0.0, centre.y)) - radius).abs() < 1e-5);
            // The sparks of a detonation stop short of the boundary.
            for spark in &burst[1..] {
                assert!(spark.reach(Vec3::new(centre.x, 0.0, centre.y)) <= radius + 1e-4);
            }
        }
        // Each side of the cage snaps at its own middle.
        let cage = GeoShape::Pentagon {
            center: centre,
            radius: 6.0,
        };
        let middles: Vec<Vec2> = (0..5)
            .map(|side| {
                let specs = stage_oneshot(OneShot::SegmentSnap(side), &palette(), &cage, 0.0, 5);
                specs.iter().map(|spec| spec.origin.xz()).sum::<Vec2>() / specs.len() as f32
            })
            .collect();
        for (side, middle) in middles.iter().enumerate() {
            let expected = TAU * (side as f32 + 0.5) / 5.0;
            assert!(((*middle - centre).to_angle().rem_euclid(TAU) - expected).abs() < 0.02);
        }
        // Every side index of the wire stands for one of the five sides.
        assert_eq!(
            stage_oneshot(OneShot::SegmentSnap(u8::MAX), &palette(), &cage, 0.0, 5),
            stage_oneshot(OneShot::SegmentSnap(0), &palette(), &cage, 0.0, 5)
        );
        // A fade is grey and unlit, so it cannot be taken for a hit.
        for wisp in stage_oneshot(OneShot::Fade, &palette(), &cage, 0.0, 5) {
            assert_eq!(wisp.color.gain, 1.0);
            let colour = wisp.color.color.to_srgba();
            assert!(colour.alpha < 1.0 && colour.red > 0.2 && colour.blue > 0.2);
        }
        assert_eq!(OneShot::of(ExpireKind::None), None);
        assert_eq!(OneShot::of(ExpireKind::Fade), Some(OneShot::Fade));
        assert_eq!(OneShot::of(ExpireKind::Crumble), Some(OneShot::Crumble));
        assert_eq!(OneShot::of(ExpireKind::Discharge), Some(OneShot::Discharge));
        assert_eq!(OneShot::of(ExpireKind::Detonate), Some(OneShot::Detonate));
    }

    /// A strip is received with its caster-side end first. Its fade is the same set of
    /// wisps whichever end comes first, so a strip whose owner the viewer does not see
    /// dissolves without saying where it came from.
    #[test]
    fn the_fade_of_a_strip_tells_no_direction() {
        let near = Vec2::new(ORIGIN.x, ORIGIN.z);
        let far = near + Vec2::new(9.0, 12.0);
        let wisps = |geo: GeoShape| {
            let mut wisps: Vec<_> = stage_oneshot(OneShot::Fade, &palette(), &geo, ORIGIN.y, 5)
                .into_iter()
                .map(|wisp| {
                    (
                        (wisp.origin * 1e3).round().to_array().map(|v| v as i64),
                        (wisp.delay * 1e4).round() as i64,
                        (wisp.velocity * 1e3).round().to_array().map(|v| v as i64),
                    )
                })
                .collect();
            wisps.sort();
            wisps
        };
        for (there, back) in [
            (
                GeoShape::Capsule {
                    from: near,
                    to: far,
                    radius: 1.2,
                },
                GeoShape::Capsule {
                    from: far,
                    to: near,
                    radius: 1.2,
                },
            ),
            (
                GeoShape::Lane {
                    from: near,
                    to: far,
                    half_width: 0.8,
                },
                GeoShape::Lane {
                    from: far,
                    to: near,
                    half_width: 0.8,
                },
            ),
            (
                GeoShape::Segment {
                    from: near,
                    to: far,
                },
                GeoShape::Segment {
                    from: far,
                    to: near,
                },
            ),
        ] {
            let drawn = wisps(there);
            assert_eq!(drawn.len(), FADE_WISPS, "{there:?}");
            assert_eq!(drawn, wisps(back), "{there:?}");
            // The wisps do not start together, so the equality above is about their order
            // too: the first ones stand at both ends of the strip.
            let first: Vec<_> = drawn.iter().filter(|wisp| wisp.1 == 0).collect();
            assert_eq!(first.len(), 2, "{there:?}");
            assert!(drawn.iter().any(|wisp| wisp.1 > 0), "{there:?}");
        }
    }

    #[test]
    fn cues_are_small_and_close_inward() {
        let snap = trap_cue(&palette(), ORIGIN, 3);
        assert_eq!(snap.len(), CUE_MAX);
        for spec in &snap {
            assert!(spec.is_sound() && spec.end_secs() <= CUE_SECS);
            assert_eq!((spec.event_id, spec.source), (3, ParticleSource::Cue));
            // Nothing is thrown out of the trap: every particle ends nearer than it began.
            assert!(spec.reach(ORIGIN) <= 0.76);
            assert!(
                (landing(spec) - ORIGIN.xz()).length()
                    <= (spec.origin - ORIGIN).xz().length() + 1e-5
            );
        }
        assert!(trap_cue(&palette(), Vec3::NAN, 3).is_empty());

        let gold = tint([1.0, 0.8, 0.2], 2.5);
        for (kill, count) in [(false, 2), (true, 4)] {
            let drops = camp_hit(gold, ORIGIN, kill, 8);
            assert_eq!(drops.len(), count);
            for drop in &drops {
                assert_eq!((drop.shape, drop.color), (ParticleShape::Drop, gold));
                assert!(drop.velocity.y > 0.0 && drop.velocity.xz() == Vec2::ZERO);
                assert!(drop.is_sound() && drop.end_secs() <= CUE_SECS);
                assert_eq!(drop.source, ParticleSource::Cue);
            }
        }
        assert!(camp_hit(gold, Vec3::INFINITY, true, 8).is_empty());
    }
}
