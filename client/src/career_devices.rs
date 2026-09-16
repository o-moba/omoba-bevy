//! Account transfer uses a fresh local key and explicit activation after confirmation.
use super::*;
use crate::career_identity::{CareerIdentity, hex, load_or_create_key};
use ed25519_dalek::{Signer, SigningKey};
use omoba_passport::device_account::DeviceAccountApi;
use shared::device_account::{
    DeviceAction, DeviceEnrollment, DeviceEnrollmentStatus, SignedDeviceEnrollment,
};
use std::sync::{Mutex, mpsc};

#[derive(Clone, Default, PartialEq)]
pub(super) struct DeviceState {
    pub enrollment: Option<DeviceEnrollment>,
    pub status: Option<DeviceEnrollmentStatus>,
    pub recovery_code: String,
    pub focused: bool,
    pub busy: bool,
    pub message: Option<String>,
    pub restart_required: bool,
}
#[derive(Resource, Default)]
pub(super) struct Worker {
    key: Option<SigningKey>,
    pending: Option<Mutex<mpsc::Receiver<Result<DeviceEnrollmentStatus, String>>>>,
    activating: bool,
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
pub(super) fn append(code: &mut String, text: &str) -> Result<(), &'static str> {
    let normalized = text.trim().to_ascii_lowercase().replace([' ', '-'], "");
    if !normalized.bytes().all(|b| b.is_ascii_hexdigit()) || code.len() + normalized.len() > 64 {
        return Err("Paste one complete 64-character recovery code.");
    }
    code.push_str(&normalized);
    Ok(())
}
fn launch(worker: &mut Worker, proof: SignedDeviceEnrollment) {
    let (tx, rx) = mpsc::sync_channel(1);
    worker.pending = Some(Mutex::new(rx));
    std::thread::spawn(move || {
        let _ = tx.send(DeviceAccountApi::from_env().and_then(|api| api.send(&proof)));
    });
}
fn sign(
    worker: &Worker,
    e: &DeviceEnrollment,
    action: DeviceAction,
    target: Option<String>,
    code: Option<String>,
) -> Result<SignedDeviceEnrollment, String> {
    let key = worker
        .key
        .as_ref()
        .ok_or("Start a new device request first.")?;
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
    if matches!(action, Action::DevicesOpen) {
        career.modal = CareerModal::Devices;
        career.nickname_focused = false;
        career.friend_code_focused = false;
        career.web.focused = false;
        career.preedit.clear();
        return;
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
        match action {
            Action::DevicesStart => {
                let api = DeviceAccountApi::from_env()?;
                let directory = identity.enrollment_directory()?;
                let mut key = load_or_create_key(&directory)?;
                if hex(key.verifying_key().as_bytes()) == identity.public_key()?
                    || career.devices.enrollment.is_some()
                {
                    // An explicit restart creates a fresh key, retaining any old candidate
                    // (including a completed enrollment whose response was lost).
                    let mut suffix = [0; 16];
                    getrandom::fill(&mut suffix).map_err(|_| "Secure randomness unavailable.")?;
                    let preserved =
                        directory.with_file_name(format!("enrolled-device-{}", hex(&suffix)));
                    std::fs::rename(&directory, preserved)
                        .map_err(|_| "Cannot preserve the previously linked device key.")?;
                    key = load_or_create_key(&directory)?;
                }
                let mut id = [0; 32];
                getrandom::fill(&mut id).map_err(|_| "Secure randomness unavailable.")?;
                let enrollment = DeviceEnrollment {
                    enrollment_id: hex(&id),
                    public_key: hex(key.verifying_key().as_bytes()),
                    origin: api.origin().into(),
                    label: if cfg!(target_os = "ios") {
                        "iPhone / iPad".into()
                    } else if cfg!(target_os = "android") {
                        "Android".into()
                    } else {
                        "Desktop".into()
                    },
                    expires_at: (now() + 300).to_string(),
                };
                worker.key = Some(key);
                let proof = sign(worker, &enrollment, DeviceAction::Create, None, None)?;
                career.devices.enrollment = Some(enrollment);
                career.devices.status = None;
                career.devices.focused = false;
                career.devices.recovery_code.clear();
                launch(worker, proof);
            }
            Action::DevicesPoll | Action::DevicesRecover | Action::DevicesConfirm => {
                let e = career
                    .devices
                    .enrollment
                    .as_ref()
                    .ok_or("Start a new device request first.")?;
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
                            .ok_or("Approve this device on the portal first.")?,
                    )
                } else {
                    None
                };
                if operation == DeviceAction::Complete
                    && career
                        .devices
                        .status
                        .as_ref()
                        .is_some_and(|s| s.state == "consumed")
                {
                    identity.stage_enrolled_identity(&e.public_key)?;
                    career.devices.restart_required = true;
                    career.devices.message=Some("Account saved. Restart the game to use it. Your previous account key is preserved on this device.".into());
                    return Ok(());
                }
                let code = if operation == DeviceAction::Recover {
                    if career.devices.recovery_code.len() != 64 {
                        return Err("Paste one complete recovery code.".into());
                    }
                    Some(career.devices.recovery_code.clone())
                } else {
                    None
                };
                let proof = sign(worker, e, operation, target, code)?;
                worker.activating = operation == DeviceAction::Complete;
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
    let result = worker
        .pending
        .as_ref()
        .and_then(|r| r.lock().ok().map(|rx| rx.try_recv()));
    let value = match result {
        Some(Ok(r)) => r,
        Some(Err(mpsc::TryRecvError::Disconnected)) => {
            Err("Device request stopped. Retry safely.".into())
        }
        _ => return,
    };
    worker.pending = None;
    career.devices.busy = false;
    match value {
        Ok(mut status) => {
            if status.code.is_none() {
                status.code = career.devices.status.as_ref().and_then(|s| s.code.clone());
            }
            career.devices.status = Some(status);
            if worker.activating {
                let result = career
                    .devices
                    .enrollment
                    .as_ref()
                    .ok_or_else(|| "Enrollment missing.".to_owned())
                    .and_then(|e| identity.stage_enrolled_identity(&e.public_key));
                match result {
                    Ok(()) => {
                        career.devices.restart_required = true;
                        career.devices.message=Some("Account saved. Restart the game to use it. The previous account was preserved; progress was not merged.".into());
                    }
                    Err(e) => career.devices.message = Some(e),
                }
            }
        }
        Err(error) => career.devices.message = Some(error),
    }
    worker.activating = false;
}
pub(super) fn body(parent: &mut ChildSpawnerCommands, career: &CareerClient) {
    let state = &career.devices;
    label(
        parent,
        "Use an existing account",
        26.,
        ui::GOLD,
        "DeviceTitle",
    );
    label(
        parent,
        "Link this installation from your signed-in player portal, or use a saved recovery code. You will review the account before switching. Existing progress stays in its original account.",
        16.,
        ui::IVORY,
        "DeviceInstructions",
    );
    if let Some(e) = &state.enrollment {
        label(
            parent,
            format!(
                "Trusted portal: {}\nNew device fingerprint: {}",
                e.origin,
                format!("{}…{}", &e.public_key[..8], &e.public_key[56..])
            ),
            14.,
            ui::MUTED,
            "DeviceFingerprint",
        );
    }
    if let Some(status) = &state.status {
        if status.state == "pending" {
            if let Some(code) = &status.code {
                label(
                    parent,
                    format!("Device code: {code}"),
                    28.,
                    ui::GOLD,
                    "DeviceCode",
                );
            }
            label(
                parent,
                "On your existing account: Portal → Devices → Link device. Check that the fingerprints match. The request expires in five minutes.",
                15.,
                ui::IVORY,
                "DevicePortalInstructions",
            );
            if !state.busy {
                button(parent, "Check approval", Action::DevicesPoll, "DevicePoll");
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
                    "Recover with this code",
                    Action::DevicesRecover,
                    "DeviceRecover",
                );
            }
        } else if matches!(status.state.as_str(), "approved" | "consumed") {
            label(
                parent,
                format!(
                    "Use account: {}",
                    status.nickname.as_deref().unwrap_or("Confirmed account")
                ),
                22.,
                ui::GOLD,
                "DeviceAccountConfirm",
            );
            if !state.busy && !state.restart_required {
                button(
                    parent,
                    "Use this account after restart",
                    Action::DevicesConfirm,
                    "DeviceConfirm",
                );
            }
        }
    }
    if !state.busy && !state.restart_required {
        button(
            parent,
            "Start new device request",
            Action::DevicesStart,
            "DeviceStart",
        );
    }
    if state.busy {
        label(parent, "Connecting…", 15., ui::MUTED, "DeviceBusy");
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
    format!(
        "Recovery code: {}{}",
        if state.recovery_code.is_empty() {
            "paste your saved code".into()
        } else {
            format!("{} / 64 characters", state.recovery_code.len())
        },
        if state.focused { " |" } else { "" }
    )
}
