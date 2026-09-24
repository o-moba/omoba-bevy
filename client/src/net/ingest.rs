//! Drains the incoming packet channel and stages the newest admissible snapshot for this frame.

use bevy::prelude::*;
use crossbeam_channel::TryRecvError;
use std::time::Instant;

use shared::combat::CombatEvent;
use shared::protocol::{JoinRejection, PROTOCOL_VERSION, SnapshotMeta};
use shared::wire::{
    MinionState, NeutralState, PlayerState, ProjectileState, ServerPacket, StructureState,
};

use crate::team::{Team, TeamSelection};

use super::session::{ClientSession, NetIncomingDisconnected, ReconnectState};
use super::transport::NetworkChannels;
use super::{GameState, TeamBuffState};

/// Latest drained snapshot for this frame (filled by [`ingest_server_snapshot_packets`]).
#[derive(Resource, Default)]
pub(in crate::net) struct PendingServerSnapshotFrame {
    pub(in crate::net) frame: Option<PendingSnapshotData>,
}

pub(in crate::net) struct PendingSnapshotData {
    pub(in crate::net) vision: Option<shared::vision::TeamVision>,
    pub(in crate::net) forest_pickups: Vec<shared::forest_pickups::ForestPickupState>,
    pub(in crate::net) sandbox: Option<shared::sandbox::SandboxSnapshot>,
    pub(in crate::net) prematch: Option<shared::prematch::PrematchSnapshot>,
    pub(in crate::net) match_mode: String,
    pub(in crate::net) geometry_id: String,
    pub(in crate::net) map_profile: String,
    pub(in crate::net) meta: SnapshotMeta,
    pub(in crate::net) wall_time: Instant,
    pub(in crate::net) your_id: u64,
    pub(in crate::net) players: Vec<PlayerState>,
    pub(in crate::net) scoreboard: Option<shared::live_score::LiveScoreboard>,
    pub(in crate::net) projectiles: Vec<ProjectileState>,
    pub(in crate::net) structures: Vec<StructureState>,
    pub(in crate::net) minions: Vec<MinionState>,
    pub(in crate::net) neutrals: Vec<NeutralState>,
    pub(in crate::net) team_buffs: Vec<TeamBuffState>,
    pub(in crate::net) combat_events: Vec<CombatEvent>,
    pub(in crate::net) game_state: GameState,
    pub(in crate::net) rematch_in_secs: Option<u64>,
    /// Local team choice at ingest time (spawn gate when server has not yet mirrored selection).
    pub(in crate::net) selected_team_for_spawn: Option<Team>,
}

pub(in crate::net) fn ingest_server_snapshot_packets(
    channels: Option<Res<NetworkChannels>>,
    mut client_session: ResMut<ClientSession>,
    mut pending: ResMut<PendingServerSnapshotFrame>,
    mut incoming_dead: ResMut<NetIncomingDisconnected>,
    team_selection: Res<TeamSelection>,
    mut career_client: Option<ResMut<crate::career::CareerClient>>,
    mut career_identity: Option<ResMut<crate::career_identity::CareerIdentity>>,
    mut social_client: Option<ResMut<crate::social::SocialClient>>,
) {
    pending.frame = None;
    let Some(channels) = channels.as_ref() else {
        return;
    };

    if client_session.discard_incoming_snapshots {
        while let Ok(_ignored) = channels.incoming.try_recv() {}
        return;
    }

    let mut latest_snapshot: Option<PendingSnapshotData> = None;

    loop {
        match channels.incoming.try_recv() {
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                incoming_dead.0 = true;
                break;
            }
            Ok(packet) => match packet {
                ServerPacket::Social {
                    server_epoch,
                    match_id,
                    sequence,
                    social,
                } => {
                    if let Some(client) = social_client.as_mut() {
                        client.apply_view(server_epoch, match_id, sequence, social);
                    }
                }
                ServerPacket::Career {
                    server_epoch,
                    sequence,
                    career,
                } => {
                    if server_epoch == 0
                        || server_epoch != client_session.career_server_epoch
                        || sequence <= client_session.career_packet_sequence
                    {
                        continue;
                    }
                    client_session.career_packet_sequence = sequence;
                    if let Some(identity) = career_identity.as_mut() {
                        identity.observe_server_auth(
                            &career,
                            &client_session.server_addr_display,
                            server_epoch,
                        );
                    }
                    if matches!(
                        career.queue,
                        shared::career::QueueView::Waiting { .. }
                            | shared::career::QueueView::Selected
                    ) || career.auth_nonce.is_some() && !client_session.admitted
                    {
                        client_session.join_attempts = 0;
                        client_session.join_exhausted = false;
                        client_session.join_error = None;
                    }
                    if let Some(client) = career_client.as_mut() {
                        client.apply_view(career);
                    }
                }
                ServerPacket::Snapshot {
                    sandbox,
                    forest_pickups,
                    vision,
                    geometry_id,
                    map_profile,
                    match_mode,
                    meta,
                    join_error,
                    your_id,
                    players,
                    scoreboard,
                    prematch,
                    projectiles,
                    structures,
                    minions,
                    neutrals,
                    team_buffs,
                    combat_events,
                    game_state,
                    rematch_in_secs,
                } => {
                    if meta.protocol_version != PROTOCOL_VERSION {
                        client_session.join_error = Some(JoinRejection::ProtocolMismatch);
                        continue;
                    }
                    if !geometry_id.is_empty() && geometry_id != shared::map::GEOMETRY_ID {
                        client_session.join_error = Some(JoinRejection::MapGeometryMismatch);
                        client_session.admitted = false;
                        latest_snapshot = None;
                        continue;
                    }
                    if !client_session.snapshot_order.accept(meta) {
                        continue;
                    }
                    if let Some(social) = social_client.as_mut() {
                        social.bind(meta.server_epoch, meta.match_id);
                    }
                    if client_session.career_server_epoch != meta.server_epoch {
                        client_session.career_server_epoch = meta.server_epoch;
                        client_session.career_packet_sequence = 0;
                    }
                    if let Some(career_client) = career_client.as_mut() {
                        career_client.local_player_id = Some(your_id);
                    }
                    client_session.join_error = join_error;
                    client_session.admitted =
                        join_error.is_none() && players.iter().any(|player| player.id == your_id);
                    if client_session.admitted {
                        client_session.reconnect = ReconnectState::default();
                        client_session.join_exhausted = false;
                    }
                    latest_snapshot = Some(PendingSnapshotData {
                        sandbox,
                        forest_pickups,
                        vision,
                        match_mode,
                        geometry_id,
                        map_profile,
                        meta,
                        wall_time: Instant::now(),
                        your_id,
                        players,
                        scoreboard,
                        prematch,
                        projectiles,
                        structures,
                        minions,
                        neutrals,
                        team_buffs,
                        combat_events,
                        game_state,
                        rematch_in_secs,
                        selected_team_for_spawn: team_selection.team,
                    });
                }
            },
        }
    }

    pending.frame = latest_snapshot;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::session::{ClientConnectionState, MAX_JOIN_ATTEMPTS};
    use crate::net::test_fixtures::*;
    use crate::net::transport::forward_complete_server_datagram;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn admission_rejection_exhaustion_and_snapshot_order_are_actionable() {
        use shared::protocol::JoinRejection;
        let (mut app, incoming, outgoing) = admission_app();
        for (tick, error) in [
            (1, JoinRejection::MatchFull),
            (2, JoinRejection::SessionActive),
            (3, JoinRejection::ProtocolMismatch),
        ] {
            incoming
                .send(admission_snapshot(1, tick, false, Some(error)))
                .unwrap();
            app.update();
            let session = app.world().resource::<ClientSession>();
            assert_eq!(session.join_error, Some(error));
            assert!(!session.join_confirmed());
            assert!(!error.message().is_empty());
            assert!(outgoing.try_recv().is_err());
        }
        incoming.send(admission_snapshot(2, 0, true, None)).unwrap();
        incoming
            .send(admission_snapshot(
                1,
                999,
                false,
                Some(JoinRejection::MatchFull),
            ))
            .unwrap();
        incoming
            .send(admission_snapshot(
                2,
                0,
                false,
                Some(JoinRejection::MatchFull),
            ))
            .unwrap();
        app.update();
        assert!(
            app.world().resource::<ClientSession>().join_confirmed(),
            "old round/duplicate cannot undo admission"
        );
        assert!(
            app.world()
                .resource::<PendingServerSnapshotFrame>()
                .frame
                .is_some()
        );
        incoming
            .send(admission_snapshot(1, 1000, false, None))
            .unwrap();
        app.update();
        assert!(
            app.world()
                .resource::<PendingServerSnapshotFrame>()
                .frame
                .is_none()
        );
        {
            let mut session = app.world_mut().resource_mut::<ClientSession>();
            session.clear_join_attempt();
            session.join_attempts = MAX_JOIN_ATTEMPTS;
        }
        app.update();
        assert!(app.world().resource::<ClientSession>().join_exhausted);
        assert!(outgoing.try_recv().is_err());
    }

    #[test]
    fn incompatible_map_geometry_never_applies_and_legacy_default_still_works() {
        let (mut app, incoming, _) = admission_app();
        let mut incompatible = serde_json::to_value(admission_snapshot(1, 1, true, None)).unwrap();
        incompatible["geometry_id"] = json!("unsupported-terrain-v9");
        incoming
            .send(serde_json::from_value(incompatible).unwrap())
            .unwrap();
        app.update();
        assert_eq!(
            app.world().resource::<ClientSession>().join_error,
            Some(shared::protocol::JoinRejection::MapGeometryMismatch)
        );
        assert!(!app.world().resource::<ClientSession>().admitted);
        assert!(
            app.world()
                .resource::<PendingServerSnapshotFrame>()
                .frame
                .is_none()
        );
        incoming.send(admission_snapshot(1, 2, true, None)).unwrap();
        app.update();
        assert!(app.world().resource::<ClientSession>().admitted);
        assert!(
            app.world()
                .resource::<PendingServerSnapshotFrame>()
                .frame
                .is_some()
        );
    }

    #[test]
    fn production_datagram_forwarding_stages_complete_large_snapshots_only() {
        let (outgoing_tx, _outgoing_rx) = crossbeam_channel::unbounded();
        let (incoming_tx, incoming_rx) = crossbeam_channel::unbounded();
        let (_signal_tx, signal_rx) = crossbeam_channel::unbounded();
        let baseline = Instant::now() - Duration::from_secs(1);
        let mut app = App::new();
        app.insert_resource(NetworkChannels {
            gameplay_signer: Default::default(),
            outgoing: outgoing_tx,
            incoming: incoming_rx,
            signals: signal_rx,
        })
        .insert_resource(ClientSession {
            state: ClientConnectionState::Connected,
            last_qualifying_snapshot_wall: Some(baseline),
            ..default()
        })
        .init_resource::<PendingServerSnapshotFrame>()
        .init_resource::<NetIncomingDisconnected>()
        .init_resource::<TeamSelection>()
        .add_systems(Update, ingest_server_snapshot_packets);

        let populated = populated_snapshot_fixture();
        assert!(populated.len() > 8 * 1024);
        forward_complete_server_datagram(&populated, &incoming_tx)
            .expect("complete populated datagram enters the production channel");
        app.update();
        {
            let pending = app.world().resource::<PendingServerSnapshotFrame>();
            let staged = pending.frame.as_ref().expect("large snapshot is staged");
            assert_eq!(
                staged.structures.last().map(|structure| structure.id),
                Some(8)
            );
            assert_eq!(staged.minions.last().map(|minion| minion.id), Some(18));
            assert_eq!(staged.rematch_in_secs, Some(4242));
        }
        assert_eq!(
            app.world()
                .resource::<ClientSession>()
                .last_qualifying_snapshot_wall,
            Some(baseline),
            "staging alone must not advance the last-applied snapshot timestamp"
        );

        let malformed = br#"{"type":"snapshot","your_id":7,"players":["#;
        assert!(forward_complete_server_datagram(malformed, &incoming_tx).is_err());
        app.update();
        assert!(
            app.world()
                .resource::<PendingServerSnapshotFrame>()
                .frame
                .is_none(),
            "malformed JSON must not publish a partial pending snapshot"
        );
        assert_eq!(
            app.world()
                .resource::<ClientSession>()
                .last_qualifying_snapshot_wall,
            Some(baseline),
            "malformed JSON must not advance the qualifying timestamp"
        );

        let recovery = exact_size_snapshot_fixture(9_000, 9_000);
        forward_complete_server_datagram(&recovery, &incoming_tx)
            .expect("a complete datagram after malformed traffic is accepted");
        app.update();
        let pending = app.world().resource::<PendingServerSnapshotFrame>();
        let staged = pending.frame.as_ref().expect("recovery snapshot is staged");
        assert_eq!(
            staged.structures.last().map(|structure| structure.id),
            Some(808)
        );
        assert_eq!(staged.minions.last().map(|minion| minion.id), Some(909));
        assert_eq!(staged.rematch_in_secs, Some(9_000));
    }
}
