use omoba_account_api::{App, Config, crypto, migrate, router};
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Account API: {error}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), String> {
    let url = std::env::var("OMOBA_DATABASE_URL").map_err(|_| "Set OMOBA_DATABASE_URL")?;
    if std::env::args().nth(1).as_deref() == Some("migrate") {
        migrate(&url).await?;
        println!("Career v2 and portal v1 ready.");
        return Ok(());
    }
    let origin =
        std::env::var("OMOBA_PORTAL_ORIGIN").map_err(|_| "Set the trusted OMOBA_PORTAL_ORIGIN")?;
    let insecure = std::env::var("OMOBA_ALLOW_INSECURE_LOCAL").as_deref() == Ok("1");
    let local = origin == "http://127.0.0.1:3010" || origin == "http://localhost:3010";
    let uri: axum::http::Uri = origin.parse().map_err(|_| "Invalid portal origin")?;
    if !(uri.scheme_str() == Some("https") || insecure && local)
        || uri.host().is_none()
        || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
        || origin.ends_with('/')
        || origin.contains(['?', '#', '@', '\n', '\r'])
    {
        return Err(
            "Use an exact trusted HTTPS portal origin; loopback development must be explicit"
                .into(),
        );
    }
    let secret = std::env::var("OMOBA_PORTAL_SECRET")
        .ok()
        .and_then(|s| crypto::decode::<32>(&s))
        .ok_or("Set a private random 32-byte OMOBA_PORTAL_SECRET in hex")?;
    let bind: SocketAddr = std::env::var("OMOBA_ACCOUNT_BIND")
        .unwrap_or_else(|_| "127.0.0.1:40550".into())
        .parse()
        .map_err(|_| "Invalid bind address")?;
    // TLS and client-IP limits belong to a trusted reverse proxy. Do not expose raw HTTP.
    if !bind.ip().is_loopback() {
        return Err("Bind Account API to loopback behind a trusted TLS ingress".into());
    }
    let app = App::connect(
        &url,
        Config {
            origin,
            secret,
            trust_loopback_proxy: std::env::var("OMOBA_TRUST_LOOPBACK_PROXY").as_deref() == Ok("1"),
            release_file: std::env::var_os("OMOBA_RELEASE_CATALOG").map(Into::into),
        },
    )
    .await
    .map_err(|e| e.1)?;
    let worker = app.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            if let Err(error) = omoba_account_api::read::project(&worker, 128).await {
                eprintln!("projection: {}", error.1);
            }
            if let Err(error) = worker.cleanup().await {
                eprintln!("cleanup: {}", error.1);
            }
        }
    });
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|_| "Cannot bind account listener")?;
    println!("Account API listening on {bind}");
    axum::serve(
        listener,
        router(app).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
    .map_err(|_| "HTTP service stopped unexpectedly".into())
}
