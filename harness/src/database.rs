//! Explicit disposable database selection for opt-in process rehearsals.

/// These tests create profiles and run migrations. Connection options and
/// non-loopback hosts are never accepted, including a `host=` URI override.
pub fn isolated_database_url(raw: &str) -> bool {
    if raw.contains(['?', '#', '\n', '\r']) {
        return false;
    }
    let Some(rest) = raw
        .strip_prefix("postgres://")
        .or_else(|| raw.strip_prefix("postgresql://"))
    else {
        return false;
    };
    let Some((authority, database)) = rest.split_once('/') else {
        return false;
    };
    let endpoint = authority.rsplit('@').next().unwrap_or_default();
    let (host, suffix) = if let Some(ipv6) = endpoint.strip_prefix('[') {
        let Some(parts) = ipv6.split_once(']') else {
            return false;
        };
        parts
    } else {
        endpoint
            .split_once(':')
            .map_or((endpoint, ""), |(host, _)| (host, &endpoint[host.len()..]))
    };
    let valid_port = suffix.is_empty()
        || suffix.strip_prefix(':').is_some_and(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u16>().is_ok_and(|port| port != 0)
        });
    database == "omoba_test" && valid_port && matches!(host, "127.0.0.1" | "localhost" | "::1")
}

fn select(harness: Option<String>, general: Option<String>) -> Result<String, String> {
    let url = harness
        .or(general)
        .ok_or("set OMOBA_HARNESS_DATABASE_URL or OMOBA_TEST_DATABASE_URL")?;
    if !isolated_database_url(&url) {
        return Err(
            "harness requires an explicit loopback omoba_test database without URI options".into(),
        );
    }
    Ok(url)
}

/// A dedicated harness URL takes precedence; never hide an unsafe explicit URL
/// by silently falling back to a different one.
pub fn test_database_url() -> Result<String, String> {
    select(
        std::env::var("OMOBA_HARNESS_DATABASE_URL").ok(),
        std::env::var("OMOBA_TEST_DATABASE_URL").ok(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_unsafe_url_cannot_be_overridden_by_a_safe_fallback() {
        let safe = "postgres://postgres@127.0.0.1:5432/omoba_test".to_owned();
        assert_eq!(select(None, Some(safe.clone())).unwrap(), safe);
        for bad in [
            "",
            "postgres://db.example/omoba_test",
            "postgres://localhost/production",
            "postgres://localhost/omoba_test?host=other",
            "postgres://localhost/omoba_test#other",
            "postgres://localhost:5432?host=foreign&dbname=x/omoba_test",
            "postgres://localhost:bad/omoba_test",
            "postgres://[::1]evil/omoba_test",
        ] {
            assert!(select(Some(bad.into()), Some(safe.clone())).is_err());
        }
        assert!(isolated_database_url(
            "postgresql://postgres@[::1]:5432/omoba_test"
        ));
        assert!(select(None, None).is_err());
    }
}
