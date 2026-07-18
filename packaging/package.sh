#!/bin/sh
# Stage release binaries into a versioned tarball under dist/.
#
# Usage: sh packaging/package.sh <platform-label> [binary...]
#   e.g. sh packaging/package.sh linux-x86_64
#        sh packaging/package.sh illumos-x86_64 wrsrvd wradm wrsessd wrlogin wr-wm-tiling
#
# Binaries default to the full workspace set. Run from the repo root after
# `cargo build --release`. POSIX sh so it runs on illumos as well; prefers
# GNU tar (gtar) where the system tar lacks -z.
set -eu

PLATFORM="${1:?usage: package.sh <platform-label> [binary...]}"
shift || true
BINS="${*:-wrsrvd wrclient wradm wrsessd wrlogin wr-wm-tiling}"

# Version precedence: explicit WAYRAY_VERSION (set by the release workflow
# from the tag — the OpenIndiana VM has no git, so `git describe` can't work
# there), then git describe, then a dev fallback.
VERSION="${WAYRAY_VERSION:-$(git describe --tags --always 2>/dev/null || echo 0.1.0-dev)}"
# Strip a leading v so tarballs read wayray-0.1.0-beta.1-... not wayray-v0.1.0.
VERSION="${VERSION#v}"

NAME="wayray-$VERSION-$PLATFORM"
STAGE="dist/$NAME"

rm -rf "$STAGE"
mkdir -p "$STAGE/bin"

for b in $BINS; do
    if [ ! -f "target/release/$b" ]; then
        echo "error: target/release/$b missing — build before packaging" >&2
        exit 1
    fi
    cp "target/release/$b" "$STAGE/bin/"
done

cp packaging/install.sh "$STAGE/"
# Ship the mdbook user guide when it has been built (optional).
[ -d book/book ] && cp -r book/book "$STAGE/doc" || true

TAR="$(command -v gtar || command -v tar)"
(cd dist && "$TAR" -czf "$NAME.tar.gz" "$NAME")
echo "created dist/$NAME.tar.gz ($(du -h "dist/$NAME.tar.gz" | cut -f1))"
