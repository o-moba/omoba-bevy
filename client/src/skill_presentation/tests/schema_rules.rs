//! One rejecting case per validation rule of schema 2, and the matrix of picks that must
//! agree with the catalog. Cases are built by mutating a registry as a `serde_json::Value`,
//! so they do not depend on the formatting of the packaged file.
use super::super::category::{self, Category, SkillKey};
use super::super::vocab::{AccentPattern, ImpactKind};
use super::super::*;
use serde_json::{Value, json};

const SHIPPED: &str = include_str!("../../../assets/config/skills.skillfx");

fn shipped() -> Value {
    serde_json::from_str(SHIPPED).unwrap()
}

pub(in crate::skill_presentation) fn parse(config: &Value) -> Result<SkillPresentation, String> {
    SkillPresentation::parse(&config.to_string())
}

/// The orb is one object ordered by two skills, so both rows carry this body.
fn orb() -> Value {
    json!({
        "archetype": "orbiter", "altitude": "high", "model": "orb",
        "core": { "mesh": "torus", "slot": "secondary", "size": [0.72, 0.72, 0.72], "behave": "tumble" },
        "shell": { "mesh": "torus", "size": [0.95, 0.95, 0.95], "behave": "gyro" },
        "satellites": { "mesh": "ball", "layout": "orbit", "count": 2, "size": [0.12, 0.12, 0.12],
                        "slot": "accent" },
        "trail": "ribbon", "marker": "owner_tether"
    })
}

/// The packaged registry with complete schema-2 rows for skills of every category and two
/// basic attacks. Every case below changes one thing in it.
pub(in crate::skill_presentation) fn samples() -> Value {
    let mut config = shipped();
    let rows = json!({
        "winter_shard": {
            "home": "frostguard", "release": "punch", "color": [0.10, 0.45, 0.85], "hdr_gain": 3.2,
            "motion": { "rate": 0.9, "start": 0.15 },
            "cast": { "pattern": "muzzle_flash", "shape": "diamond", "count": 4, "scale": 0.9 },
            "body": { "archetype": "traveller", "altitude": "chest",
                      "core": { "mesh": "shard", "size": [0.4, 0.4, 2.4], "behave": "tumble" },
                      "satellites": { "mesh": "diamond", "layout": "halo", "count": 3,
                                      "size": [0.2, 0.2, 0.2], "slot": "accent", "behave": "spin" },
                      "trail": "motes" },
            "impact": { "kind": "shard_burst", "shape": "diamond", "count": 6 },
            "sound": { "cast": { "base": "holy", "speed": 1.3, "slice": "tick" },
                       "impact": { "base": "holy", "speed": 1.4, "slice": "body" } }
        },
        "dawn_field": {
            "home": "dawnweaver", "release": "kneel_plant", "color": [1.0, 0.66, 0.5],
            "motion": { "rate": 1.6, "start": 0.25, "recast": "draw_in", "recast_rate": 1.8 },
            "cast": { "pattern": "ground_ring", "shape": "star", "count": 6, "link": "streak",
                      "recast": "none", "recast_marker": "ring_pips" },
            "body": { "archetype": "zone", "fill": true,
                      "core": { "mesh": "star", "size": [0.5, 0.05, 0.5], "behave": "spin" },
                      "satellites": { "mesh": "diamond", "layout": "rim", "count": 8,
                                      "size": [0.1, 0.1, 0.1], "behave": "blink_last" },
                      "marker": "remaining_ring", "expire": "detonate" },
            "impact": { "kind": "ring_burst", "shape": "star" },
            "sound": { "cast": { "base": "holy", "speed": 1.2 },
                       "impact": { "base": "holy", "speed": 1.2, "slice": "tick" },
                       "recast": { "base": "holy", "speed": 0.8, "slice": "body",
                                   "notes": [{ "delay_ms": 100, "speed": 1.0, "gain": 0.7 }] } }
        },
        "dawn_ray": {
            "home": "dawnweaver", "windup": "spell_prepare", "release": "cast",
            "color": [1.0, 0.82, 0.32], "hdr_gain": 4.2,
            "motion": { "rate": 0.8, "phase": "warn_fire", "fit_windup": true },
            "cast": { "pattern": "inward_gather", "shape": "streak" },
            "body": { "archetype": "lane", "fill": true,
                      "shell": { "mesh": "block", "size": [0.9, 0.2, 1.0], "behave": "hide_in_telegraph" },
                      "satellites": { "mesh": "block", "layout": "stagger", "count": 8,
                                      "size": [0.1, 0.3, 0.1], "behave": "only_in_telegraph" },
                      "marker": "fill_to_edge", "expire": "fade" },
            "impact": { "kind": "pierce_through", "shape": "streak" },
            "sound": { "cast": { "base": "holy", "speed": 1.25 },
                       "release": { "base": "holy", "speed": 0.7 },
                       "impact": { "base": "holy", "speed": 1.4, "slice": "tail" } }
        },
        "furnace_breath": {
            "home": "cinderforge", "windup": "fist_guard_loop", "release": "two_hand_push",
            "color": [1.0, 0.62, 0.14], "hdr_gain": 3.4,
            "motion": { "phase": "fuse", "start": 0.25 },
            "cast": { "pattern": "inward_gather", "shape": "diamond", "link": "drop" },
            "body": { "archetype": "sector", "fill": true, "marker": "fill_to_edge",
                      "satellites": { "mesh": "drop", "layout": "fan", "count": 2,
                                      "size": [0.2, 0.2, 0.4], "slot": "accent", "behave": "flicker" },
                      "expire": "discharge" },
            "impact": { "kind": "spark_fork", "shape": "drop" },
            "sound": { "cast": { "base": "tower", "speed": 0.75, "slice": "tail" },
                       "release": { "base": "tower", "speed": 0.75 },
                       "impact": { "base": "tower", "speed": 0.8, "slice": "tail" } }
        },
        "thunder_pulse": {
            "home": "stormfist", "release": "ground_pound", "color": [0.80, 0.90, 1.0], "hdr_gain": 2.4,
            "secondary": [0.12, 0.62, 1.0],
            "motion": { "rate": 1.2, "recast": "point_command", "recast_rate": 1.1 },
            "cast": { "pattern": "ground_ring", "shape": "ringlet", "count": 8, "scale": 0.75,
                      "lifetime": 0.45, "recast": "none", "link": "ringlet", "area": true,
                      "recast_marker": "ring_pips" },
            "impact": { "kind": "ring_burst", "shape": "ringlet", "count": 6, "scale": 0.7,
                        "lifetime": 0.40, "slots": ["accent", "primary"] },
            "sound": { "cast": { "base": "caster", "speed": 1.20,
                                 "notes": [{ "delay_ms": 70, "speed": 0.95, "gain": 0.7 },
                                           { "delay_ms": 160, "speed": 0.75, "gain": 0.5 }] },
                       "recast": { "base": "caster", "speed": 1.35, "slice": "tick" },
                       "impact": { "base": "tower", "speed": 1.15, "slice": "tick", "gain": 0.7 } }
        },
        "anchor_step": {
            "home": "stormfist", "release": "leap_land", "color": [0.44, 0.65, 0.85], "hdr_gain": 2.4,
            "accent": [0.55, 1.0, 0.88],
            "motion": { "recast": "rally_raise", "recast_rate": 1.2 },
            "cast": { "pattern": "shield_flash", "shape": "kite", "count": 3, "lifetime": 0.40,
                      "recast": "spiral_up", "move": { "pattern": "leap_arc", "shape": "ringlet" } },
            "aux": { "anchor": { "archetype": "zone", "fill": false, "altitude": "ground",
                                 "core": { "mesh": "chevron", "slot": "secondary",
                                           "size": [0.8, 0.05, 0.8], "behave": "spin" },
                                 "marker": "remaining_ring" } },
            "sound": { "cast": { "base": "caster", "speed": 1.15, "slice": "body", "gain": 0.8 },
                       "recast": { "base": "caster", "speed": 1.25, "slice": "body", "gain": 0.6 } }
        },
        "orbital_command": {
            "home": "orbitwright", "release": "point_command", "color": [0.12, 0.88, 1.0], "hdr_gain": 3.2,
            "motion": { "rate": 0.9, "start": 0.1 },
            "cast": { "pattern": "toss_arc", "shape": "glow", "count": 3, "scale": 0.6 },
            "aux": { "orb": orb() },
            "impact": { "kind": "facet_pop", "shape": "ringlet", "count": 2 },
            "sound": { "cast": { "base": "holy", "speed": 0.85, "slice": "body" },
                       "impact": { "base": "holy", "speed": 0.9, "slice": "tick", "gain": 0.7 } }
        },
        "orbital_guard": {
            "home": "orbitwright", "release": "cast_thrust_r", "color": [1.0, 0.84, 0.50], "hdr_gain": 2.6,
            "accent": [0.12, 0.88, 1.0],
            "motion": { "start": 0.2 },
            "cast": { "pattern": "shield_flash", "shape": "ringlet", "count": 3, "scale": 0.8 },
            "aux": { "orb": orb() },
            "impact": { "kind": "glow_pop" },
            "sound": { "cast": { "base": "holy", "speed": 0.8 },
                       "impact": { "base": "holy", "speed": 0.95, "slice": "tick" } }
        },
        "orbital_collapse": {
            "home": "orbitwright", "windup": "levitate_loop", "release": "draw_in",
            "color": [0.30, 0.66, 1.0], "hdr_gain": 4.4,
            "motion": { "phase": "fuse", "rate": 1.4 },
            "cast": { "pattern": "rune_mark", "shape": "arc", "count": 1 },
            "body": { "archetype": "zone", "altitude": "ground", "fill": true,
                      "core": { "mesh": "cone", "slot": "accent", "size": [0.06, 0.34, 0.06], "behave": "pulse" },
                      "shell": { "mesh": "torus", "size": [0.2, 0.2, 0.2] },
                      "marker": "fill_to_edge", "expire": "discharge" },
            "impact": { "kind": "blast", "count": 8, "scale": 1.2 },
            "sound": { "cast": { "base": "holy", "speed": 0.7 },
                       "release": { "base": "holy", "speed": 0.7, "slice": "tail" },
                       "impact": { "base": "holy", "speed": 0.75, "slice": "body" } }
        },
        "heroic_strike": {
            "home": "warrior", "release": "slash_down", "color": [1.0, 0.42, 0.08], "hdr_gain": 3.4,
            "motion": { "rate": 0.85, "start": 0.12 },
            "cast": { "pattern": "muzzle_burst", "shape": "crescent" },
            "impact": { "kind": "slash_cut" },
            "sound": { "cast": { "base": "melee", "speed": 0.75, "slice": "body" },
                       "impact": { "base": "melee", "speed": 0.75 } }
        },
        "battle_rally": {
            "home": "warrior", "release": "rally_raise", "color": [1.0, 0.8, 0.52], "hdr_gain": 2.6,
            "secondary": [0.55, 0.33, 0.1],
            "motion": { "rate": 0.85, "start": 0.1 },
            "cast": { "pattern": "rising_motes", "shape": "chevron", "count": 6 },
            "sound": { "cast": { "base": "melee", "speed": 0.8, "slice": "tail" } }
        },
        "dagger_bluff": {
            "home": "adventurer", "release": "dagger_feint", "color": [0.86, 0.74, 0.52], "hdr_gain": 1.2,
            "motion": { "rate": 1.1 },
            "cast": { "pattern": "muzzle_flash", "shape": "glow" },
            "sound": { "cast": { "base": "bluff" } }
        }
    });
    let basics = json!({
        "stormfist": {
            "motions": ["shoulder_drive", "thrust_lunge"], "rate": 1.5,
            "accent": { "pattern": "muzzle_flash", "shape": "ringlet", "count": 2, "scale": 0.5,
                        "lifetime": 0.18, "slots": ["secondary", "accent"] },
            "impact": { "kind": "flash_star", "shape": "star", "count": 3, "scale": 0.55,
                        "lifetime": 0.18, "slots": ["white", "accent"] },
            "sound": { "base": "tower", "speed": 1.40, "slice": "tick", "gain": 0.6 }
        },
        "wildspark": {
            "motions": ["pistol_shoot"], "rate": 1.35,
            "impact": { "kind": "splinter", "shape": "streak", "slots": ["accent", "white"] },
            "rockets": {
                "motions": ["pistol_shoot"], "rate": 0.8,
                "accent": { "pattern": "fan_spray", "shape": "star", "count": 4, "scale": 0.8,
                            "slots": ["secondary", "accent"] },
                "impact": { "kind": "ember_puff", "shape": "star", "slots": ["secondary", "accent"] },
                "sound": { "base": "caster", "speed": 0.7, "slice": "body", "gain": 0.8 }
            }
        }
    });
    for (id, row) in rows.as_object().unwrap() {
        config["skills"][id] = row.clone();
    }
    config["basic_attacks"] = basics;
    config
}

/// Sets `path` (a JSON pointer) to `value`, creating the last key when it is missing.
fn set(config: &mut Value, path: &str, value: Value) {
    let (parent, key) = path.rsplit_once('/').unwrap();
    let parent = config
        .pointer_mut(parent)
        .unwrap_or_else(|| panic!("no {parent}"));
    match parent {
        Value::Array(items) => items[key.parse::<usize>().unwrap()] = value,
        other => other[key] = value,
    }
}

fn remove(config: &mut Value, path: &str) {
    let (parent, key) = path.rsplit_once('/').unwrap();
    let removed = config
        .pointer_mut(parent)
        .and_then(Value::as_object_mut)
        .and_then(|object| object.remove(key));
    assert!(removed.is_some(), "no {path}");
}

fn rejects(config: &Value, expected: &str, case: &str) {
    match parse(config) {
        Ok(_) => panic!("{case}: accepted"),
        Err(error) => assert!(
            error.contains(expected),
            "{case}: expected `{expected}` in `{error}`"
        ),
    }
}

/// `samples()` with one value set.
fn with(path: &str, value: Value) -> Value {
    let mut config = samples();
    set(&mut config, path, value);
    config
}

fn without(path: &str) -> Value {
    let mut config = samples();
    remove(&mut config, path);
    config
}

fn cue() -> Value {
    json!({ "base": "holy", "speed": 1.0 })
}

#[test]
fn sample_rows_of_every_category_parse() {
    let registry = parse(&samples()).unwrap();
    assert_eq!(registry.rows().count(), shared::HeroClass::ALL.len() * 4);
    let shard = registry.profile(SkillId::WinterShard).unwrap();
    assert!(shard.migrated() && shard.effect.is_none());
    assert_eq!(shard.motion.rate, 0.9);
    assert_eq!(shard.body.as_ref().unwrap().trail, vocab::Trail::Motes);
    // Blocks a row does not name keep their defaults.
    let rally = registry.row("battle_rally").unwrap();
    assert_eq!(rally.cast.as_ref().unwrap().lifetime, 0.35);
    assert_eq!(rally.cast.as_ref().unwrap().scale, 1.0);
    assert!(rally.body.is_none() && rally.impact.is_none() && rally.aux.is_empty());
    let untouched = registry.profile(SkillId::IronHook).unwrap();
    assert!(!untouched.migrated());
    assert_eq!(untouched.motion, schema::MotionPlayback::default());
    assert_eq!(untouched.hdr_gain, 3.2);
    // The phase follows the skill's own telegraph only when the row holds a windup.
    let phase = |id: SkillId| registry.profile(id).unwrap().phase(SkillKey::Modular(id));
    assert_eq!(phase(SkillId::DawnRay), vocab::MotionPhase::WarnFire);
    assert_eq!(phase(SkillId::FurnaceBreath), vocab::MotionPhase::Fuse);
    assert_eq!(phase(SkillId::HorizonWave), vocab::MotionPhase::WarnFire);
    assert_eq!(phase(SkillId::WinterShard), vocab::MotionPhase::Instant);
    assert_eq!(phase(SkillId::MirrorGuard), vocab::MotionPhase::Instant);
    let wildspark = registry.basic(shared::HeroClass::Wildspark).unwrap();
    assert_eq!(wildspark.rockets.as_ref().unwrap().rate, 0.8);
    assert!(registry.basic(shared::HeroClass::Mage).is_none());
}

#[test]
fn top_level_rules_reject_wrong_versions_sizes_themes_and_ids() {
    rejects(
        &with("/schema_version", json!(1)),
        "schema_version 1",
        "version",
    );
    rejects(
        &with("/schema_version", json!(3)),
        "schema_version 3",
        "newer version",
    );
    rejects(
        &with("/damage", json!(999)),
        "unknown field `damage`",
        "top-level field",
    );
    let padded = format!("{SHIPPED}{}", " ".repeat(256 * 1024));
    assert!(
        SkillPresentation::parse(&padded).is_err_and(|error| error.contains("256 KiB")),
        "size"
    );
    let mut crowded = samples();
    let row = crowded["skills"]["iron_hook"].clone();
    for index in 0..13 {
        crowded["skills"][format!("extra_{index}")] = row.clone();
    }
    rejects(&crowded, "Too many skill profiles", "row count");

    rejects(
        &without("/themes/mage"),
        "no theme for mage",
        "missing theme",
    );
    rejects(
        &with(
            "/themes/bard",
            json!({ "secondary": [0, 0, 0], "accent": [1, 1, 1] }),
        ),
        "unknown class bard",
        "unknown theme",
    );
    rejects(
        &with("/themes/mage/accent/0", json!(1.5)),
        "themes.mage: invalid color",
        "theme colour",
    );
    rejects(
        &with("/themes/mage/glow", json!(2)),
        "unknown field `glow`",
        "theme field",
    );
    let mut renamed = samples();
    let row = renamed["skills"]
        .as_object_mut()
        .unwrap()
        .remove("iron_hook")
        .unwrap();
    renamed["skills"]["fireball"] = row;
    rejects(&renamed, "Unknown skill fireball", "skill id");
}

#[test]
fn row_rules_reject_wrong_homes_palettes_and_missing_blocks() {
    rejects(
        &with("/skills/winter_shard/home", json!("mage")),
        "winter_shard: `home` must be frostguard",
        "home of a modular skill",
    );
    rejects(
        &with("/skills/battle_rally/home", json!("warden")),
        "battle_rally: `home` must be warrior",
        "home of a legacy ability",
    );
    rejects(
        &without("/skills/iron_hook/home"),
        "missing field `home`",
        "no home",
    );
    rejects(
        &with("/skills/winter_shard/damage", json!(999)),
        "unknown field `damage`",
        "profile field",
    );
    for path in ["color/1", "secondary/0", "accent/2"] {
        let mut config = samples();
        set(
            &mut config,
            "/skills/winter_shard/secondary",
            json!([0.9, 0.9, 1.0]),
        );
        set(
            &mut config,
            "/skills/winter_shard/accent",
            json!([0.5, 0.6, 1.0]),
        );
        set(
            &mut config,
            &format!("/skills/winter_shard/{path}"),
            json!(1.5),
        );
        rejects(&config, "winter_shard: Invalid color", path);
    }
    rejects(
        &with("/skills/winter_shard/hdr_gain", json!(9.0)),
        "Invalid HDR gain",
        "gain",
    );
    // The legacy look stays until `cast` replaces it; afterwards every block is required.
    rejects(
        &without("/skills/iron_hook/effect"),
        "iron_hook: `effect` is required",
        "unmigrated row without effect",
    );
    rejects(
        &without("/skills/winter_shard/body"),
        "needs `body`",
        "replicated effect without body",
    );
    rejects(
        &without("/skills/winter_shard/impact"),
        "needs `impact`",
        "damaging row without impact",
    );
    rejects(
        &without("/skills/winter_shard/sound"),
        "needs `sound.cast`",
        "row without sound",
    );
    rejects(
        &without("/skills/winter_shard/sound/cast"),
        "needs `sound.cast`",
        "row without a cast voice",
    );
    rejects(
        &with("/skills/winter_shard/color", json!([0.8, 0.9, 1.0])),
        "luminance",
        "colour too close to the matter colour",
    );
    // Blocks the catalog rules out.
    let body = samples()["skills"]["winter_shard"]["body"].clone();
    rejects(
        &with("/skills/thunder_pulse/body", body.clone()),
        "thunder_pulse: `body` needs",
        "body on an instant skill",
    );
    rejects(
        &with("/skills/heroic_strike/body", body),
        "heroic_strike: `body` needs",
        "body on a legacy ability",
    );
    rejects(
        &with("/skills/anchor_step/impact", json!({ "kind": "glow_pop" })),
        "anchor_step: `impact` needs",
        "impact on a shield skill",
    );
    rejects(
        &with("/skills/battle_rally/impact", json!({ "kind": "glow_pop" })),
        "battle_rally: `impact` needs",
        "impact on a self heal",
    );
}

#[test]
fn motion_rules_reject_unknown_clips_wrong_phases_and_late_contacts() {
    let shard = |field: &str, value: Value| with(&format!("/skills/winter_shard/{field}"), value);
    rejects(
        &shard("release", json!("missing_clip")),
        "Unknown motion missing_clip",
        "clip",
    );
    rejects(
        &shard("release", json!("idle")),
        "idle is a base state",
        "base state",
    );
    rejects(
        &shard("release", json!("levitate_loop")),
        "is a loop",
        "loop as release",
    );
    rejects(&shard("motion/rate", json!(2.5)), "motion.rate", "rate");
    rejects(&shard("motion/start", json!(0.9)), "motion.start", "start");
    rejects(
        &shard("motion/recast_rate", json!(0.4)),
        "motion.recast_rate",
        "recast rate",
    );
    rejects(
        &shard("motion/mirror", json!(true)),
        "unknown field `mirror`",
        "motion field",
    );
    rejects(
        &shard("motion/phase", json!("fuse")),
        "motion.phase fuse is not the phase of this skill (instant)",
        "phase of a skill without a telegraph",
    );
    rejects(
        &with("/skills/dawn_ray/motion/phase", json!("parry")),
        "not the phase of this skill (warn_fire)",
        "phase of another telegraph",
    );
    rejects(
        &shard("windup", json!("aim_hold_loop")),
        "`windup` needs a skill with a telegraph",
        "windup without a telegraph",
    );
    rejects(
        &with("/skills/heroic_strike/windup", json!("spell_prepare")),
        "`windup` needs a skill with a telegraph",
        "windup on a legacy ability",
    );
    rejects(
        &with("/skills/dawn_ray/motion/phase", json!("instant")),
        "`windup` needs a non-instant motion.phase",
        "windup released at once",
    );
    rejects(
        &without("/skills/furnace_breath/windup"),
        "a non-instant motion.phase needs a `windup`",
        "phase without a pose to hold",
    );
    rejects(
        &with("/skills/dawn_ray/windup", json!("missing_clip")),
        "Unknown motion missing_clip",
        "windup clip",
    );
    rejects(
        &with("/skills/dawn_ray/windup", json!("run")),
        "run is a base state",
        "windup base state",
    );
    rejects(
        &with("/skills/furnace_breath/motion/fit_windup", json!(true)),
        "motion.fit_windup needs",
        "fitting a loop",
    );
    rejects(
        &shard("motion/fit_windup", json!(true)),
        "motion.fit_windup needs",
        "fitting without a windup",
    );
    rejects(
        &shard("motion/recast", json!("draw_in")),
        "motion.recast needs a skill with a recast",
        "recast clip without a recast",
    );
    rejects(
        &with("/skills/dawn_field/motion/recast", json!("levitate_loop")),
        "motion.recast: levitate_loop is a loop",
        "loop as recast",
    );
    // The contact rule: nothing may delay the pose that sells the accepted cast.
    rejects(
        &shard("motion", json!({ "rate": 0.5 })),
        "release: punch reaches its contact 0.500 s after the cast",
        "slow release",
    );
    rejects(
        &with("/skills/dawn_field/motion/recast_rate", json!(0.5)),
        "motion.recast: draw_in reaches its contact",
        "slow recast",
    );
    // A telegraphed release is not started by the cast edge, so the rule does not bind it.
    assert!(parse(&with("/skills/furnace_breath/motion/rate", json!(0.5))).is_ok());
}

#[test]
fn cast_rules_reject_out_of_range_values_and_picks_the_skill_cannot_back() {
    let shard =
        |field: &str, value: Value| with(&format!("/skills/winter_shard/cast/{field}"), value);
    rejects(
        &shard("at", json!("arrival")),
        "unknown field `at`",
        "cast field",
    );
    rejects(
        &shard("pattern", json!("line_mark")),
        "unknown variant `line_mark`",
        "pattern id",
    );
    rejects(&shard("count", json!(0)), "cast.count", "no particles");
    rejects(
        &shard("count", json!(9)),
        "cast.count",
        "too many particles",
    );
    rejects(&shard("scale", json!(0.3)), "cast.scale", "scale");
    rejects(&shard("lifetime", json!(0.6)), "cast.lifetime", "lifetime");
    rejects(
        &with("/skills/heroic_strike/cast/scale", json!(1.5)),
        "cast.scale lets muzzle_burst reach beyond 2.0 units",
        "extent",
    );
    rejects(
        &with("/skills/dawn_ray/cast/pattern", json!("muzzle_flash")),
        "needs a charge pattern",
        "release pattern on a windup row",
    );
    rejects(
        &shard("pattern", json!("strike_line")),
        "strike_line needs",
        "strike line without an own effect",
    );
    rejects(
        &with("/skills/battle_rally/cast/pattern", json!("shield_flash")),
        "a self heal may not use a shield shape",
        "shield pattern on a heal",
    );
    rejects(
        &with("/skills/battle_rally/cast/shape", json!("kite")),
        "a self heal may not use a shield shape",
        "shield shape on a heal",
    );
    rejects(
        &shard("recast", json!("none")),
        "cast.recast needs a skill with a recast",
        "recast accent without a recast",
    );
    rejects(
        &with("/skills/dawn_field/cast/recast", json!("strike_line")),
        "strike_line needs",
        "strike line as a recast accent",
    );
    rejects(
        &with("/skills/dawn_field/cast/recast", json!("rune_mark")),
        "cast.recast: the default lead of rune_mark is not legal",
        "recast accent with a status lead",
    );
    rejects(
        &shard("recast_marker", json!("ring_pips")),
        "cast.recast_marker needs",
        "recast marker without a recast",
    );
    rejects(
        &with(
            "/skills/thunder_pulse/cast/move",
            json!({ "pattern": "afterimage" }),
        ),
        "cast.move needs a skill that can move its caster",
        "move on a standing skill",
    );
    rejects(
        &with("/skills/anchor_step/cast/link", json!("streak")),
        "cast.link needs a skill that can deal damage",
        "link without damage",
    );
    rejects(
        &shard("link", json!("streak")),
        "cast.link would precede the hit of a travelling body",
        "link ahead of a projectile",
    );
    rejects(
        &with("/skills/heroic_strike/cast/link", json!("streak")),
        "cast.link would precede the hit of a travelling body",
        "link ahead of a legacy projectile",
    );
    rejects(
        &with("/skills/dawn_field/cast/area", json!(true)),
        "cast.area is not signed off",
        "area without sign-off",
    );
    rejects(
        &with("/skills/heroic_strike/cast/area", json!(true)),
        "cast.area is not signed off",
        "area on a legacy ability",
    );
    for lead in ["diamond", "star"] {
        rejects(
            &with("/skills/orbital_collapse/cast/shape", json!(lead)),
            "rune_mark needs a lead of arc, chevron, crescent or ringlet",
            lead,
        );
    }
    rejects(
        &without("/skills/orbital_collapse/cast/shape"),
        "rune_mark needs a lead of",
        "default lead of rune_mark",
    );
}

#[test]
fn impact_and_sound_rules_reject_claims_without_a_matching_fact() {
    let impact =
        |field: &str, value: Value| with(&format!("/skills/winter_shard/impact/{field}"), value);
    rejects(
        &impact("stun", json!(true)),
        "unknown field `stun`",
        "impact field",
    );
    rejects(
        &impact("kind", json!("vital_break")),
        "unknown variant `vital_break`",
        "engine kind",
    );
    rejects(&impact("count", json!(13)), "impact.count", "count");
    rejects(&impact("scale", json!(0.2)), "impact.scale", "scale");
    rejects(
        &impact("lifetime", json!(1.3)),
        "impact.lifetime",
        "lifetime",
    );
    rejects(
        &impact("kind", json!("blast")),
        "impact.kind blast needs a skill with area damage",
        "blast without an area",
    );
    rejects(
        &impact("kind", json!("pierce_through")),
        "impact.kind pierce_through needs a skill that pierces",
        "pierce without a pierce",
    );
    for kind in ["facet_pop", "chain_snap"] {
        rejects(
            &with(
                "/skills/winter_shard/impact",
                json!({ "kind": kind, "shape": "arc" }),
            ),
            "arc may not lead",
            kind,
        );
    }

    let sound =
        |field: &str, value: Value| with(&format!("/skills/winter_shard/sound/{field}"), value);
    rejects(
        &sound("cast/speed", json!(1.23)),
        "sound.cast: speed must be",
        "speed off the grid",
    );
    rejects(
        &sound("cast/speed", json!(1.45)),
        "sound.cast: speed must be",
        "speed out of range",
    );
    rejects(&sound("cast/gain", json!(0.1)), "sound.cast: gain", "gain");
    rejects(
        &sound("cast/base", json!("kill")),
        "unknown variant `kill`",
        "game-state cue",
    );
    rejects(
        &sound("cast/pitch", json!(2)),
        "unknown field `pitch`",
        "cue field",
    );
    let note = |delay: u32, speed: f32, gain: f32| json!({ "delay_ms": delay, "speed": speed, "gain": gain });
    rejects(
        &sound(
            "cast/notes",
            json!([note(50, 1.0, 0.5), note(100, 1.0, 0.5), note(150, 1.0, 0.5)]),
        ),
        "at most 2 extra notes",
        "three notes",
    );
    rejects(
        &sound("cast/notes", json!([note(300, 1.0, 0.5)])),
        "at most 240 ms",
        "late note",
    );
    rejects(
        &sound("cast/notes", json!([note(50, 1.02, 0.5)])),
        "note speed",
        "note speed",
    );
    rejects(
        &sound("cast/notes", json!([note(50, 1.0, 1.5)])),
        "note gain",
        "note gain",
    );
    rejects(
        &sound("cast/base", json!("bluff")),
        "sound.cast: bluff belongs to dagger_bluff",
        "bluff on another skill",
    );
    rejects(
        &sound("release", cue()),
        "sound.release needs a non-instant phase",
        "release voice without a telegraph",
    );
    rejects(
        &sound("recast", cue()),
        "sound.recast needs a skill with a recast",
        "recast voice without a recast",
    );
    rejects(
        &with("/skills/anchor_step/sound/impact", cue()),
        "sound.impact needs a skill that can deal damage",
        "impact voice without damage",
    );
}

#[test]
fn body_rules_reject_shapes_the_replicated_effect_cannot_have() {
    let shard =
        |field: &str, value: Value| with(&format!("/skills/winter_shard/body/{field}"), value);
    let field =
        |field: &str, value: Value| with(&format!("/skills/dawn_field/body/{field}"), value);
    rejects(
        &shard("anchor", json!("owner")),
        "unknown field `anchor`",
        "body field",
    );
    rejects(
        &shard("core/mesh", json!("disc")),
        "unknown variant `disc`",
        "engine mesh",
    );
    rejects(
        &shard("model", json!("colossus")),
        "unknown variant `colossus`",
        "retired model",
    );
    for (id, archetype) in [
        ("winter_shard", "zone"),
        ("dawn_field", "traveller"),
        ("dawn_ray", "sector"),
        ("furnace_breath", "lane"),
        ("orbital_collapse", "lane"),
    ] {
        rejects(
            &with(&format!("/skills/{id}/body/archetype"), json!(archetype)),
            &format!("{id}: body: archetype {archetype} cannot show this effect"),
            id,
        );
    }
    let mut bare = samples();
    remove(&mut bare, "/skills/winter_shard/body/core");
    remove(&mut bare, "/skills/winter_shard/body/satellites");
    rejects(
        &bare,
        "body: needs `core`, `model` or `satellites`",
        "empty body",
    );
    rejects(
        &shard("core/size/0", json!(0.0)),
        "core.size must be finite and positive",
        "flat part",
    );
    rejects(
        &shard("satellites/count", json!(9)),
        "satellites.count",
        "satellite count",
    );
    rejects(
        &shard("satellites/layout", json!("rim")),
        "satellites.layout rim is not legal for traveller",
        "layout",
    );
    rejects(
        &field("trail", json!("ribbon")),
        "trail needs a traveller or an orbiter",
        "trail",
    );
    rejects(
        &shard("fill", json!(true)),
        "fill needs an area archetype",
        "fill",
    );
    rejects(
        &with("/skills/dawn_ray/body/model", json!("orb")),
        "model is not legal for lane",
        "model",
    );
    for (config, marker) in [
        (shard("marker", json!("remaining_ring")), "remaining_ring"),
        // A fuse is removed by its firing tick; a countdown ring would promise an expiry.
        (
            with(
                "/skills/orbital_collapse/body/marker",
                json!("remaining_ring"),
            ),
            "remaining_ring",
        ),
        (field("marker", json!("arming_pips")), "arming_pips"),
        (field("marker", json!("fill_to_edge")), "fill_to_edge"),
        (field("marker", json!("owner_tether")), "owner_tether"),
    ] {
        rejects(
            &config,
            &format!("marker {marker} is not legal for this body"),
            marker,
        );
    }
    for (config, expire) in [
        (shard("expire", json!("fade")), "fade"),
        (field("expire", json!("crumble")), "crumble"),
        (field("expire", json!("discharge")), "discharge"),
        (
            with("/skills/orbital_collapse/body/expire", json!("detonate")),
            "detonate",
        ),
        (
            with("/skills/furnace_breath/body/expire", json!("fade")),
            "fade",
        ),
    ] {
        rejects(
            &config,
            &format!("expire {expire} is not legal for this body"),
            expire,
        );
    }
    rejects(
        &shard("core/behave", json!("gyro")),
        "core: gyro belongs to the orb",
        "gyro",
    );
    rejects(
        &field("core/behave", json!("only_after_renew")),
        "core: only_after_renew needs",
        "renew on a zone",
    );
    let mut ring = samples();
    set(
        &mut ring,
        "/skills/winter_shard/body/core/mesh",
        json!("torus"),
    );
    set(
        &mut ring,
        "/skills/winter_shard/body/core/behave",
        json!("only_after_renew"),
    );
    rejects(&ring, "core: only_after_renew needs", "renew on a ring");
    rejects(
        &shard("satellites/behave", json!("rise_on_spawn")),
        "satellites: rise_on_spawn needs a skill with a fixed spawn lifetime",
        "rise on a projectile",
    );
    // Accepted where the fact holds.
    assert!(parse(&shard("core/behave", json!("only_after_renew"))).is_ok());
}

#[test]
fn body_rules_bound_the_size_and_the_number_of_parts() {
    // `winter_shard`: a traveller at chest height, replicated radius 0.6, cooldown 8 s.
    let shard =
        |field: &str, value: Value| with(&format!("/skills/winter_shard/body/{field}"), value);
    // `dawn_field`: a zone of radius 3, cooldown 10 s.
    let field =
        |field: &str, value: Value| with(&format!("/skills/dawn_field/body/{field}"), value);

    // A body in flight is sized in metres and stays near its hit circle.
    assert!(parse(&shard("core/size", json!([0.9, 2.0, 3.0]))).is_ok());
    rejects(
        &shard("core/size", json!([0.95, 0.4, 2.4])),
        "body: core.size is 0.95 wide (at most 0.90000004 for a radius of 0.6)",
        "wide part",
    );
    rejects(
        &shard("satellites/size", json!([1.0, 0.2, 0.2])),
        "body: satellites.size is 1 wide",
        "wide satellites",
    );
    for size in [[0.4, 2.1, 2.4], [0.4, 0.4, 3.1]] {
        rejects(
            &shard("core/size", json!(size)),
            "body: core.size may be at most 2.0 high and 3.0 long",
            "tall or long part",
        );
    }
    // A thin bolt may still be half a unit wide (`dawn_bind`: radius 0.35).
    let bolt = |width: f32| {
        with(
            "/skills/dawn_bind/body",
            json!({ "archetype": "traveller",
                    "core": { "mesh": "ball", "size": [width, 0.3, 0.3] } }),
        )
    };
    assert!(parse(&bolt(0.525)).is_ok());
    rejects(
        &bolt(0.53),
        "dawn_bind: body: core.size is 0.53 wide",
        "thin bolt",
    );
    // On the ground a part reaches at most the radius from the centre.
    let mut skimming = shard("altitude", json!("ground"));
    rejects(
        &skimming,
        "body: core.size: a part on the ground may reach at most the radius (0.6)",
        "long part on the ground",
    );
    set(
        &mut skimming,
        "/skills/winter_shard/body/core/size",
        json!([0.4, 0.4, 1.2]),
    );
    assert!(parse(&skimming).is_ok());

    // An area body is sized in multiples of its radius, as full extents, and stays inside
    // its boundary. The core in the middle of a circle may span it up to the inner edge
    // of its line: the star of the row turns, and its points reach 0.94 of the radius.
    assert!(parse(&field("core/size", json!([1.0, 4.0, 1.0]))).is_ok());
    assert!(parse(&field("core/size", json!([1.88, 0.05, 1.88]))).is_ok());
    rejects(
        &field("core/size", json!([1.9, 0.05, 0.5])),
        "body: core.size leaves the boundary",
        "wide part of a zone",
    );
    // The measure is taken on the mesh: the corners of a block reach farther than the
    // points of a star of the same size.
    let block = |size: f32| {
        with(
            "/skills/dawn_field/body/core",
            json!({ "mesh": "block", "size": [size, 0.05, size] }),
        )
    };
    assert!(parse(&block(1.3)).is_ok());
    rejects(
        &block(1.4),
        "body: core.size leaves the boundary",
        "block in a zone",
    );
    // Copies stand around the middle, so each keeps to half the circle.
    rejects(
        &field("satellites/size", json!([0.1, 0.1, 1.01])),
        "body: satellites.size leaves the boundary",
        "long satellites of a zone",
    );
    // Neither does a strip let a part cross its outline.
    rejects(
        &with("/skills/dawn_ray/body/shell/size", json!([1.1, 0.2, 1.0])),
        "body: shell.size leaves the boundary",
        "wide part of a lane",
    );
    // A part that turns end over end sweeps its height across the ground.
    let mut tumbling = field("core/behave", json!("tumble"));
    set(
        &mut tumbling,
        "/skills/dawn_field/body/core/size",
        json!([0.5, 1.5, 0.5]),
    );
    assert!(parse(&tumbling).is_ok());
    set(
        &mut tumbling,
        "/skills/dawn_field/body/core/size",
        json!([0.5, 1.9, 0.5]),
    );
    rejects(
        &tumbling,
        "body: core.size leaves the boundary",
        "tall tumbling part",
    );

    // The trail of a body in flight may be scaled, and stays near the hit circle like
    // every part of it: the motes of the shard (radius 0.6) are 0.36 wide at scale 1.
    assert!(parse(&shard("trail_scale", json!(2.5))).is_ok());
    for scale in [0.4, 2.6] {
        rejects(
            &shard("trail_scale", json!(scale)),
            "body: trail_scale must be within 0.5..=2.5",
            "trail scale range",
        );
    }
    let mut chevrons = shard("trail", json!("chevrons"));
    set(
        &mut chevrons,
        "/skills/winter_shard/body/trail_scale",
        json!(1.05),
    );
    assert!(parse(&chevrons).is_ok());
    set(
        &mut chevrons,
        "/skills/winter_shard/body/trail_scale",
        json!(1.2),
    );
    rejects(
        &chevrons,
        "body: trail_scale makes the trail 1.01 wide (at most 0.90 for a radius of 0.6)",
        "wide trail",
    );
    let mut bare = shard("trail_scale", json!(1.5));
    remove(&mut bare, "/skills/winter_shard/body/trail");
    rejects(&bare, "body: trail_scale needs a trail", "trail scale");
    // The strength of the interior layer belongs to a body that has one.
    assert!(parse(&field("fill_strength", json!(1.0))).is_ok());
    for strength in [0.3, 1.1] {
        rejects(
            &field("fill_strength", json!(strength)),
            "body: fill_strength must be within 0.4..=1",
            "fill strength range",
        );
    }
    rejects(
        &shard("fill_strength", json!(0.8)),
        "body: fill_strength needs a body with a fill",
        "fill strength in flight",
    );

    // Twelve mesh parts: the ring, the core, the satellites and three motes.
    assert!(parse(&shard("satellites/count", json!(7))).is_ok());
    rejects(
        &shard("satellites/count", json!(8)),
        "winter_shard: body: draws 13 mesh parts (at most 12 for this cooldown)",
        "part budget",
    );
    // The zone is full: ring, fill, marker, core and eight satellites.
    rejects(
        &field("shell", json!({ "mesh": "ring", "size": [0.3, 0.3, 0.3] })),
        "dawn_field: body: draws 13 mesh parts",
        "zone part budget",
    );
    // A cooldown of 40 s or more allows eighteen: the ray has four boundary parts.
    assert!(
        parse(&with(
            "/skills/dawn_ray/body/core",
            json!({ "mesh": "block", "size": [0.3, 0.4, 1.0] }),
        ))
        .is_ok()
    );
    let cage = |count: u8| {
        with(
            "/skills/iron_boundary/body",
            json!({ "archetype": "cage",
                    "core": { "mesh": "torus", "size": [0.1, 0.1, 0.1] },
                    "satellites": { "mesh": "torus", "layout": "rim", "count": count,
                                    "size": [0.1, 0.3, 0.1] } }),
        )
    };
    assert!(parse(&cage(7)).is_ok());
    rejects(
        &cage(8),
        "iron_boundary: body: draws 19 mesh parts (at most 18 for this cooldown)",
        "long cooldown part budget",
    );
    // An auxiliary body is held to the budget of the skill that declares it.
    let mut orbs = samples();
    for id in ["orbital_command", "orbital_guard"] {
        set(
            &mut orbs,
            &format!("/skills/{id}/aux/orb/satellites/count"),
            json!(6),
        );
    }
    rejects(
        &orbs,
        "aux.orb: draws 13 mesh parts (at most 12 for this cooldown)",
        "aux part budget",
    );
}

#[test]
fn the_rows_together_fit_the_material_budget() {
    // One spark colour for each row is still inside the budget.
    let mut config = samples();
    let ids: Vec<String> = config["skills"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(ids.len(), 68);
    for (index, id) in ids.iter().enumerate() {
        let shade = index as f64 / 100.0;
        set(
            &mut config,
            &format!("/skills/{id}/accent"),
            json!([shade, 0.9, 0.5]),
        );
    }
    assert!(parse(&config).is_ok());
    // A skill colour costs three materials. With a colour and a matter colour of its own
    // for every row that is free to take them, the rows no longer fit.
    let mut own = 0;
    for (index, id) in ids.iter().enumerate() {
        if config["skills"][id].get("cast").is_some() {
            continue;
        }
        own += 1;
        let shade = index as f64 / 100.0;
        set(
            &mut config,
            &format!("/skills/{id}/color"),
            json!([0.9, shade, 0.4]),
        );
        set(
            &mut config,
            &format!("/skills/{id}/secondary"),
            json!([0.2, 0.3, shade]),
        );
    }
    assert!(own >= 50, "{own}");
    rejects(&config, "effect materials (at most 272)", "material budget");
}

#[test]
fn aux_rules_bind_secondary_objects_to_the_skills_that_own_them() {
    rejects(
        &with("/skills/winter_shard/aux", json!({ "orb": orb() })),
        "winter_shard: aux.orb: the skill has no such secondary object",
        "orb of another kit",
    );
    rejects(
        &with("/skills/heroic_strike/aux", json!({ "orb": orb() })),
        "aux.orb: the skill has no such secondary object",
        "aux on a legacy ability",
    );
    rejects(
        &with("/skills/anchor_step/aux/field", orb()),
        "aux.field: the skill has no such secondary object",
        "kind the skill does not replicate",
    );
    rejects(
        &with("/skills/anchor_step/aux/anchor/archetype", json!("lane")),
        "anchor_step: aux.anchor: archetype lane cannot show this effect",
        "archetype of an aux body",
    );
    // Positional ids cannot be followed, so nothing may be claimed about how they end.
    rejects(
        &with("/skills/anchor_step/aux/anchor/expire", json!("fade")),
        "aux.anchor: expire fade is not legal",
        "expire on a positional id",
    );
    rejects(
        &with(
            "/skills/anchor_step/aux/anchor/marker",
            json!("owner_tether"),
        ),
        "aux.anchor: marker owner_tether is not legal",
        "tether on the anchor",
    );
    rejects(
        &with("/skills/orbital_guard/aux/orb/trail", json!("motes")),
        "orbital_guard: aux.orb must equal the body declared by orbital_command",
        "two different orbs",
    );
    // One of the two rows may leave the orb to the other.
    assert!(parse(&without("/skills/orbital_guard/aux")).is_ok());
}

#[test]
fn basic_attack_rules_keep_the_class_body_inside_its_theme() {
    let basic =
        |field: &str, value: Value| with(&format!("/basic_attacks/stormfist/{field}"), value);
    rejects(
        &with(
            "/basic_attacks/bard",
            samples()["basic_attacks"]["stormfist"].clone(),
        ),
        "basic_attacks.bard: unknown class",
        "class id",
    );
    rejects(
        &basic("color", json!([1, 1, 1])),
        "unknown field `color`",
        "basic field",
    );
    rejects(
        &basic("motions", json!([])),
        "`motions` needs one or two clips",
        "no clip",
    );
    rejects(
        &basic("motions", json!(["punch", "guard", "cast"])),
        "`motions` needs one or two clips",
        "three clips",
    );
    rejects(
        &basic("motions", json!(["missing_clip"])),
        "Unknown motion missing_clip",
        "clip",
    );
    rejects(
        &basic("motions", json!(["fist_guard_loop"])),
        "is a loop",
        "loop",
    );
    rejects(
        &basic("motions", json!(["death"])),
        "death is a base state",
        "base state",
    );
    rejects(
        &basic("rate", json!(2.5)),
        "basic_attacks.stormfist: rate",
        "rate",
    );
    rejects(
        &basic("start", json!(0.9)),
        "basic_attacks.stormfist: start",
        "start",
    );
    rejects(
        &basic("rate", json!(0.5)),
        "motions: shoulder_drive reaches its contact",
        "slow basic attack",
    );
    for (field, value) in [
        ("recast", json!("none")),
        ("move", json!({ "pattern": "afterimage" })),
        ("link", json!("streak")),
        ("area", json!(true)),
        ("recast_marker", json!("ring_pips")),
    ] {
        rejects(
            &basic(&format!("accent/{field}"), value),
            &format!("a basic attack accent has no `{field}`"),
            field,
        );
    }
    rejects(
        &basic("accent/count", json!(9)),
        "accent.count",
        "accent count",
    );
    rejects(
        &basic("accent/lifetime", json!(0.05)),
        "accent.lifetime",
        "accent lifetime",
    );
    rejects(
        &basic("accent/pattern", json!("strike_line")),
        "strike_line needs",
        "strike line",
    );
    rejects(
        &basic("accent/slots", json!(["primary", "accent"])),
        "accent.slots must be named without `primary`",
        "skill colour in an accent",
    );
    let mut unnamed = samples();
    remove(&mut unnamed, "/basic_attacks/stormfist/accent/slots");
    rejects(
        &unnamed,
        "accent.slots must be named",
        "default accent slots",
    );
    rejects(
        &basic("impact/slots", json!(["white", "primary"])),
        "impact.slots must be named without `primary`",
        "skill colour in an impact",
    );
    rejects(
        &basic("impact/kind", json!("blast")),
        "blast needs",
        "blast",
    );
    rejects(
        &basic("impact/kind", json!("pierce_through")),
        "pierce_through needs",
        "pierce",
    );
    rejects(
        &basic("sound/base", json!("bluff")),
        "sound: bluff belongs",
        "bluff",
    );
    rejects(
        &basic("sound/speed", json!(0.72)),
        "sound: speed must be",
        "speed",
    );
    let rockets = samples()["basic_attacks"]["wildspark"]["rockets"].clone();
    rejects(
        &basic("rockets", rockets.clone()),
        "`rockets` needs the repeater attack profile",
        "rockets without a repeater",
    );
    rejects(
        &with("/basic_attacks/wildspark/rockets/rockets", rockets),
        "rockets: `rockets` cannot nest",
        "nested rockets",
    );
    rejects(
        &with("/basic_attacks/wildspark/rockets/rate", json!(3.0)),
        "basic_attacks.wildspark: rockets: rate",
        "rate of the rocket mode",
    );
}

/// The row of one skill with a single block added; everything else is as packaged.
fn shipped_with(id: &str, block: &str, value: Value) -> Value {
    let mut config = shipped();
    config["skills"][id][block] = value;
    config
}

/// `Some(error)` when the registry is rejected.
fn error_of(config: &Value) -> Option<String> {
    parse(config).err()
}

/// Whether adding `block` to the packaged row is rejected for the reason `expected` names.
fn refuses(id: &str, block: &str, value: Value, expected: &str) -> bool {
    error_of(&shipped_with(id, block, value)).is_some_and(|error| error.contains(expected))
}

/// The required and forbidden matrix: for every one of the 68 rows, a pick is accepted only
/// where the derived fact holds.
#[test]
fn authored_picks_agree_with_the_catalog() {
    let registry = parse(&shipped()).unwrap();
    let ids: Vec<String> = registry.rows().map(|(id, _)| id.to_string()).collect();
    assert_eq!(ids.len(), 68);
    for id in &ids {
        let key = SkillKey::from_id(id).unwrap();
        let skill = key.modular();
        let windup = registry.row(id).unwrap().windup.is_some();
        let damaging = category::can_damage(key);
        let replicated = category::category(key) == Category::ReplicatedEffect;

        // Impact: only a skill that can produce a receipt of its own.
        let impact =
            |kind: ImpactKind| error_of(&shipped_with(id, "impact", json!({ "kind": kind.id() })));
        assert_eq!(
            impact(ImpactKind::GlowPop).is_some_and(|e| e.contains("`impact` needs")),
            !damaging,
            "{id}: impact"
        );
        if damaging {
            assert_eq!(impact(ImpactKind::GlowPop), None, "{id}: impact");
            assert_eq!(
                impact(ImpactKind::PierceThrough).is_none(),
                category::pierces(key),
                "{id}: pierce_through"
            );
            assert_eq!(
                impact(ImpactKind::Blast).is_none(),
                category::area_damage(key),
                "{id}: blast"
            );
        }

        // Body: only a skill whose cast replicates a world effect.
        let body = json!({ "archetype": "traveller",
                           "core": { "mesh": "ball", "size": [0.3, 0.3, 0.3] } });
        assert_eq!(
            refuses(id, "body", body, "`body` needs"),
            !replicated,
            "{id}: body"
        );

        // Aux: only the secondary objects the server tags with this skill.
        for kind in [
            EffectVisualKind::Orb,
            EffectVisualKind::Soul,
            EffectVisualKind::Anchor,
            EffectVisualKind::Healing,
            EffectVisualKind::BeamWarning,
            EffectVisualKind::Bolt,
        ] {
            let name = category::kind_id(kind);
            let owned = skill.is_some_and(|skill| category::aux_kinds(skill).contains(&kind));
            assert_eq!(
                refuses(
                    id,
                    "aux",
                    json!({ name: orb() }),
                    "no such secondary object"
                ),
                !owned,
                "{id}: aux.{name}"
            );
        }

        // Windup and phase: only against a telegraph of the skill's own.
        let telegraphed = skill
            .is_some_and(|skill| category::derived_phase(skill) != vocab::MotionPhase::Instant);
        assert_eq!(
            refuses(
                id,
                "windup",
                json!("aim_hold_loop"),
                "`windup` needs a skill with a telegraph"
            ),
            !telegraphed,
            "{id}: windup"
        );
        for phase in [
            vocab::MotionPhase::WarnFire,
            vocab::MotionPhase::Fuse,
            vocab::MotionPhase::Parry,
        ] {
            let own = skill.is_some_and(|skill| category::derived_phase(skill) == phase);
            assert_eq!(
                refuses(
                    id,
                    "motion",
                    json!({ "phase": phase.id() }),
                    "is not the phase of this skill"
                ),
                !own,
                "{id}: phase {}",
                phase.id()
            );
        }
        assert_eq!(
            refuses(
                id,
                "motion",
                json!({ "recast": "draw_in" }),
                "motion.recast needs"
            ),
            !skill.is_some_and(category::has_recast),
            "{id}: motion.recast"
        );

        // Cast: every modifier needs its fact. A charge pattern keeps windup rows legal.
        let cast = |fields: Value| {
            let mut cast = json!({ "pattern": "none" });
            for (field, value) in fields.as_object().unwrap() {
                cast[field] = value.clone();
            }
            error_of(&shipped_with(id, "cast", cast)).unwrap_or_default()
        };
        let recast = skill.is_some_and(category::has_recast);
        assert_eq!(
            cast(json!({ "recast": "none" })).contains("cast.recast needs"),
            !recast,
            "{id}: cast.recast"
        );
        assert_eq!(
            cast(json!({ "recast_marker": "ring_pips" })).contains("cast.recast_marker needs"),
            !recast,
            "{id}: cast.recast_marker"
        );
        let moves = skill.is_some_and(|skill| {
            category::movement_capable(skill, false) || category::movement_capable(skill, true)
        });
        assert_eq!(
            cast(json!({ "move": { "pattern": "afterimage" } })).contains("cast.move needs"),
            !moves,
            "{id}: cast.move"
        );
        let linked = damaging
            && (!category::travelling_body(key) || skill.is_some_and(category::recast_instant_hit));
        assert_eq!(
            cast(json!({ "link": "streak" })).contains("cast.link"),
            !linked,
            "{id}: cast.link"
        );
        assert_eq!(
            cast(json!({ "area": true })).contains("cast.area is not signed off"),
            !skill.is_some_and(|skill| category::AREA_FLASH_SIGNED_OFF.contains(&skill)),
            "{id}: cast.area"
        );
        if !windup {
            assert_eq!(
                cast(json!({ "pattern": "strike_line" })).contains("strike_line needs"),
                !skill.is_some_and(category::own_effect_strike),
                "{id}: strike_line"
            );
        }
        for pattern in AccentPattern::ALL {
            let refused = cast(json!({ "pattern": pattern.id(), "shape": "arc" }))
                .contains("needs a charge pattern");
            assert_eq!(
                refused,
                windup && !pattern.is_charge(),
                "{id}: {}",
                pattern.id()
            );
        }
        assert_eq!(
            cast(json!({ "pattern": "shield_flash" })).contains("a self heal may not"),
            category::self_heal(key),
            "{id}: shield_flash"
        );

        // Sound: a voice for a moment that cannot happen is refused.
        let sound = |slot: &str| {
            error_of(&shipped_with(id, "sound", json!({ slot: cue() }))).unwrap_or_default()
        };
        assert_eq!(
            sound("impact").contains("sound.impact needs"),
            !damaging,
            "{id}: sound.impact"
        );
        assert_eq!(
            sound("recast").contains("sound.recast needs"),
            !recast,
            "{id}: sound.recast"
        );
        // Without a windup a packaged row releases on the cast edge.
        assert_eq!(
            sound("release").contains("sound.release needs"),
            !windup,
            "{id}: sound.release"
        );
        assert_eq!(
            error_of(&shipped_with(
                id,
                "sound",
                json!({ "cast": { "base": "bluff" } })
            ))
            .is_some(),
            id != "dagger_bluff",
            "{id}: bluff"
        );
    }
}
