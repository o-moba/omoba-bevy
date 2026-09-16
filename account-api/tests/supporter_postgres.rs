//! Requires a disposable PostgreSQL database. Provider evidence is synthetic and local.
use axum::{Json, Router, routing::post};
use ed25519_dalek::{Signer, SigningKey};
use omoba_account_api::{
    App, Config, crypto, migrate, now,
    supporter::{
        self, BillingConfig, VerifiedEvent,
        solana::{self, SolanaConfig},
    },
};
use serde_json::{Value, json};
use shared::supporter::{
    NativeSupporterAction, NativeSupporterRequest, SignedNativeSupporterRequest,
};
use std::sync::Arc;

async fn app() -> App {
    let url =
        std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL").expect("isolated test database required");
    migrate(&url).await.unwrap();
    migrate(&url).await.unwrap();
    App::connect(
        &url,
        Config {
            origin: "https://players.example".into(),
            secret: [71; 32],
            trust_loopback_proxy: false,
            release_file: None,
        },
    )
    .await
    .unwrap()
}
async fn account(app: &App) -> (String, SigningKey) {
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).unwrap();
    let key = SigningKey::from_bytes(&seed);
    let p = app
        .career
        .authenticate(&crypto::hex(key.verifying_key().as_bytes()), "Supporter")
        .await
        .unwrap();
    (p.profile_id, key)
}
fn event(period: &str, version: i64) -> VerifiedEvent {
    VerifiedEvent {
        event_id: format!("{period}-{version}"),
        provider: "apple".into(),
        period_id: period.into(),
        original_transaction_id: Some(format!("original-{period}")),
        app_account_token: None,
        product_id: Some("test.supporter".into()),
        environment: Some("Sandbox".into()),
        valid_from: now() - 10,
        valid_until: now() + 86400,
        revoked_at: None,
        event_version: version,
        renewal_enabled: Some(true),
    }
}
fn proof(key: &SigningKey, action: NativeSupporterAction) -> Vec<u8> {
    let request = NativeSupporterRequest {
        origin: "https://players.example".into(),
        public_key: crypto::hex(key.verifying_key().as_bytes()),
        nonce: crypto::random::<32>(),
        expires_at: (now() + 60) as u64,
        action,
    };
    let signature = crypto::hex(&key.sign(&request.signing_bytes()).to_bytes());
    serde_json::to_vec(&SignedNativeSupporterRequest { request, signature }).unwrap()
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn grants_are_account_bound_replay_safe_terminally_revoked_and_expiring() {
    let app = app().await;
    let (a, key) = account(&app).await;
    let (b, _) = account(&app).await;
    assert_eq!(
        supporter::apple_prepare(&app, &a).await.unwrap_err().1,
        "payment_provider_unavailable"
    );
    assert_eq!(
        solana::checkout(&app, &a, b"{}").await.unwrap_err().1,
        "payment_provider_unavailable"
    );
    assert!(
        supporter::equip(&app, &a, b"{\"aura\":\"solar\"}")
            .await
            .is_err()
    );
    let period = crypto::random::<16>();
    let mut e = event(&period, 200);
    assert!(supporter::apply_event(&app, &a, &e).await.unwrap());
    assert!(!supporter::apply_event(&app, &a, &e).await.unwrap());
    assert!(supporter::apply_event(&app, &b, &e).await.is_err());
    let mut changed = e.clone();
    changed.valid_until += 86400;
    assert_eq!(
        supporter::apply_event(&app, &a, &changed)
            .await
            .unwrap_err()
            .1,
        "state_changed"
    );
    e.event_id = format!("{period}-cancel");
    e.event_version = 300;
    e.renewal_enabled = Some(false);
    supporter::apply_event(&app, &a, &e).await.unwrap();
    let s = supporter::status(&app, &a).await.unwrap();
    assert!(s.active);
    assert_eq!(s.grants[0].renewal_enabled, Some(false));
    supporter::equip(&app, &a, b"{\"aura\":\"lunar\"}")
        .await
        .unwrap();
    assert_eq!(
        supporter::status(&app, &a)
            .await
            .unwrap()
            .equipped_aura
            .unwrap()
            .id(),
        "lunar"
    );
    // Even a delayed refund wins over a newer nonrevoked snapshot of that period.
    e.event_id = format!("{period}-refund");
    e.event_version = 250;
    e.revoked_at = Some(now());
    supporter::apply_event(&app, &a, &e).await.unwrap();
    assert!(!supporter::status(&app, &a).await.unwrap().active);
    e.event_id = format!("{period}-late");
    e.event_version = 400;
    e.revoked_at = None;
    supporter::apply_event(&app, &a, &e).await.unwrap();
    assert!(!supporter::status(&app, &a).await.unwrap().active);
    let mut next = event(&crypto::random::<16>(), 500);
    next.original_transaction_id = e.original_transaction_id.clone();
    assert!(supporter::apply_event(&app, &b, &next).await.is_err());
    supporter::apply_event(&app, &a, &next).await.unwrap();
    assert!(supporter::status(&app, &a).await.unwrap().active);
    next.event_version = 600;
    next.event_id.push_str("-expired");
    next.valid_from = now() - 86400;
    next.valid_until = now() - 1;
    supporter::apply_event(&app, &a, &next).await.unwrap();
    assert!(!supporter::status(&app, &a).await.unwrap().active);
    let p = proof(&key, NativeSupporterAction::Status);
    assert!(supporter::native(&app, &p).await.is_ok());
    assert_eq!(
        supporter::native(&app, &p).await.unwrap_err().1,
        "request_replayed"
    );
    let mut forged: Value =
        serde_json::from_slice(&proof(&key, NativeSupporterAction::Status)).unwrap();
    forged["request"]["action"] = json!({"kind":"equip","aura":"solar"});
    assert!(
        supporter::native(&app, &serde_json::to_vec(&forged).unwrap())
            .await
            .is_err()
    );
    sqlx::query("UPDATE career_keys SET revoked_at=clock_timestamp() WHERE public_key=$1")
        .bind(crypto::hex(key.verifying_key().as_bytes()))
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(
        supporter::native(&app, &proof(&key, NativeSupporterAction::Status))
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn solana_checkout_fetches_rpc_and_settles_exactly_once() {
    let mut app = app().await;
    let (a, _) = account(&app).await;
    let (b, _) = account(&app).await;
    let genesis = solana::encode58(&[21; 32]);
    let mint = solana::encode58(&[22; 32]);
    let treasury = solana::encode58(&[23; 32]);
    let mut signature_bytes = [0; 64];
    getrandom::fill(&mut signature_bytes).unwrap();
    let signature = solana::encode58(&signature_bytes);
    let rpc_reply = Arc::new(std::sync::Mutex::new(Value::Null));
    let reply = rpc_reply.clone();
    let expected_genesis = genesis.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let routes = Router::new().route("/", post(move |Json(request): Json<Value>| {
        let reply = reply.clone(); let genesis = expected_genesis.clone();
        async move {
            let result = match request["method"].as_str() {
                Some("getGenesisHash") => json!(genesis),
                Some("getSignaturesForAddress") => {
                    assert_eq!(request["params"][1]["commitment"], "finalized");
                    assert_eq!(request["params"][1]["limit"], 5);
                    let transaction = reply.lock().unwrap();
                    match transaction["transaction"]["signatures"][0].as_str() {
                        Some(signature) => json!([{"signature":signature,"err":null,"confirmationStatus":"confirmed"},{"signature":signature,"err":null,"confirmationStatus":"finalized"}]),
                        None => json!([]),
                    }
                },
                Some("getTransaction") => { assert_eq!(request["params"][1]["commitment"], "finalized"); assert_eq!(request["params"][1]["maxSupportedTransactionVersion"], 1); reply.lock().unwrap().clone() },
                _ => panic!("Unexpected fixture RPC method"),
            };
            Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
        }
    }));
    let server = tokio::spawn(async move {
        axum::serve(listener, routes).await.unwrap();
    });
    app.billing = Arc::new(BillingConfig {
        apple: None,
        solana: Some(SolanaConfig {
            rpc_url: format!("http://{addr}"),
            genesis_hash: genesis.clone(),
            network: "devnet".into(),
            mint: mint.clone(),
            recipient: treasury.clone(),
            amount: 4_990_000,
            decimals: 6,
        }),
    });
    // Historic auto-renew=true periods do not override the latest expired,
    // cancelled period of the same original subscription.
    let old_period = crypto::random::<16>();
    let mut old = event(&old_period, 10);
    old.valid_from = now() - 90 * 86400;
    old.valid_until = now() - 60 * 86400;
    supporter::apply_event(&app, &a, &old).await.unwrap();
    let mut cancelled = event(&crypto::random::<16>(), 20);
    cancelled.original_transaction_id = old.original_transaction_id.clone();
    cancelled.valid_from = now() - 60 * 86400;
    cancelled.valid_until = now() - 30 * 86400;
    cancelled.renewal_enabled = Some(false);
    supporter::apply_event(&app, &a, &cancelled).await.unwrap();
    let order = solana::checkout(&app, &a, b"{}").await.unwrap();
    let repeat = solana::checkout(&app, &a, b"{}").await.unwrap();
    assert_eq!(order, repeat);
    let restored = solana::orders(&app, &a).await.unwrap();
    assert_eq!(restored["orders"][0]["order_id"], order["order_id"]);
    assert_eq!(restored["orders"][0]["status"], "pending");
    assert_eq!(solana::orders(&app, &b).await.unwrap()["orders"], json!([]));
    let reference: String =
        sqlx::query_scalar("SELECT reference FROM portal.supporter_orders WHERE order_id=$1")
            .bind(order["order_id"].as_str().unwrap())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    let body =
        serde_json::to_vec(&json!({"order_id":order["order_id"],"signature":signature})).unwrap();
    assert!(solana::confirm(&app, &b, &body).await.is_err());
    assert_eq!(
        solana::confirm(&app, &a, &body).await.unwrap_err().1,
        "payment_not_finalized"
    );
    let mut data = vec![12];
    data.extend(4_990_000_u64.to_le_bytes());
    data.push(6);
    let transaction = json!({"blockTime":now(),"transaction":{"signatures":[signature],"message":{"accountKeys":["payer","source",mint,"destination","TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",reference],"header":{"numRequiredSignatures":1,"numReadonlyUnsignedAccounts":2},"instructions":[{"programIdIndex":4,"accounts":[1,2,3,0,5],"data":solana::encode58(&data)}]}},"meta":{"err":null,"preTokenBalances":[],"postTokenBalances":[{"accountIndex":3,"mint":mint,"owner":treasury,"uiTokenAmount":{"decimals":6,"amount":"4990000"}}]}});
    *rpc_reply.lock().unwrap() = transaction.clone();
    let mut wrong_network = app.clone();
    let mut cfg = (*app.billing).clone();
    cfg.solana.as_mut().unwrap().genesis_hash = "wrong".into();
    wrong_network.billing = Arc::new(cfg);
    assert!(solana::confirm(&wrong_network, &a, &body).await.is_err());
    let automatic_body = serde_json::to_vec(&json!({"order_id":order["order_id"]})).unwrap();
    let result = solana::confirm(&app, &a, &automatic_body).await.unwrap();
    assert_eq!(result["status"], "confirmed");
    assert_eq!(result["supporter"]["active"], true);
    assert_eq!(solana::confirm(&app, &a, &body).await.unwrap(), result);
    assert_eq!(
        solana::confirm(&app, &a, &automatic_body).await.unwrap(),
        result
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM portal.supporter_grants WHERE profile_id=$1 AND provider='solana'",
    )
    .bind(&a)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    let paid = solana::orders(&app, &a).await.unwrap();
    assert_eq!(paid["orders"][0]["status"], "confirmed");
    assert_eq!(paid["orders"][0]["confirmed_signature"], signature);
    assert_eq!(
        solana::checkout(&app, &a, b"{}").await.unwrap_err().1,
        "already_supported"
    );
    // One blockchain transfer cannot purchase multiple orders, even across profiles.
    let other = solana::checkout(&app, &b, b"{}").await.unwrap();
    let reference_b: String =
        sqlx::query_scalar("SELECT reference FROM portal.supporter_orders WHERE order_id=$1")
            .bind(other["order_id"].as_str().unwrap())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    let mut reused = transaction;
    reused["transaction"]["message"]["accountKeys"][5] = json!(reference_b);
    *rpc_reply.lock().unwrap() = reused;
    let other_body =
        serde_json::to_vec(&json!({"order_id":other["order_id"],"signature":signature})).unwrap();
    assert_eq!(
        solana::confirm(&app, &b, &other_body).await.unwrap_err().1,
        "payment_already_used"
    );
    assert!(!supporter::status(&app, &b).await.unwrap().active);
    server.abort();
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn apple_private_verifier_binds_accounts_preserves_cancellation_and_reconciles_refunds() {
    use omoba_account_api::supporter::AppleConfig;
    let mut app = app().await;
    let (a, _) = account(&app).await;
    let (b, _) = account(&app).await;
    let event_reply = Arc::new(std::sync::Mutex::new(Vec::<VerifiedEvent>::new()));
    let replies = event_reply.clone();
    let secret = crypto::random::<32>();
    let expected = format!("Bearer {secret}");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let routes = Router::new().route(
        "/verify",
        post(
            move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                let events = replies.clone();
                let expected = expected.clone();
                async move {
                    assert_eq!(headers["authorization"].to_str().unwrap(), expected);
                    assert!(matches!(
                        body["kind"].as_str(),
                        Some("transaction" | "notification" | "reconcile")
                    ));
                    Json(json!({"events":events.lock().unwrap().clone()}))
                }
            },
        ),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, routes).await.unwrap();
    });
    app.billing = Arc::new(BillingConfig {
        apple: Some(AppleConfig {
            verifier_url: format!("http://{addr}"),
            secret,
            product_id: "test.supporter".into(),
            environment: "Sandbox".into(),
        }),
        solana: None,
    });
    let prepared = supporter::apple_prepare(&app, &a).await.unwrap();
    assert_eq!(prepared, supporter::apple_prepare(&app, &a).await.unwrap());
    let period = crypto::random::<16>();
    let mut e = event(&period, now() * 1000);
    e.app_account_token = Some(prepared["app_account_token"].as_str().unwrap().into());
    e.renewal_enabled = None;
    *event_reply.lock().unwrap() = vec![e.clone()];
    let body = b"{\"signed_payload\":\"synthetic_private_verifier_fixture\"}";
    assert!(supporter::apple_verify(&app, &b, body).await.is_err());
    assert!(!supporter::status(&app, &b).await.unwrap().active);
    supporter::apple_verify(&app, &a, body).await.unwrap();
    let s = supporter::status(&app, &a).await.unwrap();
    assert!(s.active);
    assert_eq!(s.grants[0].renewal_enabled, None);
    assert_eq!(
        prepared,
        supporter::apple_prepare(&app, &a).await.unwrap(),
        "restore always gets its account token even when already active"
    );
    e.event_id.push_str("-cancel");
    // Equal-timestamp renewal data enriches the transaction's unknown status.
    e.renewal_enabled = Some(false);
    *event_reply.lock().unwrap() = vec![e.clone()];
    supporter::apple_notification(&app, b"{\"signedPayload\":\"synthetic_notification\"}")
        .await
        .unwrap();
    e.event_id.push_str("-restore");
    e.event_version += 1;
    e.renewal_enabled = None;
    *event_reply.lock().unwrap() = vec![e.clone()];
    supporter::apple_verify(&app, &a, body).await.unwrap();
    let s = supporter::status(&app, &a).await.unwrap();
    assert!(s.active);
    assert_eq!(s.grants[0].renewal_enabled, Some(false));
    let mut wrong = e.clone();
    wrong.event_id.push_str("-product");
    wrong.product_id = Some("other.product".into());
    *event_reply.lock().unwrap() = vec![wrong];
    assert!(supporter::apple_verify(&app, &a, body).await.is_err());
    e.event_id.push_str("-refund");
    e.event_version += 1;
    e.revoked_at = Some(now());
    *event_reply.lock().unwrap() = vec![e];
    // Constrain fixture reconciliation to this subscription; tests share a disposable database.
    sqlx::query("UPDATE portal.supporter_grants SET last_reconciled_at=$1 WHERE provider='apple'")
        .bind(now())
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE portal.supporter_grants SET last_reconciled_at=0 WHERE provider='apple' AND period_id=$1").bind(period).execute(&app.pool).await.unwrap();
    supporter::reconcile_apple(&app).await.unwrap();
    assert!(!supporter::status(&app, &a).await.unwrap().active);
    server.abort();
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn future_or_revoked_history_cannot_hide_current_entitlement() {
    let app = app().await;
    let (profile, _) = account(&app).await;
    let e = event(&crypto::random::<16>(), 200);
    supporter::apply_event(&app, &profile, &e).await.unwrap();
    sqlx::query("INSERT INTO portal.supporter_grants(provider,period_id,profile_id,valid_from,valid_until,event_version,renewal_enabled,revoked_at) SELECT 'solana',$1||'-future-'||n::text,$1,$2+100000,$2+200000+n,1,false,CASE WHEN n%2=0 THEN $2 ELSE NULL END FROM generate_series(1,101) n").bind(&profile).bind(now()).execute(&app.pool).await.unwrap();
    let status = supporter::status(&app, &profile).await.unwrap();
    assert!(status.active);
    assert_eq!(status.active_until, Some(e.valid_until));
    assert_eq!(status.grants.len(), 100);
}
