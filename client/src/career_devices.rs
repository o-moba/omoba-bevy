//! Account transfer uses a fresh local key and explicit activation after confirmation.
//!
//! Sign in with VK ID (markets with `vk_sign_in`) is the same device request: the
//! game opens the portal's `/connect?code=…` in the browser and checks the request
//! every few seconds. A VK account already linked to a game profile approves the
//! request as that account (switch after restart, like any device link); an unlinked
//! VK sign-in waits for the player to link it to this profile, signed by this
//! installation's active key.
// i18n-strict
use super::*;
use crate::career_identity::{CareerIdentity, hex, load_or_create_key};
use ed25519_dalek::{Signer, SigningKey};
use omoba_passport::device_account::DeviceAccountApi;
use shared::device_account::{
    DeviceAction, DeviceEnrollment, DeviceEnrollmentStatus, SignedDeviceEnrollment,
};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

/// How often a VK sign-in request is checked while the browser is open.
const SOCIAL_POLL: Duration = Duration::from_secs(3);
/// Bound retries even if the service keeps rejecting a request. The player can
/// check the same enrollment again; neither retries nor status checks rotate keys.
const AUTOMATIC_RETRIES: u8 = 5;

fn retry_delay(attempt: u8) -> Duration {
    SOCIAL_POLL * (1 << attempt.saturating_sub(1).min(3))
}

/// Whether this build offers "Sign in with VK ID" (`crate::account_region::features()`).
pub(super) fn vk_button_visible(features: shared::market::Features) -> bool {
    features.vk_sign_in
}

#[derive(Clone, Default, PartialEq)]
pub(super) struct DeviceState {
    pub enrollment: Option<DeviceEnrollment>,
    pub status: Option<DeviceEnrollmentStatus>,
    pub recovery_code: String,
    pub focused: bool,
    pub busy: bool,
    pub message: Option<String>,
    pub restart_required: bool,
    /// This request is a VK sign-in (`Action::SocialStart`).
    pub social: bool,
    /// The portal page was opened (or offered) for the request's code.
    pub browser_opened: bool,
    /// This request was started by «Enter a recovery code» (LR-18): it asks
    /// for a saved code first.
    pub recovery_entry: bool,
}
#[derive(Resource, Default)]
pub(super) struct Worker {
    key: Option<SigningKey>,
    pending: Option<Mutex<mpsc::Receiver<Result<DeviceEnrollmentStatus, String>>>>,
    activating: Option<String>,
    linking: bool,
    next_poll: Option<Instant>,
    retry: Option<SignedDeviceEnrollment>,
    retry_attempts: u8,
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
pub(super) fn append(code: &mut String, text: &str) -> Result<(), &'static str> {
    let normalized = text.trim().to_ascii_lowercase().replace([' ', '-'], "");
    if !normalized.bytes().all(|b| b.is_ascii_hexdigit()) || code.len() + normalized.len() > 64 {
        return Err(tr("devices.error.paste_full_code"));
    }
    code.push_str(&normalized);
    Ok(())
}
fn launch(worker: &mut Worker, proof: SignedDeviceEnrollment) {
    let (tx, rx) = mpsc::sync_channel(1);
    worker.pending = Some(Mutex::new(rx));
    worker.next_poll = None;
    // Recovery codes are bearer secrets: leave retrying that operation to the
    // player. The safe replayable enrollment requests carry no recovery code.
    worker.retry = (proof.action != DeviceAction::Recover).then(|| proof.clone());
    std::thread::spawn(move || {
        let _ = tx.send(DeviceAccountApi::from_env().and_then(|api| api.send(&proof)));
    });
}
fn launch_link(worker: &mut Worker, proof: shared::device_account::SignedSocialLink) {
    let (tx, rx) = mpsc::sync_channel(1);
    worker.pending = Some(Mutex::new(rx));
    worker.linking = true;
    worker.retry = None;
    worker.activating = None;
    worker.next_poll = None;
    std::thread::spawn(move || {
        let _ = tx.send(DeviceAccountApi::from_env().and_then(|api| api.link_social(&proof)));
    });
}
fn sign(
    worker: &Worker,
    e: &DeviceEnrollment,
    action: DeviceAction,
    target: Option<String>,
    code: Option<String>,
) -> Result<SignedDeviceEnrollment, String> {
    let key = worker.key.as_ref().ok_or(tr("devices.error.start_first"))?;
    let api = DeviceAccountApi::from_env()?;
    e.validate(api.origin(), &hex(key.verifying_key().as_bytes()), now())
        .map_err(str::to_owned)?;
    let signature = hex(&key
        .sign(&e.signing_bytes(action, target.as_deref(), code.as_deref()))
        .to_bytes());
    Ok(SignedDeviceEnrollment {
        enrollment: e.clone(),
        action,
        target_profile: target,
        recovery_code: code,
        signature,
    })
}
pub(super) fn act(
    action: &Action,
    career: &mut CareerClient,
    identity: &CareerIdentity,
    worker: &mut Worker,
) {
    if matches!(action, Action::DevicesOpen)
        || matches!(action, Action::SocialStart)
            && !career.devices.busy
            && !career.devices.restart_required
    {
        career.modal = CareerModal::Devices;
        career.nickname_focused = false;
        career.friend_code_focused = false;
        career.web.focused = false;
        career.preedit.clear();
        if matches!(action, Action::DevicesOpen) {
            return;
        }
    }
    if career.devices.busy || career.devices.restart_required {
        return;
    }
    if matches!(action, Action::DevicesRecoveryEdit) {
        career.devices.focused = true;
        career.devices.message = None;
        return;
    }
    let result = (|| -> Result<(), String> {
        if matches!(action, Action::DevicesPoll)
            && let Some(proof) = worker.retry.clone()
        {
            // Retry the original create/status/complete request. In particular,
            // a lost create response must not produce a different enrollment.
            if !enrollment_live(&career.devices, now()) {
                return Err(tr("devices.error.expired").into());
            }
            worker.retry_attempts = 0;
            launch(worker, proof);
            career.devices.busy = true;
            career.devices.message = None;
            return Ok(());
        }
        match action {
            Action::DevicesStart | Action::DevicesRecoveryStart | Action::SocialStart => {
                let api = DeviceAccountApi::from_env()?;
                let directory = identity.enrollment_directory()?;
                let mut key = load_or_create_key(&directory)?;
                if hex(key.verifying_key().as_bytes()) == identity.public_key()?
                    || career.devices.enrollment.is_some()
                {
                    // An explicit restart creates a fresh key, retaining any old candidate
                    // (including a completed enrollment whose response was lost).
                    let mut suffix = [0; 16];
                    getrandom::fill(&mut suffix).map_err(|_| tr("devices.error.randomness"))?;
                    let preserved =
                        directory.with_file_name(format!("enrolled-device-{}", hex(&suffix)));
                    std::fs::rename(&directory, preserved)
                        .map_err(|_| tr("devices.error.preserve_key"))?;
                    key = load_or_create_key(&directory)?;
                }
                let mut id = [0; 32];
                getrandom::fill(&mut id).map_err(|_| tr("devices.error.randomness"))?;
                let enrollment = DeviceEnrollment {
                    enrollment_id: hex(&id),
                    public_key: hex(key.verifying_key().as_bytes()),
                    origin: api.origin().into(),
                    // Sent to the account service and shown on the portal: not localized.
                    label: if cfg!(target_os = "ios") {
                        "iPhone / iPad".into() // i18n-allow
                    } else if cfg!(target_os = "android") {
                        "Android".into() // i18n-allow
                    } else {
                        "Desktop".into() // i18n-allow
                    },
                    expires_at: (now() + 300).to_string(),
                };
                worker.key = Some(key);
                let proof = sign(worker, &enrollment, DeviceAction::Create, None, None)?;
                career.devices.enrollment = Some(enrollment);
                career.devices.status = None;
                career.devices.focused = false;
                career.devices.recovery_code.clear();
                career.devices.social = matches!(action, Action::SocialStart);
                career.devices.recovery_entry = matches!(action, Action::DevicesRecoveryStart);
                career.devices.browser_opened = false;
                worker.next_poll = None;
                worker.activating = None;
                worker.retry_attempts = 0;
                launch(worker, proof);
            }
            Action::SocialLink => {
                let e = career
                    .devices
                    .enrollment
                    .as_ref()
                    .ok_or(tr("devices.error.start_first"))?;
                if career.devices.status.as_ref().map(|s| s.state.as_str())
                    != Some("social_pending")
                {
                    return Err(tr("devices.error.start_first").into());
                }
                let api = DeviceAccountApi::from_env()?;
                let proof = identity.sign_social_link(e, api.origin(), now())?;
                launch_link(worker, proof);
            }
            Action::DevicesPoll | Action::DevicesRecover | Action::DevicesConfirm => {
                let e = career
                    .devices
                    .enrollment
                    .as_ref()
                    .ok_or(tr("devices.error.start_first"))?;
                let operation = match action {
                    Action::DevicesPoll => DeviceAction::Status,
                    Action::DevicesRecover => DeviceAction::Recover,
                    _ => DeviceAction::Complete,
                };
                let target = if operation == DeviceAction::Complete {
                    Some(
                        career
                            .devices
                            .status
                            .as_ref()
                            .and_then(|s| s.profile_id.clone())
                            .ok_or(tr("devices.error.approve_first"))?,
                    )
                } else {
                    None
                };
                if target
                    .as_deref()
                    .is_some_and(|profile| !shared::career::valid_profile_id(profile))
                {
                    return Err(tr("devices.error.completion").into());
                }
                if operation == DeviceAction::Complete
                    && career
                        .devices
                        .status
                        .as_ref()
                        .is_some_and(|s| s.state == "consumed")
                {
                    identity.stage_enrolled_identity(&e.public_key)?;
                    career.devices.restart_required = true;
                    career.devices.message = Some(tr("devices.saved.preserved").into());
                    return Ok(());
                }
                let code = if operation == DeviceAction::Recover {
                    if career.devices.recovery_code.len() != 64 {
                        return Err(tr("devices.error.paste_code").into());
                    }
                    Some(career.devices.recovery_code.clone())
                } else {
                    None
                };
                let proof = sign(worker, e, operation, target, code)?;
                worker.activating = proof.target_profile.clone();
                worker.retry_attempts = 0;
                launch(worker, proof);
                career.devices.recovery_code.clear();
                career.devices.focused = false;
            }
            _ => return Ok(()),
        }
        career.devices.busy = true;
        career.devices.message = None;
        Ok(())
    })();
    if let Err(error) = result {
        career.devices.message = Some(error);
    }
}
pub(super) fn poll(
    mut career: ResMut<CareerClient>,
    mut worker: ResMut<Worker>,
    identity: Res<CareerIdentity>,
) {
    if worker.pending.is_none() {
        // Read-only until a check is due: no per-frame change detection on the UI.
        let now_instant = Instant::now();
        if worker.next_poll.is_some_and(|due| now_instant >= due) {
            request_tick(&mut career, &mut worker, &identity, now_instant);
        }
        return;
    }
    let result = worker
        .pending
        .as_ref()
        .and_then(|r| r.lock().ok().map(|rx| rx.try_recv()));
    let value = match result {
        Some(Ok(r)) => r,
        Some(Err(mpsc::TryRecvError::Disconnected)) => Err(tr("devices.error.stopped").into()),
        _ => return,
    };
    worker.pending = None;
    career.devices.busy = false;
    if std::mem::take(&mut worker.linking) {
        match value {
            Ok(status) if status.state == "linked" => {
                // The account is protected now: Profile and Home read it again.
                career.protection.status = None;
                career.protection.wanted = true;
                finish_social(&mut career, &mut worker, tr("devices.vk.linked").into())
            }
            Ok(_) => career.devices.message = Some(tr("devices.error.stopped").into()),
            Err(error) => career.devices.message = Some(error),
        }
        return;
    }
    match value {
        Ok(mut status) => {
            worker.retry = None;
            worker.retry_attempts = 0;
            let activating = worker.activating.take();
            if activating
                .as_deref()
                .is_some_and(|target| !completion_matches(&status, target))
            {
                // Keep the approved account visible so the player can safely
                // confirm again. Never stage a key from an unexpected reply.
                worker.next_poll = None;
                career.devices.message = Some(tr("devices.error.completion").into());
                return;
            }
            if status.code.is_none() {
                status.code = career.devices.status.as_ref().and_then(|s| s.code.clone());
            }
            let own_account = career.view.profile.as_ref().map(|p| p.profile_id.as_str());
            match social_next(&career.devices, &status, own_account) {
                SocialNext::OpenBrowser => {
                    career.devices.browser_opened = true;
                    career.devices.message = open_sign_in(status.code.as_deref());
                    worker.next_poll = Some(Instant::now() + SOCIAL_POLL);
                }
                SocialNext::Wait => worker.next_poll = Some(Instant::now() + SOCIAL_POLL),
                SocialNext::SameAccount => {
                    finish_social(
                        &mut career,
                        &mut worker,
                        tr("devices.vk.same_account").into(),
                    );
                    return;
                }
                SocialNext::Stop => worker.next_poll = None,
            }
            career.devices.status = Some(status);
            if activating.is_some() {
                let result = career
                    .devices
                    .enrollment
                    .as_ref()
                    .ok_or_else(|| tr("devices.error.enrollment_missing").to_owned())
                    .and_then(|e| identity.stage_enrolled_identity(&e.public_key));
                match result {
                    Ok(()) => {
                        career.devices.restart_required = true;
                        career.devices.message = Some(tr("devices.saved.not_merged").into());
                    }
                    Err(e) => career.devices.message = Some(e),
                }
            }
        }
        Err(error) => {
            career.devices.message = Some(error);
            if worker.retry.is_some()
                && enrollment_live(&career.devices, now())
                && worker.retry_attempts < AUTOMATIC_RETRIES
            {
                worker.retry_attempts += 1;
                worker.next_poll = Some(Instant::now() + retry_delay(worker.retry_attempts));
            } else {
                worker.next_poll = None;
            }
        }
    }
}
fn completion_matches(status: &DeviceEnrollmentStatus, target: &str) -> bool {
    status.state == "consumed"
        && shared::career::valid_profile_id(target)
        && status.profile_id.as_deref() == Some(target)
}
fn enrollment_live(state: &DeviceState, now_secs: u64) -> bool {
    state
        .enrollment
        .as_ref()
        .and_then(|e| e.expires_at.parse::<u64>().ok())
        .is_some_and(|expiry| expiry > now_secs)
}
/// What a VK sign-in request does after a status reply.
#[derive(Debug, PartialEq, Eq)]
enum SocialNext {
    /// First reply with a code: open the portal's sign-in page.
    OpenBrowser,
    /// Still waiting for the browser: check again shortly.
    Wait,
    /// The VK account is this installation's own: nothing to switch.
    SameAccount,
    /// The player decides now (link, switch account) or the request ended.
    Stop,
}
fn social_next(
    state: &DeviceState,
    status: &DeviceEnrollmentStatus,
    own_account: Option<&str>,
) -> SocialNext {
    if !state.social {
        return SocialNext::Stop;
    }
    match status.state.as_str() {
        "pending" if !state.browser_opened && status.code.is_some() => SocialNext::OpenBrowser,
        "pending" => SocialNext::Wait,
        "approved" | "consumed"
            if own_account.is_some() && status.profile_id.as_deref() == own_account =>
        {
            SocialNext::SameAccount
        }
        _ => SocialNext::Stop,
    }
}
/// Open the portal's sign-in page; the modal always shows its address too
/// (phones without a browser bridge open it by hand).
fn open_sign_in(code: Option<&str>) -> Option<String> {
    let url = DeviceAccountApi::from_env().and_then(|api| api.sign_in_url(code.unwrap_or("")));
    match url {
        Ok(url) if crate::platform::open_external_url(&url).is_ok() => {
            Some(tr("devices.vk.opened").to_owned())
        }
        Ok(_) => None,
        Err(error) => Some(error),
    }
}
fn finish_social(career: &mut CareerClient, worker: &mut Worker, message: String) {
    career.devices.enrollment = None;
    career.devices.status = None;
    career.devices.social = false;
    career.devices.browser_opened = false;
    career.devices.message = Some(message);
    worker.next_poll = None;
    worker.activating = None;
    worker.retry = None;
    worker.retry_attempts = 0;
}
/// Re-check browser sign-in or replay a failed enrollment operation. Every replay
/// keeps the original proof/key and stops at expiry or the automatic retry limit.
fn request_tick(
    career: &mut CareerClient,
    worker: &mut Worker,
    identity: &CareerIdentity,
    now_instant: Instant,
) {
    let Some(due) = worker.next_poll else {
        return;
    };
    if now_instant < due || career.devices.busy {
        return;
    }
    worker.next_poll = None;
    if !enrollment_live(&career.devices, now()) {
        let message = if career.devices.social {
            tr("devices.vk.expired")
        } else {
            tr("devices.error.expired")
        };
        finish_social(career, worker, message.into());
        return;
    }
    if let Some(proof) = worker.retry.clone() {
        launch(worker, proof);
        career.devices.busy = true;
    } else if career.devices.social {
        act(&Action::DevicesPoll, career, identity, worker);
    }
}
pub(super) fn body(parent: &mut ChildSpawnerCommands, career: &CareerClient) {
    let state = &career.devices;
    label(parent, tr("devices.title"), 26., ui::GOLD, "DeviceTitle");
    label(
        parent,
        tr("devices.instructions"),
        16.,
        ui::IVORY,
        "DeviceInstructions",
    );
    if let Some(e) = &state.enrollment {
        label(
            parent,
            trf(
                "devices.fingerprint",
                &[
                    ("origin", &e.origin),
                    ("start", &&e.public_key[..8]),
                    ("end", &&e.public_key[56..]),
                ],
            ),
            14.,
            ui::MUTED,
            "DeviceFingerprint",
        );
    }
    if let Some(status) = &state.status {
        if state.social && status.state == "pending" {
            label(
                parent,
                tr("devices.vk.waiting"),
                16.,
                ui::IVORY,
                "DeviceVkWaiting",
            );
            if let Some(code) = &status.code {
                let url = state.enrollment.as_ref().and_then(|e| {
                    omoba_passport::device_account::sign_in_url(&e.origin, code).ok()
                });
                if let Some(url) = url {
                    label(
                        parent,
                        trf("devices.vk.open_manually", &[("url", &url)]),
                        14.,
                        ui::MUTED,
                        "DeviceVkUrl",
                    );
                }
                label(
                    parent,
                    trf("devices.code", &[("code", code)]),
                    22.,
                    ui::GOLD,
                    "DeviceCode",
                );
            }
            if !state.busy {
                button(
                    parent,
                    tr("devices.button.poll"),
                    Action::DevicesPoll,
                    "DevicePoll",
                );
            }
        } else if status.state == "social_pending" {
            let name = status
                .social
                .as_ref()
                .map_or("VK", |s| s.display_name.as_str()); // i18n-allow
            let profile = career
                .view
                .profile
                .as_ref()
                .map_or(tr("devices.vk.this_profile"), |p| p.nickname.as_str());
            label(
                parent,
                trf(
                    "devices.vk.link_prompt",
                    &[("name", &name), ("profile", &profile)],
                ),
                18.,
                ui::GOLD,
                "DeviceVkLinkPrompt",
            );
            if !state.busy {
                button(
                    parent,
                    tr("devices.button.vk_link"),
                    Action::SocialLink,
                    "DeviceVkLink",
                );
            }
        } else if status.state == "pending" && state.recovery_entry {
            label(
                parent,
                tr("protect.recovery.entry"),
                16.,
                ui::IVORY,
                "DeviceRecoveryPrompt",
            );
            button(
                parent,
                &recovery_field(state),
                Action::DevicesRecoveryEdit,
                "DeviceRecoveryField",
            );
            if !state.busy {
                button(
                    parent,
                    tr("devices.button.recover"),
                    Action::DevicesRecover,
                    "DeviceRecover",
                );
            }
        } else if status.state == "pending" {
            if let Some(code) = &status.code {
                label(
                    parent,
                    trf("devices.code", &[("code", code)]),
                    28.,
                    ui::GOLD,
                    "DeviceCode",
                );
            }
            label(
                parent,
                tr("devices.portal_instructions"),
                15.,
                ui::IVORY,
                "DevicePortalInstructions",
            );
            if !state.busy {
                button(
                    parent,
                    tr("devices.button.poll"),
                    Action::DevicesPoll,
                    "DevicePoll",
                );
            }
            button(
                parent,
                &recovery_field(state),
                Action::DevicesRecoveryEdit,
                "DeviceRecoveryField",
            );
            if !state.busy {
                button(
                    parent,
                    tr("devices.button.recover"),
                    Action::DevicesRecover,
                    "DeviceRecover",
                );
            }
        } else if matches!(status.state.as_str(), "approved" | "consumed") {
            label(
                parent,
                trf(
                    "devices.use_account",
                    &[(
                        "name",
                        &status
                            .nickname
                            .as_deref()
                            .unwrap_or(tr("devices.confirmed_account")),
                    )],
                ),
                22.,
                ui::GOLD,
                "DeviceAccountConfirm",
            );
            if !state.busy && !state.restart_required {
                button(
                    parent,
                    tr("devices.button.confirm"),
                    Action::DevicesConfirm,
                    "DeviceConfirm",
                );
            }
        }
    }
    if state.enrollment.is_some() && state.status.is_none() && !state.busy {
        button(
            parent,
            tr("devices.button.poll"),
            Action::DevicesPoll,
            "DevicePoll",
        );
    }
    // Three equal ways in (LR-68 rules): the website (VK ID) where the region
    // signs in with it, a saved recovery code, another device.
    if !state.busy && !state.restart_required {
        if vk_button_visible(crate::account_region::features()) {
            button(
                parent,
                tr("devices.button.vk"),
                Action::SocialStart,
                "DeviceVkSignIn",
            );
        }
        button(
            parent,
            tr("protect.button.enter_code"),
            Action::DevicesRecoveryStart,
            "DeviceRecoveryStart",
        );
        button(
            parent,
            tr("devices.button.start"),
            Action::DevicesStart,
            "DeviceStart",
        );
    }
    if state.busy {
        label(parent, tr("devices.busy"), 15., ui::MUTED, "DeviceBusy");
    }
    label(
        parent,
        state.message.clone().unwrap_or_default(),
        15.,
        ui::GOLD,
        "DeviceMessage",
    );
    label(
        parent,
        career.form_error.clone().unwrap_or_default(),
        15.,
        ui::GOLD,
        "DeviceInputError",
    );
}
pub(super) fn recovery_field(state: &DeviceState) -> String {
    let value = if state.recovery_code.is_empty() {
        tr("devices.recovery.empty").to_owned()
    } else {
        trf(
            "devices.recovery.count",
            &[("count", &state.recovery_code.len())],
        )
    };
    format!(
        "{}{}",
        trf("devices.recovery.field", &[("value", &value)]),
        if state.focused { " |" } else { "" }
    )
}

#[cfg(test)]
mod social_tests {
    use super::*;

    fn status(state: &str, code: Option<&str>, profile: Option<&str>) -> DeviceEnrollmentStatus {
        DeviceEnrollmentStatus {
            account_region: None,
            state: state.into(),
            code: code.map(str::to_owned),
            profile_id: profile.map(str::to_owned),
            nickname: None,
            social: None,
        }
    }

    fn proof(action: DeviceAction) -> SignedDeviceEnrollment {
        SignedDeviceEnrollment {
            enrollment: DeviceEnrollment {
                enrollment_id: "a".repeat(64),
                public_key: "b".repeat(64),
                origin: "https://omoba.ru".into(),
                label: "Desktop".into(), // i18n-allow
                expires_at: (now() + 240).to_string(),
            },
            action,
            target_profile: (action == DeviceAction::Complete).then(|| "c".repeat(64)),
            recovery_code: None,
            signature: "d".repeat(128),
        }
    }

    fn poll_app(proof: &SignedDeviceEnrollment) -> App {
        let mut app = App::new();
        app.insert_resource(CareerClient {
            devices: DeviceState {
                enrollment: Some(proof.enrollment.clone()),
                status: Some(status(
                    if proof.action == DeviceAction::Complete {
                        "approved"
                    } else {
                        "pending"
                    },
                    Some("ABCD2345"), // i18n-allow
                    proof.target_profile.as_deref(),
                )),
                social: true,
                browser_opened: true,
                ..default()
            },
            ..default()
        })
        .init_resource::<Worker>()
        .init_resource::<CareerIdentity>()
        .add_systems(Update, poll);
        app
    }

    fn deliver(
        app: &mut App,
        proof: &SignedDeviceEnrollment,
        reply: Result<DeviceEnrollmentStatus, String>,
    ) {
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send(reply).unwrap();
        {
            let mut worker = app.world_mut().resource_mut::<Worker>();
            worker.pending = Some(Mutex::new(rx));
            worker.retry = Some(proof.clone());
            worker.activating = proof.target_profile.clone();
        }
        app.world_mut().resource_mut::<CareerClient>().devices.busy = true;
        app.update();
    }

    #[test]
    fn failed_social_status_keeps_the_enrollment_and_recovers_approval() {
        let proof = proof(DeviceAction::Status);
        let mut app = poll_app(&proof);
        deliver(&mut app, &proof, Err("temporary failure".into())); // i18n-allow
        let worker = app.world().resource::<Worker>();
        assert!(worker.next_poll.is_some());
        assert_eq!(worker.retry_attempts, 1);
        assert_eq!(
            serde_json::to_value(worker.retry.as_ref().unwrap()).unwrap(),
            serde_json::to_value(&proof).unwrap(),
            "retry the same proof, candidate key and enrollment"
        );
        let state = &app.world().resource::<CareerClient>().devices;
        assert_eq!(state.enrollment.as_ref(), Some(&proof.enrollment));
        assert_eq!(state.status.as_ref().unwrap().state, "pending");
        assert!(!state.busy && !state.restart_required);

        let target = "c".repeat(64);
        deliver(
            &mut app,
            &proof,
            Ok(status("approved", None, Some(&target))),
        );
        let worker = app.world().resource::<Worker>();
        assert!(worker.retry.is_none() && worker.next_poll.is_none());
        let state = &app.world().resource::<CareerClient>().devices;
        assert_eq!(state.enrollment.as_ref(), Some(&proof.enrollment));
        assert_eq!(state.status.as_ref().unwrap().profile_id, Some(target));
        assert!(!state.restart_required, "approval still needs confirmation");
    }

    #[test]
    fn completion_retries_are_bounded_and_retain_the_confirmed_account() {
        let proof = proof(DeviceAction::Complete);
        let mut app = poll_app(&proof);
        for attempt in 1..=AUTOMATIC_RETRIES + 1 {
            deliver(&mut app, &proof, Err("temporary failure".into())); // i18n-allow
            let worker = app.world().resource::<Worker>();
            assert_eq!(worker.activating, proof.target_profile);
            assert_eq!(worker.retry_attempts, attempt.min(AUTOMATIC_RETRIES));
            assert_eq!(worker.next_poll.is_some(), attempt <= AUTOMATIC_RETRIES);
            assert_eq!(worker.retry.as_ref().unwrap().enrollment, proof.enrollment);
            assert_eq!(
                app.world()
                    .resource::<CareerClient>()
                    .devices
                    .status
                    .as_ref()
                    .unwrap()
                    .state,
                "approved",
                "keep the confirmation visible after a failed completion"
            );
        }
        assert_eq!(retry_delay(1), Duration::from_secs(3));
        assert_eq!(retry_delay(2), Duration::from_secs(6));
        assert_eq!(retry_delay(3), Duration::from_secs(12));
        assert_eq!(retry_delay(4), Duration::from_secs(24));
        assert_eq!(retry_delay(u8::MAX), Duration::from_secs(24));
    }

    #[test]
    fn unexpected_completion_cannot_replace_the_approved_account_or_stage_a_key() {
        let proof = proof(DeviceAction::Complete);
        for invalid in [
            status("pending", None, proof.target_profile.as_deref()),
            status("approved", None, proof.target_profile.as_deref()),
            status("consumed", None, Some(&"e".repeat(64))),
            status("consumed", None, None),
        ] {
            let mut app = poll_app(&proof);
            deliver(&mut app, &proof, Ok(invalid));
            let state = &app.world().resource::<CareerClient>().devices;
            assert!(!state.restart_required);
            assert_eq!(state.status.as_ref().unwrap().state, "approved");
            assert_eq!(
                state.status.as_ref().unwrap().profile_id,
                proof.target_profile
            );
            assert_eq!(
                state.message.as_deref(),
                Some(tr("devices.error.completion")),
                "reject the reply before attempting to access a staged key"
            );
        }
        assert!(completion_matches(
            &status("consumed", None, proof.target_profile.as_deref()),
            proof.target_profile.as_deref().unwrap()
        ));
    }

    #[test]
    fn an_expired_retry_never_launches_transport_or_activates_the_candidate() {
        for action in [
            DeviceAction::Create,
            DeviceAction::Status,
            DeviceAction::Complete,
        ] {
            let mut proof = proof(action);
            proof.enrollment.expires_at = now().saturating_sub(1).to_string();
            let mut app = poll_app(&proof);
            {
                let mut worker = app.world_mut().resource_mut::<Worker>();
                worker.next_poll = Some(Instant::now() - Duration::from_secs(1));
                worker.retry = Some(proof.clone());
                worker.activating = proof.target_profile.clone();
            }
            app.update();
            let worker = app.world().resource::<Worker>();
            assert!(worker.pending.is_none() && worker.next_poll.is_none());
            assert!(worker.retry.is_none() && worker.activating.is_none());
            let state = &app.world().resource::<CareerClient>().devices;
            assert!(state.enrollment.is_none());
            assert!(!state.restart_required);
            assert_eq!(state.message.as_deref(), Some(tr("devices.vk.expired")));
        }
    }

    #[test]
    fn pending_social_and_lost_create_replies_offer_a_manual_check() {
        let proof = proof(DeviceAction::Create);
        let mut app = poll_app(&proof);
        app.insert_resource(crate::ui::UiPlatform(UiProfile::Desktop))
            .add_systems(Update, render);
        app.world_mut().resource_mut::<CareerClient>().modal = CareerModal::Devices;
        for has_status in [true, false] {
            if !has_status {
                app.world_mut()
                    .resource_mut::<CareerClient>()
                    .devices
                    .status = None;
            }
            app.update();
            assert!(
                app.world_mut()
                    .query::<crate::ui::test_id::NodeKey>()
                    .iter(app.world())
                    .any(|key| key.as_str() == "DevicePoll"),
                "the same request remains checkable without creating another key"
            );
        }
    }

    #[test]
    fn vk_button_follows_the_market_feature() {
        use shared::market::Market;
        assert!(vk_button_visible(Market::Ru.default_features()));
        assert!(!vk_button_visible(Market::Global.default_features()));
        assert!(!vk_button_visible(Market::Cn.default_features()));
    }

    /// LR-18 and the LR-68 rules: a saved recovery code is its own way in,
    /// next to VK ID (where the region has it) and another device; a request
    /// started from it asks for the code, not for the portal.
    #[test]
    fn a_recovery_code_is_a_first_class_way_in() {
        let mut app = App::new();
        app.insert_resource(crate::ui::UiPlatform(UiProfile::Desktop))
            .insert_resource(CareerClient {
                modal: CareerModal::Devices,
                ..default()
            })
            .add_systems(Update, render);
        app.update();
        let ids = |app: &mut App| -> Vec<String> {
            app.world_mut()
                .query::<crate::ui::test_id::NodeKey>()
                .iter(app.world())
                .map(|key| key.as_str().to_owned())
                .collect()
        };
        let options = ids(&mut app);
        for id in ["DeviceRecoveryStart", "DeviceStart"] {
            assert!(options.iter().any(|found| found == id), "{id}: {options:?}");
        }
        assert_eq!(
            options.iter().any(|found| found == "DeviceVkSignIn"),
            vk_button_visible(crate::account_region::features())
        );
        {
            let mut career = app.world_mut().resource_mut::<CareerClient>();
            career.devices.recovery_entry = true;
            career.devices.status = Some(status("pending", Some("ABCD2345"), None)); // i18n-allow
        }
        app.update();
        let asked = ids(&mut app);
        for id in [
            "DeviceRecoveryPrompt",
            "DeviceRecoveryField",
            "DeviceRecover",
        ] {
            assert!(asked.iter().any(|found| found == id), "{id}: {asked:?}");
        }
        for id in ["DeviceCode", "DevicePoll", "DevicePortalInstructions"] {
            assert!(!asked.iter().any(|found| found == id), "{id}: {asked:?}");
        }
    }

    #[test]
    fn a_vk_request_opens_the_browser_once_then_waits_for_the_player() {
        let mut state = DeviceState {
            social: true,
            ..default()
        };
        let first = status("pending", Some("ABCD2345"), None); // i18n-allow
        assert_eq!(social_next(&state, &first, None), SocialNext::OpenBrowser);
        state.browser_opened = true;
        assert_eq!(social_next(&state, &first, None), SocialNext::Wait);
        assert_eq!(
            social_next(&state, &status("social_pending", None, None), None),
            SocialNext::Stop
        );
        let mine = "a".repeat(64);
        assert_eq!(
            social_next(&state, &status("approved", None, Some(&mine)), Some(&mine)),
            SocialNext::SameAccount
        );
        assert_eq!(
            social_next(
                &state,
                &status("approved", None, Some(&"b".repeat(64))),
                Some(&mine)
            ),
            SocialNext::Stop
        );
        // An ordinary device request never opens a browser or polls on its own.
        let manual = DeviceState::default();
        assert_eq!(social_next(&manual, &first, None), SocialNext::Stop);
    }
}
