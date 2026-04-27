#!/usr/bin/env python3
"""Tiny static server with PUT/DELETE/rename support for viz/ working dirs.

Extends the stdlib http.server so the viz UI can save run results, manage
selections, and rename directories. Writes are scoped to a small set of
allowed roots (``viz/results/``, ``viz/stackpath/``, ``viz/selections/``).
Intended to replace ``python3 -m http.server`` for local dev.
"""

import json
import shutil
import sys
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlparse

ROOT = Path(__file__).resolve().parent.parent
RESULTS = (ROOT / "viz" / "results").resolve()
STACKPATH = (ROOT / "viz" / "stackpath").resolve()
SELECTIONS = (ROOT / "viz" / "selections").resolve()
WRITE_ROOTS = (RESULTS, STACKPATH, SELECTIONS)
# Sibling repo containing rewrite rule files used by the interactive UI.
BABBLE = ROOT.parent / "babble"


def _resolve_under(rel: str, roots) -> Path | None:
    """Resolve ``rel`` under ROOT and return the path if it lies under any of ``roots``."""
    try:
        target = (ROOT / rel).resolve()
    except (OSError, ValueError):
        return None
    for r in roots:
        try:
            target.relative_to(r)
            return target
        except ValueError:
            continue
    return None


def _build_tree(root: Path) -> dict:
    """Return a JSON-friendly tree of ``root``: each node has name, path (relative
    to ROOT), children (sorted dirs), and is_run (true if a ``type.txt`` is present)."""
    rel = root.relative_to(ROOT).as_posix()
    children = []
    is_run = False
    try:
        entries = sorted(root.iterdir(), key=lambda p: p.name, reverse=True)
    except OSError:
        entries = []
    for e in entries:
        if e.is_dir():
            children.append(_build_tree(e))
        elif e.name == "type.txt":
            is_run = True
    return {"name": root.name, "path": rel, "children": children, "is_run": is_run}


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(ROOT), **kwargs)

    def end_headers(self):
        """Disable caching for pkg/ and viz/ assets during development."""
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def translate_path(self, path):
        """Serve /babble/... from the sibling babble repo."""
        clean = unquote(urlparse(path).path)
        if clean.startswith("/babble/"):
            return str(BABBLE / clean[len("/babble/"):])
        return super().translate_path(path)

    def _send_json(self, status: int, payload):
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _read_json(self) -> dict | None:
        length = int(self.headers.get("Content-Length", 0))
        try:
            return json.loads(self.rfile.read(length) or b"{}")
        except json.JSONDecodeError:
            return None

    def do_GET(self):
        """Handle JSON API endpoints; fall through to static serving otherwise."""
        url = urlparse(self.path)
        if url.path == "/api/stackpath-tree":
            STACKPATH.mkdir(parents=True, exist_ok=True)
            self._send_json(200, _build_tree(STACKPATH))
            return
        if url.path == "/api/selections":
            SELECTIONS.mkdir(parents=True, exist_ok=True)
            names = sorted(p.stem for p in SELECTIONS.glob("*.json"))
            self._send_json(200, {"names": names})
            return
        super().do_GET()

    def do_PUT(self):
        """Write a file under any allowed write-root. Creates parent dirs."""
        rel = unquote(urlparse(self.path).path.lstrip("/"))
        target = _resolve_under(rel, WRITE_ROOTS)
        if target is None:
            self.send_error(403, "path not under an allowed write-root")
            return
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        try:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(body)
        except OSError as e:
            self.send_error(500, f"write failed: {e}")
            return
        self.send_response(201)
        self.end_headers()

    def do_DELETE(self):
        """Delete a file or directory under any allowed write-root."""
        rel = unquote(urlparse(self.path).path.lstrip("/"))
        target = _resolve_under(rel, WRITE_ROOTS)
        if target is None:
            self.send_error(403, "path not under an allowed write-root")
            return
        if target in WRITE_ROOTS:
            self.send_error(403, "refusing to delete a write-root itself")
            return
        if not target.exists():
            self.send_error(404)
            return
        try:
            if target.is_dir():
                shutil.rmtree(target)
            else:
                target.unlink()
        except OSError as e:
            self.send_error(500, f"delete failed: {e}")
            return
        self.send_response(204)
        self.end_headers()

    def do_POST(self):
        """Rename endpoint: POST /api/rename with JSON ``{from, to}`` paths.

        Both endpoints must lie under the same write-root, the destination
        must not already exist, and the destination's parent must exist.
        """
        url = urlparse(self.path)
        if url.path != "/api/rename":
            self.send_error(404)
            return
        body = self._read_json()
        if not body or "from" not in body or "to" not in body:
            self.send_error(400, "expected JSON body with 'from' and 'to'")
            return
        src = _resolve_under(body["from"].lstrip("/"), WRITE_ROOTS)
        dst = _resolve_under(body["to"].lstrip("/"), WRITE_ROOTS)
        if src is None or dst is None:
            self.send_error(403, "paths must be under an allowed write-root")
            return
        if not src.exists():
            self.send_error(404, "source does not exist")
            return
        if dst.exists():
            self.send_error(409, "destination already exists")
            return
        if not dst.parent.exists():
            self.send_error(400, "destination parent does not exist")
            return
        try:
            src.rename(dst)
        except OSError as e:
            self.send_error(500, f"rename failed: {e}")
            return
        self.send_response(204)
        self.end_headers()


def main():
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8066
    print(f"serving on http://localhost:{port}/viz/", flush=True)
    ThreadingHTTPServer(("", port), Handler).serve_forever()


if __name__ == "__main__":
    main()
