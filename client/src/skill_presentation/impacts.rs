//! The pooled-particle burst of one accepted damage receipt. A recipe is drawn at the
//! receipt position only: nothing follows the target, stays on it or depicts a status, and
//! a skill without area damage keeps its whole burst close to the unit it hit. Every burst
//! opens with the flash of the hit, so the frame that shows the receipt shows the burst at
//! its brightest.
use super::SkillPresentation;
use super::accents::{Palette, drift, fly, ground, heading, share, sized, spread, tagged};
use super::cast::CastKey;
use super::category::{self, SkillKey};
pub(crate) use super::schema::ImpactRecipe;
use super::vocab::{ImpactKind, PaletteSlot, ParticleShape};
use crate::game_vfx::{
    Curve, Orient, ParticleSource, ParticleSpec, Tint, blade_depth, jitter, unit_radius,
};
use bevy::prelude::*;
use shared::loadout::{SkillEffectState, SkillId};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

/// Budget of one impact burst (particles, seconds until the last one is gone). The flash
/// of the hit is one of the particles, so a kind draws at most eleven of its own.
pub(crate) const IMPACT_MAX: usize = 12;
pub(crate) const IMPACT_SECS: f32 = 1.7;
/// Debris outlives the authored lifetime by this factor at most, as in the wire-style burst.
const DEBRIS_LIFE: f32 = 1.4;
/// A skill without area damage draws nothing farther than this from the receipt position on
/// the ground plane; vertical travel is free.
pub(crate) const SINGLE_TARGET_REACH: f32 = 1.5;
/// Ground reach of a recipe in units per unit of `scale`. A row of scale 1 reaches the
/// single-target bound; a larger one is drawn there by `contain` unless its skill damages
/// an area.
const REACH: f32 = SINGLE_TARGET_REACH;
/// `blast` is the one kind that reads as an area; only a skill with area damage may use it.
const BLAST_REACH: f32 = 1.6;
/// The ground ring of `thud_ring` never grows past this radius.
const THUD_RING: f32 = 1.0;
/// `drain_wisp` motes never drift farther than this toward the source.
const DRAIN_TRAVEL: f32 = 1.2;
/// Half extent of a thrown spark as a share of the reach of its recipe, and how far its
/// centre is thrown: the spark ends with its rim just inside the reach. The glows of the
/// wire-style burst are as large (0.42 units).
const SPARK: f32 = 0.26;
const SPARK_THROW: f32 = 0.97 - SPARK;
/// Half extent of the one solid mark of `flash_star`, `star_shards` and `facet_pop` as a
/// share of the reach of its recipe: a shape on the unit, not a plate over it.
const MARK: f32 = 0.62;
/// Half extent of the flash of the hit as a share of the reach of its recipe, and the share
/// of the authored lifetime it lasts. It is a dense glow: its middle is covered in the skill
/// colour and only its outer half fades, so the hit has a core on pale ground.
const FLASH: f32 = 0.95;
const FLASH_LIFE: f32 = 0.8;
/// Height of the burst above the receipt position. A receipt is at the aim height of its
/// target; the built-in burst of the wire style is drawn this far above it too, where the
/// camera sees it over the body it hit.
const BODY: f32 = 0.75;

/// What the client knows about one accepted receipt.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ImpactContext {
    /// The receipt position.
    pub position: Vec3,
    /// Height of the ground under the receipt, where the ground rings of a recipe lie.
    pub ground: f32,
    /// Ground direction from the source to the target; zero when the source is not known.
    pub direction: Vec2,
    /// Heading of the live effect of the same owner and skill, when it is in the snapshot.
    /// `pierce_through` continues along it (rule E-15), and continues only then: a body
    /// that is gone did not fly on.
    pub heading: Option<Vec2>,
    /// Whether the skill damages an area; otherwise the burst is held inside
    /// `SINGLE_TARGET_REACH`.
    pub area_damage: bool,
    /// The receipt id: the `event_id` of every particle and the seed of its jitter.
    pub receipt: u64,
}

/// Particles a kind draws when the row names no `count`.
const fn default_count(kind: ImpactKind) -> u8 {
    use ImpactKind as K;
    match kind {
        K::FlashStar => 1,
        K::FacetPop => 2,
        K::PierceThrough | K::ChainSnap => 4,
        K::ClawRake | K::SparkFork | K::StarShards | K::DrainWisp | K::Splinter => 5,
        K::SlashCut | K::GlowPop | K::CrossCut | K::ShardBurst | K::EmberPuff | K::ThudRing => 6,
        K::RingBurst => 8,
        K::Blast => 10,
    }
}

/// One recipe laid out at one receipt.
struct Burst {
    centre: Vec3,
    /// The ground level under the receipt.
    floor: Vec3,
    /// The hit direction: away from the source, or along the body that flies on.
    along: Vec3,
    /// Whether the client sees the body of the skill fly on past this hit.
    onward: bool,
    side: Vec3,
    angle: f32,
    /// Ground reach of the recipe in units.
    reach: f32,
    life: f32,
    shape: ParticleShape,
    lead: Tint,
    companion: Tint,
    /// The colour of the flash of the hit.
    flash: Tint,
    id: u64,
}

impl Burst {
    /// A particle of `shape` whose half extent is `radius` reaches. It starts in the lead
    /// colour and ends in the companion's.
    fn shaped(&self, shape: ParticleShape, radius: f32, curve: Curve) -> ParticleSpec {
        ParticleSpec {
            event_id: self.id,
            origin: self.centre,
            lifetime: self.life,
            size: sized(shape, radius * self.reach, curve),
            angle: self.angle,
            color: self.lead,
            end_color: Some(self.companion),
            shape,
            curve,
            ..ParticleSpec::BASE
        }
    }

    fn lead(&self, radius: f32, curve: Curve) -> ParticleSpec {
        self.shaped(self.shape, radius, curve)
    }

    /// A lead-shaped cut that reaches `extent` reaches from `at`, crosses it, and runs along
    /// the ground angle `axis`. A curved blade runs across its own heading and is drawn
    /// off its centre, so it is set back by its depth and bulges away from the source.
    fn stroke(&self, extent: f32, axis: f32, at: Vec3) -> ParticleSpec {
        let depth = blade_depth(self.shape);
        // A lead that is neither a blade nor a line is a mark on the unit, not a plate
        // across it.
        let narrow =
            depth > 0.0 || matches!(self.shape, ParticleShape::Streak | ParticleShape::Claw);
        let extent = if narrow { extent } else { extent.min(MARK) };
        let mut cut = self.lead(extent, Curve::Pop);
        cut.origin = at;
        cut.angle = axis;
        if depth > 0.0 {
            let across = axis - FRAC_PI_2;
            cut.angle = if (across - self.angle).cos() >= 0.0 {
                across
            } else {
                across + PI
            };
            cut.size = extent * self.reach / (depth + unit_radius(self.shape));
            cut.origin = at - ground(cut.angle) * (depth * cut.size);
        }
        cut
    }

    /// A soft spark of `radius` reaches in the companion colour.
    fn spark(&self, radius: f32) -> ParticleSpec {
        ParticleSpec {
            end_color: None,
            color: self.companion,
            ..self.shaped(ParticleShape::Glow, radius, Curve::Shrink)
        }
    }

    /// The flash of the hit: a glow in the skill colour that is whole on the frame the
    /// receipt is drawn and shrinks from there. The marks and the debris of a kind need a
    /// moment to swell or to fly out; the hit itself has none to give.
    fn flash(&self) -> ParticleSpec {
        ParticleSpec {
            color: self.flash,
            end_color: None,
            lifetime: self.life * FLASH_LIFE,
            dense: true,
            ..self.shaped(ParticleShape::Glow, FLASH, Curve::Shrink)
        }
    }

    /// A ring in the lead colour that grows to `radius` units.
    fn ring(&self, origin: Vec3, radius: f32, orient: Orient) -> ParticleSpec {
        ParticleSpec {
            event_id: self.id,
            origin,
            lifetime: self.life,
            size: sized(ParticleShape::Ringlet, radius, Curve::Grow),
            color: self.lead,
            shape: ParticleShape::Ringlet,
            curve: Curve::Grow,
            orient,
            ..ParticleSpec::BASE
        }
    }

    /// The direction of debris `index` of `count`, evenly around the receipt from a seeded
    /// start.
    fn outward(&self, index: usize, count: usize) -> Vec3 {
        ground(jitter(self.id, 0, 61) * PI + TAU * index as f32 / count.max(1) as f32)
    }

    /// Sparks thrown outward to the reach of the recipe that live as long as debris.
    fn sparks(&self, count: usize, out: &mut Vec<ParticleSpec>) {
        out.extend((0..count).map(|i| {
            let mut spark = self.spark(SPARK);
            spark.lifetime = self.life * DEBRIS_LIFE;
            spark.velocity = Vec3::Y * (0.4 + 0.6 * (i % 3) as f32);
            fly(spark, self.outward(i, count), SPARK_THROW * self.reach)
        }));
    }

    fn particles(&self, kind: ImpactKind, n: usize) -> Vec<ParticleSpec> {
        use ImpactKind as K;
        // Ring kinds keep their ring; a named lead shapes their debris, and the default
        // lead leaves soft glows.
        let debris = match self.shape {
            ParticleShape::Ringlet => ParticleShape::Glow,
            shape => shape,
        };
        let mut out = Vec::with_capacity(n);
        match kind {
            K::RingBurst => {
                out.push(self.ring(self.centre, self.reach, Orient::Billboard));
                for i in 0..n - 1 {
                    let mut mote = self.shaped(debris, 0.18, Curve::Pop);
                    mote.lifetime = self.life * DEBRIS_LIFE;
                    mote.orient = Orient::Velocity;
                    mote.velocity = Vec3::Y * (0.4 + 0.6 * (i % 3) as f32);
                    out.push(fly(mote, self.outward(i, n - 1), 0.8 * self.reach));
                }
            }
            K::SlashCut => {
                out.push(self.stroke(0.95, self.angle + 1.0, self.centre));
                if n > 1 {
                    let mut core = self.spark(0.4);
                    core.color = self.lead;
                    core.lifetime = self.life * 0.65;
                    out.push(core);
                }
                self.sparks(n.saturating_sub(2), &mut out);
            }
            K::GlowPop => {
                let curve = if self.shape == ParticleShape::Glow {
                    Curve::Shrink
                } else {
                    Curve::Pop
                };
                out.push(self.lead(0.6, curve));
                self.sparks(n - 1, &mut out);
            }
            K::CrossCut => {
                for (i, turn) in [FRAC_PI_4, -FRAC_PI_4].into_iter().enumerate().take(n) {
                    let cut = self.stroke(0.9, self.angle + turn, self.centre);
                    out.push(held(cut, 0.12 * self.life * i as f32));
                }
                self.sparks(n.saturating_sub(2), &mut out);
            }
            K::ClawRake => {
                let marks = n.min(3);
                let rake = self.angle + 0.9;
                for i in 0..marks {
                    let lane = ground(rake + FRAC_PI_2) * (0.38 * spread(i, marks) * self.reach);
                    let mut mark = self.stroke(0.5, rake, self.centre + lane);
                    mark.velocity = ground(rake) * (0.12 * self.reach / self.life);
                    out.push(held(mark, 0.1 * self.life * i as f32));
                }
                self.sparks(n.saturating_sub(3), &mut out);
            }
            K::PierceThrough if self.onward => {
                // Everything moves on past the target; nothing is thrown back or sideways.
                let mut streak = self.lead(0.6, Curve::Stretch);
                streak.origin = self.centre + self.along * (0.15 * self.reach);
                streak.velocity = self.along * (0.2 * self.reach / self.life);
                out.push(streak);
                for i in 1..n {
                    let mut tail = held(
                        self.lead(0.18, Curve::Pop),
                        0.1 * self.life * share(i - 1, n - 1),
                    );
                    tail.origin = self.centre
                        + self.along * (0.1 * self.reach)
                        + self.side * (0.08 * self.reach * jitter(self.id, i as u64, 62));
                    tail.orient = Orient::Velocity;
                    out.push(fly(tail, self.along, 0.65 * self.reach));
                }
            }
            K::PierceThrough => {
                // No body is seen to fly on, so the mark stays where it struck.
                out.push(self.lead(0.6, Curve::Stretch));
                for i in 1..n {
                    let mut mark = held(
                        self.lead(0.18, Curve::Pop),
                        0.1 * self.life * share(i - 1, n - 1),
                    );
                    mark.origin =
                        self.centre + self.along * (0.4 * self.reach * spread(i - 1, n - 1));
                    out.push(mark);
                }
            }
            K::SparkFork => {
                let forks = n.min(3);
                for i in 0..forks {
                    let way = ground(self.angle + 0.6 * spread(i, forks));
                    let mut fork = self.lead(0.36, Curve::Pop);
                    fork.orient = Orient::Velocity;
                    fork.angle = heading(way);
                    out.push(fly(fork, way, 0.6 * self.reach));
                }
                self.sparks(n.saturating_sub(3), &mut out);
            }
            K::ShardBurst => {
                for i in 0..n {
                    let mut shard = self.lead(0.24, Curve::Pop);
                    shard.lifetime = self.life * 1.3;
                    shard.orient = Orient::Velocity;
                    shard.gravity = 9.0;
                    shard.velocity = Vec3::Y * (3.0 + 0.5 * jitter(self.id, i as u64, 63));
                    out.push(fly(shard, self.outward(i, n), 0.72 * self.reach));
                }
            }
            K::FlashStar => {
                // One mark and its echoes, all at rest: no debris. The flash of the hit is
                // the width of the burst; the mark stays a shape on the unit it struck.
                for i in 0..n {
                    let mut flash = held(
                        self.lead(MARK * 0.6_f32.powi(i as i32), Curve::Pop),
                        0.1 * self.life * i as f32,
                    );
                    flash.angle = self.angle + PI / 4.0 * i as f32;
                    flash.spin = 0.6 / self.life;
                    out.push(flash);
                }
            }
            K::EmberPuff => {
                for i in 0..n {
                    let outward = self.outward(i, n);
                    let mut ember = self.lead(0.22, Curve::Pop);
                    ember.lifetime = self.life * DEBRIS_LIFE;
                    ember = held(ember, 0.3 * self.life * share(i, n));
                    ember.origin = self.centre + outward * (0.25 * self.reach * (i % 2) as f32);
                    ember.orient = Orient::Velocity;
                    // A steady drift sideways under a faster, quickening rise.
                    ember.gravity = -1.0;
                    ember.velocity = Vec3::Y * (2.2 + 0.5 * jitter(self.id, i as u64, 64));
                    out.push(drift(ember, outward, 0.35 * self.reach));
                }
            }
            K::StarShards => {
                out.push(self.lead(MARK, Curve::Pop));
                for i in 1..n {
                    let outward = self.outward(i - 1, n - 1);
                    let mut chip = self.shaped(ParticleShape::Diamond, 0.2, Curve::Pop);
                    chip.color = self.companion;
                    chip.end_color = Some(self.lead);
                    chip.lifetime = self.life * 1.3;
                    chip.origin = self.centre + outward * (0.3 * self.reach);
                    chip.orient = Orient::Velocity;
                    chip.gravity = 9.0;
                    chip.velocity = Vec3::Y;
                    out.push(fly(chip, outward, 0.35 * self.reach));
                }
            }
            K::ThudRing => {
                let ground_ring = self.floor + Vec3::Y * 0.06;
                out.push(self.ring(ground_ring, self.reach.min(THUD_RING), Orient::Ground));
                for i in 0..n - 1 {
                    let mut dust = self.shaped(debris, 0.2, Curve::Pop);
                    dust.lifetime = self.life * 1.2;
                    dust.origin = self.floor + Vec3::Y * 0.15;
                    dust.orient = Orient::Velocity;
                    dust.gravity = 5.0;
                    dust.velocity = Vec3::Y * 0.9;
                    out.push(fly(dust, self.outward(i, n - 1), 0.7 * self.reach));
                }
            }
            K::FacetPop => {
                // Rule E-15: the facet lies along the hit direction, turned a little by the
                // receipt id. Nothing moves.
                let lie = self.angle + 0.35 * jitter(self.id, 0, 65);
                let mut facet = self.lead(MARK, Curve::Pop);
                facet.angle = lie;
                out.push(facet);
                if n > 1 {
                    let mut ring = self.shaped(ParticleShape::Ringlet, 0.5, Curve::Pop);
                    ring.color = self.companion;
                    ring.end_color = None;
                    ring.lifetime = self.life * 0.8;
                    out.push(ring);
                }
                for i in 2..n {
                    let step = (i - 1) as f32;
                    let mut echo = held(
                        self.lead(0.35 * 0.8_f32.powi(i as i32 - 2), Curve::Pop),
                        0.08 * self.life * step,
                    );
                    echo.angle = lie + 1.1 * step;
                    out.push(echo);
                }
            }
            K::Blast => {
                let mut flash = self.shaped(ParticleShape::Glow, 0.7, Curve::Shrink);
                flash.end_color = None;
                out.push(flash);
                if n > 1 {
                    // The ground ring swells to its radius in the first moments and holds
                    // it: the area is read while the burst is bright, not as it fades.
                    let mut ring = self.ring(
                        self.floor + Vec3::Y * 0.08,
                        0.95 * self.reach,
                        Orient::Ground,
                    );
                    ring.color = self.companion;
                    ring.curve = Curve::Pop;
                    ring.size = sized(ParticleShape::Ringlet, 0.95 * self.reach, Curve::Pop);
                    out.push(ring);
                }
                for i in 2..n {
                    let mut piece = self.lead(0.16, Curve::Pop);
                    piece.lifetime = self.life * DEBRIS_LIFE;
                    piece.orient = Orient::Velocity;
                    piece.gravity = 9.0;
                    piece.velocity = Vec3::Y * (3.5 + 0.8 * jitter(self.id, i as u64, 66));
                    out.push(fly(piece, self.outward(i - 2, n - 2), 0.8 * self.reach));
                }
            }
            K::DrainWisp => {
                // Toward the source, and never far enough to reach it.
                let travel = (0.85 * self.reach).min(DRAIN_TRAVEL) - 0.18 * self.reach;
                for i in 0..n {
                    let mut wisp = self.lead(0.18, Curve::Hold);
                    wisp.lifetime = self.life * DEBRIS_LIFE;
                    wisp = held(wisp, 0.2 * self.life * share(i, n));
                    wisp.origin = self.centre
                        + self.side * (0.3 * self.reach * spread(i, n))
                        + Vec3::Y * (0.25 * ((i % 3) as f32 - 1.0));
                    wisp.orient = Orient::Velocity;
                    wisp.angle = heading(-self.along);
                    wisp.velocity = Vec3::Y * 0.3 - self.along * (travel.max(0.0) / wisp.lifetime);
                    out.push(wisp);
                }
            }
            K::ChainSnap => {
                for i in 0..n {
                    let mut link = held(self.lead(0.3, Curve::Pop), 0.4 * self.life * share(i, n));
                    link.origin = self.centre + self.along * (0.5 * self.reach * spread(i, n));
                    link.velocity = self.along * (0.1 * self.reach / self.life);
                    out.push(link);
                }
            }
            K::Splinter => {
                for i in 0..n {
                    // To both sides of the hit direction, some a little ahead or behind.
                    let way = if i % 2 == 0 { 1.0 } else { -1.0 };
                    let lean = 0.8 * jitter(self.id, i as u64, 68);
                    let throw = (self.side * way + self.along * lean).normalize_or_zero();
                    let mut chip = self.lead(0.2, Curve::Pop);
                    chip.lifetime = self.life * 1.2;
                    chip.orient = Orient::Velocity;
                    chip.angle = heading(throw);
                    chip.gravity = 7.0;
                    chip.velocity = Vec3::Y * 1.2;
                    let distance = 0.45 + 0.125 * (i % 3) as f32;
                    out.push(fly(chip, throw, distance * self.reach));
                }
            }
        }
        out
    }
}

/// Holds the particle back by `secs`, at most a quarter second and half its life, and ends
/// it when it would have ended.
fn held(mut spec: ParticleSpec, secs: f32) -> ParticleSpec {
    let secs = secs.min(0.25).min(0.5 * spec.lifetime);
    spec.delay += secs;
    spec.lifetime -= secs;
    spec
}

/// Shrinks a burst toward `centre` on the ground plane until nothing is drawn farther than
/// `bound` from it. Heights and vertical speeds are kept.
fn contain(specs: &mut [ParticleSpec], centre: Vec3, bound: f32) {
    let reach = specs
        .iter()
        .map(|spec| spec.reach(centre))
        .fold(0.0, f32::max);
    if reach <= bound {
        return;
    }
    let fit = bound / reach;
    let flat = Vec3::new(fit, 1.0, fit);
    for spec in specs {
        spec.origin = centre + (spec.origin - centre) * flat;
        spec.velocity *= flat;
        spec.size *= fit;
    }
}

/// The burst of one accepted receipt for the recipe of the skill that caused it.
pub(crate) fn impact_particles(
    recipe: &ImpactRecipe,
    palette: &Palette,
    ctx: &ImpactContext,
) -> Vec<ParticleSpec> {
    if !(ctx.position.is_finite() && ctx.ground.is_finite() && ctx.direction.is_finite()) {
        return Vec::new();
    }
    let kind = recipe.kind;
    let count = usize::from(recipe.count.unwrap_or(default_count(kind))).clamp(1, IMPACT_MAX - 1);
    let [lead, companion] = palette.pair(recipe.slots);
    // Whatever colours the row picks for its marks, the flash is the skill colour, at no
    // more than the gain of the pool: a broad glow above it turns pale on pale ground.
    let primary = palette.slot(PaletteSlot::Primary);
    let flash = Tint {
        gain: primary.gain.min(ParticleSpec::BASE.color.gain),
        ..primary
    };
    // A live effect knows where it is heading; a receipt only where it came from.
    let onward = ctx
        .heading
        .filter(|heading| kind == ImpactKind::PierceThrough && heading.is_finite())
        .and_then(Vec2::try_normalize);
    let direction = onward
        .or_else(|| ctx.direction.try_normalize())
        .unwrap_or(Vec2::X);
    let scale = recipe.scale.clamp(0.3, 2.0);
    let centre = ctx.position + Vec3::Y * BODY;
    let burst = Burst {
        centre,
        floor: ctx.position.with_y(ctx.ground),
        along: Vec3::new(direction.x, 0.0, direction.y),
        onward: onward.is_some(),
        side: Vec3::new(-direction.y, 0.0, direction.x),
        angle: direction.to_angle(),
        reach: scale
            * if kind == ImpactKind::Blast {
                BLAST_REACH
            } else {
                REACH
            },
        life: recipe.lifetime.clamp(0.08, 1.2),
        shape: recipe.lead(),
        lead,
        companion,
        flash,
        id: ctx.receipt,
    };
    let mut specs = burst.particles(kind, count);
    specs.push(burst.flash());
    if !ctx.area_damage {
        contain(&mut specs, centre, SINGLE_TARGET_REACH);
    }
    tagged(specs, ParticleSource::Impact)
}

/// One accepted receipt dealt by a hero the client sees.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Receipt {
    pub id: u64,
    pub position: Vec3,
    /// Height of the ground under the receipt.
    pub ground: f32,
    /// The hero that dealt it and where the client observes that hero.
    pub source: u64,
    pub source_position: Vec3,
}

/// Heading of the live effect of this skill and owner that is nearest to a receipt.
fn live_heading(effects: &[SkillEffectState], receipt: &Receipt, id: SkillId) -> Option<Vec2> {
    let at = receipt.position.xz();
    effects
        .iter()
        .filter(|effect| {
            receipt.source != 0
                && effect.owner_id == receipt.source
                && effect.skill == id
                && category::own_kinds(id).contains(&effect.kind)
        })
        .min_by(|a, b| {
            let reach = |effect: &SkillEffectState| Vec2::from_array(effect.position).distance(at);
            reach(a).total_cmp(&reach(b))
        })
        .and_then(|effect| {
            (Vec2::from_array(effect.end) - Vec2::from_array(effect.position)).try_normalize()
        })
}

/// The burst of one accepted receipt, from the `impact` recipe of the row that dealt it.
/// `None` when that row carries no recipe: the built-in burst of the wire style stays.
/// There is no other way to an impact burst than a receipt.
pub(crate) fn receipt_burst(
    registry: &SkillPresentation,
    key: CastKey,
    receipt: &Receipt,
    effects: &[SkillEffectState],
) -> Option<Vec<ParticleSpec>> {
    let look = registry.look(key)?;
    let skill = key.skill();
    Some(impact_particles(
        look.impact?,
        &look.palette,
        &ImpactContext {
            position: receipt.position,
            ground: receipt.ground,
            direction: (receipt.position - receipt.source_position).xz(),
            heading: skill
                .and_then(SkillKey::modular)
                .and_then(|id| live_heading(effects, receipt, id)),
            area_damage: skill.is_some_and(category::area_damage),
            receipt: receipt.id,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::super::category::{self, SkillKey};
    use super::super::tests::target::target;
    use super::*;
    use shared::HeroClass;

    const HIT: Vec3 = Vec3::new(-6.0, 0.5, 9.0);

    fn palette() -> Palette {
        Palette::of_class(&super::super::Theme {
            secondary: [0.2, 0.6, 0.2],
            accent: [0.9, 0.3, 0.1],
        })
    }

    fn receipt(area_damage: bool) -> ImpactContext {
        ImpactContext {
            position: HIT,
            ground: HIT.y,
            direction: Vec2::new(0.6, 0.8),
            heading: None,
            area_damage,
            receipt: 501,
        }
    }

    fn recipe(
        kind: ImpactKind,
        shape: Option<ParticleShape>,
        count: u8,
        scale: f32,
    ) -> ImpactRecipe {
        ImpactRecipe {
            kind,
            shape,
            count: Some(count),
            scale,
            lifetime: 0.45,
            slots: None,
        }
    }

    fn reach(specs: &[ParticleSpec]) -> f32 {
        specs.iter().map(|spec| spec.reach(HIT)).fold(0.0, f32::max)
    }

    #[test]
    fn impact_extent_is_bounded_without_area_damage() {
        for kind in ImpactKind::ALL {
            let per_scale = if *kind == ImpactKind::Blast {
                BLAST_REACH
            } else {
                REACH
            };
            for shape in ParticleShape::ALL {
                for count in [1, default_count(*kind), 12] {
                    for scale in [0.3, 1.0, 1.6, 2.0] {
                        for lifetime in [0.08, 0.45, 1.2] {
                            let row = ImpactRecipe {
                                lifetime,
                                ..recipe(*kind, Some(*shape), count, scale)
                            };
                            let name =
                                format!("{} {} x{count} {scale} {lifetime}", kind.id(), shape.id());
                            // With and without a body that flies on past the hit.
                            let heading = (count % 2 == 0).then_some(Vec2::NEG_X);
                            let seen = |area_damage: bool| ImpactContext {
                                heading,
                                ..receipt(area_damage)
                            };
                            let wide = impact_particles(&row, &palette(), &seen(true));
                            let held = impact_particles(&row, &palette(), &seen(false));
                            // A recipe reaches no farther than its scale says, and a skill
                            // without area damage never past the single-target bound.
                            assert!(reach(&wide) <= per_scale * scale + 1e-3, "{name}");
                            assert!(reach(&held) <= SINGLE_TARGET_REACH + 1e-3, "{name}");
                            if reach(&wide) <= SINGLE_TARGET_REACH {
                                // Inside the bound nothing is altered.
                                assert_eq!(held, wide, "{name}");
                            } else {
                                // Past it the burst is shrunk on the ground only, until it
                                // just fits: the same particles, as tall and as long-lived.
                                assert!(per_scale * scale > SINGLE_TARGET_REACH, "{name}");
                                assert!(
                                    (reach(&held) - SINGLE_TARGET_REACH).abs() < 1e-3,
                                    "{name}"
                                );
                                for (a, b) in held.iter().zip(&wide) {
                                    assert_eq!(
                                        (
                                            a.shape,
                                            a.lifetime,
                                            a.delay,
                                            a.origin.y,
                                            a.velocity.y,
                                            a.gravity
                                        ),
                                        (
                                            b.shape,
                                            b.lifetime,
                                            b.delay,
                                            b.origin.y,
                                            b.velocity.y,
                                            b.gravity
                                        ),
                                        "{name}"
                                    );
                                    assert!(a.size < b.size && a.reach(HIT) <= b.reach(HIT));
                                }
                            }
                        }
                    }
                }
            }
        }
        // Every single-target row of the roster stays at the unit it hit. A row of scale 1
        // or less is drawn as authored; a larger one is drawn at the bound, and no row is
        // drawn smaller than a row of scale 1 would be. Vertical travel is free:
        // Pyroblast's embers climb three units.
        let registry = target();
        let (mut single_target, mut at_the_bound) = (0, 0);
        for (id, profile) in registry.rows() {
            let key = SkillKey::from_id(id).unwrap();
            let Some(row) = &profile.impact else {
                continue;
            };
            let theme = registry.theme(key.home()).unwrap();
            let palette = Palette::of(profile, theme);
            let area = category::area_damage(key);
            let specs = impact_particles(row, &palette, &receipt(area));
            if !area {
                single_target += 1;
                assert!(reach(&specs) <= SINGLE_TARGET_REACH + 1e-3, "{id}");
                let wide = impact_particles(row, &palette, &receipt(true));
                if row.scale <= 1.0 {
                    assert_eq!(specs, wide, "{id}");
                } else {
                    at_the_bound += 1;
                    let unit = ImpactRecipe {
                        scale: 1.0,
                        ..row.clone()
                    };
                    let least = reach(&impact_particles(&unit, &palette, &receipt(false)));
                    assert!(reach(&specs) >= least - 1e-3, "{id}");
                }
            }
        }
        assert_eq!(single_target, 44);
        assert!(at_the_bound > 0);
        let pyroblast = registry.row("pyroblast").unwrap();
        let embers = impact_particles(
            pyroblast.impact.as_ref().unwrap(),
            &Palette::of(pyroblast, registry.theme(HeroClass::Mage).unwrap()),
            &receipt(false),
        );
        assert_eq!(embers.len(), 12);
        let climb = embers
            .iter()
            .map(|ember| {
                ember
                    .pose_at(ember.lifetime, false, Quat::IDENTITY)
                    .translation
                    .y
                    - HIT.y
            })
            .fold(0.0, f32::max);
        assert!(climb > 3.0, "the embers rise {climb}");
    }

    #[test]
    fn every_kind_draws_its_own_mark_at_the_receipt() {
        let centre = HIT + Vec3::Y * BODY;
        let (along, side) = (Vec3::new(0.6, 0.0, 0.8), Vec3::new(-0.8, 0.0, 0.6));
        let draw = |kind: ImpactKind, shape: Option<ParticleShape>, count: u8| {
            impact_particles(
                &recipe(kind, shape, count, 1.0),
                &palette(),
                &receipt(false),
            )
        };
        // The particles of the kind itself: all but the flash of the hit.
        let marks = |kind: ImpactKind, shape: Option<ParticleShape>, count: u8| {
            let mut specs = draw(kind, shape, count);
            specs.pop();
            specs
        };
        for kind in ImpactKind::ALL {
            // The count is the number of particles of the kind, eleven at most; without one
            // the kind decides. The flash of the hit closes every burst: a glow in the skill
            // colour at the receipt, whole at once, at rest and gone before the marks are.
            for count in 1..=12 {
                let specs = draw(*kind, None, count);
                let own = usize::from(count).min(IMPACT_MAX - 1);
                assert_eq!(specs.len(), own + 1, "{}", kind.id());
                let flash = &specs[own];
                assert_eq!(
                    (flash.shape, flash.curve, flash.origin, flash.velocity),
                    (ParticleShape::Glow, Curve::Shrink, centre, Vec3::ZERO),
                    "{}",
                    kind.id()
                );
                assert_eq!((flash.delay, flash.end_color), (0.0, None));
                assert_eq!(flash.color, palette().slot(PaletteSlot::Primary));
                assert!((flash.lifetime - 0.45 * FLASH_LIFE).abs() < 1e-6);
                // It is as large as its share of the reach; a blast without area damage is
                // held at the unit it hit, and its flash with it.
                let wide =
                    impact_particles(&recipe(*kind, None, count, 1.0), &palette(), &receipt(true));
                let per_scale = if *kind == ImpactKind::Blast {
                    BLAST_REACH
                } else {
                    REACH
                };
                assert!((wide[own].reach(centre) - FLASH * per_scale).abs() < 1e-5);
                assert!(flash.reach(centre) <= wide[own].reach(centre) + 1e-6);
                for spec in &specs {
                    assert_eq!((spec.event_id, spec.source), (501, ParticleSource::Impact));
                    assert!(spec.is_sound() && spec.delay <= 0.25, "{}", kind.id());
                    // The mark lives as long as the row says, debris a little longer.
                    assert!(
                        spec.end_secs() <= 0.45 * DEBRIS_LIFE + 1e-5,
                        "{}",
                        kind.id()
                    );
                }
                // Four kinds are debris from their first particle on.
                let debris_only = matches!(
                    kind,
                    ImpactKind::EmberPuff
                        | ImpactKind::ShardBurst
                        | ImpactKind::Splinter
                        | ImpactKind::DrainWisp
                );
                assert_eq!(
                    (specs[0].end_secs() - 0.45).abs() < 1e-5,
                    !debris_only,
                    "{}",
                    kind.id()
                );
            }
            let unnamed = ImpactRecipe {
                count: None,
                ..recipe(*kind, None, 1, 1.0)
            };
            assert_eq!(
                impact_particles(&unnamed, &palette(), &receipt(false)).len(),
                usize::from(default_count(*kind)) + 1,
                "{}",
                kind.id()
            );
            // A live effect's heading steers only a pierce (rule E-15).
            let steered = ImpactContext {
                heading: Some(Vec2::NEG_X),
                ..receipt(false)
            };
            let row = recipe(*kind, None, 6, 1.0);
            assert_eq!(
                impact_particles(&row, &palette(), &steered)
                    == impact_particles(&row, &palette(), &receipt(false)),
                *kind != ImpactKind::PierceThrough,
                "{}",
                kind.id()
            );
        }
        let lead = palette().slot(PaletteSlot::Primary);
        let companion = palette().slot(PaletteSlot::Accent);

        // Ring kinds keep their ring; a named lead shapes the debris, the default leaves glows.
        for kind in [ImpactKind::RingBurst, ImpactKind::ThudRing] {
            for (shape, debris) in [
                (None, ParticleShape::Glow),
                (Some(ParticleShape::Star), ParticleShape::Star),
                (Some(ParticleShape::Drop), ParticleShape::Drop),
            ] {
                let specs = marks(kind, shape, 7);
                assert_eq!(
                    (specs[0].shape, specs[0].curve),
                    (ParticleShape::Ringlet, Curve::Grow)
                );
                assert_eq!(specs[0].color, lead);
                assert!(specs[1..].iter().all(|spec| spec.shape == debris));
            }
        }
        // The thud ring lies on the ground under the unit and never grows past one unit.
        for scale in [0.3, 1.0, 2.0] {
            for area_damage in [false, true] {
                let ring = impact_particles(
                    &recipe(ImpactKind::ThudRing, None, 8, scale),
                    &palette(),
                    &receipt(area_damage),
                )[0]
                .clone();
                assert_eq!(ring.orient, Orient::Ground);
                assert!(ring.origin.y - HIT.y < 0.1);
                let radius = unit_radius(ring.shape) * ring.size * ring.curve.peak();
                assert!(radius <= THUD_RING + 1e-5);
                // A burst that is held at the unit it hit shrinks its ring with it.
                if area_damage || REACH * scale <= SINGLE_TARGET_REACH {
                    assert!((radius - (REACH * scale).min(THUD_RING)).abs() < 1e-5);
                }
            }
        }
        // A slash, a pop and the crossing and raking cuts are led by the row's shape.
        let cut = marks(ImpactKind::SlashCut, Some(ParticleShape::Crescent), 6);
        assert_eq!(
            (cut[0].shape, cut[0].color, cut[0].end_color),
            (ParticleShape::Crescent, lead, Some(companion))
        );
        assert!(
            cut[2..]
                .iter()
                .all(|spark| spark.shape == ParticleShape::Glow && spark.color == companion)
        );
        // A cut crosses the receipt whatever its shape: a straight lead lies on it, a curved
        // blade is set back by its depth and bulges away from the source.
        let hit_angle = Vec2::new(0.6, 0.8).to_angle();
        let crossing = |cut: &ParticleSpec| {
            cut.origin + ground(cut.angle) * (blade_depth(cut.shape) * cut.size)
        };
        assert!(crossing(&cut[0]).distance(centre) < 1e-5);
        assert!(ground(cut[0].angle).dot(along) > 0.0);
        let line = draw(ImpactKind::SlashCut, Some(ParticleShape::Streak), 1);
        assert_eq!(line[0].origin, centre);
        assert!((line[0].angle - hit_angle - 1.0).abs() < 1e-5);
        // A blade, a line and a claw cut across the unit; any other lead is a mark on it.
        let half = |shape: ParticleShape| {
            let cut = &draw(ImpactKind::SlashCut, Some(shape), 1)[0];
            unit_radius(cut.shape) * cut.size * cut.curve.peak()
        };
        assert!((half(ParticleShape::Streak) - 0.95 * REACH).abs() < 1e-5);
        assert!((half(ParticleShape::Claw) - 0.95 * REACH).abs() < 1e-5);
        for shape in [
            ParticleShape::Diamond,
            ParticleShape::Star,
            ParticleShape::Kite,
        ] {
            assert!((half(shape) - MARK * REACH).abs() < 1e-5, "{}", shape.id());
        }
        for shape in [ParticleShape::Slash, ParticleShape::Streak] {
            // Two cuts through one point at a right angle: an X.
            let cross = draw(ImpactKind::CrossCut, Some(shape), 6);
            let axes: Vec<Vec3> = cross[..2]
                .iter()
                .map(|cut| {
                    assert_eq!(cut.shape, shape);
                    assert!(crossing(cut).distance(centre) < 1e-5, "{}", shape.id());
                    assert!(ground(cut.angle).dot(along) > 0.0, "{}", shape.id());
                    ground(cut.angle)
                })
                .collect();
            assert!(axes[0].dot(axes[1]).abs() < 1e-5, "{}", shape.id());
            // Three parallel marks side by side across their own direction.
            let rake = draw(ImpactKind::ClawRake, Some(shape), 5);
            let across = ground(hit_angle + 0.9 + FRAC_PI_2);
            let lanes: Vec<f32> = rake[..3]
                .iter()
                .map(|mark| {
                    assert_eq!((mark.shape, mark.angle), (shape, rake[0].angle));
                    let offset = crossing(mark) - centre;
                    assert!(offset.cross(across).length() < 1e-5, "{}", shape.id());
                    offset.dot(across)
                })
                .collect();
            assert!((lanes[0] + 0.38 * REACH).abs() < 1e-5 && lanes[1].abs() < 1e-5);
            assert!((lanes[2] - 0.38 * REACH).abs() < 1e-5, "{}", shape.id());
        }
        let pop = draw(ImpactKind::GlowPop, Some(ParticleShape::Diamond), 4);
        assert_eq!(
            (pop[0].shape, pop[0].curve),
            (ParticleShape::Diamond, Curve::Pop)
        );
        assert_eq!(draw(ImpactKind::GlowPop, None, 4)[0].curve, Curve::Shrink);

        // A pierce moves on past the target along the heading of the live effect, and
        // nothing goes back or sideways.
        let pierce = recipe(ImpactKind::PierceThrough, None, 6, 1.0);
        let flying = ImpactContext {
            heading: Some(Vec2::new(0.0, -2.0)),
            ..receipt(false)
        };
        let onward = impact_particles(&pierce, &palette(), &flying);
        for spec in &onward[..onward.len() - 1] {
            assert!(spec.velocity.dot(Vec3::NEG_Z) > 0.0);
            assert!(spec.velocity.cross(Vec3::NEG_Z).length() < 1e-4);
            assert!((spec.origin - centre).dot(Vec3::NEG_Z) > 0.0);
        }
        // When no body of the skill is in the snapshot nothing is seen to fly on (the last
        // hit of a bolt ends it): the same mark stays on the hit axis, at rest.
        let stopped = impact_particles(&pierce, &palette(), &receipt(false));
        assert_eq!(stopped.len(), onward.len());
        for spec in &stopped {
            assert_eq!(spec.velocity, Vec3::ZERO);
            assert!((spec.origin - centre).cross(along).length() < 1e-5);
        }
        assert_eq!(
            (stopped[0].origin, stopped[0].curve),
            (centre, Curve::Stretch)
        );
        assert_eq!(onward[0].curve, Curve::Stretch);
        let broken = ImpactContext {
            heading: Some(Vec2::NAN),
            ..receipt(false)
        };
        assert_eq!(impact_particles(&pierce, &palette(), &broken), stopped);
        // Forks fly out ahead of the hit; shards, splinters and blast debris fall.
        let forks = marks(ImpactKind::SparkFork, None, 3);
        assert!(
            forks
                .iter()
                .all(|fork| fork.velocity.dot(along) > 0.0 && fork.orient == Orient::Velocity)
        );
        assert!(forks[0].velocity.dot(side) * forks[2].velocity.dot(side) < 0.0);
        for kind in [ImpactKind::ShardBurst, ImpactKind::Splinter] {
            assert!(
                marks(kind, None, 6)
                    .iter()
                    .all(|piece| piece.gravity > 0.0 && piece.velocity.y > 0.0)
            );
        }
        let chips = marks(ImpactKind::Splinter, None, 6);
        assert!(
            chips
                .iter()
                .all(|chip| chip.velocity.dot(side).abs() > chip.velocity.dot(along).abs())
        );
        assert!(chips[0].velocity.dot(side) * chips[1].velocity.dot(side) < 0.0);
        // A flash and a facet are marks at rest: no debris of any kind.
        for kind in [ImpactKind::FlashStar, ImpactKind::FacetPop] {
            assert!(
                draw(kind, None, 5)
                    .iter()
                    .all(|mark| mark.velocity == Vec3::ZERO && mark.origin == centre)
            );
        }
        let star = marks(ImpactKind::FlashStar, None, 3);
        assert!(star.iter().all(|flash| flash.shape == ParticleShape::Star));
        assert!(star[1].size < star[0].size && star[2].size < star[1].size);
        // The facet lies along the hit direction, turned a little by the receipt id.
        let facet = draw(ImpactKind::FacetPop, Some(ParticleShape::Streak), 2);
        assert_eq!(
            (facet[0].shape, facet[1].shape),
            (ParticleShape::Streak, ParticleShape::Ringlet)
        );
        assert_eq!((facet[0].color, facet[1].color), (lead, companion));
        let lie = facet[0].angle - Vec2::new(0.6, 0.8).to_angle();
        assert!(lie.abs() <= 0.35 && lie != 0.0);
        let other = ImpactContext {
            receipt: 502,
            ..receipt(false)
        };
        let again = impact_particles(
            &recipe(ImpactKind::FacetPop, None, 2, 1.0),
            &palette(),
            &other,
        );
        assert!(again[0].angle != facet[0].angle);
        assert_eq!(
            draw(ImpactKind::FacetPop, Some(ParticleShape::Streak), 2),
            facet
        );
        // Embers rise and darken; a frost star stands while its chips fall.
        let embers = marks(ImpactKind::EmberPuff, None, 6);
        assert!(
            embers
                .iter()
                .all(|ember| ember.gravity < 0.0 && ember.velocity.y > 1.5)
        );
        assert!(
            embers
                .iter()
                .all(|ember| ember.end_color == Some(companion))
        );
        let frost = marks(ImpactKind::StarShards, Some(ParticleShape::Star), 5);
        assert_eq!(
            (frost[0].shape, frost[0].velocity),
            (ParticleShape::Star, Vec3::ZERO)
        );
        assert!(
            frost[1..]
                .iter()
                .all(|chip| chip.shape == ParticleShape::Diamond && chip.gravity > 0.0)
        );
        // Chain links snap along the hit axis; drained motes drift back toward the source
        // and stop within 1.2 units however large the row is.
        let links = marks(ImpactKind::ChainSnap, None, 4);
        assert!(
            links
                .iter()
                .all(|link| (link.origin - centre).dot(side).abs() < 1e-5
                    && link.shape == ParticleShape::Chevron
                    && link.velocity.dot(along) > 0.0)
        );
        let drain = impact_particles(
            &recipe(ImpactKind::DrainWisp, None, 5, 2.0),
            &palette(),
            &receipt(true),
        );
        for wisp in &drain[..drain.len() - 1] {
            assert!(wisp.velocity.dot(along) < 0.0);
            let travel = (wisp.velocity * wisp.lifetime).dot(-along);
            assert!(travel > 0.3 && travel <= DRAIN_TRAVEL);
        }
        // A blast is a glow, a ground ring and falling debris, and the only kind that reaches
        // past the single-target scale.
        let blast = impact_particles(
            &recipe(ImpactKind::Blast, Some(ParticleShape::Star), 12, 1.0),
            &palette(),
            &receipt(true),
        );
        assert_eq!(
            (blast[0].shape, blast[1].shape, blast[1].orient),
            (ParticleShape::Glow, ParticleShape::Ringlet, Orient::Ground)
        );
        // Its ring holds the radius it reaches: 0.95 of the reach, from the first quarter of
        // its life on.
        assert_eq!(blast[1].curve, Curve::Pop);
        let radius = unit_radius(blast[1].shape) * blast[1].size * blast[1].curve.peak();
        assert!((radius - 0.95 * BLAST_REACH).abs() < 1e-5);
        assert!(
            blast[2..blast.len() - 1]
                .iter()
                .all(|piece| piece.shape == ParticleShape::Star && piece.gravity > 0.0)
        );
        assert!(reach(&blast) > REACH && reach(&blast) <= BLAST_REACH + 1e-4);

        // A receipt whose source is not known still draws, along +X; a broken one draws nothing.
        let unknown = ImpactContext {
            direction: Vec2::ZERO,
            ..receipt(false)
        };
        let chain = impact_particles(
            &recipe(ImpactKind::ChainSnap, None, 4, 1.0),
            &palette(),
            &unknown,
        );
        assert!(
            chain
                .iter()
                .all(|link| link.is_sound() && link.velocity.z == 0.0)
        );
        let broken = ImpactContext {
            position: Vec3::NAN,
            ..receipt(false)
        };
        assert!(
            impact_particles(
                &recipe(ImpactKind::Blast, None, 4, 1.0),
                &palette(),
                &broken
            )
            .is_empty()
        );
        // Slots are lead and companion; the flash keeps the skill colour.
        let row = ImpactRecipe {
            slots: Some([PaletteSlot::White, PaletteSlot::Secondary]),
            ..recipe(ImpactKind::SlashCut, None, 4, 1.0)
        };
        let specs = impact_particles(&row, &palette(), &receipt(false));
        assert_eq!(specs[0].color, palette().slot(PaletteSlot::White));
        assert_eq!(specs[3].color, palette().slot(PaletteSlot::Secondary));
        assert_eq!(specs[4].color, palette().slot(PaletteSlot::Primary));
        // A row with more gain than the pool draws its marks with it, and its flash with
        // the gain of the pool.
        let registry = target();
        let pyroblast = registry.row("pyroblast").unwrap();
        let hot = Palette::of(pyroblast, registry.theme(HeroClass::Mage).unwrap());
        let burst = impact_particles(pyroblast.impact.as_ref().unwrap(), &hot, &receipt(false));
        let flash = burst.last().unwrap();
        assert!(hot.slot(PaletteSlot::Primary).gain > ParticleSpec::BASE.color.gain);
        assert_eq!(flash.color.color, hot.slot(PaletteSlot::Primary).color);
        assert_eq!(flash.color.gain, ParticleSpec::BASE.color.gain);
        assert_eq!(burst[0].color.gain, hot.slot(PaletteSlot::Primary).gain);
    }

    #[test]
    fn the_burst_sits_above_the_receipt_and_its_ground_rings_on_the_ground() {
        // A receipt is at aim height; the ground is below it.
        let lifted = ImpactContext {
            ground: HIT.y - 1.05,
            ..receipt(true)
        };
        for kind in ImpactKind::ALL {
            let row = recipe(*kind, None, 8, 1.0);
            let flat = impact_particles(&row, &palette(), &receipt(true));
            let raised = impact_particles(&row, &palette(), &lifted);
            assert_eq!(flat.len(), raised.len());
            let mut lowered = 0;
            for (a, b) in flat.iter().zip(&raised) {
                // Only the height of a ground piece differs, by the height of the receipt.
                assert_eq!(a.origin.xz(), b.origin.xz(), "{}", kind.id());
                assert_eq!((a.velocity, a.size), (b.velocity, b.size));
                if a.origin.y != b.origin.y {
                    assert!((a.origin.y - b.origin.y - 1.05).abs() < 1e-5);
                    assert!(b.origin.y < lifted.ground + 0.2, "{}", kind.id());
                    lowered += 1;
                } else {
                    assert!(b.origin.y >= HIT.y, "{}", kind.id());
                }
            }
            let grounded = matches!(kind, ImpactKind::ThudRing | ImpactKind::Blast);
            assert_eq!(lowered > 0, grounded, "{}", kind.id());
        }
        let broken = ImpactContext {
            ground: f32::NAN,
            ..receipt(false)
        };
        assert!(
            impact_particles(
                &recipe(ImpactKind::ThudRing, None, 4, 1.0),
                &palette(),
                &broken
            )
            .is_empty()
        );
    }

    #[test]
    fn a_receipt_is_drawn_from_the_row_that_dealt_it() {
        use shared::loadout::EffectVisualKind;
        let registry = target();
        let dawn = SkillId::DawnRay;
        let key = CastKey::Skill(SkillKey::Modular(dawn));
        let hit = Receipt {
            id: 77,
            position: HIT,
            ground: HIT.y - 1.05,
            source: 7,
            source_position: HIT - Vec3::X * 6.0,
        };
        let beam = |owner: u64, skill: SkillId, kind: EffectVisualKind| SkillEffectState {
            id: 5,
            owner_id: owner,
            owner_team: shared::map::Team::Green,
            skill,
            kind,
            position: [HIT.x - 6.0, HIT.z],
            end: [HIT.x - 6.0, HIT.z - 12.0],
            radius: 0.6,
            remaining_secs: 0.1,
            armed: true,
            consumed_segments: 0,
        };
        let look = registry.look(key).unwrap();
        let drawn = |heading: Option<Vec2>| {
            impact_particles(
                look.impact.unwrap(),
                &look.palette,
                &ImpactContext {
                    position: hit.position,
                    ground: hit.ground,
                    direction: Vec2::X,
                    heading,
                    area_damage: false,
                    receipt: 77,
                },
            )
        };
        // The recipe, the colours and the receipt id come from the row; the hit points away
        // from the hero that dealt it.
        let at_rest = receipt_burst(&registry, key, &hit, &[]).unwrap();
        assert_eq!(at_rest, drawn(None));
        assert!(at_rest.iter().all(|spec| spec.event_id == 77));
        // Dawn Ray pierces: the mark flies on along the live beam of this hero (rule E-15),
        // and along nothing else.
        let live = [beam(7, dawn, EffectVisualKind::Beam)];
        let onward = receipt_burst(&registry, key, &hit, &live).unwrap();
        assert_eq!(onward, drawn(Some(Vec2::NEG_Y)));
        assert_ne!(onward, at_rest);
        for other in [
            beam(8, dawn, EffectVisualKind::Beam),
            beam(0, dawn, EffectVisualKind::Beam),
            beam(7, SkillId::HorizonWave, EffectVisualKind::Beam),
            beam(7, dawn, EffectVisualKind::Orb),
        ] {
            assert_eq!(
                receipt_burst(&registry, key, &hit, &[other]),
                Some(at_rest.clone())
            );
        }
        let unknown = Receipt { source: 0, ..hit };
        assert_eq!(
            receipt_burst(
                &registry,
                key,
                &unknown,
                &[beam(0, dawn, EffectVisualKind::Beam)]
            ),
            Some(at_rest)
        );
        // Area damage is a fact of the skill, not of the row: Thunder Pulse may draw wide.
        let pulse = CastKey::Skill(SkillKey::Modular(SkillId::ThunderPulse));
        assert!(category::area_damage(SkillKey::Modular(
            SkillId::ThunderPulse
        )));
        assert!(receipt_burst(&registry, pulse, &hit, &[]).is_some());
        // A row without a recipe, and a basic attack without a row, keep the built-in burst.
        let step = CastKey::Skill(SkillKey::Modular(SkillId::AnchorStep));
        assert_eq!(receipt_burst(&registry, step, &hit, &[]), None);
        let packaged = SkillPresentation::packaged();
        assert_eq!(receipt_burst(&packaged, key, &hit, &[]), None);
        assert_eq!(
            receipt_burst(&packaged, CastKey::Basic(HeroClass::Mage), &hit, &[]),
            None
        );
        let basic = receipt_burst(&registry, CastKey::Basic(HeroClass::Mage), &hit, &[]).unwrap();
        assert!(
            basic
                .iter()
                .all(|spec| spec.source == ParticleSource::Impact)
        );
    }
}
