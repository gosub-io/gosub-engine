#!/usr/bin/env python3
"""Serves this directory and echoes form submissions on /echo (GET query or POST body)."""
import http.server, urllib.parse, html, sys
class H(http.server.SimpleHTTPRequestHandler):
    def _echo(self, method, pairs):
        rows = "".join(f"<tr><td>{html.escape(k)}</td><td>{html.escape(v)}</td></tr>" for k, v in pairs)
        body = f"""<!DOCTYPE html><html><head><style>
body{{font-family:DejaVu Sans,sans-serif;margin:24px}} h1{{font-size:20px}}
table{{border-collapse:collapse}} td{{border:1px solid #999;padding:4px 10px}}</style></head>
<body><h1>Echo: {method} {html.escape(self.path.split('?')[0])}</h1>
<table>{rows}</table></body></html>"""
        data = body.encode()
        self.send_response(200); self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)
    def do_GET(self):
        if self.path.startswith("/echo"):
            q = urllib.parse.urlsplit(self.path).query
            return self._echo("GET", urllib.parse.parse_qsl(q, keep_blank_values=True))
        return super().do_GET()
    def do_POST(self):
        n = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(n).decode()
        ct = self.headers.get("Content-Type", "")
        pairs = urllib.parse.parse_qsl(raw, keep_blank_values=True) if "urlencoded" in ct else [("(raw)", raw), ("(ct)", ct)]
        return self._echo("POST", pairs)
    def log_message(self, *a): pass
import os
os.chdir(os.path.dirname(os.path.abspath(__file__)))
port = int(sys.argv[1]) if len(sys.argv) > 1 else 8080
print(f"serving {os.getcwd()} on http://127.0.0.1:{port}/index.html  (echo at /echo)")
http.server.ThreadingHTTPServer(("127.0.0.1", port), H).serve_forever()
