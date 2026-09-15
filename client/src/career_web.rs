//! Explicit website approval; background HTTP never receives the private game key.
use super::*;
use crate::career_identity::CareerIdentity;
use omoba_passport::web_account::WebAccountApi;
use shared::web_account::{WebPairChallenge, WebPairDecision};
use std::sync::{Mutex, mpsc};
#[derive(Clone, Default, PartialEq)]
pub(super) struct WebState {
    pub code: String,
    pub focused: bool,
    pub challenge: Option<WebPairChallenge>,
    pub message: Option<String>,
    pub busy: bool,
    generation: u64,
}
#[derive(Resource, Default)]
pub(super) struct Worker {
    pending: Option<Mutex<mpsc::Receiver<(u64, Result<Option<WebPairChallenge>, String>)>>>,
}
impl WebState {
    pub(super) fn dismiss(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.challenge = None;
        self.focused = false;
        self.busy = false;
    }
}
pub(super) fn append(code: &mut String, text: &str) -> Result<(), &'static str> {
    let normalized = text.trim().to_ascii_uppercase().replace([' ', '-'], "");
    if !normalized.bytes().all(|b| b.is_ascii_alphanumeric()) || code.len() + normalized.len() > 8 {
        return Err("Enter the eight-character code shown on the website.");
    }
    code.push_str(&normalized);
    Ok(())
}
pub(super) fn act(
    action: &Action,
    career: &mut CareerClient,
    identity: &CareerIdentity,
    worker: &mut Worker,
) {
    match action {
        Action::WebOpen => {
            career.web.dismiss();
            career.web.code.clear();
            career.web.message = None;
            career.modal = CareerModal::WebLink;
            career.nickname_focused = false;
            career.friend_code_focused = false;
            career.preedit.clear();
        }
        Action::WebEdit if !career.web.busy => {
            career.web.challenge = None;
            career.web.focused = true;
            career.web.message = None;
        }
        Action::WebLookup if !career.web.busy => {
            if worker.pending.is_some() {
                career.web.message =
                    Some("Previous website request is finishing. Try again shortly.".into());
                return;
            }
            if career.web.code.len() != 8 {
                career.web.message =
                    Some("Enter the eight-character code from the website.".into());
                return;
            }
            let key = match identity.public_key() {
                Ok(k) => k,
                Err(e) => {
                    career.web.message = Some(e);
                    return;
                }
            };
            let code = career.web.code.clone();
            let generation = career.web.generation;
            career.web.focused = false;
            career.web.busy = true;
            career.web.challenge = None;
            career.web.message = None;
            launch(worker, generation, move || {
                WebAccountApi::from_env()?.lookup(&code, &key).map(Some)
            });
        }
        Action::WebApprove | Action::WebDeny if !career.web.busy => {
            if worker.pending.is_some() {
                return;
            }
            let Some(challenge) = career.web.challenge.clone() else {
                return;
            };
            let decision = if matches!(action, Action::WebApprove) {
                WebPairDecision::Approve
            } else {
                WebPairDecision::Deny
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |t| t.as_secs());
            let proof =
                match identity.sign_web_pair(challenge.clone(), decision, &challenge.origin, now) {
                    Ok(p) => p,
                    Err(e) => {
                        career.web.message = Some(e);
                        return;
                    }
                };
            let generation = career.web.generation;
            career.web.busy = true;
            career.web.message = None;
            launch(worker, generation, move || {
                WebAccountApi::from_env()?.decide(&proof)?;
                Ok(None)
            });
        }
        _ => {}
    }
}
fn launch(
    worker: &mut Worker,
    generation: u64,
    job: impl FnOnce() -> Result<Option<WebPairChallenge>, String> + Send + 'static,
) {
    let (tx, rx) = mpsc::sync_channel(1);
    worker.pending = Some(Mutex::new(rx));
    std::thread::spawn(move || {
        let _ = tx.send((generation, job()));
    });
}
pub(super) fn poll(mut career: ResMut<CareerClient>, mut worker: ResMut<Worker>) {
    let result = worker
        .pending
        .as_ref()
        .and_then(|r| r.lock().ok().map(|rx| rx.try_recv()));
    match result {
        Some(Ok((generation, result))) => {
            worker.pending = None;
            if career.modal != CareerModal::WebLink || career.web.generation != generation {
                return;
            }
            career.web.busy = false;
            match result {
                Ok(Some(challenge)) => career.web.challenge = Some(challenge),
                Ok(None) => {
                    career.web.challenge = None;
                    career.web.message = Some("Decision sent. Return to the website.".into());
                }
                Err(e) => career.web.message = Some(e),
            }
        }
        Some(Err(mpsc::TryRecvError::Disconnected)) => {
            worker.pending = None;
            career.web.busy = false;
            career.web.message = Some("Website request stopped. Please retry.".into());
        }
        _ => {}
    }
}
pub(super) fn body(parent: &mut ChildSpawnerCommands, career: &CareerClient) {
    label(
        parent,
        "Connect player portal",
        26.,
        ui::GOLD,
        "WebLinkTitle",
    );
    label(
        parent,
        "Open the player website and enter its eight-character code here. Only approve a login you started yourself.",
        16.,
        ui::IVORY,
        "WebLinkInstructions",
    );
    button(
        parent,
        &format!(
            "Code: {}{}",
            career.web.code,
            if career.web.focused { " |" } else { "" }
        ),
        Action::WebEdit,
        "WebLinkCode",
    );
    if !career.web.busy {
        button(parent, "Look up code", Action::WebLookup, "WebLinkLookup");
    }
    if let Some(c) = &career.web.challenge {
        label(
            parent,
            format!(
                "Website: {}\nAccount: {}\nRead career history, manage friends, nickname and privacy settings.\nExpires: {} (Unix seconds).",
                c.origin,
                career
                    .view
                    .profile
                    .as_ref()
                    .map(|p| p.nickname.as_str())
                    .unwrap_or(&career.nickname),
                c.expires_at
            ),
            16.,
            ui::IVORY,
            "WebLinkAudience",
        );
        if !career.web.busy {
            button(
                parent,
                "Approve website access",
                Action::WebApprove,
                "WebLinkApprove",
            );
            button(parent, "Deny", Action::WebDeny, "WebLinkDeny");
        }
    }
    if career.web.busy {
        label(parent, "Contacting website…", 15., ui::MUTED, "WebLinkBusy");
    }
    if let Some(message) = &career.web.message {
        label(parent, message, 15., ui::GOLD, "WebLinkMessage");
    }
    if let Some(error) = &career.form_error {
        label(parent, error, 14., ui::GOLD, "WebLinkInputError");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn code_paste_is_bounded_and_atomic() {
        let mut code = String::new();
        append(&mut code, " abcd-2345 ").unwrap();
        assert_eq!(code, "ABCD2345");
        assert!(append(&mut code, "X").is_err());
        assert_eq!(code, "ABCD2345");
        assert!(append(&mut String::new(), "ошибка").is_err());
    }
    #[test]
    fn closing_invalidates_inflight_lookup_and_confirmation() {
        let mut state = WebState::default();
        let generation = state.generation;
        state.busy = true;
        state.focused = true;
        state.dismiss();
        assert_ne!(state.generation, generation);
        assert!(!state.focused);
        assert!(!state.busy);
        assert!(state.challenge.is_none());
    }
    #[test]
    fn opening_never_signs_or_sends_a_network_request() {
        let mut career = CareerClient::default();
        let identity = CareerIdentity::default();
        let mut worker = Worker::default();
        act(&Action::WebOpen, &mut career, &identity, &mut worker);
        assert_eq!(career.modal, CareerModal::WebLink);
        assert!(worker.pending.is_none());
        act(&Action::WebApprove, &mut career, &identity, &mut worker);
        assert!(worker.pending.is_none());
        assert!(!career.web.busy);
    }
}
