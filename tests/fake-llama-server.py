#!/usr/bin/env python3
"""Stands in for llama-server: answers /health and streams a chat reply.

Extra flags for tests: --die (exit at once), --hang (never become healthy).
"""
import http.server
import json
import sys
import time

args = sys.argv[1:]
if "--die" in args:
    print("fake error: could not load model", flush=True)
    sys.exit(1)
if "--hang" in args:
    time.sleep(60)
    sys.exit(0)


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        self.send_response(200 if self.path == "/health" else 404)
        self.end_headers()

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        deltas = [{"reasoning_content": "hmm"}, {"content": "turns="}, {"content": str(len(body["messages"]))}]
        for delta in deltas:
            self.wfile.write(f"data: {json.dumps({'choices': [{'delta': delta}]})}\n\n".encode())
        self.wfile.write(b"data: [DONE]\n\n")


port = int(args[len(args) - 1 - args[::-1].index("--port") + 1])
http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
