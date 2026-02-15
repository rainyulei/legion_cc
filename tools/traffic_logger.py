#!/usr/bin/env python3
"""
Traffic logger proxy: sits between client and OpenCode Zen API.
Logs all request/response details, then forwards to real API.

Usage:
  python3 tools/traffic_logger.py

Then point requests to http://127.0.0.1:48080 instead of https://opencode.ai/zen/v1
"""

import http.server
import json
import ssl
import sys
import urllib.request
import urllib.error
from datetime import datetime

LISTEN_PORT = 48080
UPSTREAM = "https://opencode.ai/zen/v1"

class LoggingHandler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        ts = datetime.now().strftime("%H:%M:%S.%f")[:-3]
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length) if content_length > 0 else b""

        # Log request
        print(f"\n{'='*80}")
        print(f"[{ts}] >>> REQUEST: POST {self.path}")
        print(f"--- Headers ---")
        for k, v in self.headers.items():
            # Mask API keys
            if "key" in k.lower() or "authorization" in k.lower():
                v = v[:20] + "..." if len(v) > 20 else v
            print(f"  {k}: {v}")

        if body:
            try:
                parsed = json.loads(body)
                print(f"--- Body (JSON) ---")
                print(json.dumps(parsed, indent=2, ensure_ascii=False)[:2000])
            except:
                print(f"--- Body (raw, {len(body)} bytes) ---")
                print(body[:500].decode("utf-8", errors="replace"))

        # Forward to upstream
        target_url = f"{UPSTREAM}{self.path}"
        print(f"\n[{ts}] -> Forwarding to: {target_url}")

        req = urllib.request.Request(target_url, data=body, method="POST")
        for k, v in self.headers.items():
            if k.lower() not in ("host", "content-length", "transfer-encoding"):
                req.add_header(k, v)

        try:
            ctx = ssl.create_default_context()
            resp = urllib.request.urlopen(req, context=ctx, timeout=30)
            status = resp.status
            resp_headers = dict(resp.headers)
            resp_body = resp.read()
        except urllib.error.HTTPError as e:
            status = e.code
            resp_headers = dict(e.headers)
            resp_body = e.read()
        except Exception as e:
            print(f"[{ts}] !!! UPSTREAM ERROR: {e}")
            self.send_response(502)
            self.send_header("Content-Type", "text/plain")
            self.end_headers()
            self.wfile.write(str(e).encode())
            return

        # Log response
        print(f"\n[{ts}] <<< RESPONSE: {status}")
        print(f"--- Response Headers ---")
        for k, v in resp_headers.items():
            print(f"  {k}: {v}")
        print(f"--- Response Body ({len(resp_body)} bytes) ---")
        try:
            print(resp_body[:3000].decode("utf-8", errors="replace"))
        except:
            print(f"(binary, {len(resp_body)} bytes)")
        print(f"{'='*80}\n")

        # Send response back to client
        self.send_response(status)
        for k, v in resp_headers.items():
            if k.lower() not in ("transfer-encoding",):
                self.send_header(k, v)
        self.end_headers()
        self.wfile.write(resp_body)

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.end_headers()
        self.wfile.write(b"Traffic logger proxy running\n")

    def log_message(self, format, *args):
        pass  # Suppress default access log

if __name__ == "__main__":
    print(f"Traffic logger proxy starting on http://127.0.0.1:{LISTEN_PORT}")
    print(f"Forwarding to: {UPSTREAM}")
    print(f"Configure OpenCode provider API to: http://127.0.0.1:{LISTEN_PORT}")
    print()
    server = http.server.HTTPServer(("127.0.0.1", LISTEN_PORT), LoggingHandler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nStopped.")
