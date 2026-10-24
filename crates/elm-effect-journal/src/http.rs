//! A small HTTP adapter over one [`DurableLog`], with SSE and long-poll tails.
//!
//! The stream URL is `/stream`:
//! - `GET /stream?offset=<token>` is a bounded catch-up page (NDJSON body).
//!   `offset` is `-1`/absent for the start, `now` for the tail, or a token a
//!   previous response returned. Unknown offsets are `400`.
//! - `GET /stream?offset=<token>&live=sse` streams catch-up records, then
//!   tails. Data arrives as `event: data`, followed by a `control` event with
//!   `streamNextOffset`, `upToDate`, and `streamClosed` (EOF, then the
//!   connection closes).
//! - `GET /stream?offset=<token>&live=long-poll&timeout=<seconds>` holds the
//!   request at the tail until a record arrives, the stream closes, or the
//!   timeout (1 to 30 seconds, default 25) expires. It returns the same
//!   NDJSON page and headers as a catch-up read. A timed-out page is empty
//!   with `Stream-Up-To-Date: true`, so the client re-requests at once.
//! - `POST /stream` appends one envelope. `Stream-Closed: true` closes, with
//!   the body as the final record when present. A later append is `409` with
//!   `Stream-Closed: true`.
//! - `HEAD /stream` reports the catch-up cursor and closure.
//!
//! This is the read/write surface, not a deployment: there is no auth, TLS,
//! TTL, delete, fork, `Stream-Seq`, or chunked request body. It binds whatever
//! address the caller gives it, so bind loopback unless a proxy fronts it.
//!
//! ponytail: one thread per connection and a 25 ms tail poll; a connection
//! pool and a condvar wake if a real deployment needs them.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::durable_log::{AppendError, DurableLog, Offset, Page};

/// Records per page and per live read.
const PAGE_LIMIT: usize = 256;
/// Longest request/header line accepted.
const MAX_LINE: usize = 16 * 1024;
/// Largest append body accepted.
const MAX_BODY: usize = 8 * 1024 * 1024;
/// Header count ceiling, so a peer cannot grow one request without bound.
const MAX_HEADERS: usize = 64;
/// Tail polling interval.
const POLL: Duration = Duration::from_millis(25);
/// Default long-poll wait. Short enough for common proxy idle timeouts.
const LONG_POLL_DEFAULT: Duration = Duration::from_secs(25);
/// Longest wait a client may ask for, so one handler cannot be pinned.
const LONG_POLL_MAX_SECS: u64 = 30;
/// SSE comment interval, keeps idle proxies from dropping the connection.
const KEEPALIVE: Duration = Duration::from_secs(15);

/// A running adapter. Dropping it stops the accept loop and lets live SSE
/// and long-poll readers exit on their next poll.
pub struct HttpServer {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl HttpServer {
    /// Serve `log` on `addr`. The log is shared by every connection, so the
    /// adapter inherits the log's single-writer/multi-reader contract.
    pub fn start<L>(log: Arc<L>, addr: impl ToSocketAddrs) -> io::Result<Self>
    where
        L: DurableLog + 'static,
    {
        let listener = TcpListener::bind(addr)?;
        // Non-blocking accept is how shutdown wakes the loop without a
        // second socket or a dependency.
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&shutdown);
        let accept = thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let log = Arc::clone(&log);
                        let flag = Arc::clone(&flag);
                        thread::spawn(move || {
                            let _ = handle_connection(stream, log.as_ref(), &flag);
                        });
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            addr,
            shutdown,
            accept: Some(accept),
        })
    }

    /// The bound address, useful when the caller asked for port 0.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stop accepting and wait for the accept thread. Connection threads
    /// notice the flag on their next read or poll; an SSE reader exits within
    /// one poll interval.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.accept.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A parsed request. Header names are lowercased.
struct Request {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

/// Why a request could not be parsed.
enum RequestError {
    Io(io::Error),
    Bad(&'static str),
    TooLarge,
    Unsupported,
}

impl From<io::Error> for RequestError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn handle_connection<L: DurableLog>(
    mut stream: TcpStream,
    log: &L,
    shutdown: &AtomicBool,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;

    let request = match read_request(&mut stream) {
        Ok(Some(request)) => request,
        Ok(None) => return Ok(()),
        Err(RequestError::Io(error)) => return Err(error),
        Err(RequestError::Bad(reason)) => return respond_text(&mut stream, 400, reason),
        Err(RequestError::TooLarge) => {
            return respond_text(&mut stream, 413, "request body too large");
        }
        Err(RequestError::Unsupported) => {
            return respond_text(
                &mut stream,
                501,
                "chunked transfer encoding is not supported",
            );
        }
    };

    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((&request.target, ""));
    if path != "/stream" {
        return respond_text(&mut stream, 404, "not found");
    }
    let query = parse_query(query);

    match request.method.as_str() {
        "GET" => match query.get("live").map(String::as_str) {
            Some("sse") => serve_sse(stream, log, &query, shutdown),
            Some("long-poll") => serve_long_poll(stream, log, &query, shutdown),
            Some(other) => {
                respond_text(&mut stream, 400, &format!("unsupported live mode: {other}"))
            }
            None => serve_catch_up(&mut stream, log, &query),
        },
        "HEAD" => serve_head(&mut stream, log),
        "POST" => serve_append(&mut stream, log, &request),
        _ => respond(
            &mut stream,
            405,
            &[("Allow", "GET, HEAD, POST".to_string())],
            b"",
        ),
    }
}

fn read_request(stream: &mut TcpStream) -> Result<Option<Request>, RequestError> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let Some(line) = read_line(&mut reader)? else {
        return Ok(None);
    };
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(RequestError::Bad("malformed request line"));
    };
    if !version.starts_with("HTTP/") {
        return Err(RequestError::Bad("unsupported HTTP version"));
    }

    let mut headers = HashMap::new();
    loop {
        let Some(line) = read_line(&mut reader)? else {
            return Err(RequestError::Bad("request ended inside the headers"));
        };
        if line.is_empty() {
            break;
        }
        if headers.len() >= MAX_HEADERS {
            return Err(RequestError::Bad("too many headers"));
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(RequestError::Bad("malformed header"));
        };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }

    if headers.contains_key("transfer-encoding") {
        return Err(RequestError::Unsupported);
    }
    let length = match headers.get("content-length") {
        Some(raw) => raw
            .parse::<usize>()
            .map_err(|_| RequestError::Bad("invalid content-length"))?,
        None => 0,
    };
    if length > MAX_BODY {
        return Err(RequestError::TooLarge);
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;

    Ok(Some(Request {
        method: method.to_string(),
        target: target.to_string(),
        headers,
        body,
    }))
}

fn read_line(reader: &mut BufReader<TcpStream>) -> Result<Option<String>, RequestError> {
    let mut line = String::new();
    // Bound the read itself, not just the result: an endless line must not
    // grow the String before the length check.
    let read = reader.take((MAX_LINE + 1) as u64).read_line(&mut line)?;
    if read == 0 {
        return Ok(None);
    }
    if line.len() > MAX_LINE || !line.ends_with('\n') {
        return Err(RequestError::Bad("header line too long"));
    }
    Ok(Some(line.trim_end_matches(['\r', '\n']).to_string()))
}

fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            (key.to_string(), value.to_string())
        })
        .collect()
}

/// Resolve an HTTP offset token to a log cursor.
fn requested_offset(query: &HashMap<String, String>) -> Result<Offset, &'static str> {
    match query.get("offset").map(String::as_str) {
        None | Some("") | Some("-1") => Ok(Offset::Start),
        Some("now") => Ok(Offset::Now),
        Some(raw) if raw.parse::<u64>().is_ok() => Ok(Offset::At(raw.to_string())),
        Some(_) => Err("offset is not a token this log minted"),
    }
}

/// The long-poll wait. A missing value uses the default; an explicit value
/// must be a whole number of seconds inside the allowed range.
fn requested_timeout(query: &HashMap<String, String>) -> Result<Duration, String> {
    let Some(raw) = query.get("timeout") else {
        return Ok(LONG_POLL_DEFAULT);
    };
    let seconds: u64 = raw
        .parse()
        .map_err(|_| "timeout is not a whole number of seconds".to_string())?;
    if !(1..=LONG_POLL_MAX_SECS).contains(&seconds) {
        return Err(format!(
            "timeout must be between 1 and {LONG_POLL_MAX_SECS} seconds"
        ));
    }
    Ok(Duration::from_secs(seconds))
}

fn offset_token(offset: &Offset) -> String {
    match offset {
        Offset::Start => "-1".to_string(),
        Offset::Now => "now".to_string(),
        Offset::At(raw) => raw.clone(),
    }
}

/// The cursor a client can pass back to read what comes next. An empty log
/// resolves to `-1`, which is the start of the log.
fn tail_cursor<L: DurableLog>(log: &L) -> String {
    log.read_page(&Offset::Now, 1)
        .map(|page| offset_token(&page.next))
        .unwrap_or_else(|_| "-1".to_string())
}

fn serve_catch_up<L: DurableLog>(
    stream: &mut TcpStream,
    log: &L,
    query: &HashMap<String, String>,
) -> io::Result<()> {
    let from = match requested_offset(query) {
        Ok(offset) => offset,
        Err(reason) => return respond_text(stream, 400, reason),
    };
    let page = match log.read_page(&from, PAGE_LIMIT) {
        Ok(page) => page,
        Err(error) => return respond_read_error(stream, error),
    };
    respond_page(stream, &page)
}

/// Hold the request at the tail, then answer with the same page shape as a
/// catch-up read. `offset=now` resolves once, before the wait, so the next
/// append is visible instead of skipped on the following poll.
fn serve_long_poll<L: DurableLog>(
    mut stream: TcpStream,
    log: &L,
    query: &HashMap<String, String>,
    shutdown: &AtomicBool,
) -> io::Result<()> {
    let from = match requested_offset(query) {
        Ok(offset) => offset,
        Err(reason) => return respond_text(&mut stream, 400, reason),
    };
    let wait = match requested_timeout(query) {
        Ok(wait) => wait,
        Err(reason) => return respond_text(&mut stream, 400, &reason),
    };

    // Resolve `Now` and validate an explicit cursor before waiting. A
    // sentinel left in the cursor would re-resolve to the new tail on every
    // read and skip the append this request is waiting for.
    let mut page = match log.read_page(&from, PAGE_LIMIT) {
        Ok(page) => page,
        Err(error) => return respond_read_error(&mut stream, error),
    };
    let deadline = Instant::now() + wait;

    loop {
        if !page.records.is_empty() || page.closed || !page.up_to_date {
            return respond_page(&mut stream, &page);
        }
        if shutdown.load(Ordering::SeqCst) || Instant::now() >= deadline {
            return respond_page(&mut stream, &page);
        }
        let cursor = page.next.clone();
        thread::sleep(POLL);
        page = match log.read_page(&cursor, PAGE_LIMIT) {
            Ok(page) => page,
            Err(error) => return respond_read_error(&mut stream, error),
        };
    }
}

fn respond_page(stream: &mut TcpStream, page: &Page) -> io::Result<()> {
    let body = page_body(page);
    let mut headers = vec![
        ("Content-Type", "application/x-ndjson".to_string()),
        ("Stream-Next-Offset", offset_token(&page.next)),
        ("Cache-Control", "no-store".to_string()),
    ];
    if page.up_to_date {
        headers.push(("Stream-Up-To-Date", "true".to_string()));
    }
    if page.closed {
        headers.push(("Stream-Closed", "true".to_string()));
    }
    respond(stream, 200, &headers, body.as_bytes())
}

fn respond_read_error(stream: &mut TcpStream, error: AppendError) -> io::Result<()> {
    match error {
        AppendError::UnknownOffset => {
            respond_text(stream, 400, "offset is not a record in this log")
        }
        other => respond_text(stream, 500, &format!("read failed: {other}")),
    }
}

fn page_body(page: &Page) -> String {
    let mut body = String::new();
    for (_, payload) in &page.records {
        body.push_str(payload);
        body.push('\n');
    }
    body
}

fn serve_head<L: DurableLog>(stream: &mut TcpStream, log: &L) -> io::Result<()> {
    let page = match log.read_page(&Offset::Now, 1) {
        Ok(page) => page,
        Err(error) => return respond_read_error(stream, error),
    };
    let mut headers = vec![
        ("Content-Type", "application/x-ndjson".to_string()),
        ("Stream-Next-Offset", offset_token(&page.next)),
        ("Cache-Control", "no-store".to_string()),
    ];
    if log.is_closed() {
        headers.push(("Stream-Closed", "true".to_string()));
    }
    // HEAD carries no body and no Content-Length: this adapter does not
    // compute the length a GET would return, and a wrong length is worse than
    // none.
    respond_with(stream, 200, &headers, b"", None)
}

fn serve_append<L: DurableLog>(
    stream: &mut TcpStream,
    log: &L,
    request: &Request,
) -> io::Result<()> {
    let close = request
        .headers
        .get("stream-closed")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));

    if request.body.is_empty() {
        if !close {
            return respond_text(stream, 400, "empty body; send Stream-Closed: true to close");
        }
        return match log.close() {
            Ok(()) => respond_closed(stream, log),
            Err(error) => respond_append_error(stream, log, error),
        };
    }

    let envelope = match std::str::from_utf8(&request.body) {
        Ok(envelope) => envelope,
        Err(_) => return respond_text(stream, 400, "body is not UTF-8 JSON"),
    };
    let result = if close {
        log.append_json_and_close(envelope)
    } else {
        log.append_json(envelope)
    };
    match result {
        Ok(offset) => {
            let mut headers = vec![("Stream-Next-Offset", offset_token(&offset))];
            if log.is_closed() {
                headers.push(("Stream-Closed", "true".to_string()));
            }
            respond(stream, 204, &headers, b"")
        }
        Err(error) => respond_append_error(stream, log, error),
    }
}

fn respond_closed<L: DurableLog>(stream: &mut TcpStream, log: &L) -> io::Result<()> {
    respond(
        stream,
        204,
        &[
            ("Stream-Next-Offset", tail_cursor(log)),
            ("Stream-Closed", "true".to_string()),
        ],
        b"",
    )
}

fn respond_append_error<L: DurableLog>(
    stream: &mut TcpStream,
    log: &L,
    error: AppendError,
) -> io::Result<()> {
    match error {
        // Closed is checked before every other conflict, so a client always
        // gets the header that says EOF rather than a generic 409.
        AppendError::Closed => respond(
            stream,
            409,
            &[
                ("Stream-Next-Offset", tail_cursor(log)),
                ("Stream-Closed", "true".to_string()),
            ],
            b"",
        ),
        AppendError::StaleEpoch { current, .. } => {
            respond(stream, 403, &[("Producer-Epoch", current.to_string())], b"")
        }
        AppendError::SeqGap { got, last } => {
            let expected = last.map_or(0, |last| last + 1);
            respond(
                stream,
                409,
                &[
                    ("Producer-Expected-Seq", expected.to_string()),
                    ("Producer-Received-Seq", got.to_string()),
                ],
                b"",
            )
        }
        AppendError::PayloadMismatch { .. } => respond(stream, 409, &[], b""),
        AppendError::NotAnEnvelope(reason) => respond_text(stream, 400, &reason),
        AppendError::PartialIdentity => respond_text(stream, 400, "partial producer identity"),
        AppendError::UnknownVersion(version) => {
            respond_text(stream, 400, &format!("unknown schema version: {version}"))
        }
        AppendError::UnknownOffset => respond_text(stream, 400, "unknown offset"),
        AppendError::Storage(reason) => {
            respond_text(stream, 500, &format!("storage failed: {reason}"))
        }
        AppendError::Unavailable(reason) => respond_text(
            stream,
            503,
            &format!("log unavailable (outcome unknown, reconcile): {reason}"),
        ),
    }
}

fn serve_sse<L: DurableLog>(
    mut stream: TcpStream,
    log: &L,
    query: &HashMap<String, String>,
    shutdown: &AtomicBool,
) -> io::Result<()> {
    let from = match requested_offset(query) {
        Ok(offset) => offset,
        Err(reason) => return respond_text(&mut stream, 400, reason),
    };
    // Validate before the 200 goes out: an SSE response has already committed
    // its status once the headers are written.
    if let Err(error) = log.read_page(&from, 1) {
        return respond_read_error(&mut stream, error);
    }

    stream.write_all(
        b"HTTP/1.1 200 OK\r\n\
          Content-Type: text/event-stream\r\n\
          Cache-Control: no-store\r\n\
          Connection: close\r\n\r\n",
    )?;
    stream.flush()?;

    let mut cursor = from;
    let mut announced = false;
    let mut keepalive = Instant::now();
    loop {
        if shutdown.load(Ordering::SeqCst) {
            return Ok(());
        }
        match log.read_page(&cursor, PAGE_LIMIT) {
            Ok(page) if !page.records.is_empty() => {
                for (_, payload) in &page.records {
                    write!(stream, "event: data\ndata: {payload}\n\n")?;
                }
                let mut control = serde_json::json!({
                    "streamNextOffset": offset_token(&page.next),
                });
                if page.up_to_date {
                    control["upToDate"] = true.into();
                }
                if page.closed {
                    control["streamClosed"] = true.into();
                }
                write_control(&mut stream, &control)?;
                cursor = page.next;
                announced = page.up_to_date;
                if page.closed {
                    return Ok(());
                }
                if !page.up_to_date {
                    continue;
                }
            }
            Ok(page) if page.closed => {
                let control = serde_json::json!({
                    "streamNextOffset": offset_token(&page.next),
                    "streamClosed": true,
                    "upToDate": true,
                });
                write_control(&mut stream, &control)?;
                return Ok(());
            }
            Ok(page) => {
                if page.up_to_date && !announced {
                    let control = serde_json::json!({
                        "streamNextOffset": offset_token(&page.next),
                        "upToDate": true,
                    });
                    write_control(&mut stream, &control)?;
                    announced = true;
                    cursor = page.next;
                }
            }
            Err(error) => {
                let failure = serde_json::json!({ "error": error.to_string() });
                write!(stream, "event: error\ndata: {failure}\n\n")?;
                stream.flush()?;
                return Ok(());
            }
        }

        if keepalive.elapsed() >= KEEPALIVE {
            stream.write_all(b": keep-alive\n\n")?;
            stream.flush()?;
            keepalive = Instant::now();
        }
        thread::sleep(POLL);
    }
}

fn write_control(stream: &mut TcpStream, control: &serde_json::Value) -> io::Result<()> {
    write!(stream, "event: control\ndata: {control}\n\n")?;
    stream.flush()
}

fn respond_text(stream: &mut TcpStream, status: u16, message: &str) -> io::Result<()> {
    respond(
        stream,
        status,
        &[("Content-Type", "text/plain; charset=utf-8".to_string())],
        format!("{message}\n").as_bytes(),
    )
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    headers: &[(&str, String)],
    body: &[u8],
) -> io::Result<()> {
    // 204 carries no body and must not carry a Content-Length.
    let length = (status != 204).then_some(body.len());
    respond_with(stream, status, headers, body, length)
}

fn respond_with(
    stream: &mut TcpStream,
    status: u16,
    headers: &[(&str, String)],
    body: &[u8],
    length: Option<usize>,
) -> io::Result<()> {
    let mut head = format!("HTTP/1.1 {status} {}\r\n", reason(status));
    head.push_str("Connection: close\r\n");
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    if let Some(length) = length {
        head.push_str(&format!("Content-Length: {length}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if !body.is_empty() {
        stream.write_all(body)?;
    }
    stream.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}
