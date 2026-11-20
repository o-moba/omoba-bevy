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

/// The region portal's Supporter page (`<portal>/supporter`), where website
/// checkouts (YooKassa donations, LR-67) happen. The origin is the build's
/// validated `OMOBA_PORTAL_ORIGIN`, never a literal.
pub fn website_url(api: &WebAccountApi) -> String {
    page_url(&api.origin)
}

fn page_url(origin: &str) -> String {
    format!("{origin}/supporter")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_website_page_hangs_off_the_trusted_portal_origin() {
        assert_eq!(
            super::page_url("https://portal.example"),
            "https://portal.example/supporter"
        );
        let api = crate::web_account::WebAccountApi::new(
            "https://account.example",
            "https://portal.example",
            false,
        )
        .unwrap();
        assert_eq!(super::website_url(&api), "https://portal.example/supporter");
    }
}
