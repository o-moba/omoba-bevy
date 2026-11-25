//! The explicit disposable-test launcher can use an isolated random host port.
use sqlx::postgres::PgConnectOptions;

pub fn launcher_database(options: &PgConnectOptions) -> bool {
    matches_launcher(
        options,
        std::env::var("OMOBA_TEST_DATABASE_URL").ok().as_deref(),
        std::env::var("OMOBA_REQUIRE_TEST_DATABASE").as_deref() == Ok("1"),
    )
}
fn matches_launcher(options: &PgConnectOptions, url: Option<&str>, required: bool) -> bool {
    required
        && options.get_host() == "127.0.0.1"
        && options.get_database() == Some("omoba_test")
        && url
            .and_then(|s| s.parse::<PgConnectOptions>().ok())
            .is_some_and(|owner| {
                owner.get_host() == "127.0.0.1"
                    && owner.get_port() == options.get_port()
                    && owner.get_database() == options.get_database()
            })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disposable_launcher_never_adopts_a_different_listener_or_database() {
        let url = "postgres://fixture@127.0.0.1:52845/omoba_test";
        let options = url.parse().unwrap();
        assert!(matches_launcher(&options, Some(url), true));
        assert!(!matches_launcher(&options, Some(url), false));
        for other in [
            "postgres://fixture@127.0.0.1:52846/omoba_test",
            "postgres://fixture@remote.example:52845/omoba_test",
            "postgres://fixture@127.0.0.1:52845/production",
        ] {
            assert!(!matches_launcher(&options, Some(other), true));
        }
        for forbidden in [
            "postgres://fixture@remote.example:52845/omoba_test",
            "postgres://fixture@127.0.0.1:52845/production",
        ] {
            assert!(!matches_launcher(
                &forbidden.parse().unwrap(),
                Some(forbidden),
                true
            ));
        }
    }
}
