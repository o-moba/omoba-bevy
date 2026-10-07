//! The closed vocabulary of skill presentation. Every ID a data row may name is a variant
//! here, so a new look is a code change with a test and never a data edit.
//! `docs/skill-vocabulary.md` is rendered from these lists.
// The pick list is complete; renderers and generators adopt the IDs package by package.
#![cfg_attr(not(test), allow(dead_code))]

use serde::Deserialize;

macro_rules! vocabulary {
    ($(#[$meta:meta])* $name:ident {
        $($(#[$variant_meta:meta])* $variant:ident = $id:literal: $doc:literal,)+
    }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
        pub(crate) enum $name {
            $($(#[$variant_meta])* #[doc = $doc] #[serde(rename = $id)] $variant,)+
        }
        impl $name {
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant,)+];
            pub(crate) const fn id(self) -> &'static str {
                match self { $(Self::$variant => $id,)+ }
            }
            /// What the player sees, as printed in the pick list.
            pub(crate) const fn doc(self) -> &'static str {
                match self { $(Self::$variant => $doc,)+ }
            }
        }
    };
}

vocabulary! {
    /// When the release clip of a skill starts.
    MotionPhase {
        Instant = "instant": "the release clip starts on the accepted cast",
        WarnFire = "warn_fire": "the windup is held while the hero's own warning exists; the release follows its change of kind",
        Fuse = "fuse": "the windup starts on the accepted cast and is held while the hero's own telegraph exists, 1.2 s at most when that is never seen; the release follows its inferred firing",
        Parry = "parry": "the windup is held while the hero parries; the release follows the riposte",
    }
}

vocabulary! {
    /// The eight shapes a replicated world effect can take.
    Archetype {
        Traveller = "traveller": "an object moving along a heading",
        Orbiter = "orbiter": "a round object that follows, hovers or returns",
        Zone = "zone": "a circle on the ground",
        Lane = "lane": "a strip from the effect position to its end",
        Sector = "sector": "a cone opening from the effect position toward its end",
        Prop = "prop": "a placed object with a trigger or collision radius",
        Wall = "wall": "a wall across the facing",
        Cage = "cage": "a pentagon with breakable sides",
    }
}

vocabulary! {
    /// Shared low-poly meshes a body part or a legacy projectile form may use.
    Silhouette {
        Ball = "ball": "low-poly sphere",
        Cone = "cone": "five-sided cone",
        Block = "block": "cube; also the bar primitive",
        Ring = "ring": "thin flat annulus at unit radius",
        Torus = "torus": "thin 3D ring: gyro ring, chain link, shackle",
        Shard = "shard": "elongated octahedron: crystal, needle, ice",
        Kite = "kite": "flat shield outline",
        Star = "star": "flat four-point star",
        Chevron = "chevron": "flat open V",
        Diamond = "diamond": "flat rhombus",
        Arc = "arc": "flat half annulus",
        Drop = "drop": "flat teardrop: flame, leaf, feather, thorn when stretched",
        Cross = "cross": "flat plus sign",
        Crescent = "crescent": "flat thick blade moon",
        Claw = "claw": "flat three-tine rake",
    }
}

vocabulary! {
    /// Packaged props a body may carry.
    Model {
        Rocket = "rocket": "the Wildspark rocket",
        Trap = "trap": "the Wildspark trap",
        Hook = "hook": "the Chainkeeper hook",
        Lantern = "lantern": "the Chainkeeper lantern",
        Orb = "orb": "the Orbitwright sphere",
    }
}

vocabulary! {
    /// How the copies of a satellite part are arranged.
    SatelliteLayout {
        Orbit = "orbit": "copies on a horizontal ring around the centre, turning",
        Halo = "halo": "copies on a vertical ring around the heading axis",
        Helix = "helix": "copies corkscrewing behind a moving body",
        Column = "column": "copies stacked upward",
        QuadX = "quad_x": "copies in an X around the centre",
        Rim = "rim": "copies evenly on the boundary circle, static",
        Line = "line": "copies evenly along the strip",
        Stagger = "stagger": "copies alternating left and right along the strip",
        Fan = "fan": "copies spread across the cone",
    }
}

vocabulary! {
    /// What a moving body leaves on the positions the client has observed.
    #[derive(Default)]
    Trail {
        #[default]
        None = "none": "no trail",
        Ribbon = "ribbon": "two stretched bars between the last observed positions",
        Motes = "motes": "three small parts on the last observed positions",
        Chevrons = "chevrons": "three flat chevrons on the last observed positions",
        Links = "links": "up to six chain links on the instance's own observed positions",
    }
}

vocabulary! {
    /// Read-out of the replicated stage of a body.
    #[derive(Default)]
    Marker {
        #[default]
        None = "none": "no read-out",
        RemainingRing = "remaining_ring": "a ring that shrinks with the remaining time",
        ArmingPips = "arming_pips": "pips that light when the effect arms",
        FillToEdge = "fill_to_edge": "an inner shape that grows to the boundary with telegraph progress",
        OwnerTether = "owner_tether": "one thin bar from the body to its visible owner",
    }
}

vocabulary! {
    /// How one part of a body moves or shows.
    #[derive(Default)]
    Behaviour {
        #[default]
        Steady = "steady": "does not move",
        Spin = "spin": "turns about the vertical axis",
        Pulse = "pulse": "breathes",
        Bob = "bob": "floats up and down",
        Flicker = "flicker": "flickers",
        Tumble = "tumble": "turns end over end",
        HideInTelegraph = "hide_in_telegraph": "hidden while the effect is a telegraph",
        OnlyInTelegraph = "only_in_telegraph": "shown only while the effect is a telegraph",
        RiseOnArm = "rise_on_arm": "flat until the effect arms, then full height",
        BlinkLast = "blink_last": "blinks in the last half second of the remaining time",
        Gyro = "gyro": "orb only: rings spin level at rest and tilt along the travel",
        OnlyAfterRenew = "only_after_renew": "hidden until this instance was seen to renew",
        RiseOnSpawn = "rise_on_spawn": "grows over the first 0.2 s of the instance's life",
    }
}

vocabulary! {
    /// Height band of a body.
    Altitude {
        Ground = "ground": "on the ground plane",
        Chest = "chest": "at chest height",
        High = "high": "above head height",
    }
}

vocabulary! {
    /// One-shot shown when an on-screen effect ends in a way the client can classify.
    #[derive(Default)]
    ExpireKind {
        #[default]
        None = "none": "silent removal",
        Fade = "fade": "soft desaturated dissipation that does not read as a hit",
        Crumble = "crumble": "the structure breaks into falling pieces",
        Discharge = "discharge": "the telegraphed shape flashes as it releases",
        Detonate = "detonate": "the zone bursts outward to its boundary",
    }
}

vocabulary! {
    /// Pooled-particle accent at the caster on an accepted cast.
    AccentPattern {
        ArcSweep = "arc_sweep": "one blade arc sweeping across the front",
        DoubleArc = "double_arc": "two crossing arcs, 80 ms apart",
        RakeTriple = "rake_triple": "three short parallel rakes forward",
        ThrustLine = "thrust_line": "a forward streak from hand height along the aim",
        MuzzleFlash = "muzzle_flash": "one shape and a glow at hand height, pointing forward",
        MuzzleBurst = "muzzle_burst": "three staggered shapes forward",
        FanSpray = "fan_spray": "shapes thrown in a forward fan",
        GroundRing = "ground_ring": "a ring expanding on the ground around the caster",
        GroundSlam = "ground_slam": "a ground ring and shapes thrown upward at the caster",
        RisingMotes = "rising_motes": "shapes rising around the caster",
        InwardGather = "inward_gather": "shapes converging on the caster",
        SpiralUp = "spiral_up": "a helix of shapes around the caster",
        ShieldFlash = "shield_flash": "one flat plate in front of the caster, facing the aim",
        RuneMark = "rune_mark": "flat shapes on a small ring above the caster's head, turning",
        TossArc = "toss_arc": "shapes on a short upward arc ahead",
        StrikeLine = "strike_line": "shapes laid on the ground from the caster to the skill's own new effect",
        None = "none": "nothing; the telegraph body is the cast read",
    }
}

impl AccentPattern {
    /// The lead shape drawn when a row names none, and always on a recast accent.
    pub(crate) const fn default_lead(self) -> Option<ParticleShape> {
        Some(match self {
            Self::ArcSweep => ParticleShape::Crescent,
            Self::DoubleArc => ParticleShape::Slash,
            Self::RakeTriple => ParticleShape::Claw,
            Self::ThrustLine => ParticleShape::Streak,
            Self::MuzzleFlash | Self::MuzzleBurst => ParticleShape::Chevron,
            Self::FanSpray => ParticleShape::Drop,
            Self::GroundRing => ParticleShape::Ringlet,
            Self::GroundSlam | Self::RuneMark | Self::StrikeLine => ParticleShape::Diamond,
            Self::RisingMotes | Self::InwardGather | Self::TossArc => ParticleShape::Glow,
            Self::SpiralUp => ParticleShape::Star,
            Self::ShieldFlash => ParticleShape::Kite,
            Self::None => return None,
        })
    }

    /// Reach of the pattern in units at `scale` 1. `strike_line` ends at a replicated
    /// position and has no authored reach.
    pub(crate) const fn base_extent(self) -> f32 {
        match self {
            Self::ArcSweep | Self::DoubleArc | Self::FanSpray => 1.6,
            Self::RakeTriple | Self::MuzzleBurst | Self::GroundSlam => 1.4,
            Self::ThrustLine => 2.0,
            Self::MuzzleFlash | Self::RisingMotes | Self::SpiralUp | Self::ShieldFlash => 1.0,
            Self::GroundRing => 1.2,
            Self::InwardGather | Self::TossArc => 1.5,
            Self::RuneMark => 0.8,
            Self::StrikeLine | Self::None => 0.0,
        }
    }

    /// The only patterns a row with a windup may use: they do not imply an immediate release.
    pub(crate) const fn is_charge(self) -> bool {
        matches!(
            self,
            Self::InwardGather | Self::RuneMark | Self::SpiralUp | Self::RisingMotes | Self::None
        )
    }
}

vocabulary! {
    /// Particles drawn between two observed positions of a hero that a cast moved.
    MovePattern {
        Afterimage = "afterimage": "streak afterimages along the travelled line",
        BlinkPair = "blink_pair": "a collapse at the origin and a burst at the arrival, nothing between",
        LeapArc = "leap_arc": "a dust ring at the origin, a landing ring and radial dust at the arrival",
        ChargeDust = "charge_dust": "ground dust along the path and a thud ring at the arrival",
        WhirlStep = "whirl_step": "shapes spiralling along the path",
        VeilStep = "veil_step": "a dark dissolve at the origin and a re-form at the arrival",
    }
}

vocabulary! {
    /// Ground gizmo shown while the replicated slot can be recast.
    RecastMarker {
        RingPips = "ring_pips": "pips on a small ring at the feet",
        OrbitMotes = "orbit_motes": "two marks circling the feet",
        GroundArrows = "ground_arrows": "arrows pointing along the facing",
    }
}

vocabulary! {
    /// Pooled-particle burst at an accepted damage receipt.
    ImpactKind {
        RingBurst = "ring_burst": "an expanding ringlet with radiating glows",
        SlashCut = "slash_cut": "one slash with sparks",
        GlowPop = "glow_pop": "a glow pop with sparks",
        CrossCut = "cross_cut": "two crossing slashes",
        ClawRake = "claw_rake": "three parallel claw marks",
        PierceThrough = "pierce_through": "a streak continuing along the hit direction past the target",
        SparkFork = "spark_fork": "three forked streaks",
        ShardBurst = "shard_burst": "shards thrown outward that fall with gravity",
        FlashStar = "flash_star": "one bright star, no debris",
        EmberPuff = "ember_puff": "rising drops that darken",
        StarShards = "star_shards": "a static star and a few falling diamonds",
        ThudRing = "thud_ring": "a flat ground ringlet and low dust",
        FacetPop = "facet_pop": "a flat shape and a ringlet that pop and vanish, no debris",
        Blast = "blast": "a large glow, a ringlet and debris",
        DrainWisp = "drain_wisp": "motes drifting from the hit toward the source",
        ChainSnap = "chain_snap": "chevrons snapping along the hit direction",
        Splinter = "splinter": "short drops thrown sideways",
    }
}

impl ImpactKind {
    /// The lead shape drawn when a row names none.
    pub(crate) const fn default_lead(self) -> ParticleShape {
        match self {
            Self::RingBurst | Self::ThudRing => ParticleShape::Ringlet,
            Self::SlashCut | Self::CrossCut => ParticleShape::Slash,
            Self::GlowPop | Self::Blast | Self::DrainWisp => ParticleShape::Glow,
            Self::ClawRake => ParticleShape::Claw,
            Self::PierceThrough | Self::SparkFork => ParticleShape::Streak,
            Self::ShardBurst | Self::FacetPop => ParticleShape::Diamond,
            Self::FlashStar | Self::StarShards => ParticleShape::Star,
            Self::EmberPuff | Self::Splinter => ParticleShape::Drop,
            Self::ChainSnap => ParticleShape::Chevron,
        }
    }
}

vocabulary! {
    /// Planar shapes of pooled particles; all are valid in both render backends.
    ParticleShape {
        Glow = "glow": "soft round glow",
        Ringlet = "ringlet": "thin ring",
        Slash = "slash": "curved cut",
        Streak = "streak": "thin line",
        Star = "star": "four-point star",
        Chevron = "chevron": "open V",
        Diamond = "diamond": "rhombus",
        Arc = "arc": "half ring",
        Drop = "drop": "teardrop",
        Cross = "cross": "plus sign",
        Crescent = "crescent": "blade moon",
        Claw = "claw": "three-tine rake",
        Kite = "kite": "shield outline",
    }
}

vocabulary! {
    /// Which colour of a row a part or particle takes.
    #[derive(Default)]
    PaletteSlot {
        #[default]
        Primary = "primary": "the skill colour, drawn with the skill's HDR gain",
        Secondary = "secondary": "the class matter colour, drawn without HDR gain",
        Accent = "accent": "the class spark colour, drawn with the skill's HDR gain",
        White = "white": "neutral white",
    }
}

vocabulary! {
    /// Existing samples a skill voice may be built from.
    AudioBase {
        Melee = "melee": "the melee strike sample",
        Arrow = "arrow": "the arrow sample",
        Arcane = "arcane": "the arcane sample",
        Holy = "holy": "the holy sample",
        Caster = "caster": "the caster bolt sample",
        Tower = "tower": "the tower shot sample",
        Bluff = "bluff": "the bluff sample; `dagger_bluff` only",
    }
}

vocabulary! {
    /// Which part of a sample is played.
    #[derive(Default)]
    AudioSlice {
        #[default]
        Full = "full": "the whole sample",
        Tick = "tick": "the first 0.12 s",
        Body = "body": "0.06 s to 0.40 s",
        Tail = "tail": "from 0.20 s on",
    }
}

vocabulary! {
    /// 3D body of a legacy projectile (`combat_visuals.json`).
    ProjectileForm {
        Dart = "dart": "one silhouette stretched along the heading",
        Comet = "comet": "a pulsing head and three shrinking afterimages of itself",
        DiscSkim = "disc_skim": "a flat silhouette spinning about the vertical axis, skimming",
        Tumbler = "tumbler": "a silhouette tumbling end over end",
        Wavefront = "wavefront": "a wide silhouette perpendicular to the heading, hugging the ground",
        Volley = "volley": "three small silhouettes in a tight fan",
        TwinHelix = "twin_helix": "two small silhouettes winding around the heading axis",
    }
}

vocabulary! {
    /// How a legacy projectile is shown in flight (`combat_visuals.json`).
    #[derive(Default)]
    ProjectilePresentation {
        #[default]
        Projectile = "projectile": "a thrown body at chest height with a trail",
        Wave = "wave": "a ground-hugging body, no weapon model, no trail, no puffs",
        MeleeContact = "melee_contact": "no thrown body; a short reach streak follows the server projectile",
    }
}

vocabulary! {
    /// Aim previews; derived from the skill, never authored.
    PreviewShape {
        None = "none": "nothing",
        Lane = "lane": "rails of the real half-width from the caster to range",
        LaneCapsule = "lane_capsule": "the lane with round ends",
        LaneToPoint = "lane_to_point": "a narrow lane ending at the bounded aim point",
        PointRing = "point_ring": "a ring of the real radius at the bounded aim point or at the orb",
        TrapRow = "trap_row": "the three trap rings",
        Sector = "sector": "the server's half-angle and radius from the caster",
        SelfRing = "self_ring": "a ring of the real radius around the caster",
        SelfPentagon = "self_pentagon": "the cage outline around the caster",
        RangeRing = "range_ring": "the cast-range ring",
        DashLanding = "dash_landing": "a line to the landing point and the strike ring there",
        BlinkLanding = "blink_landing": "a landing marker and the strike ring, no line",
        UnitPick = "unit_pick": "a pick ring at the aim point and a highlight on the unit the server would pick",
        PickThenLane = "pick_then_lane": "the unit pick plus the push lane beyond the picked unit",
        EffectOriginLane = "effect_origin_lane": "a lane starting at the owner's own replicated object",
        WallAhead = "wall_ahead": "the wall bar one unit ahead",
    }
}

vocabulary! {
    /// How the stage of a replicated effect is read; derived from the skill, never authored.
    StageRule {
        Active = "active": "live for its whole replicated life",
        ArmedGate = "armed_gate": "a telegraph until the replicated `armed` flag is set",
        KindGate = "kind_gate": "a telegraph while the replicated kind is a warning",
        Fuse = "fuse": "a telegraph for its whole observed life; the release is inferred",
    }
}

/// One table of the pick list.
#[cfg(test)]
struct Section {
    title: &'static str,
    note: &'static str,
    columns: &'static [&'static str],
    rows: Vec<Vec<String>>,
}

#[cfg(test)]
fn section<T: Copy>(
    title: &'static str,
    note: &'static str,
    all: &[T],
    id: fn(T) -> &'static str,
    doc: fn(T) -> &'static str,
) -> Section {
    Section {
        title,
        note,
        columns: &["ID", "What the player sees"],
        rows: all
            .iter()
            .map(|item| vec![format!("`{}`", id(*item)), doc(*item).to_string()])
            .collect(),
    }
}

/// The pick list for designers. `docs/skill-vocabulary.md` must equal this text.
#[cfg(test)]
pub(crate) fn render_markdown() -> String {
    let lead = |shape: Option<ParticleShape>| {
        shape.map_or_else(|| "-".to_string(), |shape| format!("`{}`", shape.id()))
    };
    let sections = [
        section(
            "Motion phases",
            "`motion.phase`. An explicit value must equal the phase derived for the skill, and a phase other than `instant` needs a `windup`.",
            MotionPhase::ALL,
            MotionPhase::id,
            MotionPhase::doc,
        ),
        section(
            "Body archetypes",
            "`body.archetype`. The boundary parts of an archetype are drawn by the engine from replicated geometry.",
            Archetype::ALL,
            Archetype::id,
            Archetype::doc,
        ),
        section(
            "Silhouette meshes",
            "`mesh` of a body part and `silhouette` of a legacy projectile form.",
            Silhouette::ALL,
            Silhouette::id,
            Silhouette::doc,
        ),
        section("Models", "`body.model`.", Model::ALL, Model::id, Model::doc),
        section(
            "Satellite layouts",
            "`body.satellites.layout`.",
            SatelliteLayout::ALL,
            SatelliteLayout::id,
            SatelliteLayout::doc,
        ),
        section("Trails", "`body.trail`.", Trail::ALL, Trail::id, Trail::doc),
        section(
            "Markers",
            "`body.marker`.",
            Marker::ALL,
            Marker::id,
            Marker::doc,
        ),
        section(
            "Part behaviours",
            "`behave` of a body part.",
            Behaviour::ALL,
            Behaviour::id,
            Behaviour::doc,
        ),
        section(
            "Altitudes",
            "`body.altitude`.",
            Altitude::ALL,
            Altitude::id,
            Altitude::doc,
        ),
        section(
            "Expire kinds",
            "`body.expire`.",
            ExpireKind::ALL,
            ExpireKind::id,
            ExpireKind::doc,
        ),
        Section {
            title: "Cast accent patterns",
            note: "`cast.pattern` and `cast.recast`. A recast accent always draws the default lead shape. \
                   `rune_mark` must name a lead of `arc`, `chevron`, `crescent` or `ringlet`, so it cannot \
                   be a recast accent. A row with a windup may use only a charge pattern.",
            columns: &[
                "ID",
                "What the player sees",
                "Default lead shape",
                "Base extent (units)",
                "Charge pattern",
            ],
            rows: AccentPattern::ALL
                .iter()
                .map(|pattern| {
                    vec![
                        format!("`{}`", pattern.id()),
                        pattern.doc().to_string(),
                        lead(pattern.default_lead()),
                        format!("{:.1}", pattern.base_extent()),
                        if pattern.is_charge() { "yes" } else { "no" }.to_string(),
                    ]
                })
                .collect(),
        },
        section(
            "Move patterns",
            "`cast.move.pattern`.",
            MovePattern::ALL,
            MovePattern::id,
            MovePattern::doc,
        ),
        section(
            "Recast markers",
            "`cast.recast_marker`.",
            RecastMarker::ALL,
            RecastMarker::id,
            RecastMarker::doc,
        ),
        Section {
            title: "Impact kinds",
            note: "`impact.kind`. `pierce_through` needs a skill that pierces and `blast` one with area \
                   damage; `arc` may not lead `chain_snap` or `facet_pop`. Every kind closes its \
                   burst with the flash of the hit: a glow in the lead colour at the receipt.",
            columns: &["ID", "What the player sees", "Default lead shape"],
            rows: ImpactKind::ALL
                .iter()
                .map(|kind| {
                    vec![
                        format!("`{}`", kind.id()),
                        kind.doc().to_string(),
                        lead(Some(kind.default_lead())),
                    ]
                })
                .collect(),
        },
        section(
            "Particle shapes",
            "`shape` of an accent, a move, a link or an impact.",
            ParticleShape::ALL,
            ParticleShape::id,
            ParticleShape::doc,
        ),
        section(
            "Palette slots",
            "`slot` and `slots`. A basic-attack row may not name `primary`.",
            PaletteSlot::ALL,
            PaletteSlot::id,
            PaletteSlot::doc,
        ),
        section(
            "Audio bases",
            "`base` of a sound cue.",
            AudioBase::ALL,
            AudioBase::id,
            AudioBase::doc,
        ),
        section(
            "Audio slices",
            "`slice` of a sound cue.",
            AudioSlice::ALL,
            AudioSlice::id,
            AudioSlice::doc,
        ),
        section(
            "Projectile forms",
            "`form` of a profile in `combat_visuals.json`.",
            ProjectileForm::ALL,
            ProjectileForm::id,
            ProjectileForm::doc,
        ),
        section(
            "Projectile presentations",
            "`presentation` of a profile in `combat_visuals.json`.",
            ProjectilePresentation::ALL,
            ProjectilePresentation::id,
            ProjectilePresentation::doc,
        ),
        section(
            "Preview shapes",
            "Derived from the skill. Not authorable.",
            PreviewShape::ALL,
            PreviewShape::id,
            PreviewShape::doc,
        ),
        section(
            "Stage rules",
            "Derived from the skill and the replicated kind. Not authorable.",
            StageRule::ALL,
            StageRule::id,
            StageRule::doc,
        ),
    ];
    let mut out = String::from(
        "# Skill presentation vocabulary\n\n\
         Generated from `client/src/skill_presentation/vocab.rs`. Do not edit by hand; run\n\
         `OMOBA_WRITE_SKILL_VOCABULARY=1 cargo test -p client --lib vocabulary_doc_is_current`.\n\n\
         These are the only IDs a row of `client/assets/config/skills.skillfx` (schema 2) or a\n\
         profile of `client/assets/config/combat_visuals.json` may name. Motion IDs are the clips\n\
         of `client/assets/animations/humanoid-motion-v1.json` (see `docs/humanoid-motion.md`).\n",
    );
    for section in sections {
        out.push_str(&format!("\n## {}\n\n{}\n\n", section.title, section.note));
        out.push_str(&format!("| {} |\n", section.columns.join(" | ")));
        out.push_str(&format!(
            "| {} |\n",
            vec!["---"; section.columns.len()].join(" | ")
        ));
        for row in section.rows {
            out.push_str(&format!("| {} |\n", row.join(" | ")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use std::fmt::Debug;

    /// Every ID of a list is spelled in snake case, is unique, has a description and parses
    /// back to its variant; nothing else parses.
    fn round_trips<T: Copy + PartialEq + Debug + DeserializeOwned>(
        all: &[T],
        id: fn(T) -> &'static str,
        doc: fn(T) -> &'static str,
    ) -> usize {
        let mut seen = std::collections::BTreeSet::new();
        for item in all {
            let name = id(*item);
            assert!(
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_'),
                "{name}"
            );
            assert!(seen.insert(name), "duplicate ID {name}");
            assert!(!doc(*item).is_empty(), "{name}");
            let parsed: T = serde_json::from_value(serde_json::json!(name)).unwrap();
            assert_eq!(parsed, *item);
        }
        assert!(serde_json::from_value::<T>(serde_json::json!("not_a_vocabulary_id")).is_err());
        all.len()
    }

    #[test]
    fn every_vocabulary_id_round_trips() {
        macro_rules! sizes {
            ($($name:ident: $count:literal,)+) => {
                $(assert_eq!(
                    round_trips($name::ALL, $name::id, $name::doc),
                    $count,
                    stringify!($name)
                );)+
            };
        }
        sizes! {
            MotionPhase: 4,
            Archetype: 8,
            Silhouette: 15,
            Model: 5,
            SatelliteLayout: 9,
            Trail: 5,
            Marker: 5,
            Behaviour: 13,
            Altitude: 3,
            ExpireKind: 5,
            AccentPattern: 17,
            MovePattern: 6,
            RecastMarker: 3,
            ImpactKind: 17,
            ParticleShape: 13,
            PaletteSlot: 4,
            AudioBase: 7,
            AudioSlice: 4,
            ProjectileForm: 7,
            ProjectilePresentation: 3,
            PreviewShape: 16,
            StageRule: 4,
        }
    }

    #[test]
    fn defaults_are_the_values_a_row_gets_when_it_names_nothing() {
        assert_eq!(Trail::default(), Trail::None);
        assert_eq!(Marker::default(), Marker::None);
        assert_eq!(Behaviour::default(), Behaviour::Steady);
        assert_eq!(ExpireKind::default(), ExpireKind::None);
        assert_eq!(PaletteSlot::default(), PaletteSlot::Primary);
        assert_eq!(AudioSlice::default(), AudioSlice::Full);
        assert_eq!(
            ProjectilePresentation::default(),
            ProjectilePresentation::Projectile
        );
    }

    #[test]
    fn accent_and_impact_tables_are_complete() {
        for pattern in AccentPattern::ALL {
            // Only `none` draws nothing; only it and the replicated strike line have no reach.
            assert_eq!(
                pattern.default_lead().is_none(),
                *pattern == AccentPattern::None
            );
            assert_eq!(
                pattern.base_extent() == 0.0,
                matches!(pattern, AccentPattern::None | AccentPattern::StrikeLine)
            );
            assert!(pattern.base_extent() <= 2.0, "{}", pattern.id());
        }
        let charge: Vec<_> = AccentPattern::ALL
            .iter()
            .filter(|pattern| pattern.is_charge())
            .map(|pattern| pattern.id())
            .collect();
        assert_eq!(
            charge,
            [
                "rising_motes",
                "inward_gather",
                "spiral_up",
                "rune_mark",
                "none"
            ]
        );
        assert_eq!(
            AccentPattern::ThrustLine.default_lead(),
            Some(ParticleShape::Streak)
        );
        assert_eq!(ImpactKind::ChainSnap.default_lead(), ParticleShape::Chevron);
        assert_eq!(ImpactKind::FacetPop.default_lead(), ParticleShape::Diamond);
        assert_eq!(ImpactKind::Blast.default_lead(), ParticleShape::Glow);
    }

    /// Set `OMOBA_WRITE_SKILL_VOCABULARY=1` to regenerate the document.
    #[test]
    fn vocabulary_doc_is_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/skill-vocabulary.md");
        let rendered = render_markdown();
        if std::env::var_os("OMOBA_WRITE_SKILL_VOCABULARY").is_some() {
            std::fs::write(path, &rendered).unwrap();
        }
        assert!(
            std::fs::read_to_string(path).is_ok_and(|current| current == rendered),
            "docs/skill-vocabulary.md is stale; run OMOBA_WRITE_SKILL_VOCABULARY=1 cargo test -p client --lib vocabulary_doc_is_current"
        );
    }
}
