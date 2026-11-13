//! Explicit seeded paid periods for connected local QA, never real provider evidence.
//! Defaults to seeding/resetting only these known accounts; `serve` starts the local API.
use ed25519_dalek::SigningKey;
use omoba_account_api::{App, Config, crypto, migrate, now, router, supporter::BillingConfig};
use serde_json::json;
use std::{net::SocketAddr, sync::Arc};

#[tokio::main]
async fn main() {
    let action = std::env::args().nth(1).unwrap_or_else(|| "seed".into());
    assert!(
        matches!(action.as_str(), "seed" | "serve"),
        "Use seed or serve"
    );
    let url = std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL")
        .expect("Explicit disposable fixture database URL required");
    let options: sqlx::postgres::PgConnectOptions = url.parse().expect("valid PostgreSQL URL");
    assert_eq!(options.get_host(), "127.0.0.1", "Loopback fixture only");
    assert_eq!(options.get_port(), 55581, "Dedicated fixture port required");
    assert_eq!(
        options.get_database(),
        Some("hero_supporter"),
        "Dedicated disposable database required"
    );
    migrate(&url).await.unwrap();
    let mut app = App::connect(
        &url,
        Config {
            origin: "http://127.0.0.1:3034".into(),
            secret: [0; 32],
            trust_loopback_proxy: false,
            release_file: None,
        },
    )
    .await
    .unwrap();
    // This local example deliberately offers no purchase provider or billing worker.
    app.billing = Arc::new(BillingConfig::default());
    if action == "serve" {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:40558")
            .await
            .unwrap();
        println!(
            "Synthetic Hero fixture API: http://127.0.0.1:40558; portal http://127.0.0.1:3034"
        );
        axum::serve(
            listener,
            router(app).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap();
        return;
    }
    let current = now();
    let mut profiles = Vec::new();
    for (seed, scenario, name, visible, public, stats, count) in [
        (7u8, "active", "Aurora Hero", true, true, true, 3),
        (8, "expired", "Starlight Patron", true, true, true, 2),
        (9, "stats_private", "Quiet Guardian", true, true, false, 1),
        (10, "default_hidden", "Hidden Patron", false, true, true, 1),
        (
            11,
            "profile_private",
            "Private Patron",
            true,
            false,
            false,
            1,
        ),
        (12, "refunded", "Refunded Patron", true, true, true, 1),
        (13, "future", "Future Patron", true, true, true, 1),
    ] {
        let key = SigningKey::from_bytes(&[seed; 32]);
        let profile = app
            .career
            .authenticate(&crypto::hex(key.verifying_key().as_bytes()), name)
            .await
            .unwrap();
        let profile = app.career.rename(&profile.profile_id, name).await.unwrap();
        sqlx::query("INSERT INTO portal.profile_settings(profile_id,public_profile_enabled,public_stats_enabled) VALUES($1,$2,$3) ON CONFLICT(profile_id) DO UPDATE SET public_profile_enabled=$2,public_stats_enabled=$3,discoverable=false,version=portal.profile_settings.version+1")
            .bind(&profile.profile_id).bind(public).bind(stats).execute(&app.pool).await.unwrap();
        sqlx::query("INSERT INTO portal.supporter_preferences(profile_id,aura,recognition_visible) VALUES($1,'solar',$2) ON CONFLICT(profile_id) DO UPDATE SET aura='solar',recognition_visible=$2")
            .bind(&profile.profile_id).bind(visible).execute(&app.pool).await.unwrap();
        let mut periods = Vec::new();
        for index in 0..count {
            let id = format!("hero-fixture-seed{seed}-month{index}");
            let historic = scenario == "expired" || index < count - 1;
            let (from, until) = if historic {
                (
                    current - (count - index + 1) * 2592000,
                    current - (count - index) * 2592000,
                )
            } else if scenario == "future" {
                (current + 86400, current + 86400 + 2592000)
            } else {
                (current - 86400, current + 29 * 86400)
            };
            let revoked = (scenario == "refunded").then_some(current);
            sqlx::query("INSERT INTO portal.supporter_grants(provider,period_id,profile_id,valid_from,valid_until,revoked_at,event_version,renewal_enabled) VALUES('solana',$1,$2,$3,$4,$5,1,false) ON CONFLICT(provider,period_id) DO UPDATE SET valid_from=$3,valid_until=$4,revoked_at=$5")
                .bind(&id).bind(&profile.profile_id).bind(from).bind(until).bind(revoked)
                .execute(&app.pool).await.unwrap();
            periods.push(json!({"period_id":id,"valid_from":from,"valid_until":until,"revoked":revoked.is_some()}));
        }
        profiles.push(json!({"seed":seed,"scenario":scenario,"profile_id":profile.profile_id,"nickname":profile.nickname,"visible":visible,"public_profile_enabled":public,"public_stats_enabled":stats,"periods":periods}));
    }
    println!(
        "{}",
        json!({"synthetic":true,"evidence":"explicitly seeded periods; no provider verification or real payments","api_origin":"http://127.0.0.1:40558","portal_origin":"http://127.0.0.1:3034","profiles":profiles})
    );
}
