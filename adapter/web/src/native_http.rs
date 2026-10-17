//! Direct, credentialless HTTP only. Public-address admission and politeness stay in Web::request.
//! HTTPS and every operator proxy retain their existing transport and trust configuration.
use super::{MAX_BODY_BYTES, declared_length_exceeds, location_text, unsolicited_encoding};
use ecdev_core::{json, json::Value, web_address::Url};
use ecdev_net::client::{Client, ClientError, ClientRequest, ErrorKind, Scheme, Target};
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

pub(super) fn eligible(
    url: &Url,
    has_egress: bool,
    conditional: &Value,
    addresses: &[SocketAddr],
) -> bool {
    // Native Headers normalize outer whitespace. Retain the original backend for an unusual
    // validator whose exact wire bytes would otherwise change.
    let unchanged_validators = ["etag", "last_modified"].iter().all(|field| {
        conditional[*field]
            .as_str()
            .is_none_or(|v| v.trim_matches([' ', '\t']) == v)
    });
    // Both connectors try a single family sequentially. Mixed families retain the existing
    // backend's address-family racing; preserve the admitted address order for native HTTP.
    addresses.first().is_some_and(|first| {
        addresses
            .iter()
            .all(|address| address.is_ipv4() == first.is_ipv4())
    }) && unchanged_validators
        && url.scheme() == "http"
        && !has_egress
        && url.username().is_empty()
        && url.password().is_none()
}

// Retry only a known connection-establishment failure, once. No fallback transport can repeat
// an origin request after an ambiguous send/read failure.
fn connect_retry<T>(mut send: impl FnMut() -> Result<T, ClientError>) -> Result<(T, u8), String> {
    match send() {
        Ok(response) => Ok((response, 0)),
        Err(error) if error.is_connect() => send()
            .map(|r| (r, 1))
            .map_err(|_| "FETCH_NETWORK_ERROR".into()),
        Err(_) => Err("FETCH_NETWORK_ERROR".into()),
    }
}

pub(super) fn request(
    url: &Url,
    conditional: &Value,
    addresses: &[SocketAddr],
    timeout: Duration,
) -> Result<(u16, Vec<u8>, Value), String> {
    let mut path = url.path().to_string();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    let mut request = ClientRequest::new(
        "GET",
        Target {
            scheme: Scheme::Http,
            host: url
                .host_str()
                .ok_or("INVALID_HOST")?
                .trim_start_matches('[')
                .trim_end_matches(']')
                .into(),
            port: url.port_or_known_default().ok_or("INVALID_PORT")?,
            path_and_query: path,
        },
    );
    // Never resolve again: these are exactly the addresses admitted by the public-address guard.
    request.addrs = Some(addresses.to_vec());
    for (field, header) in [
        ("etag", "If-None-Match"),
        ("last_modified", "If-Modified-Since"),
    ] {
        if let Some(value) = conditional[field].as_str() {
            request
                .headers
                .append(header, value)
                .map_err(|_| "FETCH_NETWORK_ERROR")?;
        }
    }
    let client = Client::builder()
        .user_agent("ECDEV/0.1 (+read-only research)")
        .timeout(timeout)
        .connect_timeout(timeout)
        .max_response_fields(100)
        .max_response_bytes(MAX_BODY_BYTES as usize)
        .build();
    let sent = Instant::now();
    let (response, connect_retries) = connect_retry(|| client.send_head(&request))?;
    let status = response.status;
    let ttfb_ms = sent.elapsed().as_millis() as u64;
    let h = &response.headers;
    let mut headers = json!({"connect_retries":connect_retries,"ttfb_ms":ttfb_ms,
        "etag":h.get("etag"),"last_modified":h.get("last-modified"),
        "content_type":h.get("content-type"),"location":h.get_bytes("location").map(location_text),
        "retry_after":h.get("retry-after"),"cache_control":h.get("cache-control"),
        "expires":h.get("expires"),"date":h.get("date"),"age":h.get("age"),
        "content_length":h.get("content-length"),
        "received_at_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default().as_millis().min(i64::MAX as u128) as i64});
    if status != 200 {
        return Ok((status, vec![], headers));
    }
    if unsolicited_encoding(h.get_bytes("content-encoding")) {
        return Err("UNSOLICITED_CONTENT_ENCODING".into());
    }
    if declared_length_exceeds(headers["content_length"].as_str(), MAX_BODY_BYTES) {
        return Err("BODY_LIMIT_EXCEEDED".into());
    }
    let response = response.read_body().map_err(|error| {
        if matches!(error.kind, ErrorKind::TooLarge) {
            "BODY_LIMIT_EXCEEDED"
        } else {
            "BODY_READ_FAILED"
        }
    })?;
    headers["body_ms"] = json!(sent.elapsed().as_millis() as u64 - ttfb_ms);
    Ok((status, response.body, headers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Web;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::mpsc,
        thread,
    };

    type Exchange = (Result<(u16, Vec<u8>, Value), String>, String);

    fn read_get(socket: &mut TcpStream) -> Vec<u8> {
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut head = Vec::new();
        let mut byte = [0];
        while !head.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        head
    }

    fn assert_pinned_get(head: &[u8], port: u16) {
        let captured = std::str::from_utf8(head).unwrap();
        let lower = captured.to_ascii_lowercase();
        assert!(
            captured.starts_with("GET /a%2Fb?q=%2F HTTP/1.1\r\n"),
            "{captured}"
        );
        assert!(lower.contains(&format!("host: public-fake.invalid:{port}\r\n")));
        assert!(lower.contains("user-agent: ecdev/0.1 (+read-only research)\r\n"));
        assert!(lower.contains("if-none-match: \"v1\"\r\n"));
        assert!(lower.contains("if-modified-since: wed, 15 oct 2025 01:02:03 gmt\r\n"));
        assert!(!lower.contains("accept-encoding:") && !lower.contains("authorization:"));
    }

    fn pinned_request(
        native: bool,
        addresses: &[SocketAddr],
    ) -> Result<(u16, Vec<u8>, Value), String> {
        let url = Url::parse(&format!(
            "http://public-fake.invalid:{}/a%2Fb?q=%2F",
            addresses[0].port()
        ))
        .unwrap();
        let conditional = json!({"etag":"\"v1\"","last_modified":"Wed, 15 Oct 2025 01:02:03 GMT"});
        if native {
            request(&url, &conditional, addresses, Duration::from_secs(3))
        } else {
            Web::request_reqwest(&url, &conditional, &None, addresses)
        }
    }

    fn queued_connections(listener: &TcpListener) -> usize {
        listener.set_nonblocking(true).unwrap();
        let mut count = 0;
        loop {
            match listener.accept() {
                Ok(_) => count += 1,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return count,
                Err(e) => panic!("accept: {e}"),
            }
        }
    }

    fn exchange(wire: Vec<u8>, native: bool, query: &str) -> Exchange {
        exchange_host(wire, native, query, "pinned-source.invalid")
    }

    fn exchange_host(wire: Vec<u8>, native: bool, query: &str, host: &str) -> Exchange {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                assert_eq!(socket.read(&mut byte).unwrap(), 1);
                request.push(byte[0]);
            }
            // A refusal may close while a deliberately oversized body is still being sent.
            let _ = socket.write_all(&wire);
            String::from_utf8(request).unwrap()
        });
        // The test-only transport boundary uses loopback; the production entrypoint still
        // refuses it. A nonexistent public hostname proves both backends use the pinned address.
        let url = Url::parse(&format!("http://{host}:{}/a%2Fb{query}", addr.port())).unwrap();
        let conditional = json!({"etag":"\"v1\"","last_modified":"Wed, 15 Oct 2025 01:02:03 GMT"});
        let result = if native {
            request(&url, &conditional, &[addr], Duration::from_secs(3))
        } else {
            Web::request_reqwest(&url, &conditional, &None, &[addr])
        };
        let captured = server.join().unwrap();
        let lower = captured.to_ascii_lowercase();
        assert!(
            captured.starts_with(&format!("GET /a%2Fb{query} HTTP/1.1\r\n")),
            "{captured}"
        );
        assert!(lower.contains(&format!("host: {host}:{}\r\n", addr.port())));
        assert!(lower.contains("user-agent: ecdev/0.1 (+read-only research)\r\n"));
        assert!(lower.contains("if-none-match: \"v1\"\r\n"));
        assert!(lower.contains("if-modified-since: wed, 15 oct 2025 01:02:03 gmt\r\n"));
        assert!(!lower.contains("accept-encoding:") && !lower.contains("authorization:"));
        (result, captured)
    }

    fn stable(mut response: (u16, Vec<u8>, Value)) -> (u16, Vec<u8>, Value) {
        // Compare all source evidence; clock readings are intentionally measured per request.
        for key in ["ttfb_ms", "body_ms", "received_at_ms"] {
            if let Some(object) = response.2.as_object_mut() {
                object.remove(key);
            }
        }
        response
    }

    // Record every connection until the caller finishes, including an unintended HTTP replay.
    fn record_origin(
        listener: TcpListener,
        wire: Option<Vec<u8>>,
    ) -> (mpsc::Sender<()>, thread::JoinHandle<Vec<Vec<u8>>>) {
        let (done, wait) = mpsc::channel();
        listener.set_nonblocking(true).unwrap();
        let server = thread::spawn(move || {
            let mut heads = Vec::new();
            loop {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        heads.push(read_get(&mut socket));
                        if let Some(wire) = &wire {
                            socket.write_all(wire).unwrap();
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if wait.try_recv().is_ok() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(e) => panic!("accept: {e}"),
                }
            }
            heads
        });
        (done, server)
    }

    fn assert_differential_pinned(addresses: &[SocketAddr], listener: TcpListener) {
        let wire = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Type: text/html\r\nETag: \"source\"\r\nLast-Modified: Wed, 15 Oct 2025 01:02:03 GMT\r\nCache-Control: max-age=60\r\nExpires: Thu, 16 Oct 2025 01:02:03 GMT\r\nDate: Wed, 15 Oct 2025 01:02:03 GMT\r\nAge: 7\r\nConnection: close\r\n\r\nhello".to_vec();
        let (done, server) = record_origin(listener, Some(wire));
        let native = pinned_request(true, addresses).unwrap();
        let retained = pinned_request(false, addresses).unwrap();
        done.send(()).unwrap();
        let heads = server.join().unwrap();
        assert_eq!(heads.len(), 2);
        for head in heads {
            assert_pinned_get(&head, addresses[0].port());
        }
        assert_eq!(native.0, 200);
        assert_eq!(native.1, b"hello");
        assert_eq!(native.2["connect_retries"], 0);
        assert_eq!(stable(native), stable(retained));
    }

    #[test]
    fn same_family_refusal_then_success_matches_retained_pinned_transport() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let success = listener.local_addr().unwrap();
        let refused = SocketAddr::from(([127, 0, 0, 2], success.port()));
        // Confirm the first same-port address is available, then leave it without a listener.
        drop(TcpListener::bind(refused).unwrap());
        assert_differential_pinned(&[refused, success], listener);
    }

    #[test]
    fn same_family_first_success_never_connects_second_address() {
        let first = TcpListener::bind("127.0.0.1:0").unwrap();
        let first_addr = first.local_addr().unwrap();
        let second =
            TcpListener::bind(SocketAddr::from(([127, 0, 0, 2], first_addr.port()))).unwrap();
        let addresses = [first_addr, second.local_addr().unwrap()];
        assert_differential_pinned(&addresses, first);
        assert_eq!(queued_connections(&second), 0);
    }

    #[test]
    fn same_family_origin_close_never_replays_or_connects_second_address() {
        for native in [true, false] {
            let first = TcpListener::bind("127.0.0.1:0").unwrap();
            let first_addr = first.local_addr().unwrap();
            let second =
                TcpListener::bind(SocketAddr::from(([127, 0, 0, 2], first_addr.port()))).unwrap();
            let addresses = [first_addr, second.local_addr().unwrap()];
            let (done, server) = record_origin(first, None);
            assert_eq!(
                pinned_request(native, &addresses).unwrap_err(),
                "FETCH_NETWORK_ERROR"
            );
            assert_eq!(queued_connections(&second), 0);
            done.send(()).unwrap();
            let heads = server.join().unwrap();
            assert_eq!(
                heads.len(),
                1,
                "backend native={native} must send exactly one GET"
            );
            assert_pinned_get(&heads[0], first_addr.port());
        }
    }

    #[test]
    fn same_family_actual_ipv6_pinned_transport_matches_retained() {
        let listener = TcpListener::bind("[::1]:0").unwrap();
        let addr = listener.local_addr().unwrap();
        assert_differential_pinned(&[addr, addr], listener);
    }

    #[test]
    fn same_family_fallback_keeps_one_absolute_deadline_for_head_and_body() {
        for body_stall in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let refused = SocketAddr::from(([127, 0, 0, 2], addr.port()));
            drop(TcpListener::bind(refused).unwrap());
            let (done, wait) = mpsc::channel();
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let head = read_get(&mut socket);
                if body_stall {
                    // Receiving the head spends most of the original exchange budget.
                    thread::sleep(Duration::from_millis(250));
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
                        .unwrap();
                } else {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nX-Slow: ").unwrap();
                }
                // Progress faster than the timeout must not renew it for each read.
                for _ in 0..16 {
                    if wait.recv_timeout(Duration::from_millis(75)).is_ok()
                        || socket.write_all(b"x").is_err()
                    {
                        break;
                    }
                }
                head
            });
            let url = Url::parse(&format!("http://public-fake.invalid:{}/", addr.port())).unwrap();
            let started = Instant::now();
            let error = request(
                &url,
                &Value::Null,
                &[refused, addr],
                Duration::from_millis(400),
            )
            .unwrap_err();
            let elapsed = started.elapsed();
            let _ = done.send(());
            let head = server.join().unwrap();
            assert!(head.starts_with(b"GET / HTTP/1.1\r\n"));
            assert_eq!(
                error,
                if body_stall {
                    "BODY_READ_FAILED"
                } else {
                    "FETCH_NETWORK_ERROR"
                }
            );
            assert!(elapsed >= Duration::from_millis(300), "{elapsed:?}");
            assert!(
                elapsed < Duration::from_millis(600),
                "deadline renewed: {elapsed:?}"
            );
        }
    }

    #[test]
    fn same_family_fallback_holding_head_does_not_renew_body_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let refused = SocketAddr::from(([127, 0, 0, 2], addr.port()));
        drop(TcpListener::bind(refused).unwrap());
        let (done, wait) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            read_get(&mut socket);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
                .unwrap();
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
        });
        let mut req = ClientRequest::new(
            "GET",
            Target {
                scheme: Scheme::Http,
                host: "public-fake.invalid".into(),
                port: addr.port(),
                path_and_query: "/".into(),
            },
        );
        req.addrs = Some(vec![refused, addr]);
        let response = Client::builder()
            .timeout(Duration::from_millis(200))
            .connect_timeout(Duration::from_millis(200))
            .build()
            .send_head(&req)
            .unwrap();
        assert_eq!(response.status, 200);
        thread::sleep(Duration::from_millis(250));
        assert!(response.read_body().unwrap_err().is_timeout());
        done.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn direct_http_matches_retained_transport_on_recorded_wire() {
        for wire in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Type: text/html\r\nETag: \"one\"\r\nETag: \"two\"\r\nCache-Control: max-age=60\r\n\r\nhello".to_vec(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Encoding: identity\r\n\r\n2\r\nhe\r\n3\r\nllo\r\n0\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nEOF body".to_vec(),
            // The truncated unused body must not erase a redirect/rate/validator response.
            b"HTTP/1.1 302 Found\r\nLocation: /\xe6\x97\xa5\r\nContent-Length: 5000\r\n\r\n".to_vec(),
            b"HTTP/1.1 429 Slow\r\nRetry-After: 13\r\nContent-Length: 5000\r\n\r\n".to_vec(),
            b"HTTP/1.1 304 Not Modified\r\nETag: \"v1\"\r\n\r\n".to_vec(),
            b"HTTP/1.1 304 Not Modified\r\nContent-Length: garbage\r\n\r\n".to_vec(),
            b"HTTP/1.1 204 No Content\r\nContent-Length: garbage\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n\x00\xffx".to_vec(),
            b"HTTP/1.1 200 OK\r\nETag: \xff\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ] {
            for query in ["?q=%2F", "?"] {
                let native = exchange(wire.clone(), true, query).0.unwrap();
                let retained = exchange(wire.clone(), false, query).0.unwrap();
                assert!(native.2["ttfb_ms"].as_u64().is_some());
                assert!(native.2["received_at_ms"].as_i64().is_some());
                if native.0 == 200 {
                    assert!(native.2["body_ms"].as_u64().is_some());
                } else {
                    assert!(native.1.is_empty() && native.2["body_ms"].is_null());
                }
                assert_eq!(stable(native), stable(retained));
            }
        }
    }

    #[test]
    fn pinned_ipv6_authority_is_bracketed_once() {
        let result = exchange_host(
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec(),
            true,
            "",
            "[2606:4700:4700::1111]",
        )
        .0
        .unwrap();
        assert_eq!(result.0, 200);
    }

    #[test]
    fn direct_http_refusals_preserve_body_and_encoding_bounds() {
        let oversized = vec![b'x'; MAX_BODY_BYTES as usize + 1];
        let mut eof = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        eof.extend_from_slice(&oversized);
        let mut chunked = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
            oversized.len()
        )
        .into_bytes();
        chunked.extend_from_slice(&oversized);
        chunked.extend_from_slice(b"\r\n0\r\n\r\n");
        for (wire, expected) in [
            (
                b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 100\r\n\r\n"
                    .to_vec(),
                "UNSOLICITED_CONTENT_ENCODING",
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 4194305\r\n\r\n".to_vec(),
                "BODY_LIMIT_EXCEEDED",
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort".to_vec(),
                "BODY_READ_FAILED",
            ),
            (eof, "BODY_LIMIT_EXCEEDED"),
            (chunked, "BODY_LIMIT_EXCEEDED"),
        ] {
            assert_eq!(exchange(wire.clone(), true, "").0.unwrap_err(), expected);
            assert_eq!(exchange(wire, false, "").0.unwrap_err(), expected);
        }
    }

    #[test]
    fn direct_http_accepts_the_exact_body_limit() {
        let mut wire =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {MAX_BODY_BYTES}\r\n\r\n").into_bytes();
        wire.extend(vec![b'x'; MAX_BODY_BYTES as usize]);
        let native = exchange(wire.clone(), true, "").0.unwrap();
        let retained = exchange(wire, false, "").0.unwrap();
        assert_eq!(native.1.len(), MAX_BODY_BYTES as usize);
        assert_eq!(stable(native), stable(retained));
    }

    #[test]
    fn an_origin_close_after_request_is_never_retried() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut head = Vec::new();
            let mut byte = [0];
            while !head.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            drop(socket);
            // Keep the listening socket alive through the attempted retry window.
            thread::sleep(Duration::from_millis(150));
            listener.set_nonblocking(true).unwrap();
            assert!(
                matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock)
            );
            head
        });
        let url = Url::parse(&format!("http://pinned-source.invalid:{}/", addr.port())).unwrap();
        assert_eq!(
            request(&url, &Value::Null, &[addr], Duration::from_secs(1)).unwrap_err(),
            "FETCH_NETWORK_ERROR"
        );
        assert!(server.join().unwrap().starts_with(b"GET / HTTP/1.1\r\n"));
    }

    #[test]
    fn native_http_retains_strict_framing_refusals() {
        let mut too_many = b"HTTP/1.1 429 Slow\r\n".to_vec();
        for _ in 0..101 {
            too_many.extend_from_slice(b"X-Field: x\r\n");
        }
        too_many.extend_from_slice(b"\r\n");
        assert_eq!(
            exchange(too_many.clone(), true, "").0.unwrap_err(),
            "FETCH_NETWORK_ERROR"
        );
        assert_eq!(
            exchange(too_many, false, "").0.unwrap_err(),
            "FETCH_NETWORK_ERROR"
        );
        // The native parser's deliberate stricter contract is not universal hyper parity.
        for wire in [
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 0\r\n\r\n0\r\n\r\n"
                .as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\nx",
        ] {
            assert_eq!(
                exchange(wire.to_vec(), true, "").0.unwrap_err(),
                "FETCH_NETWORK_ERROR"
            );
        }
        for status in [200, 429] {
            let wire =
                format!("HTTP/1.1 {status} Result\r\nContent-Length: garbage\r\n\r\n").into_bytes();
            assert_eq!(
                exchange(wire.clone(), true, "").0.unwrap_err(),
                "FETCH_NETWORK_ERROR"
            );
            assert_eq!(
                exchange(wire, false, "").0.unwrap_err(),
                "FETCH_NETWORK_ERROR"
            );
        }
    }

    #[test]
    fn native_http_has_no_proxy_tls_or_credential_dispatch() {
        let v4: SocketAddr = "93.184.216.34:80".parse().unwrap();
        let other_v4: SocketAddr = "93.184.216.35:80".parse().unwrap();
        let v6: SocketAddr = "[2606:4700:4700::1111]:80".parse().unwrap();
        let other_v6: SocketAddr = "[2606:4700:4700::1001]:80".parse().unwrap();
        let http = Url::parse("http://example.com/").unwrap();
        for addresses in [vec![v4], vec![v4, other_v4], vec![v6], vec![v6, other_v6]] {
            assert!(eligible(&http, false, &Value::Null, &addresses));
        }
        for addresses in [vec![], vec![v4, v6], vec![v6, v4]] {
            assert!(!eligible(&http, false, &Value::Null, &addresses));
        }
        for (url, proxy, expected) in [
            ("http://example.com/", false, true),
            ("https://example.com/", false, false),
            ("http://example.com/", true, false),
            ("https://example.com/", true, false),
            ("http://user:password@example.com/", false, false),
            ("http://user@example.com/", false, false),
        ] {
            for addresses in [[v4, other_v4], [v6, other_v6]] {
                assert_eq!(
                    eligible(&Url::parse(url).unwrap(), proxy, &Value::Null, &addresses),
                    expected
                );
            }
        }
        for conditional in [
            json!({"etag":" \"v1\" "}),
            json!({"etag":"\t\"v1\""}),
            json!({"last_modified":"Wed, 15 Oct 2025 01:02:03 GMT\t"}),
        ] {
            for addresses in [[v4, other_v4], [v6, other_v6]] {
                assert!(!eligible(&http, false, &conditional, &addresses));
            }
        }
        let web = Web::default();
        for scheme in ["http", "https"] {
            let url = Url::parse(&format!("{scheme}://127.0.0.1:9/")).unwrap();
            assert_eq!(
                web.request(&url, &Value::Null, Duration::ZERO).unwrap_err(),
                "PRIVATE_OR_SPECIAL_ADDRESS_DENIED"
            );
            assert!(web.gate.lock().unwrap().is_none());
        }
    }

    #[test]
    fn retry_is_bounded_and_never_repeats_unknown_effects() {
        for (kind, expected_attempts) in [
            (ErrorKind::Connect, 2),
            (ErrorKind::Timeout, 1),
            (ErrorKind::Io, 1),
            (ErrorKind::Protocol, 1),
        ] {
            let mut attempts = 0;
            let result: Result<((), u8), String> = connect_retry(|| {
                attempts += 1;
                Err(ClientError {
                    kind: match kind {
                        ErrorKind::Connect => ErrorKind::Connect,
                        ErrorKind::Timeout => ErrorKind::Timeout,
                        ErrorKind::Io => ErrorKind::Io,
                        _ => ErrorKind::Protocol,
                    },
                    message: "recorded failure".into(),
                })
            });
            assert_eq!(result.unwrap_err(), "FETCH_NETWORK_ERROR");
            assert_eq!(attempts, expected_attempts);
        }
        let mut attempts = 0;
        let (value, retries) = connect_retry(|| {
            attempts += 1;
            if attempts == 1 {
                Err(ClientError {
                    kind: ErrorKind::Connect,
                    message: "not established".into(),
                })
            } else {
                Ok(7)
            }
        })
        .unwrap();
        assert_eq!((value, retries, attempts), (7, 1, 2));
    }
}
