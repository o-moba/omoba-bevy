mod apply;
mod commands;
mod components;
mod ingest;
mod interpolate;
mod offline;
#[cfg(feature = "qa")]
pub(crate) use offline::{qa_finish_match, qa_practice_positions};
mod public_transport;
pub(crate) mod recovery;
pub(crate) mod recovery_ui;
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
pub(crate) use status_ui::{CompatibilityVerdict, LinkStatus, compatibility_reason, link_status};

use apply::{
    StagedSnapshot, respawn_players_with_new_store_models, respawn_sandbox_models,
    snapshot_apply_systems,
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
    /// and the Draft gate. Runs even when the entity stages are skipped.
    Resources,
    /// Draft: despawns every hero and keeps the stages below closed.
    /// Otherwise reconciles or spawns the local hero, or closes the stages
    /// below while the listed local hero has no team and no committed join.
    LocalPlayer,
    /// Remote heroes (interpolation buffers, dash VFX after the local one).
    RemotePlayers,
    Projectiles,
    Structures,
    Minions,
    /// Jungle camps and bosses.
    Neutrals,
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
            SnapshotApply::LocalPlayer,
            SnapshotApply::RemotePlayers,
            SnapshotApply::Projectiles,
            SnapshotApply::Structures,
            SnapshotApply::Minions,
            SnapshotApply::Neutrals,
            SnapshotApply::Finish,
        )
            .chain()
            .in_set(ClientNetPipeline::ApplySnapshot),
    );
}

impl Plugin for NetworkingPlugin {
    fn build(&self, app: &mut App) {
        use crate::ui::UiActionAppExt;
        configure_network_pipeline(app);
        app.add_message::<NetworkCommand>()
            .add_message::<SessionUiCommand>()
            .add_ui_action::<status_ui::RetryPressed>()
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
            // The lobby transport opens at boot only once the region's legal
            // documents are accepted, else in the frame the player accepts
            // them (LR-06, `crate::legal`).
            .add_systems(
                Startup,
                (
                    start_networking
                        .after(crate::persistence::load_persistent_client_settings)
                        .run_if(crate::legal::bound_and_accepted),
                    setup_connection_status_ui,
                    offline::setup_banner,
                ),
            )
            .add_systems(
                Update,
                start_networking
                    .after(crate::account_region::ActivateRegion)
                    .run_if(on_message::<crate::legal::LegalAccepted>)
                    .before(ClientNetPipeline::SendLocalState),
            )
            .add_systems(
                Update,
                send_local_state
                    .run_if(crate::account_region::bound)
                    .in_set(ClientNetPipeline::SendLocalState),
            )
            .add_systems(
                Update,
                send_network_commands
                    .run_if(crate::account_region::bound)
                    .in_set(ClientNetPipeline::SendCommands),
            )
            .add_systems(Update, offline::sync_banner)
            .add_systems(
                Update,
                (
                    respawn_players_with_new_store_models,
                    respawn_sandbox_models,
                )
                    .after(ClientNetPipeline::IngestSnapshot)
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
                handle_connection_retry_button
                    .in_set(ClientNetPipeline::SessionRetryInput)
                    .after(crate::ui::UiSet::Dispatch),
            )
            .add_systems(
                Update,
                (
                    update_session_lifecycle,
                    retry_pending_join,
                    flush_session_events,
                )
                    .chain()
                    // Every transport (re)start but the boot one is here.
                    .distributive_run_if(crate::legal::bound_and_accepted)
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

/// A region switch takes effect only at the next cold start. Dropping the sole
/// outgoing channel closes its UDP thread; no old account traffic continues.
pub(crate) fn stop_for_region_switch(world: &mut World) {
    if world
        .remove_resource::<transport::NetworkChannels>()
        .is_some()
    {
        world.insert_resource(ClientSession::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legal::gate_tests::{assert_carries, gate, on_acceptance};

    /// LR-06: no lobby transport opens before the region's legal documents
    /// are accepted. The boot start waits for the acceptance and runs again
    /// in its frame; every later (re)start is in the gated session lifecycle.
    #[test]
    fn the_lobby_waits_for_the_legal_acceptance() {
        let mut app = App::new();
        app.add_plugins(NetworkingPlugin);
        let start = IntoSystem::system_type_id(&start_networking);
        assert_carries(&app, Startup, start, gate(), "start_networking at boot");
        assert_carries(
            &app,
            Update,
            start,
            on_acceptance(),
            "start_networking on acceptance",
        );
        for (system, what) in [
            (
                IntoSystem::system_type_id(&update_session_lifecycle),
                "update_session_lifecycle",
            ),
            (
                IntoSystem::system_type_id(&retry_pending_join),
                "retry_pending_join",
            ),
        ] {
            assert_carries(&app, Update, system, gate(), what);
        }
    }
}
