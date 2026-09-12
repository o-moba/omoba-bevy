//! Versioned gameplay objects on the fixed, authored Verdant arena.
//! Configuration is resolved once by the server; visual profiles never alter collision.
use crate::navigation::{Bounds, Point, world_navigation};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub const GEOMETRY_ID: &str = "verdant-confluence-v1";
pub const BASE_PAD_SIZE: f32 = 46.0;
pub const BASE_EDGE_MARGIN: f32 = 6.0;
pub const LANE_WIDTH: f32 = 12.0;
pub const LANE_EDGE_PADDING: f32 = 6.0;
pub const TARGET_BASE_DISTANCE: f32 = 225.0;
pub const MAX_STRUCTURES: usize = 32;
pub const MAX_CONFIG_BYTES: usize = 65_536;
pub const DEFAULT_JSON: &str = include_str!("../assets/maps/verdant.json");

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Top,
    Mid,
    Bot,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Team {
    Green,
    Blue,
}

#[derive(Clone, Copy, Debug)]
pub struct ArenaGeometry {
    pub home: Point,
    pub away: Point,
    pub bounds: Bounds,
    pub left_x: f32,
    pub right_x: f32,
    pub top_z: f32,
    pub bottom_z: f32,
}

pub fn geometry() -> ArenaGeometry {
    let half_inner = TARGET_BASE_DISTANCE / 2.0_f32.sqrt() * 0.5;
    let half_map = half_inner + BASE_PAD_SIZE * 0.5 + BASE_EDGE_MARGIN;
    let edge = half_map - LANE_EDGE_PADDING - LANE_WIDTH * 0.5;
    ArenaGeometry {
        home: [-half_inner; 2],
        away: [half_inner; 2],
        bounds: Bounds {
            min: [-half_map; 2],
            max: [half_map; 2],
        },
        left_x: -edge,
        right_x: edge,
        top_z: edge,
        bottom_z: -edge,
    }
}

/// Authored road order, from Green to Blue. Preserve the legacy corner spurs
/// here: tower placement uses this arc length, while marching omits the spurs.
pub fn lane_points(lane: Lane) -> Vec<Point> {
    let g = geometry();
    match lane {
        Lane::Mid => vec![g.home, g.away],
        Lane::Top => vec![
            g.home,
            [g.left_x, g.home[1]],
            [g.left_x, g.top_z],
            [g.right_x, g.top_z],
            [g.away[0], g.top_z],
            g.away,
        ],
        Lane::Bot => vec![
            g.home,
            [g.home[0], g.bottom_z],
            [g.left_x, g.bottom_z],
            [g.right_x, g.bottom_z],
            [g.right_x, g.away[1]],
            g.away,
        ],
    }
}

pub fn minion_lane_points(lane: Lane) -> Vec<Point> {
    let mut points = lane_points(lane);
    match lane {
        Lane::Top => {
            points.remove(3);
        }
        Lane::Bot => {
            points.remove(2);
        }
        Lane::Mid => {}
    }
    points
}

pub fn sample_lane(lane: Lane, t: f32) -> Point {
    let points = lane_points(lane);
    let lengths: Vec<f32> = points.windows(2).map(|p| distance(p[0], p[1])).collect();
    let mut remaining = lengths.iter().sum::<f32>() * t.clamp(0.0, 1.0);
    for (index, length) in lengths.into_iter().enumerate() {
        if remaining <= length {
            let fraction = remaining / length;
            return [
                points[index][0] + (points[index + 1][0] - points[index][0]) * fraction,
                points[index][1] + (points[index + 1][1] - points[index][1]) * fraction,
            ];
        }
        remaining -= length;
    }
    *points.last().expect("lanes have endpoints")
}
/// Project onto the authored road; tied points on its legacy return spur use
/// the branch nearest the instance's requested t rather than changing branches.
fn effective_lane_progress(lane: Lane, position: Point, authored_t: f32) -> f32 {
    project_onto_lane(&lane_points(lane), position, authored_t).1
}

fn project_onto_lane(points: &[Point], position: Point, authored_t: f32) -> (f32, f32) {
    let lengths: Vec<_> = points
        .windows(2)
        .map(|pair| distance(pair[0], pair[1]))
        .collect();
    let total: f32 = lengths.iter().sum();
    let mut traversed = 0.0;
    let mut best_distance = f32::INFINITY;
    let mut best_progress = authored_t;
    for (pair, length) in points.windows(2).zip(lengths) {
        let delta = [pair[1][0] - pair[0][0], pair[1][1] - pair[0][1]];
        let fraction = (((position[0] - pair[0][0]) * delta[0]
            + (position[1] - pair[0][1]) * delta[1])
            / (length * length))
            .clamp(0.0, 1.0);
        let projected = [
            pair[0][0] + delta[0] * fraction,
            pair[0][1] + delta[1] * fraction,
        ];
        let distance = distance(position, projected);
        let progress = (traversed + length * fraction) / total;
        if distance < best_distance - 0.0001
            || ((distance - best_distance).abs() <= 0.0001
                && (progress - authored_t).abs() < (best_progress - authored_t).abs())
        {
            best_distance = distance;
            best_progress = progress;
        }
        traversed += length;
    }
    (best_distance, best_progress)
}

fn distance(a: Point, b: Point) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureStats {
    pub max_hp: f32,
    pub attack_range: f32,
    pub attack_damage: f32,
    pub attack_cooldown_ms: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StatOverrides {
    pub max_hp: Option<f32>,
    pub attack_range: Option<f32>,
    pub attack_damage: Option<f32>,
    pub attack_cooldown_ms: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Placement {
    Tower {
        lane: Lane,
        t: f32,
        #[serde(default)]
        offset: Point,
    },
    Base,
}
#[derive(Clone, Debug, Serialize)]
pub struct StructureDefinition {
    pub id: u64,
    pub key: String,
    pub team: Team,
    #[serde(flatten)]
    pub placement: Placement,
    pub profile: String,
    pub visual_profile: String,
    #[serde(default)]
    pub overrides: StatOverrides,
}
// `flatten` with a unit enum variant can silently discard placement fields.
// Decode the flat contributor schema strictly before constructing the variant.
impl<'de> Deserialize<'de> for StructureDefinition {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            id: u64,
            key: String,
            team: Team,
            kind: String,
            profile: String,
            visual_profile: String,
            lane: Option<Lane>,
            t: Option<f32>,
            offset: Option<Point>,
            #[serde(default)]
            overrides: StatOverrides,
        }
        let fields = Fields::deserialize(deserializer)?;
        let placement = match fields.kind.as_str() {
            "tower" => Placement::Tower {
                lane: fields
                    .lane
                    .ok_or_else(|| serde::de::Error::missing_field("lane"))?,
                t: fields
                    .t
                    .ok_or_else(|| serde::de::Error::missing_field("t"))?,
                offset: fields.offset.unwrap_or_default(),
            },
            "base" => {
                if fields.lane.is_some() || fields.t.is_some() || fields.offset.is_some() {
                    return Err(serde::de::Error::custom(
                        "base anchors are fixed; lane, t and offset are not allowed",
                    ));
                }
                Placement::Base
            }
            _ => {
                return Err(serde::de::Error::unknown_variant(
                    &fields.kind,
                    &["tower", "base"],
                ));
            }
        };
        Ok(Self {
            id: fields.id,
            key: fields.key,
            team: fields.team,
            placement,
            profile: fields.profile,
            visual_profile: fields.visual_profile,
            overrides: fields.overrides,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapDefinition {
    pub format_version: u32,
    pub geometry_id: String,
    pub map_profile: String,
    pub profiles: BTreeMap<String, StructureStats>,
    pub structures: Vec<StructureDefinition>,
}
#[derive(Clone, Debug)]
pub struct ResolvedStructure {
    pub id: u64,
    pub key: String,
    pub team: Team,
    pub lane: Option<Lane>,
    /// Zero is the outermost tower. Higher tiers unlock toward the base.
    pub tier: u8,
    pub position: Point,
    pub stats: StructureStats,
    pub visual_profile: String,
}
#[derive(Clone, Debug)]
pub struct ResolvedMap {
    pub geometry_id: String,
    pub map_profile: String,
    pub structures: Vec<ResolvedStructure>,
}
impl Default for ResolvedMap {
    fn default() -> Self {
        MapDefinition::from_json(DEFAULT_JSON)
            .and_then(|map| map.resolve())
            .expect("embedded Verdant map is valid")
    }
}
impl MapDefinition {
    pub fn from_json(source: &str) -> Result<Self, String> {
        if source.len() > MAX_CONFIG_BYTES {
            return Err("map config exceeds 65536 bytes".into());
        }
        serde_json::from_str(source).map_err(|error| format!("invalid map JSON: {error}"))
    }
    pub fn resolve(&self) -> Result<ResolvedMap, String> {
        if self.format_version != 1 {
            return Err("unsupported map format_version (expected 1)".into());
        }
        if self.geometry_id != GEOMETRY_ID {
            return Err(format!("unsupported geometry_id; expected {GEOMETRY_ID}"));
        }
        if !valid_key(&self.map_profile) || self.profiles.is_empty() || self.profiles.len() > 32 {
            return Err(
                "map_profile must be a stable key and profiles must contain 1..32 entries".into(),
            );
        }
        for (name, stats) in &self.profiles {
            if !valid_key(name) {
                return Err(format!("invalid profile key {name:?}"));
            }
            validate_stats(stats).map_err(|error| format!("profile {name}: {error}"))?;
        }
        if !(2..=MAX_STRUCTURES).contains(&self.structures.len()) {
            return Err("map requires 2..32 structures including two bases".into());
        }
        let mut ids = HashSet::new();
        let mut keys = HashSet::new();
        let mut bases = [0; 2];
        let mut resolved = Vec::new();
        let mut progress = Vec::new();
        let mut effective_progress = Vec::new();
        for item in &self.structures {
            if item.id == 0
                || !ids.insert(item.id)
                || !valid_key(&item.key)
                || !keys.insert(&item.key)
                || !valid_key(&item.visual_profile)
            {
                return Err(format!(
                    "structure {:?}: unique nonzero id, unique stable key and visual_profile required",
                    item.key
                ));
            }
            let mut stats = self
                .profiles
                .get(&item.profile)
                .ok_or_else(|| format!("structure {}: unknown profile {}", item.key, item.profile))?
                .clone();
            if let Some(value) = item.overrides.max_hp {
                stats.max_hp = value;
            }
            if let Some(value) = item.overrides.attack_range {
                stats.attack_range = value;
            }
            if let Some(value) = item.overrides.attack_damage {
                stats.attack_damage = value;
            }
            if let Some(value) = item.overrides.attack_cooldown_ms {
                stats.attack_cooldown_ms = value;
            }
            validate_stats(&stats).map_err(|error| format!("structure {}: {error}", item.key))?;
            let (lane, position, t) = match item.placement {
                Placement::Tower { lane, t, offset } => {
                    let valid_t = match item.team {
                        Team::Green => (0.08..=0.48).contains(&t),
                        Team::Blue => (0.52..=0.92).contains(&t),
                    };
                    if !valid_t
                        || offset.iter().any(|v| !v.is_finite())
                        || distance(offset, [0.0; 2]) > 3.0
                    {
                        return Err(format!(
                            "structure {}: tower t must lie in its team's half (Green .08..48, Blue .52..92), offset length <=3",
                            item.key
                        ));
                    }
                    let center = sample_lane(lane, t);
                    (
                        Some(lane),
                        [center[0] + offset[0], center[1] + offset[1]],
                        t,
                    )
                }
                Placement::Base => {
                    let index = usize::from(item.team == Team::Blue);
                    bases[index] += 1;
                    (
                        None,
                        if item.team == Team::Green {
                            geometry().home
                        } else {
                            geometry().away
                        },
                        0.0,
                    )
                }
            };
            if let Some(lane) = lane {
                let (route_distance, _) = project_onto_lane(&minion_lane_points(lane), position, t);
                if route_distance > 3.0 + 0.0001 {
                    return Err(format!(
                        "structure {}: placement is farther than 3m from the walked lane; decorative dead-end road spurs cannot host siege towers",
                        item.key
                    ));
                }
            }
            let radius = if lane.is_some() {
                crate::TOWER_TARGET_RADIUS
            } else {
                3.2
            };
            if !world_navigation().point_clear_with_radius(position, radius) {
                return Err(format!(
                    "structure {}: placement intersects fixed arena collision/bounds",
                    item.key
                ));
            }
            for old in &resolved {
                let old: &ResolvedStructure = old;
                let old_radius = if old.lane.is_some() {
                    crate::TOWER_TARGET_RADIUS
                } else {
                    3.2
                };
                if distance(position, old.position) < radius + old_radius + 1.0 {
                    return Err(format!(
                        "structure {}: overlaps {} or leaves no hero clearance",
                        item.key, old.key
                    ));
                }
            }
            for base in [geometry().home, geometry().away] {
                let scale = 7.0 / distance(base, [0.0; 2]);
                let spawn = [base[0] * (1.0 - scale), base[1] * (1.0 - scale)];
                if distance(position, spawn) < radius + crate::navigation::HERO_RADIUS + 0.15 {
                    return Err(format!("structure {}: blocks a player spawn", item.key));
                }
            }
            resolved.push(ResolvedStructure {
                id: item.id,
                key: item.key.clone(),
                team: item.team,
                lane,
                tier: 0,
                position,
                stats,
                visual_profile: item.visual_profile.clone(),
            });
            progress.push(t);
            effective_progress
                .push(lane.map_or(0.0, |lane| effective_lane_progress(lane, position, t)));
        }
        if bases != [1, 1] {
            return Err("exactly one fixed-anchor base per team is required".into());
        }
        // Derive explicit siege order independently of config array order or IDs.
        for team in [Team::Green, Team::Blue] {
            for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
                let mut indices: Vec<usize> = resolved
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.team == team && s.lane == Some(lane))
                    .map(|(i, _)| i)
                    .collect();
                indices.sort_by(|&a, &b| {
                    if team == Team::Green {
                        progress[b].total_cmp(&progress[a])
                    } else {
                        progress[a].total_cmp(&progress[b])
                    }
                });
                for pair in indices.windows(2) {
                    let outer_progress = effective_progress[pair[0]];
                    let inner_progress = effective_progress[pair[1]];
                    let correctly_ordered = if team == Team::Green {
                        outer_progress > inner_progress
                    } else {
                        outer_progress < inner_progress
                    };
                    if !correctly_ordered {
                        return Err(format!(
                            "structures {} and {}: offsets reverse or collapse physical siege order; keep the outer tier farther forward along its lane",
                            resolved[pair[0]].key, resolved[pair[1]].key
                        ));
                    }
                    if (progress[pair[0]] - progress[pair[1]]).abs() < 0.005 {
                        return Err("same-lane tower progress must differ by at least .005".into());
                    }
                }
                for (tier, index) in indices.into_iter().enumerate() {
                    resolved[index].tier = tier as u8;
                }
            }
        }
        Ok(ResolvedMap {
            geometry_id: self.geometry_id.clone(),
            map_profile: self.map_profile.clone(),
            structures: resolved,
        })
    }
}
fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn validate_stats(stats: &StructureStats) -> Result<(), String> {
    if !stats.max_hp.is_finite()
        || !(1.0..=100_000.0).contains(&stats.max_hp)
        || !stats.attack_range.is_finite()
        || !(1.0..=60.0).contains(&stats.attack_range)
        || !stats.attack_damage.is_finite()
        || !(0.0..=10_000.0).contains(&stats.attack_damage)
        || !(100..=60_000).contains(&stats.attack_cooldown_ms)
    {
        return Err("stats require finite HP 1..100000, range 1..60, damage 0..10000, cooldown_ms 100..60000".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn definition() -> MapDefinition {
        MapDefinition::from_json(DEFAULT_JSON).unwrap()
    }
    #[test]
    fn defaults_preserve_authored_anchors_and_canonical_collision_bounds() {
        let map = ResolvedMap::default();
        assert_eq!(map.structures.len(), 8);
        assert_eq!(map.map_profile, "verdant_default");
        for (id, x, z) in [
            (1, -96.54951, 29.5099),
            (2, 4.49010, 96.54951),
            (3, -31.8198, -31.8198),
            (4, 31.8198, 31.8198),
            (5, -4.49010, -96.54951),
            (6, 96.54951, -29.5099),
            (7, -79.54951, -79.54951),
            (8, 79.54951, 79.54951),
        ] {
            let s = map.structures.iter().find(|s| s.id == id).unwrap();
            assert!(distance(s.position, [x, z]) < 0.001, "{} moved", s.key);
        }
        assert_eq!(geometry().bounds, world_navigation().bounds());
        for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
            let points = minion_lane_points(lane);
            assert_eq!(points.first(), Some(&geometry().home));
            assert_eq!(points.last(), Some(&geometry().away));
        }
    }
    #[test]
    fn invalid_identity_numeric_placement_profiles_and_overlap_fail_closed() {
        let mutate: Vec<fn(&mut MapDefinition)> = vec![
            |m| m.format_version = 2,
            |m| m.geometry_id = "other-arena".into(),
            |m| m.map_profile = "../bad".into(),
            |m| m.structures[1].id = m.structures[0].id,
            |m| m.structures[0].id = 0,
            |m| m.structures[1].key = m.structures[0].key.clone(),
            |m| m.structures[0].visual_profile = "http://bad".into(),
            |m| m.structures[0].profile = "absent".into(),
            |m| m.structures[0].overrides.max_hp = Some(f32::NAN),
            |m| m.structures[0].overrides.max_hp = Some(0.0),
            |m| m.structures[0].overrides.attack_range = Some(f32::INFINITY),
            |m| m.structures[0].overrides.attack_cooldown_ms = Some(0),
            |m| {
                m.structures[0].placement = Placement::Tower {
                    lane: Lane::Mid,
                    t: 0.6,
                    offset: [0.0; 2],
                }
            },
            |m| {
                m.structures[0].placement = Placement::Tower {
                    lane: Lane::Mid,
                    t: 0.3,
                    offset: [3.1, 0.0],
                }
            },
            |m| {
                m.structures[0].placement = Placement::Tower {
                    lane: Lane::Mid,
                    t: 0.3,
                    offset: [0.0; 2],
                }
            },
            |m| {
                m.structures.pop();
            },
            |m| m.structures[7].team = Team::Green,
            |m| m.structures = (0..33).map(|_| m.structures[0].clone()).collect(),
        ];
        for (index, change) in mutate.into_iter().enumerate() {
            let mut map = definition();
            change(&mut map);
            assert!(map.resolve().is_err(), "invalid case {index} accepted");
        }
        assert!(MapDefinition::from_json(&" ".repeat(MAX_CONFIG_BYTES + 1)).is_err());
        let source = DEFAULT_JSON.replacen("\"max_hp\": 240.0", "\"max_hpp\": 240.0", 1);
        assert!(MapDefinition::from_json(&source).is_err());
        let source =
            DEFAULT_JSON.replacen("\"kind\": \"base\"", "\"kind\": \"base\", \"x\": 20", 1);
        assert!(
            MapDefinition::from_json(&source).is_err(),
            "bases cannot relocate through ignored fields"
        );
    }
    #[test]
    fn decorative_dead_end_spurs_are_rejected_without_shifting_supported_placements() {
        for (id, lane, t) in [(2, Lane::Top, 0.92), (5, Lane::Bot, 0.08)] {
            let mut map = definition();
            map.structures
                .iter_mut()
                .find(|s| s.id == id)
                .unwrap()
                .placement = Placement::Tower {
                lane,
                t,
                offset: [0.0; 2],
            };
            let error = map.resolve().unwrap_err();
            assert!(error.contains("walked lane"), "{error}");
        }
        assert!(definition().resolve().is_ok());
        let example =
            MapDefinition::from_json(include_str!("../../examples/maps/two-tier.json")).unwrap();
        assert!(example.resolve().is_ok());
        for (id, t) in [(3, 0.30), (4, 0.70)] {
            let mut map = definition();
            map.structures
                .iter_mut()
                .find(|s| s.id == id)
                .unwrap()
                .placement = Placement::Tower {
                lane: Lane::Mid,
                t,
                offset: [2.12132, -2.12132],
            };
            assert!(
                map.resolve().is_ok(),
                "a three-meter perpendicular adjustment remains supported"
            );
        }
    }

    #[test]
    fn offsets_cannot_reverse_physical_siege_order_but_perpendicular_adjustments_work() {
        let component = 2.12132;
        for (index, outer_t, inner_t, sign) in [(2, 0.30, 0.29, 1.0), (3, 0.70, 0.71, -1.0)] {
            let mut map = definition();
            map.structures[index].placement = Placement::Tower {
                lane: Lane::Mid,
                t: outer_t,
                offset: [-sign * component; 2],
            };
            let mut inner = map.structures[index].clone();
            inner.id = 9;
            inner.key = "mid_inner".into();
            inner.placement = Placement::Tower {
                lane: Lane::Mid,
                t: inner_t,
                offset: [sign * component; 2],
            };
            map.structures.push(inner);
            let error = map.resolve().unwrap_err();
            assert!(error.contains("offsets reverse"), "{error}");
            map.structures[index].placement = Placement::Tower {
                lane: Lane::Mid,
                t: outer_t,
                offset: [-component, component],
            };
            map.structures.last_mut().unwrap().placement = Placement::Tower {
                lane: Lane::Mid,
                t: inner_t,
                offset: [component, -component],
            };
            assert!(
                map.resolve().is_ok(),
                "perpendicular offsets preserve tier order"
            );
        }
    }

    #[test]
    fn siege_tiers_follow_lane_progress_independent_of_id_and_json_order() {
        let mut map = definition();
        let mut inner = map.structures[2].clone();
        inner.id = 100;
        inner.key = "inner".into();
        inner.placement = Placement::Tower {
            lane: Lane::Mid,
            t: 0.18,
            offset: [1.0, -1.0],
        };
        inner.overrides.max_hp = Some(420.0);
        map.structures.insert(0, inner);
        map.structures.reverse();
        let resolved = map.resolve().unwrap();
        let inner = resolved.structures.iter().find(|s| s.id == 100).unwrap();
        let outer = resolved.structures.iter().find(|s| s.id == 3).unwrap();
        assert_eq!((outer.tier, inner.tier), (0, 1));
        assert_eq!(inner.stats.max_hp, 420.0);
        assert_eq!(inner.stats.attack_damage, 14.0);
        let center = sample_lane(Lane::Mid, 0.18);
        assert_eq!(inner.position, [center[0] + 1.0, center[1] - 1.0]);
    }
}
