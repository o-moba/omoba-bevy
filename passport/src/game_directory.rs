//! Anonymous directory bytes from the account region's fixed portal. This is
//! transport only: the native consumer must authenticate its independent pin.
use reqwest::{Url, blocking::Client, redirect::Policy};
use shared::{game_directory::MAX_WIRE_BYTES, region::RegionId};
use std::{io::Read, time::Duration};

#[derive(Clone)]
pub struct DirectoryTransport {
    url: Url,
    client: Client,
}
impl DirectoryTransport {
    /// No runtime/environment URL override, session, cookie jar or bearer token.
    pub fn for_region(region: RegionId, contour: Option<&str>) -> Result<Self, &'static str> {
        if contour.is_some_and(|s| {
            s.is_empty()
                || s.len() > 32
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        }) {
            return Err("invalid directory contour");
        }
        let origin = region.profile().endpoints(contour).portal;
        Self::at_origin(&origin, false)
    }
    fn at_origin(origin: &str, local_fixture: bool) -> Result<Self, &'static str> {
        let mut url = Url::parse(origin).map_err(|_| "invalid directory origin")?;
        if url.username() != ""
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || (url.scheme() != "https"
                && !(local_fixture
                    && url.scheme() == "http"
                    && matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))))
        {
            return Err("invalid directory origin");
        }
        url.set_path("/api/game/directory");
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| "cannot initialize directory transport")?;
        Ok(Self { url, client })
    }
    /// Bounded streaming even without an honest Content-Length. Never returns
    /// usable bytes on a redirect, non-JSON response, interruption or overload.
    pub fn fetch(&self) -> Result<Vec<u8>, &'static str> {
        let reply = self
            .client
            .get(self.url.clone())
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .map_err(|_| "directory unavailable")?;
        let json = reply
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(';')
                    .next()
                    .is_some_and(|s| s.trim().eq_ignore_ascii_case("application/json"))
            });
        if !reply.status().is_success()
            || !json
            || reply
                .content_length()
                .is_some_and(|n| n > MAX_WIRE_BYTES as u64)
        {
            return Err("invalid directory response");
        }
        let mut bytes = Vec::new();
        reply
            .take(MAX_WIRE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "directory response interrupted")?;
        if bytes.len() > MAX_WIRE_BYTES {
            return Err("directory response too large");
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests;
