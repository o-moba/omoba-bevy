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
/// The running connection profile's resume hint: another profile's lobby
/// never sees it (production keeps it beside the preferences file).
fn resume_path() -> Option<std::path::PathBuf> {
    crate::connection_profile::profile_file("match-resume.json")
}
pub(crate) fn load_resume(
    mut resume: ResMut<ResumeMatchState>,
    session: Res<ClientSessionId>,
    directory: Option<Res<super::directory::DirectoryRouting>>,
) {
    if directory
        .as_ref()
        .is_some_and(|directory| directory.configured())
    {
        // Do not delete an old owning-lobby hint by comparing it to today's
        // static default. Authenticated directory ownership is checked at boot.
        return;
    }
    load_legacy_resume(
        &mut resume,
        &session.0,
        &super::session::startup_lobby_addr(),
    );
}
pub(super) fn load_legacy_resume(resume: &mut ResumeMatchState, identity: &str, lobby: &str) {
    resume.saved = resume_path().and_then(|path| load_resume_at(&path, identity, lobby));
}
fn read_hint(path: &std::path::Path) -> Option<SavedResume> {
    use std::io::Read;
    // Bound allocation even if an untrusted file grows after metadata lookup.
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > 16_384 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(16_385).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 16_384 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}
pub(super) fn directory_resume_hint(identity: &str) -> Option<SavedResume> {
    let saved = read_hint(&resume_path()?)?;
    valid_resume(&saved, identity, &saved.lobby).then_some(saved)
}
/// The hint at `path` when it belongs to `session` on `lobby` (this process's
/// lobby). A hint for another lobby is deleted: a build that no longer offers
/// that server (an older build's custom or LAN lobby, the production app since
/// it lost server selection) must not send the player back to it.
fn load_resume_at(path: &std::path::Path, session: &str, lobby: &str) -> Option<SavedResume> {
    let saved = read_hint(path)?;
    if !same_lobby(&saved.lobby, lobby) {
        let _ = std::fs::remove_file(path);
        return None;
    }
    valid_resume(&saved, session, lobby).then_some(saved)
}
fn same_lobby(saved: &str, lobby: &str) -> bool {
    crate::persistence::validate_game_server_addr(saved).is_some_and(|saved| saved == lobby)
}
fn valid_resume(saved: &SavedResume, session: &str, lobby: &str) -> bool {
    saved.session_id == session
        && saved.server_epoch != 0
        && saved.match_id != 0
        && crate::persistence::validate_game_server_addr(&saved.allocation.endpoint).is_some()
        && same_lobby(&saved.lobby, lobby)
}
pub(crate) fn remember_resume(
    mut resume: ResMut<ResumeMatchState>,
    flow: Res<MatchServiceClient>,
    session: Res<ClientSession>,
    identity: Res<ClientSessionId>,
    snapshot: Res<GameStateSnapshot>,
    career: Res<CareerClient>,
    auth: Res<crate::career_identity::CareerIdentity>,
    directory: Option<Res<super::directory::DirectoryRouting>>,
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
    let lobby = directory
        .as_ref()
        .filter(|d| d.active)
        .and_then(|d| d.owned_lobby())
        .unwrap_or_else(super::session::startup_lobby_addr);
    if resume.saved.as_ref() != Some(&saved) && valid_resume(&saved, &identity.0, &lobby) {
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
    career: Option<Res<CareerClient>>,
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
    // The lobby's in-match answer to this search (LR-13). The saved hint of
    // the same allocation is preferred: it holds the exact committed join and
    // the match scope that guards against a reused worker port.
    let view = career.as_ref().map(|career| &career.view);
    let live = view.and_then(|view| {
        flow.in_match(view)
            .map(|(seat, _)| seat.allocation_id.clone())
    });
    if let (Some(view), Some(live)) = (view, live)
        && resume
            .saved
            .as_ref()
            .is_none_or(|saved| saved.allocation.allocation_id != live)
    {
        if let Some(endpoint) = flow.resume_in_match(view) {
            messages
                .p1()
                .write(SessionUiCommand::ConnectAllocated(endpoint));
        }
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
        assert!(valid_resume(&decoded, "stable-session", "127.0.0.1:4000"));
        assert!(!valid_resume(&decoded, "another-session", "127.0.0.1:4000"));
        assert!(!valid_resume(&decoded, "stable-session", "127.0.0.1:4001"));
    }

    fn saved_on(lobby: &str) -> SavedResume {
        SavedResume {
            allocation: shared::match_service::MatchAllocation {
                allocation_id: "b".repeat(32),
                endpoint: "192.168.1.20:41000".into(),
                preference: Default::default(),
                team: shared::map::Team::Blue,
                human_count: 1,
                bot_count: 9,
                rated: false,
                join_deadline_ms: 1,
            },
            lobby: lobby.into(),
            session_id: "stable-session".into(),
            server_epoch: 3,
            match_id: 5,
            join: CommittedJoin::for_test(),
        }
    }

    fn scratch_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "omoba-resume-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("match-resume.json")
    }

    #[test]
    fn a_hint_for_another_lobby_is_dropped_and_deleted() {
        // An older build saved this while the player was on a LAN lobby; this
        // build connects to its own lobby (the public one in production).
        let lobby = super::super::session::startup_lobby_addr();
        assert_ne!(lobby, "192.168.1.20:4000");
        let path = scratch_file("other-lobby");
        std::fs::write(
            &path,
            serde_json::to_vec(&saved_on("192.168.1.20:4000")).unwrap(),
        )
        .unwrap();
        assert_eq!(load_resume_at(&path, "stable-session", &lobby), None);
        assert!(!path.exists(), "a stale hint must not offer Resume again");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_hint_for_this_lobby_is_kept() {
        let lobby = super::super::session::startup_lobby_addr();
        let path = scratch_file("this-lobby");
        let saved = saved_on(&lobby);
        std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
        assert_eq!(load_resume_at(&path, "stable-session", &lobby), Some(saved));
        // Another identity on the same lobby gets no Resume, but the file stays.
        assert_eq!(load_resume_at(&path, "another-session", &lobby), None);
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
