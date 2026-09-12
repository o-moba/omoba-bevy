//! Real UDP map configuration receipts. No teleports, damage or synthetic state.
use harness::{Bot, ServerPacket, ServerProcess, Team};
use shared::map::{DEFAULT_JSON, MapDefinition, ResolvedMap};
use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct ConfigurationFile(PathBuf);
impl ConfigurationFile {
    fn new(json: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "omoba-map-harness-{}-{unique}.json",
            std::process::id()
        ));
        std::fs::write(&path, json).unwrap();
        Self(path)
    }
    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}
impl Drop for ConfigurationFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn receipt(bot: &mut Bot) -> ServerPacket {
    bot.ping();
    bot.recv_snapshot(Instant::now() + Duration::from_secs(3))
        .expect("complete live UDP snapshot")
}

fn assert_receipt(packet: &ServerPacket, expected: &ResolvedMap) {
    assert_eq!(packet.geometry_id(), expected.geometry_id);
    assert_eq!(packet.map_profile(), expected.map_profile);
    assert_eq!(packet.structures().len(), expected.structures.len());
    let mut observed = std::collections::HashSet::new();
    for structure in &expected.structures {
        let actual = packet
            .structures()
            .iter()
            .find(|s| s.id == structure.id)
            .expect("stable configured structure identity");
        assert!(observed.insert(actual.id), "duplicate live structure id");
        assert_eq!(actual.map_key, structure.key);
        assert_eq!(actual.visual_profile, structure.visual_profile);
        assert_eq!(
            actual.team,
            Some(match structure.team {
                shared::map::Team::Green => Team::Green,
                shared::map::Team::Blue => Team::Blue,
            })
        );
        let is_tower = structure.lane.is_some();
        assert_eq!(actual.kind, if is_tower { "tower" } else { "base_tower" });
        assert_eq!(
            actual.lane,
            structure.lane.map(|lane| serde_json::to_value(lane)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned())
        );
        assert_eq!(actual.tier, structure.tier);
        assert!((actual.x - structure.position[0]).abs() < 0.0001);
        assert!((actual.z - structure.position[1]).abs() < 0.0001);
        assert_eq!(actual.y, if is_tower { 3.0 } else { 4.0 });
        assert_eq!(actual.hp, structure.stats.max_hp);
        assert_eq!(actual.max_hp, structure.stats.max_hp);
        // Fresh outer towers can be attacked, inner towers and bases are gated.
        assert_eq!(actual.protected, !is_tower || structure.tier > 0);
    }
}

#[test]
fn default_map_is_eight_exact_authoritative_objects_over_udp() {
    let config = ConfigurationFile::new(DEFAULT_JSON);
    let server = ServerProcess::spawn_with_env(&[
        ("OMOBA_MAP_CONFIG", config.path()),
        ("OMOBA_MATCH_MODE", "dev"),
    ]);
    let mut observer = Bot::connect_framed(server.addr());
    let packet = receipt(&mut observer);
    let expected = ResolvedMap::default();
    assert_eq!(expected.structures.len(), 8);
    assert_receipt(&packet, &expected);
    assert_eq!(
        packet
            .structures()
            .iter()
            .filter(|s| s.kind == "tower")
            .count(),
        6
    );
    assert!(
        packet
            .structures()
            .iter()
            .filter(|s| s.kind == "tower")
            .all(|s| s.max_hp == 240.0)
    );
}

#[test]
fn custom_tower_count_position_hp_tiers_and_identity_are_pinned_after_startup() {
    // Contributor example is exercised as the same JSON an actual host can use.
    let json = include_str!("../../examples/maps/two-tier.json");
    let resolved = MapDefinition::from_json(json)
        .unwrap()
        .resolve()
        .expect("valid contributor map");
    assert_eq!(resolved.structures.len(), 10);
    let config = ConfigurationFile::new(json);
    let server = ServerProcess::spawn_with_env(&[
        ("OMOBA_MAP_CONFIG", config.path()),
        ("OMOBA_MATCH_MODE", "dev"),
    ]);
    let mut observer = Bot::connect_framed(server.addr());
    assert_receipt(&receipt(&mut observer), &resolved);
    let outer = resolved.structures.iter().find(|s| s.id == 3).unwrap();
    let inner = resolved.structures.iter().find(|s| s.id == 9).unwrap();
    assert_eq!(outer.stats.max_hp, 300.0);
    assert_eq!(inner.stats.max_hp, 420.0);
    assert_eq!((outer.tier, inner.tier), (0, 1));
    assert_ne!(
        outer.position,
        ResolvedMap::default()
            .structures
            .iter()
            .find(|s| s.id == 3)
            .unwrap()
            .position
    );
    // Editing the file while the process is alive must not mutate this match.
    // This checks startup pinning, not the separate server round-reset unit test.
    std::fs::write(&config.0, DEFAULT_JSON).unwrap();
    let mut fresh_observer = Bot::connect_framed(server.addr());
    assert_receipt(&receipt(&mut fresh_observer), &resolved);
    assert_receipt(&receipt(&mut observer), &resolved);
}
