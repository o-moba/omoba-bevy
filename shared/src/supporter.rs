//! Cosmetic-only Supporter contract. Status is issued by the authoritative server.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuraStyle {
    Solar,
    Lunar,
    Verdant,
}
impl AuraStyle {
    pub const ALL: [Self; 3] = [Self::Solar, Self::Lunar, Self::Verdant];
    pub const fn id(self) -> &'static str {
        match self {
            Self::Solar => "solar",
            Self::Lunar => "lunar",
            Self::Verdant => "verdant",
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Solar => "Solar",
            Self::Lunar => "Lunar",
            Self::Verdant => "Verdant",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|style| style.id() == id)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupporterGrantSummary {
    pub provider: String,
    pub valid_from: i64,
    pub valid_until: i64,
    pub revoked: bool,
    /// False means no future renewal, not immediate loss of paid access.
    pub renewal_enabled: Option<bool>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupporterStatus {
    pub active: bool,
    pub equipped_aura: Option<AuraStyle>,
    /// UTC epoch seconds, never a client entitlement assertion.
    pub active_until: Option<i64>,
    pub grants: Vec<SupporterGrantSummary>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeSupporterAction {
    Status,
    Equip { aura: Option<AuraStyle> },
    ApplePrepare,
    AppleVerify { signed_payload: String },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NativeSupporterRequest {
    pub origin: String,
    pub public_key: String,
    pub nonce: String,
    pub expires_at: u64,
    pub action: NativeSupporterAction,
}
impl NativeSupporterRequest {
    pub fn validate(&self, origin: &str, key: &str, now: u64) -> Result<(), &'static str> {
        if self.origin != origin
            || self.public_key != key
            || !crate::web_account::lowercase_hex(key, 32)
            || !crate::web_account::lowercase_hex(&self.nonce, 32)
            || self.expires_at <= now
            || self.expires_at > now.saturating_add(60)
        {
            return Err("Invalid or expired supporter request");
        }
        if let NativeSupporterAction::AppleVerify { signed_payload } = &self.action {
            if signed_payload.len() > 32768 || signed_payload.is_empty() {
                return Err("Invalid Apple payload");
            }
        }
        Ok(())
    }
    pub fn signing_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&(
            "omoba.supporter.native.v1",
            &self.origin,
            &self.public_key,
            &self.nonce,
            self.expires_at,
            &self.action,
        ))
        .expect("typed supporter request")
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedNativeSupporterRequest {
    pub request: NativeSupporterRequest,
    pub signature: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proof_binds_every_operation_and_never_accepts_unknown_auras() {
        let mut r = NativeSupporterRequest {
            origin: "https://portal.example".into(),
            public_key: "a".repeat(64),
            nonce: "b".repeat(64),
            expires_at: 160,
            action: NativeSupporterAction::Status,
        };
        assert!(r.validate(&r.origin, &r.public_key, 100).is_ok());
        assert!(r.validate(&r.origin, &r.public_key, 99).is_err());
        assert!(r.validate(&r.origin, &r.public_key, 160).is_err());
        let bytes = r.signing_bytes();
        r.action = NativeSupporterAction::ApplePrepare;
        assert_ne!(bytes, r.signing_bytes());
        assert!(serde_json::from_str::<AuraStyle>("\"admin\"").is_err());
    }
}
