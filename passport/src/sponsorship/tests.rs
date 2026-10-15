use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
};
const CAT: &[u8] = include_bytes!("../../../assets-src/community/lucky-cat/lucky-cat.glb");
fn placement(base: &str) -> Placement {
    Placement {
        id: "11111111-1111-4111-8111-111111111111".into(),
        slot_id: SLOT.into(),
        asset_id: "ekza:map_prop:22222222-2222-4222-8222-222222222222".into(),
        revision_id: "33333333-3333-4333-8333-333333333333".into(),
        rendition_sha256: format!("{:x}", Sha256::digest(CAT)),
        rendition_bytes: CAT.len() as u64,
        rendition_url: format!(
            "{base}/v1/studio/assets/{:x}/rendition",
            Sha256::digest(CAT)
        ),
        title: "Lucky cat".into(),
        credit: "OMOBA community".into(),
        description: String::new(),
        starts_at: "2026-10-15T00:00:00Z".into(),
        expires_at: "2026-10-16T00:00:00+00:00".into(),
    }
}
fn manifest(p: Placement) -> Manifest {
    Manifest {
        server_now: "2026-10-15T12:00:00.125Z".into(),
        refresh_after_seconds: 30,
        max_stale_seconds: 90,
        placements: vec![p],
    }
}
#[test]
fn source_cat_meets_profile_and_has_grounded_fixed_fit() {
    let fit = model::validate(CAT).unwrap();
    assert!(fit.scale > 0.1 && fit.scale <= 2.);
    assert!(fit.offset.iter().all(|v| v.is_finite()));
}
#[test]
fn utc_dates_are_calendar_checked_and_wall_clock_independent() {
    assert_eq!(timestamp("1970-01-01T00:00:00Z").unwrap(), 0.);
    assert_eq!(
        timestamp("2026-10-16T00:00:00+00:00").unwrap()
            - timestamp("2026-10-15T00:00:00Z").unwrap(),
        86400.
    );
    for date in [
        "2026-02-29T00:00:00Z",
        "2026-01-01T24:00:00Z",
        "2026-01-01T-1:00:00Z",
        "2026-10-15T00:00:00+08:00",
        "2026-10-15T00:00:00.Z",
        "é",
    ] {
        assert!(timestamp(date).is_err(), "{date}");
    }
    assert!(timestamp("2024-02-29T23:59:59.5Z").is_ok());
}
#[test]
fn unknown_slots_inexact_urls_and_expired_identity_fail_closed() {
    let base = origin("https://registry.example").unwrap();
    let p = placement("https://registry.example");
    let now = Instant::now();
    assert!(authorize(&base, manifest(p.clone()), now).is_ok());
    for bad in [
        "https://evil.test/model.glb",
        "https://registry.example/v1/studio/assets/other/rendition",
        "https://registry.example:443/v1/studio/assets/../../evil/rendition",
    ] {
        let mut item = p.clone();
        item.rendition_url = bad.into();
        assert!(authorize(&base, manifest(item), now).is_err());
    }
    let mut item = p.clone();
    item.slot_id = "dragon".into();
    assert!(authorize(&base, manifest(item), now).is_err());
    let mut item = p.clone();
    item.expires_at = "2026-10-15T12:00:00Z".into();
    assert!(authorize(&base, manifest(item), now).is_err());
    let mut item = p.clone();
    item.starts_at = "2026-10-14T00:00:00Z".into();
    assert!(authorize(&base, manifest(item), now).is_err());
    let mut m = manifest(p.clone());
    m.placements.push(p);
    assert!(authorize(&base, m, now).is_err());
    for url in [
        "http://registry.example",
        "https://user@registry.example",
        "https://registry.example/path",
        "https://registry.example?redirect=1",
    ] {
        assert!(origin(url).is_err());
    }
}
#[test]
fn revoked_expired_stale_and_late_downloads_cannot_reappear() {
    let now = Instant::now();
    let base = origin("https://registry.example").unwrap();
    let p = placement("https://registry.example");
    let ready = || DownloadResult {
        identity: p.clone(),
        result: Ok((
            "ekza://map-props/cat.glb".into(),
            model::validate(CAT).unwrap(),
        )),
    };
    let mut state = State::default();
    state.apply_read(evaluate_manifest(&base, manifest(p.clone()), now), now);
    state.apply_download(ready(), now);
    assert!(state.ready.is_some());
    state.apply_read(
        ReadResult {
            server_now: None,
            result: Err("offline".into()),
        },
        now + Duration::from_secs(40),
    );
    assert!(state.ready.is_some());
    state.expire(now + MAX_STALE);
    assert!(state.ready.is_none());
    state.apply_download(ready(), now + MAX_STALE);
    assert!(state.ready.is_none());
    let mut state = State::default();
    state.apply_read(evaluate_manifest(&base, manifest(p.clone()), now), now);
    state.apply_download(ready(), now);
    assert!(state.ready.is_some());
    state.apply_read(
        ReadResult {
            server_now: Some(timestamp("2026-10-15T12:01:00Z").unwrap()),
            result: Ok(None),
        },
        now,
    );
    state.apply_download(ready(), now);
    assert!(state.ready.is_none());
    let mut m = manifest(p.clone());
    m.server_now = "2026-10-15T23:59:58Z".into();
    state.apply_read(evaluate_manifest(&base, m, now), now);
    state.apply_download(ready(), now);
    state.expire(now + Duration::from_secs(2));
    assert!(state.ready.is_none());
    // Delayed server responses consume their own request duration, not a fresh lease.
    state.apply_read(
        evaluate_manifest(&base, manifest(p), now),
        now + Duration::from_secs(91),
    );
    assert!(state.authorization.is_none());
}
fn document(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let len = u32::from_le_bytes(CAT[12..16].try_into().unwrap()) as usize;
    let mut doc = serde_json::from_slice(&CAT[20..20 + len]).unwrap();
    edit(&mut doc);
    let mut json = serde_json::to_vec(&doc).unwrap();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let mut out = CAT[..12].to_vec();
    out[8..12].copy_from_slice(&((20 + json.len() + CAT.len() - 20 - len) as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend(json);
    out.extend_from_slice(&CAT[20 + len..]);
    out
}
#[test]
fn hostile_models_fail_before_loading() {
    use serde_json::json;
    for (key, value) in [
        ("images", json!([{"uri":"https://evil.test/x.png"}])),
        ("nodes", json!([{"mesh":0,"children":[0]}])),
        ("extensionsRequired", json!(["KHR_draco_mesh_compression"])),
        ("skins", json!([{}])),
        ("scene", json!(1)),
    ] {
        assert!(
            model::validate(&document(|d| d[key] = value)).is_err(),
            "{key}"
        );
    }
    assert!(
        model::validate(&document(
            |d| d["animations"][0]["channels"][0]["target"]["path"] = json!("translation")
        ))
        .is_err()
    );
    assert!(model::validate(&document(|d| d["accessors"][0]["count"] = json!(9999999))).is_err());
    assert!(model::validate(&CAT[..CAT.len() - 1]).is_err());
    let p = placement("https://registry.example");
    let mut corrupt = CAT.to_vec();
    let n = corrupt.len();
    corrupt[n - 1] ^= 1;
    assert!(verified(&corrupt, &p).is_err());
}
#[test]
fn real_http_manifest_and_atomic_install_verify_bytes_and_reject_redirects() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let p = placement(&base);
    let body = serde_json::to_vec(&manifest(p.clone())).unwrap();
    let server = std::thread::spawn(move || {
        for body in [body, CAT.to_vec()] {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = [0; 4096];
            assert!(s.read(&mut req).unwrap() > 0);
            write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            s.write_all(&body).unwrap();
        }
    });
    let base = origin(&base).unwrap();
    let c = client().unwrap();
    let auth = read_manifest(&c, &base, Instant::now())
        .result
        .unwrap()
        .unwrap();
    assert_eq!(auth.placement, p);
    let dir = std::env::temp_dir().join(format!(
        "omoba-map-prop-{}-{}",
        std::process::id(),
        base.port().unwrap()
    ));
    let (path, _) = install(&c, &base, &dir, &p).unwrap();
    assert!(path.starts_with("ekza://map-props/"));
    assert_eq!(
        fs::read(
            dir.join("map-props")
                .join(format!("{}.glb", p.rendition_sha256))
        )
        .unwrap(),
        CAT
    );
    server.join().unwrap();
    // Cached bytes are rechecked, authorization itself is never cached.
    assert!(install(&c, &base, &dir, &p).is_ok());
    fs::remove_dir_all(dir).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let server = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut req = [0; 4096];
        assert!(s.read(&mut req).unwrap() > 0);
        s.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://evil.test/model\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    assert!(fetch(&c, url, 1000).is_err());
    server.join().unwrap();
}

#[test]
fn replayed_server_time_cannot_extend_a_live_placement() {
    let now = Instant::now();
    let base = origin("https://registry.example").unwrap();
    let p = placement("https://registry.example");
    let mut m = manifest(p.clone());
    m.server_now = "2026-10-15T23:59:58Z".into();
    let mut state = State::default();
    state.apply_read(evaluate_manifest(&base, m, now), now);
    let mut replay = manifest(p);
    replay.server_now = "2026-10-15T23:59:58Z".into();
    state.apply_read(
        evaluate_manifest(&base, replay, now + Duration::from_secs(1)),
        now + Duration::from_secs(1),
    );
    state.expire(now + Duration::from_secs(2));
    assert!(state.authorization.is_none());
}

#[test]
fn replay_after_expiry_cannot_restart_a_grant() {
    let now = Instant::now();
    let base = origin("https://registry.example").unwrap();
    let short = || {
        let mut m = manifest(placement("https://registry.example"));
        m.server_now = "2026-10-15T23:59:58Z".into();
        m
    };
    let mut state = State::default();
    state.apply_read(evaluate_manifest(&base, short(), now), now);
    assert!(state.authorization.is_some());
    let later = now + Duration::from_secs(5);
    state.expire(later);
    assert!(state.authorization.is_none());
    state.apply_read(evaluate_manifest(&base, short(), later), later);
    assert!(state.authorization.is_none());
}
#[test]
fn empty_revocation_advances_watermark_and_rejects_an_unseen_older_snapshot() {
    let now = Instant::now();
    let base = origin("https://registry.example").unwrap();
    let p = placement("https://registry.example");
    let mut state = State::default();
    state.apply_read(evaluate_manifest(&base, manifest(p.clone()), now), now);
    assert!(state.authorization.is_some());
    let mut empty = manifest(p.clone());
    empty.server_now = "2026-10-15T12:01:00Z".into();
    empty.placements.clear();
    let later = now + Duration::from_secs(60);
    state.apply_read(evaluate_manifest(&base, empty, later), later);
    assert!(state.authorization.is_none());
    let mut stale = manifest(p.clone());
    stale.server_now = "2026-10-15T12:00:30Z".into();
    state.apply_read(evaluate_manifest(&base, stale, later), later);
    assert!(state.authorization.is_none());
    // A genuinely later snapshot may authorize a new placement.
    let mut fresh = manifest(p);
    fresh.server_now = "2026-10-15T12:01:30Z".into();
    fresh.placements[0].id = "44444444-4444-4444-8444-444444444444".into();
    state.apply_read(evaluate_manifest(&base, fresh, later), later);
    assert!(state.authorization.is_some());
}

#[test]
fn description_is_optional_exact_plain_text_with_scalar_and_control_limits() {
    let base = origin("https://registry.example").unwrap();
    let p = placement("https://registry.example");
    let now = Instant::now();
    let mut value = serde_json::to_value(manifest(p.clone())).unwrap();
    value["placements"][0]
        .as_object_mut()
        .unwrap()
        .remove("description");
    let old: Manifest = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(old.placements[0].description, "");
    assert!(authorize(&base, old, now).unwrap().is_some());
    for text in [
        "".into(),
        "  <b>招财猫</b>\nhttps://example.test\ne\u{301}  ".into(),
        "🐈".repeat(1_000),
    ] {
        value["placements"][0]["description"] = serde_json::Value::String(text.clone());
        let parsed: Manifest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            authorize(&base, parsed, now)
                .unwrap()
                .unwrap()
                .placement
                .description,
            text
        );
    }
    for text in [
        "🐈".repeat(1_001),
        "a\r\nb".into(),
        "a\t".into(),
        "a\0".into(),
        "a\u{7f}".into(),
        "a\u{80}".into(),
        "a\u{85}".into(),
        "a\u{9f}".into(),
    ] {
        value["placements"][0]["description"] = serde_json::Value::String(text);
        let parsed: Manifest = serde_json::from_value(value.clone()).unwrap();
        assert!(authorize(&base, parsed, now).is_err());
    }
    for bad in [
        serde_json::Value::Null,
        serde_json::json!(12),
        serde_json::json!([]),
        serde_json::json!({}),
    ] {
        value["placements"][0]["description"] = bad;
        assert!(serde_json::from_value::<Manifest>(value.clone()).is_err());
    }
    // serde's UTF-8 String cannot represent unpaired UTF-16 surrogates.
    let json = serde_json::to_string(&manifest(p))
        .unwrap()
        .replace("\"description\":\"\"", r#""description":"\ud800""#);
    assert!(serde_json::from_str::<Manifest>(&json).is_err());
}
