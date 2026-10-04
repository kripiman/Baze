#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Regression checks for the findings of the October 2026 security/functionality audit.

Starts the real `baze-server` binary on 127.0.0.1 (never any other host) in production
mode and replays the attacks that were reproduced during the audit. Each check declares
the remediation step (`since`) after which it must pass, so the script doubles as a
progress meter: run it with `--through N` to enforce every check up to step N.

    cargo build --manifest-path backend/Cargo.toml -p baze-app
    python3 scripts/e2e/audit_regression.py                 # report only
    python3 scripts/e2e/audit_regression.py --through 3     # fail if a step <= 3 regressed

Standard library only.
"""

import argparse
import hashlib
import hmac
import http.client
import json
import os
import secrets
import socket
import subprocess
import sys
import tempfile
import time
import uuid
from contextlib import contextmanager

REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
DEFAULT_BINARY = os.path.join(REPO, "backend", "target", "debug", "baze-server")

# The value the project used to ship in infra/.env.example. It is public, so every attacker knows it.
PUBLIC_EXAMPLE_SECRET = "change_me_to_a_random_32_bytes_secret_key_in_production"
# Fixtures are generated on every run: no key-like literal is committed to the repository.
STRONG_SECRET = secrets.token_hex(32)
DB_URL = f"postgres://baze_app:{secrets.token_urlsafe(18)}@127.0.0.1:1/baze_db"  # lazy pool, never connected
GOOD_COMMIT = "0123456789abcdef0123456789abcdef01234567"

POINT = {"type": "Point", "coordinates": [-70.65, -33.45]}
WORLD = "min_lon=-180&min_lat=-90&max_lon=180&max_lat=90"


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def prod_env(**overrides):
    env = {
        "PATH": os.environ.get("PATH", ""),
        "ENVIRONMENT": "production",
        "DATABASE_URL": DB_URL,
        "JWT_SECRET": STRONG_SECRET,
        "GIT_COMMIT_HASH": GOOD_COMMIT,
        "HOST": "127.0.0.1",
    }
    env.update({k: v for k, v in overrides.items() if v is not None})
    for k, v in overrides.items():
        if v is None:
            env.pop(k, None)
    return env


@contextmanager
def server(binary, **env_overrides):
    port = free_port()
    env = prod_env(PORT=str(port), **env_overrides)
    log = tempfile.TemporaryFile()
    proc = subprocess.Popen([binary], env=env, stdout=log, stderr=log)
    try:
        for _ in range(100):
            if proc.poll() is not None:
                log.seek(0)
                raise RuntimeError("server exited early: " + log.read().decode(errors="replace")[-400:])
            try:
                http.client.HTTPConnection("127.0.0.1", port, timeout=1).request("GET", "/health")
                break
            except OSError:
                time.sleep(0.1)
        else:
            raise RuntimeError("server did not become ready")
        yield port, proc
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
        log.close()


def boot_exit(binary, **env_overrides):
    """Run the binary and return (exit_code_or_None_if_still_running, output)."""
    port = free_port()
    env = prod_env(PORT=str(port), **env_overrides)
    with tempfile.TemporaryFile() as log:
        proc = subprocess.Popen([binary], env=env, stdout=log, stderr=log)
        try:
            code = proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            code = None
            proc.terminate()
            proc.wait(timeout=5)
        log.seek(0)
        return code, log.read().decode(errors="replace")


def call(port, method, path, body=None, headers=None, raw=None, ip=None, timeout=10):
    h = dict(headers or {})
    if ip:
        h["X-Real-IP"] = ip  # honoured only because 127.0.0.1 is a trusted proxy
    data = raw
    if raw is None and body is not None:
        data = json.dumps(body).encode()
        h.setdefault("Content-Type", "application/json")
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    try:
        conn.request(method, path, body=data, headers=h)
        r = conn.getresponse()
        return r.status, r.read().decode(errors="replace"), dict(r.getheaders())
    except (OSError, http.client.HTTPException) as e:
        return 0, str(e), {}
    finally:
        conn.close()


def signup(port, ip):
    status, text, _ = call(port, "POST", "/api/v1/auth/anonymous", ip=ip)
    return json.loads(text)["token"] if status == 201 else None


def forge_v1(secret):
    u = uuid.uuid4()
    return f"baze_anon_{u}." + hmac.new(secret.encode(), u.bytes, hashlib.sha256).hexdigest(), u


def bearer(token):
    return {"Authorization": f"Bearer {token}"}


# ----------------------------------------------------------------------------- checks
CHECKS = []


def check(cid, title, since):
    def deco(fn):
        CHECKS.append((cid, title, since, fn))
        return fn

    return deco


@check("SEC-01", "boot refuses the public JWT_SECRET that used to ship in .env.example", 2)
def _(binary):
    code, out = boot_exit(binary, JWT_SECRET=PUBLIC_EXAMPLE_SECRET)
    return code not in (None, 0), f"exit={code}"


@check("SEC-09", "boot refuses extreme or invalid numeric settings", 2)
def _(binary):
    bad = [
        {"RATE_LIMIT_REQUESTS_PER_MINUTE": "0"},
        {"RATE_LIMIT_REQUESTS_PER_MINUTE": "abc"},
        {"HAZARD_DEFAULT_TTL_HOURS": "9223372036854775807"},
        {"HAZARD_CONFIRMATION_THRESHOLD": "0"},
        {"HAZARD_CONFIRMATION_THRESHOLD": "1"},
    ]
    results = [boot_exit(binary, **b)[0] not in (None, 0) for b in bad]
    return all(results), f"rejected={results}"


@check("SEC-09", "production boot requires a real git commit hash", 2)
def _(binary):
    code, _ = boot_exit(binary, GIT_COMMIT_HASH=None)
    return code not in (None, 0), f"exit={code}"


@check("FUN-01", "routing and geocoding answer 501 instead of fake data", 3)
def _(binary):
    with server(binary) as (port, _):
        s1, _, _ = call(
            port,
            "POST",
            "/api/v1/routing/route",
            {"origin": POINT, "destination": {"type": "Point", "coordinates": [-71.55, -33.02]}},
            ip="198.51.100.10",
        )
        s2, _, _ = call(port, "GET", "/api/v1/geocoding/search?q=plaza", ip="198.51.100.10")
        return (s1, s2) == (501, 501), f"routing={s1} geocoding={s2}"


@check("SEC-02", "varying Authorization no longer evades the signup limit", 0)
def _(binary):
    with server(binary) as (port, _):
        codes = [
            call(port, "POST", "/api/v1/auth/anonymous", headers={"Authorization": f"Bearer x{i}"}, ip="198.51.100.20")[0]
            for i in range(150)
        ]
        return codes.count(429) > 0 and codes.count(201) <= 60, f"201={codes.count(201)} 429={codes.count(429)}"


@check("SEC-02", "expensive endpoints are rate limited too", 0)
def _(binary):
    body = {"origin": POINT, "destination": {"type": "Point", "coordinates": [-70.60, -33.40]}}
    with server(binary) as (port, _):
        codes = [call(port, "POST", "/api/v1/routing/route", body, ip="198.51.100.21")[0] for _ in range(150)]
        reads = [call(port, "GET", f"/api/v1/hazards?{WORLD}", ip="198.51.100.22")[0] for _ in range(150)]
        return 429 in codes and 429 in reads, f"route429={codes.count(429)} list429={reads.count(429)}"


@check("SEC-04", "public hazard payload does not expose the creator account", 0)
def _(binary):
    with server(binary) as (port, _):
        tok = signup(port, "198.51.100.30")
        s, text, _ = call(port, "POST", "/api/v1/hazards", {"category": "glass", "location": POINT}, bearer(tok), ip="198.51.100.30")
        listing = call(port, "GET", "/api/v1/hazards?min_lon=-70.7&min_lat=-33.5&max_lon=-70.6&max_lat=-33.4", ip="198.51.100.31")[1]
        return s == 201 and "creator" not in text and "creator" not in listing, f"create={s}"


@check("SEC-03", "hazard type is derived from the category, a conflicting type cannot be injected", 0)
def _(binary):
    with server(binary) as (port, _):
        tok = signup(port, "198.51.100.32")
        s, text, _ = call(
            port, "POST", "/api/v1/hazards", {"category": "glass", "hazard_type": "blocking", "location": POINT}, bearer(tok), ip="198.51.100.32"
        )
        hazard_type = json.loads(text).get("hazard_type") if s == 201 else text[:60]
        return s in (201, 400) and hazard_type != "blocking", f"status={s} type={hazard_type}"


@check("SEC-02", "SSE connections are capped per client network", 0)
def _(binary):
    with server(binary) as (port, _):
        socks, rejected = [], False
        try:
            for _ in range(60):
                s = socket.create_connection(("127.0.0.1", port), timeout=3)
                s.sendall(
                    b"GET /api/v1/realtime/sse?min_lon=-70.7&min_lat=-33.5&max_lon=-70.6&max_lat=-33.4 HTTP/1.1\r\n"
                    b"Host: x\r\nX-Real-IP: 198.51.100.40\r\nAccept: text/event-stream\r\n\r\n"
                )
                socks.append(s)
                head = s.recv(32)
                if head.startswith(b"HTTP/1.1 429") or head.startswith(b"HTTP/1.1 503"):
                    rejected = True
                    break
        finally:
            for s in socks:
                s.close()
        return rejected, f"opened={len(socks)} rejected={rejected}"


@check("SEC-05", "GET /hazards rejects a world-sized bounding box", 4)
def _(binary):
    with server(binary) as (port, _):
        s, _, _ = call(port, "GET", f"/api/v1/hazards?{WORLD}", ip="198.51.100.50")
        return s == 400, f"status={s}"


@check("SEC-05", "request bodies above 16 KiB are rejected with 413", 4)
def _(binary):
    with server(binary) as (port, _):
        tok = signup(port, "198.51.100.51")
        s, _, _ = call(
            port, "POST", "/api/v1/hazards", raw=b"x" * (20 * 1024), headers={**bearer(tok), "Content-Type": "application/json"}, ip="198.51.100.51"
        )
        return s == 413, f"status={s}"


@check("SEC-05", "a connection that never finishes its headers is closed", 4)
def _(binary):
    with server(binary) as (port, _):
        s = socket.create_connection(("127.0.0.1", port), timeout=15)
        s.sendall(b"GET /health HTTP/1.1\r\nHost: x\r\nX-Slow: ")
        start = time.time()
        s.settimeout(15)
        try:
            data = s.recv(64)  # b"" or a 408 when the server gives up, a timeout when it never does
            closed = True
        except socket.timeout:
            closed, data = False, b""
        finally:
            s.close()
        return closed and time.time() - start <= 12, f"closed={closed} after={time.time() - start:.1f}s"


@check("SEC-06", "Bearer scheme is case-insensitive (RFC 9110)", 4)
def _(binary):
    with server(binary) as (port, _):
        tok = signup(port, "198.51.100.52")
        s, _, _ = call(
            port, "POST", "/api/v1/hazards", {"category": "glass", "location": POINT}, {"Authorization": f"bearer {tok}"}, ip="198.51.100.52"
        )
        return s == 201, f"status={s}"


@check("SEC-10", "API documentation is not served in production", 4)
def _(binary):
    with server(binary) as (port, _):
        s1, _, _ = call(port, "GET", "/swagger-ui/", ip="198.51.100.53")
        s2, _, _ = call(port, "GET", "/api-docs/openapi.json", ip="198.51.100.53")
        return (s1, s2) == (404, 404), f"swagger={s1} openapi={s2}"


@check("SEC-11", "description limit counts characters and rejects control characters", 5)
def _(binary):
    with server(binary) as (port, _):
        tok = signup(port, "198.51.100.60")

        def post(desc):
            return call(
                port, "POST", "/api/v1/hazards", {"category": "glass", "description": desc, "location": POINT}, bearer(tok), ip="198.51.100.60"
            )[0]

        r = (post("ñ" * 300), post("a" * 501), post("\u0000x"), post("a‮b"))
        return r == (201, 400, 400, 400), f"300xñ,501,NUL,bidi={r}"


@check("SEC-06", "only the canonical token form authenticates (no uppercase or alternate UUID spellings)", 8)
def _(binary):
    with server(binary) as (port, _):
        tok = signup(port, "198.51.100.61")
        body, sig = tok.rsplit(".", 1)
        variants = [body + "." + sig.upper(), body.replace("-", "") + "." + sig]
        codes = [
            call(port, "POST", "/api/v1/hazards", {"category": "glass", "location": POINT}, bearer(v), ip="198.51.100.61")[0] for v in variants
        ]
        return all(c == 401 for c in codes), f"variants={codes}"


@check("SEC-01", "legacy v1 tokens (HMAC over the bare UUID, no expiry) are rejected", 8)
def _(binary):
    with server(binary) as (port, _):
        tok, _ = forge_v1(STRONG_SECRET)  # the harness knows the real secret: only the token format is under test
        s, _, _ = call(port, "POST", "/api/v1/hazards", {"category": "glass", "location": POINT}, bearer(tok), ip="198.51.100.62")
        return s == 401, f"status={s}"


@check("SEC-03", "three fresh accounts on three networks cannot confirm a blocking hazard", 10)
def _(binary):
    with server(binary) as (port, _):
        t1 = signup(port, "203.0.113.1")
        s, text, _ = call(port, "POST", "/api/v1/hazards", {"category": "road_closed", "location": POINT}, bearer(t1), ip="203.0.113.1")
        hid = json.loads(text)["id"]
        status = None
        for n in (2, 3):
            t = signup(port, f"203.0.{n}.1")
            s, text, _ = call(port, "POST", f"/api/v1/hazards/{hid}/vote", {"vote": 1}, bearer(t), ip=f"203.0.{n}.1")
            status = json.loads(text).get("status") if s == 200 else f"http {s}"
        return status != "confirmed", f"final status={status}"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--binary", default=DEFAULT_BINARY, help="path to baze-server (default: backend debug build)")
    ap.add_argument("--through", type=int, default=-1, help="enforce every check whose step is <= N (default: report only)")
    ap.add_argument("--only", help="comma-separated finding ids to run, e.g. SEC-01,SEC-02")
    args = ap.parse_args()

    if not os.access(args.binary, os.X_OK):
        sys.exit(f"binary not found: {args.binary} (build it first)")

    wanted = set(args.only.split(",")) if args.only else None
    failures = 0
    print(f"{'finding':8} {'step':>4}  {'result':6} check")
    for cid, title, since, fn in CHECKS:
        if wanted and cid not in wanted:
            continue
        try:
            ok, detail = fn(args.binary)
        except Exception as e:  # a crashed check is a failed check
            ok, detail = False, f"error: {e}"
        enforced = since <= args.through
        label = "PASS" if ok else ("FAIL" if enforced else "open")
        if enforced and not ok:
            failures += 1
        print(f"{cid:8} {since:>4}  {label:6} {title}  [{detail}]")
    print(f"\n{failures} enforced failure(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
