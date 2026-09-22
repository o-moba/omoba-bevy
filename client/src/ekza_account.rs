//! Persistent, project-scoped Ekza connection. All HTTP uses the Ekza SDK.
use omoba_passport::{
    PairingState,
    account::{self, AccountCredential, AccountError, AccountFlow, AccountSession},
    store,
};
use std::{
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
struct State {
    session: Option<AccountSession>,
    flow: Option<AccountFlow>,
    path: Option<PathBuf>,
    generation: u64,
    worker: bool,
    last: Option<Instant>,
    opened: Option<String>,
    error: Option<String>,
}
static STATE: Mutex<State> = Mutex::new(State {
    session: None,
    flow: None,
    path: None,
    generation: 0,
    worker: false,
    last: None,
    opened: None,
    error: None,
});
const REFRESH: Duration = Duration::from_secs(15);

pub fn initialize(path: PathBuf) {
    STATE.lock().unwrap().path = Some(path);
}
pub fn account_connected() -> bool {
    STATE.lock().unwrap().session.is_some()
}

/// The menu button toggles between connecting and signing out of this installation.
pub fn connect_account() {
    let mut state = STATE.lock().unwrap();
    if state.session.is_some() {
        if let Some(path) = &state.path {
            if let Err(error) = std::fs::remove_file(path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    state.error =
                        Some("Could not remove the saved connection. Retry sign out.".into());
                    return;
                }
            }
        }
        state.generation += 1;
        state.session = None;
        state.flow = None;
        state.worker = false;
        state.error = None;
        state.last = Some(Instant::now());
        return;
    }
    if state.flow.as_ref().is_some_and(AccountFlow::in_progress) {
        return;
    }
    state.generation += 1;
    state.worker = false;
    state.opened = None;
    state.error = None;
    match account::start() {
        Ok(flow) => state.flow = Some(flow),
        Err(error) => state.error = Some(error),
    }
}

pub fn poll_account() -> bool {
    let mut state = STATE.lock().unwrap();
    match state.flow.as_ref().map(AccountFlow::state) {
        Some(PairingState::AwaitingApproval {
            verification_url, ..
        }) => {
            if state.opened.as_ref() != Some(&verification_url) {
                let _ = omoba_passport::open_in_browser(&verification_url);
                state.opened = Some(verification_url);
            }
        }
        Some(PairingState::Connected) => {
            let session = state.flow.as_ref().and_then(AccountFlow::take_session);
            state.flow = None;
            if let Some(session) = session {
                if let Some(path) = &state.path {
                    if session.credential().save(path).is_err() {
                        state.error =
                            Some("Connected for this run; could not save the connection.".into());
                    }
                }
                state.session = Some(session);
                state.last = Some(Instant::now());
                store::request_refresh();
                return true;
            }
        }
        Some(PairingState::Starting) => {}
        _ => {
            if state.worker || state.last.is_some_and(|at| at.elapsed() < REFRESH) {
                return false;
            }
            let Some(path) = state.path.clone() else {
                return false;
            };
            let generation = state.generation;
            let active = state.session.clone();
            state.worker = true;
            state.last = Some(Instant::now());
            std::thread::spawn(move || {
                let result = (|| {
                    if let Some(mut session) = active {
                        session.refresh_checked()?;
                        return Ok(Some(session));
                    }
                    let saved = AccountCredential::load(&path).map_err(|_| {
                        AccountError::Unavailable("Could not read the saved connection.".into())
                    })?;
                    let Some(saved) = saved else {
                        return Ok(None);
                    };
                    let api = account::client().map_err(AccountError::Unavailable)?;
                    api.restore(&saved, &omoba_passport::selector()).map(Some)
                })();
                let mut state = STATE.lock().unwrap();
                if state.generation != generation {
                    return;
                }
                state.worker = false;
                match result {
                    Ok(session) => {
                        state.session = session;
                        state.error = None;
                        store::request_refresh();
                    }
                    Err(AccountError::Unauthorized) => {
                        state.session = None;
                        let _ = std::fs::remove_file(&path);
                        state.error =
                            Some("Ekza connection expired or was revoked. Connect again.".into());
                    }
                    Err(AccountError::InvalidCredential) => { /* Keep credentials for the other configured server. */
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
            });
        }
    }
    false
}

pub fn account_status_line() -> String {
    let state = STATE.lock().unwrap();
    if let Some(error) = &state.error {
        return error.clone();
    }
    if let Some(session) = &state.session {
        return format!(
            "Ekza account {} · connection saved on this device",
            session.username
        );
    }
    match state.flow.as_ref().map(AccountFlow::state) {
        Some(PairingState::Starting) => "Contacting Ekza…".into(),
        Some(PairingState::AwaitingApproval {
            user_code,
            verification_url,
            ..
        }) => format!("Confirm in your browser · code {user_code} · {verification_url}"),
        Some(PairingState::Failed(error)) => error,
        _ if state.worker => "Restoring Ekza connection…".into(),
        _ => "Connect your Ekza account to see your own library · no wallet needed".into(),
    }
}

pub fn library_avatars() -> Vec<&'static shared::AvatarDefinition> {
    let state = STATE.lock().unwrap();
    let Some(session) = &state.session else {
        return Vec::new();
    };
    shared::store_avatars()
        .into_iter()
        .filter(|avatar| avatar.free && session.has(&avatar.slug))
        .collect()
}
