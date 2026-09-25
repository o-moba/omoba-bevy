//! Deterministic HTTP evidence for the real SDK v2 catalogue and asset pipeline.
use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct Fixture {
    origin: String,
    catalogue: Arc<Mutex<Value>>,
    unavailable: Arc<AtomicBool>,
    partial: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Fixture {
    fn start(model: Vec<u8>, thumbnail: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let catalogue = Arc::new(Mutex::new(catalogue_envelope(vec![])));
        let unavailable = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let partial = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (feed, failed, stopped, calls) = (
            catalogue.clone(),
            unavailable.clone(),
            stop.clone(),
            requests.clone(),
        );
        let partially_failed = partial.clone();
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0u8; 8192];
                let count = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..count]);
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                calls.lock().unwrap().push(path.clone());
                let (status, mime, bytes) = if path.starts_with("/v1/") {
                    ("200 OK", "application/json", br#"{"avatars":[]}"#.to_vec())
                } else if failed.load(Ordering::Relaxed) {
                    (
                        "503 Service Unavailable",
                        "application/json",
                        b"{}".to_vec(),
                    )
                } else if path.starts_with("/v2/avatars") {
                    (
                        "200 OK",
                        "application/json",
                        serde_json::to_vec(&*feed.lock().unwrap()).unwrap(),
                    )
                } else if path == "/model.glb" {
                    ("200 OK", "model/gltf-binary", model.clone())
                } else if path.starts_with("/portrait") {
                    ("200 OK", "image/png", thumbnail.clone())
                } else {
                    ("404 Not Found", "application/json", b"{}".to_vec())
                };
                let partial_header = if partially_failed.load(Ordering::Relaxed) {
                    "X-Studio-Status: unavailable\r\n"
                } else {
                    ""
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{partial_header}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(&bytes).unwrap();
            }
        });
        Self {
            origin,
            catalogue,
            unavailable,
            partial,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.thread.take().unwrap().join();
    }
}

fn catalogue_envelope(items: Vec<Value>) -> Value {
    json!({"schema": "ekza.avatar.catalog.v2", "count": items.len(), "items": items})
}

fn avatar(origin: &str, id: u8, access: &str, sha: &str, size: usize) -> Value {
    json!({
        "id": format!("ekza:avatar:00000000-0000-4000-8000-{id:012}"),
        "name": format!("Studio fixture {id}"),
        "access": access,
        "creator": {"name": "Isolated test fixture"},
        "license": {"text": "CC0"},
        "thumbnailUrl": format!("{origin}/portrait.png"),
        "renditions": [{
            "platform": "desktop", "profile": "humanoid-glb-v1", "format": "glb",
            "sha256": sha, "sizeBytes": size, "downloadUrl": format!("{origin}/model.glb")
        }],
        "projectSupport": [{
            "projectId": "omoba", "platform": "desktop", "profile": "humanoid-glb-v1", "status": "approved"
        }]
    })
}

#[test]
fn sdk_http_catalogue_refresh_validation_and_failed_cache_are_real() {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../client/assets/avatars");
    let bytes = std::fs::read(assets.join("mega-angel.glb")).expect("existing release model");
    crate::validate_humanoid_profile(&bytes).expect("real shipped humanoid model");
    // A PNG envelope is sufficient for the SDK thumbnail container check. Native
    // render evidence uses the actual release portrait, not this tiny fixture.
    let png = vec![137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0];
    let fixture = Fixture::start(bytes.clone(), png);
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let free = avatar(&fixture.origin, 1, "free", &sha, bytes.len());
    let paid = avatar(&fixture.origin, 2, "owned", &sha, bytes.len());
    let mut unapproved = avatar(&fixture.origin, 3, "free", &sha, bytes.len());
    unapproved["projectSupport"][0]["status"] = "pending".into();
    *fixture.catalogue.lock().unwrap() = catalogue_envelope(vec![free, paid, unapproved]);
    let root = std::env::temp_dir().join(format!("omoba-sdk-fixture-{}", std::process::id()));
    let runtime = Box::leak(Box::new(Runtime {
        store: AvatarStore::new(root.clone(), &fixture.origin, crate::selector()).unwrap(),
        registry: fixture.origin.clone(),
        state: Mutex::default(),
    }));
    assert_eq!(
        status_for(&runtime.state.lock().unwrap()),
        CatalogueStatus::Loading { cached: 0 }
    );
    refresh_now(runtime);
    assert_eq!(
        status_for(&runtime.state.lock().unwrap()),
        CatalogueStatus::Ready { count: 2 }
    );
    let initial = runtime.state.lock().unwrap().items.clone();
    assert_eq!(initial.values().filter(|item| item.free).count(), 1);
    let approved = initial.values().find(|item| item.free).unwrap();
    let definition = crate::avatars::avatar_definition(&approved.slug).unwrap();
    assert!(definition.free);
    assert!(definition.thumbnail.is_some());
    install_blocking_inner(runtime, approved)
        .expect("real SDK download, size/hash/envelope/profile validation");
    assert!(matches!(
        runtime.state.lock().unwrap().installs[&approved.slug],
        Install::Ready
    ));
    crate::verify_local(
        &approved.protected,
        &root.join("avatars").join(format!("{}.glb", approved.slug)),
    )
    .unwrap();
    let installed = runtime.state.lock().unwrap().changed.clone();
    assert!(installed.contains(&approved.slug));

    // Same identity, new display/access metadata: the live picker snapshot
    // updates while the shared model boundary remains immutable.
    let old_thumbnail = definition.thumbnail.clone();
    let mut renamed = avatar(&fixture.origin, 1, "owned", &sha, bytes.len());
    renamed["name"] = "Renamed Studio fixture".into();
    renamed["thumbnailUrl"] = format!("{}/portrait2.png", fixture.origin).into();
    *fixture.catalogue.lock().unwrap() = catalogue_envelope(vec![
        renamed,
        avatar(&fixture.origin, 2, "owned", &sha, bytes.len()),
    ]);
    refresh_now(runtime);
    {
        let state = runtime.state.lock().unwrap();
        let changed = &state.definitions[&approved.slug];
        assert_eq!(changed.display_name, "Renamed Studio fixture");
        assert!(!changed.free);
        assert_ne!(changed.thumbnail, old_thumbnail);
        assert_eq!(changed.passport, definition.passport);
        assert_eq!(
            crate::avatars::avatar_definition(&approved.slug)
                .unwrap()
                .display_name,
            definition.display_name
        );
    }

    // Same count, different identities: stale avatars must disappear from menus.
    let replacement = avatar(&fixture.origin, 4, "free", &sha, bytes.len());
    let corrupt = avatar(&fixture.origin, 5, "free", &"a".repeat(64), bytes.len());
    *fixture.catalogue.lock().unwrap() = catalogue_envelope(vec![replacement, corrupt]);
    refresh_now(runtime);
    let current = runtime.state.lock().unwrap().items.clone();
    assert_eq!(initial.len(), current.len());
    assert!(initial.keys().all(|slug| !current.contains_key(slug)));
    let bad = current
        .values()
        .find(|item| item.protected.support.rendition.sha256 == "a".repeat(64))
        .unwrap();
    assert!(
        install_blocking_inner(runtime, bad).is_err(),
        "hash mismatch must not become Ready"
    );
    assert!(matches!(
        runtime.state.lock().unwrap().installs[&bad.slug],
        Install::Failed(_)
    ));

    fixture.unavailable.store(true, Ordering::Relaxed);
    refresh_now(runtime);
    assert_eq!(
        status_for(&runtime.state.lock().unwrap()),
        CatalogueStatus::Unavailable { cached: 2 }
    );
    assert_eq!(runtime.state.lock().unwrap().items.len(), 2);
    assert!(
        !fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|path| path.starts_with("/v1/")),
        "503 must never fall back to the empty v1 feed"
    );
    fixture.unavailable.store(false, Ordering::Relaxed);
    fixture.partial.store(true, Ordering::Relaxed);
    refresh_now(runtime);
    assert_eq!(
        status_for(&runtime.state.lock().unwrap()),
        CatalogueStatus::Unavailable { cached: 2 }
    );
    fixture.partial.store(false, Ordering::Relaxed);
    *fixture.catalogue.lock().unwrap() = catalogue_envelope(vec![]);
    refresh_now(runtime);
    assert_eq!(
        status_for(&runtime.state.lock().unwrap()),
        CatalogueStatus::Empty
    );
    assert!(runtime.state.lock().unwrap().items.is_empty());
    let requests = fixture.requests.lock().unwrap();
    assert!(requests.iter().any(|path| path.starts_with("/v2/avatars?")
        && path.contains("project=omoba")
        && path.contains("profile=humanoid-glb-v1")));
    assert!(requests.iter().any(|path| path == "/model.glb"));
    assert!(requests.iter().any(|path| path == "/portrait.png"));
    println!(
        "SDK fixture: approved free + owned catalogue, rejected pending approval; downloaded real {}-byte humanoid GLB; hash corruption rejected; equal-count replacement; unavailable cache and empty refresh verified",
        bytes.len()
    );
    std::fs::remove_dir_all(root).unwrap();
}
