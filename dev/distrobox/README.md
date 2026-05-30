# WayRay dev/test via distrobox (server) + macOS (thin client)

**Topology A, network-exposed.** The headless compositor and Wayland test apps
run in a distrobox container on your **Linux** host (clean separation of the
heavy Wayland/pixman/Mesa deps, while your live source tree and host
`~/.cargo`/`~/.rustup` are shared in). Your **macOS** machine builds and runs
`wrclient` natively and connects over the network — it *is* the thin-client
viewer, which is also where you actually see results (the Linux box is headless).

```
  macOS (native wrclient, has a display)        Linux host
  ┌───────────────────────────┐                 ┌───────────────────────────────┐
  │  wrclient  ──── QUIC :4433 ─┼──── LAN ───────┼─►  distrobox: wayray-server    │
  │  (window, input, decode)   │                 │     wrsrvd (headless) + foot   │
  └───────────────────────────┘                 └───────────────────────────────┘
```

Why distrobox and not the `docker/` compose stack? They're complementary: the
compose stack is sealed/reproducible (good for CI and the canned demo);
distrobox mounts your live tree for fast `cargo build` iteration and shares the
host network so an external client (your Mac) can connect.

---

## 0. Prerequisites (Linux host)

- `podman` (or `docker`) + `distrobox` installed.
- Rust toolchain on the host (shared into the box via `$HOME`); or install it
  in-box on first run (`run-server.sh` tells you how if it's missing).
- Open the QUIC port to the LAN: `sudo ufw allow 4433/udp` (or firewalld
  equivalent). distrobox uses host networking, so no port mapping is needed —
  the port is simply on the host's stack.

## 1. Create the box

```sh
distrobox assemble create --file dev/distrobox/wayray-server.ini
```

## 2. Run the server (terminal 1)

```sh
distrobox enter wayray-server -- ./dev/distrobox/run-server.sh
```

In the log, note:
- `wayland socket created socket_name="wayland-N"` — the N for the next step.
- `QUIC server listening ... fingerprint=sha256:<hex>` — the cert pin (optional,
  for strict client verification).

## 3. Give the compositor content (terminal 2)

```sh
distrobox enter wayray-server -- ./dev/distrobox/add-content.sh wayland-1
```

A `foot` terminal now lives inside the compositor; whatever you type/draw there
is what the client will see.

## 4. Connect from macOS (the thin client)

On the Mac (native build — `wrclient` is cross-platform winit+wgpu):

```sh
cargo build --release -p wrclient
./target/release/wrclient <linux-host-ip>:4433 --token deadbeefcafebabe0011223344556677
```

- A window opens showing the `foot` terminal running on the Linux server.
  Keyboard/mouse go back over QUIC.
- **TLS pinning:** the first connection trust-on-first-use records the server's
  cert fingerprint in `~/.config/wayray/known_hosts` on the Mac and pins it
  thereafter (a later mismatch aborts — MITM protection). To enforce the pin
  from the very first connection, copy the `fingerprint=` from the server log:
  ```sh
  ./target/release/wrclient <linux-host-ip>:4433 --token … \
      --server-fingerprint sha256:<hex>
  ```

## 5. Hot-desking / session mobility demo

The `--token` is your smart-card stand-in (no thin-client hardware needed).
Reconnect with the same token and the server **resumes** the existing session:

```sh
# Kill the client (Ctrl-C) and re-run the exact same command, or run it from a
# second machine with the same --token. Server log shows:
#   session ... active -> suspended ... -> active   (resumed=true, ~5-9ms)
```

## 6. Multi-server / redirect demo (optional)

Run two servers (two boxes, or two `wrsrvd` processes on different ports) each
with a cluster config. Use `docker/cluster.toml` as a template — note it now
requires a shared `cluster_secret` (peer probes are refused without it):

```sh
distrobox enter wayray-server -- ./dev/distrobox/run-server.sh --cluster docker/cluster.toml
```

The client follows `SessionRedirect` only to addresses already in its trusted
candidate/cluster set (pass peers via `--cluster <file>` or repeated `host:port`
args), so an impersonated server can't redirect it off-cluster.

---

## Note on Phase 4 (peripherals) — direction matters

USB forwarding in WayRay is **client → server**: the thin client's USB devices
are forwarded *up* to the server session (the SunRay model — plug a stick into
the endpoint, it appears in your session), **not** the server's USB pushed down.

Implication for this setup: the **macOS client** is what captures local USB and
forwards it over QUIC; the **server** (this distrobox) receives a *userspace
virtual* device into the session and therefore does **not** need host USB
passthrough. So distrobox device-passthrough is not the seam for client USB —
it would only matter if we ever wanted the *server* box to expose host hardware,
which is out of scope. (Audio is the mirror case: server captures app audio →
client plays it natively on the Mac.)
