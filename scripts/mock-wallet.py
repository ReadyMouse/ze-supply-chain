#!/usr/bin/env python3
# Mock Wallet Service
#
#   Stand-in HTTP server mimicking wallet-service for UI dev without a Zcash node.
#   Returns fake addresses and txids; nothing touches the chain.
#
# INPUT:
#   - HTTP requests on /enroll, /submit, /process-batch, /status, etc.
#   - Optional port arg (default 7001)
#
# OUTPUT:
#   - JSON responses matching wallet-service surface area
#
# NOTES:
#   Addresses prefixed u1mock. Use when gateway/frontend need end-to-end flow only.
#
# Written by Composer for Ze Supply Chain. June 2025. All rights reserved.

"""Mock wallet-service for UI development / demo rehearsal without a node.

Implements just enough of the wallet-service HTTP surface that the gateway and
frontend work end to end: enroll returns a fake (clearly labelled) address,
submissions are accepted, process-batch returns fake txids, status returns
plausible numbers. NO transactions are constructed and nothing touches a chain.

Usage: python3 scripts/mock-wallet.py [port]   (default 7001)
"""

import json
import random
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

PENDING = []  # submission_ids queued since the last process-batch
FAKE_TIP = 3_100_000


def fake_address(user_index: int) -> str:
    rand = "".join(random.choice("acdefghjklmnpqrstuvwxyz023456789") for _ in range(60))
    return f"u1mock{user_index:04d}{rand}"


def fake_txid() -> str:
    return "".join(random.choice("0123456789abcdef") for _ in range(64))


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        print(f"[mock-wallet] {fmt % args}")

    def _json(self, code: int, body: dict):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _read(self) -> dict:
        length = int(self.headers.get("Content-Length", 0))
        return json.loads(self.rfile.read(length)) if length else {}

    def do_GET(self):
        global FAKE_TIP
        if self.path == "/status":
            FAKE_TIP += random.choice([0, 0, 1])  # creep forward like a chain
            self._json(200, {
                "chain_tip": FAKE_TIP,
                "scanned_height": FAKE_TIP - random.choice([0, 1]),
                "balance_zat": 5_000_000,
                "spendable_zat": 4_800_000,
                "queued_records": len(PENDING),
                "org_address": fake_address(0),
            })
        else:
            self._json(404, {"error": "not found"})

    def do_POST(self):
        body = self._read()
        if self.path == "/enroll":
            PENDING.append(body.get("submission_id"))
            self._json(200, {"address": fake_address(body.get("user_index", 0))})
        elif self.path == "/submit":
            PENDING.append(body.get("submission_id"))
            self._json(202, {})
        elif self.path == "/process-batch":
            txid = fake_txid()
            broadcast = [{"submission_id": s, "txid": txid} for s in PENDING]
            PENDING.clear()
            self._json(200, {"broadcast": broadcast})
        elif self.path == "/split-notes":
            self._json(200, {
                "txid": fake_txid(),
                "parts": body.get("parts", 10),
                "zat_per_part": body.get("zat_per_part", 200_000),
            })
        elif self.path == "/rebuild":
            self._json(200, {"rebuilt_records": 0})
        else:
            self._json(404, {"error": "not found"})


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 7001
    print(f"[mock-wallet] listening on 127.0.0.1:{port} — FAKE addresses, FAKE txids, no chain")
    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
