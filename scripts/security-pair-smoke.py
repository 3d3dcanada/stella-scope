#!/usr/bin/env python3
"""Exercise desktop/phone HTTP gates against an isolated demo recorder."""

import http.client
import json
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target/release/stella-scope"


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def request(port_number, method, path, host, body=None, **headers):
    conn = http.client.HTTPConnection("127.0.0.1", port_number, timeout=3)
    data = json.dumps(body).encode() if body is not None else None
    if data is not None:
        headers["Content-Type"] = "application/json"
    conn.request(method, path, body=data, headers={"Host": host, **headers})
    response = conn.getresponse()
    payload = response.read()
    result = response.status, dict(response.getheaders()), payload
    conn.close()
    return result


def check(status, expected, label):
    assert status == expected, f"{label}: expected {expected}, got {status}"
    print(f"{label}: {status}")


def main():
    if not BIN.is_file():
        raise SystemExit("build target/release/stella-scope first")
    with tempfile.TemporaryDirectory(prefix="stella-pair-smoke-") as temporary:
        data = Path(temporary)
        desktop, phone = port(), port()
        while phone == desktop:
            phone = port()
        config = data / "config.json"
        config.write_text(json.dumps({
            "moonraker_url": "http://127.0.0.1:1",
            "bind": "127.0.0.1", "port": desktop,
            "lan_bind": "127.0.0.1", "lan_port": phone,
            "remote_link": False, "failure_watch": False,
            "controls_enabled": False, "desktop_notifications": False,
            "database": str(data / "demo.sqlite3"),
        }))
        log = data / "recorder.log"
        with log.open("w+") as output:
            proc = subprocess.Popen([str(BIN), "--config", str(config), "--demo"],
                                    cwd=data, stdout=output, stderr=subprocess.STDOUT)
            try:
                host = f"127.0.0.1:{desktop}"
                phone_host = f"127.0.0.1:{phone}"
                for _ in range(100):
                    if proc.poll() is not None:
                        raise AssertionError("demo recorder exited before listening")
                    try:
                        request(desktop, "GET", "/api/meta", host)
                        break
                    except OSError:
                        time.sleep(0.05)
                else:
                    raise AssertionError("demo recorder did not listen")

                check(request(desktop, "GET", "/api/meta", host)[0], 200, "desktop data")
                check(request(desktop, "GET", "/api/meta", f"evil.example:{desktop}")[0], 403, "rebound Host data")
                check(request(desktop, "GET", "/", f"evil.example:{desktop}")[0], 403, "rebound Host page")
                check(request(desktop, "GET", "/next.js", f"evil.example:{desktop}")[0], 403, "rebound Host asset")
                check(request(desktop, "GET", "/api/camera/stream", f"evil.example:{desktop}")[0], 403, "rebound Host camera")
                check(request(desktop, "POST", "/api/pair/new", host, {})[0], 403, "pairing without desktop Origin")
                check(request(desktop, "POST", "/api/pair/new", host, {}, Origin="http://evil.example")[0], 403, "pairing with foreign Origin")
                check(request(phone, "GET", "/api/meta", phone_host)[0], 401, "unpaired phone")
                status, _, raw = request(desktop, "POST", "/api/pair/new", host, {}, Origin=f"http://{host}")
                check(status, 200, "desktop opens pairing")
                code = json.loads(raw)["code"]
                check(request(phone, "POST", "/api/pair", phone_host, {"code": code})[0], 403, "pairing without phone Origin")
                check(request(phone, "POST", "/api/pair", phone_host, {"code": code}, Origin="http://evil.example")[0], 403, "pairing with foreign Origin")
                check(request(phone, "POST", "/api/pair", phone_host, {"code": "xxxxxx"}, Origin=f"http://{phone_host}")[0], 403, "wrong code")
                status, headers, _ = request(phone, "POST", "/api/pair", phone_host, {"code": code}, Origin=f"http://{phone_host}")
                check(status, 200, "phone pairs")
                cookie = headers["Set-Cookie"].split(";", 1)[0]
                check(request(phone, "GET", "/api/meta", phone_host, Cookie=cookie)[0], 200, "paired phone data")
                check(request(phone, "POST", "/api/control/stop", phone_host, {}, Cookie=cookie, Origin="http://evil.example")[0], 403, "paired phone rejects foreign Origin")
                check(request(phone, "GET", "/api/pair/devices", phone_host, Cookie=cookie)[0], 403, "phone cannot manage pairing")
                check(request(phone, "POST", "/api/notify", phone_host, {}, Cookie=cookie)[0], 403, "phone cannot set alerts")
                check(request(phone, "POST", "/api/eye/switch", phone_host, {}, Cookie=cookie)[0], 403, "phone cannot switch watch")
                status, _, raw = request(desktop, "GET", "/api/pair/remote", host)
                check(status, 200, "desktop link status")
                assert json.loads(raw)["app_url"].startswith("https://")
                status, _, raw = request(desktop, "GET", "/api/pair/devices", host)
                check(status, 200, "desktop paired list")
                device_id = json.loads(raw)["devices"][0]["id"]
                check(request(desktop, "POST", "/api/pair/revoke", host, {"id": device_id}, Origin=f"http://{host}")[0], 200, "desktop revokes")
                check(request(phone, "GET", "/api/meta", phone_host, Cookie=cookie)[0], 401, "revoked phone")
                held = []
                try:
                    for _ in range(8):
                        held.append(socket.create_connection(("127.0.0.1", phone), timeout=2))
                    time.sleep(0.2)
                    check(request(phone, "GET", "/api/meta", phone_host)[0], 503, "phone reader cap")
                    check(request(desktop, "GET", "/api/meta", host)[0], 200, "desktop survives phone saturation")
                finally:
                    for sock in held:
                        sock.close()
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


if __name__ == "__main__":
    main()
