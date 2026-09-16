//! Native device enrollment is a separate authority from browser login.
use crate::web_account::lowercase_hex;
use serde::{Deserialize, Serialize};

pub const ENROLLMENT_SECONDS: u64 = 300;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceAction {
    Create,
    Status,
    Recover,
    Complete,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeviceEnrollment {
    pub enrollment_id: String,
    pub public_key: String,
    pub origin: String,
    pub label: String,
    pub expires_at: String,
}
impl DeviceEnrollment {
    pub fn validate(&self, origin: &str, public_key: &str, now: u64) -> Result<(), &'static str> {
        if self.origin != origin
            || self.public_key != public_key
            || !lowercase_hex(&self.enrollment_id, 32)
            || !lowercase_hex(&self.public_key, 32)
            || self.label.is_empty()
            || self.label.chars().count() > 40
            || self.label.chars().any(char::is_control)
        {
            return Err("Invalid device enrollment identity or trusted portal.");
        }
        let expiry = self
            .expires_at
            .parse::<u64>()
            .map_err(|_| "Invalid enrollment expiry.")?;
        if expiry.to_string() != self.expires_at
            || expiry <= now
            || expiry > now.saturating_add(ENROLLMENT_SECONDS)
        {
            return Err("Device enrollment expired. Start a new request.");
        }
        Ok(())
    }
    pub fn signing_bytes(
        &self,
        action: DeviceAction,
        target_profile: Option<&str>,
        recovery_code: Option<&str>,
    ) -> Vec<u8> {
        serde_json::to_vec(&(
            "omoba.device.enroll.v1",
            &self.origin,
            &self.enrollment_id,
            &self.public_key,
            &self.label,
            &self.expires_at,
            action,
            target_profile,
            recovery_code,
        ))
        .expect("typed enrollment tuple")
    }
}

// No Debug: recovery_code is a bearer secret. It never enters logs or game packets.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedDeviceEnrollment {
    pub enrollment: DeviceEnrollment,
    pub action: DeviceAction,
    pub target_profile: Option<String>,
    pub recovery_code: Option<String>,
    pub signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DeviceEnrollmentStatus {
    pub state: String,
    pub code: Option<String>,
    pub profile_id: Option<String>,
    pub nickname: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn enrollment() -> DeviceEnrollment {
        DeviceEnrollment {
            enrollment_id: "a".repeat(64),
            public_key: "b".repeat(64),
            origin: "https://players.example".into(),
            label: "iPhone".into(),
            expires_at: "1200".into(),
        }
    }
    #[test]
    fn proof_binds_origin_expiry_action_and_target() {
        let e = enrollment();
        assert!(e.validate(&e.origin, &e.public_key, 1000).is_ok());
        assert!(
            e.validate("https://evil.example", &e.public_key, 1000)
                .is_err()
        );
        assert!(e.validate(&e.origin, &e.public_key, 1200).is_err());
        assert!(e.validate(&e.origin, &e.public_key, 899).is_err());
        assert_ne!(
            e.signing_bytes(DeviceAction::Create, None, None),
            e.signing_bytes(DeviceAction::Complete, None, None)
        );
        assert_ne!(
            e.signing_bytes(DeviceAction::Complete, Some("a"), None),
            e.signing_bytes(DeviceAction::Complete, Some("b"), None)
        );
        assert_ne!(
            e.signing_bytes(DeviceAction::Recover, None, Some("a")),
            e.signing_bytes(DeviceAction::Recover, None, Some("b"))
        );
    }
}
