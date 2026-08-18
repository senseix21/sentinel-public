"""Loopback-only development login and SENTINEL trust-boundary proxy.

This is deliberately not a deployment authentication provider. It owns a
bounded, in-memory browser session for ``make run`` and asserts a principal only
after that session is established. Production human authentication remains the
responsibility of the identity-aware TLS reverse proxy chosen in ADR 009.

Environment: UPSTREAM_PORT, LISTEN_PORT, PROOF_SECRET, PRINCIPAL,
DEV_USERNAME, DEV_PASSWORD, and optional DEV_SESSION_TTL_SECONDS.
"""

import hashlib
import hmac
import html
import http.client
import json
import os
import secrets
import sys
import threading
import time
from http import cookies
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

MAX_BODY_BYTES = 4096
MAX_SESSIONS = 128
MAX_FAILURE_CLIENTS = 128
MAX_FAILURES_PER_MINUTE = 5
SESSION_COOKIE = "sentinel_dev_session"
STRIP = {
    "connection", "keep-alive", "proxy-authenticate", "proxy-authorization",
    "te", "trailer", "transfer-encoding", "upgrade", "host", "cookie",
    "x-forwarded-for", "x-forwarded-proof", "x-authenticated-principal",
}


class SessionStore:
    """Bounded in-memory sessions and per-client login failure windows."""

    def __init__(self, ttl_seconds):
        self.ttl_seconds = ttl_seconds
        self.sessions = {}
        self.failures = {}
        self.lock = threading.Lock()

    def create(self):
        now = time.monotonic()
        with self.lock:
            self._prune(now)
            if len(self.sessions) >= MAX_SESSIONS:
                return None
            token = secrets.token_urlsafe(32)
            self.sessions[token] = now + self.ttl_seconds
            return token

    def valid(self, token):
        if not token:
            return False
        now = time.monotonic()
        with self.lock:
            expires = self.sessions.get(token)
            if expires is None:
                return False
            if expires <= now:
                self.sessions.pop(token, None)
                return False
            return True

    def revoke(self, token):
        if not token:
            return
        with self.lock:
            self.sessions.pop(token, None)

    def throttled(self, client):
        now = time.monotonic()
        with self.lock:
            recent = [value for value in self.failures.get(client, ()) if now - value < 60]
            if recent:
                self.failures[client] = recent
            else:
                self.failures.pop(client, None)
            return len(recent) >= MAX_FAILURES_PER_MINUTE

    def record_failure(self, client):
        now = time.monotonic()
        with self.lock:
            self._prune(now)
            if client not in self.failures and len(self.failures) >= MAX_FAILURE_CLIENTS:
                oldest = min(self.failures, key=lambda key: self.failures[key][-1])
                self.failures.pop(oldest, None)
            recent = [value for value in self.failures.get(client, ()) if now - value < 60]
            recent.append(now)
            self.failures[client] = recent

    def clear_failures(self, client):
        with self.lock:
            self.failures.pop(client, None)

    def _prune(self, now):
        self.sessions = {
            token: expires for token, expires in self.sessions.items() if expires > now
        }
        self.failures = {
            client: [value for value in values if now - value < 60]
            for client, values in self.failures.items()
            if any(now - value < 60 for value in values)
        }


def bounded_ttl(raw):
    try:
        value = int(raw)
    except ValueError as error:
        raise ValueError("DEV_SESSION_TTL_SECONDS must be an integer") from error
    if value < 60 or value > 86400:
        raise ValueError("DEV_SESSION_TTL_SECONDS must be between 60 and 86400")
    return value


def login_page(error=None):
    message = ""
    if error:
        message = '<div class="error" role="alert">%s</div>' % html.escape(error)
    return """<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>SENTINEL · Sign in</title>
<style>
:root{color-scheme:dark;--bg:#071018;--panel:#101c26;--line:#263744;--text:#e9f1f6;--muted:#91a4b2;--accent:#41d6a3;--bad:#ff8b8b}
*{box-sizing:border-box}body{margin:0;min-height:100vh;display:grid;place-items:center;background:radial-gradient(circle at 50%% 0,#173044 0,var(--bg) 48%%);color:var(--text);font:15px system-ui,sans-serif}
main{width:min(420px,calc(100%% - 32px));padding:32px;border:1px solid var(--line);border-radius:16px;background:rgba(16,28,38,.96);box-shadow:0 24px 80px #0008}
h1{margin:0;letter-spacing:.24em;font-size:28px}h1 span{color:var(--accent)}.mode{display:inline-block;margin:12px 0 24px;padding:5px 9px;border:1px solid #836b35;border-radius:999px;color:#e8c875;font-size:11px;letter-spacing:.12em}
p{color:var(--muted);line-height:1.5}label{display:block;margin:16px 0 6px;font-weight:650}input{width:100%%;padding:12px;border:1px solid var(--line);border-radius:8px;background:#08131c;color:var(--text);font:inherit}input:focus{outline:2px solid var(--accent);outline-offset:2px}
button{width:100%%;margin-top:22px;padding:12px;border:0;border-radius:8px;background:var(--accent);color:#04140f;font:700 15px system-ui;cursor:pointer}.error{margin:0 0 16px;padding:10px;border:1px solid #713c3c;border-radius:8px;color:var(--bad);background:#281719}.warning{margin-top:24px;padding-top:18px;border-top:1px solid var(--line);font-size:12px}
</style></head><body><main>
<h1><span>●</span> SENTINEL</h1><div class="mode">LOCAL DEVELOPMENT</div>
%s
<p>Sign in to the local surveillance console.</p>
<form method="post" action="/login">
<label for="username">Username</label><input id="username" name="username" autocomplete="username" required autofocus>
<label for="password">Password</label><input id="password" name="password" type="password" autocomplete="current-password" required>
<button type="submit">Sign in</button></form>
<p class="warning">This loopback-only login is a development fixture, not the production identity provider.</p>
</main></body></html>""" % message


class Proxy(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    upstream = None
    proof = None
    principal = None
    username = None
    password = None
    origins = None
    allowed_hosts = None
    sessions = None

    def log_message(self, fmt, *args):
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))

    def _send(self, status, body=b"", content_type="text/plain; charset=utf-8", headers=None):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header(
            "Content-Security-Policy",
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; "
            "base-uri 'none'; frame-ancestors 'none'",
        )
        # Authentication requests can be rejected before their body is read
        # (bad origin, throttling, excessive length). Closing prevents unread
        # bytes from being parsed as a second request on this connection.
        self.send_header("Connection", "close")
        self.close_connection = True
        if headers:
            for key, value in headers:
                self.send_header(key, value)
        self.end_headers()
        if self.command != "HEAD" and body:
            self.wfile.write(body)

    def _session_token(self):
        raw = self.headers.get("Cookie")
        if not raw:
            return None
        parsed = cookies.SimpleCookie()
        try:
            parsed.load(raw)
        except cookies.CookieError:
            return None
        morsel = parsed.get(SESSION_COOKIE)
        return morsel.value if morsel else None

    def _same_origin(self, require_evidence=False):
        host = self.headers.get("Host", "")
        if host not in self.allowed_hosts:
            return False
        origin = self.headers.get("Origin")
        if origin and origin != "null":
            return any(hmac.compare_digest(origin, allowed) for allowed in self.origins)
        referer = self.headers.get("Referer")
        if referer:
            parts = urlsplit(referer)
            observed = f"{parts.scheme}://{parts.netloc}"
            return any(hmac.compare_digest(observed, allowed) for allowed in self.origins)
        # Some browsers omit Origin on an ordinary same-origin form POST, and
        # this page deliberately suppresses Referer. For login only, an exact
        # loopback Host is sufficient. Authenticated mutations still require
        # explicit evidence in `_require_session_and_origin` below.
        return not require_evidence

    def _read_form(self):
        content_type = self.headers.get("Content-Type", "").split(";", 1)[0].strip().lower()
        if content_type != "application/x-www-form-urlencoded":
            return None
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            return None
        if length <= 0 or length > MAX_BODY_BYTES:
            return None
        raw = self.rfile.read(length)
        try:
            return parse_qs(raw.decode("utf-8"), keep_blank_values=True, strict_parsing=True)
        except (UnicodeDecodeError, ValueError):
            return None

    def _login(self):
        if not self._same_origin():
            self._send(403, b"forbidden\n")
            return
        client = self.client_address[0]
        if self.sessions.throttled(client):
            self._send(429, login_page("Too many attempts. Try again shortly.").encode(), "text/html; charset=utf-8")
            return
        form = self._read_form()
        username = form.get("username", [""])[0] if form else ""
        password = form.get("password", [""])[0] if form else ""
        username_valid = hmac.compare_digest(username, self.username)
        password_valid = hmac.compare_digest(password, self.password)
        valid = username_valid & password_valid
        if not valid:
            self.sessions.record_failure(client)
            self._send(401, login_page("The username or password was not accepted.").encode(), "text/html; charset=utf-8")
            return
        token = self.sessions.create()
        if token is None:
            self._send(503, b"session capacity reached\n")
            return
        self.sessions.clear_failures(client)
        cookie = f"{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={self.sessions.ttl_seconds}"
        self._send(303, headers=(("Location", "/"), ("Set-Cookie", cookie)))

    def _logout(self):
        if not self._same_origin(require_evidence=True):
            self._send(403, b"forbidden\n")
            return
        self.sessions.revoke(self._session_token())
        cookie = f"{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"
        self._send(303, headers=(("Location", "/login"), ("Set-Cookie", cookie)))

    def _unauthenticated(self):
        if self.path.startswith("/api/"):
            body = json.dumps({"error": "unauthenticated"}).encode()
            self._send(401, body, "application/json", (("X-Sentinel-Login", "/login"),))
        else:
            self._send(303, headers=(("Location", "/login"),))

    def _require_session_and_origin(self):
        if not self.sessions.valid(self._session_token()):
            self._unauthenticated()
            return False
        if not self._same_origin(require_evidence=True):
            self._send(403, b"forbidden\n")
            return False
        return True

    def relay(self):
        length = int(self.headers.get("content-length") or 0)
        if length > MAX_BODY_BYTES and self.path in {"/login", "/logout"}:
            self._send(413, b"request too large\n")
            return
        body = self.rfile.read(length) if length else None
        headers = {key: value for key, value in self.headers.items() if key.lower() not in STRIP}
        headers["x-forwarded-for"] = self.client_address[0]
        headers["x-forwarded-proof"] = self.proof
        headers["x-authenticated-principal"] = self.principal
        upstream = http.client.HTTPConnection(*self.upstream, timeout=30)
        try:
            upstream.request(self.command, self.path, body=body, headers=headers)
            response = upstream.getresponse()
            self.send_response(response.status, response.reason)
            for key, value in response.getheaders():
                if key.lower() in {"transfer-encoding", "content-length", "connection"}:
                    continue
                self.send_header(key, value)
            self.send_header("connection", "close")
            self.end_headers()
            self.close_connection = True
            while True:
                chunk = response.read(65536)
                if not chunk:
                    break
                self.wfile.write(chunk)
                self.wfile.flush()
        except Exception:  # noqa: BLE001 - report a bounded error, never internals
            self.send_error(502, "upstream unavailable")
        finally:
            upstream.close()

    def do_GET(self):
        if self.path == "/login":
            if self.sessions.valid(self._session_token()):
                self._send(303, headers=(("Location", "/"),))
            else:
                self._send(200, login_page().encode(), "text/html; charset=utf-8")
            return
        if not self.sessions.valid(self._session_token()):
            self._unauthenticated()
            return
        self.relay()

    def do_POST(self):
        if self.path == "/login":
            self._login()
            return
        if self.path == "/logout":
            self._logout()
            return
        if not self._require_session_and_origin():
            return
        self.relay()

    def do_PUT(self):
        if not self._require_session_and_origin():
            return
        self.relay()

    do_DELETE = do_PATCH = do_PUT

    def do_HEAD(self):
        self.do_GET()


def configure_handler():
    listen_port = int(os.environ["LISTEN_PORT"])
    password = os.environ["DEV_PASSWORD"]
    if not password:
        raise ValueError("DEV_PASSWORD must not be empty")
    Proxy.upstream = ("127.0.0.1", int(os.environ["UPSTREAM_PORT"]))
    Proxy.proof = os.environ["PROOF_SECRET"]
    Proxy.principal = os.environ["PRINCIPAL"]
    Proxy.username = os.environ["DEV_USERNAME"]
    Proxy.password = password
    Proxy.origins = (
        f"http://127.0.0.1:{listen_port}",
        f"http://localhost:{listen_port}",
    )
    Proxy.allowed_hosts = (
        f"127.0.0.1:{listen_port}",
        f"localhost:{listen_port}",
    )
    Proxy.sessions = SessionStore(bounded_ttl(os.environ.get("DEV_SESSION_TTL_SECONDS", "28800")))
    return listen_port


def main():
    listen_port = configure_handler()
    server = ThreadingHTTPServer(("127.0.0.1", listen_port), Proxy)
    server.daemon_threads = True
    server.serve_forever()


if __name__ == "__main__":
    main()
