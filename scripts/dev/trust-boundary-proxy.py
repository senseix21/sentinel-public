"""The reverse proxy the trust boundary expects, and nothing more.

Adds the three headers a real proxy would add after it has authenticated
someone: the forwarded client address, the shared-secret origin proof, and the
principal it authenticated. It authenticates nobody. That is the whole reason
this lives under `scripts/` and not in the repository's deployment story.

There is one copy of this on purpose. Both `scripts/phase9/authenticated-ui-
fixture.sh` and `scripts/dev/local-stack.sh` need it, and the thing that must
not drift between two copies is `STRIP`: forgetting one name there is not a
cosmetic bug but a hole, because a browser that can set the header the server
trusts can mint its own origin.

Environment: UPSTREAM_PORT, LISTEN_PORT, PROOF_SECRET, PRINCIPAL.
"""
import http.client
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

UPSTREAM = ("127.0.0.1", int(os.environ["UPSTREAM_PORT"]))
PROOF = os.environ["PROOF_SECRET"]
PRINCIPAL = os.environ["PRINCIPAL"]
# Hop-by-hop headers and anything we are about to assert ourselves. Forwarding
# a client-supplied x-forwarded-proof would let a browser mint its own origin.
STRIP = {
    "connection", "keep-alive", "proxy-authenticate", "proxy-authorization",
    "te", "trailer", "transfer-encoding", "upgrade", "host",
    "x-forwarded-for", "x-forwarded-proof", "x-authenticated-principal",
}


class Proxy(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))

    def relay(self):
        length = int(self.headers.get("content-length") or 0)
        body = self.rfile.read(length) if length else None
        headers = {
            key: value
            for key, value in self.headers.items()
            if key.lower() not in STRIP
        }
        headers["x-forwarded-for"] = self.client_address[0]
        headers["x-forwarded-proof"] = PROOF
        headers["x-authenticated-principal"] = PRINCIPAL
        upstream = http.client.HTTPConnection(*UPSTREAM, timeout=30)
        try:
            upstream.request(self.command, self.path, body=body, headers=headers)
            response = upstream.getresponse()
            # Relay the body as it arrives rather than reading it whole.
            #
            # The playback endpoint streams an assembled multi-minute MP4, and a
            # media element reads it slowly on purpose. Buffering it here and
            # re-framing it with a content-length made the browser stall and
            # rewind on a stream that plays straight through when the same bytes
            # are served directly -- an artefact of the proxy, which would then
            # have been mistaken for a defect in the endpoint. Close the
            # connection instead of guessing a length we do not know until the
            # last byte, which is after the headers have to go out.
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
        except Exception as error:  # noqa: BLE001 - a fixture reports and continues
            self.send_error(502, str(error))
        finally:
            upstream.close()

    do_GET = do_POST = do_PUT = do_DELETE = do_PATCH = do_HEAD = relay


ThreadingHTTPServer(("127.0.0.1", int(os.environ["LISTEN_PORT"])), Proxy).serve_forever()
