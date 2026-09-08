//! The routes that answer without the job daemon, driven over a real socket.
//! Everything that needs the daemon is in the CLI crate's `rest_api` test, which
//! can point a child process at a daemon address of its own.

use dcpwizard_core::rest_api::bind_rest_api;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};

const API_KEY: &str = "correct-horse-battery-staple";

fn serve(api_key: Option<&str>) -> SocketAddr {
    let (server, listener) = bind_rest_api("127.0.0.1:0", api_key).expect("bind the API");
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || server.serve_forever(listener).unwrap());
    address
}

fn send(address: SocketAddr, raw: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    stream.write_all(raw.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn get(address: SocketAddr, path: &str, headers: &str) -> String {
    send(
        address,
        &format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n{headers}Connection: close\r\n\r\n"),
    )
}

fn post(address: SocketAddr, path: &str, body: &str) -> String {
    send(
        address,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

#[test]
fn health_answers_without_a_key() {
    let address = serve(None);
    let response = get(address, "/health", "");
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.ends_with(r#"{"status":"ok"}"#), "{response}");
}

#[test]
fn an_unknown_path_is_404() {
    let address = serve(None);
    assert!(
        get(address, "/nope", "").starts_with("HTTP/1.1 404"),
        "an unregistered path must not reach a handler"
    );
}

#[test]
fn a_key_guards_every_path_but_health() {
    let address = serve(Some(API_KEY));
    assert!(
        get(address, "/health", "").starts_with("HTTP/1.1 200 OK"),
        "/health is the exempt path"
    );
    assert!(
        get(address, "/daemon-status", "").starts_with("HTTP/1.1 401"),
        "a guarded path answers nothing without the key"
    );
}

#[test]
fn either_key_header_authorizes() {
    let address = serve(Some(API_KEY));
    for header in [
        format!("X-Api-Key: {API_KEY}\r\n"),
        format!("Authorization: Bearer {API_KEY}\r\n"),
    ] {
        let response = get(address, "/daemon-status", &header);
        assert!(
            response.starts_with("HTTP/1.1 200 OK"),
            "{header}: {response}"
        );
    }
    let wrong = get(
        address,
        "/daemon-status",
        &format!("X-Api-Key: {API_KEY}x\r\n"),
    );
    assert!(wrong.starts_with("HTTP/1.1 401"), "{wrong}");
}

#[test]
fn a_key_in_the_body_does_not_authorize() {
    let address = serve(Some(API_KEY));
    let response = post(address, "/create", &format!(r#"{{"key":"{API_KEY}"}}"#));
    assert!(
        response.starts_with("HTTP/1.1 401"),
        "only a header carries the key: {response}"
    );
}

#[test]
fn a_create_body_that_is_not_a_config_is_400() {
    let address = serve(None);
    let response = post(address, "/create", "not json at all");
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    assert!(
        response.contains("Invalid config"),
        "the 400 must name the problem: {response}"
    );
}

#[test]
fn an_empty_verify_body_is_400() {
    let address = serve(None);
    let response = post(address, "/verify", "");
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    assert!(
        response.contains("Missing DCP path"),
        "the 400 must name the problem: {response}"
    );
}
