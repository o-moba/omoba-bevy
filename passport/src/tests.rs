use super::*;
use ekza_bevy_sdk::passport::Rendition;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::atomic::{AtomicU64, Ordering},
    thread,
};

fn model() -> Vec<u8> {
    // Protocol fixture only, not a render-quality model.
    let animations = ["idle", "walk", "attack", "cast", "death"]
        .map(|name| json!({"name":name,"channels":[{"target":{"node":0,"path":"rotation"}}]}));
    let mut json = serde_json::to_vec(&json!({"asset":{"version":"2.0"},"nodes":[{"name":"Hips"}],
        "skins":[{"joints":[0]}],"animations":animations}))
    .unwrap();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let size = 20 + json.len();
    let mut bytes = b"glTF".to_vec();
    bytes.extend(2_u32.to_le_bytes());
    bytes.extend((size as u32).to_le_bytes());
    bytes.extend((json.len() as u32).to_le_bytes());
    bytes.extend(b"JSON");
    bytes.extend(json);
    bytes
}

fn asset(url: &str, bytes: &[u8]) -> ProtectedAvatar {
    ProtectedAvatar {
        avatar_id: format!("solana:devnet:avatar-data:{}", "1".repeat(32)),
        support: ProjectSupport {
            project_id: "omoba".into(),
            platform: "desktop".into(),
            profile: "humanoid-glb-v1".into(),
            status: "approved".into(),
            rendition: Rendition {
                id: "r1".into(),
                url: url.into(),
                sha256: format!("{:x}", Sha256::digest(bytes)),
                size_bytes: bytes.len() as u64,
                format: "glb".into(),
            },
        },
    }
}

fn fixture_server(responses: Vec<(u16, Vec<u8>)>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let count = stream.read(&mut chunk).unwrap();
                if count == 0 {
                    break;
                }
                bytes.extend(&chunk[..count]);
                if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    let header = String::from_utf8_lossy(&bytes[..header_end]);
                    let content_len = header
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= header_end + 4 + content_len {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(&body).unwrap();
        }
        requests
    });
    (origin, worker)
}

#[test]
fn rejects_corrupt_truncated_or_incomplete_approved_model() {
    let bytes = model();
    let protected = asset("https://example.test/a.glb", &bytes);
    assert!(verify_bytes(&protected, &bytes).is_ok());
    let mut corrupt = bytes.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(verify_bytes(&protected, &corrupt).is_err());
    assert!(verify_bytes(&protected, &bytes[..bytes.len() - 1]).is_err());
    let bad = b"glTF\x02\0\0\0\x14\0\0\0\0\0\0\0JSON";
    assert!(verify_bytes(&asset("https://example.test/a.glb", bad), bad).is_err());
}

#[test]
fn only_explicit_secure_operator_origin_is_allowed() {
    for url in [
        "http://example.test/api/passport",
        "https://a:b@example.test/api/passport",
        "https://example.test/api/passport?token=secret",
        "https://example.test/other",
    ] {
        assert!(PassportApi::new(url).is_err());
    }
    assert!(PassportApi::new("https://example.test/api/passport").is_ok());
    assert!(PassportApi::new("http://127.0.0.1:1234/api/passport").is_ok());
}

#[test]
fn poll_uses_secret_body_and_reports_pending_then_approval() {
    let (origin, worker) = fixture_server(vec![
        (200, br#"{"status":"pending"}"#.to_vec()),
        (200, serde_json::to_vec(&json!({"status":"approved","accessToken":"private-access-token","expiresAt":"2099-01-01T00:00:00Z","wallet":"2".repeat(32)})).unwrap()),
    ]);
    let api = PassportApi::new(&format!("{origin}/api/passport")).unwrap();
    assert!(matches!(
        api.poll("private-device-code-123").unwrap(),
        PairingPoll::Pending
    ));
    assert!(matches!(
        api.poll("private-device-code-123").unwrap(),
        PairingPoll::Approved { .. }
    ));
    for request in worker.join().unwrap() {
        assert!(request.starts_with("POST /api/passport/device/poll HTTP/1.1"));
        assert!(!request.lines().next().unwrap().contains("private-device"));
        assert!(request.contains("\"deviceCode\":\"private-device-code-123\""));
    }
}

#[test]
fn denied_expired_replay_and_redirect_responses_never_grant() {
    for status in [401, 403, 404, 410, 409, 302] {
        let (origin, worker) =
            fixture_server(vec![(status, br#"{"error":"secret-do-not-log"}"#.to_vec())]);
        let api = PassportApi::new(&format!("{origin}/api/passport")).unwrap();
        let error = api
            .consume("private-one-use-ticket", "session-1")
            .unwrap_err();
        assert!(!error.contains("secret-do-not-log"));
        let request = worker.join().unwrap().pop().unwrap();
        assert!(request.contains("\"projectId\":\"omoba\""));
        assert!(request.contains("\"sessionId\":\"session-1\""));
    }
}

#[test]
fn download_is_bounded_checked_and_never_sends_session_token() {
    let bytes = model();
    let (origin, worker) = fixture_server(vec![(200, bytes.clone())]);
    let api = PassportApi::new(&format!("{origin}/api/passport")).unwrap();
    assert_eq!(
        api.download(&asset(&format!("{origin}/asset.glb"), &bytes))
            .unwrap(),
        bytes
    );
    let requests = worker.join().unwrap();
    assert!(!requests[0].to_lowercase().contains("authorization:"));
    let (origin, worker) = fixture_server(vec![(200, vec![0; bytes.len() + 1])]);
    let api = PassportApi::new(&format!("{origin}/api/passport")).unwrap();
    assert!(
        api.download(&asset(&format!("{origin}/asset.glb"), &bytes))
            .is_err()
    );
    worker.join().unwrap();
}

#[test]
fn library_owner_and_exact_support_selectability_are_independent_of_shared_roster() {
    let bytes = model();
    let protected = asset("https://example.test/a.glb", &bytes);
    let mut session = NativeSession {
        api: PassportApi::new("https://example.test/api/passport").unwrap(),
        token: "private".into(),
        library: PurchasedLibrary {
            schema: "ekza.passport.library.v1".into(),
            network: "solana-devnet".into(),
            wallet: "2".repeat(32),
            expires_at: "2099-01-01T00:00:00Z".into(),
            items: vec![],
        },
    };
    assert!(session.owns(&protected).is_none());
    session.library.items.push(PurchasedAvatar {
        avatar_id: protected.avatar_id.clone(),
        mint: "3".repeat(32),
        name: "Owned avatar".into(),
        thumbnail_url: "".into(),
        support: vec![protected.support.clone()],
    });
    assert!(session.owns(&protected).is_some());
    let mut revised = protected;
    revised.support.rendition.id = "r2".into();
    assert!(session.owns(&revised).is_none());
}

#[test]
fn imported_bytes_and_sidecar_retain_identity_and_preserve_original_roster() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "omoba-passport-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(root.join("avatars")).unwrap();
    let original = br#"{"avatars":[{"slug":"free","display_name":"Free","collection":"CC0","license":"CC0","source_url":"https://example.test/free"}]}"#;
    fs::write(root.join("avatars/manifest.json"), original).unwrap();
    let bytes = model();
    let (origin, worker) = fixture_server(vec![(200, bytes.clone())]);
    let protected = asset(&format!("{origin}/asset.glb"), &bytes);
    let mut mobile = protected.support.clone();
    mobile.platform = "mobile".into();
    mobile.profile = "mobile-lod-v1".into();
    let session = NativeSession {
        api: PassportApi::new(&format!("{origin}/api/passport")).unwrap(),
        token: "private".into(),
        library: PurchasedLibrary {
            schema: "ekza.passport.library.v1".into(),
            network: "solana-devnet".into(),
            wallet: "2".repeat(32),
            expires_at: "2099-01-01T00:00:00Z".into(),
            items: vec![PurchasedAvatar {
                avatar_id: protected.avatar_id.clone(),
                mint: "3".repeat(32),
                name: "Owned".into(),
                thumbnail_url: "".into(),
                // Another explicitly approved platform must not prevent this
                // desktop importer from choosing its supported rendition.
                support: vec![mobile, protected.support.clone()],
            }],
        },
    };
    let manifest = root.join("passport-manifest.json");
    assert_eq!(import_owned(&session, &root, &manifest).unwrap(), 1);
    worker.join().unwrap();
    assert_eq!(
        fs::read(root.join("avatars/manifest.json")).unwrap(),
        original
    );
    let imported: Value = serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
    assert_eq!(imported["avatars"][0]["slug"], "free");
    assert_eq!(
        imported["avatars"][1]["passport"]["avatarId"],
        protected.avatar_id
    );
    let slug = protected_slug(&protected);
    verify_local(
        &protected,
        &root.join("avatars").join(format!("{slug}.glb")),
    )
    .unwrap();
    fs::remove_dir_all(root).unwrap(); // This test's unique fixture directory only.
}
