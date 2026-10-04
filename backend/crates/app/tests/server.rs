// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The accept loop, exercised over real loopback sockets.

mod common;

use baze_app::config::AppConfig;
use baze_app::server::{ServerOptions, serve};
use common::{test_app_with, test_config};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

const SSE_REQUEST: &str = "GET /api/v1/realtime/sse?min_lon=-70.7&min_lat=-33.5&max_lon=-70.6&max_lat=-33.4 HTTP/1.1\r\nHost: x\r\nAccept: text/event-stream\r\n\r\n";

struct Running {
    addr: SocketAddr,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<std::io::Result<()>>,
}

async fn start(config: AppConfig, options: ServerOptions) -> Running {
    let app = test_app_with(config);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown, signal) = oneshot::channel::<()>();
    let task = tokio::spawn(serve(listener, app, options, async move {
        let _ = signal.await;
    }));
    Running {
        addr,
        shutdown,
        task,
    }
}

fn fast_options() -> ServerOptions {
    ServerOptions {
        header_read_timeout: Duration::from_millis(300),
        shutdown_grace: Duration::from_millis(500),
    }
}

/// Reads until the end of the response head and returns it.
async fn read_head(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 256];
    let deadline = Instant::now() + Duration::from_secs(3);
    while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
        assert!(Instant::now() < deadline, "no response head: {buffer:?}");
        let n = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut chunk))
            .await
            .expect("read timed out")
            .expect("read failed");
        assert!(
            n > 0,
            "connection closed before a response head: {buffer:?}"
        );
        buffer.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

#[tokio::test]
async fn serves_a_normal_request() {
    let server = start(test_config(), fast_options()).await;

    let mut stream = TcpStream::connect(server.addr).await.unwrap();
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();

    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("\"status\":\"ok\""), "{response}");
}

#[tokio::test]
async fn a_client_that_never_finishes_its_headers_is_cut_off() {
    let server = start(test_config(), fast_options()).await;

    let mut stream = TcpStream::connect(server.addr).await.unwrap();
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nX-Slow: ")
        .await
        .unwrap();
    let started = Instant::now();
    let mut sink = Vec::new();
    // The server answers with an error or just closes; either way the read ends.
    let result = tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut sink)).await;

    assert!(result.is_ok(), "the connection was still open after 3 s");
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(250), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
}

#[tokio::test]
async fn a_stalled_client_does_not_prevent_others_from_being_served() {
    let server = start(test_config(), fast_options()).await;
    let mut stalled = TcpStream::connect(server.addr).await.unwrap();
    stalled
        .write_all(b"GET /health HTTP/1.1\r\n")
        .await
        .unwrap();

    let mut stream = TcpStream::connect(server.addr).await.unwrap();
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();

    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
}

#[tokio::test]
async fn an_event_stream_outlives_the_request_deadline() {
    let mut config = test_config();
    config.request_timeout_secs = 1;
    let server = start(config, fast_options()).await;

    let mut stream = TcpStream::connect(server.addr).await.unwrap();
    stream.write_all(SSE_REQUEST.as_bytes()).await.unwrap();
    let head = read_head(&mut stream).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(
        head.to_ascii_lowercase().contains("text/event-stream"),
        "{head}"
    );

    // Past the 1 s request deadline the stream must still be open: nothing arrives, but no EOF either.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let mut chunk = [0u8; 64];
    let outcome = tokio::time::timeout(Duration::from_millis(300), stream.read(&mut chunk)).await;
    assert!(
        outcome.is_err(),
        "the stream was closed or sent data: {outcome:?}"
    );
}

#[tokio::test]
async fn shutdown_returns_within_the_grace_period_even_with_an_open_stream() {
    let server = start(test_config(), fast_options()).await;
    let mut stream = TcpStream::connect(server.addr).await.unwrap();
    stream.write_all(SSE_REQUEST.as_bytes()).await.unwrap();
    read_head(&mut stream).await;

    let started = Instant::now();
    server.shutdown.send(()).unwrap();
    let finished = tokio::time::timeout(Duration::from_secs(5), server.task).await;

    let result = finished
        .expect("serve() did not return")
        .expect("task panicked");
    assert!(result.is_ok());
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn after_shutdown_new_connections_are_refused() {
    let server = start(test_config(), fast_options()).await;
    let addr = server.addr;

    server.shutdown.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), server.task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    assert!(TcpStream::connect(addr).await.is_err());
}

#[tokio::test]
async fn the_container_healthcheck_probe_sees_a_healthy_server() {
    let server = start(test_config(), fast_options()).await;
    let addr = server.addr;

    let healthy = tokio::task::spawn_blocking(move || baze_app::healthcheck::probe(addr))
        .await
        .unwrap();

    assert!(healthy);
}

#[tokio::test]
async fn the_healthcheck_probe_fails_when_nothing_is_listening() {
    // Bind and drop to get a port that is certainly closed.
    let addr = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap()
    };

    let healthy = tokio::task::spawn_blocking(move || baze_app::healthcheck::probe(addr))
        .await
        .unwrap();

    assert!(!healthy);
}

#[tokio::test]
async fn the_healthcheck_probe_rejects_a_server_that_answers_an_error() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            use std::io::Write;
            let _ =
                stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n");
        }
    });

    let healthy = tokio::task::spawn_blocking(move || baze_app::healthcheck::probe(addr))
        .await
        .unwrap();

    assert!(!healthy);
}
