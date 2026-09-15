//! Explicit schema administration; the game runtime never performs DDL.
fn main() {
    let url =
        std::env::var("OMOBA_DATABASE_URL").expect("Set OMOBA_DATABASE_URL for the migration role");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    match runtime.block_on(server::career_store::CareerStore::connect(&url)) {
        Ok(_) => println!("Career schema version 1 ready."),
        Err(error) => {
            eprintln!("Career migration failed: {error}");
            std::process::exit(1);
        }
    }
}
