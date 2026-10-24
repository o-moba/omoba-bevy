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
    /// `state: social_pending`: a website sign-in (VK ID) that is not linked to any
    /// game profile yet waits for this game to link it ([`SignedSocialLink`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub social: Option<SocialPending>,
}

/// The unlinked website sign-in offered to a pending enrollment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SocialPending {
    /// `vk` (VK ID: VK, OK or Mail.ru).
    pub provider: String,
    /// A short label the player recognizes ("Иван П."), never an identifier.
    pub display_name: String,
}

/// The game's confirmation that links the website sign-in waiting on its pending
/// enrollment to the profile of its *current* (active) device key. Signed by
/// that key; binds the enrollment (id, pending key, expiry) the player started
/// in this game, so a sign-in can only be linked by the game that asked for it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SocialLink {
    pub enrollment_id: String,
    /// The pending enrollment's key (`DeviceEnrollment::public_key`).
    pub enrollment_key: String,
    /// The active device key whose profile receives the sign-in.
    pub public_key: String,
    pub origin: String,
    /// The enrollment's expiry (`DeviceEnrollment::expires_at`).
    pub expires_at: String,
}
impl SocialLink {
    pub fn for_enrollment(enrollment: &DeviceEnrollment, public_key: String) -> Self {
        Self {
            enrollment_id: enrollment.enrollment_id.clone(),
            enrollment_key: enrollment.public_key.clone(),
            public_key,
            origin: enrollment.origin.clone(),
            expires_at: enrollment.expires_at.clone(),
        }
    }
    pub fn validate(&self, origin: &str, now: u64) -> Result<(), &'static str> {
        if self.origin != origin
            || !lowercase_hex(&self.enrollment_id, 32)
            || !lowercase_hex(&self.enrollment_key, 32)
            || !lowercase_hex(&self.public_key, 32)
            || self.public_key == self.enrollment_key
        {
            return Err("Invalid sign-in link or trusted portal.");
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
    pub fn signing_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&(
            "omoba.social.link.v1",
            &self.origin,
            &self.enrollment_id,
            &self.enrollment_key,
            &self.public_key,
            &self.expires_at,
        ))
        .expect("typed social link tuple")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedSocialLink {
    pub link: SocialLink,
    pub signature: String,
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

    #[test]
    fn social_link_binds_the_enrollment_and_a_different_active_key() {
        let e = enrollment();
        let link = SocialLink::for_enrollment(&e, "c".repeat(64));
        assert!(link.validate(&e.origin, 1000).is_ok());
        assert!(link.validate("https://evil.example", 1000).is_err());
        assert!(link.validate(&e.origin, 1200).is_err());
        let same_key = SocialLink::for_enrollment(&e, e.public_key.clone());
        assert!(same_key.validate(&e.origin, 1000).is_err());
        let mut other = link.clone();
        other.enrollment_id = "d".repeat(64);
        assert_ne!(link.signing_bytes(), other.signing_bytes());
        // A link proof can never verify as an enrollment proof.
        assert_ne!(
            link.signing_bytes(),
            e.signing_bytes(DeviceAction::Status, None, None)
        );
    }

    #[test]
    fn older_status_replies_without_social_still_parse() {
        let status: DeviceEnrollmentStatus = serde_json::from_str(
            r#"{"state":"pending","code":"ABCD2345","profile_id":null,"nickname":null}"#,
        )
        .unwrap();
        assert_eq!(status.social, None);
        let pending: DeviceEnrollmentStatus = serde_json::from_str(
            r#"{"state":"social_pending","code":null,"profile_id":null,"nickname":null,"social":{"provider":"vk","display_name":"Иван П."}}"#,
        )
        .unwrap();
        assert_eq!(pending.social.unwrap().display_name, "Иван П.");
    }
}
