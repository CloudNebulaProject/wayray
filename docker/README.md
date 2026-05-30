# WayRay Docker Test Deployment

A reproducible `docker compose` stack that exercises the WayRay remote-display
loop end-to-end, including the Phase 3 workspace, session-mobility, and
multi-server features.

## What it runs

| Service       | Image                   | Role |
|---------------|-------------------------|------|
| `wrsrvd`      | `wayray/wrsrvd:test`    | Headless Smithay compositor. Listens for QUIC clients on `4433/udp`, runs in **clustered mode** (Phase 3.6) via `cluster.toml`, and creates an auto-named Wayland socket (`wayland-N`) in a shared runtime volume. |
| `test-client` | `wayray/test-client:test` | A real Wayland app (`foot`, with `weston-terminal` fallback). Connects to `wrsrvd`'s Wayland socket through the shared `XDG_RUNTIME_DIR` volume so the compositor has genuine content to capture and stream. |
| `wrclient`    | `wayray/wrclient:test`  | The WayRay thin-client viewer. Connects to `wrsrvd` over QUIC with a fixed session token to exercise the Phase 3.5 hot-desking / session-rebind path. |

## Image build

`Dockerfile` is multi-stage:

- **builder** — uses `rust:1-bookworm`, which always resolves to the *current*
  stable Rust 1.x (never an old pinned toolchain). Build caches (`cargo`
  registry, git, and the `target/` dir) are mounted as BuildKit caches keyed on
  `$TARGETPLATFORM`, so parallel multi-arch builds stay isolated.
- **runtime-base** — slim `debian:bookworm-slim` with only the shared runtime
  libraries (`libwayland`, `libpixman`, `libxkbcommon`).
- **wrsrvd / wrclient / wayland-test-client** — three slim final stages, each
  selected by compose via `target:`.

The illumos core path is headless-first and avoids the Linux-only crates pulled
in here; this Dockerfile is the **Linux** build.

## Bring it up

From the repository root:

```sh
docker compose -f docker/docker-compose.yml up -d --build
```

## What to look for

Check service state:

```sh
docker compose -f docker/docker-compose.yml ps
```

`wrsrvd` should become `healthy` (its healthcheck passes once the Wayland
socket exists).

Key log lines:

```sh
# Compositor: socket created, QUIC listening, clustered mode, client connect.
docker compose -f docker/docker-compose.yml logs wrsrvd

# Look for:
#   "wayland socket created"
#   "QUIC server listening" on 0.0.0.0:4433
#   "running in multi-server cluster mode"   (Phase 3.6)
#   "client connected" / "received ClientHello" / "sent ServerHello"  (handshake + session bind)

# Test client: found the shared socket and launched the terminal.
docker compose -f docker/docker-compose.yml logs test-client
#   "found socket: WAYLAND_DISPLAY=wayland-..."

# Viewer: reached the QUIC handshake and got a session id from ServerHello.
docker compose -f docker/docker-compose.yml logs wrclient
#   "connecting to server"
#   "connected to server" with session_id   (Phase 3.5 token bind)
```

### Note on `wrclient` in a headless container

`wrclient` renders with `winit` + `wgpu`, which need a display/GPU. In a
headless CI container it will complete the QUIC handshake and log the
`ServerHello` (proving the session bind / Phase 3.5 path) and then fail to open
a window. That is expected here — the meaningful evidence is the handshake and
session-id log line. To see actual rendering, run `wrclient` on a host with a
display:

```sh
# On a desktop host (Wayland or X11):
cargo run --release -p wrclient -- 127.0.0.1:4433
```

## Tear down

```sh
docker compose -f docker/docker-compose.yml down -v
```

## Phase 3 features exercised

- **3.2 Workspaces** — the compositor runs its WM workspace dispatch; a real
  Wayland client mapping a surface drives the workspace/window assignment and
  visibility paths in the render loop.
- **3.5 Hot-desking** — `wrclient` presents a fixed session token; reconnecting
  it (`docker compose restart wrclient`) rebinds the existing session rather
  than creating a new one.
- **3.6 Multi-server** — `wrsrvd` boots with `cluster.toml` (two servers), so
  the static-discovery session registry, placement, and cross-server redirect
  code paths are active. The second peer is intentionally unreachable to show
  graceful fallback to the local server.
