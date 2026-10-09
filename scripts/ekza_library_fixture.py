#!/usr/bin/env python3
"""Loopback-only synthetic Registry protocol for the class-workshop demo.

Never connects to a real Registry, account or asset. Consent requires clicking the
clearly labelled local page. Restarting forgets every synthetic device and token.
"""
import argparse
import datetime
import html
import json
import secrets
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlsplit


def timestamp(seconds):
    return datetime.datetime.fromtimestamp(seconds, datetime.timezone.utc).isoformat()


class Fixture(BaseHTTPRequestHandler):
    devices = {}
    tokens = {}

    def log_message(self, *_):
        pass  # Never log codes, grants, paths or Authorization headers.

    def reply(self, status, value, content_type="application/json"):
        data = json.dumps(value).encode() if content_type == "application/json" else value.encode()
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'")
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        url = urlsplit(self.path)
        if url.path == "/v1/account/library":
            token = self.headers.get("Authorization", "").removeprefix("Bearer ")
            if self.tokens.get(token, 0) <= time.time():
                return self.reply(401, {"code": "link_invalid"})
            return self.reply(200, {"schema": "ekza.account.library.v1", "projectId": "omoba", "account": {"username": "SyntheticWorkshopArtist"}, "expiresAt": timestamp(self.tokens[token]), "items": []})
        if url.path != "/studio":
            return self.reply(404, {"code": "not_found"})
        code = parse_qs(url.query).get("code", [""])[0]
        device = next((d for d in self.devices.values() if d["code"] == code and d["until"] > time.time()), None)
        if not device:
            return self.reply(410, "This synthetic request expired.", "text/html; charset=utf-8")
        safe = html.escape(code, quote=True)
        page = f"""<!doctype html><html lang="en"><meta charset="utf-8"><title>Local Ekza protocol fixture</title><style>body{{background:#101626;color:#edf2fa;font:18px system-ui;max-width:680px;margin:80px auto;padding:24px}}button{{font:inherit;padding:16px;background:#95e2d1;border:0;border-radius:8px}}code{{color:#95e2d1}}p{{line-height:1.6}}</style><h1>Local Ekza protocol fixture</h1><p><strong>Synthetic demo account · no hosted Ekza account is involved.</strong></p><p>OMOBA requests read-only library consent for <code>SyntheticWorkshopArtist</code>. This does not sign you into OMOBA, publish assets or grant game permissions.</p><p>Connection code: <code>{safe}</code></p><form method="post" action="/approve"><input type="hidden" name="code" value="{safe}"><button>Approve synthetic library connection</button></form></html>"""
        self.reply(200, page, "text/html; charset=utf-8")

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        if not 0 < length <= 4096:
            return self.reply(400, {"code": "invalid_request"})
        raw = self.rfile.read(length)
        if self.path == "/approve":
            # Browser consent is same-origin; absent/foreign origins cannot approve.
            if self.headers.get("Origin") != self.server.origin:
                return self.reply(403, {"code": "origin_rejected"})
            code = parse_qs(raw.decode()).get("code", [""])[0]
            device = next((d for d in self.devices.values() if d["code"] == code and d["until"] > time.time()), None)
            if not device:
                return self.reply(410, {"code": "link_expired"})
            device["approved"] = True
            return self.reply(200, "<!doctype html><html lang=en><title>Synthetic consent approved</title><h1>Synthetic consent approved</h1><p>Return to OMOBA Workshop and press Check connection. No hosted account changed.</p></html>", "text/html; charset=utf-8")
        try:
            body = json.loads(raw)
        except (ValueError, UnicodeDecodeError):
            return self.reply(400, {"code": "invalid_request"})
        if self.path == "/v1/account/device" and body == {"projectId": "omoba", "scope": "library"}:
            code = "".join(secrets.choice("ABCDEFGHJKMNPQRSTUVWXYZ23456789") for _ in range(8))
            device = secrets.token_urlsafe(32)
            until = time.time() + 600
            self.devices[device] = {"code": code, "until": until, "approved": False}
            return self.reply(200, {"deviceCode": device, "userCode": code, "verificationUrl": f"{self.server.origin}/studio?view=connect&code={code}", "expiresAt": timestamp(until), "interval": 3})
        if self.path == "/v1/account/device/poll":
            device = self.devices.get(body.get("deviceCode"))
            if not device or device["until"] <= time.time():
                return self.reply(410, {"code": "link_expired"})
            if not device["approved"]:
                return self.reply(200, {"status": "pending"})
            token = secrets.token_urlsafe(48)
            until = time.time() + 86400
            self.tokens[token] = until
            del self.devices[body["deviceCode"]]
            return self.reply(200, {"status": "approved", "scope": "library", "accessToken": token, "expiresAt": timestamp(until), "projectId": "omoba", "account": {"username": "SyntheticWorkshopArtist"}})
        self.reply(400, {"code": "invalid_request"})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=40561)
    args = parser.parse_args()
    server = HTTPServer(("127.0.0.1", args.port), Fixture)
    server.origin = f"http://127.0.0.1:{server.server_port}"
    print(f"Synthetic local Registry fixture at {server.origin}; no hosted accounts", flush=True)
    server.serve_forever()
