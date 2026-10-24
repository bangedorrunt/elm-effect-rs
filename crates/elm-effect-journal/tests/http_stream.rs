//! End-to-end HTTP/SSE adapter tests: real sockets, real log, real EOF.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use elm_effect_journal::durable_log::{DurableLog, FileLog, MemLog, Offset, SCHEMA_VERSION};
use elm_effect_journal::http::HttpServer;

struct Response {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn envelope(seq: u64) -> String {
    format!(
        "{{\"v\":{SCHEMA_VERSION},\"producer\":{{\"producer_id\":\"http\",\"epoch\":0,\"seq\":{seq}}},\"payload\":{{\"n\":{seq}}}}}"
    )
}

fn at(n: u64) -> String {
    format!("{n:020}")
}

/// A unique log file per test, removed at the end of the test.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "elm-http-{name}-{}-{nanos}.jsonl",
            std::process::id()
        ));
        Scratch(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        let _ = fs::remove_file(self.0.with_extension("compacting"));
    }
}

fn send(
    addr: SocketAddr,
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Response {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let mut request = format!(
        "{method} {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).expect("write head");
    stream.write_all(body).expect("write body");

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("read response");
    parse_response(&raw)
}

fn parse_response(raw: &[u8]) -> Response {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("response head");
    let head = std::str::from_utf8(&raw[..split]).expect("utf-8 head");
    let body = raw[split + 4..].to_vec();
    let mut lines = head.lines();
    let status = lines
        .next()
        .expect("status line")
        .split_whitespace()
        .nth(1)
        .expect("status code")
        .parse()
        .expect("numeric status");
    let headers = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_string()))
        .collect();
    Response {
        status,
        headers,
        body,
    }
}

fn body_lines(response: &Response) -> Vec<String> {
    String::from_utf8(response.body.clone())
        .expect("utf-8 body")
        .lines()
        .map(str::to_string)
        .collect()
}

struct Sse {
    reader: BufReader<TcpStream>,
}

fn connect_sse(addr: SocketAddr, target: &str) -> Sse {
    let mut stream = TcpStream::connect(addr).expect("connect sse");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("sse timeout");
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n"
    )
    .expect("write sse request");
    let mut reader = BufReader::new(stream);
    let mut head = String::new();
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).expect("sse headers");
        assert!(read > 0, "connection closed before SSE headers");
        if line == "\r\n" {
            break;
        }
        head.push_str(&line);
    }
    assert!(
        head.starts_with("HTTP/1.1 200"),
        "SSE must start with 200, got {head}"
    );
    assert!(
        head.to_ascii_lowercase()
            .contains("content-type: text/event-stream"),
        "SSE content type missing: {head}"
    );
    Sse { reader }
}

impl Sse {
    fn next(&mut self) -> (String, String) {
        let mut event = String::new();
        let mut data = String::new();
        loop {
            let mut line = String::new();
            let read = self.reader.read_line(&mut line).expect("sse event");
            assert!(read > 0, "SSE closed before the next event");
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if event.is_empty() && data.is_empty() {
                    continue;
                }
                return (event, data);
            }
            if let Some(name) = line.strip_prefix("event: ") {
                event = name.to_string();
            } else if let Some(value) = line.strip_prefix("data: ") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value);
            }
        }
    }

    fn at_eof(&mut self) -> bool {
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("sse eof") == 0
    }
}

#[test]
fn http_catch_up_append_and_eof() {
    let log = Arc::new(MemLog::new());
    let mut server = HttpServer::start(Arc::clone(&log), "127.0.0.1:0").expect("start");
    let addr = server.local_addr();

    let head = send(addr, "HEAD", "/stream", &[], b"");
    assert_eq!(head.status, 200);
    assert_eq!(
        head.headers.get("stream-next-offset").map(String::as_str),
        Some("-1"),
        "an empty log reads from the start"
    );
    assert!(!head.headers.contains_key("stream-closed"));
    assert!(
        !head.headers.contains_key("content-length"),
        "HEAD has no body length to promise"
    );

    let first = send(addr, "POST", "/stream", &[], envelope(0).as_bytes());
    assert_eq!(first.status, 204);
    assert_eq!(first.headers["stream-next-offset"], at(0));

    let second = send(addr, "POST", "/stream", &[], envelope(1).as_bytes());
    assert_eq!(second.status, 204);
    assert_eq!(second.headers["stream-next-offset"], at(1));

    let catch_up = send(addr, "GET", "/stream?offset=-1", &[], b"");
    assert_eq!(catch_up.status, 200);
    assert_eq!(
        body_lines(&catch_up),
        vec!["{\"n\":0}".to_string(), "{\"n\":1}".to_string()]
    );
    assert_eq!(catch_up.headers["stream-next-offset"], at(1));
    assert_eq!(
        catch_up
            .headers
            .get("stream-up-to-date")
            .map(String::as_str),
        Some("true")
    );
    assert!(!catch_up.headers.contains_key("stream-closed"));

    // The cursor is the last consumed record: reading from it is empty now.
    let empty = send(addr, "GET", &format!("/stream?offset={}", at(1)), &[], b"");
    assert_eq!(empty.status, 200);
    assert!(empty.body.is_empty());
    assert_eq!(empty.headers["stream-next-offset"], at(1));

    send(addr, "POST", "/stream", &[], envelope(2).as_bytes());
    let third = send(addr, "GET", &format!("/stream?offset={}", at(1)), &[], b"");
    assert_eq!(body_lines(&third), vec!["{\"n\":2}".to_string()]);
    assert_eq!(third.headers["stream-next-offset"], at(2));

    // Close-only is idempotent and reports the final cursor.
    let closed = send(addr, "POST", "/stream", &[("Stream-Closed", "true")], b"");
    assert_eq!(closed.status, 204);
    assert_eq!(closed.headers["stream-closed"], "true");
    assert_eq!(closed.headers["stream-next-offset"], at(2));

    let eof = send(addr, "GET", &format!("/stream?offset={}", at(2)), &[], b"");
    assert_eq!(eof.status, 200);
    assert!(eof.body.is_empty());
    assert_eq!(eof.headers["stream-closed"], "true");
    assert_eq!(eof.headers["stream-up-to-date"], "true");

    let head = send(addr, "HEAD", "/stream", &[], b"");
    assert_eq!(head.headers["stream-closed"], "true");
    assert_eq!(head.headers["stream-next-offset"], at(2));

    let refused = send(addr, "POST", "/stream", &[], envelope(3).as_bytes());
    assert_eq!(refused.status, 409);
    assert_eq!(refused.headers["stream-closed"], "true");
    assert_eq!(refused.headers["stream-next-offset"], at(2));

    assert_eq!(
        send(addr, "GET", "/stream?offset=999", &[], b"").status,
        400
    );
    assert_eq!(
        send(addr, "GET", "/stream?offset=nope", &[], b"").status,
        400
    );
    assert_eq!(send(addr, "GET", "/nope", &[], b"").status, 404);

    server.shutdown();
}

#[test]
fn append_and_close_is_atomic_over_http() {
    let log = Arc::new(MemLog::new());
    let mut server = HttpServer::start(Arc::clone(&log), "127.0.0.1:0").expect("start");
    let addr = server.local_addr();

    let body = envelope(0);
    let closed = send(
        addr,
        "POST",
        "/stream",
        &[("Stream-Closed", "true")],
        body.as_bytes(),
    );
    assert_eq!(closed.status, 204);
    assert_eq!(closed.headers["stream-closed"], "true");
    assert_eq!(closed.headers["stream-next-offset"], at(0));

    // Retrying the exact closing triple dedupes: same offset, still closed.
    let retry = send(
        addr,
        "POST",
        "/stream",
        &[("Stream-Closed", "true")],
        body.as_bytes(),
    );
    assert_eq!(retry.status, 204);
    assert_eq!(retry.headers["stream-next-offset"], at(0));
    assert_eq!(retry.headers["stream-closed"], "true");

    let all = send(addr, "GET", "/stream?offset=-1", &[], b"");
    assert_eq!(body_lines(&all), vec!["{\"n\":0}".to_string()]);
    assert_eq!(all.headers["stream-closed"], "true");
    assert_eq!(all.headers["stream-next-offset"], at(0));

    server.shutdown();
}

#[test]
fn sse_catches_up_then_tails_and_stops_at_eof() {
    let log = Arc::new(MemLog::new());
    log.append_json(&envelope(0)).unwrap();
    log.append_json(&envelope(1)).unwrap();
    let mut server = HttpServer::start(Arc::clone(&log), "127.0.0.1:0").expect("start");
    let addr = server.local_addr();

    let mut sse = connect_sse(addr, "/stream?offset=-1&live=sse");

    assert_eq!(sse.next().0, "data");
    let (event, data) = sse.next();
    assert_eq!(event, "data");
    assert!(data.contains("\"n\":1"), "got {data}");
    let (event, data) = sse.next();
    assert_eq!(event, "control");
    let control: serde_json::Value = serde_json::from_str(&data).expect("control json");
    assert_eq!(control["streamNextOffset"], at(1));
    assert_eq!(control["upToDate"], true);

    log.append_json(&envelope(2)).unwrap();
    let (event, data) = sse.next();
    assert_eq!(event, "data");
    assert!(data.contains("\"n\":2"), "got {data}");
    let (event, data) = sse.next();
    assert_eq!(event, "control");
    let control: serde_json::Value = serde_json::from_str(&data).expect("control json");
    assert_eq!(control["streamNextOffset"], at(2));
    assert_eq!(control["upToDate"], true);

    log.close().unwrap();
    let (event, data) = sse.next();
    assert_eq!(event, "control");
    let control: serde_json::Value = serde_json::from_str(&data).expect("control json");
    assert_eq!(control["streamClosed"], true);
    assert_eq!(control["streamNextOffset"], at(2));
    assert!(sse.at_eof(), "a closed stream ends the SSE connection");

    server.shutdown();
}

#[test]
fn sse_from_now_does_not_skip_the_next_append() {
    let log = Arc::new(MemLog::new());
    log.append_json(&envelope(0)).unwrap();
    log.append_json(&envelope(1)).unwrap();
    let mut server = HttpServer::start(Arc::clone(&log), "127.0.0.1:0").expect("start");
    let addr = server.local_addr();

    let mut sse = connect_sse(addr, "/stream?offset=now&live=sse");
    let (event, data) = sse.next();
    assert_eq!(event, "control", "no old record is replayed from now");
    let control: serde_json::Value = serde_json::from_str(&data).expect("control json");
    assert_eq!(control["streamNextOffset"], at(1));
    assert_eq!(control["upToDate"], true);

    log.append_json(&envelope(2)).unwrap();
    let (event, data) = sse.next();
    assert_eq!(event, "data");
    assert!(data.contains("\"n\":2"), "got {data}");

    let (event, _) = sse.next();
    assert_eq!(event, "control");
    log.close().unwrap();
    let (event, data) = sse.next();
    assert_eq!(event, "control");
    let control: serde_json::Value = serde_json::from_str(&data).expect("control json");
    assert_eq!(control["streamClosed"], true);
    assert!(sse.at_eof());

    server.shutdown();
}

#[test]
fn long_poll_waits_for_the_next_record_and_returns_eof() {
    let log = Arc::new(MemLog::new());
    log.append_json(&envelope(0)).unwrap();
    let mut server = HttpServer::start(Arc::clone(&log), "127.0.0.1:0").expect("start");
    let addr = server.local_addr();

    let writer = Arc::clone(&log);
    let appender = thread::spawn(move || {
        thread::sleep(Duration::from_millis(150));
        writer.append_json(&envelope(1)).unwrap();
    });

    let response = send(
        addr,
        "GET",
        "/stream?offset=now&live=long-poll&timeout=5",
        &[],
        b"",
    );
    appender.join().unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(
        body_lines(&response),
        vec!["{\"n\":1}".to_string()],
        "offset=now must wait for and return the next append"
    );
    assert_eq!(response.headers["stream-next-offset"], at(1));
    assert_eq!(response.headers["stream-up-to-date"], "true");
    assert!(!response.headers.contains_key("stream-closed"));

    log.close().unwrap();
    let started = Instant::now();
    let eof = send(
        addr,
        "GET",
        &format!("/stream?offset={}&live=long-poll&timeout=5", at(1)),
        &[],
        b"",
    );
    assert_eq!(eof.status, 200);
    assert!(eof.body.is_empty());
    assert_eq!(eof.headers["stream-next-offset"], at(1));
    assert_eq!(eof.headers["stream-up-to-date"], "true");
    assert_eq!(eof.headers["stream-closed"], "true");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a closed tail is EOF, not a wait"
    );

    server.shutdown();
}

#[test]
fn long_poll_times_out_empty_at_the_tail() {
    let log = Arc::new(MemLog::new());
    log.append_json(&envelope(0)).unwrap();
    let mut server = HttpServer::start(Arc::clone(&log), "127.0.0.1:0").expect("start");
    let addr = server.local_addr();

    let started = Instant::now();
    let response = send(
        addr,
        "GET",
        &format!("/stream?offset={}&live=long-poll&timeout=1", at(0)),
        &[],
        b"",
    );
    assert_eq!(response.status, 200);
    assert!(response.body.is_empty());
    assert_eq!(response.headers["stream-next-offset"], at(0));
    assert_eq!(response.headers["stream-up-to-date"], "true");
    assert!(!response.headers.contains_key("stream-closed"));
    assert!(
        started.elapsed() >= Duration::from_millis(700),
        "the request waited for the timeout"
    );

    for target in [
        "/stream?live=long-poll&timeout=0",
        "/stream?live=long-poll&timeout=31",
        "/stream?live=long-poll&timeout=nope",
        "/stream?live=poll",
    ] {
        assert_eq!(
            send(addr, "GET", target, &[], b"").status,
            400,
            "{target} must be rejected before any wait"
        );
    }

    server.shutdown();
}

#[test]
fn sse_resumes_from_the_last_cursor_after_a_file_log_restart() {
    let scratch = Scratch::new("restart");
    let log = Arc::new(FileLog::open(&scratch.0).expect("open"));
    let mut server = HttpServer::start(Arc::clone(&log), "127.0.0.1:0").expect("start");
    let addr = server.local_addr();

    let first = send(addr, "POST", "/stream", &[], envelope(0).as_bytes());
    assert_eq!(first.status, 204);
    let second = send(addr, "POST", "/stream", &[], envelope(1).as_bytes());
    assert_eq!(second.status, 204);

    let mut sse = connect_sse(addr, "/stream?offset=-1&live=sse");
    let (event, data) = sse.next();
    assert_eq!(event, "data");
    assert!(data.contains("\"n\":0"), "got {data}");
    let (event, data) = sse.next();
    assert_eq!(event, "data");
    assert!(data.contains("\"n\":1"), "got {data}");
    let (event, data) = sse.next();
    assert_eq!(event, "control");
    let control: serde_json::Value = serde_json::from_str(&data).expect("control json");
    let cursor = control["streamNextOffset"]
        .as_str()
        .expect("a cursor")
        .to_string();
    assert_eq!(cursor, at(1));
    drop(sse);

    server.shutdown();
    drop(server);
    drop(log);

    let log = Arc::new(FileLog::open(&scratch.0).expect("reopen"));
    assert_eq!(
        log.read_from(&Offset::Start).expect("read").len(),
        2,
        "the records survive the restart"
    );
    let mut server = HttpServer::start(Arc::clone(&log), "127.0.0.1:0").expect("restart");
    let addr = server.local_addr();

    let mut sse = connect_sse(addr, &format!("/stream?offset={cursor}&live=sse"));
    let (event, data) = sse.next();
    assert_eq!(
        event, "control",
        "the reopened log must not replay old records"
    );
    let control: serde_json::Value = serde_json::from_str(&data).expect("control json");
    assert_eq!(control["streamNextOffset"], cursor);
    assert_eq!(control["upToDate"], true);
    assert!(control.get("streamClosed").is_none());

    let appended = send(addr, "POST", "/stream", &[], envelope(2).as_bytes());
    assert_eq!(appended.status, 204);
    assert_eq!(appended.headers["stream-next-offset"], at(2));

    let (event, data) = sse.next();
    assert_eq!(event, "data");
    assert!(
        data.contains("\"n\":2"),
        "the resumed tail sees the new append: {data}"
    );
    let (event, data) = sse.next();
    assert_eq!(event, "control");
    let control: serde_json::Value = serde_json::from_str(&data).expect("control json");
    assert_eq!(control["streamNextOffset"], at(2));
    assert_eq!(control["upToDate"], true);

    drop(sse);
    server.shutdown();
}

#[test]
fn a_reader_sees_the_closing_append_and_eof_in_one_page() {
    let log = Arc::new(MemLog::new());
    log.append_json(&envelope(0)).unwrap();
    let mut server = HttpServer::start(Arc::clone(&log), "127.0.0.1:0").expect("start");
    let addr = server.local_addr();

    let closing = send(
        addr,
        "POST",
        "/stream",
        &[("Stream-Closed", "true")],
        envelope(1).as_bytes(),
    );
    assert_eq!(closing.status, 204);

    let page = send(addr, "GET", &format!("/stream?offset={}", at(0)), &[], b"");
    assert_eq!(body_lines(&page), vec!["{\"n\":1}".to_string()]);
    assert_eq!(page.headers["stream-next-offset"], at(1));
    assert_eq!(page.headers["stream-closed"], "true");
    assert_eq!(page.headers["stream-up-to-date"], "true");

    server.shutdown();
}
