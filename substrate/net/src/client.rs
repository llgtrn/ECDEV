//! A blocking HTTP/1.1 client: one connection per request, optional HTTP proxy (absolute-form for
//! `http` targets, `CONNECT` tunnels for `https`), pinned addresses, a deadline over the whole
//! exchange, and a pluggable TLS connector. Redirects are the caller's business, and so is
//! decompression.

use std::{
    fmt,
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::http::{self, Headers, Limits};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}

impl Scheme {
    pub fn default_port(self) -> u16 {
        match self {
            Scheme::Http => 80,
            Scheme::Https => 443,
        }
    }
}

/// Where a request goes, already parsed by the caller's URL parser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub scheme: Scheme,
    /// A registered name or an IP literal without brackets.
    pub host: String,
    pub port: u16,
    /// The path and query, starting with `/`.
    pub path_and_query: String,
}

impl Target {
    fn host_header(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.port == self.scheme.default_port() {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }

    fn authority(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("{host}:{}", self.port)
    }
}

/// A stream the client can read, write and bound in time.
pub trait Connection: Read + Write + Send {
    fn set_read_timeout(&mut self, d: Option<Duration>) -> io::Result<()>;
    fn set_write_timeout(&mut self, d: Option<Duration>) -> io::Result<()>;
}

impl Connection for TcpStream {
    fn set_read_timeout(&mut self, d: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, d)
    }
    fn set_write_timeout(&mut self, d: Option<Duration>) -> io::Result<()> {
        TcpStream::set_write_timeout(self, d)
    }
}

/// Wraps a connected socket in a secure channel to `server_name`.
pub trait Tls: Send + Sync {
    fn connect(&self, server_name: &str, stream: TcpStream) -> io::Result<Box<dyn Connection>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proxy {
    pub host: String,
    pub port: u16,
    /// Credentials for `Proxy-Authorization: Basic`.
    pub auth: Option<(String, String)>,
}

#[derive(Debug)]
pub enum ErrorKind {
    Dns,
    Connect,
    Timeout,
    Tls,
    Proxy,
    Protocol,
    TooLarge,
    Io,
}

#[derive(Debug)]
pub struct ClientError {
    pub kind: ErrorKind,
    pub message: String,
}

impl ClientError {
    fn new(kind: ErrorKind, message: impl Into<String>) -> ClientError {
        ClientError {
            kind,
            message: message.into(),
        }
    }

    pub fn is_timeout(&self) -> bool {
        matches!(self.kind, ErrorKind::Timeout)
    }

    /// The connection could not be established (name lookup, TCP connect or the proxy tunnel).
    pub fn is_connect(&self) -> bool {
        matches!(
            self.kind,
            ErrorKind::Dns | ErrorKind::Connect | ErrorKind::Proxy
        )
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ClientError {}

fn from_io(e: io::Error) -> ClientError {
    match e.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
            ClientError::new(ErrorKind::Timeout, e.to_string())
        }
        _ => ClientError::new(ErrorKind::Io, e.to_string()),
    }
}

fn from_http(e: http::Error) -> ClientError {
    match e {
        http::Error::Io(e) => from_io(e),
        http::Error::TooLarge(m) => ClientError::new(ErrorKind::TooLarge, m),
        http::Error::Closed => {
            ClientError::new(ErrorKind::Protocol, "connection closed before a response")
        }
        other => ClientError::new(ErrorKind::Protocol, other.to_string()),
    }
}

#[derive(Debug)]
pub struct ClientRequest {
    pub method: String,
    pub target: Target,
    pub headers: Headers,
    pub body: Vec<u8>,
    /// When set, connect to these addresses instead of resolving `target.host`.
    pub addrs: Option<Vec<SocketAddr>>,
}

impl ClientRequest {
    pub fn new(method: &str, target: Target) -> ClientRequest {
        ClientRequest {
            method: method.to_string(),
            target,
            headers: Headers::new(),
            body: Vec::new(),
            addrs: None,
        }
    }
}

#[derive(Debug)]
pub struct ClientResponse {
    pub status: u16,
    pub reason: String,
    pub minor: u8,
    pub headers: Headers,
    pub body: Vec<u8>,
}

/// A received response head and its owned connection, before the body is read.
///
/// Dropping this value closes the connection without reading the body. Calling
/// `read_body` keeps the original exchange deadline, including time spent holding this value.
pub struct IncomingResponse {
    pub status: u16,
    pub reason: String,
    pub minor: u8,
    pub headers: Headers,
    framing: http::Framing,
    max_body: usize,
    reader: ResponseReader,
}

impl fmt::Debug for IncomingResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IncomingResponse")
            .field("status", &self.status)
            .field("reason", &self.reason)
            .field("minor", &self.minor)
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

impl IncomingResponse {
    /// Reads the bounded body under the original exchange deadline.
    pub fn read_body(mut self) -> Result<ClientResponse, ClientError> {
        self.reader.check_deadline().map_err(from_io)?;
        let body =
            http::read_body(&mut self.reader, self.framing, self.max_body).map_err(from_http)?;
        self.reader.check_deadline().map_err(from_io)?;
        Ok(ClientResponse {
            status: self.status,
            reason: self.reason,
            minor: self.minor,
            headers: self.headers,
            body,
        })
    }
}

#[derive(Clone)]
pub struct ClientBuilder {
    user_agent: Option<String>,
    timeout: Option<Duration>,
    connect_timeout: Duration,
    proxy: Option<Proxy>,
    tls: Option<Arc<dyn Tls>>,
    limits: Limits,
}

#[derive(Clone)]
pub struct Client {
    config: ClientBuilder,
}

impl Client {
    pub fn builder() -> ClientBuilder {
        ClientBuilder {
            user_agent: None,
            timeout: None,
            connect_timeout: Duration::from_secs(10),
            proxy: None,
            tls: None,
            limits: Limits::default(),
        }
    }
}

impl ClientBuilder {
    pub fn user_agent(mut self, ua: &str) -> Self {
        self.user_agent = Some(ua.to_string());
        self
    }

    /// A deadline for the whole exchange: connect, send, and read the full response.
    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = Some(d);
        self
    }

    pub fn connect_timeout(mut self, d: Duration) -> Self {
        self.connect_timeout = d;
        self
    }

    pub fn proxy(mut self, p: Proxy) -> Self {
        self.proxy = Some(p);
        self
    }

    pub fn tls(mut self, t: Arc<dyn Tls>) -> Self {
        self.tls = Some(t);
        self
    }

    pub fn max_response_bytes(mut self, n: usize) -> Self {
        self.limits.max_body = n;
        self
    }

    /// The maximum number of fields in each interim or final response head.
    pub fn max_response_fields(mut self, n: usize) -> Self {
        self.limits.max_fields = n;
        self
    }

    pub fn build(self) -> Client {
        Client { config: self }
    }
}

struct Deadline(Option<Instant>);

impl Deadline {
    fn left(&self) -> Result<Option<Duration>, ClientError> {
        match self.0 {
            None => Ok(None),
            Some(d) => {
                let left = d.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    Err(ClientError::new(ErrorKind::Timeout, "deadline passed"))
                } else {
                    Ok(Some(left))
                }
            }
        }
    }

    fn io_left(&self) -> io::Result<Option<Duration>> {
        self.left()
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "deadline passed"))
    }
}

/// Applies the remaining time to the socket before every read and write.
struct Bounded<'a> {
    inner: &'a mut dyn Connection,
    deadline: &'a Deadline,
}

impl Read for Bounded<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.deadline.io_left()?;
        self.inner.set_read_timeout(left)?;
        self.inner.read(buf)
    }
}

impl Write for Bounded<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let left = self.deadline.io_left()?;
        self.inner.set_write_timeout(left)?;
        self.inner.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct OwnedBounded {
    inner: Box<dyn Connection>,
    deadline: Deadline,
}

impl Read for OwnedBounded {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Bounded {
            inner: self.inner.as_mut(),
            deadline: &self.deadline,
        }
        .read(buf)
    }
}

/// Checks the deadline before buffered reads as well as reads from the socket.
struct ResponseReader {
    inner: BufReader<OwnedBounded>,
}

impl ResponseReader {
    fn check_deadline(&self) -> io::Result<()> {
        self.inner.get_ref().deadline.io_left().map(|_| ())
    }
}

impl Read for ResponseReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.check_deadline()?;
        self.inner.read(buf)
    }
}

impl BufRead for ResponseReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.check_deadline()?;
        self.inner.fill_buf()
    }

    fn consume(&mut self, n: usize) {
        self.inner.consume(n);
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

impl Client {
    fn tcp(
        &self,
        host: &str,
        port: u16,
        addrs: Option<&[SocketAddr]>,
        deadline: &Deadline,
    ) -> Result<TcpStream, ClientError> {
        let resolved: Vec<SocketAddr> = match addrs {
            Some(a) => a.to_vec(),
            None => (host, port)
                .to_socket_addrs()
                .map_err(|e| ClientError::new(ErrorKind::Dns, format!("{host}: {e}")))?
                .collect(),
        };
        if resolved.is_empty() {
            return Err(ClientError::new(
                ErrorKind::Dns,
                format!("{host}: no addresses"),
            ));
        }
        let mut last = None;
        for addr in resolved {
            let budget = match deadline.left()? {
                Some(left) => left.min(self.config.connect_timeout),
                None => self.config.connect_timeout,
            };
            match TcpStream::connect_timeout(&addr, budget) {
                Ok(s) => {
                    s.set_nodelay(true).ok();
                    return Ok(s);
                }
                Err(e) => last = Some(e),
            }
        }
        let e = last.unwrap();
        Err(
            if matches!(
                e.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) {
                ClientError::new(ErrorKind::Timeout, format!("connecting to {host}: {e}"))
            } else {
                ClientError::new(ErrorKind::Connect, format!("connecting to {host}: {e}"))
            },
        )
    }

    /// Opens the byte stream to the origin: direct, through a proxy, or through a tunnel.
    fn open(
        &self,
        req: &ClientRequest,
        deadline: &Deadline,
    ) -> Result<Box<dyn Connection>, ClientError> {
        let t = &req.target;
        let (mut tcp, via_proxy) = match &self.config.proxy {
            Some(p) => (self.tcp(&p.host, p.port, None, deadline)?, true),
            None => (
                self.tcp(&t.host, t.port, req.addrs.as_deref(), deadline)?,
                false,
            ),
        };
        if via_proxy && t.scheme == Scheme::Https {
            self.tunnel(&mut tcp, t, deadline)?;
        }
        if t.scheme == Scheme::Http {
            return Ok(Box::new(tcp));
        }
        let tls =
            self.config.tls.as_ref().ok_or_else(|| {
                ClientError::new(ErrorKind::Tls, "no TLS connector is configured")
            })?;
        let left = deadline.left()?;
        tcp.set_read_timeout(left).map_err(from_io)?;
        tcp.set_write_timeout(left).map_err(from_io)?;
        tls.connect(&t.host, tcp).map_err(|e| match e.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
                ClientError::new(ErrorKind::Timeout, e.to_string())
            }
            _ => ClientError::new(ErrorKind::Tls, e.to_string()),
        })
    }

    /// `CONNECT host:port`, reading the answer byte by byte so nothing past it is consumed.
    fn tunnel(
        &self,
        tcp: &mut TcpStream,
        t: &Target,
        deadline: &Deadline,
    ) -> Result<(), ClientError> {
        let p = self.config.proxy.as_ref().expect("a tunnel needs a proxy");
        let mut head = Headers::new();
        head.append("Host", t.authority()).map_err(from_http)?;
        if let Some((u, pw)) = &p.auth {
            head.append(
                "Proxy-Authorization",
                format!("Basic {}", base64(format!("{u}:{pw}").as_bytes())),
            )
            .map_err(from_http)?;
        }
        let mut out = Vec::new();
        http::write_request_head(&mut out, "CONNECT", &t.authority(), &head).map_err(from_io)?;
        let mut b = Bounded {
            inner: tcp,
            deadline,
        };
        b.write_all(&out).map_err(from_io)?;
        let mut answer = Vec::new();
        let mut byte = [0u8; 1];
        while !answer.ends_with(b"\r\n\r\n") {
            if answer.len() > 16 * 1024 {
                return Err(ClientError::new(ErrorKind::Proxy, "proxy answer too long"));
            }
            if b.read(&mut byte).map_err(from_io)? == 0 {
                return Err(ClientError::new(
                    ErrorKind::Proxy,
                    "proxy closed the connection",
                ));
            }
            answer.push(byte[0]);
        }
        let status = http::read_response_head(&mut io::Cursor::new(answer), &Limits::default())
            .map_err(|e| ClientError::new(ErrorKind::Proxy, e.to_string()))?;
        if !(200..300).contains(&status.status) {
            return Err(ClientError::new(
                ErrorKind::Proxy,
                format!("proxy refused the tunnel with {}", status.status),
            ));
        }
        Ok(())
    }

    /// Sends a request and reads its final response head, retaining the bounded connection.
    ///
    /// Redirects are not followed. The head, including its framing fields, is validated before
    /// returning. A caller that needs only status and headers can drop the response without
    /// consuming its body; body bytes and size are checked by `read_body`.
    pub fn send_head(&self, req: &ClientRequest) -> Result<IncomingResponse, ClientError> {
        let deadline = Deadline(self.config.timeout.map(|d| Instant::now() + d));
        let mut conn = self.open(req, &deadline)?;
        let t = &req.target;
        let via_proxy_plain = self.config.proxy.is_some() && t.scheme == Scheme::Http;
        let mut headers = Headers::new();
        headers.append("Host", t.host_header()).map_err(from_http)?;
        for (n, v) in req.headers.iter() {
            if n.eq_ignore_ascii_case("host")
                || n.eq_ignore_ascii_case("connection")
                || n.eq_ignore_ascii_case("content-length")
                || n.eq_ignore_ascii_case("transfer-encoding")
            {
                continue;
            }
            headers.append(n, v).map_err(from_http)?;
        }
        if !headers.contains("user-agent")
            && let Some(ua) = &self.config.user_agent
        {
            headers.append("User-Agent", ua).map_err(from_http)?;
        }
        if !headers.contains("accept") {
            headers.append("Accept", "*/*").map_err(from_http)?;
        }
        if via_proxy_plain
            && let Some((u, p)) = self.config.proxy.as_ref().and_then(|p| p.auth.as_ref())
        {
            headers
                .append(
                    "Proxy-Authorization",
                    format!("Basic {}", base64(format!("{u}:{p}").as_bytes())),
                )
                .map_err(from_http)?;
        }
        if !req.body.is_empty() || matches!(req.method.as_str(), "POST" | "PUT" | "PATCH") {
            headers
                .append("Content-Length", req.body.len().to_string())
                .map_err(from_http)?;
        }
        headers.append("Connection", "close").map_err(from_http)?;
        let request_target = if via_proxy_plain {
            format!("http://{}{}", t.host_header(), t.path_and_query)
        } else {
            t.path_and_query.clone()
        };
        let mut wire = Vec::with_capacity(req.body.len() + 512);
        http::write_request_head(&mut wire, &req.method, &request_target, &headers)
            .map_err(from_io)?;
        wire.extend_from_slice(&req.body);
        {
            let mut b = Bounded {
                inner: conn.as_mut(),
                deadline: &deadline,
            };
            b.write_all(&wire).map_err(from_io)?;
            b.flush().map_err(from_io)?;
        }
        let mut reader = ResponseReader {
            inner: BufReader::with_capacity(
                32 * 1024,
                OwnedBounded {
                    inner: conn,
                    deadline,
                },
            ),
        };
        let head = loop {
            let head =
                http::read_response_head(&mut reader, &self.config.limits).map_err(from_http)?;
            match head.status {
                101 => {
                    return Err(ClientError::new(
                        ErrorKind::Protocol,
                        "protocol switch is not supported",
                    ));
                }
                100..=199 => continue,
                _ => break head,
            }
        };
        let framing = http::response_framing(req.method == "HEAD", head.status, &head.headers)
            .map_err(from_http)?;
        Ok(IncomingResponse {
            status: head.status,
            reason: head.reason,
            minor: head.minor,
            headers: head.headers,
            framing,
            max_body: self.config.limits.max_body,
            reader,
        })
    }

    /// Sends a request and reads the full bounded response.
    pub fn send(&self, req: &ClientRequest) -> Result<ClientResponse, ClientError> {
        self.send_head(req)?.read_body()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MemoryConnection(io::Cursor<Vec<u8>>);

    impl Read for MemoryConnection {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for MemoryConnection {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Connection for MemoryConnection {
        fn set_read_timeout(&mut self, _: Option<Duration>) -> io::Result<()> {
            Ok(())
        }

        fn set_write_timeout(&mut self, _: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn buffered_body_bytes_cannot_be_read_after_the_original_deadline() {
        let mut reader = ResponseReader {
            inner: BufReader::new(OwnedBounded {
                inner: Box::new(MemoryConnection(io::Cursor::new(b"hello".to_vec()))),
                deadline: Deadline(Some(Instant::now() + Duration::from_secs(5))),
            }),
        };
        assert_eq!(reader.fill_buf().unwrap(), b"hello");
        reader.inner.get_mut().deadline = Deadline(Some(Instant::now()));
        assert_eq!(reader.inner.buffer(), b"hello");
        assert_eq!(
            reader.fill_buf().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        let mut headers = Headers::new();
        headers.append("Content-Length", "5").unwrap();
        let response = IncomingResponse {
            status: 200,
            reason: "OK".into(),
            minor: 1,
            headers,
            framing: http::Framing::Length(5),
            max_body: 5,
            reader,
        };
        let error = response.read_body().unwrap_err();
        assert!(error.is_timeout(), "{error}");
    }
}
