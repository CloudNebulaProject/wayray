# syntax=docker/dockerfile:1.7

###############################################################################
# WayRay test-deployment image build.
#
# Multi-stage build of the WayRay workspace using the *current* stable Rust
# image (rust:1-bookworm always resolves to the latest stable 1.x release, so
# we never pin to an old toolchain -- per CLAUDE.md). Two slim runtime images
# are produced from a single builder: `wrsrvd` (compositor) and `wrclient`
# (viewer), selected via the final `--target` stage.
#
# Build caches are keyed on $TARGETPLATFORM so parallel multi-arch builds do
# not stomp on each other's cargo registry / target dirs (per CLAUDE.md).
###############################################################################

ARG RUST_IMAGE=rust:1-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

# ---------------------------------------------------------------------------
# Builder
# ---------------------------------------------------------------------------
FROM ${RUST_IMAGE} AS builder

# Build-time system dependencies.
#  - Smithay (wayland_frontend + desktop + renderer_pixman) links against
#    libwayland, libpixman and libxkbcommon.
#  - wrclient (winit + wgpu) links against libxkbcommon / X11 / Wayland client
#    libs for windowing.
# These are Linux build deps only; the illumos port avoids these crates in its
# core path (headless-first), so this Dockerfile targets the Linux build.
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        libwayland-dev \
        libpixman-1-dev \
        libxkbcommon-dev \
        libudev-dev \
        libx11-dev \
        libxcb1-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# TARGETPLATFORM is provided automatically by BuildKit (e.g. linux/amd64).
# We fold it into the cache-mount ids so each target platform gets its own
# isolated registry + target cache.
ARG TARGETPLATFORM=linux/amd64

# Copy the whole workspace. The cargo cache mounts make incremental rebuilds
# cheap; copying everything keeps the build reproducible regardless of which
# crate paths the workspace references.
COPY . .

# Build only the two binaries the test deployment needs, in release mode.
# The target dir lives on a cache mount, so we copy the artifacts out to a
# stable location before the mount is unmounted.
RUN --mount=type=cache,id=wayray-cargo-registry-${TARGETPLATFORM},target=/usr/local/cargo/registry \
    --mount=type=cache,id=wayray-cargo-git-${TARGETPLATFORM},target=/usr/local/cargo/git \
    --mount=type=cache,id=wayray-target-${TARGETPLATFORM},target=/build/target \
    cargo build --release --locked -p wrsrvd -p wrclient \
    && mkdir -p /build/out \
    && cp target/release/wrsrvd target/release/wrclient /build/out/

# ---------------------------------------------------------------------------
# Common slim runtime base (shared runtime libraries for both images)
# ---------------------------------------------------------------------------
FROM ${RUNTIME_IMAGE} AS runtime-base
RUN apt-get update && apt-get install -y --no-install-recommends \
        libwayland-client0 \
        libwayland-server0 \
        libpixman-1-0 \
        libxkbcommon0 \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home --home-dir /home/wayray wayray

# ---------------------------------------------------------------------------
# wrsrvd runtime image  (build with --target wrsrvd)
# ---------------------------------------------------------------------------
FROM runtime-base AS wrsrvd
COPY --from=builder /build/out/wrsrvd /usr/local/bin/wrsrvd
# Shared Wayland-socket / runtime dir, mounted from a volume in compose.
ENV XDG_RUNTIME_DIR=/run/wayray \
    XDG_CONFIG_HOME=/home/wayray/.config \
    RUST_LOG=info
EXPOSE 4433/udp
ENTRYPOINT ["/usr/local/bin/wrsrvd"]

# ---------------------------------------------------------------------------
# wrclient runtime image  (build with --target wrclient)
# ---------------------------------------------------------------------------
FROM runtime-base AS wayland-test-client
# A real Wayland application (foot terminal + weston-terminal as fallback) used
# to produce actual on-screen content for the compositor to capture and stream.
# It connects to wrsrvd's auto-created Wayland socket via the shared
# XDG_RUNTIME_DIR volume and WAYLAND_DISPLAY.
RUN apt-get update && apt-get install -y --no-install-recommends \
        foot \
        weston \
        libwayland-cursor0 \
    && rm -rf /var/lib/apt/lists/*
COPY docker/wait-and-launch-client.sh /usr/local/bin/wait-and-launch-client.sh
RUN chmod +x /usr/local/bin/wait-and-launch-client.sh
ENV XDG_RUNTIME_DIR=/run/wayray
ENTRYPOINT ["/usr/local/bin/wait-and-launch-client.sh"]

# ---------------------------------------------------------------------------
# wrclient runtime image  (build with --target wrclient)
# ---------------------------------------------------------------------------
FROM runtime-base AS wrclient
# wgpu/winit need GL + Vulkan loader libs even to initialise an offscreen /
# software path; install the loaders so the binary starts and reaches the QUIC
# handshake even on a headless host.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libgl1 \
        libvulkan1 \
        libegl1 \
        mesa-vulkan-drivers \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/out/wrclient /usr/local/bin/wrclient
ENV XDG_CONFIG_HOME=/home/wayray/.config \
    RUST_LOG=info
ENTRYPOINT ["/usr/local/bin/wrclient"]
