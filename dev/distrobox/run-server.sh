#!/usr/bin/env bash
# Build and run the headless WayRay compositor inside the wayray-server box.
#
# Run from the Linux host:
#   distrobox enter wayray-server -- ./dev/distrobox/run-server.sh
# Multi-server / cluster mode:
#   distrobox enter wayray-server -- ./dev/distrobox/run-server.sh --cluster docker/cluster.toml
#
# wrsrvd binds 0.0.0.0:4433 by default; thanks to distrobox host networking it
# is reachable at this host's LAN IP, so the macOS wrclient can connect to it.
# Watch the log for two things:
#   - "wayland socket created socket_name=wayland-N"  -> pass N to add-content.sh
#   - "fingerprint=sha256:..."                        -> optional strict pin on the client
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# Keep box-built artifacts separate from any host target/ to avoid linking the
# host's (dep-less) build against the box's libraries.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/distrobox}"

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found. Either your host rustup isn't on PATH inside the box," >&2
    echo "or install it in-box:  curl https://sh.rustup.rs -sSf | sh -s -- -y" >&2
    exit 1
fi

echo ">> Building wrsrvd (release) into $CARGO_TARGET_DIR ..."
cargo build --release -p wrsrvd

echo ">> Starting wrsrvd (Ctrl-C to stop). Note the wayland socket name and the"
echo ">> certificate fingerprint printed below."
exec "$CARGO_TARGET_DIR/release/wrsrvd" "$@"
