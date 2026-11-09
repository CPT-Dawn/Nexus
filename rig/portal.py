#!/usr/bin/env python3
"""The rig's connectivity-check server and captive portal (runs in the AP namespace).

    portal.py <portal-subnet-prefix>     e.g. portal.py 10.99.3.

NetworkManager checks http://10.99.99.1/check.txt from every network. Stations
outside the portal subnet get the expected answer. Stations on the portal
subnet are redirected to /login until they fetch /login (the "sign-in").
Standard library only.
"""
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORTAL_PREFIX = sys.argv[1]
ONLINE = b"NEXUS-ONLINE\n"  # keep in step with `response=` in rig/inner.sh
signed_in = set()


class Handler(BaseHTTPRequestHandler):
    def reply(self, code, body=b"", headers=()):
        self.send_response(code)
        for k, v in headers:
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        ip = self.client_address[0]
        if self.path.startswith("/login"):
            signed_in.add(ip)
            self.reply(200, b"signed in\n", [("Content-Type", "text/plain")])
        elif ip.startswith(PORTAL_PREFIX) and ip not in signed_in:
            self.reply(302, b"", [("Location", "http://10.99.99.1/login")])
        else:
            self.reply(200, ONLINE, [("Content-Type", "text/plain")])

    def log_message(self, fmt, *args):
        sys.stderr.write("portal: %s %s\n" % (self.client_address[0], fmt % args))


ThreadingHTTPServer(("0.0.0.0", 80), Handler).serve_forever()
