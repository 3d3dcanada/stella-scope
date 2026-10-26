#!/usr/bin/env python3
"""Check camera response isolation with local fake Moonraker and camera servers."""

import http.client
import json
import socket
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target/release/stella-scope"
CASE = "jpeg"


class FakePrinter(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/server/webcams/list":
            camera = f"http://127.0.0.1:{self.server.server_port}/cam"
            body = json.dumps({"result": {"webcams": [{"name": "fake", "enabled": True,
                "stream_url": camera, "snapshot_url": camera}]}}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path != "/cam":
            self.send_error(404)
            return
        headers, body = {
            "jpeg": ([("Content-Type", "image/jpeg"), ("Set-Cookie", "evil=1")], b"FAKE-JPEG"),
            "html": ([("Content-Type", "text/html"), ("Set-Cookie", "evil=1")], b"<script>evil()</script>"),
            "redirect": ([("Location", "https://evil.example/"), ("Content-Type", "image/jpeg")], b"redirect"),
            "duplicate": ([("Content-Type", "image/jpeg"), ("Content-Type", "text/html")], b"duplicate"),
            "chunked": ([("Content-Type", "image/jpeg"), ("Transfer-Encoding", "chunked")], b"4\r\nFAKE\r\n0\r\n\r\n"),
            "mjpeg": ([("Content-Type", "multipart/x-mixed-replace; boundary=frame_1")],
                      b"--frame_1\r\nContent-Type: image/jpeg\r\n\r\nFAKE-JPEG\r\n--frame_1--\r\n"),
        }[CASE]
        self.send_response_only(302 if CASE == "redirect" else 200)
        for name, value in headers:
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def request(service_port, route):
    conn = http.client.HTTPConnection("127.0.0.1", service_port, timeout=4)
    conn.request("GET", route, headers={"Host": f"127.0.0.1:{service_port}"})
    response = conn.getresponse()
    result = response.status, dict(response.getheaders()), response.read()
    conn.close()
    return result


def main():
    global CASE
    fake = ThreadingHTTPServer(("127.0.0.1", 0), FakePrinter)
    thread = threading.Thread(target=fake.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="stella-camera-smoke-") as temporary:
            data = Path(temporary)
            service_port = port()
            config = data / "config.json"
            config.write_text(json.dumps({"moonraker_url": f"http://127.0.0.1:{fake.server_port}",
                "bind": "127.0.0.1", "port": service_port, "remote_link": False,
                "failure_watch": False, "controls_enabled": False, "desktop_notifications": False,
                "database": str(data / "demo.sqlite3")}))
            log = data / "recorder.log"
            with log.open("w+") as output:
                proc = subprocess.Popen([str(BIN), "--config", str(config), "--demo"],
                                        cwd=data, stdout=output, stderr=subprocess.STDOUT)
                try:
                    for _ in range(100):
                        if proc.poll() is not None:
                            raise AssertionError("demo recorder exited before listening")
                        try:
                            request(service_port, "/api/meta")
                            break
                        except OSError:
                            time.sleep(0.05)
                    else:
                        raise AssertionError("demo recorder did not listen")
                    for CASE, route, expected in [
                        ("jpeg", "/api/camera/snapshot?name=fake", 200),
                        ("html", "/api/camera/snapshot?name=fake", 502),
                        ("redirect", "/api/camera/snapshot?name=fake", 502),
                        ("duplicate", "/api/camera/snapshot?name=fake", 502),
                        ("chunked", "/api/camera/snapshot?name=fake", 502),
                        ("mjpeg", "/api/camera/stream?name=fake", 200),
                    ]:
                        status, headers, body = request(service_port, route)
                        assert status == expected, (CASE, status, body)
                        assert "Set-Cookie" not in headers and "Location" not in headers, (CASE, headers)
                        assert b"<script>" not in body, CASE
                        if status == 200:
                            assert headers.get("X-Content-Type-Options") == "nosniff", CASE
                            assert headers.get("Content-Security-Policy") == "default-src 'none'; sandbox", CASE
                            assert b"FAKE-JPEG" in body, CASE
                        print(f"{CASE}: {status}")
                except Exception:
                    output.flush()
                    print(log.read_text(), file=sys.stderr)
                    raise
                finally:
                    proc.terminate()
                    try:
                        proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        proc.kill()
                        proc.wait(timeout=5)
    finally:
        fake.shutdown()
        fake.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    main()
