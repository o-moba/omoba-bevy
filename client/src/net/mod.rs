mod apply;
mod commands;
mod components;
mod ingest;
mod interpolate;
mod offline;
mod public_transport;
mod session;
mod status_ui;
#[cfg(test)]
mod test_fixtures;
mod transport;

pub use shared::map::Lane;
pub use shared::wire::{
    GameState, MinionBrainState, NeutralAiState, NeutralCampType, TargetId, TargetKind,
    TeamBuffKind, TeamBuffState,
};

use bevy::prelude::*;

pub use apply::SnapshotApplied;
pub use commands::NetworkCommand;
pub use components::*;
pub(crate) use interpolate::NetworkGroundingSet;
pub use session::{
    ClientConnectionState, ClientSession, NetIncomingDisconnected, SessionEvent, SessionUiCommand,
};

use apply::{
    StagedSnapshot, mirror_debug_flags_to_network_state, respawn_players_with_new_store_models,
    respawn_sandbox_models, snapshot_apply_systems,
};
use commands::{LocalStateSendTimer, send_local_state, send_network_commands};
use ingest::{PendingServerSnapshotFrame, ingest_server_snapshot_packets};
use interpolate::{
    age_utility_timers, ground_networked_entities, interpolate_remote_players,
    interpolate_snapshot_entities,
};
use session::{
    flush_session_events, retry_pending_join, start_networking, update_session_lifecycle,
};
use status_ui::{
    handle_connection_retry_button, setup_connection_status_ui, sync_connection_status_ui,
};

pub(in crate::net) const UPDATE_INTERVAL_SECONDS: f32 = 0.05;

pub struct NetworkingPlugin;

/// Strict main-thread ordering for networking / snapshot / UI sync (Bevy 0.18: avoid `.after(fn)`).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ClientNetPipeline {
    SendLocalState,
    SendCommands,
    IngestSnapshot,
    ApplySnapshot,
    AgeUtilityTimers,
    InterpolateNetEntities,
    InterpolateRemotePlayers,
    SessionRetryInput,
    SessionLifecycle,
    SyncConnectionUi,
}

/// Stages of snapshot application, chained inside
/// [`ClientNetPipeline::ApplySnapshot`] (see `apply::snapshot_apply_systems`).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SnapshotApply {
    /// Moves the frame ingest staged into `StagedSnapshot`.
    Begin,
    /// Connection state, the `Connected` and `Joined` edges.
    Session,
    /// `GameStateSnapshot`, the round (`RoundChanged`), the prematch loadout
    /// and the Draft gate. Runs even when the entity work is skipped.
    Resources,
    /// Local hero, remote players, projectiles, structures, minions, neutrals.
    Entities,
    /// Writes `SnapshotApplied` and clears the staged frame.
    Finish,
}

/// Systems that react to [`SessionEvent`]s in the frame they are written:
/// after the session lifecycle (and its event flush), before the next frame's
/// ingest can apply a view the reaction would clear.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SessionReactions;

/// Shared production scheduling contract, also used by the isolated ECS regression.
pub(crate) fn configure_network_pipeline(app: &mut App) {
    app.configure_sets(
        Update,
        (
            // Resolve current authoritative state before reading modal/gameplay input.
            // Commands created by those inputs leave in the same frame.
            crate::input_context::InputContextSet::Modal
                .after(ClientNetPipeline::InterpolateRemotePlayers),
            ClientNetPipeline::SendLocalState.after(crate::input_context::InputContextSet::Actions),
            ClientNetPipeline::SendCommands.after(ClientNetPipeline::SendLocalState),
            ClientNetPipeline::ApplySnapshot.after(ClientNetPipeline::IngestSnapshot),
            ClientNetPipeline::AgeUtilityTimers.after(ClientNetPipeline::ApplySnapshot),
            ClientNetPipeline::InterpolateNetEntities.after(ClientNetPipeline::AgeUtilityTimers),
            ClientNetPipeline::InterpolateRemotePlayers
                .after(ClientNetPipeline::InterpolateNetEntities),
            ClientNetPipeline::SessionRetryInput.after(ClientNetPipeline::SendCommands),
            ClientNetPipeline::SessionLifecycle.after(ClientNetPipeline::SessionRetryInput),
            ClientNetPipeline::SyncConnectionUi.after(ClientNetPipeline::SessionLifecycle),
            SessionReactions.after(ClientNetPipeline::SessionLifecycle),
        ),
    );
    app.configure_sets(
        Update,
        (
            SnapshotApply::Begin,
            SnapshotApply::Session,
            SnapshotApply::Resources,
            SnapshotApply::Entities,
            SnapshotApply::Finish,
        )
            .chain()
            .in_set(ClientNetPipeline::ApplySnapshot),
    );
}

impl Plugin for NetworkingPlugin {
    fn build(&self, app: &mut App) {
        configure_network_pipeline(app);
        app.add_message::<NetworkCommand>()
            .add_message::<SessionUiCommand>()
            .add_message::<SessionEvent>()
            .add_message::<SnapshotApplied>()
            // Snapshot application announces accepted dashes to the VFX layer.
            .add_message::<crate::game_vfx::UtilityVfx>()
            .init_resource::<NetworkState>()
            .init_resource::<GameStateSnapshot>()
            .init_resource::<PendingServerSnapshotFrame>()
            .init_resource::<StagedSnapshot>()
            .init_resource::<ClientSession>()
            .init_resource::<NetIncomingDisconnected>()
            .insert_resource(LocalStateSendTimer(Timer::from_seconds(
                UPDATE_INTERVAL_SECONDS,
                TimerMode::Repeating,
            )))
            .add_systems(
                Startup,
                (
                    start_networking.after(crate::persistence::load_persistent_client_settings),
                    setup_connection_status_ui,
                    offline::setup_banner,
                ),
            )
            .add_systems(
                Update,
                send_local_state.in_set(ClientNetPipeline::SendLocalState),
            )
            .add_systems(
                Update,
                send_network_commands.in_set(ClientNetPipeline::SendCommands),
            )
            .add_systems(
                Update,
                (mirror_debug_flags_to_network_state, offline::sync_banner),
            )
            .add_systems(
                Update,
                (
                    respawn_players_with_new_store_models,
                    respawn_sandbox_models,
                )
                    .before(ClientNetPipeline::ApplySnapshot),
            )
            .add_systems(
                Update,
                (offline::step, ingest_server_snapshot_packets)
                    .chain()
                    .in_set(ClientNetPipeline::IngestSnapshot),
            )
            .add_systems(Update, snapshot_apply_systems())
            .add_systems(
                Update,
                age_utility_timers.in_set(ClientNetPipeline::AgeUtilityTimers),
            )
            .add_systems(
                Update,
                interpolate_snapshot_entities.in_set(ClientNetPipeline::InterpolateNetEntities),
            )
            .add_systems(
                Update,
                interpolate_remote_players.in_set(ClientNetPipeline::InterpolateRemotePlayers),
            )
            .add_systems(
                Update,
                handle_connection_retry_button.in_set(ClientNetPipeline::SessionRetryInput),
            )
            .add_systems(
                Update,
                (
                    update_session_lifecycle,
                    retry_pending_join,
                    flush_session_events,
                )
                    .chain()
                    .in_set(ClientNetPipeline::SessionLifecycle),
            )
            .add_systems(
                Update,
                sync_connection_status_ui.in_set(ClientNetPipeline::SyncConnectionUi),
            )
            .add_systems(
                PostUpdate,
                ground_networked_entities
                    .in_set(NetworkGroundingSet)
                    .before(bevy::transform::TransformSystems::Propagate),
            );
    }
}
