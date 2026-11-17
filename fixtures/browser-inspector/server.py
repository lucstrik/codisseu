#!/usr/bin/env python3
"""Owned loopback-only HTTP fixture for native inspector acceptance tests.

No external service, credentials or dependencies. The first stdout line is JSON
with the chosen origin. /state exposes only this disposable fixture's requests.
"""
import argparse
import json
from pathlib import Path
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--journal", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    requests = []
    lock = threading.Lock()

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_):
            pass

        def reply(self, status, body, mime="application/json; charset=utf-8", headers=()):
            self.send_response(status)
            self.send_header("Content-Type", mime)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.send_header("Cache-Control", "no-store")
            for name, value in headers:
                self.send_header(name, value)
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            path = urlsplit(self.path).path
            if path == "/state":
                with lock:
                    body = json.dumps({"requests": requests}, ensure_ascii=False).encode()
                self.reply(200, body)
                return
            names = {"/": "index.html", "/index.html": "index.html", "/app.js": "app.js", "/style.css": "style.css"}
            name = names.get(path)
            if not name:
                self.reply(404, b'{"error":"fixture route not found"}')
                return
            mime = {"index.html": "text/html; charset=utf-8", "app.js": "text/javascript; charset=utf-8", "style.css": "text/css; charset=utf-8"}[name]
            headers = ()
            if name == "index.html":
                headers = (
                    ("Set-Cookie", "fixture_http=fixture-cookie-secret-42; HttpOnly; SameSite=Strict; Path=/"),
                    ("Set-Cookie", "fixture_edit=initial-42; SameSite=Lax; Path=/"),
                )
            self.reply(200, (root / name).read_bytes(), mime, headers)

        def do_POST(self):
            if urlsplit(self.path).path != "/api/echo":
                self.reply(404, b'{"error":"fixture route not found"}')
                return
            try:
                size = int(self.headers.get("Content-Length", "0"))
                if size <= 0 or size > 65536:
                    raise ValueError("bounded request body required")
                body = self.rfile.read(size).decode("utf-8")
                value = json.loads(body)
                left, right = value["left"], value["right"]
                if type(left) is not int or type(right) is not int:
                    raise ValueError("integer operands required")
            except (ValueError, KeyError, UnicodeError):
                self.reply(400, b'{"error":"invalid fixture JSON"}')
                return
            record = {"method": "POST", "path": self.path, "headers": dict(self.headers), "body": body, "received_at": time.time()}
            with lock:
                requests.append(record)
                if args.journal:
                    with args.journal.open("a", encoding="utf-8") as journal:
                        journal.write(json.dumps(record, ensure_ascii=False) + "\n")
            # A real bounded delay makes Network timing observable without any
            # fabricated CDP durations. The tests do not assert exact wall time.
            time.sleep(0.06)
            reply = {"answer": left + right, "message": value.get("message"), "authorization_received": self.headers.get("Authorization") == "Bearer fixture-bearer-42"}
            self.reply(201, json.dumps(reply, ensure_ascii=False).encode(), headers=(("X-Fixture-Response", "response42"),))

    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.daemon_threads = True
    print(json.dumps({"origin": f"http://127.0.0.1:{server.server_port}"}), flush=True)
    server.serve_forever(poll_interval=0.05)


if __name__ == "__main__":
    main()
