//! Outbound gameplay commands: the `NetworkCommand` message and its packet encoding.

use bevy::prelude::*;

use shared::HeroClass;
use shared::wire::ClientPacket;

use crate::persistence::ClientSessionId;
use crate::player::Player;
use crate::team::{CharacterChoice, Team};

use super::components::{GameStateSnapshot, PlayerBasicAttackCooldown, PlayerUtility};
use super::session::{ClientSession, CommittedJoin, send_join_attempt};
use super::transport::NetworkChannels;
use super::{GameState, TargetId};

#[derive(Message, Clone, Debug)]
pub enum NetworkCommand {
    Sandbox(shared::sandbox::SandboxRequest),
    /// Profile/history requests are valid before arena admission as well.
    Career(shared::career::CareerRequest),
    Social {
        request_id: u64,
        command: shared::social::SocialCommand,
    },
    /// Party presence, invites and launch; valid before any match.
    Party(shared::party::PartyCommand),
    Utility {
        action: shared::utility::UtilityAction,
        direction: Vec2,
    },
    BasicAttack {
        target: TargetId,
    },
    Cast {
        target: TargetId,
        /// Hotbar slot index (0=Q .. 3=R).
        slot: u8,
    },
    Prematch(shared::prematch::PrematchRequest),
    JoinPrematch {
        character: CharacterChoice,
        hero_class: HeroClass,
        avatar: Option<String>,
        sprite_character: Option<String>,
    },
    Join {
        team: Team,
        character: CharacterChoice,
        hero_class: HeroClass,
        /// Selected roster avatar slug (cosmetic), if any.
        avatar: Option<String>,
        /// Selected 2D sprite cosmetic. The renderer mode remains client-local.
        sprite_character: Option<String>,
    },
    RequestRematch,
    /// God mode, the speed boost or a practice sandbox request (bots,
    /// dummies, 1v1); sent as the command's existing packet.
    Debug(shared::debug::DebugCommand),
    UpgradeSkill {
        slot: u8,
    },
    BuyItem {
        server_epoch: u64,
        item_id: String,
        request_id: u64,
        match_id: u64,
    },
}

#[derive(Resource)]
pub(in crate::net) struct LocalStateSendTimer(pub(in crate::net) Timer);

pub(in crate::net) fn send_local_state(
    time: Res<Time>,
    mut timer: ResMut<LocalStateSendTimer>,
    channels: Option<Res<NetworkChannels>>,
    client_session: Res<ClientSession>,
    player_query: Query<(&Transform, Option<&PlayerUtility>), With<Player>>,
) {
    let Some(channels) = channels else {
        return;
    };

    if !client_session.join_confirmed() {
        return;
    }

    timer.0.tick(time.delta());
    if !timer.0.just_finished() {
        return;
    }

    let Ok((player_transform, utility)) = player_query.single() else {
        return;
    };

    let (yaw, _pitch, _roll) = player_transform.rotation.to_euler(EulerRot::YXZ);
    let packet = ClientPacket::Transform {
        dash_sequence: utility.map_or(0, |u| u.state.dash_sequence),
        x: player_transform.translation.x,
        y: player_transform.translation.y,
        z: player_transform.translation.z,
        yaw,
    };
    let _ = channels.outgoing.try_send(packet);
}

fn social_requires_signature(
    mode: &str,
    storage_enabled: bool,
    profile_or_nonce: bool,
    authenticated_identity: bool,
) -> bool {
    profile_or_nonce
        || authenticated_identity
        || (storage_enabled && !matches!(mode, "practice" | "dev"))
}

pub(in crate::net) fn send_network_commands(
    mut command_events: MessageReader<NetworkCommand>,
    channels: Option<Res<NetworkChannels>>,
    mut client_session: ResMut<ClientSession>,
    client_session_id: Res<ClientSessionId>,
    snapshot: Option<Res<GameStateSnapshot>>,
    basic_cooldown: Query<&PlayerBasicAttackCooldown, With<Player>>,
    utility: Query<&PlayerUtility, With<Player>>,
    mut utility_sequence: Local<u64>,
    mut basic_sequence: Local<u64>,
    mut career_identity: Option<ResMut<crate::career_identity::CareerIdentity>>,
    mut career_client: Option<ResMut<crate::career::CareerClient>>,
    mut social_client: Option<ResMut<crate::social::SocialClient>>,
    mut match_service: Option<ResMut<crate::match_service::MatchServiceClient>>,
) {
    let Some(channels) = channels else {
        return;
    };

    if let Ok(mut signer) = channels.gameplay_signer.lock() {
        *signer = career_identity.as_ref().and_then(|identity| {
            snapshot.as_ref().and_then(|snapshot| {
                identity.gameplay_signer(
                    &client_session.server_addr_display,
                    snapshot.meta.server_epoch,
                    snapshot.meta.match_id,
                    &client_session_id.0,
                )
            })
        });
    }
    for command in command_events.read() {
        if client_session.is_offline()
            && matches!(
                command,
                NetworkCommand::Career(_)
                    | NetworkCommand::Social { .. }
                    | NetworkCommand::Party(_)
                    | NetworkCommand::BuyItem { .. }
            )
        {
            continue;
        }
        if let (Some(service), Some(career)) = (match_service.as_mut(), career_client.as_ref())
            && service.intercept_join(&career.view, &client_session.server_addr_display, command)
        {
            continue;
        }
        match command {
            NetworkCommand::Social {
                request_id,
                command,
            } => {
                let error = if !client_session.join_confirmed() {
                    Some("Join a match before sending a message.".to_owned())
                } else if let Some(snapshot) = snapshot.as_ref() {
                    let request = shared::social::SocialRequest {
                        request_id: *request_id,
                        server_epoch: snapshot.meta.server_epoch,
                        match_id: snapshot.meta.match_id,
                        session_id: client_session_id.0.clone(),
                        command: command.clone(),
                    };
                    let identity_established = career_identity.as_ref().is_some_and(|identity| {
                        identity.authenticated_for_scope(
                            &client_session.server_addr_display,
                            snapshot.meta.server_epoch,
                            &client_session_id.0,
                        )
                    });
                    let (storage_enabled, profile_or_nonce) =
                        career_client.as_ref().map_or((false, false), |career| {
                            (
                                career.view.storage_enabled,
                                career.view.profile.is_some() || career.view.auth_nonce.is_some(),
                            )
                        });
                    let packet = if social_requires_signature(
                        &snapshot.match_mode,
                        storage_enabled,
                        profile_or_nonce,
                        identity_established,
                    ) {
                        career_identity
                            .as_mut()
                            .ok_or_else(|| "Your profile is not connected.".to_owned())
                            .and_then(|identity| {
                                identity.prepare_request(
                                    &shared::career::CareerRequest::Social { request },
                                    &client_session.server_addr_display,
                                    snapshot.meta.server_epoch,
                                    &client_session_id.0,
                                )
                            })
                            .map(|request| ClientPacket::Career { request })
                    } else {
                        Ok(ClientPacket::Social { request })
                    };
                    match packet {
                        Ok(packet) => channels
                            .outgoing
                            .try_send(packet)
                            .err()
                            .map(|_| "Connection lost.".to_owned()),
                        Err(error) => Some(error),
                    }
                } else {
                    Some("The match is not ready.".to_owned())
                };
                if let Some(error) = error
                    && let Some(social) = social_client.as_mut()
                {
                    social.request_failed(*request_id, error);
                }
            }
            NetworkCommand::Party(command) => {
                let _ = channels.outgoing.try_send(ClientPacket::Party {
                    command: command.clone(),
                });
            }
            NetworkCommand::Career(request) => {
                let Some(identity) = career_identity.as_mut() else {
                    continue;
                };
                let epoch = snapshot.as_ref().map_or(0, |s| s.meta.server_epoch);
                match identity.prepare_request(
                    request,
                    &client_session.server_addr_display,
                    epoch,
                    &client_session_id.0,
                ) {
                    Ok(signed) => {
                        let _ = channels
                            .outgoing
                            .try_send(ClientPacket::Career { request: signed });
                        if matches!(request, shared::career::CareerRequest::CancelQueue) {
                            client_session.last_join = None;
                            client_session.join_flow_committed = false;
                            client_session.clear_join_attempt();
                        }
                    }
                    Err(error) => {
                        if let Some(career) = career_client.as_mut() {
                            career.request_failed(error);
                        }
                    }
                }
            }
            NetworkCommand::Utility { action, direction } => {
                if !client_session.join_confirmed() {
                    continue;
                }
                let Some(meta) = snapshot
                    .as_ref()
                    .filter(|s| matches!(s.state, GameState::Running))
                    .map(|s| s.meta)
                    .filter(|m| m.server_epoch != 0 && m.match_id != 0)
                else {
                    continue;
                };
                let acknowledged = utility.single().map_or(0, |u| u.state.last_request_id);
                let Some(request_id) = (*utility_sequence).max(acknowledged).checked_add(1) else {
                    continue;
                };
                *utility_sequence = request_id;
                let _ = channels.outgoing.try_send(ClientPacket::Utility {
                    action: *action,
                    direction: direction.to_array(),
                    server_epoch: meta.server_epoch,
                    match_id: meta.match_id,
                    request_id,
                });
            }
            NetworkCommand::BasicAttack { target } => {
                if !client_session.join_confirmed() {
                    continue;
                }
                let Some(meta) = snapshot
                    .as_ref()
                    .map(|s| s.meta)
                    .filter(|meta| meta.server_epoch != 0 && meta.match_id != 0)
                else {
                    continue;
                };
                let acknowledged = basic_cooldown
                    .single()
                    .map(|c| c.last_request_id)
                    .unwrap_or(0);
                let Some(request_id) = (*basic_sequence).max(acknowledged).checked_add(1) else {
                    continue;
                };
                *basic_sequence = request_id;
                let _ = channels.outgoing.try_send(ClientPacket::BasicAttack {
                    target: *target,
                    server_epoch: meta.server_epoch,
                    match_id: meta.match_id,
                    request_id,
                });
            }
            NetworkCommand::Cast { target, slot } => {
                if !client_session.join_confirmed() {
                    continue;
                }
                let _ = channels.outgoing.try_send(ClientPacket::Cast {
                    target: *target,
                    slot: *slot,
                });
            }
            NetworkCommand::Prematch(request) => {
                if client_session.admitted {
                    let _ = channels.outgoing.try_send(ClientPacket::Prematch {
                        request: request.clone(),
                    });
                }
            }
            NetworkCommand::JoinPrematch {
                character,
                hero_class,
                avatar,
                sprite_character,
            } => {
                if client_session.admitted {
                    continue;
                }
                client_session.clear_join_attempt();
                client_session.join_flow_committed = true;
                client_session.last_join = Some(CommittedJoin {
                    prematch: true,
                    team: Team::Green,
                    character: *character,
                    hero_class: *hero_class,
                    avatar: avatar.clone(),
                    sprite_character: sprite_character.clone(),
                });
                send_join_attempt(&channels, &mut client_session, &client_session_id);
            }
            NetworkCommand::Join {
                team,
                character,
                hero_class,
                avatar,
                sprite_character,
            } => {
                if client_session.admitted {
                    continue;
                }
                client_session.clear_join_attempt();
                client_session.join_flow_committed = true;
                client_session.last_join = Some(CommittedJoin {
                    prematch: false,
                    team: *team,
                    character: *character,
                    hero_class: *hero_class,
                    avatar: avatar.clone(),
                    sprite_character: sprite_character.clone(),
                });
                send_join_attempt(&channels, &mut client_session, &client_session_id);
            }
            NetworkCommand::Sandbox(request) => {
                if client_session.join_confirmed() {
                    let _ = channels.outgoing.try_send(ClientPacket::Sandbox {
                        request: request.clone(),
                    });
                }
            }
            NetworkCommand::RequestRematch => {
                if !client_session.join_confirmed() {
                    continue;
                }
                let _ = channels.outgoing.try_send(ClientPacket::RequestRematch);
            }
            NetworkCommand::Debug(command) => {
                if !client_session.join_confirmed() {
                    continue;
                }
                let _ = channels.outgoing.try_send(command.to_packet());
            }
            NetworkCommand::BuyItem {
                server_epoch,
                item_id,
                request_id,
                match_id,
            } => {
                if client_session.join_confirmed() {
                    let _ = channels.outgoing.try_send(ClientPacket::BuyItem {
                        server_epoch: *server_epoch,
                        item_id: item_id.clone(),
                        request_id: *request_id,
                        match_id: *match_id,
                    });
                }
            }
            NetworkCommand::UpgradeSkill { slot } => {
                if !client_session.join_confirmed() {
                    continue;
                }
                let _ = channels
                    .outgoing
                    .try_send(ClientPacket::UpgradeSkill { slot: *slot });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::TargetKind;
    use serde_json::json;
    use shared::protocol::SnapshotMeta;

    #[test]
    fn basic_sender_uses_current_identity_and_advances_above_reconnect_acknowledgment() {
        let (outgoing, received) = crossbeam_channel::unbounded();
        let (_, incoming) = crossbeam_channel::unbounded();
        let (_, signals) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_message::<NetworkCommand>()
            .insert_resource(NetworkChannels {
                gameplay_signer: Default::default(),
                outgoing,
                incoming,
                signals,
            })
            .insert_resource(ClientSession::admitted_for_test())
            .init_resource::<ClientSessionId>()
            .insert_resource(GameStateSnapshot {
                meta: SnapshotMeta::new(17, 3, 9),
                ..default()
            })
            .add_systems(Update, send_network_commands);
        let player = app
            .world_mut()
            .spawn((
                Player,
                PlayerBasicAttackCooldown {
                    last_request_id: 5,
                    ..default()
                },
            ))
            .id();
        let target = TargetId {
            kind: TargetKind::Player,
            id: 42,
        };
        for request_id in [6, 7] {
            app.world_mut()
                .write_message(NetworkCommand::BasicAttack { target });
            app.update();
            let encoded = serde_json::to_value(received.try_recv().unwrap()).unwrap();
            assert_eq!(
                encoded,
                json!({"type":"basic_attack","target":{"kind":"player","id":42},
                "server_epoch":17,"match_id":3,"request_id":request_id})
            );
            let decoded: ClientPacket = serde_json::from_value(encoded).unwrap();
            assert!(matches!(decoded, ClientPacket::BasicAttack { .. }));
        }
        app.world_mut()
            .get_mut::<PlayerBasicAttackCooldown>(player)
            .unwrap()
            .last_request_id = 100;
        app.world_mut()
            .write_message(NetworkCommand::BasicAttack { target });
        app.update();
        assert!(matches!(
            received.try_recv().unwrap(),
            ClientPacket::BasicAttack {
                request_id: 101,
                ..
            }
        ));
        app.world_mut().resource_mut::<ClientSession>().admitted = false;
        app.world_mut()
            .write_message(NetworkCommand::BasicAttack { target });
        app.update();
        assert!(
            received.try_recv().is_err(),
            "pre-admission strike cannot leave the client"
        );
    }

    #[test]
    fn utility_sender_binds_match_and_continues_above_reconnect_acknowledgment() {
        let (outgoing, received) = crossbeam_channel::unbounded();
        let (_, incoming) = crossbeam_channel::unbounded();
        let (_, signals) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_message::<NetworkCommand>()
            .insert_resource(NetworkChannels {
                gameplay_signer: Default::default(),
                outgoing,
                incoming,
                signals,
            })
            .insert_resource(ClientSession::admitted_for_test())
            .init_resource::<ClientSessionId>()
            .insert_resource(GameStateSnapshot {
                meta: SnapshotMeta::new(17, 3, 9),
                state: GameState::Running,
                ..default()
            })
            .add_systems(Update, send_network_commands);
        app.world_mut().spawn((
            Player,
            PlayerUtility {
                state: shared::utility::UtilityState {
                    last_request_id: 8,
                    ..default()
                },
            },
        ));
        app.world_mut().write_message(NetworkCommand::Utility {
            action: shared::utility::UtilityAction::Dash,
            direction: Vec2::X,
        });
        app.update();
        let encoded = serde_json::to_value(received.try_recv().unwrap()).unwrap();
        assert_eq!(
            encoded,
            json!({"type":"utility","action":"dash","direction":[1.0,0.0],"server_epoch":17,"match_id":3,"request_id":9})
        );
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Lobby;
        app.world_mut().write_message(NetworkCommand::Utility {
            action: shared::utility::UtilityAction::Haste,
            direction: Vec2::ZERO,
        });
        app.update();
        assert!(received.try_recv().is_err());
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Running;
        app.world_mut().resource_mut::<ClientSession>().admitted = false;
        app.world_mut().write_message(NetworkCommand::Utility {
            action: shared::utility::UtilityAction::Haste,
            direction: Vec2::ZERO,
        });
        app.update();
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn social_guest_routing_preserves_practice_during_profile_outage_and_fails_closed_for_accounts()
    {
        use super::social_requires_signature as signed;
        assert!(!signed("practice", true, false, false));
        assert!(!signed("dev", true, false, false));
        assert!(signed("practice", true, true, false));
        assert!(signed("practice", false, false, true));
        assert!(signed("release", true, false, false));
        assert!(!signed("release", false, false, false));
        assert!(signed("", true, false, false));
    }
}
