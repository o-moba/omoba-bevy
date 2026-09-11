#!/usr/bin/env python3
"""Real Omoba UDP server admission with a deterministic local passport fixture.

This is a transport/security integration test, not a Solana purchase or GPU
render proof. No wallets, public APIs, or production services are contacted.
"""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    support = {"projectId": "omoba", "platform": "desktop", "profile": "humanoid-glb-v1", "status": "approved",
               "rendition": {"id": "fixture-r1", "url": "https://example.test/model.glb", "sha256": "a" * 64, "sizeBytes": 20, "format": "glb"}}
    avatar_id = "solana:devnet:avatar-data:" + "1" * 32
    slug = "ekza-" + hashlib.sha256((avatar_id + "\n" + "a" * 64).encode()).hexdigest()
    consumed = set()
    requests = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            requests.append({"projectId": body.get("projectId"), "sessionId": body.get("sessionId")})
            ticket = body.get("ticket", "")
            valid = {"owner-proof-123456": "owner-session", "wrong-rendition-123456": "other-session"}
            status = 200
            if self.path != "/api/passport/ticket/consume" or body.get("projectId") != "omoba":
                status = 403
            elif ticket in consumed:
                status = 409
            elif valid.get(ticket) != body.get("sessionId"):
                status = 403
            consumed.add(ticket)
            rendition = json.loads(json.dumps(support))
            if ticket == "wrong-rendition-123456":
                rendition["rendition"]["sha256"] = "b" * 64
            value = {"wallet": "2" * 32, "avatarId": avatar_id, "mint": "3" * 32,
                     "expiresAt": "2099-01-01T00:00:00Z", "support": rendition} if status == 200 else {"error": "denied"}
            encoded = json.dumps(value).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

    fixture = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=fixture.serve_forever, daemon=True).start()
    reserve = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    reserve.bind(("127.0.0.1", 0))
    address = reserve.getsockname()
    reserve.close()
    clients = []
    checks = []

    def client():
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        sock.connect(address)
        sock.settimeout(0.15)
        clients.append(sock)
        return sock

    def send(sock, data):
        sock.send(json.dumps(data).encode())

    def wait(sock, predicate, seconds=4):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            for peer in clients:
                try:
                    send(peer, {"type": "ping"})
                except ConnectionRefusedError:
                    pass
            try:
                data = json.loads(sock.recv(65535))
            except (socket.timeout, ConnectionRefusedError):
                continue
            if predicate(data):
                return data
        raise AssertionError("Expected Omoba snapshot was not received")

    def join(sock, session, avatar, ticket=None):
        wait(sock, lambda data: data.get("type") == "snapshot")
        packet = {"type": "join", "team": "green", "hero_class": "mage", "avatar": avatar, "session_id": session}
        if ticket is not None:
            packet["passport_ticket"] = ticket
        send(sock, packet)

    with tempfile.TemporaryDirectory(prefix="omoba-passport-udp-") as temporary:
        manifest = Path(temporary) / "manifest.json"
        base = {"display_name": "Fixture", "collection": "Test", "license": "Test", "source_url": "https://example.test/model.glb"}
        manifest.write_text(json.dumps({"avatars": [dict(base, slug="free"), dict(base, slug=slug, passport={"avatarId": avatar_id, "support": support})]}))
        env = dict(os.environ, SERVER_ADDR=f"{address[0]}:{address[1]}", OMOBA_MATCH_MODE="dev",
                   OMOBA_AVATAR_MANIFEST=str(manifest), OMOBA_PASSPORT_URL=f"http://127.0.0.1:{fixture.server_port}/api/passport")
        with (args.output / "server.log").open("w") as log:
            server = subprocess.Popen([str(args.server.resolve())], env=env, stdout=log, stderr=subprocess.STDOUT)
            try:
                owner = client()
                join(owner, "owner-session", slug, "owner-proof-123456")
                snapshot = wait(owner, lambda data: any(p["id"] == data["your_id"] and p.get("avatar") == slug for p in data["players"]))
                owner_state = next(p for p in snapshot["players"] if p["id"] == snapshot["your_id"])
                assert owner_state["hero_class"] == "mage"
                checks.append("owner admitted with requested cosmetic and unchanged class")
                observer = client()
                join(observer, "observer-session", "free")
                wait(observer, lambda data: len(data["players"]) == 2 and any(p.get("avatar") == slug for p in data["players"]))
                checks.append("second client receives the exact protected slug")
                for session, ticket in [("no-proof", None), ("non-owner", "nonowner-proof-123456"),
                                        ("wrong-session", "owner-proof-123456"), ("other-session", "wrong-rendition-123456")]:
                    attacker = client()
                    join(attacker, session, slug, ticket)
                    wait(attacker, lambda data: data.get("join_error") == "avatar_not_authorized")
                    checks.append(session + " rejected")
                attacker = client()
                join(attacker, "padded-slug", " " + slug + " ")
                wait(attacker, lambda data: data.get("join_error") == "avatar_not_authorized")
                checks.append("padded paid slug without ticket rejected")
                # Force retained-session downgrade bypass while the paid owner is
                # still active; this must reject before ordinary reclaim logic.
                for retained in ("owner-session", " owner-session "):
                    attacker = client()
                    join(attacker, retained, "free")
                    wait(attacker, lambda data: data.get("join_error") == "avatar_not_authorized")
                    checks.append("free-slug retained paid session bypass rejected: " + repr(retained))
                assert all(request["projectId"] == "omoba" for request in requests)
            finally:
                server.terminate()
                server.wait(timeout=10)
                for sock in clients:
                    sock.close()
                fixture.shutdown()
    result = {"status": "PASS", "kind": "real UDP server + local HTTP fixture; no chain or GPU proof", "checks": checks}
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
