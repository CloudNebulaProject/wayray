#!/bin/sh
# Wait for wrsrvd's auto-created Wayland socket to appear in the shared
# XDG_RUNTIME_DIR, then launch a real Wayland application so the compositor has
# actual content to capture and stream over QUIC.
set -eu

RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/wayray}"
mkdir -p "$RUNTIME_DIR"
chmod 0700 "$RUNTIME_DIR" 2>/dev/null || true

echo "[test-client] waiting for a Wayland socket in $RUNTIME_DIR ..."
SOCKET=""
i=0
while [ "$i" -lt 60 ]; do
    # Smithay's new_auto() names sockets wayland-0, wayland-1, ...
    for s in "$RUNTIME_DIR"/wayland-*; do
        case "$s" in
            *.lock) continue ;;
        esac
        if [ -S "$s" ]; then
            SOCKET="$(basename "$s")"
            break
        fi
    done
    [ -n "$SOCKET" ] && break
    i=$((i + 1))
    sleep 1
done

if [ -z "$SOCKET" ]; then
    echo "[test-client] ERROR: no Wayland socket appeared after 60s" >&2
    ls -la "$RUNTIME_DIR" >&2 || true
    exit 1
fi

export WAYLAND_DISPLAY="$SOCKET"
echo "[test-client] found socket: WAYLAND_DISPLAY=$WAYLAND_DISPLAY"

# Prefer foot; fall back to weston-terminal. Run a trivial command loop so the
# window stays mapped (producing continuous content) without needing a TTY.
LAUNCH_CMD='while true; do date; sleep 5; done'

if command -v foot >/dev/null 2>&1; then
    echo "[test-client] launching foot"
    # foot runs as a standalone client by default; the trailing argv is the
    # command to run inside the terminal.
    exec foot sh -c "$LAUNCH_CMD"
fi

if command -v weston-terminal >/dev/null 2>&1; then
    echo "[test-client] launching weston-terminal"
    exec weston-terminal --shell="/bin/sh -c '$LAUNCH_CMD'"
fi

echo "[test-client] ERROR: no Wayland terminal available" >&2
exit 1
