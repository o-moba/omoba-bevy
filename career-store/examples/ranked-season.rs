//! Explicit operator entry point; never migrates or closes on startup/read.
//! Drain every host and refresh/restart lobby caches as described in the plan.
use omoba_career_store::career_store::CareerStore;

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let expected = match args.as_slice() {
        [] => None,
        [show] if show == "show" => None,
        [close, flag, id, apply]
            if close == "close" && flag == "--expected-season" && apply == "--apply" =>
        {
            Some(
                id.parse::<u64>()
                    .ok()
                    .filter(|n| *n > 0 && *n <= i64::MAX as u64)
                    .ok_or("Expected season must be a positive database identity.")?,
            )
        }
        _ => return Err("Usage: ranked-season show | close --expected-season ID --apply".into()),
    };
    let database = std::env::var(if expected.is_some() {
        "OMOBA_SEASON_OPERATOR_DATABASE_URL"
    } else { "DATABASE_URL" }).map_err(|_| {
        if expected.is_some() { "Set the separately authorized OMOBA_SEASON_OPERATOR_DATABASE_URL after draining all hosts." }
        else { "DATABASE_URL is required for the read-only season view." }
    })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Unable to start season operator runtime.")?;
    runtime.block_on(async {
        let store = CareerStore::connect_runtime(&database).await?;
        let value = if let Some(expected) = expected {
            serde_json::to_value(store.close_ranked_season(expected).await?)
        } else {
            serde_json::to_value(store.ranked_season().await?)
        }
        .map_err(|_| "Unable to encode the season receipt.")?;
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|_| "Unable to encode season output.")?
        );
        Ok(())
    })
}
