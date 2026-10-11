use ecdev_net::{
    client::{Client, ClientRequest, Connection, Proxy, Scheme, Target, Tls},
    http::Headers,
    server::{Config, Reply, Request, Response, Server},
};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

fn target(addr: SocketAddr, path: &str) -> Target {
    Target {
        scheme: Scheme::Http,
        host: addr.ip().to_string(),
        port: addr.port(),
        path_and_query: path.to_string(),
    }
}

fn app(req: &Request) -> Reply {
    match req.path.as_str() {
        "/echo" => Response::bytes(200, "application/octet-stream", req.body.clone())
            .with_header("X-Method", &req.method)
            .into(),
        "/big" => Response::bytes(200, "application/octet-stream", vec![b'x'; 200_000]).into(),
        "/host" => Response::text(200, req.headers.get("host").unwrap_or("-")).into(),
        _ => Response::text(404, "nope").into(),
    }
}

fn serve_app() -> (SocketAddr, ecdev_net::server::ShutdownHandle) {
    let server = Server::bind("127.0.0.1:0", Config::default()).unwrap();
    let addr = server.local_addr().unwrap();
    let handle = server.shutdown_handle().unwrap();
    thread::spawn(move || server.serve(app).unwrap());
    (addr, handle)
}

/// A one-shot server that sends fixed bytes and returns the request it read.
fn canned(reply: &'static [u8], linger: Duration) -> (SocketAddr, thread::JoinHandle<Vec<u8>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = s.read(&mut buf).unwrap_or(0);
            got.extend_from_slice(&buf[..n]);
            if n == 0 || got.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        s.write_all(reply).unwrap();
        thread::sleep(linger);
        got
    });
    (addr, h)
}

#[test]
fn get_post_and_sizes_round_trip_against_the_server() {
    let (addr, stop) = serve_app();
    let client = Client::builder()
        .user_agent("ecdev-test/1")
        .timeout(Duration::from_secs(10))
        .build();
    let r = client
        .send(&ClientRequest::new("GET", target(addr, "/host")))
        .unwrap();
    assert_eq!(
        (r.status, String::from_utf8_lossy(&r.body).into_owned()),
        (200, addr.to_string())
    );
    let mut post = ClientRequest::new("POST", target(addr, "/echo"));
    post.body = b"payload".to_vec();
    post.headers.append("Content-Type", "text/plain").unwrap();
    let r = client.send(&post).unwrap();
    assert_eq!(
        (r.status, r.body.as_slice(), r.headers.get("x-method")),
        (200, &b"payload"[..], Some("POST"))
    );
    let r = client
        .send(&ClientRequest::new("GET", target(addr, "/big")))
        .unwrap();
    assert_eq!(r.body.len(), 200_000);
    assert_eq!(
        client
            .send(&ClientRequest::new("GET", target(addr, "/missing")))
            .unwrap()
            .status,
        404
    );
    let head = client
        .send(&ClientRequest::new("HEAD", target(addr, "/big")))
        .unwrap();
    assert!(head.body.is_empty() && head.headers.get("content-length") == Some("200000"));
    let small = Client::builder().max_response_bytes(1000).build();
    assert!(
        matches!(small.send(&ClientRequest::new("GET", target(addr, "/big"))), Err(e) if format!("{e}").contains("TooLarge"))
    );
    stop.shutdown();
}

#[test]
fn response_framings_are_all_read() {
    for (reply, want) in [
        (&b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello"[..], &b"hello"[..]),
        (b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n", b"abcde"),
        (b"HTTP/1.0 200 OK\r\n\r\nuntil close", b"until close"),
        (b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok", b"ok"),
        (b"HTTP/1.1 204 No Content\r\n\r\n", b""),
    ] {
        let (addr, h) = canned(reply, Duration::from_millis(50));
        let r = Client::builder().timeout(Duration::from_secs(5)).build().send(&ClientRequest::new("GET", target(addr, "/"))).unwrap();
        assert_eq!(r.body, want, "{}", String::from_utf8_lossy(reply));
        let seen = String::from_utf8(h.join().unwrap()).unwrap();
        assert!(seen.starts_with("GET / HTTP/1.1\r\n") && seen.contains("\r\nConnection: close\r\n") && seen.contains("\r\nAccept: */*\r\n"), "{seen}");
    }
}

#[test]
fn broken_responses_are_errors_not_hangs() {
    for reply in [
        &b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort"[..],
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nab",
        b"garbage\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\nhello",
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\nhello",
        b"HTTP/1.1 101 Switching Protocols\r\n\r\n",
        b"",
    ] {
        let (addr, h) = canned(reply, Duration::from_millis(50));
        let started = Instant::now();
        let r = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .send(&ClientRequest::new("GET", target(addr, "/")));
        assert!(r.is_err(), "{}", String::from_utf8_lossy(reply));
        assert!(started.elapsed() < Duration::from_secs(4));
        let _ = h.join();
    }
}

#[test]
fn a_silent_server_hits_the_deadline_and_a_closed_port_is_a_connect_error() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let started = Instant::now();
    let e = Client::builder()
        .timeout(Duration::from_millis(400))
        .build()
        .send(&ClientRequest::new("GET", target(addr, "/")))
        .unwrap_err();
    assert!(e.is_timeout(), "{e}");
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(l);
    let e = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .send(&ClientRequest::new("GET", target(addr, "/")))
        .unwrap_err();
    assert!(e.is_connect(), "{e}");
}

#[test]
fn a_slow_drip_cannot_outlast_the_deadline() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut buf = [0u8; 1024];
        let _ = s.read(&mut buf);
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n");
        for _ in 0..100 {
            if s.write_all(b"x").is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    });
    let started = Instant::now();
    let e = Client::builder()
        .timeout(Duration::from_millis(600))
        .build()
        .send(&ClientRequest::new("GET", target(addr, "/")))
        .unwrap_err();
    assert!(e.is_timeout(), "{e}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn pinned_addresses_replace_name_resolution_and_host_stays_the_name() {
    let (addr, stop) = serve_app();
    let mut req = ClientRequest::new(
        "GET",
        Target {
            scheme: Scheme::Http,
            host: "shop.invalid".into(),
            port: addr.port(),
            path_and_query: "/host".into(),
        },
    );
    req.addrs = Some(vec![addr]);
    let r = Client::builder().build().send(&req).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&r.body),
        format!("shop.invalid:{}", addr.port())
    );
    stop.shutdown();
}

#[test]
fn an_http_proxy_gets_the_absolute_form_and_credentials() {
    let (paddr, h) = canned(
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi",
        Duration::from_millis(50),
    );
    let client = Client::builder()
        .proxy(Proxy {
            host: "127.0.0.1".into(),
            port: paddr.port(),
            auth: Some(("u".into(), "p:w".into())),
        })
        .build();
    let r = client
        .send(&ClientRequest::new(
            "GET",
            Target {
                scheme: Scheme::Http,
                host: "origin.example".into(),
                port: 8080,
                path_and_query: "/a?b=c".into(),
            },
        ))
        .unwrap();
    assert_eq!(r.body, b"hi");
    let seen = String::from_utf8(h.join().unwrap()).unwrap();
    assert!(
        seen.starts_with("GET http://origin.example:8080/a?b=c HTTP/1.1\r\n"),
        "{seen}"
    );
    assert!(seen.contains("Host: origin.example:8080\r\n"));
    assert!(
        seen.contains("Proxy-Authorization: Basic dTpwOnc=\r\n"),
        "{seen}"
    );
}

struct Passthrough(Mutex<Vec<String>>);
impl Tls for Passthrough {
    fn connect(
        &self,
        server_name: &str,
        stream: TcpStream,
    ) -> std::io::Result<Box<dyn Connection>> {
        self.0.lock().unwrap().push(server_name.to_string());
        Ok(Box::new(stream))
    }
}

#[test]
fn https_through_a_proxy_opens_a_connect_tunnel_then_hands_the_socket_to_tls() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let paddr = l.local_addr().unwrap();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut got = Vec::new();
        let mut b = [0u8; 1];
        while !got.ends_with(b"\r\n\r\n") {
            s.read_exact(&mut b).unwrap();
            got.push(b[0]);
        }
        s.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .unwrap();
        // The "tunnel" now speaks plain HTTP, since the test connector does not encrypt.
        let mut req = Vec::new();
        while !req.ends_with(b"\r\n\r\n") {
            s.read_exact(&mut b).unwrap();
            req.push(b[0]);
        }
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\ntunnel")
            .unwrap();
        (
            String::from_utf8(got).unwrap(),
            String::from_utf8(req).unwrap(),
        )
    });
    let tls = Arc::new(Passthrough(Mutex::new(Vec::new())));
    let client = Client::builder()
        .proxy(Proxy {
            host: "127.0.0.1".into(),
            port: paddr.port(),
            auth: None,
        })
        .tls(tls.clone())
        .timeout(Duration::from_secs(5))
        .build();
    let r = client
        .send(&ClientRequest::new(
            "GET",
            Target {
                scheme: Scheme::Https,
                host: "secure.example".into(),
                port: 443,
                path_and_query: "/p".into(),
            },
        ))
        .unwrap();
    assert_eq!(r.body, b"tunnel");
    let (connect, inner) = h.join().unwrap();
    assert!(
        connect.starts_with("CONNECT secure.example:443 HTTP/1.1\r\n")
            && connect.contains("Host: secure.example:443\r\n"),
        "{connect}"
    );
    assert!(
        inner.starts_with("GET /p HTTP/1.1\r\nHost: secure.example\r\n"),
        "{inner}"
    );
    assert_eq!(*tls.0.lock().unwrap(), vec!["secure.example".to_string()]);

    // A refused tunnel is a proxy error, and nothing is sent through it.
    let (paddr, h) = canned(b"HTTP/1.1 403 Forbidden\r\n\r\n", Duration::from_millis(50));
    let client = Client::builder()
        .proxy(Proxy {
            host: "127.0.0.1".into(),
            port: paddr.port(),
            auth: None,
        })
        .tls(tls)
        .build();
    let e = client
        .send(&ClientRequest::new(
            "GET",
            Target {
                scheme: Scheme::Https,
                host: "secure.example".into(),
                port: 443,
                path_and_query: "/".into(),
            },
        ))
        .unwrap_err();
    assert!(e.is_connect() && format!("{e}").contains("403"), "{e}");
    let _ = h.join();
}

#[test]
fn https_without_a_connector_fails_clearly_and_headers_cannot_be_injected() {
    let e = Client::builder()
        .build()
        .send(&ClientRequest::new(
            "GET",
            Target {
                scheme: Scheme::Https,
                host: "127.0.0.1".into(),
                port: 9,
                path_and_query: "/".into(),
            },
        ))
        .unwrap_err();
    assert!(
        format!("{e}").contains("Connect") || format!("{e}").contains("Tls"),
        "{e}"
    );
    let mut h = Headers::new();
    assert!(h.append("X-A", "b\r\nHost: evil").is_err());
}
