//! Typed native billing transport. It never accepts a client entitlement assertion.
use crate::web_account::WebAccountApi;
use shared::supporter::SignedNativeSupporterRequest;

pub fn send(
    api: &WebAccountApi,
    proof: &SignedNativeSupporterRequest,
) -> Result<serde_json::Value, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "Invalid system clock.")?
        .as_secs();
    proof
        .request
        .validate(&api.origin, &proof.request.public_key, now)
        .map_err(str::to_owned)?;
    api.request(
        "v1/supporter/native",
        serde_json::to_value(proof).map_err(|_| "Invalid Supporter request.")?,
    )
}
