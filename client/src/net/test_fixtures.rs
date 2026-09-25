//! Shared test apps and wire fixtures for the `net` submodule tests.

use bevy::prelude::*;
use serde_json::json;

use shared::wire::{ClientPacket, ServerPacket};

use crate::team::TeamSelection;

use super::apply::{SnapshotApplied, StagedSnapshot, snapshot_apply_systems};
use super::components::{GameStateSnapshot, NetworkPlayerId, NetworkState};
use super::ingest::{PendingServerSnapshotFrame, ingest_server_snapshot_packets};
use super::session::{
    ClientConnectionState, ClientSession, CommittedJoin, NetIncomingDisconnected, SessionEvent,
    TeardownQueries, TeardownReason, perform_network_teardown, retry_pending_join,
};
use super::transport::{NetworkChannels, decode_server_packet};
use super::{ClientNetPipeline, configure_network_pipeline};

pub(in crate::net) fn admission_app() -> (
    App,
    crossbeam_channel::Sender<ServerPacket>,
    crossbeam_channel::Receiver<ClientPacket>,
) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let (incoming, incoming_rx) = crossbeam_channel::unbounded();
    let (_, signals) = crossbeam_channel::unbounded();
    let mut app = App::new();
    app.insert_resource(NetworkChannels {
        gameplay_signer: Default::default(),
        outgoing: tx,
        incoming: incoming_rx,
        signals,
    })
    .insert_resource(ClientSession {
        state: ClientConnectionState::Connected,
        last_join: Some(CommittedJoin {
            prematch: false,
            team: crate::team::Team::Green,
            character: crate::team::CharacterChoice::Ipfs,
            hero_class: shared::HeroClass::Warrior,
            avatar: None,
            sprite_character: None,
        }),
        ..default()
    })
    .init_resource::<crate::persistence::ClientSessionId>()
    .init_resource::<PendingServerSnapshotFrame>()
    .init_resource::<NetIncomingDisconnected>()
    .init_resource::<TeamSelection>()
    .add_systems(
        Update,
        (ingest_server_snapshot_packets, retry_pending_join).chain(),
    );
    (app, incoming, rx)
}

pub(in crate::net) fn admission_snapshot(
    round: u64,
    tick: u64,
    admitted: bool,
    error: Option<shared::protocol::JoinRejection>,
) -> ServerPacket {
    let mut value: serde_json::Value =
        serde_json::from_slice(&populated_snapshot_fixture()).unwrap();
    value["your_id"] = json!(1);
    value["match_id"] = json!(round);
    value["snapshot_tick"] = json!(tick);
    value["join_error"] = json!(error);
    if !admitted {
        value["players"] = json!([]);
    }
    serde_json::from_value(value).unwrap()
}

pub(in crate::net) fn snapshot_app() -> (App, crossbeam_channel::Sender<ServerPacket>) {
    use crate::{
        camera::CameraState,
        maps::MapLayout,
        sprite::PlayerVisualMode,
        world::{AvatarAssetCache, PlayerAssets, PlayerModelCatalog},
    };
    let (outgoing, _outgoing_rx) = crossbeam_channel::unbounded();
    let (incoming, incoming_rx) = crossbeam_channel::unbounded();
    let (_signal_tx, signals) = crossbeam_channel::unbounded();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .insert_resource(NetworkChannels {
            gameplay_signer: Default::default(),
            outgoing,
            incoming: incoming_rx,
            signals,
        })
        .insert_resource(ClientSession {
            state: ClientConnectionState::Connected,
            ..default()
        })
        .insert_resource(TeamSelection {
            team: Some(crate::team::Team::Green),
            ..default()
        })
        .insert_resource(PlayerVisualMode::Models3d)
        .insert_resource(PlayerAssets {
            scene: None,
            gltf: None,
            mesh: default(),
            material: default(),
        })
        .init_resource::<PlayerModelCatalog>()
        .init_resource::<AvatarAssetCache>()
        .init_resource::<CameraState>()
        .init_resource::<MapLayout>()
        .init_resource::<NetworkState>()
        .init_resource::<crate::debug::DebugToggles>()
        .init_resource::<GameStateSnapshot>()
        .init_resource::<PendingServerSnapshotFrame>()
        .init_resource::<StagedSnapshot>()
        .init_resource::<NetIncomingDisconnected>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_message::<crate::game_vfx::UtilityVfx>()
        .add_message::<SessionEvent>()
        .add_message::<SnapshotApplied>()
        .add_systems(
            Update,
            (
                ingest_server_snapshot_packets.in_set(ClientNetPipeline::IngestSnapshot),
                snapshot_apply_systems(),
            ),
        );
    configure_network_pipeline(&mut app);
    crate::world::register_local_player_spawn(&mut app);
    (app, incoming)
}

/// Session events flushed since the last call (the flush at the end of
/// `ApplySnapshot` runs every frame, staged snapshot or not).
pub(in crate::net) fn drain_session_events(app: &mut App) -> Vec<SessionEvent> {
    app.world_mut()
        .resource_mut::<Messages<SessionEvent>>()
        .drain()
        .collect()
}

pub(in crate::net) fn drain_snapshot_applied(app: &mut App) -> Vec<SnapshotApplied> {
    app.world_mut()
        .resource_mut::<Messages<SnapshotApplied>>()
        .drain()
        .collect()
}

/// Runs the production teardown outside the lifecycle system, as a transport
/// failure would.
pub(in crate::net) fn tear_down(app: &mut App, reason: TeardownReason) {
    use bevy::ecs::system::RunSystemOnce;
    app.world_mut()
        .run_system_once(
            move |mut commands: Commands,
                  mut session: ResMut<ClientSession>,
                  mut network: ResMut<NetworkState>,
                  mut snapshot: ResMut<GameStateSnapshot>,
                  mut team: ResMut<TeamSelection>,
                  mut camera: ResMut<crate::camera::CameraState>,
                  queries: TeardownQueries| {
                perform_network_teardown(
                    reason,
                    &mut commands,
                    &mut session,
                    &mut network,
                    &mut snapshot,
                    &mut team,
                    &mut camera,
                    &queries.remote_query,
                    &queries.projectile_query,
                    &queries.structure_query,
                    &queries.minion_query,
                    &queries.neutral_query,
                    &queries.player_query,
                );
            },
        )
        .unwrap();
}

/// Recipient-shaped packets exercise the ordinary ingest/apply pipeline;
/// omission is the server visibility contract, not a client hiding flag.
pub(in crate::net) fn team_vision_snapshot(
    tick: u64,
    enemy_visible: bool,
    local_dead: bool,
) -> ServerPacket {
    let mut value = serde_json::to_value(admission_snapshot(1, tick, true, None)).unwrap();
    let mut heroes = value["players"].as_array().unwrap()[..3].to_vec();
    for hero in &mut heroes {
        hero["avatar"] = serde_json::Value::Null;
    }
    if local_dead {
        heroes[0]["hp"] = json!(0.0);
    }
    heroes.retain(|hero| hero["id"] != 2 || enemy_visible);
    value["players"] = json!(heroes);
    for field in ["structures", "minions", "neutrals", "projectiles"] {
        value[field] = json!([]);
    }
    value["vision"] = json!({
        "sources": [{"position": [tick as f32, -6.0], "radius": 32.0}],
        "local_brush": if tick.is_multiple_of(2) { Some(1) } else { None },
        "local_hidden": tick.is_multiple_of(2)
    });
    serde_json::from_value(value).unwrap()
}

pub(in crate::net) fn network_hero_entity(app: &mut App, id: u64) -> Entity {
    app.world_mut()
        .query::<(Entity, &NetworkPlayerId)>()
        .iter(app.world())
        .find_map(|(entity, player)| (player.0 == id).then_some(entity))
        .unwrap()
}

pub(in crate::net) fn exact_size_snapshot_fixture(size: usize, sentinel: u64) -> Vec<u8> {
    let prefix = String::from(
        r#"{"type":"snapshot","protocol_version":2,"server_epoch":1,"match_id":1,"snapshot_tick":2,"your_id":7,"padding":""#,
    );
    let suffix = format!(
        r#"","players":[],"structures":[{{"id":808,"kind":"tower","team":"blue","x":1.0,"y":2.0,"z":3.0,"hp":4.0,"max_hp":5.0}}],"minions":[{{"id":909,"team":"green","lane":"bot","x":6.0,"y":0.5,"z":7.0,"yaw":0.0,"hp":8.0,"max_hp":9.0,"state":"marching","target_kind":null,"target_id":null}}],"rematch_in_secs":{sentinel}}}"#
    );
    let fixed_len = prefix.len() + suffix.len();
    assert!(
        size >= fixed_len,
        "fixture size {size} is smaller than fixed JSON length {fixed_len}"
    );
    let mut payload = Vec::with_capacity(size);
    payload.extend_from_slice(prefix.as_bytes());
    payload.resize(size - suffix.len(), b'x');
    payload.extend_from_slice(suffix.as_bytes());
    assert_eq!(payload.len(), size);
    payload
}

pub(in crate::net) fn assert_fixture_sentinel(payload: &[u8], expected_sentinel: u64) {
    let packet = decode_server_packet(payload).expect("complete fixture should decode");
    let ServerPacket::Snapshot {
        structures,
        minions,
        rematch_in_secs,
        ..
    } = packet
    else {
        panic!("expected snapshot");
    };
    assert_eq!(structures.last().map(|structure| structure.id), Some(808));
    assert_eq!(minions.last().map(|minion| minion.id), Some(909));
    assert_eq!(rematch_in_secs, Some(expected_sentinel));
}

pub(in crate::net) fn populated_snapshot_fixture() -> Vec<u8> {
    let players = (1_u64..=10)
        .map(|id| {
            json!({
                "id": id,
                "x": id as f32 * 1.125,
                "y": 0.5,
                "z": id as f32 * -2.25,
                "yaw": 1.75,
                "team": if id % 2 == 0 { "blue" } else { "green" },
                "hp": 100.0,
                "max_hp": 100.0,
                "mana": 87.25,
                "max_mana": 100.0,
                "gold": 123,
                "xp": 45,
                "level": 2,
                "next_level_xp": 180,
                "skill_points": 1,
                "ranks": [1, 1, 1, 1],
                "character": "ipfs",
                "hero_class": "warrior",
                "avatar": "osa-kardialtheconsumer-00bea9121db1",
                "sprite_character": "cathedral-moth-bellringer",
                "action_sequence": id,
                "action_kind": "attack",
                "action_slot": 0
            })
        })
        .collect::<Vec<_>>();
    let structures = (1_u64..=8)
        .map(|id| {
            json!({
                "id": id,
                "kind": if id > 6 { "base_tower" } else { "tower" },
                "team": if id % 2 == 0 { "blue" } else { "green" },
                "x": id as f32 * 11.25,
                "y": 3.0,
                "z": id as f32 * -9.75,
                "hp": 240.0,
                "max_hp": 240.0
            })
        })
        .collect::<Vec<_>>();
    let minions = (1_u64..=18)
        .map(|id| {
            let lane = ["top", "mid", "bot"][(id as usize - 1) % 3];
            json!({
                "id": id,
                "team": if id % 2 == 0 { "blue" } else { "green" },
                "lane": lane,
                "x": id as f32 * 3.125,
                "y": 0.5,
                "z": id as f32 * -4.25,
                "yaw": 2.75,
                "hp": 65.0,
                "max_hp": 65.0,
                "state": "chasing",
                "target_kind": "minion",
                "target_id": id + 100
            })
        })
        .collect::<Vec<_>>();
    let projectiles = (1_u64..=4)
        .map(|id| {
            json!({
                "id": id,
                "owner_id": id,
                "owner_team": if id % 2 == 0 { "blue" } else { "green" },
                "x": id as f32 * 7.125,
                "y": 1.35,
                "z": id as f32 * -8.75
            })
        })
        .collect::<Vec<_>>();
    let neutrals = (1_u64..=5)
        .map(|id| {
            let camp_type = [
                "skirmisher",
                "bruiser",
                "spitter",
                "wendigo_boss",
                "king_mutatio_boss",
            ][id as usize - 1];
            json!({
                "id": 9_000 + id,
                "camp_type": camp_type,
                "x": id as f32 * 13.25,
                "y": 0.7,
                "z": id as f32 * -14.5,
                "yaw": 0.25,
                "hp": 900.0,
                "max_hp": 1500.0,
                "ai_state": "idle"
            })
        })
        .collect::<Vec<_>>();

    serde_json::to_vec(&json!({
        "type": "snapshot",
        "protocol_version": shared::protocol::PROTOCOL_VERSION, "server_epoch": 1, "match_id": 1, "snapshot_tick": 1,
        "your_id": 10,
        "players": players,
        "projectiles": projectiles,
        "structures": structures,
        "minions": minions,
        "neutrals": neutrals,
        "team_buffs": [],
        "game_state": { "type": "running" },
        "rematch_in_secs": 4242
    }))
    .expect("populated fixture should serialize")
}
