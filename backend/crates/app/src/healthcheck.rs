// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `baze-server healthcheck`: the probe behind the container HEALTHCHECK. The runtime image has no
//! curl or shell tooling on purpose, so the binary checks itself with a minimal HTTP/1.1 request.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);

/// True when `GET /health` on `addr` answers `200 OK`.
pub fn probe(addr: SocketAddr) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, TIMEOUT) else {
        return false;
    };
    if stream.set_read_timeout(Some(TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(TIMEOUT)).is_err()
    {
        return false;
    }
    let request = b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    if stream.write_all(request).is_err() {
        return false;
    }
    // The status line is all that matters; do not read an unbounded body.
    let mut head = [0u8; 64];
    let mut filled = 0;
    while filled < head.len() {
        match stream.read(&mut head[filled..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => filled += n,
        }
    }
    head[..filled].starts_with(b"HTTP/1.1 200")
}

/// Probes the port the server itself would listen on (`PORT`, default 8080) on the loopback interface.
pub fn probe_local() -> bool {
    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(8080);
    probe(SocketAddr::from(([127, 0, 0, 1], port)))
}
