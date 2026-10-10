"""Sends the pushes the instrumented tests ask for.

The tests write `<id>.json` into `files/pushups-ci` of the test package, either an FCM HTTP v1
`message` object or `{"webpush": <subscription>, "payload": <text>}`. This script polls that
directory over adb, sends each push with keys that stay on the host, the FCM service account or the
VAPID private key, and writes the answer to `<id>.status` as `<id> <HTTP status> <body>`. It stops
after `--bound` seconds on the monotonic clock, or when killed by run.sh.

Usage: python3 send.py <service account json> [--webpush-sender path --vapid pem] [--bound seconds]
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
# How long a push is re-sent while FCM answers with something that passes: 404 UNREGISTERED for a
# moment after a token is new (the web job's 410), and the 429 and 5xx that FCM asks callers to retry.
RETRY_S = 60


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


def post(key: dict, token: str, message: dict) -> str:
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


def transient(status: str) -> bool:
    code = status.split(" ", 1)[0]
    return (code == "404" and "UNREGISTERED" in status) or code == "429" or code.startswith("5")


def send(key: dict, token: str, message: dict) -> str:
    """Posts the message, re-sending it for up to RETRY_S while FCM's answer is transient."""
    deadline = time.monotonic() + RETRY_S
    while True:
        status = post(key, token, message)
        if not transient(status) or time.monotonic() >= deadline:
            return status
        time.sleep(3)


def send_web_push(sender: Path, vapid: Path, request: dict) -> str:
    """Sends through the `web-push` crate's sender, which retries transient answers itself."""
    with tempfile.NamedTemporaryFile("w", suffix=".json") as subscription:
        json.dump(request["webpush"], subscription)
        subscription.flush()
        done = subprocess.run(
            [str(sender), subscription.name, str(vapid), request["payload"]],
            capture_output=True,
            text=True,
            timeout=90,
        )
    return f"200 {done.stdout.strip()}" if done.returncode == 0 else f"502 {done.stdout.strip()} {done.stderr.strip()}"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("key", type=Path)
    parser.add_argument("--webpush-sender", type=Path)
    parser.add_argument("--vapid", type=Path)
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
            request = json.loads(run_as(f"cat {DIR}/{name}"))
            if "webpush" in request:
                if args.webpush_sender is None or args.vapid is None:
                    status = "500 send.py has no --webpush-sender and --vapid"
                else:
                    status = send_web_push(args.webpush_sender, args.vapid, request)
            else:
                status = send(key, token, request)
            print(f"push {request_id}: {status.splitlines()[0][:120]}", flush=True)
            # Renamed into place, so the test never reads a status still being written.
            part = f"{DIR}/{request_id}.status.part"
            run_as(f"cat > {part} && mv {part} {DIR}/{request_id}.status", stdin=f"{request_id} {status}".encode())
            answered.add(request_id)
        time.sleep(POLL_S)


if __name__ == "__main__":
    main()
