//! Bounded native account enrollment transport. HTTPS policy matches web pairing.
use crate::web_account::WebAccountApi;
use shared::device_account::{DeviceAction, DeviceEnrollmentStatus, SignedDeviceEnrollment};

pub struct DeviceAccountApi {
    inner: WebAccountApi,
}
impl DeviceAccountApi {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            inner: WebAccountApi::from_env()?,
        })
    }
    pub fn origin(&self) -> &str {
        &self.inner.origin
    }
    pub fn send(&self, proof: &SignedDeviceEnrollment) -> Result<DeviceEnrollmentStatus, String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Invalid system clock.")?
            .as_secs();
        proof
            .enrollment
            .validate(self.origin(), &proof.enrollment.public_key, now)
            .map_err(str::to_owned)?;
        let path = match proof.action {
            DeviceAction::Create => "v1/auth/devices/create",
            DeviceAction::Status => "v1/auth/devices/status",
            DeviceAction::Recover => "v1/auth/devices/recover",
            DeviceAction::Complete => "v1/auth/devices/complete",
        };
        self.inner.request(
            path,
            serde_json::to_value(proof).map_err(|_| "Invalid device request.")?,
        )
    }
}
