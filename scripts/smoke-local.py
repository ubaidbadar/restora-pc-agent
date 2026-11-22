"""Exercise the real loopback server in an isolated profile; never submit a print.

Usage: python scripts/smoke-local.py target/debug/restora-pc-agent
Uses only Python's standard library. Requires port 17777 to be free.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import tomllib
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:17777"
ORIGIN = "https://app.restora.live"
VERSION = tomllib.loads((Path(__file__).resolve().parent.parent / "Cargo.toml").read_text())["package"]["version"]
HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def request(path, method="GET", headers=None, body=None):
    req = urllib.request.Request(
        BASE + path, method=method, headers=headers or {},
        data=json.dumps(body).encode() if body is not None else None,
    )
    try:
        result = HTTP.open(req, timeout=2)
    except urllib.error.HTTPError as error:
        result = error
    with result:
        return result.status, result.headers, result.read()


def run(executable, paired):
    with tempfile.TemporaryDirectory(prefix="restora-smoke-") as profile:
        folder = Path(profile) / "restora"
        folder.mkdir()
        if paired:
            # Local-only fixture. Never use or read a staff member's real token.
            (folder / "agent.json").write_text(json.dumps({
                "api": "http://127.0.0.1:1", "token": "smoke-fixture-not-a-credential",
                "branchId": "smoke-branch",
            }))
        env = dict(os.environ, APPDATA=profile, RESTORA_ORIGINS=ORIGIN)
        with open(Path(profile) / "process.log", "w+") as log:
            process = subprocess.Popen([executable, "--background"], env=env, stdout=log, stderr=log)
            try:
                for _ in range(100):
                    assert process.poll() is None, "Agent exited before readiness"
                    try:
                        code, _, raw = request("/status")
                        if code == 200:
                            break
                    except OSError:
                        pass
                    time.sleep(0.1)
                else:
                    raise AssertionError("Agent never became ready")
                snapshot = json.loads(raw)
                assert snapshot["paired"] == paired, snapshot
                assert snapshot["version"] == VERSION, snapshot
                assert snapshot["wpp"] in ("unknown", True, False), snapshot
                assert isinstance(snapshot["printers"], list), snapshot
                assert b"smoke-fixture-not-a-credential" not in raw
                assert request("/status", headers={"Host": "attacker.example"})[0] == 403
                assert request("/print", "POST", body={})[0] == 403
                assert request("/test-print", "POST", body={})[0] == 403
                assert request("/test-print", "POST", {"Origin": "https://attacker.example"}, {})[0] == 403
                assert request("/print", "POST", {"Origin": "https://attacker.example"}, {})[0] == 403
                code, headers, _ = request("/status", "OPTIONS", {
                    "Origin": ORIGIN, "Access-Control-Request-Method": "GET",
                    "Access-Control-Request-Private-Network": "true",
                })
                assert code == 200 and headers["Access-Control-Allow-Origin"] == ORIGIN
                assert headers["Access-Control-Allow-Private-Network"] == "true"
                code, _, raw = request("/config", headers={"Origin": ORIGIN})
                assert code == 200 and b"smoke-fixture-not-a-credential" not in raw
                # A complete but unauthorized job must stop before the spooler.
                code, _, raw = request("/print", "POST", {
                    "Origin": ORIGIN, "Content-Type": "application/json",
                }, {
                    "branchId": "different-branch", "printer": "smoke-never-print",
                    "kind": "kitchen", "orderKey": "smoke-order",
                    "order": {"dailyToken": "1", "type": "takeaway", "items": []},
                })
                assert code == 403, (code, raw)
                expected = "different Restora branch" if paired else "Connect this agent"
                assert expected in json.loads(raw)["message"]
                # The explicit test route must reject unpaired or wrong-branch
                # calls before any RAW data reaches a physical printer.
                code, _, raw = request("/test-print", "POST", {
                    "Origin": ORIGIN, "Content-Type": "application/json",
                }, {"branchId": "wrong-branch", "printer": "never-print"})
                assert code == 403, (code, raw)
            except BaseException:
                log.flush()
                log.seek(0)
                print(log.read(), file=sys.stderr)
                raise
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


if __name__ == "__main__":
    executable = str(Path(sys.argv[1]).resolve())
    # Refuse to accidentally test or interfere with an existing user's agent.
    import socket
    with socket.socket() as probe:
        assert probe.connect_ex(("127.0.0.1", 17777)) != 0, "Port 17777 is already in use"
    for paired in (False, True):
        run(executable, paired)
    print("Loopback smoke checks passed: readiness, version, CORS, Host, Origin, branch isolation, token secrecy")
