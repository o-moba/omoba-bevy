//! Bounded HTTPS transport for confirming an existing game account in the portal.
use reqwest::{Url, blocking::Client, redirect::Policy};
use serde::Deserialize;
use shared::web_account::{SignedWebPair, WebPairChallenge, WebPairDecision};
use std::{io::Read, time::Duration};
#[derive(Clone)]
pub struct WebAccountApi {
    base: Url,
    pub origin: String,
    client: Client,
}
impl WebAccountApi {
    pub fn new(base: &str, origin: &str, insecure_local: bool) -> Result<Self, String> {
        let base = trusted_url(base, insecure_local)?;
        let portal = trusted_url(origin, insecure_local)?;
        if portal.as_str().trim_end_matches('/') != origin {
            return Err("Use the exact portal origin without a trailing slash.".into());
        }
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| "Cannot initialize website transport.")?;
        Ok(Self {
            base,
            origin: origin.to_owned(),
            client,
        })
    }
    pub fn from_env() -> Result<Self, String> {
        let base = std::env::var("OMOBA_ACCOUNT_API_URL")
            .ok()
            .or_else(|| option_env!("OMOBA_ACCOUNT_API_URL").map(str::to_owned))
            .ok_or("Website account service is not configured in this build.")?;
        let origin = std::env::var("OMOBA_PORTAL_ORIGIN")
            .ok()
            .or_else(|| option_env!("OMOBA_PORTAL_ORIGIN").map(str::to_owned))
            .ok_or("Trusted player portal is not configured in this build.")?;
        Self::new(
            &base,
            &origin,
            std::env::var("OMOBA_ALLOW_INSECURE_LOCAL").as_deref() == Ok("1"),
        )
    }
    pub(crate) fn request<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T, String> {
        let response = self
            .client
            .post(
                self.base
                    .join(path)
                    .map_err(|_| "Invalid website endpoint.")?,
            )
            .json(&body)
            .send()
            .map_err(|_| "Website service is unavailable. Try again.")?;
        let status = response.status();
        let mut bytes = Vec::new();
        // The Supporter response includes a bounded provider-period history.
        // Pairing/device replies are smaller; all endpoints remain hard-bounded.
        response
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| "Website response was interrupted.")?;
        if bytes.len() > 65536 {
            return Err("Website response is too large.".into());
        }
        if !status.is_success() {
            return Err(match status.as_u16() {
                404 => "Request expired, cancelled, or account not available. Start a new request.",
                409 => "This request was already used or changed. Refresh before trying again.",
                429 => "Too many requests. Wait a minute before retrying.",
                _ => "Website confirmation failed. Try again.",
            }
            .into());
        }
        serde_json::from_slice(&bytes).map_err(|_| "Invalid website response.".into())
    }
    pub fn lookup(&self, code: &str, public_key: &str) -> Result<WebPairChallenge, String> {
        #[derive(Deserialize)]
        struct Response {
            challenge: WebPairChallenge,
        }
        let response: Response = self.request(
            "v1/auth/pairings/lookup",
            serde_json::json!({"code":code,"public_key":public_key}),
        )?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Invalid system clock.")?
            .as_secs();
        response
            .challenge
            .validate(&self.origin, public_key, now)
            .map_err(str::to_owned)?;
        Ok(response.challenge)
    }
    pub fn decide(&self, proof: &SignedWebPair) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Invalid clock.")?
            .as_secs();
        proof
            .challenge
            .validate(&self.origin, &proof.challenge.public_key, now)
            .map_err(str::to_owned)?;
        let path = if proof.decision == WebPairDecision::Approve {
            "v1/auth/pairings/approve"
        } else {
            "v1/auth/pairings/deny"
        };
        let _: serde_json::Value = self.request(
            path,
            serde_json::to_value(proof).map_err(|_| "Invalid confirmation.")?,
        )?;
        Ok(())
    }
}
fn trusted_url(raw: &str, insecure: bool) -> Result<Url, String> {
    let u = Url::parse(raw).map_err(|_| "Invalid website address.")?;
    let local = matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if !(u.scheme() == "https" || insecure && local && u.scheme() == "http")
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || u.path() != "/"
    {
        return Err("Use a trusted HTTPS website origin. Local HTTP requires explicit development configuration.".into());
    }
    Ok(u)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_untrusted_urls() {
        for u in [
            "http://example.com",
            "https://user:pass@example.com",
            "https://example.com/path",
            "https://example.com?x=1",
        ] {
            assert!(trusted_url(u, true).is_err());
        }
        assert!(trusted_url("http://127.0.0.1:40550", true).is_ok());
        assert!(trusted_url("http://127.0.0.1:40550", false).is_err());
    }
}
