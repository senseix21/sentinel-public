"""Focused regression tests for the loopback development login proxy."""

import http.client
import importlib.util
import pathlib
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlencode


MODULE_PATH = pathlib.Path(__file__).with_name("trust-boundary-proxy.py")
SPEC = importlib.util.spec_from_file_location("sentinel_dev_proxy", MODULE_PATH)
PROXY_MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROXY_MODULE)


class Upstream(BaseHTTPRequestHandler):
    seen_headers = None

    def log_message(self, _fmt, *_args):
        return

    def do_GET(self):
        type(self).seen_headers = {key.lower(): value for key, value in self.headers.items()}
        body = b'{"ok":true}'
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class DevelopmentProxyTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.upstream = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
        cls.upstream_thread = threading.Thread(target=cls.upstream.serve_forever, daemon=True)
        cls.upstream_thread.start()

        handler = PROXY_MODULE.Proxy
        handler.upstream = cls.upstream.server_address
        handler.proof = "test-proof"
        handler.principal = "operator"
        handler.username = "operator"
        handler.password = "correct-password"
        handler.sessions = PROXY_MODULE.SessionStore(300)
        cls.proxy = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        cls.origin = "http://127.0.0.1:%d" % cls.proxy.server_address[1]
        cls.localhost_origin = "http://localhost:%d" % cls.proxy.server_address[1]
        handler.origins = (cls.origin, cls.localhost_origin)
        handler.allowed_hosts = (
            "127.0.0.1:%d" % cls.proxy.server_address[1],
            "localhost:%d" % cls.proxy.server_address[1],
        )
        cls.proxy_thread = threading.Thread(target=cls.proxy.serve_forever, daemon=True)
        cls.proxy_thread.start()

    @classmethod
    def tearDownClass(cls):
        cls.proxy.shutdown()
        cls.proxy.server_close()
        cls.upstream.shutdown()
        cls.upstream.server_close()

    def request(self, method, path, body=None, headers=None):
        connection = http.client.HTTPConnection(*self.proxy.server_address, timeout=3)
        connection.request(method, path, body=body, headers=headers or {})
        response = connection.getresponse()
        payload = response.read()
        result = response.status, dict(response.getheaders()), payload
        connection.close()
        return result

    def login(self):
        body = urlencode({"username": "operator", "password": "correct-password"})
        status, headers, _ = self.request(
            "POST",
            "/login",
            body,
            {"Content-Type": "application/x-www-form-urlencoded", "Origin": self.origin},
        )
        self.assertEqual(status, 303)
        return headers["Set-Cookie"].split(";", 1)[0]

    def test_browser_is_redirected_to_login_without_session(self):
        status, headers, _ = self.request("GET", "/")
        self.assertEqual(status, 303)
        self.assertEqual(headers["Location"], "/login")

    def test_api_gets_json_401_without_session(self):
        status, _, body = self.request("GET", "/api/cameras")
        self.assertEqual(status, 401)
        self.assertEqual(body, b'{"error": "unauthenticated"}')

    def test_login_requires_same_origin(self):
        body = urlencode({"username": "operator", "password": "correct-password"})
        status, _, _ = self.request(
            "POST",
            "/login",
            body,
            {"Content-Type": "application/x-www-form-urlencoded", "Origin": "http://example.test"},
        )
        self.assertEqual(status, 403)

    def test_localhost_is_an_allowed_loopback_origin(self):
        body = urlencode({"username": "operator", "password": "correct-password"})
        status, headers, _ = self.request(
            "POST",
            "/login",
            body,
            {
                "Content-Type": "application/x-www-form-urlencoded",
                "Origin": self.localhost_origin,
            },
        )
        self.assertEqual(status, 303)
        self.assertIn("Set-Cookie", headers)

    def test_login_accepts_a_browser_that_sends_neither_origin_nor_referer(self):
        body = urlencode({"username": "operator", "password": "correct-password"})
        status, headers, _ = self.request(
            "POST",
            "/login",
            body,
            {"Content-Type": "application/x-www-form-urlencoded"},
        )
        self.assertEqual(status, 303)
        self.assertIn("Set-Cookie", headers)

    def test_login_accepts_privacy_origin_null_only_on_an_allowed_host(self):
        body = urlencode({"username": "operator", "password": "correct-password"})
        status, headers, _ = self.request(
            "POST",
            "/login",
            body,
            {
                "Content-Type": "application/x-www-form-urlencoded",
                "Origin": "null",
            },
        )
        self.assertEqual(status, 303)
        self.assertIn("Set-Cookie", headers)

    def test_wrong_password_does_not_create_session(self):
        body = urlencode({"username": "operator", "password": "wrong"})
        status, headers, _ = self.request(
            "POST",
            "/login",
            body,
            {"Content-Type": "application/x-www-form-urlencoded", "Origin": self.origin},
        )
        self.assertEqual(status, 401)
        self.assertNotIn("Set-Cookie", headers)

    def test_session_asserts_identity_and_strips_client_assertions(self):
        cookie = self.login()
        status, _, body = self.request(
            "GET",
            "/api/cameras",
            headers={
                "Cookie": cookie,
                "X-Authenticated-Principal": "attacker",
                "X-Forwarded-Proof": "attacker-proof",
            },
        )
        self.assertEqual(status, 200)
        self.assertEqual(body, b'{"ok":true}')
        self.assertEqual(Upstream.seen_headers["x-authenticated-principal"], "operator")
        self.assertEqual(Upstream.seen_headers["x-forwarded-proof"], "test-proof")
        self.assertNotIn("cookie", Upstream.seen_headers)

    def test_logout_revokes_session(self):
        cookie = self.login()
        status, _, _ = self.request(
            "POST", "/logout", headers={"Cookie": cookie, "Origin": self.origin}
        )
        self.assertEqual(status, 303)
        status, _, _ = self.request("GET", "/api/cameras", headers={"Cookie": cookie})
        self.assertEqual(status, 401)

    def test_authenticated_mutation_requires_same_origin(self):
        cookie = self.login()
        status, _, _ = self.request(
            "POST", "/api/cameras", body=b"{}", headers={"Cookie": cookie}
        )
        self.assertEqual(status, 403)


if __name__ == "__main__":
    unittest.main()
