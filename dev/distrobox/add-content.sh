#!/usr/bin/env bash
# Launch a Wayland test app inside the wayray-server box so the compositor has
# real content to capture and stream to the client. Run AFTER run-server.sh,
# in a second terminal, passing the wayland socket name from wrsrvd's log:
#
#   distrobox enter wayray-server -- ./dev/distrobox/add-content.sh wayland-1
set -euo pipefail

SOCK="${1:-wayland-1}"
export WAYLAND_DISPLAY="$SOCK"
echo ">> Launching foot against WAYLAND_DISPLAY=$WAYLAND_DISPLAY"
echo ">> (it appears inside the compositor; the macOS client renders it)."
exec foot
