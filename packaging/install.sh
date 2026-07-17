#!/bin/sh
# WayRay tarball installer for Linux and illumos (OpenIndiana/OmniOS).
#
# Usage: sh install.sh            # installs to /usr/local (may need root)
#        PREFIX=$HOME/.local sh install.sh
#
# Uses only POSIX sh + cp/chmod so the same script works with illumos
# /usr/bin/sh (install(1) flags differ between GNU and illumos).
set -eu

PREFIX="${PREFIX:-/usr/local}"
BINDIR="$PREFIX/bin"
HERE="$(cd "$(dirname "$0")" && pwd)"

if [ ! -d "$HERE/bin" ]; then
    echo "error: $HERE/bin not found; run this script from the unpacked tarball" >&2
    exit 1
fi

mkdir -p "$BINDIR"
installed=""
for b in "$HERE"/bin/*; do
    [ -f "$b" ] || continue
    name="$(basename "$b")"
    cp "$b" "$BINDIR/$name"
    chmod 0755 "$BINDIR/$name"
    installed="$installed $name"
done

echo "Installed to $BINDIR:$installed"
echo
echo "Quick start (server):   wrsrvd --admin-socket /var/run/wrsrvd-admin.sock"
echo "Quick start (client):   wrclient <server-host>:4433"
echo "Administration:         wradm status --server-socket /var/run/wrsrvd-admin.sock"
