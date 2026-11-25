//! Read-only aggregate report; never runs schema migrations or season resets.
use omoba_career_store::career_store::CareerStore;

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let limit = match args.as_slice() {
        [] => 1000,
        [flag, value] if flag == "--receipt-limit" => value
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=10_000).contains(n))
            .ok_or("Receipt limit must be between 1 and 10000.")?,
        _ => return Err("Usage: ranked-input-stats [--receipt-limit 1..10000]".into()),
    };
    let database = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required.")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Unable to start report runtime.")?;
    runtime.block_on(async {
        let store = CareerStore::connect_runtime(&database).await?;
        let report = store.input_statistics(limit).await?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|_| "Unable to encode input statistics.")?
        );
        Ok(())
    })
}
