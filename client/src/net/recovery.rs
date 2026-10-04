//! A durable hint for reclaiming an authenticated, ongoing allocated seat.
//! This stores no account key or admission ticket. Authority remains server-side.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::session::CommittedJoin;
use super::{ClientSession, GameStateSnapshot, SessionUiCommand};
use crate::{
    career::CareerClient, match_service::MatchServiceClient, persistence::ClientSessionId,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedResume {
    pub allocation: shared::match_service::MatchAllocation,
    pub lobby: String,
    pub session_id: String,
    pub server_epoch: u64,
    pub match_id: u64,
    pub join: CommittedJoin,
}
#[derive(Resource, Default)]
pub(crate) struct ResumeMatchState {
    pub saved: Option<SavedResume>,
}
impl ResumeMatchState {
    pub fn clear(&mut self) {
        if self.saved.take().is_some() {
            self.persist();
        }
    }
    fn persist(&self) {
        let Some(path) = resume_path() else {
            return;
        };
        let Some(saved) = &self.saved else {
            let _ = std::fs::remove_file(path);
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(bytes) = serde_json::to_vec(saved) else {
            return;
        };
        let temporary = path.with_extension("json.tmp");
        if std::fs::write(&temporary, bytes).is_ok() {
            let _ = std::fs::rename(temporary, path);
        }
    }
}
fn resume_path() -> Option<std::path::PathBuf> {
    crate::persistence::preferences_path().map(|path| path.with_file_name("match-resume.json"))
}
pub(crate) fn load_resume(mut resume: ResMut<ResumeMatchState>, session: Res<ClientSessionId>) {
    resume.saved = resume_path()
        .and_then(|path| std::fs::read(path).ok())
        .filter(|bytes| bytes.len() <= 16_384)
        .and_then(|bytes| serde_json::from_slice::<SavedResume>(&bytes).ok())
        .filter(|saved| valid_resume(saved, &session.0));
}
fn valid_resume(saved: &SavedResume, session: &str) -> bool {
    saved.session_id == session
        && saved.server_epoch != 0
        && saved.match_id != 0
        && crate::persistence::validate_game_server_addr(&saved.allocation.endpoint).is_some()
        && crate::persistence::validate_game_server_addr(&saved.lobby).is_some()
}
pub(crate) fn remember_resume(
    mut resume: ResMut<ResumeMatchState>,
    flow: Res<MatchServiceClient>,
    session: Res<ClientSession>,
    identity: Res<ClientSessionId>,
    snapshot: Res<GameStateSnapshot>,
    career: Res<CareerClient>,
    auth: Res<crate::career_identity::CareerIdentity>,
) {
    if let Some(saved) = &resume.saved
        && session.server_addr() == saved.lobby
        && auth.authenticated_for_scope(
            session.server_addr(),
            snapshot.meta.server_epoch,
            &identity.0,
        )
        && matches!(
            career.view.match_service,
            Some(
                shared::match_service::MatchServiceView::Idle
                    | shared::match_service::MatchServiceView::Failed { .. }
            )
        )
    {
        // An authenticated lobby knows that this profile no longer has an
        // allocation. Retired workers must not leave an endless Resume button.
        resume.clear();
        return;
    }
    if let Some(saved) = &resume.saved
        && career.view.last_result.as_ref().is_some_and(|result| {
            result.server_epoch == saved.server_epoch && result.match_id == saved.match_id
        })
    {
        resume.clear();
        return;
    }
    if !session.join_confirmed() || session.is_offline() {
        return;
    }
    let (Some(allocation), Some(lobby), Some(join)) =
        (&flow.allocation, &flow.lobby_addr, &session.last_join)
    else {
        return;
    };
    if career.view.last_result.as_ref().is_some_and(|result| {
        result.server_epoch == snapshot.meta.server_epoch
            && result.match_id == snapshot.meta.match_id
    }) {
        return;
    }
    let saved = SavedResume {
        allocation: allocation.clone(),
        lobby: lobby.clone(),
        session_id: identity.0.clone(),
        server_epoch: snapshot.meta.server_epoch,
        match_id: snapshot.meta.match_id,
        join: join.clone(),
    };
    if valid_resume(&saved, &identity.0) && resume.saved.as_ref() != Some(&saved) {
        resume.saved = Some(saved);
        resume.persist();
    }
}
pub(crate) fn resume_actions(
    mut messages: ParamSet<(
        MessageReader<SessionUiCommand>,
        MessageWriter<SessionUiCommand>,
    )>,
    mut flow: ResMut<MatchServiceClient>,
    resume: Res<ResumeMatchState>,
) {
    // Reader and writer use separate cursors: the emitted ConnectAllocated is
    // consumed by the normal lifecycle later this frame.
    let resume_requested = messages
        .p0()
        .read()
        .any(|event| matches!(event, SessionUiCommand::ResumeMatch));
    if !resume_requested {
        return;
    }
    let Some(saved) = &resume.saved else {
        return;
    };
    flow.restore_resume(saved);
    messages.p1().write(SessionUiCommand::ConnectAllocated(
        saved.allocation.endpoint.clone(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_resume_round_trips_loadout_without_credentials_and_rejects_other_identity() {
        let saved = SavedResume {
            allocation: shared::match_service::MatchAllocation {
                allocation_id: "a".repeat(32),
                endpoint: "127.0.0.1:41000".into(),
                preference: Default::default(),
                team: shared::map::Team::Blue,
                human_count: 1,
                bot_count: 9,
                rated: false,
                join_deadline_ms: 1,
            },
            lobby: "127.0.0.1:4000".into(),
            session_id: "stable-session".into(),
            server_epoch: 9,
            match_id: 1,
            join: CommittedJoin::for_test(),
        };
        let json = serde_json::to_string(&saved).unwrap();
        assert!(!json.contains("passport_ticket"));
        let decoded: SavedResume = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, saved);
        assert!(valid_resume(&decoded, "stable-session"));
        assert!(!valid_resume(&decoded, "another-session"));
    }
}
