//! Anonymous, bounded publication of an operator-signed directory. No signer,
//! account session, database access or request-selected region/contour.
use axum::{
    Router,
    body::Body,
    extract::{RawQuery, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use ed25519_dalek::{Signature, VerifyingKey};
use shared::{
    game_directory::{Directory, MAX_WIRE_BYTES, SignedDirectory},
    region::RegionId,
};
use std::{
    io::Read,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Default)]
pub struct DirectorySource(Option<Arc<Source>>);
struct Source {
    path: PathBuf,
    key_id: String,
    public: VerifyingKey,
    region: RegionId,
    contour: String,
}
impl DirectorySource {
    /// Optional settings are all-or-none. Public pins are independent of the
    /// signed artifact; one compiled Account API serves one build authority.
    pub fn from_env() -> Result<Self, String> {
        let text = |name| match std::env::var(name) {
            Ok(value) => Ok((!value.is_empty()).then_some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err(format!("{name} must be UTF-8")),
        };
        let file = std::env::var_os("OMOBA_GAME_DIRECTORY_FILE")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let id = text("OMOBA_GAME_DIRECTORY_KEY_ID")?;
        let public = text("OMOBA_GAME_DIRECTORY_PUBLIC_KEY")?;
        if file.is_none() && id.is_none() && public.is_none() {
            return Ok(Self::default());
        }
        let (Some(file), Some(id), Some(public)) = (file, id, public) else {
            return Err("Set OMOBA_GAME_DIRECTORY_FILE, OMOBA_GAME_DIRECTORY_KEY_ID and OMOBA_GAME_DIRECTORY_PUBLIC_KEY together".into());
        };
        let region = RegionId::from_market(shared::market::MARKET)
            .ok_or("This build has no directory account region")?;
        let contour = option_env!("OMOBA_DEFAULT_CONFIG_DIR_SUFFIX")
            .filter(|s| !s.is_empty())
            .unwrap_or("production");
        Self::configured(file, id, public, region, contour)
    }
    fn configured(
        path: PathBuf,
        key_id: String,
        public: String,
        region: RegionId,
        contour: &str,
    ) -> Result<Self, String> {
        SignedDirectory {
            key_id: key_id.clone(),
            payload: String::new(),
            signature: String::new(),
        }
        .signing_bytes()
        .map_err(str::to_owned)?;
        if contour.is_empty()
            || contour.len() > 32
            || !contour
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err("Invalid build directory contour".into());
        }
        let bytes = crate::crypto::decode::<32>(&public).ok_or("Invalid directory public pin")?;
        let public =
            VerifyingKey::from_bytes(&bytes).map_err(|_| "Invalid directory public pin")?;
        Ok(Self(Some(Arc::new(Source {
            path,
            key_id,
            public,
            region,
            contour: contour.into(),
        }))))
    }
    fn load(&self, now_ms: u64) -> Result<Vec<u8>, &'static str> {
        let source = self.0.as_ref().ok_or("disabled")?;
        // Reject non-regular configured paths before opening them, and inspect
        // the actual descriptor again. Read at most the limit plus one even if
        // the file grows. Publication uses atomic replacement of a regular file.
        let metadata = std::fs::symlink_metadata(&source.path).map_err(|_| "unavailable")?;
        if !metadata.is_file() || metadata.len() > MAX_WIRE_BYTES as u64 {
            return Err("invalid_file");
        }
        let file = std::fs::File::open(&source.path).map_err(|_| "unavailable")?;
        let metadata = file.metadata().map_err(|_| "unavailable")?;
        if !metadata.is_file() || metadata.len() > MAX_WIRE_BYTES as u64 {
            return Err("invalid_file");
        }
        let mut bytes = Vec::new();
        file.take(MAX_WIRE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "unavailable")?;
        let envelope = SignedDirectory::parse(&bytes)?;
        if envelope.key_id != source.key_id {
            return Err("untrusted_key");
        }
        let signature =
            crate::crypto::decode::<64>(&envelope.signature).ok_or("invalid_signature")?;
        source
            .public
            .verify_strict(
                &envelope.signing_bytes()?,
                &Signature::from_bytes(&signature),
            )
            .map_err(|_| "invalid_signature")?;
        Directory::parse(&envelope.payload)?.validate_at(
            source.region,
            &source.contour,
            now_ms,
            0,
        )?;
        Ok(bytes)
    }
}

/// The public route has its own immutable state, so existing Account API
/// embedders/tests need no configuration-field or database-schema changes.
pub fn routes<S: Clone + Send + Sync + 'static>(source: DirectorySource) -> Router<S> {
    Router::new()
        .route("/v1/game-directory", get(publish))
        .with_state(source)
}
async fn publish(State(source): State<DirectorySource>, RawQuery(query): RawQuery) -> Response {
    if query.is_some_and(|q| !q.is_empty()) {
        return response(
            StatusCode::BAD_REQUEST,
            b"{\"code\":\"invalid_request\"}".to_vec(),
        );
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|n| n.as_millis().min(u64::MAX as u128) as u64);
    let Ok(now) = now else {
        return unavailable();
    };
    let read = tokio::task::spawn_blocking(move || source.load(now));
    match tokio::time::timeout(Duration::from_secs(3), read).await {
        Ok(Ok(Ok(bytes))) => response(StatusCode::OK, bytes),
        _ => unavailable(),
    }
}
fn unavailable() -> Response {
    response(
        StatusCode::SERVICE_UNAVAILABLE,
        b"{\"code\":\"game_directory_unavailable\"}".to_vec(),
    )
}
fn response(status: StatusCode, bytes: Vec<u8>) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        Body::from(bytes),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, http::Request};
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;
    use tower::ServiceExt;
    struct Fixture {
        dir: PathBuf,
        source: DirectorySource,
        bytes: Vec<u8>,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "omoba-directory-http-{}",
                crate::crypto::random::<8>()
            ));
            std::fs::create_dir(&dir).unwrap();
            let key = SigningKey::from_bytes(&[7; 32]); // Publicly known fixture, never a live key.
            let now = crate::now() as u64 * 1000;
            let payload=json!({"version":1,"region":"ru","contour":"production","sequence":4,"issued_at_ms":now,"expires_at_ms":now+60000,"policy":{"min_release":"0.45.0","maintenance":true,"update_url":null,"message":"Проверка каталога"},"lobbies":[]}).to_string();
            let mut envelope = SignedDirectory {
                key_id: "fixture".into(),
                payload,
                signature: String::new(),
            };
            envelope.signature =
                crate::crypto::hex(&key.sign(&envelope.signing_bytes().unwrap()).to_bytes());
            let bytes = serde_json::to_vec_pretty(&envelope).unwrap();
            let path = dir.join("directory.json");
            std::fs::write(&path, &bytes).unwrap();
            let source = DirectorySource::configured(
                path,
                "fixture".into(),
                crate::crypto::hex(&key.verifying_key().to_bytes()),
                RegionId::Ru,
                "production",
            )
            .unwrap();
            Self { dir, source, bytes }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).unwrap();
        }
    }
    #[tokio::test]
    async fn anonymous_get_and_head_preserve_exact_signed_bytes_without_cookies() {
        let f = Fixture::new();
        let router = routes::<()>(f.source.clone());
        for method in ["GET", "HEAD"] {
            let reply = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri("/v1/game-directory")
                        .header("cookie", "account=ignored")
                        .header("authorization", "Bearer ignored")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(reply.status(), StatusCode::OK);
            assert_eq!(reply.headers()[header::CACHE_CONTROL], "no-store");
            assert!(!reply.headers().contains_key(header::SET_COOKIE));
            assert_eq!(
                to_bytes(reply.into_body(), MAX_WIRE_BYTES + 1)
                    .await
                    .unwrap()
                    .as_ref(),
                if method == "GET" {
                    f.bytes.as_slice()
                } else {
                    &[]
                }
            );
        }
        for (method, uri, status) in [
            ("POST", "/v1/game-directory", StatusCode::METHOD_NOT_ALLOWED),
            (
                "GET",
                "/v1/game-directory?region=global",
                StatusCode::BAD_REQUEST,
            ),
        ] {
            assert_eq!(
                router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method(method)
                            .uri(uri)
                            .body(Body::empty())
                            .unwrap()
                    )
                    .await
                    .unwrap()
                    .status(),
                status
            );
        }
    }
    #[test]
    fn bounded_source_refuses_tamper_scope_expiry_missing_and_nonregular_files() {
        let f = Fixture::new();
        let now = crate::now() as u64 * 1000;
        assert_eq!(f.source.load(now).unwrap(), f.bytes);
        let source = f.source.0.as_ref().unwrap();
        let wrong_scope = DirectorySource::configured(
            source.path.clone(),
            source.key_id.clone(),
            crate::crypto::hex(&source.public.to_bytes()),
            RegionId::Global,
            "production",
        )
        .unwrap();
        assert!(wrong_scope.load(now).is_err());
        let wrong_contour = DirectorySource::configured(
            source.path.clone(),
            source.key_id.clone(),
            crate::crypto::hex(&source.public.to_bytes()),
            RegionId::Ru,
            "dev",
        )
        .unwrap();
        assert!(wrong_contour.load(now).is_err());
        let wrong_key = DirectorySource::configured(
            source.path.clone(),
            source.key_id.clone(),
            crate::crypto::hex(&SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes()),
            RegionId::Ru,
            "production",
        )
        .unwrap();
        assert!(wrong_key.load(now).is_err());
        assert!(f.source.load(now + 60000).is_err());
        let mut e = SignedDirectory::parse(&f.bytes).unwrap();
        e.payload.push(' ');
        std::fs::write(&source.path, serde_json::to_vec(&e).unwrap()).unwrap();
        assert!(f.source.load(now).is_err());
        std::fs::write(&source.path, vec![b'x'; MAX_WIRE_BYTES + 1]).unwrap();
        assert!(f.source.load(now).is_err());
        std::fs::remove_file(&source.path).unwrap();
        assert!(f.source.load(now).is_err());
        std::fs::create_dir(&source.path).unwrap();
        assert!(f.source.load(now).is_err());
        assert!(DirectorySource::default().load(now).is_err());
    }
    #[tokio::test]
    async fn actual_loopback_http_refreshes_atomic_publication_and_fails_closed() {
        let f = Fixture::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server =
            tokio::spawn(axum::serve(listener, routes::<()>(f.source.clone())).into_future());
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let url = format!("http://{addr}/v1/game-directory");
        let first = client.get(&url).send().await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(first.bytes().await.unwrap().as_ref(), f.bytes.as_slice());
        let replacement = f.dir.join("replacement.json");
        std::fs::write(&replacement, b"corrupt").unwrap();
        std::fs::rename(&replacement, f.dir.join("directory.json")).unwrap();
        let bad = client.get(&url).send().await.unwrap();
        assert_eq!(bad.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            bad.text().await.unwrap(),
            "{\"code\":\"game_directory_unavailable\"}"
        );
        std::fs::write(&replacement, &f.bytes).unwrap();
        std::fs::rename(replacement, f.dir.join("directory.json")).unwrap();
        assert_eq!(
            client.get(url).send().await.unwrap().status(),
            StatusCode::OK
        );
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    #[ignore = "requires a migrated loopback omoba_test database"]
    async fn configured_directory_uses_the_real_account_router_without_a_session() {
        let url =
            std::env::var("OMOBA_TEST_DATABASE_URL").expect("Set loopback OMOBA_TEST_DATABASE_URL");
        let parsed = reqwest::Url::parse(&url).unwrap();
        assert!(
            matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                && parsed.path() == "/omoba_test"
                && parsed.query().is_none()
                && parsed.fragment().is_none()
        );
        let f = Fixture::new();
        let app = crate::App::connect(
            &url,
            crate::Config {
                origin: "http://127.0.0.1:3010".into(),
                secret: [0; 32],
                trust_loopback_proxy: false,
                release_file: None,
            },
        )
        .await
        .unwrap();
        let router = crate::router_with_game_directory(app, f.source.clone());
        let reply = router
            .oneshot(
                Request::builder()
                    .uri("/v1/game-directory")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reply.status(), StatusCode::OK);
        assert!(!reply.headers().contains_key(header::SET_COOKIE));
        assert_eq!(
            to_bytes(reply.into_body(), MAX_WIRE_BYTES + 1)
                .await
                .unwrap()
                .as_ref(),
            f.bytes.as_slice()
        );
    }
}
