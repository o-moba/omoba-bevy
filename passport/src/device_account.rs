//! Bounded native account enrollment transport. HTTPS policy matches web pairing.
use crate::web_account::WebAccountApi;
use shared::device_account::{
    AccountProtection, DeviceAction, DeviceEnrollmentStatus, SignedDeviceEnrollment,
    SignedNativeRecoveryRequest, SignedSocialLink,
};

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
        let reply: DeviceEnrollmentStatus = self.inner.request(
            path,
            serde_json::to_value(proof).map_err(|_| "Invalid device request.")?,
        )?;
        self.inner.validate_account_region(reply.account_region)?;
        Ok(reply)
    }
    /// Link the website sign-in waiting on an enrollment to this game's profile.
    pub fn link_social(&self, proof: &SignedSocialLink) -> Result<DeviceEnrollmentStatus, String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Invalid system clock.")?
            .as_secs();
        proof
            .link
            .validate(self.origin(), now)
            .map_err(str::to_owned)?;
        let reply: DeviceEnrollmentStatus = self.inner.request(
            "v1/auth/social/link",
            serde_json::to_value(proof).map_err(|_| "Invalid sign-in link.")?,
        )?;
        self.inner.validate_account_region(reply.account_region)?;
        Ok(reply)
    }
    /// This game's account protection, or a new set of recovery codes (LR-18),
    /// signed by its active key.
    pub fn recovery(
        &self,
        proof: &SignedNativeRecoveryRequest,
    ) -> Result<AccountProtection, String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Invalid system clock.")?
            .as_secs();
        proof
            .request
            .validate(self.origin(), &proof.request.public_key, now)
            .map_err(str::to_owned)?;
        self.inner.request(
            "v1/auth/devices/recovery-codes",
            serde_json::to_value(proof).map_err(|_| "Invalid account request.")?,
        )
    }
    /// The portal page that signs the player in for an enrollment code (`/connect`).
    pub fn sign_in_url(&self, code: &str) -> Result<String, String> {
        sign_in_url(self.origin(), code)
    }
}

/// `{portal}/connect?code=CODE` for an eight-character enrollment code.
pub fn sign_in_url(origin: &str, code: &str) -> Result<String, String> {
    if code.len() != 8
        || !code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return Err("Invalid device code.".into());
    }
    Ok(format!("{origin}/connect?code={code}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn sign_in_urls_carry_only_a_well_formed_code() {
        assert_eq!(
            super::sign_in_url("https://omoba.ru", "ABCD2345").unwrap(),
            "https://omoba.ru/connect?code=ABCD2345"
        );
        for bad in ["abcd2345", "ABCD234", "ABCD23456", "ABCD&x=1", "ABCD 234"] {
            assert!(
                super::sign_in_url("https://omoba.ru", bad).is_err(),
                "{bad}"
            );
        }
    }
}
