#!/usr/bin/env python3
"""Tiny static server for the repo root with DELETE support scoped to viz/results/*.

Extends the stdlib http.server with a single extra verb so the viz UI can
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


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(ROOT), **kwargs)

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
