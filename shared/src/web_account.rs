//! Browser pairing contract. It deliberately cannot sign career/game requests.
use serde::{Deserialize, Serialize};

pub const PAIR_LIFETIME_SECS: u64 = 300;
pub const WEB_SCOPES: [&str; 7] = [
    "career:read",
    "friends:write",
    "nickname:write",
    "settings:write",
    "devices:write",
    "recovery:write",
    "supporter:write",
];

pub fn lowercase_hex(value: &str, bytes: usize) -> bool {
    value.len() == bytes * 2
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WebPairChallenge {
    pub pair_id: String,
    pub nonce: String,
    pub origin: String,
    pub public_key: String,
    pub browser_binding: String,
    pub scopes: Vec<String>,
    /// Decimal string: identical signing bytes across native/web platforms.
    pub expires_at: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebPairDecision {
    Approve,
    Deny,
}

impl WebPairChallenge {
    pub fn validate(
        &self,
        trusted_origin: &str,
        public_key: &str,
        now_secs: u64,
    ) -> Result<(), &'static str> {
        if self.origin != trusted_origin
            || self.public_key != public_key
            || !lowercase_hex(&self.pair_id, 32)
            || !lowercase_hex(&self.nonce, 32)
            || !lowercase_hex(&self.browser_binding, 32)
            || !lowercase_hex(&self.public_key, 32)
            || self.scopes != WEB_SCOPES
        {
            return Err("The website confirmation does not match this account or trusted portal.");
        }
        let expiry = self
            .expires_at
            .parse::<u64>()
            .map_err(|_| "Invalid website confirmation expiry.")?;
        if expiry.to_string() != self.expires_at
            || expiry <= now_secs
            || expiry > now_secs.saturating_add(PAIR_LIFETIME_SECS)
        {
            return Err("The website confirmation has expired or has an invalid lifetime.");
        }
        Ok(())
    }

    pub fn signing_bytes(&self, decision: WebPairDecision) -> Vec<u8> {
        serde_json::to_vec(&(
            "omoba.web.pair.v1",
            &self.origin,
            &self.pair_id,
            &self.nonce,
            &self.public_key,
            &self.browser_binding,
            decision,
            &self.scopes,
            &self.expires_at,
        ))
        .expect("typed string tuple serializes")
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedWebPair {
    pub challenge: WebPairChallenge,
    pub decision: WebPairDecision,
    pub signature: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn challenge() -> WebPairChallenge {
        WebPairChallenge {
            pair_id: "a".repeat(64),
            nonce: "b".repeat(64),
            origin: "https://players.example".into(),
            public_key: "c".repeat(64),
            browser_binding: "d".repeat(64),
            scopes: WEB_SCOPES.map(str::to_owned).to_vec(),
            expires_at: "1200".into(),
        }
    }
    #[test]
    fn expiry_scope_identity_and_audience_are_bound() {
        let c = challenge();
        assert!(c.validate(&c.origin, &c.public_key, 1000).is_ok());
        assert!(
            c.validate("https://attacker.example", &c.public_key, 1000)
                .is_err()
        );
        assert!(c.validate(&c.origin, &"e".repeat(64), 1000).is_err());
        assert!(c.validate(&c.origin, &c.public_key, 1200).is_err());
        assert!(c.validate(&c.origin, &c.public_key, 899).is_err());
        let mut legacy = c.clone();
        legacy.scopes.truncate(4);
        assert!(legacy.validate(&c.origin, &c.public_key, 1000).is_err());
        let mut extra = c.clone();
        extra.scopes.push("keys:write".into());
        assert!(extra.validate(&c.origin, &c.public_key, 1000).is_err());
    }
    #[test]
    fn golden_signing_tuple_and_decision_separation() {
        let c = challenge();
        let expected = format!(
            r#"["omoba.web.pair.v1","https://players.example","{}","{}","{}","{}","approve",["career:read","friends:write","nickname:write","settings:write","devices:write","recovery:write","supporter:write"],"1200"]"#,
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64)
        );
        assert_eq!(
            c.signing_bytes(WebPairDecision::Approve),
            expected.as_bytes()
        );
        assert_ne!(
            c.signing_bytes(WebPairDecision::Approve),
            c.signing_bytes(WebPairDecision::Deny)
        );
        let mut other = c.clone();
        other.browser_binding = "e".repeat(64);
        assert_ne!(
            c.signing_bytes(WebPairDecision::Approve),
            other.signing_bytes(WebPairDecision::Approve)
        );
    }
}
