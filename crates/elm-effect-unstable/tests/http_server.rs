//! Listener-stack fence: explicit bind, route dispatch, 404 fallback.

use elm_effect_unstable::http_server::{HttpServer, Response};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

fn get(addr: &std::net::SocketAddr, path: &str) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut stream = TcpStream::connect(addr).expect("connect");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
    )
    .expect("write");
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).expect("status");
    let code: u16 = status
        .split_whitespace()
        .nth(1)
        .expect("code")
        .parse()
        .expect("code");
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("header");
        let line = line.trim_end().to_owned();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    let mut body = Vec::new();
    use std::io::Read as _;
    reader.read_to_end(&mut body).expect("body");
    (code, headers, body)
}

#[test]
fn binds_loopback_and_serves_one_route() {
    let server = HttpServer::bind("127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("addr");
    assert!(addr.ip().is_loopback());
    server.route("GET", "/hello", Response::ok("hello"));
    let handle = std::thread::spawn(move || server.serve_once().expect("serve"));
    let (code, _headers, body) = get(&addr, "/hello");
    assert_eq!(code, 200);
    assert_eq!(body, b"hello");
    assert_eq!(
        handle.join().expect("thread"),
        ("GET".to_owned(), "/hello".to_owned())
    );
}

#[test]
fn unknown_route_is_404() {
    let server = HttpServer::bind("127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("addr");
    let handle = std::thread::spawn(move || server.serve_once().expect("serve"));
    let (code, _headers, body) = get(&addr, "/missing");
    assert_eq!(code, 404);
    assert!(body.is_empty());
    assert_eq!(
        handle.join().expect("thread"),
        ("GET".to_owned(), "/missing".to_owned())
    );
}

#[test]
fn explicit_host_is_preserved() {
    let server = HttpServer::bind("127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("addr");
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
}

#[test]
fn match_path_extracts_params() {
    use elm_effect_unstable::http_server::match_path;
    assert_eq!(
        match_path("/users/:id", "/users/7"),
        Some(vec![("id".to_string(), "7".to_string())])
    );
    assert_eq!(match_path("/users/:id", "/users/"), None);
    assert_eq!(match_path("/a", "/a/b"), None);
    assert_eq!(match_path("/a/:x", "/a/b?x=1#frag"), Some(vec![("x".to_string(), "b".to_string())]));
}

#[test]
fn patterned_route_serves_with_query_stripped() {
    let server = HttpServer::bind("127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("addr");
    server.route("GET", "/users/:id", Response::ok("user"));
    let handle = std::thread::spawn(move || server.serve_once().expect("serve"));
    let (code, _, _) = get(&addr, "/users/7?verbose=1");
    assert_eq!(code, 200);
    let _ = handle.join().expect("join");
}
