use super::*;
use std::{io::Write, net::TcpListener, thread, time::Instant};

fn fixture(reply: Vec<u8>) -> (DirectoryTransport, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let worker = thread::spawn(move || {
        let started = Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        started.elapsed() < Duration::from_secs(5),
                        "fixture not called"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("fixture accept: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            assert!(request.len() < 8192);
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        // Oversize clients may close before the fixture finishes its write.
        let _ = stream.write_all(&reply);
        String::from_utf8(request).unwrap()
    });
    (
        DirectoryTransport::at_origin(&origin, true).unwrap(),
        worker,
    )
}
fn reply(body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
    out.extend(body);
    out
}

#[test]
fn origins_are_fixed_regional_https_with_no_ambient_service_override() {
    for (region, expected) in [
        (RegionId::Ru, "https://omoba.ru/api/game/directory"),
        (RegionId::Global, "https://app.omoba.io/api/game/directory"),
    ] {
        assert_eq!(
            DirectoryTransport::for_region(region, None)
                .unwrap()
                .url
                .as_str(),
            expected
        );
    }
    for origin in [
        "http://omoba.ru",
        "https://user:secret@omoba.ru",
        "https://omoba.ru?region=global",
        "https://omoba.ru/#fragment",
        "https://omoba.ru/untrusted",
        "http://127.0.0.1:3010",
    ] {
        assert!(
            DirectoryTransport::at_origin(origin, false).is_err(),
            "{origin}"
        );
    }
    for contour in ["", "../production", "dev.omoba.ru", "dev:443"] {
        assert!(DirectoryTransport::for_region(RegionId::Ru, Some(contour)).is_err());
    }
}
#[test]
fn actual_http_preserves_unicode_bytes_without_account_credentials() {
    let bytes = "{\"payload\":\"Проверка ⚔\",\"sequence\":18446744073709551615}\n".as_bytes();
    let (transport, worker) = fixture(reply(bytes));
    assert_eq!(transport.fetch().unwrap(), bytes);
    let request = worker.join().unwrap().to_ascii_lowercase();
    assert!(request.starts_with("get /api/game/directory http/1.1\r\n"));
    assert!(request.contains("accept: application/json\r\n"));
    for header in [
        "authorization:",
        "cookie:",
        "x-omoba-portal-key:",
        "referer:",
        "origin:",
    ] {
        assert!(!request.contains(header), "{header}");
    }
}
#[test]
fn redirects_errors_html_and_truncated_bodies_are_never_usable() {
    for response in [
        b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/private\r\nContent-Length: 0\r\n\r\n".to_vec(),
        b"HTTP/1.1 503 Unavailable\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}".to_vec(),
    ] {
        let (transport, worker) = fixture(response);
        assert!(transport.fetch().is_err());
        worker.join().unwrap();
    }
}
#[test]
fn real_streams_are_bounded_with_declared_missing_and_chunked_lengths() {
    let oversized = vec![b'x'; MAX_WIRE_BYTES + 1];
    let mut missing =
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n".to_vec();
    missing.extend(&oversized);
    let mut chunked = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    chunked.extend(format!("{:x}\r\n", oversized.len()).as_bytes());
    chunked.extend(&oversized);
    chunked.extend(b"\r\n0\r\n\r\n");
    for response in [reply(&oversized), missing, chunked] {
        let (transport, worker) = fixture(response);
        assert!(transport.fetch().is_err());
        worker.join().unwrap();
    }
    let exact = vec![b'x'; MAX_WIRE_BYTES];
    let (transport, worker) = fixture(reply(&exact));
    assert_eq!(transport.fetch().unwrap(), exact);
    worker.join().unwrap();
}
