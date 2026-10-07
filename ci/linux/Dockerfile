# Debian testing (forky) testbed for the UnifiedPush D-Bus flow.
# Pinned on 2026-10-07 to the amd64 manifest created 2026-10-05.
FROM debian:testing@sha256:9e1caa3238f2e4eee5b28c781ec712798d5d1192a6206d9e0bc7ee2861717a00

ENV DEBIAN_FRONTEND=noninteractive
ENV QT_QPA_PLATFORM=offscreen
ENV LANG=C.UTF-8

# dbus provides dbus-daemon, dbus-run-session and dbus-send. systemd provides busctl, procps pgrep.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        dbus \
        kunifiedpush \
        procps \
        systemd \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -u 1000 -s /bin/bash testuser \
    && mkdir -p /home/testuser/.config/KDE \
    && printf '[PushProvider]\nType=Ntfy\n\n[Ntfy]\nUrl=https://ntfy.sh\n' \
        > /home/testuser/.config/KDE/kunifiedpush-distributor.conf \
    && chown -R testuser /home/testuser

# Starts a session bus, waits for the distributor to own its name, then runs the command.
RUN cat > /usr/local/bin/pw-testbed-run <<'SCRIPT' \
    && chmod 755 /usr/local/bin/pw-testbed-run
#!/bin/bash
exec dbus-run-session -- /usr/local/bin/pw-testbed-inner "$@"
SCRIPT

RUN cat > /usr/local/bin/pw-testbed-inner <<'SCRIPT' \
    && chmod 755 /usr/local/bin/pw-testbed-inner
#!/bin/bash
set -euo pipefail

if [ "$#" -lt 1 ]; then
    echo "pw-testbed: missing command, usage: testbed.sh <command> [args...]" >&2
    exit 2
fi

export HOME=/home/testuser
export XDG_DATA_HOME="$HOME/.local/share"
export XDG_CONFIG_HOME="$HOME/.config"
mkdir -p "$XDG_DATA_HOME/dbus-1/services"

log=/tmp/pw-testbed-distributor.log
kunifiedpush-distributor >"$log" 2>&1 &
distributor_pid=$!

deadline=$((SECONDS + 30))
owned=0
while [ "$SECONDS" -lt "$deadline" ]; do
    if ! kill -0 "$distributor_pid" 2>/dev/null; then
        echo "pw-testbed: kunifiedpush-distributor exited early" >&2
        cat "$log" >&2
        exit 1
    fi
    if dbus-send --session --print-reply --dest=org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus.ListNames 2>/dev/null | grep -q org.unifiedpush.Distributor.kde; then
        owned=1
        break
    fi
    sleep 0.2
done

if [ "$owned" -ne 1 ]; then
    echo "pw-testbed: distributor did not own org.unifiedpush.Distributor.kde within 30 s" >&2
    cat "$log" >&2
    kill "$distributor_pid" 2>/dev/null || true
    exit 1
fi

set +e
"$@"
rc=$?
set -e

kill "$distributor_pid" 2>/dev/null || true
wait "$distributor_pid" 2>/dev/null || true
exit "$rc"
SCRIPT

ENTRYPOINT ["/usr/local/bin/pw-testbed-run"]
CMD ["/bin/bash"]
