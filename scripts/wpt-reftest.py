#!/usr/bin/env python3
"""WPT reftest runner for the gosub engine.

Discovers reftests (<link rel="match"/"mismatch">) under a WPT subtree,
renders test and reference pages with gosub-screenshot (Cairo backend), and
pixel-compares them on a WPT-style 800x600 canvas.

Usage:
  scripts/wpt-reftest.py --wpt-root ~/code/gosub/wpt css/CSS2/tables
  scripts/wpt-reftest.py --wpt-root ~/code/gosub/wpt css/CSS2/tables/table-anonymous-objects-001.xht

Outputs a summary, a JSON results file, and (with --report) an HTML report of
failures with test/ref/diff images side by side.

Requirements: the Ahem font must be installed (fc-match Ahem), and
target/release/gosub-screenshot must be built with the Cairo backend:
  cargo build --release -p gosub-screenshot --no-default-features --features backend_cairo
"""

import argparse
import base64
import concurrent.futures
import functools
import html
import http.server
import io
import json
import os
import re
import socket
import subprocess
import sys
import threading
from pathlib import Path

from PIL import Image, ImageChops

VIEWPORT_W = 800
VIEWPORT_H = 600

LINK_RE = re.compile(
    r'<link[^>]+rel=["\']?(match|mismatch)["\']?[^>]*>', re.I)
HREF_RE = re.compile(r'href=["\']?([^"\'\s>]+)', re.I)
FUZZY_RE = re.compile(
    r'<meta[^>]+name=["\']?fuzzy["\']?[^>]+content=["\']([^"\']+)["\']', re.I)
WAIT_RE = re.compile(r'class=["\'][^"\']*reftest-wait', re.I)
SCRIPT_RE = re.compile(r'<script\b', re.I)


def parse_fuzzy(content):
    """Parse a WPT fuzzy annotation into (max_channel_diff, max_pixel_count).

    Content looks like "maxDifference=0-2;totalPixels=0-300", optionally
    prefixed with "ref-name.html:". We take the upper bounds and, with
    multiple annotations, the loosest.
    """
    spec = content.split(":", 1)[-1] if ("=" in content.split(":", 1)[-1]) else content
    max_diff = pixels = 0
    for part in spec.split(";"):
        part = part.strip()
        m = re.match(r'(maxDifference|totalPixels)\s*=\s*(?:\d+-)?(\d+)', part)
        if not m:
            continue
        if m.group(1) == "maxDifference":
            max_diff = int(m.group(2))
        else:
            pixels = int(m.group(2))
    return max_diff, pixels


def discover(root, target):
    """Yield (test_path, [(rel, ref_path)], fuzzy, skip_reason) tuples."""
    target_path = root / target
    files = [target_path] if target_path.is_file() else sorted(
        p for p in target_path.rglob("*")
        if p.suffix in (".html", ".htm", ".xht", ".xhtml")
    )
    for f in files:
        try:
            text = f.read_text(errors="replace")
        except OSError:
            continue
        refs = []
        for link in LINK_RE.finditer(text):
            href = HREF_RE.search(link.group(0))
            if href:
                rel = link.group(1).lower()
                ref = (f.parent / href.group(1)).resolve()
                refs.append((rel, ref))
        if not refs:
            continue
        skip = None
        if WAIT_RE.search(text):
            skip = "reftest-wait (needs script support)"
        elif SCRIPT_RE.search(text):
            skip = "uses <script>"
        fuzzy = (0, 0)
        fm = FUZZY_RE.search(text)
        if fm:
            fuzzy = parse_fuzzy(fm.group(1))
        yield f, refs, fuzzy, skip


class WptHandler(http.server.SimpleHTTPRequestHandler):
    """Serves the WPT tree; XHTML goes out as text/html for the engine.

    The engine has no XML parser, so XHTML gets the HTML treatment. That
    breaks `<![CDATA[ ... ]]>`-wrapped stylesheets (the markers become
    stylesheet text and poison rule parsing), so those markers are stripped
    on the way out.
    """
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".xht": "text/html",
        ".xhtml": "text/html",
        ".htm": "text/html",
    }

    def do_GET(self):
        path = Path(self.translate_path(self.path))
        if path.suffix not in (".xht", ".xhtml", ".html", ".htm") or not path.is_file():
            return super().do_GET()
        data = path.read_bytes().replace(b"<![CDATA[", b"").replace(b"]]>", b"")
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


def start_server(root):
    handler = functools.partial(WptHandler, directory=str(root))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, server.server_address[1]


def to_canvas(png_bytes):
    """Place a full-page render on a white WPT viewport canvas (800x600)."""
    img = Image.open(io.BytesIO(png_bytes)).convert("RGB")
    canvas = Image.new("RGB", (VIEWPORT_W, VIEWPORT_H), "white")
    canvas.paste(img.crop((0, 0, min(img.width, VIEWPORT_W),
                           min(img.height, VIEWPORT_H))), (0, 0))
    return canvas


def images_match(a, b, fuzzy):
    diff = ImageChops.difference(a, b)
    bbox = diff.getbbox()
    if bbox is None:
        return True, 0, 0
    max_diff, pixel_budget = fuzzy
    hist_max = max(ch.getextrema()[1] for ch in diff.split())
    differing = sum(1 for px in diff.getdata() if px != (0, 0, 0))
    ok = hist_max <= max_diff and differing <= pixel_budget
    return ok, hist_max, differing


class Renderer:
    def __init__(self, shot_bin, root, port, out_dir, jobs):
        self.shot_bin, self.root, self.port = shot_bin, root, port
        self.out_dir = out_dir
        self.cache = {}
        self.lock = threading.Lock()
        self.sem = threading.Semaphore(jobs)

    def render(self, page: Path):
        """Render one page, cached by path. Returns PNG bytes or raises."""
        key = str(page)
        with self.lock:
            fut = self.cache.get(key)
            if fut is None:
                fut = self.cache[key] = concurrent.futures.Future()
                owner = True
            else:
                owner = False
        if not owner:
            return fut.result()
        try:
            rel = page.relative_to(self.root).as_posix()
            out = self.out_dir / (rel.replace("/", "__") + ".png")
            url = f"http://127.0.0.1:{self.port}/{rel}"
            with self.sem:
                proc = subprocess.run(
                    [self.shot_bin, url, str(out), str(VIEWPORT_W),
                     "--nav-timeout", "20", "--render-timeout", "30"],
                    capture_output=True, timeout=90)
            if proc.returncode != 0 or not out.exists():
                raise RuntimeError(
                    f"render failed: {proc.stderr.decode(errors='replace')[-200:]}")
            data = out.read_bytes()
            fut.set_result(data)
            return data
        except BaseException as e:
            fut.set_exception(e)
            raise


def run_test(renderer, test, refs, fuzzy):
    try:
        test_img = to_canvas(renderer.render(test))
    except Exception as e:
        return "ERROR", f"test render: {e}", None

    match_results, detail = [], []
    for rel, ref in refs:
        if not ref.exists():
            return "ERROR", f"missing ref {ref.name}", None
        try:
            ref_img = to_canvas(renderer.render(ref))
        except Exception as e:
            return "ERROR", f"ref render: {e}", None
        equal, max_d, n_px = images_match(test_img, ref_img, fuzzy)
        detail.append(f"{rel} {ref.name}: maxdiff={max_d} pixels={n_px}")
        if rel == "match":
            match_results.append(equal)
        elif equal:  # mismatch ref that matched
            return "FAIL", "; ".join(detail), refs[0][1]
    # Multiple match refs are alternates: any one passing suffices.
    if match_results and not any(match_results):
        return "FAIL", "; ".join(detail), refs[0][1]
    return "PASS", "; ".join(detail), None


def chrome_shot(url, chrome_bin="chromium"):
    """Render `url` with headless Chromium (writes inside $HOME: snap)."""
    tmp = Path.home() / f"gosub-wpt-chrome-{os.getpid()}.png"
    try:
        subprocess.run(
            [chrome_bin, "--headless", "--disable-gpu", "--hide-scrollbars",
             "--force-device-scale-factor=1",
             f"--window-size={VIEWPORT_W},{VIEWPORT_H}",
             f"--screenshot={tmp}", url],
            capture_output=True, timeout=60)
        return tmp.read_bytes()
    finally:
        tmp.unlink(missing_ok=True)


def write_report(path, failures, renderer, with_chrome=False):
    rows = []
    for test, refs, note in failures:
        try:
            t64 = base64.b64encode(renderer.render(test)).decode()
            r64 = base64.b64encode(renderer.render(refs[0][1])).decode()
        except Exception:
            continue
        chrome_fig = ""
        if with_chrome:
            try:
                rel = test.relative_to(renderer.root).as_posix()
                c64 = base64.b64encode(
                    chrome_shot(f"http://127.0.0.1:{renderer.port}/{rel}")).decode()
                chrome_fig = (f'<figure><figcaption>chrome (test)</figcaption>'
                              f'<img src="data:image/png;base64,{c64}"></figure>')
            except Exception:
                pass
        rows.append(f"""
<section><h2>{html.escape(str(test.relative_to(renderer.root)))}</h2>
<p>{html.escape(note)}</p>
<div class="pair">
<figure><figcaption>gosub (test)</figcaption><img src="data:image/png;base64,{t64}"></figure>
<figure><figcaption>gosub (ref, {html.escape(refs[0][0])})</figcaption><img src="data:image/png;base64,{r64}"></figure>
{chrome_fig}
</div></section>""")
    path.write_text(f"""<!doctype html><meta charset="utf-8">
<title>gosub WPT reftest failures</title>
<style>body{{font:14px sans-serif;margin:1rem;background:#f4f4f4;color:#222}}
.pair{{display:flex;gap:8px}} figure{{flex:1;margin:0;min-width:0}}
img{{width:100%;border:1px solid #ccc;background:#fff}}
figcaption{{font-size:.75rem;color:#555}}
h2{{font-size:1rem;margin:1.5rem 0 .3rem;font-family:monospace}}
p{{font-size:.8rem;color:#666;margin:.2rem 0}}</style>
<h1 style="font-size:1.2rem">{len(rows)} failures shown</h1>{''.join(rows)}""")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("target", help="path under the WPT root, dir or file")
    ap.add_argument("--wpt-root", default=os.path.expanduser("~/code/gosub/wpt"))
    ap.add_argument("--shot-bin", default="target/release/gosub-screenshot")
    ap.add_argument("--jobs", type=int, default=max(2, (os.cpu_count() or 4) // 2))
    ap.add_argument("--out", default="target/wpt-reftest")
    ap.add_argument("--report", action="store_true",
                    help="write failures.html with side-by-side images")
    ap.add_argument("--max-report", type=int, default=80)
    ap.add_argument("--chrome", action="store_true",
                    help="add a headless-Chromium render of each failing test to the report")
    ap.add_argument("--filter", default="",
                    help="only run tests whose filename contains this substring")
    args = ap.parse_args()

    root = Path(args.wpt_root).resolve()
    out_dir = Path(args.out)
    (out_dir / "shots").mkdir(parents=True, exist_ok=True)

    if subprocess.run(["fc-match", "Ahem"], capture_output=True,
                      text=True).stdout.split(":")[0] != "Ahem.ttf":
        print("warning: Ahem font not resolved - expect bogus failures", file=sys.stderr)

    tests = [t for t in discover(root, args.target) if args.filter in t[0].name]
    if not tests:
        sys.exit(f"no reftests found under {args.target}")

    server, port = start_server(root)
    renderer = Renderer(args.shot_bin, root, port, out_dir / "shots", args.jobs)

    results, failures = {}, []
    done = 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futs = {}
        for test, refs, fuzzy, skip in tests:
            name = str(test.relative_to(root))
            if skip:
                results[name] = {"status": "SKIP", "note": skip}
                continue
            futs[pool.submit(run_test, renderer, test, refs, fuzzy)] = (test, refs, name)
        for fut in concurrent.futures.as_completed(futs):
            test, refs, name = futs[fut]
            status, note, _ = fut.result()
            results[name] = {"status": status, "note": note}
            if status == "FAIL":
                failures.append((test, refs, note))
            done += 1
            if done % 25 == 0:
                print(f"  ...{done}/{len(futs)}", file=sys.stderr)

    counts = {}
    for r in results.values():
        counts[r["status"]] = counts.get(r["status"], 0) + 1
    (out_dir / "results.json").write_text(json.dumps(results, indent=1, sort_keys=True))

    total_run = counts.get("PASS", 0) + counts.get("FAIL", 0) + counts.get("ERROR", 0)
    print(f"\n{args.target}: {counts.get('PASS', 0)}/{total_run} pass "
          f"({counts.get('FAIL', 0)} fail, {counts.get('ERROR', 0)} error, "
          f"{counts.get('SKIP', 0)} skipped)")
    print(f"results: {out_dir}/results.json")

    if args.report and failures:
        failures.sort(key=lambda f: str(f[0]))
        write_report(out_dir / "failures.html", failures[:args.max_report],
                     renderer, with_chrome=args.chrome)
        print(f"report:  {out_dir}/failures.html")

    server.shutdown()


if __name__ == "__main__":
    main()
