//! A threaded HTTP/1.1 server: one thread per connection, persistent connections, request bodies
//! buffered under a limit, streamed responses for event streams, and a deadline on every read so
//! a slow client cannot hold a thread.

use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crate::http::{self, ChunkedWriter, Error, Headers, Limits};

/// A parsed request with its body.
#[derive(Debug)]
pub struct Request {
    pub method: String,
    /// The request target as sent (origin form).
    pub target: String,
    /// The target before any `?`.
    pub path: String,
    /// The target after the first `?`.
    pub query: Option<String>,
    pub minor: u8,
    pub headers: Headers,
    pub body: Vec<u8>,
    pub peer: SocketAddr,
}

/// A complete response.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16) -> Response {
        Response {
            status,
            headers: Headers::new(),
            body: Vec::new(),
        }
    }

    pub fn bytes(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Response {
        let mut r = Response::new(status);
        r.headers
            .append("Content-Type", content_type)
            .expect("content type is a valid field value");
        r.body = body.into();
        r
    }

    pub fn text(status: u16, body: impl Into<String>) -> Response {
        Response::bytes(
            status,
            "text/plain; charset=utf-8",
            body.into().into_bytes(),
        )
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Response {
        self.headers
            .set(name, value)
            .expect("caller supplies a valid header");
        self
    }
}

/// The write side of a streamed response.
pub struct Stream<'a> {
    out: StreamOut<'a>,
    stop: &'a AtomicBool,
}

enum StreamOut<'a> {
    Chunked(ChunkedWriter<&'a mut TcpStream>),
    Raw(&'a mut TcpStream),
}

impl Stream<'_> {
    /// Sends bytes now, unless shutdown has been observed. Write failures reach the producer.
    pub fn send(&mut self, data: &[u8]) -> io::Result<()> {
        if self.should_stop() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "server shutdown",
            ));
        }
        match &mut self.out {
            StreamOut::Chunked(w) => w.chunk(data),
            StreamOut::Raw(w) => {
                w.write_all(data)?;
                w.flush()
            }
        }
    }

    /// True once the server has been asked to shut down; a producer should return.
    pub fn should_stop(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

type Producer = Box<dyn FnOnce(&mut Stream<'_>) -> io::Result<()> + Send>;

pub enum Reply {
    Response(Response),
    /// A response whose body is produced over time. The connection closes when it ends.
    Stream {
        status: u16,
        headers: Headers,
        produce: Producer,
    },
}

impl From<Response> for Reply {
    fn from(r: Response) -> Reply {
        Reply::Response(r)
    }
}

pub trait Handler: Send + Sync + 'static {
    fn handle(&self, request: &Request) -> Reply;
}

impl<F: Fn(&Request) -> Reply + Send + Sync + 'static> Handler for F {
    fn handle(&self, request: &Request) -> Reply {
        self(request)
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub max_connections: usize,
    /// How long an idle persistent connection may wait for its next request.
    pub idle_timeout: Duration,
    /// How long a request may take to arrive once its first byte has.
    pub request_timeout: Duration,
    pub write_timeout: Duration,
    pub limits: Limits,
    pub max_requests_per_connection: usize,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            max_connections: 256,
            idle_timeout: Duration::from_secs(30),
            request_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(30),
            limits: Limits {
                max_body: 16 * 1024 * 1024,
                ..Limits::default()
            },
            max_requests_per_connection: 1000,
        }
    }
}

pub struct Server {
    listener: TcpListener,
    config: Config,
    shutdown: Arc<AtomicBool>,
}

/// Asks a running server to stop accepting and to end its streams.
#[derive(Clone)]
pub struct ShutdownHandle {
    flag: Arc<AtomicBool>,
    addr: SocketAddr,
}

impl ShutdownHandle {
    pub fn shutdown(&self) {
        self.flag.store(true, Ordering::SeqCst);
        // Wake the accept call.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1));
    }
}

impl Server {
    pub fn bind(addr: impl ToSocketAddrs, config: Config) -> io::Result<Server> {
        Ok(Server {
            listener: TcpListener::bind(addr)?,
            config,
            shutdown: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn shutdown_handle(&self) -> io::Result<ShutdownHandle> {
        Ok(ShutdownHandle {
            flag: self.shutdown.clone(),
            addr: self.listener.local_addr()?,
        })
    }

    /// Accepts connections until shut down, then waits briefly for the open ones to finish.
    pub fn serve<H: Handler>(self, handler: H) -> io::Result<()> {
        let handler = Arc::new(handler);
        let config = Arc::new(self.config);
        let active = Arc::new(AtomicUsize::new(0));
        for incoming in self.listener.incoming() {
            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }
            let mut stream = match incoming {
                Ok(s) => s,
                Err(_) => {
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
            };
            if active.fetch_add(1, Ordering::SeqCst) >= config.max_connections {
                active.fetch_sub(1, Ordering::SeqCst);
                let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                let _ = stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
                continue;
            }
            let (handler, config, shutdown, active) = (
                handler.clone(),
                config.clone(),
                self.shutdown.clone(),
                active.clone(),
            );
            let spawned = thread::Builder::new().stack_size(4 * 1024 * 1024).spawn({
                let active = active.clone();
                move || {
                    let _ = connection(stream, &*handler, &config, &shutdown);
                    active.fetch_sub(1, Ordering::SeqCst);
                }
            });
            if spawned.is_err() {
                active.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let until = Instant::now() + Duration::from_secs(2);
        while active.load(Ordering::SeqCst) > 0 && Instant::now() < until {
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}

/// A reader that enforces one deadline across every read of a message.
struct Timed {
    stream: TcpStream,
    deadline: Option<Instant>,
}

impl Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(d) = self.deadline {
            let left = d.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "deadline passed"));
            }
            self.stream.set_read_timeout(Some(left))?;
        }
        self.stream.read(buf)
    }
}

fn is_timeout(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

fn error_response(e: &Error) -> Option<(u16, &'static str)> {
    match e {
        Error::Closed => None,
        Error::Io(e) if is_timeout(e) => Some((408, "request timed out")),
        Error::Io(_) => None,
        Error::Malformed(_) => Some((400, "malformed request")),
        Error::TooLarge("body") => Some((413, "request body too large")),
        Error::TooLarge(_) => Some((431, "request header fields too large")),
        Error::Unsupported("HTTP version") => Some((505, "HTTP version not supported")),
        Error::Unsupported(_) => Some((501, "not implemented")),
    }
}

fn write_simple(stream: &mut TcpStream, status: u16, message: &str) {
    let mut h = Headers::new();
    let _ = h.append("Content-Type", "text/plain; charset=utf-8");
    let _ = h.append("Content-Length", message.len().to_string());
    let _ = h.append("Connection", "close");
    let mut out = Vec::new();
    let _ = http::write_response_head(&mut out, 1, status, &h);
    out.extend_from_slice(message.as_bytes());
    let _ = stream.write_all(&out);
}

fn connection(
    stream: TcpStream,
    handler: &dyn Handler,
    config: &Config,
    shutdown: &Arc<AtomicBool>,
) -> io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(config.write_timeout))?;
    let peer = stream.peer_addr()?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::with_capacity(
        16 * 1024,
        Timed {
            stream,
            deadline: None,
        },
    );
    let mut served = 0;
    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        // Wait for the first byte of the next request under the idle timeout.
        reader.get_mut().deadline = Some(Instant::now() + config.idle_timeout);
        match reader.fill_buf() {
            Ok([]) | Err(_) => break,
            Ok(_) => {}
        }
        reader.get_mut().deadline = Some(Instant::now() + config.request_timeout);
        let head = match http::read_request_head(&mut reader, &config.limits) {
            Ok(Some(h)) => h,
            Ok(None) => break,
            Err(e) => {
                if let Some((status, msg)) = error_response(&e) {
                    write_simple(&mut writer, status, msg);
                }
                break;
            }
        };
        // HTTP/1.1 requires exactly one Host field.
        if head.minor == 1 && head.headers.get_all("host").count() != 1 {
            write_simple(&mut writer, 400, "exactly one Host field is required");
            break;
        }
        let framing = match http::request_framing(&head.headers) {
            Ok(f) => f,
            Err(e) => {
                if let Some((status, msg)) = error_response(&e) {
                    write_simple(&mut writer, status, msg);
                }
                break;
            }
        };
        if head.minor == 1
            && head
                .headers
                .list("expect")
                .iter()
                .any(|v| v == "100-continue")
            && framing != http::Framing::None
        {
            writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
        }
        let body = match http::read_body(&mut reader, framing, config.limits.max_body) {
            Ok(b) => b,
            Err(e) => {
                if let Some((status, msg)) = error_response(&e) {
                    write_simple(&mut writer, status, msg);
                }
                break;
            }
        };
        let (path, query) = match head.target.split_once('?') {
            Some((p, q)) => (p.to_string(), Some(q.to_string())),
            None => (head.target.clone(), None),
        };
        if !head.target.starts_with('/') && head.target != "*" {
            write_simple(
                &mut writer,
                400,
                "only origin-form request targets are accepted",
            );
            break;
        }
        let connection_tokens = head.headers.list("connection");
        let wants_close = connection_tokens.iter().any(|t| t == "close")
            || (head.minor == 0 && !connection_tokens.iter().any(|t| t == "keep-alive"));
        let is_head = head.method == "HEAD";
        let minor = head.minor;
        let request = Request {
            method: head.method,
            target: head.target,
            path,
            query,
            minor,
            headers: head.headers,
            body,
            peer,
        };
        served += 1;
        let close = wants_close
            || served >= config.max_requests_per_connection
            || shutdown.load(Ordering::Relaxed);
        match handler.handle(&request) {
            Reply::Response(mut r) => {
                let mut out = Vec::with_capacity(r.body.len() + 256);
                let no_body =
                    is_head || r.status == 204 || r.status == 304 || (100..200).contains(&r.status);
                r.headers.remove("transfer-encoding");
                r.headers.remove("connection");
                if r.status != 204 && !(100..200).contains(&r.status) {
                    r.headers
                        .set("Content-Length", r.body.len().to_string())
                        .expect("a number is a valid value");
                }
                if close {
                    r.headers.set("Connection", "close").expect("valid");
                } else if minor == 0 {
                    r.headers.set("Connection", "keep-alive").expect("valid");
                }
                http::write_response_head(
                    &mut out,
                    if minor == 0 { 0 } else { 1 },
                    r.status,
                    &r.headers,
                )?;
                if !no_body {
                    out.extend_from_slice(&r.body);
                }
                writer.write_all(&out)?;
                writer.flush()?;
            }
            Reply::Stream {
                status,
                mut headers,
                produce,
            } => {
                headers.remove("content-length");
                headers.remove("transfer-encoding");
                headers.set("Connection", "close").expect("valid");
                if minor == 1 {
                    headers.set("Transfer-Encoding", "chunked").expect("valid");
                }
                let mut out = Vec::new();
                http::write_response_head(
                    &mut out,
                    if minor == 0 { 0 } else { 1 },
                    status,
                    &headers,
                )?;
                writer.write_all(&out)?;
                writer.flush()?;
                if !is_head {
                    let mut s = Stream {
                        out: if minor == 1 {
                            StreamOut::Chunked(ChunkedWriter::new(&mut writer))
                        } else {
                            StreamOut::Raw(&mut writer)
                        },
                        stop: shutdown,
                    };
                    let produced = produce(&mut s);
                    if let (Ok(()), StreamOut::Chunked(w)) = (produced, s.out) {
                        let _ = w.finish();
                    }
                }
                break;
            }
        }
        if close {
            break;
        }
    }
    let _ = writer.shutdown(Shutdown::Both);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn start(
        config: Config,
        handler: impl Handler,
    ) -> (SocketAddr, ShutdownHandle, thread::JoinHandle<()>) {
        let server = Server::bind("127.0.0.1:0", config).unwrap();
        let addr = server.local_addr().unwrap();
        let handle = server.shutdown_handle().unwrap();
        let join = thread::spawn(move || {
            server.serve(handler).unwrap();
        });
        (addr, handle, join)
    }

    fn echo(req: &Request) -> Reply {
        match req.path.as_str() {
            "/echo" => Response::bytes(200, "application/octet-stream", req.body.clone()).into(),
            "/who" => {
                Response::text(200, format!("{} {} {:?}", req.method, req.path, req.query)).into()
            }
            "/stream" => {
                let mut h = Headers::new();
                h.append("Content-Type", "text/event-stream").unwrap();
                Reply::Stream {
                    status: 200,
                    headers: h,
                    produce: Box::new(|s| {
                        for i in 0..3 {
                            s.send(format!("data: {i}\n\n").as_bytes())?;
                        }
                        Ok(())
                    }),
                }
            }
            _ => Response::text(404, "no").into(),
        }
    }

    fn exchange(addr: SocketAddr, wire: &[u8]) -> String {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(wire).unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    #[test]
    fn requests_are_served_and_connections_persist() {
        let (addr, stop, join) = start(Config::default(), echo);
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        // Two pipelined requests on one connection, the second closing it.
        s.write_all(b"GET /who?x=1 HTTP/1.1\r\nHost: a\r\n\r\nPOST /echo HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        assert!(out.contains("GET /who Some(\"x=1\")"), "{out}");
        assert!(out.matches("HTTP/1.1 200 OK").count() == 2, "{out}");
        assert!(out.ends_with("hello"), "{out}");
        stop.shutdown();
        join.join().unwrap();
    }

    #[test]
    fn chunked_request_bodies_and_expect_continue() {
        let (addr, stop, join) = start(Config::default(), echo);
        let out = exchange(addr, b"POST /echo HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n");
        assert!(out.ends_with("abcde"), "{out}");
        let out = exchange(addr, b"POST /echo HTTP/1.1\r\nHost: a\r\nExpect: 100-continue\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi");
        assert!(
            out.starts_with("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK"),
            "{out}"
        );
        stop.shutdown();
        join.join().unwrap();
    }

    #[test]
    fn bad_requests_get_the_right_status() {
        let (addr, stop, join) = start(Config::default(), echo);
        for (wire, status) in [
            (&b"GET / HTTP/1.1\r\n\r\n"[..], "400"),
            (b"GET / HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n", "400"),
            (b"GET / HTTP/1.1\nHost: a\r\n\r\n", "400"),
            (b"GET http://a/ HTTP/1.1\r\nHost: a\r\n\r\n", "400"),
            (b"POST /echo HTTP/1.1\r\nHost: a\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n", "400"),
            (b"POST /echo HTTP/1.1\r\nHost: a\r\nContent-Length: 4\r\nContent-Length: 5\r\n\r\n", "400"),
            (b"GET / HTTP/2.0\r\nHost: a\r\n\r\n", "505"),
            (b"POST /echo HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: gzip, chunked\r\n\r\n", "501"),
            (b"POST /echo HTTP/1.1\r\nHost: a\r\nContent-Length: 99999999999\r\n\r\n", "413"),
        ] {
            let out = exchange(addr, wire);
            assert!(out.starts_with(&format!("HTTP/1.1 {status} ")), "{}: {out}", String::from_utf8_lossy(wire));
        }
        let big = format!(
            "GET / HTTP/1.1\r\nHost: a\r\nX: {}\r\n\r\n",
            "v".repeat(70_000)
        );
        assert!(exchange(addr, big.as_bytes()).starts_with("HTTP/1.1 431"));
        stop.shutdown();
        join.join().unwrap();
    }

    #[test]
    fn head_requests_and_http_10_behave() {
        let (addr, stop, join) = start(Config::default(), echo);
        let out = exchange(
            addr,
            b"HEAD /who HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n",
        );
        assert!(
            out.contains("Content-Length: ") && out.ends_with("\r\n\r\n"),
            "{out}"
        );
        let out = exchange(addr, b"GET /who HTTP/1.0\r\n\r\n");
        assert!(
            out.starts_with("HTTP/1.0 200 OK") && out.contains("Connection: close"),
            "{out}"
        );
        stop.shutdown();
        join.join().unwrap();
    }

    #[test]
    fn streams_are_chunked_and_end_the_connection() {
        let (addr, stop, join) = start(Config::default(), echo);
        let out = exchange(addr, b"GET /stream HTTP/1.1\r\nHost: a\r\n\r\n");
        assert!(out.contains("Transfer-Encoding: chunked"), "{out}");
        assert!(
            out.contains("data: 0\n\n")
                && out.contains("data: 2\n\n")
                && out.ends_with("0\r\n\r\n"),
            "{out}"
        );
        stop.shutdown();
        join.join().unwrap();
    }

    #[test]
    fn streams_refuse_bytes_after_synchronized_shutdown() {
        use std::sync::{Mutex, mpsc};

        for minor in [0, 1] {
            let (ready_tx, ready_rx) = mpsc::channel();
            let (resume_tx, resume_rx) = mpsc::channel();
            let resume_rx = Arc::new(Mutex::new(resume_rx));
            let (result_tx, result_rx) = mpsc::channel();
            let (addr, stop, join) = start(Config::default(), move |_: &Request| {
                let (ready_tx, resume_rx, result_tx) =
                    (ready_tx.clone(), resume_rx.clone(), result_tx.clone());
                Reply::Stream {
                    status: 200,
                    headers: Headers::new(),
                    produce: Box::new(move |stream| {
                        stream.send(b"before")?;
                        ready_tx.send(()).unwrap();
                        resume_rx
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                        let result = stream.send(b"after");
                        result_tx
                            .send(result.as_ref().map(|_| ()).map_err(io::Error::kind))
                            .unwrap();
                        result
                    }),
                }
            });
            let mut client = TcpStream::connect(addr).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            write!(client, "GET /stream HTTP/1.{minor}\r\nHost: a\r\n\r\n").unwrap();
            ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            // The producer cannot attempt its second send until shutdown is established.
            stop.shutdown();
            resume_tx.send(()).unwrap();
            let result = result_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let mut wire = String::new();
            client.read_to_string(&mut wire).unwrap();
            join.join().unwrap();
            assert_eq!(result, Err(io::ErrorKind::Interrupted));
            let (head, body) = wire.split_once("\r\n\r\n").unwrap();
            assert!(head.starts_with(&format!("HTTP/1.{minor} 200 OK")));
            assert_eq!(head.contains("Transfer-Encoding: chunked"), minor == 1);
            assert_eq!(
                body,
                if minor == 1 {
                    "6\r\nbefore\r\n"
                } else {
                    "before"
                }
            );
        }
    }

    #[test]
    fn stream_send_propagates_failed_writes_in_both_wire_formats() {
        for chunked in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (mut writer, _) = listener.accept().unwrap();
            writer.shutdown(Shutdown::Write).unwrap();
            let stop = AtomicBool::new(false);
            let mut stream = Stream {
                out: if chunked {
                    StreamOut::Chunked(ChunkedWriter::new(&mut writer))
                } else {
                    StreamOut::Raw(&mut writer)
                },
                stop: &stop,
            };
            let error = stream.send(b"must not succeed").unwrap_err();
            assert_ne!(error.kind(), io::ErrorKind::Interrupted);
            let mut wire = Vec::new();
            peer.read_to_end(&mut wire).unwrap();
            assert!(wire.is_empty());
        }
    }

    #[test]
    fn stream_backpressure_reaches_the_producer_as_a_write_error() {
        use std::sync::mpsc;

        let (result_tx, result_rx) = mpsc::channel();
        let cfg = Config {
            write_timeout: Duration::from_millis(100),
            ..Config::default()
        };
        let (addr, stop, join) = start(cfg, move |_: &Request| {
            let result_tx = result_tx.clone();
            Reply::Stream {
                status: 200,
                headers: Headers::new(),
                produce: Box::new(move |stream| {
                    let result = (|| {
                        let block = vec![b'x'; 1024 * 1024];
                        for _ in 0..64 {
                            stream.send(&block)?;
                        }
                        Ok(())
                    })();
                    result_tx
                        .send(result.as_ref().map(|_| ()).map_err(io::Error::kind))
                        .unwrap();
                    result
                }),
            }
        });
        let mut client = TcpStream::connect(addr).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client
            .write_all(b"GET /stream HTTP/1.1\r\nHost: a\r\n\r\n")
            .unwrap();
        // Consume only the head. The bounded producer must encounter socket backpressure.
        let mut client = BufReader::new(client);
        let mut line = String::new();
        loop {
            line.clear();
            client.read_line(&mut line).unwrap();
            assert!(!line.is_empty(), "response ended before its head");
            if line == "\r\n" {
                break;
            }
        }
        let result = result_rx.recv_timeout(Duration::from_secs(5));
        drop(client);
        stop.shutdown();
        join.join().unwrap();
        assert!(
            matches!(
                result,
                Ok(Err(io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock))
            ),
            "{result:?}"
        );
    }

    #[test]
    fn a_slow_client_is_cut_off_and_idle_connections_expire() {
        let cfg = Config {
            request_timeout: Duration::from_millis(300),
            idle_timeout: Duration::from_millis(300),
            ..Config::default()
        };
        let (addr, stop, join) = start(cfg, echo);
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(b"GET / HTTP/1.1\r\nHost: a\r\n").unwrap();
        let started = Instant::now();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        assert!(out.starts_with("HTTP/1.1 408"), "{out}");
        assert!(started.elapsed() < Duration::from_secs(3));
        let mut idle = TcpStream::connect(addr).unwrap();
        idle.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut b = [0u8; 1];
        assert_eq!(
            idle.read(&mut b).unwrap(),
            0,
            "the server closes an idle connection"
        );
        stop.shutdown();
        join.join().unwrap();
    }

    #[test]
    fn many_clients_are_served_at_once_and_excess_is_refused() {
        let cfg = Config {
            max_connections: 4,
            idle_timeout: Duration::from_millis(500),
            ..Config::default()
        };
        let (addr, stop, join) = start(cfg, echo);
        let held: Vec<TcpStream> = (0..4).map(|_| TcpStream::connect(addr).unwrap()).collect();
        thread::sleep(Duration::from_millis(100));
        let out = exchange(
            addr,
            b"GET /who HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n",
        );
        assert!(out.starts_with("HTTP/1.1 503"), "{out}");
        drop(held);
        thread::sleep(Duration::from_millis(100));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                thread::spawn(move || {
                    exchange(
                        addr,
                        b"GET /who HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n",
                    )
                })
            })
            .collect();
        // Sixteen at once against a limit of four: each is either served or told to retry, and
        // none is dropped.
        let answers: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(
            answers
                .iter()
                .all(|o| o.starts_with("HTTP/1.1 200") || o.starts_with("HTTP/1.1 503")),
            "{answers:?}"
        );
        assert!(answers.iter().any(|o| o.starts_with("HTTP/1.1 200")));
        stop.shutdown();
        join.join().unwrap();
    }

    #[test]
    fn garbage_never_crashes_the_server() {
        let cfg = Config {
            request_timeout: Duration::from_millis(200),
            idle_timeout: Duration::from_millis(200),
            ..Config::default()
        };
        let (addr, stop, join) = start(cfg, echo);
        let mut s = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..200 {
            let mut wire = b"POST /echo HTTP/1.1\r\nHost: a\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc".to_vec();
            for _ in 0..4 {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                let i = (s as usize) % wire.len();
                wire[i] = (s >> 32) as u8;
            }
            let _ = exchange(addr, &wire);
        }
        let out = exchange(
            addr,
            b"GET /who HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n",
        );
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        stop.shutdown();
        join.join().unwrap();
    }
}
