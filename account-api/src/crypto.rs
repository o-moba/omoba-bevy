//! Fixed-purpose authenticated cryptography; never log bearer material.
use ring::{aead, digest, hmac};

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn decode<const N: usize>(s: &str) -> Option<[u8; N]> {
    if !shared::web_account::lowercase_hex(s, N) {
        return None;
    }
    let mut out = [0; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}
pub fn random<const N: usize>() -> String {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).expect("OS randomness required");
    hex(&bytes)
}
pub fn hash(bytes: &[u8]) -> String {
    hex(digest::digest(&digest::SHA256, bytes).as_ref())
}
pub fn mac(key: &[u8; 32], scope: &str, value: &str) -> String {
    let message = serde_json::to_vec(&(scope, value)).expect("string tuple");
    hex(hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), &message).as_ref())
}
pub fn seal(key: &[u8; 32], associated: &str, text: &str) -> Result<Vec<u8>, &'static str> {
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::CHACHA20_POLY1305, key).map_err(|_| "crypto_unavailable")?,
    );
    let mut nonce = [0; 12];
    getrandom::fill(&mut nonce).map_err(|_| "crypto_unavailable")?;
    let mut data = text.as_bytes().to_vec();
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(associated),
        &mut data,
    )
    .map_err(|_| "crypto_unavailable")?;
    let mut out = nonce.to_vec();
    out.extend(data);
    Ok(out)
}
pub fn open(key: &[u8; 32], associated: &str, sealed: &[u8]) -> Result<String, &'static str> {
    if sealed.len() < 28 {
        return Err("invalid_delivery");
    }
    let nonce: [u8; 12] = sealed[..12].try_into().map_err(|_| "invalid_delivery")?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::CHACHA20_POLY1305, key).map_err(|_| "crypto_unavailable")?,
    );
    let mut data = sealed[12..].to_vec();
    let plain = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(associated),
            &mut data,
        )
        .map_err(|_| "invalid_delivery")?;
    String::from_utf8(plain.to_vec()).map_err(|_| "invalid_delivery")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delivery_is_bound_and_authenticated() {
        let key = [7; 32];
        let sealed = seal(&key, "pair:browser", "secret").unwrap();
        assert_eq!(open(&key, "pair:browser", &sealed).unwrap(), "secret");
        assert!(open(&key, "other:browser", &sealed).is_err());
        let mut altered = sealed;
        altered[15] ^= 1;
        assert!(open(&key, "pair:browser", &altered).is_err());
    }
}
