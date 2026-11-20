#!/usr/bin/env python3
"""Tiny loopback-only HTTP server with single-byte-range support for MPV CI."""
from __future__ import annotations

import argparse
import mimetypes
import re
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlsplit


class RangeHandler(BaseHTTPRequestHandler):
    root = Path(".").resolve()

    def do_HEAD(self) -> None:
        self._serve_file(send_body=False)

    def do_GET(self) -> None:
        self._serve_file(send_body=True)

    def _serve_file(self, *, send_body: bool) -> None:
        relative = unquote(urlsplit(self.path).path).lstrip("/")
        target = (self.root / relative).resolve()
        if not target.is_relative_to(self.root) or not target.is_file():
            self.send_error(404)
            return

        size = target.stat().st_size
        start, end, partial = 0, max(0, size - 1), False
        range_header = self.headers.get("Range")
        if range_header:
            match = re.fullmatch(r"bytes=(\d*)-(\d*)", range_header.strip())
            if not match or (not match.group(1) and not match.group(2)):
                self.send_error(416)
                return
            left, right = match.groups()
            if not left:
                suffix = int(right)
                start = max(0, size - suffix)
            else:
                start = int(left)
            if right and left:
                end = min(int(right), size - 1)
            if start >= size or end < start:
                self.send_response(416)
                self.send_header("Content-Range", f"bytes */{size}")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            partial = True

        length = end - start + 1
        self.send_response(206 if partial else 200)
        self.send_header("Content-Type", mimetypes.guess_type(target.name)[0] or "application/octet-stream")
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(length))
        if partial:
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.end_headers()
        if not send_body:
            return

        with target.open("rb") as media:
            media.seek(start)
            remaining = length
            while remaining:
                chunk = media.read(min(64 * 1024, remaining))
                if not chunk:
                    break
                self.wfile.write(chunk)
                remaining -= len(chunk)

    def log_message(self, _format: str, *_args: object) -> None:
        pass


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=18765)
    args = parser.parse_args()
    RangeHandler.root = args.directory.resolve()
    ThreadingHTTPServer((args.host, args.port), RangeHandler).serve_forever()


if __name__ == "__main__":
    main()
