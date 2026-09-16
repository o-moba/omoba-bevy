//! StoreKit payment lifecycle. Only a committed server verification finishes a transaction.
use bevy::prelude::*;

pub(crate) struct SupporterStoreKitPlugin;
impl Plugin for SupporterStoreKitPlugin {
    fn build(&self, _app: &mut App) {
        #[cfg(target_os = "ios")]
        _app.init_resource::<ios::Bridge>()
            .add_systems(Update, ios::tick);
    }
}

#[cfg(target_os = "ios")]
mod ios {
    use super::*;
    use crate::career_identity::CareerIdentity;
    use crate::supporter::{SupporterPlatformAction, SupporterPlatformState, SupporterUiState};
    use omoba_passport::{supporter_account, web_account::WebAccountApi};
    use serde_json::{Value, json};
    use shared::supporter::NativeSupporterAction;
    use std::{
        collections::VecDeque,
        ffi::{CString, c_char},
        sync::{Mutex, mpsc},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    unsafe extern "C" {
        fn omoba_storekit_request(value: *const c_char);
        fn omoba_storekit_poll(buffer: *mut c_char, capacity: i32) -> i32;
    }
    fn command(value: Value) {
        if let Ok(value) = CString::new(value.to_string()) {
            // The Swift entry copies the string synchronously before scheduling its task.
            unsafe { omoba_storekit_request(value.as_ptr()) };
        }
    }
    fn event() -> Option<Value> {
        let mut buffer = vec![0u8; 65536];
        let length =
            unsafe { omoba_storekit_poll(buffer.as_mut_ptr().cast(), buffer.len() as i32) };
        if length <= 0 || length as usize >= buffer.len() {
            return None;
        }
        serde_json::from_slice(&buffer[..length as usize]).ok()
    }
    enum Job {
        Prepare,
        Verify(String),
    }
    struct Receipt {
        id: String,
        payload: String,
        retry_at: Instant,
    }
    #[derive(Resource)]
    pub(super) struct Bridge {
        pending: Option<Mutex<mpsc::Receiver<(Job, Result<Value, String>)>>>,
        receipts: VecDeque<Receipt>,
        token: Option<String>,
        product_id: Option<String>,
        profile_key: Option<String>,
        retry_at: Instant,
    }
    impl Default for Bridge {
        fn default() -> Self {
            Self {
                pending: None,
                receipts: VecDeque::new(),
                token: None,
                product_id: None,
                profile_key: None,
                retry_at: Instant::now(),
            }
        }
    }
    fn launch(
        bridge: &mut Bridge,
        identity: &CareerIdentity,
        action: NativeSupporterAction,
        job: Job,
    ) -> Result<(), String> {
        let api = WebAccountApi::from_env()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "Invalid system clock.")?
            .as_secs();
        let proof = identity.sign_supporter_request(action, &api.origin, now)?;
        let (tx, rx) = mpsc::sync_channel(1);
        bridge.pending = Some(Mutex::new(rx));
        std::thread::spawn(move || {
            let _ = tx.send((job, supporter_account::send(&api, &proof)));
        });
        Ok(())
    }
    pub(super) fn tick(
        identity: Res<CareerIdentity>,
        ui: Res<SupporterUiState>,
        mut actions: MessageReader<SupporterPlatformAction>,
        mut platform: ResMut<SupporterPlatformState>,
        mut bridge: ResMut<Bridge>,
    ) {
        let key = identity.public_key().ok();
        if bridge.profile_key != key {
            // Native account activation requires a restart, so in-flight receipts cannot cross a switch.
            bridge.profile_key = key.clone();
            bridge.token = None;
            bridge.product_id = None;
            platform.available = false;
            platform.price_label = None;
        }
        let result = bridge
            .pending
            .as_ref()
            .and_then(|r| r.lock().ok().map(|rx| rx.try_recv()));
        if let Some(Ok((job, result))) = result {
            bridge.pending = None;
            platform.busy = false;
            match (job, result) {
                (Job::Prepare, Ok(value)) => {
                    if let (Some(token), Some(product)) = (
                        value["app_account_token"].as_str(),
                        value["product_id"].as_str(),
                    ) {
                        bridge.token = Some(token.to_owned());
                        bridge.product_id = Some(product.to_owned());
                        // Restore only needs the configured product/account, not a successful price lookup.
                        platform.available = true;
                        command(json!({"action":"configure","product_id":product}));
                        platform.message = Some("Loading App Store pricing…".into());
                    } else {
                        platform.message =
                            Some("Supporter purchases are not configured for this build.".into());
                    }
                }
                (Job::Verify(id), Ok(_)) => {
                    command(json!({"action":"finish","transaction_id":id}));
                    bridge.receipts.retain(|r| r.id != id);
                    platform.message =
                        Some("Purchase confirmed. Your Supporter status is syncing.".into());
                }
                (Job::Verify(id), Err(_)) => {
                    // Do not finish: Apple re-delivers pending transactions after restart.
                    platform.message = Some("Account service could not confirm the purchase. Check this game account and use Restore purchases; do not pay again.".into());
                    if let Some(receipt) = bridge.receipts.iter_mut().find(|r| r.id == id) {
                        receipt.retry_at = Instant::now() + Duration::from_secs(60);
                    }
                }
                (Job::Prepare, Err(_)) => {
                    platform.message = Some("App Store purchases are unavailable from the account service. You can still use free aura preview.".into());
                    bridge.retry_at = Instant::now() + Duration::from_secs(60);
                }
            }
        } else if matches!(result, Some(Err(mpsc::TryRecvError::Disconnected))) {
            bridge.pending = None;
            platform.busy = false;
            bridge.retry_at = Instant::now() + Duration::from_secs(60);
            platform.message = Some("Account request stopped. Try Restore purchases.".into());
        }
        for _ in 0..8 {
            let Some(value) = event() else {
                break;
            };
            match value["kind"].as_str() {
                Some("products") => {
                    platform.available = true;
                    platform.price_label = value["price_label"].as_str().map(str::to_owned);
                    platform.message = Some(
                        "Monthly subscription. Auto-renews until cancelled in Apple ID settings."
                            .into(),
                    );
                }
                Some("transaction") => {
                    if let (Some(id), Some(payload)) = (
                        value["transaction_id"].as_str(),
                        value["signed_payload"].as_str(),
                    ) {
                        if payload.len() <= 32768
                            && !bridge.receipts.iter().any(|r| r.id == id)
                            && bridge.receipts.len() < 16
                        {
                            bridge.receipts.push_back(Receipt {
                                id: id.to_owned(),
                                payload: payload.to_owned(),
                                retry_at: Instant::now(),
                            });
                        }
                    }
                }
                _ => {
                    platform.busy = false;
                    platform.message = value["message"].as_str().map(str::to_owned);
                }
            }
        }
        for action in actions.read() {
            if platform.busy {
                continue;
            }
            match action {
                SupporterPlatformAction::Purchase => {
                    if let Some(token) = &bridge.token {
                        if platform.available && platform.price_label.is_some() {
                            command(json!({"action":"purchase","app_account_token":token}));
                            platform.busy = true;
                        }
                    }
                }
                SupporterPlatformAction::Restore => {
                    if bridge.token.is_some() {
                        command(json!({"action":"restore"}));
                        platform.busy = true;
                        for receipt in &mut bridge.receipts {
                            receipt.retry_at = Instant::now();
                        }
                    }
                }
            }
        }
        if bridge.pending.is_none() && key.is_some() {
            let next = bridge
                .receipts
                .iter()
                .find(|r| Instant::now() >= r.retry_at)
                .map(|r| (r.id.clone(), r.payload.clone()));
            let result = if let Some((id, payload)) = next {
                Some(launch(
                    &mut bridge,
                    &identity,
                    NativeSupporterAction::AppleVerify {
                        signed_payload: payload,
                    },
                    Job::Verify(id),
                ))
            } else if ui.open && bridge.token.is_none() && Instant::now() >= bridge.retry_at {
                bridge.retry_at = Instant::now() + Duration::from_secs(60);
                Some(launch(
                    &mut bridge,
                    &identity,
                    NativeSupporterAction::ApplePrepare,
                    Job::Prepare,
                ))
            } else {
                None
            };
            if let Some(result) = result {
                match result {
                    Ok(()) => platform.busy = true,
                    Err(error) => {
                        platform.message = Some(error);
                        bridge.retry_at = Instant::now() + Duration::from_secs(60);
                    }
                }
            }
            if ui.open
                && bridge.token.is_some()
                && platform.price_label.is_none()
                && Instant::now() >= bridge.retry_at
            {
                if let Some(product) = &bridge.product_id {
                    command(json!({"action":"configure","product_id":product}));
                }
                bridge.retry_at = Instant::now() + Duration::from_secs(60);
            }
        }
    }
}
