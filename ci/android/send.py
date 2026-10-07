"""Sends the FCM messages the instrumented tests ask for.

The tests write `<id>.json`, an FCM HTTP v1 `message` object, into `files/pushups-ci` of the test
package. This script polls that directory over adb, sends each message with the service account
key, which stays on the host, and writes the answer to `<id>.status` as `<id> <HTTP status> <body>`.
It stops after `--bound` seconds on the monotonic clock, or when killed by run.sh.

Usage: python3 send.py <service account json> [--bound seconds]
"""

import argparse
import base64
import json
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

PACKAGE = "rs.pushups.example"
DIR = "files/pushups-ci"
POLL_S = 0.5


def b64url(data: bytes) -> bytes:
    return base64.urlsafe_b64encode(data).rstrip(b"=")


def access_token(key: dict) -> str:
    """An OAuth token for the service account, from a JWT signed with openssl."""
    now = int(time.time())
    header = b64url(json.dumps({"alg": "RS256", "typ": "JWT"}).encode())
    claims = b64url(
        json.dumps(
            {
                "iss": key["client_email"],
                "scope": "https://www.googleapis.com/auth/firebase.messaging",
                "aud": key["token_uri"],
                "iat": now,
                "exp": now + 3600,
            }
        ).encode()
    )
    with tempfile.NamedTemporaryFile("w", suffix=".pem") as pem:
        pem.write(key["private_key"])
        pem.flush()
        signature = subprocess.run(
            ["openssl", "dgst", "-sha256", "-sign", pem.name],
            input=header + b"." + claims,
            capture_output=True,
            check=True,
        ).stdout
    assertion = (header + b"." + claims + b"." + b64url(signature)).decode()
    body = f"grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer&assertion={assertion}".encode()
    with urllib.request.urlopen(key["token_uri"], data=body, timeout=30) as response:
        return json.load(response)["access_token"]


def adb(*args: str, stdin: bytes | None = None) -> bytes:
    return subprocess.run(["adb", *args], input=stdin, capture_output=True, check=True, timeout=30).stdout


def run_as(command: str, stdin: bytes | None = None) -> bytes:
    return adb("shell", f"run-as {PACKAGE} sh -c '{command}'", stdin=stdin)


def send(key: dict, token: str, message: dict) -> str:
    request = urllib.request.Request(
        f"https://fcm.googleapis.com/v1/projects/{key['project_id']}/messages:send",
        data=json.dumps({"message": message}).encode(),
        headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return f"{response.status} {response.read().decode()}"
    except urllib.error.HTTPError as error:
        return f"{error.code} {error.read().decode()}"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("key", type=Path)
    parser.add_argument("--bound", type=float, default=1800)
    args = parser.parse_args()
    key = json.loads(args.key.read_text())
    token = access_token(key)
    deadline = time.monotonic() + args.bound
    answered: set[str] = set()
    while time.monotonic() < deadline:
        try:
            names = run_as(f"ls {DIR} 2>/dev/null || true").decode().split()
        except subprocess.SubprocessError:
            # The package is not installed yet, or the device is busy installing it.
            time.sleep(POLL_S)
            continue
        for name in names:
            if not name.endswith(".json"):
                continue
            request_id = name.removesuffix(".json")
            if request_id in answered:
                continue
            message = json.loads(run_as(f"cat {DIR}/{name}"))
            status = send(key, token, message)
            print(f"push {request_id}: {status.splitlines()[0][:120]}", flush=True)
            # Renamed into place, so the test never reads a status still being written.
            part = f"{DIR}/{request_id}.status.part"
            run_as(f"cat > {part} && mv {part} {DIR}/{request_id}.status", stdin=f"{request_id} {status}".encode())
            answered.add(request_id)
        time.sleep(POLL_S)


if __name__ == "__main__":
    main()
