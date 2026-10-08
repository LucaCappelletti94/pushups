"""Sends one high-priority FCM data message carrying `seq` and `sent_at_ms` to a device token.

Usage: python3 fcm_push.py <service account json> <device token> <seq>
"""

import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import send  # noqa: E402


def main() -> None:
    key = json.loads(Path(sys.argv[1]).read_text())
    message = {
        "token": sys.argv[2],
        "android": {"priority": "high"},
        "data": {"seq": sys.argv[3], "sent_at_ms": str(int(time.time() * 1000))},
    }
    status = send.send(key, send.access_token(key), message)
    print(status.splitlines()[0][:200])
    if not status.startswith("200"):
        sys.exit(1)


if __name__ == "__main__":
    main()
