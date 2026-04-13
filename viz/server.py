#!/usr/bin/env python3
"""Tiny static server with PUT and DELETE support scoped to viz/results/*.

Extends the stdlib http.server so the viz UI can save run results and
remove individual run files and whole session folders in-place. Intended
to replace `python3 -m http.server` for local dev.
"""

import shutil
import sys
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parent.parent
RESULTS = (ROOT / "viz" / "results").resolve()
# Sibling repo containing rewrite rule files used by the interactive UI.
BABBLE = ROOT.parent / "babble"


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(ROOT), **kwargs)

    def end_headers(self):
        """Disable caching for pkg/ and viz/ assets during development."""
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def translate_path(self, path):
        """Serve /babble/... from the sibling babble repo."""
        clean = unquote(path)
        if clean.startswith("/babble/"):
            return str(BABBLE / clean[len("/babble/"):])
        return super().translate_path(path)

    def do_PUT(self):
        """Write a file strictly under viz/results/. Creates parent dirs."""
        rel = unquote(self.path.lstrip("/"))
        try:
            target = (ROOT / rel).resolve()
            target.relative_to(RESULTS)
        except ValueError:
            self.send_error(403, "only viz/results/* may be written")
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
        """Delete a file or directory strictly under viz/results/."""
        rel = unquote(self.path.lstrip("/"))
        try:
            target = (ROOT / rel).resolve()
            target.relative_to(RESULTS)  # rejects escape attempts
        except ValueError:
            self.send_error(403, "only viz/results/* may be deleted")
            return
        if target == RESULTS:
            self.send_error(403, "refusing to delete viz/results/ itself")
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


def main():
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8066
    print(f"serving on http://localhost:{port}/viz/", flush=True)
    ThreadingHTTPServer(("", port), Handler).serve_forever()


if __name__ == "__main__":
    main()
